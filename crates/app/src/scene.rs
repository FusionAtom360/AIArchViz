//! What the plan view draws, picks and snaps to, read from the document and
//! core's derived geometry (T-202). Every coordinate is an exact world value
//! in nanometers; nothing here computes geometry of its own beyond reading
//! extents off core's outlines.

use archviz_core::{
    Axis, DatumGeometry, Direction, Document, Function, Placement, Point, Uuid, boundary_outline,
    footprint, wall_geometry, wall_profiles,
};

use crate::viewport::World;

/// A selectable entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Entity {
    Wall(Uuid),
    Opening(Uuid),
    Occurrence(Uuid),
    Slab(Uuid),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallShape {
    pub id: Uuid,
    pub label: String,
    pub function: Function,
    pub direction: Direction,
    pub footprint: Vec<World>,
    /// The axis the wall moves along (across its length).
    pub across: Axis,
    /// Coordinates on `across` of the left and right finish faces (6.1).
    pub left: i64,
    pub right: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpeningShape {
    pub id: Uuid,
    pub host: Uuid,
    /// The host's along axis.
    pub along: Axis,
    pub start: i64,
    pub end: i64,
    /// The void through the host, min and max corners.
    pub rect: (World, World),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OccurrenceShape {
    pub id: Uuid,
    pub label: String,
    pub rect: (World, World),
    pub center: World,
    /// For a wall-mounted item, the host's along axis, the only one it moves on.
    pub along: Option<Axis>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlabShape {
    pub id: Uuid,
    pub outline: Vec<World>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatumShape {
    pub id: Uuid,
    pub name: String,
    pub geometry: DatumGeometry,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scene {
    pub walls: Vec<WallShape>,
    pub openings: Vec<OpeningShape>,
    pub occurrences: Vec<OccurrenceShape>,
    pub slabs: Vec<SlabShape>,
    pub datums: Vec<DatumShape>,
    pub nodes: Vec<World>,
}

fn xy(p: &Point) -> World {
    (p.x.nanometers(), p.y.nanometers())
}

fn on(axis: Axis, p: World) -> i64 {
    match axis {
        Axis::X => p.0,
        Axis::Y => p.1,
    }
}

pub fn along(direction: Direction) -> Axis {
    match direction {
        Direction::PosX | Direction::NegX => Axis::X,
        Direction::PosY | Direction::NegY => Axis::Y,
    }
}

pub fn across(direction: Direction) -> Axis {
    match along(direction) {
        Axis::X => Axis::Y,
        Axis::Y => Axis::X,
    }
}

/// Whether the left face (looking from node_a to node_b) is on the positive
/// side of the across axis: +X walls have +Y on their left (5.4, 6.1).
fn left_is_positive(direction: Direction) -> bool {
    matches!(direction, Direction::PosX | Direction::NegY)
}

fn extent(points: &[World], axis: Axis) -> Option<(i64, i64)> {
    let values = points.iter().map(|p| on(axis, *p));
    Some((values.clone().min()?, values.max()?))
}

impl Scene {
    /// Reads the document. Fails only where core's own geometry fails.
    pub fn build(doc: &Document) -> Result<Scene, String> {
        let mut scene = Scene::default();
        let geometry = wall_geometry(doc).map_err(|e| e.to_string())?;
        for (id, g) in &geometry {
            let Some(w) = doc.walls.get(id) else { continue };
            let footprint: Vec<World> = g.footprint.iter().map(xy).collect();
            let across = across(w.direction);
            let Some((lo, hi)) = extent(&footprint, across) else {
                continue;
            };
            let (left, right) = if left_is_positive(w.direction) {
                (hi, lo)
            } else {
                (lo, hi)
            };
            scene.walls.push(WallShape {
                id: *id,
                label: w.label.to_string(),
                function: w.function,
                direction: w.direction,
                footprint,
                across,
                left,
                right,
            });
        }
        let profiles = wall_profiles(doc).map_err(|e| e.to_string())?;
        for (wall_id, p) in &profiles {
            let Some(wall) = scene.walls.iter().find(|w| w.id == *wall_id) else {
                continue;
            };
            let (lo, hi) = (wall.left.min(wall.right), wall.left.max(wall.right));
            let along = along(wall.direction);
            for v in &p.voids {
                let (a, b) = (
                    v.start.nanometers().min(v.end.nanometers()),
                    v.start.nanometers().max(v.end.nanometers()),
                );
                let rect = match along {
                    Axis::X => ((a, lo), (b, hi)),
                    Axis::Y => ((lo, a), (hi, b)),
                };
                scene.openings.push(OpeningShape {
                    id: v.opening,
                    host: *wall_id,
                    along,
                    start: a,
                    end: b,
                    rect,
                });
            }
        }
        scene.openings.sort_by_key(|o| o.id);
        for (id, slab) in &doc.slabs {
            let outline = boundary_outline(doc, &slab.boundary).map_err(|e| e.to_string())?;
            scene.slabs.push(SlabShape {
                id: *id,
                outline: outline.iter().map(xy).collect(),
            });
        }
        for (id, o) in &doc.occurrences {
            let r = footprint(doc, *id).map_err(|e| e.to_string())?;
            let along = match &o.placement {
                Placement::WallMounted { wall, .. } => {
                    doc.walls.get(wall).map(|w| along(w.direction))
                }
                _ => None,
            };
            scene.occurrences.push(OccurrenceShape {
                id: *id,
                label: o.label.to_string(),
                rect: (xy(&r.min), xy(&r.max)),
                center: xy(&o.center),
                along,
            });
        }
        scene.datums = doc
            .datums
            .iter()
            .map(|(id, d)| DatumShape {
                id: *id,
                name: d.name.clone(),
                geometry: d.geometry,
            })
            .collect();
        scene.nodes = doc
            .nodes
            .values()
            .map(|n| (n.x.nanometers(), n.y.nanometers()))
            .collect();
        Ok(scene)
    }

    /// The extents of everything drawn, for "fit".
    pub fn bounds(&self) -> Option<(World, World)> {
        let points = self
            .walls
            .iter()
            .flat_map(|w| w.footprint.iter().copied())
            .chain(self.slabs.iter().flat_map(|s| s.outline.iter().copied()))
            .chain(self.occurrences.iter().flat_map(|o| [o.rect.0, o.rect.1]))
            .chain(self.datums.iter().filter_map(|d| match d.geometry {
                DatumGeometry::Point { x, y } => Some((x.nanometers(), y.nanometers())),
                _ => None,
            }));
        points.fold(None, |acc, (x, y)| {
            Some(match acc {
                None => ((x, y), (x, y)),
                Some(((x0, y0), (x1, y1))) => ((x0.min(x), y0.min(y)), (x1.max(x), y1.max(y))),
            })
        })
    }

    /// The entity under `p`, within `tolerance` nanometers: occurrences
    /// first, then openings, walls and slabs, as they are drawn top down.
    pub fn pick(&self, p: World, tolerance: i64) -> Option<Entity> {
        let in_rect = |r: &(World, World)| {
            near_range(p.0, r.0.0, r.1.0, tolerance) && near_range(p.1, r.0.1, r.1.1, tolerance)
        };
        if let Some(o) = self.occurrences.iter().rev().find(|o| in_rect(&o.rect)) {
            return Some(Entity::Occurrence(o.id));
        }
        if let Some(o) = self.openings.iter().find(|o| in_rect(&o.rect)) {
            return Some(Entity::Opening(o.id));
        }
        if let Some(w) = self.wall_at(p, tolerance) {
            return Some(Entity::Wall(w.id));
        }
        self.slabs
            .iter()
            .find(|s| inside(&s.outline, p) || near_outline(&s.outline, p, tolerance))
            .map(|s| Entity::Slab(s.id))
    }

    /// The wall whose footprint holds `p` or lies within `tolerance` of it.
    pub fn wall_at(&self, p: World, tolerance: i64) -> Option<&WallShape> {
        self.walls
            .iter()
            .find(|w| inside(&w.footprint, p) || near_outline(&w.footprint, p, tolerance))
    }

    /// Snap references on `axis`, (coordinate, description), leaving out
    /// those of `exclude`.
    pub fn references(&self, axis: Axis, exclude: Option<Entity>) -> Vec<(i64, String)> {
        let mut out = Vec::new();
        for w in &self.walls {
            if exclude == Some(Entity::Wall(w.id)) {
                continue;
            }
            if w.across == axis {
                out.push((w.left, format!("{} left face", w.label)));
                out.push((w.right, format!("{} right face", w.label)));
                let sum = i128::from(w.left) + i128::from(w.right);
                if sum % 2 == 0
                    && let Ok(c) = i64::try_from(sum / 2)
                {
                    out.push((c, format!("{} center", w.label)));
                }
            }
        }
        for o in &self.openings {
            if exclude == Some(Entity::Opening(o.id)) || o.along != axis {
                continue;
            }
            let host = self
                .walls
                .iter()
                .find(|w| w.id == o.host)
                .map_or(String::new(), |w| w.label.clone());
            out.push((o.start, format!("opening start in {host}")));
            out.push((o.end, format!("opening end in {host}")));
        }
        for o in &self.occurrences {
            if exclude == Some(Entity::Occurrence(o.id)) {
                continue;
            }
            out.push((on(axis, o.rect.0), format!("{} edge", o.label)));
            out.push((on(axis, o.rect.1), format!("{} edge", o.label)));
            out.push((on(axis, o.center), format!("{} center", o.label)));
        }
        for d in &self.datums {
            let v = match (d.geometry, axis) {
                (DatumGeometry::Point { x, .. } | DatumGeometry::XLine { x }, Axis::X) => Some(x),
                (DatumGeometry::Point { y, .. } | DatumGeometry::YLine { y }, Axis::Y) => Some(y),
                _ => None,
            };
            if let Some(v) = v {
                out.push((v.nanometers(), format!("datum \"{}\"", d.name)));
            }
        }
        out.sort();
        out
    }
}

fn near_range(v: i64, lo: i64, hi: i64, tolerance: i64) -> bool {
    let (v, lo, hi, t) = (
        i128::from(v),
        i128::from(lo.min(hi)),
        i128::from(lo.max(hi)),
        i128::from(tolerance.max(0)),
    );
    v >= lo - t && v <= hi + t
}

/// Even-odd point in polygon, exact in i128.
fn inside(poly: &[World], p: World) -> bool {
    let (px, py) = (i128::from(p.0), i128::from(p.1));
    let mut odd = false;
    let n = poly.len();
    for (i, a) in poly.iter().enumerate() {
        let Some(b) = poly.get((i + n - 1) % n.max(1)) else {
            continue;
        };
        let (ax, ay, bx, by) = (
            i128::from(a.0),
            i128::from(a.1),
            i128::from(b.0),
            i128::from(b.1),
        );
        if (ay > py) != (by > py) {
            // x of the edge at py, compared without division.
            let lhs = (px - ax) * (by - ay);
            let rhs = (bx - ax) * (py - ay);
            if (by > ay && lhs < rhs) || (by < ay && lhs > rhs) {
                odd = !odd;
            }
        }
    }
    odd
}

/// Whether `p` lies within `tolerance` of an orthogonal outline's edges.
fn near_outline(poly: &[World], p: World, tolerance: i64) -> bool {
    let n = poly.len();
    poly.iter().enumerate().any(|(i, a)| {
        let Some(b) = poly.get((i + 1) % n.max(1)) else {
            return false;
        };
        near_range(p.0, a.0, b.0, tolerance) && near_range(p.1, a.1, b.1, tolerance)
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use archviz_core::{Reference, WallSide, reference_coordinate};

    /// The shop slice fixture replayed into a fresh folder, and its document.
    pub(crate) fn shop(tag: &str) -> (std::path::PathBuf, Document) {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let base =
            std::env::temp_dir().join(format!("archviz-app-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("shop");
        let script = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/shop/commands.txt"),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        for line in script.lines() {
            let (sub, json) = line.split_once(' ').unwrap_or((line, ""));
            let args = [sub.to_string(), dir.display().to_string()];
            let (out, status) = archviz_cli::run(&args, json, &mut crate::session::System);
            assert_eq!(status, 0, "{sub}: {out}");
        }
        let project = archviz_cli::store::open(&dir).unwrap_or_else(|e| panic!("{e:?}"));
        let doc = project.history.document().clone();
        (dir, doc)
    }

    #[test]
    fn faces_and_extents_match_core_references() {
        let (_, doc) = shop("scene-faces");
        let scene = Scene::build(&doc).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(scene.walls.len(), doc.walls.len());
        assert!(!scene.openings.is_empty() && !scene.occurrences.is_empty());
        assert_eq!(scene.slabs.len(), doc.slabs.len());
        for w in &scene.walls {
            for (side, want) in [(WallSide::Left, w.left), (WallSide::Right, w.right)] {
                let r = Reference::WallFace {
                    wall: w.id,
                    side,
                    boundary: 0,
                };
                let got = reference_coordinate(&doc, &r, w.across)
                    .unwrap_or_else(|e| panic!("{e:?}"))
                    .nanometers();
                assert_eq!(got, want, "{} {side:?}", w.label);
            }
        }
    }

    #[test]
    fn picking_finds_each_kind_and_prefers_what_is_on_top() {
        let (_, doc) = shop("scene-pick");
        let scene = Scene::build(&doc).unwrap_or_else(|e| panic!("{e}"));
        let w = &scene.walls[0];
        let mid = |r: &(World, World)| ((r.0.0 + r.1.0) / 2, (r.0.1 + r.1.1) / 2);
        let (lo, hi) = extent(&w.footprint, along(w.direction)).unwrap_or((0, 0));
        let a = (lo + hi) / 2;
        let c = (w.left + w.right) / 2;
        let p = match w.across {
            Axis::Y => (a, c),
            Axis::X => (c, a),
        };
        let expected = scene
            .openings
            .iter()
            .find(|o| o.host == w.id && near_range(a, o.start, o.end, 0))
            .map_or(Entity::Wall(w.id), |o| Entity::Opening(o.id));
        assert_eq!(scene.pick(p, 0), Some(expected));
        let o = &scene.openings[0];
        assert_eq!(scene.pick(mid(&o.rect), 0), Some(Entity::Opening(o.id)));
        let occ = &scene.occurrences[0];
        assert_eq!(scene.pick(occ.center, 0), Some(Entity::Occurrence(occ.id)));
        let ((x0, y0), _) = scene.bounds().unwrap_or(((0, 0), (0, 0)));
        assert_eq!(scene.pick((x0 - 1_000_000_000, y0), 1000), None);
    }

    #[test]
    fn references_leave_out_the_dragged_entity_and_cover_datums() {
        let (_, doc) = shop("scene-refs");
        let scene = Scene::build(&doc).unwrap_or_else(|e| panic!("{e}"));
        let w = &scene.walls[0];
        let all = scene.references(w.across, None);
        let without = scene.references(w.across, Some(Entity::Wall(w.id)));
        assert!(all.iter().any(|(_, n)| n.starts_with(&w.label)));
        assert!(
            !without
                .iter()
                .any(|(_, n)| n.starts_with(&format!("{} ", w.label)))
        );
        assert!(
            all.iter().any(|(v, n)| *v == 0 && n.starts_with("datum")),
            "{all:?}"
        );
    }

    #[test]
    fn point_in_polygon_is_exact() {
        let l = vec![(0, 0), (10, 0), (10, 4), (4, 4), (4, 10), (0, 10)];
        assert!(inside(&l, (2, 8)));
        assert!(inside(&l, (8, 2)));
        assert!(!inside(&l, (8, 8)));
        assert!(!inside(&l, (-1, 2)));
        let big = vec![
            (i64::MIN, i64::MIN),
            (i64::MAX, i64::MIN),
            (i64::MAX, i64::MAX),
            (i64::MIN, i64::MAX),
        ];
        assert!(inside(&big, (0, 0)));
    }
}
