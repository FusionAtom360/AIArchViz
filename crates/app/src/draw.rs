//! The plan-view wall tool (T-202b, spec 3.1, 5.3, 6.1, 6.4; Q-78 (a)).
//!
//! Each click becomes a point snapped to the 1/16 in grid or to an on-grid
//! reference (snap.rs). After the first point a click moves along one axis
//! only, the one the pointer has moved farther on from the last point, so
//! every wall is axis-aligned. A click on the run's first point closes it;
//! finishing leaves it open. The run becomes one proposal: a node at each
//! point and a catalog wall between consecutive points, its layers Proposed
//! (6.4). The node positions are placeholders until dimensions drive them
//! (T-203). The tool computes no geometry: core joins the walls (6.1).
//!
//! On first use in a project with no storey or no datum anchoring each axis,
//! the tool proposes a storey and an origin datum first, as one proposal.

use archviz_core::{
    Action, Axis, CEILING_HEIGHT, Command, CreateDatum, CreateNode, CreateWall, DatumGeometry,
    Direction, Document, FLOOR_TO_FLOOR, Function, Length, SetStorey, Uuid, proposed,
};

use crate::scene::Scene;
use crate::snap::snap;
use crate::viewport::World;

/// A drawn run of points, ready to become walls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub points: Vec<World>,
    /// The last point joins back to the first.
    pub closed: bool,
}

/// What a click did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Click {
    Added,
    /// The click landed on the first point: the run is closed.
    Closed(Run),
    /// The click was refused, and why.
    Ignored(&'static str),
}

/// The run being drawn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WallTool {
    points: Vec<World>,
}

fn on(axis: Axis, p: World) -> i64 {
    match axis {
        Axis::X => p.0,
        Axis::Y => p.1,
    }
}

/// The axis and sign of the segment from `a` to `b`, axis-aligned and of
/// nonzero length.
fn heading(a: World, b: World) -> Option<(Axis, bool)> {
    match (a.0 == b.0, a.1 == b.1) {
        (false, true) => Some((Axis::X, b.0 > a.0)),
        (true, false) => Some((Axis::Y, b.1 > a.1)),
        _ => None,
    }
}

/// Whether the segment `b` runs straight back along `a`.
fn doubles_back(a: Option<(Axis, bool)>, b: Option<(Axis, bool)>) -> bool {
    matches!((a, b), (Some((x, s)), Some((y, t))) if x == y && s != t)
}

impl WallTool {
    pub fn new() -> WallTool {
        WallTool::default()
    }

    /// The points placed so far.
    pub fn points(&self) -> &[World] {
        &self.points
    }

    pub fn cancel(&mut self) {
        self.points.clear();
    }

    /// Where a click at `at` would place the next point: snapped to an
    /// on-grid reference within `tolerance` nanometers (the scene's, or a
    /// point of this run), else to the grid, and after the first point
    /// moved along one axis only.
    pub fn next_point(&self, scene: &Scene, at: World, tolerance: i64) -> World {
        let snapped = |axis: Axis| {
            let mut refs = scene.references(axis, None);
            for (i, p) in self.points.iter().enumerate() {
                refs.push((on(axis, *p), format!("run point {}", i + 1)));
            }
            snap(on(axis, at), &refs, tolerance).value
        };
        match self.points.last() {
            None => (snapped(Axis::X), snapped(Axis::Y)),
            Some(&last) => {
                let dx = (i128::from(at.0) - i128::from(last.0)).abs();
                let dy = (i128::from(at.1) - i128::from(last.1)).abs();
                if dy > dx {
                    (last.0, snapped(Axis::Y))
                } else {
                    (snapped(Axis::X), last.1)
                }
            }
        }
    }

    /// Places the next point for a click at `at` (see `next_point`).
    pub fn click(&mut self, scene: &Scene, at: World, tolerance: i64) -> Click {
        let p = self.next_point(scene, at, tolerance);
        let Some(&last) = self.points.last() else {
            self.points.push(p);
            return Click::Added;
        };
        let new = heading(last, p);
        if new.is_none() {
            return Click::Ignored("a wall needs a length");
        }
        let n = self.points.len();
        let before = n
            .checked_sub(2)
            .and_then(|i| self.points.get(i))
            .and_then(|&b| heading(b, last));
        if doubles_back(before, new) {
            return Click::Ignored("a wall cannot double back on the one before it");
        }
        if n >= 3 && self.points.first() == Some(&p) {
            let first = match self.points.as_slice() {
                [a, b, ..] => heading(*a, *b),
                _ => None,
            };
            if doubles_back(new, first) {
                return Click::Ignored("the last wall cannot double back on the first");
            }
            return Click::Closed(Run {
                points: std::mem::take(&mut self.points),
                closed: true,
            });
        }
        if self.points.contains(&p) {
            return Click::Ignored("the run already has a point there");
        }
        self.points.push(p);
        Click::Added
    }

    /// Ends the run open, if it has a wall.
    pub fn finish(&mut self) -> Option<Run> {
        if self.points.len() < 2 {
            return None;
        }
        Some(Run {
            points: std::mem::take(&mut self.points),
            closed: false,
        })
    }
}

/// The storey new walls go on: the lowest, ties to the lowest id.
pub fn storey_for(doc: &Document) -> Option<Uuid> {
    doc.storeys
        .iter()
        .min_by_key(|(id, s)| (s.elevation.value, **id))
        .map(|(id, _)| *id)
}

/// What the project needs before walls can be drawn: a storey if it has
/// none, and an origin datum if an axis has no datum (6). Empty when ready.
/// Every value is a catalog default, Proposed (6.4).
pub fn setup_actions(doc: &Document) -> Vec<Action> {
    let mut out = Vec::new();
    if doc.storeys.is_empty() {
        out.push(Action::SetStorey(SetStorey {
            storey: None,
            name: "Ground".to_string(),
            elevation: proposed(Length::from_nanometers(0)),
            floor_to_floor: proposed(FLOOR_TO_FLOOR),
            default_ceiling_height: proposed(CEILING_HEIGHT),
        }));
    }
    let anchors = |axis: Axis| {
        doc.datums.values().any(|d| {
            matches!(
                (d.geometry, axis),
                (DatumGeometry::Point { .. }, _)
                    | (DatumGeometry::XLine { .. }, Axis::X)
                    | (DatumGeometry::YLine { .. }, Axis::Y)
            )
        })
    };
    if !anchors(Axis::X) || !anchors(Axis::Y) {
        out.push(Action::CreateDatum(CreateDatum {
            name: "Origin".to_string(),
            geometry: DatumGeometry::Point {
                x: Length::from_nanometers(0),
                y: Length::from_nanometers(0),
            },
        }));
    }
    out
}

fn direction(a: World, b: World) -> Option<Direction> {
    Some(match heading(a, b)? {
        (Axis::X, true) => Direction::PosX,
        (Axis::X, false) => Direction::NegX,
        (Axis::Y, true) => Direction::PosY,
        (Axis::Y, false) => Direction::NegY,
    })
}

/// The run as commands: a node at each point, then a catalog wall (layers,
/// extents and heights from the catalog, Proposed) from each point to the
/// next, and from the last to the first when closed. `ids` gives each
/// command its (id, idempotency key); a node's id is derived from its key
/// as core derives it. None if a segment is not axis-aligned.
pub fn run_commands(
    run: &Run,
    storey: Uuid,
    mut ids: impl FnMut() -> (Uuid, Uuid),
) -> Option<Vec<Command>> {
    let mut commands = Vec::new();
    let mut nodes = Vec::new();
    for &(x, y) in &run.points {
        let (id, key) = ids();
        nodes.push(Uuid::derive(key, "command", "0"));
        commands.push(Command {
            id,
            idempotency_key: key,
            action: Action::CreateNode(CreateNode {
                x: Length::from_nanometers(x),
                y: Length::from_nanometers(y),
            }),
        });
    }
    let n = run.points.len();
    let walls = if run.closed { n } else { n.saturating_sub(1) };
    for i in 0..walls {
        let j = (i + 1) % n;
        let (a, b) = (*run.points.get(i)?, *run.points.get(j)?);
        let (id, key) = ids();
        commands.push(Command {
            id,
            idempotency_key: key,
            action: Action::CreateWall(CreateWall {
                storey,
                node_a: *nodes.get(i)?,
                node_b: *nodes.get(j)?,
                direction: direction(a, b)?,
                function: Function::New,
                layers: None,
                left_extent: None,
                base_offset: None,
                top: None,
                join_priority: None,
            }),
        });
    }
    Some(commands)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::viewport::{Screen, Viewport};
    use archviz_core::{GRID_NM, Source};
    use proptest::prelude::*;

    const FT: i64 = 304_800_000;
    const G: i64 = GRID_NM;

    fn empty() -> Scene {
        Scene::default()
    }

    /// A new project (as `archviz new` makes it) in a fresh folder, and its
    /// document: no storey, no datum.
    pub(crate) fn empty_project(tag: &str) -> (std::path::PathBuf, Document) {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let base =
            std::env::temp_dir().join(format!("archviz-app-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("project");
        let req = archviz_cli::NewRequest {
            name: "Room".to_string(),
            building: "Main".to_string(),
            project_id: None,
            building_id: None,
        };
        archviz_cli::new_project(&dir, req, &mut crate::session::System)
            .unwrap_or_else(|e| panic!("{e:?}"));
        let project = archviz_cli::store::open(&dir).unwrap_or_else(|e| panic!("{e:?}"));
        let doc = project.history.document().clone();
        (dir, doc)
    }

    fn ids() -> impl FnMut() -> (Uuid, Uuid) {
        let mut n = 0_u8;
        move || {
            n += 1;
            (Uuid::from_bytes([n; 16]), Uuid::from_bytes([n | 0x80; 16]))
        }
    }

    #[test]
    fn the_first_point_snaps_on_both_axes() {
        let mut t = WallTool::new();
        assert_eq!(
            t.click(&empty(), (G * 10 + G / 3, -G * 4 - 2), 0),
            Click::Added
        );
        assert_eq!(t.points(), &[(G * 10, -G * 4)]);
    }

    #[test]
    fn later_points_move_along_the_dominant_axis() {
        let mut t = WallTool::new();
        t.click(&empty(), (0, 0), 0);
        assert_eq!(
            t.next_point(&empty(), (10 * FT + 7, FT + 3), 0),
            (10 * FT, 0)
        );
        assert_eq!(
            t.next_point(&empty(), (FT + 3, -10 * FT - 7), 0),
            (0, -10 * FT)
        );
    }

    #[test]
    fn a_point_snaps_to_an_earlier_point_of_the_run() {
        let mut t = WallTool::new();
        for p in [(0, 0), (10 * FT, 0), (10 * FT, 8 * FT)] {
            t.click(&empty(), p, 0);
        }
        // Back west, landing a few grid steps short of the start's x.
        assert_eq!(
            t.next_point(&empty(), (5 * G, 8 * FT + 9), 8 * G),
            (0, 8 * FT)
        );
    }

    #[test]
    fn clicking_the_first_point_closes_a_rectangle() {
        let mut t = WallTool::new();
        for p in [(0, 0), (10 * FT, 0), (10 * FT, 8 * FT), (0, 8 * FT)] {
            assert_eq!(t.click(&empty(), p, 0), Click::Added);
        }
        let Click::Closed(run) = t.click(&empty(), (100, 100), 0) else {
            panic!("not closed")
        };
        assert!(run.closed);
        assert_eq!(run.points.len(), 4);
        assert!(t.points().is_empty(), "the tool starts a new run");
    }

    #[test]
    fn refused_clicks() {
        let mut t = WallTool::new();
        t.click(&empty(), (0, 0), 0);
        assert_eq!(
            t.click(&empty(), (100, 0), 0),
            Click::Ignored("a wall needs a length")
        );
        t.click(&empty(), (10 * FT, 0), 0);
        assert_eq!(
            t.click(&empty(), (5 * FT, 0), 0),
            Click::Ignored("a wall cannot double back on the one before it")
        );
        // With two points, the first point is a double-back, never a close.
        assert_eq!(
            t.click(&empty(), (0, 0), 0),
            Click::Ignored("a wall cannot double back on the one before it")
        );
        t.click(&empty(), (10 * FT, 5 * FT), 0);
        t.click(&empty(), (20 * FT, 5 * FT), 0);
        t.click(&empty(), (20 * FT, 0), 0);
        // Closing from (20 ft, 0) runs -X straight back along the +X first wall.
        assert_eq!(
            t.click(&empty(), (0, 0), 0),
            Click::Ignored("the last wall cannot double back on the first")
        );
        let mut u = WallTool::new();
        for p in [(0, 0), (10 * FT, 0), (10 * FT, 5 * FT), (5 * FT, 5 * FT)] {
            u.click(&empty(), p, 0);
        }
        u.click(&empty(), (5 * FT, -5 * FT), 0);
        assert_eq!(u.click(&empty(), (10 * FT, -5 * FT), 0), Click::Added);
        assert_eq!(
            u.click(&empty(), (10 * FT, 0), 0),
            Click::Ignored("the run already has a point there")
        );
    }

    #[test]
    fn finishing_leaves_an_open_run() {
        let mut t = WallTool::new();
        t.click(&empty(), (0, 0), 0);
        assert_eq!(t.finish(), None, "one point is no wall");
        assert_eq!(t.points().len(), 1);
        t.click(&empty(), (6 * FT, 0), 0);
        t.click(&empty(), (6 * FT, 4 * FT), 0);
        let run = t.finish().unwrap_or_else(|| panic!());
        assert!(!run.closed);
        let commands =
            run_commands(&run, Uuid::from_bytes([9; 16]), ids()).unwrap_or_else(|| panic!());
        let kinds: Vec<&str> = commands.iter().map(|c| c.action.kind()).collect();
        assert_eq!(
            kinds,
            [
                "create_node",
                "create_node",
                "create_node",
                "create_wall",
                "create_wall"
            ]
        );
    }

    #[test]
    fn a_closed_run_is_nodes_then_catalog_walls_around_it() {
        let run = Run {
            points: vec![(0, 0), (10 * FT, 0), (10 * FT, 8 * FT), (0, 8 * FT)],
            closed: true,
        };
        let storey = Uuid::from_bytes([9; 16]);
        let commands = run_commands(&run, storey, ids()).unwrap_or_else(|| panic!());
        assert_eq!(commands.len(), 8);
        let nodes: Vec<Uuid> = commands
            .iter()
            .take(4)
            .map(|c| Uuid::derive(c.idempotency_key, "command", "0"))
            .collect();
        let walls: Vec<(Uuid, Uuid, Direction)> = commands
            .iter()
            .skip(4)
            .map(|c| match &c.action {
                Action::CreateWall(w) => {
                    assert_eq!(w.storey, storey);
                    assert_eq!(w.function, Function::New);
                    assert!(
                        w.layers.is_none() && w.left_extent.is_none(),
                        "catalog wall"
                    );
                    (w.node_a, w.node_b, w.direction)
                }
                other => panic!("{other:?}"),
            })
            .collect();
        let at = |i: usize| nodes[i];
        assert_eq!(
            walls,
            [
                (at(0), at(1), Direction::PosX),
                (at(1), at(2), Direction::PosY),
                (at(2), at(3), Direction::NegX),
                (at(3), at(0), Direction::NegY),
            ]
        );
        let diagonal = Run {
            points: vec![(0, 0), (FT, FT)],
            closed: false,
        };
        assert_eq!(run_commands(&diagonal, storey, ids()), None);
    }

    #[test]
    fn setup_adds_a_storey_and_an_origin_datum_only_where_missing() {
        let mut doc = empty_project("draw-setup").1;
        let actions = setup_actions(&doc);
        let kinds: Vec<&str> = actions.iter().map(Action::kind).collect();
        assert_eq!(kinds, ["set_storey", "create_datum"]);
        let Some(Action::SetStorey(s)) = actions.first() else {
            panic!()
        };
        for v in [&s.elevation, &s.floor_to_floor, &s.default_ceiling_height] {
            assert_eq!(v.source, Source::Proposed);
        }
        // An X line alone leaves Y unanchored.
        doc.datums.insert(
            Uuid::from_bytes([1; 16]),
            archviz_core::Datum {
                name: "A".to_string(),
                geometry: DatumGeometry::XLine {
                    x: Length::from_nanometers(0),
                },
            },
        );
        assert_eq!(setup_actions(&doc).len(), 2);
        doc.datums.insert(
            Uuid::from_bytes([2; 16]),
            archviz_core::Datum {
                name: "B".to_string(),
                geometry: DatumGeometry::YLine {
                    y: Length::from_nanometers(0),
                },
            },
        );
        assert_eq!(setup_actions(&doc).len(), 1, "only the storey");
    }

    proptest! {
        /// From any fractional pointer positions at any zoom, every point
        /// the tool places is on the 1/16 in grid and every segment is
        /// axis-aligned.
        #[test]
        fn every_point_is_on_the_grid(
            scale in 1e-9_f64..2.5e-4,
            center in (-1_i64 << 40..1_i64 << 40, -1_i64 << 40..1_i64 << 40),
            clicks in proptest::collection::vec((0.0_f64..1280.0, 0.0_f64..800.0), 1..12),
            snap_px in 0.0_f64..12.0,
        ) {
            let v = Viewport::new(center, scale, Screen { x: 640.0, y: 400.0 });
            let mut t = WallTool::new();
            let mut placed = Vec::new();
            for (x, y) in clicks {
                let at = v.to_world(Screen { x, y });
                if let Click::Closed(run) = t.click(&empty(), at, v.nm(snap_px)) {
                    placed.extend(run.points);
                }
                placed.extend_from_slice(t.points());
            }
            for p in &placed {
                prop_assert_eq!(p.0.rem_euclid(G), 0);
                prop_assert_eq!(p.1.rem_euclid(G), 0);
            }
            for w in t.points().windows(2) {
                prop_assert!(heading(w[0], w[1]).is_some());
            }
        }
    }
}
