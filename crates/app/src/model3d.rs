//! What the 3D viewer draws, picks and measures (T-206, 13.1): the export
//! crate's exact solids (the same faces the GLB carries), cut by an
//! optional section plane.
//!
//! Every vertex is an exact world point in nanometers. A pick finds the
//! nearest triangle along the pointer's ray in floating point, then snaps to
//! the vertex of that element nearest the pointer on screen, so a measurement
//! is always between two exact vertices and its length is computed in
//! integers: exact when the distance is a whole number of nanometers, and
//! marked `~` when it is not (5.3).

use std::collections::BTreeSet;

use archviz_core::{Document, Length, Uuid, format_length};

use crate::camera::{Camera, P3, V3, cross, dot};

/// One element: its triangles and the edges of its faces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: Uuid,
    /// wall, door, window, slab, ceiling, roof or occurrence.
    pub kind: &'static str,
    pub name: String,
    pub triangles: Vec<[P3; 3]>,
    /// Face outlines, each edge once.
    pub edges: Vec<[P3; 2]>,
    /// Where the section plane cuts the element's faces.
    pub cuts: Vec<[P3; 2]>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Model {
    pub items: Vec<Item>,
}

/// An axis-aligned section plane: everything on the far side of `at` on
/// `axis` (0 = X, 1 = Y, 2 = Z) is cut away; `keep_below` keeps the side
/// with smaller coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section {
    pub axis: usize,
    pub at: i64,
    pub keep_below: bool,
}

impl Section {
    fn side(&self, p: &P3) -> i128 {
        let d = p[self.axis] - i128::from(self.at);
        if self.keep_below { d } else { -d }
    }

    fn keeps(&self, p: &P3) -> bool {
        self.side(p) <= 0
    }

    /// Where segment p-q crosses the plane; the plane's coordinate is
    /// exact, the others are rounded half to even to the nanometer.
    fn crossing(&self, p: &P3, q: &P3) -> P3 {
        let (sp, sq) = (self.side(p), self.side(q));
        let mut out = *p;
        for k in 0..3 {
            out[k] = if k == self.axis {
                i128::from(self.at)
            } else {
                // (p (sp - sq) + (q - p) sp) / (sp - sq), sp and sq of
                // opposite signs; the whole coordinate is rounded, so the
                // crossing is the same from either end.
                let (mut n, mut d) = (p[k] * (sp - sq) + (q[k] - p[k]) * sp, sp - sq);
                if d < 0 {
                    n = -n;
                    d = -d;
                }
                div_round(n, d)
            };
        }
        out
    }
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

fn edge(a: P3, b: P3) -> [P3; 2] {
    if a <= b { [a, b] } else { [b, a] }
}

/// Where the pointer's ray meets the model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// Index into `Model::items`.
    pub item: usize,
    /// Along the ray from the eye, nanometers.
    pub depth: f64,
}

impl Model {
    pub fn build(doc: &Document) -> Result<Model, String> {
        let solids = archviz_export::solids(doc).map_err(|e| e.to_string())?;
        let items = solids
            .into_iter()
            .map(|s| {
                let mut triangles = Vec::new();
                let mut edges = BTreeSet::new();
                for f in &s.faces {
                    for t in &f.triangles {
                        let [a, b, c] = t.map(|i| f.points.get(i).copied().unwrap_or([0; 3]));
                        triangles.push([a, b, c]);
                    }
                    let n = f.points.len();
                    for (i, p) in f.points.iter().enumerate() {
                        if let Some(q) = f.points.get((i + 1) % n)
                            && p != q
                        {
                            edges.insert(edge(*p, *q));
                        }
                    }
                }
                Item {
                    id: s.id,
                    kind: s.kind,
                    name: s.name,
                    triangles,
                    edges: edges.into_iter().collect(),
                    cuts: Vec::new(),
                }
            })
            .collect();
        Ok(Model { items })
    }

    /// The box around every vertex.
    pub fn bounds(&self) -> Option<(P3, P3)> {
        let mut points = self.items.iter().flat_map(|i| i.triangles.iter().flatten());
        let first = *points.next()?;
        Some(points.fold((first, first), |(mut lo, mut hi), p| {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
            (lo, hi)
        }))
    }

    /// The model with everything beyond the section plane cut away, and the
    /// lines where the plane cuts each face.
    pub fn cut(&self, section: &Section) -> Model {
        let items = self
            .items
            .iter()
            .map(|item| {
                let mut triangles = Vec::new();
                let mut cuts = BTreeSet::new();
                for t in &item.triangles {
                    let mut poly: Vec<P3> = Vec::with_capacity(4);
                    let mut crossings = Vec::with_capacity(2);
                    for i in 0..3 {
                        let (p, q) = (&t[i], &t[(i + 1) % 3]);
                        if section.keeps(p) {
                            poly.push(*p);
                        }
                        if section.keeps(p) != section.keeps(q) {
                            let x = section.crossing(p, q);
                            poly.push(x);
                            crossings.push(x);
                        }
                    }
                    if let [a, b] = crossings[..]
                        && a != b
                    {
                        cuts.insert(edge(a, b));
                    }
                    // A fan over at most four points keeps the winding.
                    for i in 1..poly.len().saturating_sub(1) {
                        let tri = [poly[0], poly[i], poly[i + 1]];
                        if tri[0] != tri[1] && tri[1] != tri[2] && tri[0] != tri[2] {
                            triangles.push(tri);
                        }
                    }
                }
                let edges = item
                    .edges
                    .iter()
                    .filter_map(|[p, q]| match (section.keeps(p), section.keeps(q)) {
                        (true, true) => Some([*p, *q]),
                        (true, false) => Some(edge(*p, section.crossing(p, q))),
                        (false, true) => Some(edge(section.crossing(p, q), *q)),
                        (false, false) => None,
                    })
                    .filter(|[p, q]| p != q)
                    .collect();
                Item {
                    id: item.id,
                    kind: item.kind,
                    name: item.name.clone(),
                    triangles,
                    edges,
                    cuts: cuts.into_iter().collect(),
                }
            })
            .collect();
        Model { items }
    }

    /// The nearest triangle the ray through pixel `at` meets in front of
    /// the near plane.
    pub fn pick(&self, camera: &Camera, at: [f64; 2], size: [f64; 2]) -> Option<Hit> {
        let d = camera.ray(at, size);
        let near = camera.near();
        let mut best: Option<Hit> = None;
        for (index, item) in self.items.iter().enumerate() {
            for t in &item.triangles {
                let [a, b, c] = t.map(|p| camera.relative(p));
                if let Some(depth) = intersect(d, a, b, c)
                    && depth >= near
                    && best.is_none_or(|h| depth < h.depth)
                {
                    best = Some(Hit { item: index, depth });
                }
            }
        }
        best
    }

    /// The vertex of item `index` nearest pixel `at` on screen.
    pub fn snap(&self, camera: &Camera, index: usize, at: [f64; 2], size: [f64; 2]) -> Option<P3> {
        let item = self.items.get(index)?;
        let distance = |p: &P3| {
            camera
                .project(*p, size)
                .map(|s| (s[0] - at[0]).powi(2) + (s[1] - at[1]).powi(2))
        };
        item.triangles
            .iter()
            .flatten()
            .filter_map(|p| distance(p).map(|d| (d, *p)))
            .min_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)))
            .map(|(_, p)| p)
    }

    /// The element under pixel `at` and its vertex nearest the pointer.
    pub fn pick_vertex(
        &self,
        camera: &Camera,
        at: [f64; 2],
        size: [f64; 2],
    ) -> Option<(&Item, P3)> {
        let hit = self.pick(camera, at, size)?;
        let p = self.snap(camera, hit.item, at, size)?;
        Some((self.items.get(hit.item)?, p))
    }
}

/// Möller-Trumbore: the distance along unit `d` from the origin to
/// triangle (a, b, c), both faces.
fn intersect(d: V3, a: V3, b: V3, c: V3) -> Option<f64> {
    let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let p = cross(d, e2);
    let det = dot(e1, p);
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let s = [-a[0], -a[1], -a[2]];
    let u = dot(s, p) / det;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let v = dot(d, q) / det;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    Some(dot(e2, q) / det)
}

/// A point-to-point measurement between two exact vertices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Measure {
    pub a: P3,
    pub b: P3,
}

/// A length to the nanometer and whether it is exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Distance {
    pub length: Length,
    pub exact: bool,
}

impl Distance {
    /// Feet-inches-fractions at 1/16 in, `~` first unless the value is
    /// exact and on the grid (5.3).
    pub fn text(&self) -> String {
        let t = format_length(self.length);
        if self.exact || t.starts_with('~') {
            t
        } else {
            format!("~{t}")
        }
    }
}

fn length(v: i128) -> Option<Length> {
    i64::try_from(v).ok().map(Length::from_nanometers)
}

impl Measure {
    /// The change on each axis, b less a.
    pub fn delta(&self) -> Option<[Length; 3]> {
        let [x, y, z] = [0, 1, 2].map(|k| length(self.b[k].checked_sub(self.a[k])?));
        Some([x?, y?, z?])
    }

    /// The straight-line distance: the integer square root of the exact sum
    /// of squares, rounded to the nearest nanometer.
    pub fn distance(&self) -> Option<Distance> {
        let mut sum: u128 = 0;
        for k in 0..3 {
            let d = self.b[k].checked_sub(self.a[k])?.unsigned_abs();
            sum = sum.checked_add(d.checked_mul(d)?)?;
        }
        let r = sum.isqrt();
        let rest = sum - r * r;
        // (r + 1/2)^2 = r^2 + r + 1/4, so round up when the rest exceeds r.
        let rounded = if rest > r { r + 1 } else { r };
        Some(Distance {
            length: length(i128::try_from(rounded).ok()?)?,
            exact: rest == 0,
        })
    }

    /// The measurement in words: distance, then each axis's change.
    pub fn text(&self) -> String {
        let (Some(d), Some(delta)) = (self.distance(), self.delta()) else {
            return "too far to measure".to_string();
        };
        format!(
            "{} (X {}, Y {}, Z {})",
            d.text(),
            format_length(delta[0]),
            format_length(delta[1]),
            format_length(delta[2])
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Mode;
    use archviz_core::{Axis, wall_geometry};

    const IN: i128 = 25_400_000;
    const FT: i128 = 12 * IN;
    const SIZE: [f64; 2] = [1280.0, 720.0];

    fn shop_model() -> (Document, Model) {
        let (_, doc) = crate::scene::tests::shop("model3d");
        let model = Model::build(&doc).unwrap_or_else(|e| panic!("{e}"));
        (doc, model)
    }

    fn fitted(model: &Model) -> Camera {
        let mut c = Camera::default();
        let (lo, hi) = model.bounds().unwrap_or_else(|| panic!("empty"));
        c.fit(lo, hi);
        c
    }

    /// The pixel of `p`, nudged 3 px `toward` another point and off the
    /// pixel grid by a fraction.
    fn pointer(c: &Camera, p: P3, toward: P3, frac: [f64; 2]) -> [f64; 2] {
        let a = c.project(p, SIZE).unwrap_or_else(|| panic!("behind"));
        let b = c.project(toward, SIZE).unwrap_or_else(|| panic!("behind"));
        let s = 3.0 / ((b[0] - a[0]).hypot(b[1] - a[1])).max(3.0);
        [
            a[0] + (b[0] - a[0]) * s + frac[0],
            a[1] + (b[1] - a[1]) * s + frac[1],
        ]
    }

    #[test]
    fn measuring_a_known_wall_length_reads_exactly() {
        let (doc, model) = shop_model();
        let geometry = wall_geometry(&doc).unwrap_or_else(|e| panic!("{e}"));
        let mut checked = 0;
        for (id, wall) in &doc.walls {
            let Some(g) = geometry.get(id) else { continue };
            // The wall's longest footprint edge, read from core: a known
            // length, and the top of the solid at its ends.
            let fp: Vec<(i128, i128)> = g
                .footprint
                .iter()
                .map(|p| (i128::from(p.x.nanometers()), i128::from(p.y.nanometers())))
                .collect();
            let n = fp.len();
            let Some((p, q)) = (0..n)
                .map(|i| (fp[i], fp[(i + 1) % n]))
                .max_by_key(|(p, q)| ((q.0 - p.0).abs() + (q.1 - p.1).abs(), *p))
            else {
                continue;
            };
            let known = (q.0 - p.0).abs() + (q.1 - p.1).abs();
            assert!(p.0 == q.0 || p.1 == q.1, "walls are axis-aligned");
            let Some(item) = model.items.iter().position(|i| i.id == *id) else {
                panic!("{} has no solid", wall.label);
            };
            let top = |xy: (i128, i128)| {
                model.items[item]
                    .triangles
                    .iter()
                    .flatten()
                    .filter(|v| (v[0], v[1]) == xy)
                    .map(|v| v[2])
                    .max()
                    .unwrap_or_else(|| panic!("{} has no vertex at {xy:?}", wall.label))
            };
            let (a, b) = ([p.0, p.1, top(p)], [q.0, q.1, top(q)]);
            // The middle of the wall's top face, to nudge the pointer into it.
            let tops: Vec<P3> = model.items[item]
                .triangles
                .iter()
                .flatten()
                .filter(|v| v[2] == a[2])
                .copied()
                .collect();
            let middle =
                [0, 1, 2].map(|k| tops.iter().map(|v| v[k]).sum::<i128>() / tops.len() as i128);
            for (orbit, frac) in [((0.0, 0.0), [0.37, -0.29]), ((140.0, 35.0), [-0.41, 0.13])] {
                // Each end framed 8 ft away, as a user zooms in to click it.
                let click = |end: P3| {
                    let mut c = fitted(&model);
                    c.drag(orbit.0, orbit.1);
                    c.look_at(end, 8.0 * FT as f64);
                    // Toward the face's middle, 6 in from the corner.
                    let span: Vec<f64> = (0..3).map(|k| (middle[k] - end[k]) as f64).collect();
                    let k = 6.0 * IN as f64 / span.iter().map(|v| v * v).sum::<f64>().sqrt();
                    let toward = [0, 1, 2].map(|i| end[i] + (span[i] * k).round() as i128);
                    model.pick_vertex(&c, pointer(&c, end, toward, frac), SIZE)
                };
                let (Some(pa), Some(pb)) = (click(a), click(b)) else {
                    continue;
                };
                if pa.0.id != *id || pb.0.id != *id {
                    continue;
                }
                let m = Measure { a: pa.1, b: pb.1 };
                assert_eq!((m.a, m.b), (a, b), "{}", wall.label);
                let d = m.distance().unwrap_or_else(|| panic!());
                assert!(d.exact, "{}", wall.label);
                assert_eq!(i128::from(d.length.nanometers()), known);
                assert_eq!(
                    d.text(),
                    format_length(Length::from_nanometers(known as i64))
                );
                assert!(!d.text().starts_with('~'), "{}", d.text());
                checked += 1;
            }
        }
        assert!(checked >= 4, "only {checked} walls measured");
    }

    #[test]
    fn a_wall_from_two_inches_to_seventy_reads_five_feet_eight() {
        // Exact vertices 68 in apart along X, one raised: the distance and
        // each axis read in feet and inches.
        let m = Measure {
            a: [2 * IN, 2 * IN, 0],
            b: [70 * IN, 2 * IN, 0],
        };
        assert_eq!(m.text(), "5'-8\" (X 5'-8\", Y 0'-0\", Z 0'-0\")");
        let m = Measure {
            a: [0, 0, 0],
            b: [3 * FT, 4 * FT, 0],
        };
        assert_eq!(m.distance().map(|d| d.text()), Some("5'-0\"".to_string()));
        // The diagonal of a 1 in square is irrational: marked ~.
        let m = Measure {
            a: [0, 0, 0],
            b: [IN, IN, 0],
        };
        let d = m.distance().unwrap_or_else(|| panic!());
        assert!(!d.exact);
        assert_eq!(d.length.nanometers(), 35_921_024); // 25.4e6 * sqrt 2 = 35,921,023.9...
        assert!(d.text().starts_with('~'), "{}", d.text());
        // Beyond i128 squares: refused, not wrapped.
        let far = Measure {
            a: [i128::MIN / 2, 0, 0],
            b: [i128::MAX / 2, 0, 0],
        };
        assert_eq!(far.distance(), None);
        assert_eq!(far.text(), "too far to measure");
    }

    #[test]
    fn a_section_keeps_one_side_and_cuts_on_the_plane() {
        let (_, model) = shop_model();
        let at = 4 * FT as i64;
        for keep_below in [true, false] {
            let s = Section {
                axis: 2,
                at,
                keep_below,
            };
            let cut = model.cut(&s);
            let mut cut_walls = 0;
            for item in &cut.items {
                for p in item
                    .triangles
                    .iter()
                    .flatten()
                    .chain(item.edges.iter().flatten())
                {
                    assert!(s.keeps(p), "{} {p:?}", item.name);
                }
                for p in item.cuts.iter().flatten() {
                    assert_eq!(p[2], i128::from(at));
                }
                if item.kind == "wall" && !item.cuts.is_empty() {
                    cut_walls += 1;
                }
            }
            let walls = model.items.iter().filter(|i| i.kind == "wall").count();
            assert_eq!(cut_walls, walls, "a plane at 4 ft cuts every wall");
        }
        // X and Y sections too: vertices on the kept side only.
        let (lo, hi) = model.bounds().unwrap_or_else(|| panic!());
        for axis in 0..2 {
            let mid = i64::try_from((lo[axis] + hi[axis]) / 2).unwrap_or(0);
            let s = Section {
                axis,
                at: mid,
                keep_below: false,
            };
            let cut = model.cut(&s);
            assert!(cut.items.iter().any(|i| !i.triangles.is_empty()));
            assert!(
                cut.items
                    .iter()
                    .flat_map(|i| i.triangles.iter().flatten())
                    .all(|p| p[axis] >= i128::from(mid))
            );
        }
    }

    #[test]
    fn crossings_lie_on_the_plane_and_round_half_to_even() {
        let s = Section {
            axis: Axis::X as usize,
            at: 1,
            keep_below: true,
        };
        let x = s.crossing(&[0, 0, 0], &[2, 3, 0]);
        assert_eq!(x, [1, 2, 0]); // y = 1.5, rounded half to even
        let x = s.crossing(&[0, 0, 0], &[2, 5, 0]);
        assert_eq!(x, [1, 2, 0]); // y = 2.5
        let x = s.crossing(&[2, 5, 0], &[0, 0, 0]);
        assert_eq!(x, [1, 2, 0]); // the same from the other end
    }

    #[test]
    fn picking_finds_the_front_element_and_a_section_lets_you_see_inside() {
        let (_, model) = shop_model();
        let mut c = fitted(&model);
        let center = [SIZE[0] / 2.0, SIZE[1] / 2.0];
        let hit = model
            .pick(&c, center, SIZE)
            .unwrap_or_else(|| panic!("nothing under the center"));
        // Cut a foot below where the ray met the model, keeping what is
        // below: what is hit now lies farther along the ray, or nothing is.
        let d = c.ray(center, SIZE);
        let z = c.pose().z.nanometers() as f64 + d[2] * hit.depth;
        let cut = model.cut(&Section {
            axis: 2,
            at: (z - FT as f64).round() as i64,
            keep_below: true,
        });
        let kind = model.items[hit.item].kind;
        assert!(
            cut.pick(&c, center, SIZE)
                .is_none_or(|h| h.depth > hit.depth),
            "{kind}"
        );
        // Walk mode: nothing behind the eye is picked.
        c.mode = Mode::Walk;
        c.drag(0.0, 2e6);
        assert!(cut.pick(&c, center, SIZE).is_none_or(|h| h.depth > 0.0));
    }

    #[test]
    fn every_wall_door_and_slab_has_a_solid() {
        let (doc, model) = shop_model();
        for id in doc.walls.keys().chain(doc.slabs.keys()) {
            assert!(model.items.iter().any(|i| i.id == *id));
        }
        assert!(
            model
                .items
                .iter()
                .all(|i| !i.triangles.is_empty() && !i.edges.is_empty())
        );
    }
}
