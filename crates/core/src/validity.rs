//! Validity invariants (spec 7.7; Q-27 to Q-29).
//!
//! `validate` checks, in order:
//!
//! 1. every wall has positive length along its direction, with its nodes on
//!    one line; a wall that fails is reported and left out of the checks
//!    below, with the openings it hosts, so it cannot hide other problems;
//! 2. no two walls on one storey overlap unless they share a node (a
//!    declared join); overlap means their exact footprints share area, so
//!    touching is allowed;
//! 3. every opening lies inside its host's full-thickness stretch, the
//!    minimum jamb distance from either end, and between base and top;
//! 4. every Space and Slab outline is a simple rectilinear polygon.
//!
//! The occurrence-overlap invariant is checked from T-111 on (Q-27).
//!
//! Each violation has a key (kind and sorted entity ids). A violation whose
//! key is in the document's `acknowledged` set does not block commit (Q-28).

use std::collections::BTreeSet;

use crate::document::{Direction, Document, Point, ViolationKey, ViolationKind, Wall};
use crate::geometry::{GeometryError, OpeningProblem, opening_violations, wall_geometry};
use crate::identity::Uuid;
use crate::length::Length;
use crate::space::SpaceError;
use crate::space::{boundary_outline, space_boundary};

/// What exactly is wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detail {
    /// node_b is `length` from node_a along the wall's direction (zero or less).
    NonPositiveLength { length: Length },
    /// The nodes are `offset` apart across the wall's direction.
    NotOnOneLine { offset: Length },
    /// The two walls' footprints share area.
    Overlap,
    /// Every problem with one opening.
    Opening(Vec<OpeningProblem>),
    /// Why the outline is not a simple rectilinear polygon.
    Outline(SpaceError),
}

/// One failed invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub key: ViolationKey,
    pub detail: Detail,
    /// Marked Acknowledged by the user: reported but not blocking.
    pub acknowledged: bool,
}

/// Every violation in `document`, sorted by key.
pub fn validate(document: &Document, min_jamb: Length) -> Result<Vec<Violation>, GeometryError> {
    let mut found: Vec<(ViolationKey, Detail)> = Vec::new();
    let key = |kind, mut entities: Vec<Uuid>| {
        entities.sort();
        ViolationKey { kind, entities }
    };

    // 1. Positive length, nodes on one line.
    let mut bad = BTreeSet::new();
    for (id, w) in &document.walls {
        let node = |n: Uuid| {
            document
                .nodes
                .get(&n)
                .ok_or(GeometryError::UnknownEntity { id: n })
        };
        let (a, b) = (node(w.node_a)?, node(w.node_b)?);
        let (along, across, dir) = match w.direction {
            Direction::PosX => (
                b.x.nanometers() - a.x.nanometers(),
                b.y.nanometers() - a.y.nanometers(),
                1,
            ),
            Direction::NegX => (
                b.x.nanometers() - a.x.nanometers(),
                b.y.nanometers() - a.y.nanometers(),
                -1,
            ),
            Direction::PosY => (
                b.y.nanometers() - a.y.nanometers(),
                b.x.nanometers() - a.x.nanometers(),
                1,
            ),
            Direction::NegY => (
                b.y.nanometers() - a.y.nanometers(),
                b.x.nanometers() - a.x.nanometers(),
                -1,
            ),
        };
        let detail = if across != 0 {
            Some(Detail::NotOnOneLine {
                offset: Length::from_nanometers(
                    across.checked_abs().ok_or(GeometryError::Overflow)?,
                ),
            })
        } else {
            let length = along.checked_mul(dir).ok_or(GeometryError::Overflow)?;
            (length <= 0).then_some(Detail::NonPositiveLength {
                length: Length::from_nanometers(length),
            })
        };
        if let Some(detail) = detail {
            bad.insert(*id);
            found.push((key(ViolationKind::WallLength, vec![*id]), detail));
        }
    }

    // The rest of the checks run without the bad walls and their openings.
    let mut sound = document.clone();
    sound.walls.retain(|id, _| !bad.contains(id));
    sound.openings.retain(|_, o| !bad.contains(&o.host));

    // 2. Overlapping walls without a declared join.
    let geometry = wall_geometry(&sound)?;
    let walls: Vec<(&Uuid, &Wall, &Vec<Point>)> = sound
        .walls
        .iter()
        .filter_map(|(id, w)| geometry.get(id).map(|g| (id, w, &g.footprint)))
        .collect();
    for (i, (ia, wa, pa)) in walls.iter().enumerate() {
        for (ib, wb, pb) in &walls[i + 1..] {
            let joined = [wa.node_a, wa.node_b]
                .iter()
                .any(|n| *n == wb.node_a || *n == wb.node_b);
            if wa.storey == wb.storey && !joined && interiors_overlap(pa, pb) {
                found.push((
                    key(ViolationKind::WallOverlap, vec![**ia, **ib]),
                    Detail::Overlap,
                ));
            }
        }
    }

    // 3. Openings inside their host.
    let mut by_opening: std::collections::BTreeMap<Uuid, Vec<OpeningProblem>> =
        std::collections::BTreeMap::new();
    for v in opening_violations(&sound, min_jamb)? {
        by_opening.entry(v.opening).or_default().push(v.problem);
    }
    for (id, problems) in by_opening {
        found.push((
            key(ViolationKind::OpeningOutsideHost, vec![id]),
            Detail::Opening(problems),
        ));
    }

    // 4. Space and Slab outlines.
    for id in sound.spaces.keys() {
        if let Err(e) = space_boundary(&sound, *id) {
            found.push((
                key(ViolationKind::SpaceOutline, vec![*id]),
                Detail::Outline(e),
            ));
        }
    }
    for (id, slab) in &sound.slabs {
        if let Err(e) = boundary_outline(&sound, &slab.boundary) {
            found.push((
                key(ViolationKind::SlabOutline, vec![*id]),
                Detail::Outline(e),
            ));
        }
    }

    found.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(found
        .into_iter()
        .map(|(key, detail)| Violation {
            acknowledged: document.acknowledged.contains(&key),
            key,
            detail,
        })
        .collect())
}

/// Whether two convex polygons share area (separating axis test, exact).
/// Polygons that only touch along an edge or at a point do not.
fn interiors_overlap(a: &[Point], b: &[Point]) -> bool {
    let xy = |p: &Point| (i128::from(p.x.nanometers()), i128::from(p.y.nanometers()));
    let (a, b): (Vec<_>, Vec<_>) = (a.iter().map(xy).collect(), b.iter().map(xy).collect());
    for poly in [&a, &b] {
        let n = poly.len();
        for i in 0..n {
            let (p, q) = (poly[i], poly[(i + 1) % n]);
            let axis = (p.1 - q.1, q.0 - p.0);
            if axis == (0, 0) {
                continue;
            }
            let project = |pts: &[(i128, i128)]| {
                let it = pts.iter().map(|v| v.0 * axis.0 + v.1 * axis.1);
                (it.clone().min(), it.max())
            };
            let ((Some(a0), Some(a1)), (Some(b0), Some(b1))) = (project(&a), project(&b)) else {
                return false;
            };
            if a1 <= b0 || b1 <= a0 {
                return false;
            }
        }
    }
    true
}

/// Whether any violation is unacknowledged and so blocks commit (7.7).
pub fn blocks_commit(violations: &[Violation]) -> bool {
    violations.iter().any(|v| !v.acknowledged)
}

/// Drops acknowledgments whose violation no longer occurs (Q-28).
pub fn prune_acknowledged(document: &mut Document, violations: &[Violation]) {
    let current: BTreeSet<&ViolationKey> = violations.iter().map(|v| &v.key).collect();
    document.acknowledged.retain(|k| current.contains(k));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defaults::MIN_JAMB_DISTANCE;
    use crate::document::*;
    use crate::identity::LabelAllocator;
    use crate::space::PolygonError;
    use crate::units::ProjectUnit;

    const IN: i64 = 25_400_000;

    fn len(nm: i64) -> Length {
        Length::from_nanometers(nm)
    }

    fn uid(kind: u8, n: u8) -> Uuid {
        let mut b = [0u8; 16];
        b[0] = kind;
        b[15] = n;
        Uuid::from_bytes(b)
    }
    fn node_id(n: u8) -> Uuid {
        uid(1, n)
    }
    fn wall_id(n: u8) -> Uuid {
        uid(2, n)
    }

    const STOREY: Uuid = Uuid::from_bytes([8; 16]);

    fn sourced(nm: i64) -> Sourced<Length> {
        Sourced {
            value: len(nm),
            source: Source::UserStated,
        }
    }

    fn base() -> Document {
        let mut doc = Document::new(
            Project {
                id: uid(9, 0),
                name: "T".into(),
                default_unit: ProjectUnit::Inches,
            },
            Building {
                id: uid(9, 1),
                name: "B".into(),
            },
        );
        doc.storeys.insert(
            STOREY,
            Storey {
                name: "Ground".into(),
                elevation: sourced(0),
                floor_to_floor: sourced(120 * IN),
                default_ceiling_height: sourced(108 * IN),
            },
        );
        doc.layer_sets.insert(
            uid(5, 0),
            LayerSet {
                name: "4 in".into(),
                layers: vec![Layer {
                    material: uid(6, 0),
                    thickness: sourced(4 * IN),
                    core: true,
                }],
            },
        );
        doc
    }

    fn node(doc: &mut Document, n: u8, x: i64, y: i64) {
        doc.nodes.insert(
            node_id(n),
            Node {
                x: len(x * IN),
                y: len(y * IN),
                join: JoinType::Isolated,
            },
        );
    }

    fn wall(doc: &mut Document, n: u8, a: u8, b: u8, direction: Direction) {
        let mut labels = LabelAllocator::new();
        doc.walls.insert(
            wall_id(n),
            Wall {
                label: labels.allocate("W").unwrap_or_else(|_| unreachable!()),
                storey: STOREY,
                node_a: node_id(a),
                node_b: node_id(b),
                direction,
                layer_set: uid(5, 0),
                left_extent: sourced(2 * IN),
                base_offset: sourced(0),
                top: WallTop::Storey,
                join_priority: 0,
                function: Function::New,
            },
        );
    }

    /// A closed 240 x 144 in room of four walls, with a space and a slab.
    fn room() -> Document {
        let mut doc = base();
        node(&mut doc, 1, 0, 0);
        node(&mut doc, 2, 240, 0);
        node(&mut doc, 3, 240, 144);
        node(&mut doc, 4, 0, 144);
        wall(&mut doc, 1, 1, 2, Direction::PosX);
        wall(&mut doc, 2, 2, 3, Direction::PosY);
        wall(&mut doc, 3, 3, 4, Direction::NegX);
        wall(&mut doc, 4, 4, 1, Direction::NegY);
        doc.spaces.insert(
            uid(11, 0),
            Space {
                storey: STOREY,
                name: "Room".into(),
                seed: Point {
                    x: len(120 * IN),
                    y: len(72 * IN),
                },
                boundary: vec![],
                finishes: Finishes {
                    floor: None,
                    walls: None,
                    ceiling: None,
                },
            },
        );
        doc.slabs.insert(
            uid(12, 0),
            Slab {
                storey: STOREY,
                boundary: Boundary::FromSpace { space: uid(11, 0) },
                thickness: sourced(4 * IN),
                elevation: sourced(0),
            },
        );
        doc
    }

    fn check(doc: &Document) -> Vec<Violation> {
        validate(doc, MIN_JAMB_DISTANCE).unwrap_or_else(|e| panic!("{e}"))
    }

    fn keys(v: &[Violation]) -> Vec<(ViolationKind, Vec<Uuid>)> {
        v.iter()
            .map(|v| (v.key.kind, v.key.entities.clone()))
            .collect()
    }

    fn move_node(doc: &mut Document, n: u8, x: i64, y: i64) {
        if let Some(nd) = doc.nodes.get_mut(&node_id(n)) {
            nd.x = len(x * IN);
            nd.y = len(y * IN);
        }
    }

    #[test]
    fn a_valid_room_has_no_violations() {
        assert_eq!(check(&room()), vec![]);
    }

    /// Invariant 1: every wall has positive length.
    #[test]
    fn walls_must_have_positive_length() {
        let mut doc = room();
        // Pull node 2 back past node 1: wall 1 (+X) now runs backwards.
        move_node(&mut doc, 2, -10, 0);
        move_node(&mut doc, 3, -10, 144);
        let v = check(&doc);
        let w1 = v.iter().find(|v| {
            v.key
                == ViolationKey {
                    kind: ViolationKind::WallLength,
                    entities: vec![wall_id(1)],
                }
        });
        assert_eq!(
            w1.map(|v| v.detail.clone()),
            Some(Detail::NonPositiveLength {
                length: len(-10 * IN)
            })
        );
        // Zero length.
        let mut doc = base();
        node(&mut doc, 1, 0, 0);
        node(&mut doc, 2, 0, 0);
        wall(&mut doc, 1, 1, 2, Direction::PosX);
        assert_eq!(
            check(&doc).first().map(|v| v.detail.clone()),
            Some(Detail::NonPositiveLength { length: len(0) })
        );
        // Nodes not on one line.
        let mut doc = base();
        node(&mut doc, 1, 0, 0);
        node(&mut doc, 2, 50, 3);
        wall(&mut doc, 1, 1, 2, Direction::PosX);
        assert_eq!(
            check(&doc).first().map(|v| v.detail.clone()),
            Some(Detail::NotOnOneLine {
                offset: len(3 * IN)
            })
        );
    }

    /// Invariant 2: walls do not overlap unless they share a node.
    #[test]
    fn walls_must_not_overlap_without_a_join() {
        // Two free walls crossing like a plus sign.
        let mut doc = base();
        node(&mut doc, 1, 0, 50);
        node(&mut doc, 2, 100, 50);
        node(&mut doc, 3, 50, 0);
        node(&mut doc, 4, 50, 100);
        wall(&mut doc, 1, 1, 2, Direction::PosX);
        wall(&mut doc, 2, 3, 4, Direction::PosY);
        assert_eq!(
            keys(&check(&doc)),
            vec![(ViolationKind::WallOverlap, vec![wall_id(1), wall_id(2)])]
        );
        // A stem butting into a wall's middle without a shared node.
        let mut doc = base();
        node(&mut doc, 1, 0, 0);
        node(&mut doc, 2, 100, 0);
        node(&mut doc, 3, 50, 0);
        node(&mut doc, 4, 50, 60);
        wall(&mut doc, 1, 1, 2, Direction::PosX);
        wall(&mut doc, 2, 3, 4, Direction::PosY);
        assert_eq!(
            keys(&check(&doc)),
            vec![(ViolationKind::WallOverlap, vec![wall_id(1), wall_id(2)])]
        );
        // Parallel walls 3 in apart on centerlines: 4 in thick, so they overlap.
        let mut doc = base();
        node(&mut doc, 1, 0, 0);
        node(&mut doc, 2, 100, 0);
        node(&mut doc, 3, 40, 3);
        node(&mut doc, 4, 140, 3);
        wall(&mut doc, 1, 1, 2, Direction::PosX);
        wall(&mut doc, 2, 3, 4, Direction::PosX);
        assert_eq!(
            keys(&check(&doc)),
            vec![(ViolationKind::WallOverlap, vec![wall_id(1), wall_id(2)])]
        );
    }

    #[test]
    fn touching_joined_or_other_storey_walls_are_fine() {
        // Exactly 4 in apart: faces touch, no shared area.
        let mut doc = base();
        node(&mut doc, 1, 0, 0);
        node(&mut doc, 2, 100, 0);
        node(&mut doc, 3, 0, 4);
        node(&mut doc, 4, 100, 4);
        wall(&mut doc, 1, 1, 2, Direction::PosX);
        wall(&mut doc, 2, 3, 4, Direction::PosX);
        assert_eq!(check(&doc), vec![]);
        // The plus sign again, but on another storey.
        let mut doc = base();
        node(&mut doc, 1, 0, 50);
        node(&mut doc, 2, 100, 50);
        node(&mut doc, 3, 50, 0);
        node(&mut doc, 4, 50, 100);
        wall(&mut doc, 1, 1, 2, Direction::PosX);
        wall(&mut doc, 2, 3, 4, Direction::PosY);
        let upper = doc.storeys.get(&STOREY).cloned();
        if let Some(upper) = upper {
            doc.storeys.insert(uid(8, 1), upper);
        }
        if let Some(w) = doc.walls.get_mut(&wall_id(2)) {
            w.storey = uid(8, 1);
        }
        assert_eq!(check(&doc), vec![]);
    }

    /// Invariant 3: openings lie inside their host.
    #[test]
    fn openings_must_lie_inside_their_host() {
        let mut doc = room();
        let opening = uid(10, 1);
        doc.openings.insert(
            opening,
            Opening {
                kind: OpeningKind::Door,
                host: wall_id(1),
                reference: Reference::Node { node: node_id(1) },
                anchor: OpeningEdge::Start,
                offset: sourced(220 * IN),
                width: sourced(36 * IN),
                height: sourced(130 * IN),
                sill: sourced(0),
                start: len(220 * IN),
            },
        );
        let v = check(&doc);
        assert_eq!(
            keys(&v),
            vec![(ViolationKind::OpeningOutsideHost, vec![opening])]
        );
        // Every problem is reported under the one key: past the end and above the top.
        assert_eq!(
            v.first().map(|v| v.detail.clone()),
            Some(Detail::Opening(vec![
                OpeningProblem::TooCloseToEnd {
                    clearance: len(-18 * IN)
                },
                OpeningProblem::AboveTop { by: len(10 * IN) },
            ]))
        );
    }

    /// Invariant 4a: every Space outline is a simple rectilinear polygon.
    #[test]
    fn space_outlines_must_be_simple() {
        let mut doc = room();
        doc.walls.remove(&wall_id(3));
        let v = check(&doc);
        let space = v.iter().find(|v| v.key.kind == ViolationKind::SpaceOutline);
        assert_eq!(
            space.map(|v| v.detail.clone()),
            Some(Detail::Outline(SpaceError::Open { space: uid(11, 0) }))
        );
    }

    /// Invariant 4b: every Slab outline is a simple rectilinear polygon.
    #[test]
    fn slab_outlines_must_be_simple() {
        let mut doc = room();
        let pts = |v: &[(i64, i64)]| {
            v.iter()
                .map(|&(x, y)| Point {
                    x: len(x * IN),
                    y: len(y * IN),
                })
                .collect()
        };
        if let Some(slab) = doc.slabs.get_mut(&uid(12, 0)) {
            slab.boundary = Boundary::Explicit {
                points: pts(&[(0, 0), (4, 0), (4, 4), (2, 4), (2, -2), (0, -2)]),
            };
        }
        let v = check(&doc);
        assert_eq!(
            keys(&v),
            vec![(ViolationKind::SlabOutline, vec![uid(12, 0)])]
        );
        assert!(matches!(
            v.first().map(|v| &v.detail),
            Some(Detail::Outline(SpaceError::Polygon(
                PolygonError::SelfIntersecting { .. }
            )))
        ));
    }

    /// 7.7: the checks are the only guard against a move that carries a wall
    /// through another. Move the room's north wall onto, then past, the south wall.
    #[test]
    fn a_wall_moved_through_another_is_caught() {
        // Onto it: north wall now lies on the south wall; east and west have no length.
        let mut doc = room();
        move_node(&mut doc, 3, 240, 0);
        move_node(&mut doc, 4, 0, 0);
        let k = keys(&check(&doc));
        assert!(
            k.contains(&(ViolationKind::WallOverlap, vec![wall_id(1), wall_id(3)])),
            "{k:?}"
        );
        assert!(
            k.contains(&(ViolationKind::WallLength, vec![wall_id(2)])),
            "{k:?}"
        );
        assert!(
            k.contains(&(ViolationKind::WallLength, vec![wall_id(4)])),
            "{k:?}"
        );
        // Through and past it: no overlap any more, but east and west now run backwards.
        let mut doc = room();
        move_node(&mut doc, 3, 240, -50);
        move_node(&mut doc, 4, 0, -50);
        let k = keys(&check(&doc));
        assert!(
            k.contains(&(ViolationKind::WallLength, vec![wall_id(2)])),
            "{k:?}"
        );
        assert!(
            k.contains(&(ViolationKind::WallLength, vec![wall_id(4)])),
            "{k:?}"
        );
        // A free wall slid sideways through a parallel one.
        let mut doc = base();
        node(&mut doc, 1, 0, 0);
        node(&mut doc, 2, 100, 0);
        node(&mut doc, 3, 0, 10);
        node(&mut doc, 4, 100, 10);
        wall(&mut doc, 1, 1, 2, Direction::PosX);
        wall(&mut doc, 2, 3, 4, Direction::PosX);
        assert_eq!(check(&doc), vec![]);
        move_node(&mut doc, 3, 0, 1);
        move_node(&mut doc, 4, 100, 1);
        assert_eq!(
            keys(&check(&doc)),
            vec![(ViolationKind::WallOverlap, vec![wall_id(1), wall_id(2)])]
        );
    }

    #[test]
    fn acknowledged_violations_do_not_block() {
        let mut doc = room();
        doc.walls.remove(&wall_id(3));
        let v = check(&doc);
        assert!(blocks_commit(&v));
        for violation in &v {
            doc.acknowledged.insert(violation.key.clone());
        }
        let v = check(&doc);
        assert!(v.iter().all(|v| v.acknowledged));
        assert!(!blocks_commit(&v));
        // Fixing the problem and pruning drops the stale marks.
        let fixed = room();
        let mut doc_fixed = fixed.clone();
        doc_fixed.acknowledged = doc.acknowledged.clone();
        let v = check(&doc_fixed);
        assert_eq!(v, vec![]);
        prune_acknowledged(&mut doc_fixed, &v);
        assert!(doc_fixed.acknowledged.is_empty());
    }

    #[test]
    fn violations_are_sorted_and_deterministic() {
        let mut doc = room();
        doc.walls.remove(&wall_id(3));
        move_node(&mut doc, 2, -10, 0);
        let a = check(&doc);
        let b = check(&doc.clone());
        assert_eq!(a, b);
        let ks: Vec<_> = a.iter().map(|v| v.key.clone()).collect();
        let mut sorted = ks.clone();
        sorted.sort();
        assert_eq!(ks, sorted);
    }
}
