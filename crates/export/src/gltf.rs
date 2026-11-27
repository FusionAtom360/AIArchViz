//! glTF 2.0 binary (GLB) export (spec 5.4, 11.3; Q-54, Q-55, Q-56).
//!
//! World coordinates are right-handed with Z up in nanometers; glTF is
//! right-handed with Y up in meters, so glTF = (x, z, -y) / 1e9. The mapping
//! is a rotation, so triangle winding and normals carry over unchanged.
//!
//! One node per element, in this order, each group by id:
//! - Point datums, the fiducials: empty nodes named after the datum, at the
//!   datum's x, y and the lowest storey elevation (0 with no storeys);
//! - walls: the footprint extruded from base to top, split along the wall
//!   around each opening's void (below the sill and above the head stay
//!   solid), end cuts kept;
//! - doors and windows: a 1 3/4 in panel or a 1/4 in pane filling the void,
//!   centered in the wall's thickness; plain openings have no node;
//! - slabs: the outline extruded down from the storey elevation plus the slab
//!   elevation by its thickness; ceilings: extruded up from theirs;
//! - roofs: the footprint grown by the overhangs, an inclined top through the
//!   high and low edge heights and a bottom one vertical thickness below
//!   (fascia is not modeled yet);
//! - occurrences: boxes from declared dimensions at their footprint and
//!   height range; an element type's model asset id is kept in extras.
//!
//! Each mesh is in element-local coordinates: the node translation is the
//! element's lowest corner (minimum world x, y and z), so vertex values stay
//! small and precise in 32-bit floats. Faces are flat-shaded with their own
//! vertices and normals; there are no materials yet. Geometry is built in
//! integer nanometers; floats appear only in the final conversion.

use std::fmt;

use archviz_core::{
    Axis, DatumGeometry, Direction, Document, GeometryError, OpeningKind, Point, RoofProblem,
    SpaceError, Uuid, boundary_outline, footprint, height_range, roof_geometry, wall_geometry,
    wall_profiles,
};
use serde_json::{Map, Value, json};

/// A door panel's thickness: 1 3/4 in.
const DOOR_THICKNESS: i64 = 44_450_000;
/// A window pane's thickness: 1/4 in.
const WINDOW_THICKNESS: i64 = 6_350_000;
const NM_PER_M: f64 = 1e9;

/// Why a document cannot be exported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GltfError {
    Geometry(GeometryError),
    Space(SpaceError),
    Roof(Uuid, RoofProblem),
    /// The roof has neither a low edge height nor a pitch.
    RoofWithoutLowHeight(Uuid),
    UnknownStorey(Uuid),
    /// A face could not be triangulated (it is degenerate or self-crossing).
    Triangulation(Uuid),
    Overflow,
}

impl fmt::Display for GltfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GltfError::Geometry(e) => write!(f, "{e}"),
            GltfError::Space(e) => write!(f, "{e}"),
            GltfError::Roof(id, e) => write!(f, "roof {id}: {e}"),
            GltfError::RoofWithoutLowHeight(id) => {
                write!(f, "roof {id} has neither a low edge height nor a pitch")
            }
            GltfError::UnknownStorey(id) => write!(f, "unknown storey {id}"),
            GltfError::Triangulation(id) => write!(f, "a face of {id} cannot be triangulated"),
            GltfError::Overflow => f.write_str("a coordinate is out of range"),
        }
    }
}

impl std::error::Error for GltfError {}

/// A world point in nanometers: x east, y north, z up.
type P3 = [i128; 3];
/// A plan point in nanometers.
type P2 = (i128, i128);

/// One node: a mesh made of planar faces, or an empty node at `at`.
struct Element {
    id: Uuid,
    kind: &'static str,
    name: String,
    extras: Map<String, Value>,
    faces: Vec<Vec<P3>>,
    at: P3,
}

impl Element {
    fn new(id: Uuid, kind: &'static str, name: String) -> Self {
        Element {
            id,
            kind,
            name,
            extras: Map::new(),
            faces: Vec::new(),
            at: [0, 0, 0],
        }
    }
}

fn p2(p: &Point) -> P2 {
    (i128::from(p.x.nanometers()), i128::from(p.y.nanometers()))
}

fn cross(o: P2, a: P2, b: P2) -> i128 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

/// Twice the signed area: positive when counterclockwise.
fn area2(pts: &[P2]) -> i128 {
    let n = pts.len();
    (0..n)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum()
}

/// The polygon counterclockwise, without repeated or collinear vertices;
/// None when nothing with area is left.
fn clean(pts: &[P2]) -> Option<Vec<P2>> {
    let mut v: Vec<P2> = pts.to_vec();
    loop {
        let n = v.len();
        if n < 3 {
            return None;
        }
        let drop = (0..n).find(|&i| {
            let (a, b, c) = (v[(i + n - 1) % n], v[i], v[(i + 1) % n]);
            b == a || cross(a, b, c) == 0
        });
        match drop {
            Some(i) => {
                v.remove(i);
            }
            None => break,
        }
    }
    let a = area2(&v);
    if a == 0 {
        return None;
    }
    if a < 0 {
        v.reverse();
    }
    Some(v)
}

/// `n / d` rounded half to even (d > 0).
fn div_round(n: i128, d: i128) -> i128 {
    let (q, r) = (n.div_euclid(d), n.rem_euclid(d));
    match (2 * r).cmp(&d) {
        std::cmp::Ordering::Less => q,
        std::cmp::Ordering::Greater => q + 1,
        std::cmp::Ordering::Equal => q + (q & 1),
    }
}

/// Ear clipping of a simple polygon; triangles keep the polygon's winding.
fn triangulate(pts: &[P2]) -> Option<Vec<[usize; 3]>> {
    let s = area2(pts).signum();
    if pts.len() < 3 || s == 0 {
        return None;
    }
    let inside = |p: P2, a: P2, b: P2, c: P2| {
        cross(a, b, p) * s >= 0 && cross(b, c, p) * s >= 0 && cross(c, a, p) * s >= 0
    };
    let mut idx: Vec<usize> = (0..pts.len()).collect();
    let mut out = Vec::new();
    while idx.len() > 3 {
        let m = idx.len();
        let ear = (0..m).find(|&i| {
            let (a, b, c) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let (pa, pb, pc) = (pts[a], pts[b], pts[c]);
            cross(pa, pb, pc).signum() == s
                && !idx.iter().any(|&j| {
                    let p = pts[j];
                    p != pa && p != pb && p != pc && inside(p, pa, pb, pc)
                })
        })?;
        out.push([idx[(ear + m - 1) % m], idx[ear], idx[(ear + 1) % m]]);
        idx.remove(ear);
    }
    if cross(pts[idx[0]], pts[idx[1]], pts[idx[2]]) != 0 {
        out.push([idx[0], idx[1], idx[2]]);
    }
    Some(out)
}

/// Newell's normal of a planar face (not normalized).
fn newell(face: &[P3]) -> P3 {
    let n = face.len();
    let mut out = [0i128; 3];
    for i in 0..n {
        let (a, b) = (face[i], face[(i + 1) % n]);
        out[0] += (a[1] - b[1]) * (a[2] + b[2]);
        out[1] += (a[2] - b[2]) * (a[0] + b[0]);
        out[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    out
}

/// Faces of a prism over `plan` (counterclockwise) with per-point bottom and
/// top heights, outward-facing.
fn prism(plan: &[P2], bottom: impl Fn(P2) -> i128, top: impl Fn(P2) -> i128) -> Vec<Vec<P3>> {
    let mut faces = vec![
        plan.iter().map(|&p| [p.0, p.1, top(p)]).collect(),
        plan.iter().rev().map(|&p| [p.0, p.1, bottom(p)]).collect(),
    ];
    let n = plan.len();
    for i in 0..n {
        let (p, q) = (plan[i], plan[(i + 1) % n]);
        faces.push(vec![
            [p.0, p.1, bottom(p)],
            [q.0, q.1, bottom(q)],
            [q.0, q.1, top(q)],
            [p.0, p.1, top(p)],
        ]);
    }
    faces
}

/// A flat-topped prism, or nothing when the plan has no area or the height
/// range is empty.
fn extrude(plan: &[P2], z0: i128, z1: i128) -> Vec<Vec<P3>> {
    match clean(plan) {
        Some(p) if z1 > z0 => prism(&p, |_| z0, |_| z1),
        _ => Vec::new(),
    }
}

fn rect(x0: i128, y0: i128, x1: i128, y1: i128) -> Vec<P2> {
    vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

/// The part of `poly` with `lo <= along <= hi` (Sutherland-Hodgman);
/// crossing points are rounded half to even to the nanometer.
fn clip(poly: &[P2], along_x: bool, lo: Option<i128>, hi: Option<i128>) -> Vec<P2> {
    let get = |p: P2| if along_x { p.0 } else { p.1 };
    let make = |a: i128, c: i128| if along_x { (a, c) } else { (c, a) };
    let mut out = poly.to_vec();
    let planes = [lo.map(|v| (v, 1i128)), hi.map(|v| (v, -1i128))];
    for (c, sign) in planes.into_iter().flatten() {
        let keep = |p: P2| (get(p) - c) * sign >= 0;
        let input = std::mem::take(&mut out);
        let n = input.len();
        for i in 0..n {
            let (p, q) = (input[i], input[(i + 1) % n]);
            if keep(p) {
                out.push(p);
            }
            if keep(p) != keep(q) {
                let (pa, qa) = (get(p), get(q));
                let (pc, qc) = if along_x { (p.1, q.1) } else { (p.0, q.0) };
                let (mut num, mut den) = ((qc - pc) * (c - pa), qa - pa);
                if den < 0 {
                    num = -num;
                    den = -den;
                }
                out.push(make(c, pc + div_round(num, den)));
            }
        }
    }
    out
}

fn storey_elevation(doc: &Document, id: Uuid) -> Result<i128, GltfError> {
    doc.storeys
        .get(&id)
        .map(|s| i128::from(s.elevation.value.nanometers()))
        .ok_or(GltfError::UnknownStorey(id))
}

fn fiducials(doc: &Document) -> Vec<Element> {
    let z = doc
        .storeys
        .values()
        .map(|s| i128::from(s.elevation.value.nanometers()))
        .min()
        .unwrap_or(0);
    doc.datums
        .iter()
        .filter_map(|(id, d)| match d.geometry {
            DatumGeometry::Point { x, y } => {
                let mut e = Element::new(*id, "datum", d.name.clone());
                e.at = [i128::from(x.nanometers()), i128::from(y.nanometers()), z];
                Some(e)
            }
            _ => None,
        })
        .collect()
}

fn walls_and_openings(doc: &Document) -> Result<(Vec<Element>, Vec<Element>), GltfError> {
    let geometry = wall_geometry(doc).map_err(GltfError::Geometry)?;
    let profiles = wall_profiles(doc).map_err(GltfError::Geometry)?;
    let (mut walls, mut openings) = (Vec::new(), Vec::new());
    for (id, g) in &geometry {
        let (Some(w), Some(p)) = (doc.walls.get(id), profiles.get(id)) else {
            continue;
        };
        let along_x = matches!(w.direction, Direction::PosX | Direction::NegX);
        let plan: Vec<P2> = g.footprint.iter().map(p2).collect();
        let across: Vec<i128> = plan
            .iter()
            .map(|q| if along_x { q.1 } else { q.0 })
            .collect();
        let (Some(&lo), Some(&hi)) = (across.iter().min(), across.iter().max()) else {
            continue;
        };
        let (base, top) = (
            i128::from(p.base.nanometers()),
            i128::from(p.top.nanometers()),
        );
        let mut voids: Vec<_> = p
            .voids
            .iter()
            .map(|v| {
                let (s, e) = (
                    i128::from(v.start.nanometers()),
                    i128::from(v.end.nanometers()),
                );
                (s.min(e), s.max(e), v)
            })
            .collect();
        voids.sort_by_key(|v| (v.0, v.1, v.2.opening));
        let mut e = Element::new(*id, "wall", w.label.to_string());
        let mut from = None;
        for &(a, b, v) in &voids {
            let sill = i128::from(v.sill.nanometers()).clamp(base, top);
            let head = i128::from(v.head.nanometers()).clamp(base, top);
            e.faces
                .extend(extrude(&clip(&plan, along_x, from, Some(a)), base, top));
            let span = clip(&plan, along_x, Some(a), Some(b));
            e.faces.extend(extrude(&span, base, sill));
            e.faces.extend(extrude(&span, head, top));
            from = Some(from.map_or(b, |f: i128| f.max(b)));
            let Some(o) = doc.openings.get(&v.opening) else {
                continue;
            };
            let (kind, t) = match o.kind {
                OpeningKind::Door => ("door", DOOR_THICKNESS),
                OpeningKind::Window => ("window", WINDOW_THICKNESS),
                OpeningKind::Opening => continue,
            };
            let c0 = lo + (hi - lo - i128::from(t)) / 2;
            let c1 = c0 + i128::from(t);
            let panel = if along_x {
                rect(a, c0, b, c1)
            } else {
                rect(c0, a, c1, b)
            };
            let mut d = Element::new(v.opening, kind, format!("{kind} {}", v.opening));
            d.extras.insert("host".into(), json!(id.to_string()));
            d.faces = extrude(&panel, sill, head);
            openings.push(d);
        }
        e.faces
            .extend(extrude(&clip(&plan, along_x, from, None), base, top));
        walls.push(e);
    }
    openings.sort_by_key(|o| o.id);
    Ok((walls, openings))
}

fn slabs_and_ceilings(doc: &Document) -> Result<Vec<Element>, GltfError> {
    let mut out = Vec::new();
    for (id, s) in &doc.slabs {
        let outline = boundary_outline(doc, &s.boundary).map_err(GltfError::Space)?;
        let plan: Vec<P2> = outline.iter().map(p2).collect();
        let top = storey_elevation(doc, s.storey)? + i128::from(s.elevation.value.nanometers());
        let mut e = Element::new(*id, "slab", format!("slab {id}"));
        e.faces = extrude(&plan, top - i128::from(s.thickness.value.nanometers()), top);
        out.push(e);
    }
    for (id, c) in &doc.ceilings {
        let outline = boundary_outline(doc, &c.boundary).map_err(GltfError::Space)?;
        let plan: Vec<P2> = outline.iter().map(p2).collect();
        let bottom = storey_elevation(doc, c.storey)? + i128::from(c.elevation.value.nanometers());
        let mut e = Element::new(*id, "ceiling", format!("ceiling {id}"));
        e.faces = extrude(
            &plan,
            bottom,
            bottom + i128::from(c.thickness.value.nanometers()),
        );
        out.push(e);
    }
    Ok(out)
}

fn roofs(doc: &Document) -> Result<Vec<Element>, GltfError> {
    let mut out = Vec::new();
    for (id, r) in &doc.roofs {
        let g = roof_geometry(doc, *id).map_err(|e| GltfError::Roof(*id, e))?;
        let low = g.low_height.ok_or(GltfError::RoofWithoutLowHeight(*id))?;
        let outline = boundary_outline(doc, &r.footprint).map_err(GltfError::Space)?;
        let Some(plan) = clean(&outline.iter().map(p2).collect::<Vec<_>>()) else {
            continue;
        };
        // Each edge moves outward by its facing side's overhang; edges of a
        // counterclockwise rectilinear polygon face right of their direction.
        let o = |l: &archviz_core::Sourced<archviz_core::Length>| i128::from(l.value.nanometers());
        let n = plan.len();
        let offsets: Vec<(bool, i128)> = (0..n)
            .map(|i| {
                let (p, q) = (plan[i], plan[(i + 1) % n]);
                if p.1 == q.1 {
                    // Horizontal: +X faces -Y, -X faces +Y.
                    if q.0 > p.0 {
                        (true, p.1 - o(&r.overhang.neg_y))
                    } else {
                        (true, p.1 + o(&r.overhang.pos_y))
                    }
                } else if q.1 > p.1 {
                    (false, p.0 + o(&r.overhang.pos_x))
                } else {
                    (false, p.0 - o(&r.overhang.neg_x))
                }
            })
            .collect();
        let grown: Vec<P2> = (0..n)
            .map(|i| {
                let (prev, next) = (offsets[(i + n - 1) % n], offsets[i]);
                let (mut x, mut y) = plan[i];
                for (horizontal, v) in [prev, next] {
                    if horizontal {
                        y = v;
                    } else {
                        x = v;
                    }
                }
                (x, y)
            })
            .collect();
        let Some(grown) = clean(&grown) else {
            continue;
        };
        let level = storey_elevation(doc, r.storey)?;
        let (high_edge, low_edge) = (
            i128::from(g.high_edge.nanometers()),
            i128::from(g.low_edge.nanometers()),
        );
        let (hh, lh) = (
            i128::from(g.high_height.nanometers()),
            i128::from(low.nanometers()),
        );
        let axis = match g.slope_direction {
            Direction::PosX | Direction::NegX => Axis::X,
            Direction::PosY | Direction::NegY => Axis::Y,
        };
        let (mut num_scale, mut den) = (lh - hh, low_edge - high_edge);
        if den < 0 {
            num_scale = -num_scale;
            den = -den;
        }
        if den == 0 {
            return Err(GltfError::Roof(*id, RoofProblem::NoRun));
        }
        let top = move |p: P2| {
            let s = match axis {
                Axis::X => p.0,
                Axis::Y => p.1,
            };
            level + hh + div_round(num_scale * (s - high_edge), den)
        };
        let t = i128::from(r.thickness.value.nanometers());
        let mut e = Element::new(*id, "roof", format!("roof {id}"));
        e.faces = prism(&grown, |p| top(p) - t, top);
        out.push(e);
    }
    Ok(out)
}

fn equipment(doc: &Document) -> Result<Vec<Element>, GltfError> {
    let mut out = Vec::new();
    for (id, o) in &doc.occurrences {
        let f = footprint(doc, *id).map_err(GltfError::Geometry)?;
        let (z0, z1) = height_range(doc, *id).map_err(GltfError::Geometry)?;
        let (a, b) = (p2(&f.min), p2(&f.max));
        let mut e = Element::new(*id, "occurrence", o.label.to_string());
        e.extras
            .insert("element_type".into(), json!(o.element_type.to_string()));
        if let Some(model) = doc.element_types.get(&o.element_type).and_then(|t| t.model) {
            e.extras.insert("model".into(), json!(model.to_string()));
        }
        e.faces = extrude(
            &rect(a.0, a.1, b.0, b.1),
            i128::from(z0.nanometers()),
            i128::from(z1.nanometers()),
        );
        out.push(e);
    }
    Ok(out)
}

/// World nanometers to glTF meters: (x, z, -y).
fn to_gltf(p: P3) -> Result<[f64; 3], GltfError> {
    let f = |v: i128| {
        i64::try_from(v)
            .map(|v| v as f64 / NM_PER_M)
            .map_err(|_| GltfError::Overflow)
    };
    Ok([f(p[0])?, f(p[2])?, f(-p[1])?])
}

/// Appends `bytes` as a buffer view; returns its index.
fn view(bin: &mut Vec<u8>, views: &mut Vec<Value>, bytes: &[u8], target: u32) -> usize {
    views.push(json!({
        "buffer": 0,
        "byteOffset": bin.len(),
        "byteLength": bytes.len(),
        "target": target,
    }));
    bin.extend_from_slice(bytes);
    views.len() - 1
}

/// The GLB bytes for `document`. The same document gives the same bytes.
pub fn write_glb(document: &Document) -> Result<Vec<u8>, GltfError> {
    let mut elements = fiducials(document);
    let (walls, openings) = walls_and_openings(document)?;
    elements.extend(walls);
    elements.extend(openings);
    elements.extend(slabs_and_ceilings(document)?);
    elements.extend(roofs(document)?);
    elements.extend(equipment(document)?);

    let (mut bin, mut views, mut accessors, mut meshes, mut nodes) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for e in &elements {
        let mut extras = e.extras.clone();
        extras.insert("id".into(), json!(e.id.to_string()));
        extras.insert("kind".into(), json!(e.kind));
        let mut node = json!({ "name": e.name, "extras": extras });
        let origin = if e.faces.is_empty() {
            e.at
        } else {
            let mut m = e.faces[0][0];
            for p in e.faces.iter().flatten() {
                for k in 0..3 {
                    m[k] = m[k].min(p[k]);
                }
            }
            m
        };
        node["translation"] = json!(to_gltf(origin)?);
        if !e.faces.is_empty() {
            let (mut positions, mut normals, mut indices) = (
                Vec::<[f32; 3]>::new(),
                Vec::<[f32; 3]>::new(),
                Vec::<u32>::new(),
            );
            for face in &e.faces {
                let n = newell(face);
                let k = (0..3)
                    .max_by_key(|&k| (n[k].abs(), std::cmp::Reverse(k)))
                    .unwrap_or(2);
                let (u, v) = ((k + 1) % 3, (k + 2) % 3);
                let flat: Vec<P2> = face.iter().map(|p| (p[u], p[v])).collect();
                let tris = triangulate(&flat).ok_or(GltfError::Triangulation(e.id))?;
                let g = [n[0] as f64, n[2] as f64, -(n[1] as f64)];
                let len = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt();
                let normal = g.map(|c| (c / len) as f32);
                let first = u32::try_from(positions.len()).map_err(|_| GltfError::Overflow)?;
                for p in face {
                    let local = [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]];
                    positions.push(to_gltf(local)?.map(|c| c as f32));
                    normals.push(normal);
                }
                for t in tris {
                    for i in t {
                        let i = u32::try_from(i).map_err(|_| GltfError::Overflow)?;
                        indices.push(first + i);
                    }
                }
            }
            let mut min = [f32::INFINITY; 3];
            let mut max = [f32::NEG_INFINITY; 3];
            for p in &positions {
                for k in 0..3 {
                    min[k] = min[k].min(p[k]);
                    max[k] = max[k].max(p[k]);
                }
            }
            let bytes3 = |v: &[[f32; 3]]| -> Vec<u8> {
                v.iter().flatten().flat_map(|c| c.to_le_bytes()).collect()
            };
            let pv = view(&mut bin, &mut views, &bytes3(&positions), 34962);
            let nv = view(&mut bin, &mut views, &bytes3(&normals), 34962);
            let index_bytes: Vec<u8> = indices.iter().flat_map(|i| i.to_le_bytes()).collect();
            let iv = view(&mut bin, &mut views, &index_bytes, 34963);
            let a = accessors.len();
            accessors.push(json!({
                "bufferView": pv, "componentType": 5126, "count": positions.len(), "type": "VEC3",
                "min": min.map(f64::from), "max": max.map(f64::from),
            }));
            accessors.push(json!({
                "bufferView": nv, "componentType": 5126, "count": normals.len(), "type": "VEC3",
            }));
            accessors.push(json!({
                "bufferView": iv, "componentType": 5125, "count": indices.len(), "type": "SCALAR",
            }));
            node["mesh"] = json!(meshes.len());
            meshes.push(json!({
                "name": e.name,
                "primitives": [{ "attributes": { "POSITION": a, "NORMAL": a + 1 }, "indices": a + 2 }],
            }));
        }
        nodes.push(node);
    }
    let mut root = json!({
        "asset": { "version": "2.0", "generator": "archviz-export" },
        "scene": 0,
        "scenes": [{ "nodes": (0..nodes.len()).collect::<Vec<_>>() }],
        "nodes": nodes,
    });
    if !meshes.is_empty() {
        root["meshes"] = json!(meshes);
        root["accessors"] = json!(accessors);
        root["bufferViews"] = json!(views);
        root["buffers"] = json!([{ "byteLength": bin.len() }]);
    }
    let mut text = serde_json::to_vec(&root).map_err(|_| GltfError::Overflow)?;
    while text.len() % 4 != 0 {
        text.push(b' ');
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let chunk = |len: usize| u32::try_from(len).map_err(|_| GltfError::Overflow);
    let total = 12 + 8 + text.len() + if bin.is_empty() { 0 } else { 8 + bin.len() };
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&chunk(total)?.to_le_bytes());
    out.extend_from_slice(&chunk(text.len())?.to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&text);
    if !bin.is_empty() {
        out.extend_from_slice(&chunk(bin.len())?.to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&bin);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
