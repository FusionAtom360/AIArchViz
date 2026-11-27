//! clearance_resolve: the deterministic push operator (spec 8.8; Q-46).
//!
//! Given the occurrences allowed to move, an axis, and violations to fix,
//! the operator takes the violations in label order. In each, the movable
//! item (the later label when both are movable) moves away from the other
//! entity along the axis by the exact minimal amount, rounded up to a whole
//! 1/16 in. A moved item that then violates something else pushes it the
//! minimal amount in the same direction, and so on. A wall, a Fixed item or
//! an item not allowed to move in the way fails the operator, naming the
//! chain. The result is a proposal of MoveOccurrence commands; nothing is
//! committed. The agent chooses the inputs, never the displacements.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::command::{Action, Command, Motion, MoveOccurrence};
use crate::defaults::MIN_JAMB_DISTANCE;
use crate::document::{
    Axis, ConstraintKind, ConstraintStatus, Direction, Document, Placement, ViolationKey,
    ViolationKind,
};
use crate::geometry::{GeometryError, wall_geometry, wall_profiles};
use crate::identity::{Label, Uuid};
use crate::items::{Rect, clearance_zone, footprint, height_range};
use crate::length::Length;
use crate::log::History;
use crate::proposal::{Proposal, ProposeError, ProposedAction, propose};
use crate::units::GRID_NM;
use crate::validity::validate;

/// What to resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClearanceRequest {
    /// The occurrences allowed to move.
    pub movable: Vec<Uuid>,
    /// The axis items are pushed along (see `wall_normal_axis`).
    pub axis: Axis,
    /// OccurrenceOverlap, ClearanceZone and MinClearance violations to fix.
    pub violations: Vec<ViolationKey>,
}

/// Why the operator could not resolve the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClearanceError {
    /// A push ran into something that may not move. `chain` lists the moved
    /// items in order, then the blocker.
    Blocked {
        chain: Vec<Uuid>,
        blocker: Uuid,
    },
    /// The key is not a current violation the operator can fix.
    NotResolvable(ViolationKey),
    /// Pushes did not settle (a cycle).
    DidNotSettle,
    Geometry(GeometryError),
    Propose(ProposeError),
    UnknownEntity {
        id: Uuid,
    },
}

impl fmt::Display for ClearanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClearanceError::Blocked { chain, blocker } => write!(
                f,
                "blocked by {blocker} after pushing {} item(s)",
                chain.len().saturating_sub(1)
            ),
            ClearanceError::NotResolvable(k) => {
                write!(f, "{k:?} is not a violation this operator can fix")
            }
            ClearanceError::DidNotSettle => f.write_str("the pushes did not settle"),
            ClearanceError::Geometry(e) => write!(f, "{e}"),
            ClearanceError::Propose(e) => write!(f, "{e}"),
            ClearanceError::UnknownEntity { id } => write!(f, "no entity {id}"),
        }
    }
}

impl std::error::Error for ClearanceError {}

/// The axis normal to a wall: the default policy of 8.8.
pub fn wall_normal_axis(document: &Document, wall: Uuid) -> Result<Axis, ClearanceError> {
    use crate::document::Direction;
    let w = document
        .walls
        .get(&wall)
        .ok_or(ClearanceError::UnknownEntity { id: wall })?;
    Ok(match w.direction {
        Direction::PosX | Direction::NegX => Axis::Y,
        Direction::PosY | Direction::NegY => Axis::X,
    })
}

type Box2 = ((i128, i128), (i128, i128));

/// A wall or an occurrence as the operator sees it.
struct Body {
    id: Uuid,
    label: Option<Label>,
    storey: Uuid,
    footprint: Box2,
    zone: Option<Box2>,
    heights: (i128, i128),
    /// Walls are None.
    item: Option<ItemFlags>,
}

#[derive(Clone, Copy)]
struct ItemFlags {
    overlap_allowed: bool,
    /// In the movable set, not Fixed, and able to move on the axis.
    movable: bool,
}

fn n(l: Length) -> i128 {
    i128::from(l.nanometers())
}

fn rect_box(r: Rect) -> Box2 {
    ((n(r.min.x), n(r.min.y)), (n(r.max.x), n(r.max.y)))
}

fn on(axis: Axis, p: (i128, i128)) -> i128 {
    match axis {
        Axis::X => p.0,
        Axis::Y => p.1,
    }
}

fn across(axis: Axis) -> Axis {
    match axis {
        Axis::X => Axis::Y,
        Axis::Y => Axis::X,
    }
}

fn shifted(b: Box2, axis: Axis, d: i128) -> Box2 {
    match axis {
        Axis::X => ((b.0.0 + d, b.0.1), (b.1.0 + d, b.1.1)),
        Axis::Y => ((b.0.0, b.0.1 + d), (b.1.0, b.1.1 + d)),
    }
}

fn boxes_overlap(a: Box2, b: Box2) -> bool {
    a.0.0 < b.1.0 && b.0.0 < a.1.0 && a.0.1 < b.1.1 && b.0.1 < a.1.1
}

/// How far `b` must move along `dir` to stop overlapping `a`.
fn separation(a: Box2, b: Box2, axis: Axis, dir: i128) -> i128 {
    if dir > 0 {
        on(axis, a.1) - on(axis, b.0)
    } else {
        on(axis, b.1) - on(axis, a.0)
    }
}

/// The gap between two intervals (0 when they overlap).
fn gap(a0: i128, a1: i128, b0: i128, b1: i128) -> i128 {
    0.max(b0 - a1).max(a0 - b1)
}

/// The smallest integer whose square is at least `v`.
fn ceil_sqrt(v: i128) -> i128 {
    if v <= 0 {
        return 0;
    }
    let v = v.unsigned_abs();
    let mut x: u128 = 1u128 << ((128 - v.leading_zeros()).div_ceil(2));
    loop {
        let y = (x + v / x) / 2;
        if y >= x {
            break;
        }
        x = y;
    }
    // x is now floor(sqrt(v)) or one more; settle on the ceiling.
    while x > 0 && (x - 1) * (x - 1) >= v {
        x -= 1;
    }
    while x * x < v {
        x += 1;
    }
    i128::try_from(x).unwrap_or(i128::MAX)
}

/// Rounds a positive shift up to a whole 1/16 in (8.8).
fn to_grid(shift: i128) -> i128 {
    let g = i128::from(GRID_NM);
    shift.div_euclid(g) * g + if shift.rem_euclid(g) == 0 { 0 } else { g }
}

struct Operator<'d> {
    doc: &'d Document,
    axis: Axis,
    bodies: BTreeMap<Uuid, Body>,
    /// Bodies in scanning order: by label, then id.
    order: Vec<Uuid>,
    delta: BTreeMap<Uuid, i128>,
    steps: usize,
}

impl Operator<'_> {
    fn footprint(&self, id: Uuid) -> Option<Box2> {
        let b = self.bodies.get(&id)?;
        Some(shifted(
            b.footprint,
            self.axis,
            self.delta.get(&id).copied().unwrap_or(0),
        ))
    }

    fn zone(&self, id: Uuid) -> Option<Box2> {
        let b = self.bodies.get(&id)?;
        Some(shifted(
            b.zone?,
            self.axis,
            self.delta.get(&id).copied().unwrap_or(0),
        ))
    }

    fn twice_center(&self, id: Uuid) -> i128 {
        self.footprint(id)
            .map_or(0, |f| on(self.axis, f.0) + on(self.axis, f.1))
    }

    /// How far `mover` must move along `dir` to clear `still`: overlap,
    /// clearance zones both ways, and MinClearance (exact; Q-46).
    fn need(&self, still: Uuid, mover: Uuid, dir: i128) -> i128 {
        let (Some(a), Some(b)) = (self.bodies.get(&still), self.bodies.get(&mover)) else {
            return 0;
        };
        if a.storey != b.storey {
            return 0;
        }
        let (Some(af), Some(bf)) = (self.footprint(still), self.footprint(mover)) else {
            return 0;
        };
        let exempt = [a, b]
            .iter()
            .any(|x| x.item.is_some_and(|i| i.overlap_allowed));
        let stacked = a.heights.0 < b.heights.1 && b.heights.0 < a.heights.1;
        let mut need = 0i128;
        if !exempt && stacked {
            let mut pairs = vec![(af, bf)];
            if let Some(z) = self.zone(still) {
                pairs.push((z, bf));
            }
            if let Some(z) = self.zone(mover) {
                pairs.push((af, z));
            }
            for (x, y) in pairs {
                if boxes_overlap(x, y) {
                    need = need.max(separation(x, y, self.axis, dir));
                }
            }
        }
        for c in self.doc.constraints.values() {
            let ConstraintKind::MinClearance {
                object,
                other,
                value,
            } = c.kind
            else {
                continue;
            };
            let pair = (object == still && other == mover) || (object == mover && other == still);
            if !pair || c.status != ConstraintStatus::Driving {
                continue;
            }
            let o = across(self.axis);
            let perpendicular = gap(on(o, af.0), on(o, af.1), on(o, bf.0), on(o, bf.1));
            let along = if dir > 0 {
                on(self.axis, bf.0) - on(self.axis, af.1)
            } else {
                on(self.axis, af.0) - on(self.axis, bf.1)
            };
            let v = n(value);
            let required = if v > perpendicular {
                ceil_sqrt(v * v - perpendicular * perpendicular)
            } else {
                0
            };
            need = need.max(required - along);
        }
        need
    }

    fn movable(&self, id: Uuid) -> bool {
        self.bodies
            .get(&id)
            .and_then(|b| b.item)
            .is_some_and(|i| i.movable)
    }

    /// Moves `id` by `shift` along `dir`, then pushes whatever it runs into.
    fn push(
        &mut self,
        id: Uuid,
        dir: i128,
        shift: i128,
        chain: &mut Vec<Uuid>,
    ) -> Result<(), ClearanceError> {
        self.steps += 1;
        if self.steps > 100_000 {
            return Err(ClearanceError::DidNotSettle);
        }
        *self.delta.entry(id).or_insert(0) += dir * to_grid(shift);
        loop {
            let me = self.twice_center(id);
            let hit = self.order.iter().copied().find_map(|e| {
                if e == id || (self.twice_center(e) - me) * dir < 0 {
                    return None;
                }
                let r = self.need(id, e, dir);
                (r > 0).then_some((e, r))
            });
            let Some((e, r)) = hit else {
                return Ok(());
            };
            chain.push(e);
            if !self.movable(e) {
                return Err(ClearanceError::Blocked {
                    chain: chain.clone(),
                    blocker: e,
                });
            }
            self.push(e, dir, r, chain)?;
            chain.pop();
        }
    }
}

/// Runs the operator against the head of `history` and proposes the result.
pub fn clearance_resolve(
    history: &History,
    request: &ClearanceRequest,
) -> Result<Proposal, ClearanceError> {
    let doc = history.document();
    let axis = request.axis;
    let current: BTreeSet<ViolationKey> = validate(doc, MIN_JAMB_DISTANCE)
        .map_err(ClearanceError::Geometry)?
        .into_iter()
        .map(|v| v.key)
        .collect();
    // Bodies: every wall (by its footprint's bounding box) and occurrence.
    let geometry = wall_geometry(doc).map_err(ClearanceError::Geometry)?;
    let profiles = wall_profiles(doc).map_err(ClearanceError::Geometry)?;
    let mut bodies = BTreeMap::new();
    for (id, w) in &doc.walls {
        let (Some(g), Some(p)) = (geometry.get(id), profiles.get(id)) else {
            continue;
        };
        let xs = g.footprint.iter().map(|p| n(p.x));
        let ys = g.footprint.iter().map(|p| n(p.y));
        let (Some(x0), Some(x1), Some(y0), Some(y1)) =
            (xs.clone().min(), xs.max(), ys.clone().min(), ys.max())
        else {
            continue;
        };
        bodies.insert(
            *id,
            Body {
                id: *id,
                label: Some(w.label.clone()),
                storey: w.storey,
                footprint: ((x0, y0), (x1, y1)),
                zone: None,
                heights: (n(p.base), n(p.top)),
                item: None,
            },
        );
    }
    let movable: BTreeSet<Uuid> = request.movable.iter().copied().collect();
    for (id, o) in &doc.occurrences {
        let footprint = rect_box(footprint(doc, *id).map_err(ClearanceError::Geometry)?);
        let zone = rect_box(clearance_zone(doc, *id).map_err(ClearanceError::Geometry)?);
        let (bottom, top) = height_range(doc, *id).map_err(ClearanceError::Geometry)?;
        let can_move_on_axis = match &o.placement {
            Placement::WallMounted { wall, .. } => doc.walls.get(wall).is_some_and(|w| {
                matches!(
                    (w.direction, axis),
                    (Direction::PosX | Direction::NegX, Axis::X)
                        | (Direction::PosY | Direction::NegY, Axis::Y)
                )
            }),
            _ => true,
        };
        bodies.insert(
            *id,
            Body {
                id: *id,
                label: Some(o.label.clone()),
                storey: o.storey,
                footprint,
                zone: (zone != footprint).then_some(zone),
                heights: (n(bottom), n(top)),
                item: Some(ItemFlags {
                    overlap_allowed: o.overlap_allowed,
                    movable: movable.contains(id) && !o.fixed && can_move_on_axis,
                }),
            },
        );
    }
    let mut order: Vec<&Body> = bodies.values().collect();
    order.sort_by(|a, b| (&a.label, a.id).cmp(&(&b.label, b.id)));
    let order: Vec<Uuid> = order.into_iter().map(|b| b.id).collect();
    let mut op = Operator {
        doc,
        axis,
        bodies,
        order,
        delta: BTreeMap::new(),
        steps: 0,
    };

    // Each requested violation as (mover, other), in label order.
    let mut pairs = Vec::new();
    for key in &request.violations {
        let fixable = matches!(
            key.kind,
            ViolationKind::OccurrenceOverlap
                | ViolationKind::ClearanceZone
                | ViolationKind::MinClearance
        );
        if !fixable || !current.contains(key) {
            return Err(ClearanceError::NotResolvable(key.clone()));
        }
        let (p, q) = match key.kind {
            ViolationKind::MinClearance => {
                let c = key.entities.first().and_then(|c| doc.constraints.get(c));
                match c.map(|c| &c.kind) {
                    Some(ConstraintKind::MinClearance { object, other, .. }) => (*object, *other),
                    _ => return Err(ClearanceError::NotResolvable(key.clone())),
                }
            }
            _ => match key.entities.as_slice() {
                [p, q] => (*p, *q),
                _ => return Err(ClearanceError::NotResolvable(key.clone())),
            },
        };
        let label = |id: Uuid| op.bodies.get(&id).and_then(|b| b.label.clone());
        let (mover, other) = match (op.movable(p), op.movable(q)) {
            (true, true) => {
                if (label(p), p) > (label(q), q) {
                    (p, q)
                } else {
                    (q, p)
                }
            }
            (true, false) => (p, q),
            (false, true) => (q, p),
            (false, false) => return Err(ClearanceError::NotResolvable(key.clone())),
        };
        pairs.push(((label(mover), mover), (label(other), other)));
    }
    pairs.sort();
    for ((_, mover), (_, other)) in pairs {
        let d = op.twice_center(mover) - op.twice_center(other);
        let dir = if d < 0 { -1 } else { 1 };
        let needed = op.need(other, mover, dir);
        if needed > 0 {
            op.push(mover, dir, needed, &mut vec![mover])?;
        }
    }

    let base = history.version().to_string();
    let commands = op
        .delta
        .iter()
        .filter(|(_, d)| **d != 0)
        .map(|(id, d)| {
            let delta = i64::try_from(*d)
                .map(Length::from_nanometers)
                .map_err(|_| ClearanceError::DidNotSettle)?;
            Ok(Command {
                id: Uuid::derive(*id, "clearance_resolve", &base),
                idempotency_key: Uuid::derive(*id, "clearance_resolve-key", &base),
                action: Action::MoveOccurrence(MoveOccurrence {
                    occurrence: *id,
                    axis,
                    motion: Motion::Delta { delta },
                    edit: None,
                    absorb: Vec::new(),
                }),
            })
        })
        .collect::<Result<Vec<_>, ClearanceError>>()?;
    propose(history, &ProposedAction::Commands { commands }).map_err(ClearanceError::Propose)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{Action, DimensionTarget, SetConstraintValue};
    use crate::document::*;
    use crate::items::tests::{IN, base, floor, len, put, uid};
    use crate::proposal::ProposedAction;

    const WALL: Uuid = Uuid::from_bytes([2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);

    fn label(doc: &mut Document, id: Uuid, n: u32) {
        if let (Some(o), Ok(l)) = (
            doc.occurrences.get_mut(&id),
            Label::try_from(format!("EQ-{n:03}")),
        ) {
            o.label = l;
        }
    }

    fn overlap(a: Uuid, b: Uuid) -> ViolationKey {
        let mut entities = vec![a, b];
        entities.sort();
        ViolationKey {
            kind: ViolationKind::OccurrenceOverlap,
            entities,
        }
    }

    fn request(movable: Vec<Uuid>, axis: Axis, violations: Vec<ViolationKey>) -> ClearanceRequest {
        ClearanceRequest {
            movable,
            axis,
            violations,
        }
    }

    /// Resolved edits of the proposal: (occurrence, axis, new center in nm).
    fn moves(p: &Proposal) -> Vec<(Uuid, Axis, i64)> {
        let ProposedAction::Commands { commands } = &p.resolved else {
            return vec![];
        };
        commands
            .iter()
            .filter_map(|c| match c.action {
                Action::SetConstraintValue(SetConstraintValue {
                    target: DimensionTarget::OccurrenceCoordinate { occurrence, axis },
                    value,
                }) => Some((occurrence, axis, value.nanometers())),
                _ => None,
            })
            .collect()
    }

    /// A bench 48 x 24 in. The wall's footprint is y = -2 to 2 in.
    #[test]
    fn a_single_push_clears_the_wall() -> Result<(), ClearanceError> {
        let mut doc = base();
        // Center y = 10: footprint y = -2 to 22, into the wall by 4 in.
        let a = put(&mut doc, 1, floor(100, 10, 0), (100, 10));
        let history = History::new(doc);
        let p = clearance_resolve(
            &history,
            &request(
                vec![a],
                wall_normal_axis(history.document(), WALL)?,
                vec![overlap(a, WALL)],
            ),
        )?;
        assert_eq!(moves(&p), vec![(a, Axis::Y, 14 * IN)]);
        assert!(p.violations.is_empty(), "{:?}", p.violations);
        assert_eq!(history.version(), 0, "nothing is committed");
        Ok(())
    }

    #[test]
    fn shifts_round_up_to_the_grid() -> Result<(), ClearanceError> {
        let mut doc = base();
        // 1/32 in further into the wall: the exact shift 4 1/32 in rounds up to 4 1/16 in.
        let a = put(&mut doc, 1, floor(100, 10, 0), (100, 10));
        if let Some(o) = doc.occurrences.get_mut(&a) {
            o.center.y = len(10 * IN - IN / 32);
        }
        let p = clearance_resolve(
            &History::new(doc),
            &request(vec![a], Axis::Y, vec![overlap(a, WALL)]),
        )?;
        assert_eq!(
            moves(&p),
            vec![(a, Axis::Y, 10 * IN - IN / 32 + 4 * IN + IN / 16)]
        );
        Ok(())
    }

    /// Two benches in a row: pushing the first off the wall pushes the second.
    fn row() -> (Document, Uuid, Uuid) {
        let mut doc = base();
        let a = put(&mut doc, 1, floor(100, 10, 0), (100, 10));
        // y = 24 to 48: after A moves to y = 2..26, they overlap by 2 in.
        let b = put(&mut doc, 2, floor(100, 36, 0), (100, 36));
        label(&mut doc, a, 1);
        label(&mut doc, b, 2);
        (doc, a, b)
    }

    #[test]
    fn a_chained_push_moves_the_next_item() -> Result<(), ClearanceError> {
        let (doc, a, b) = row();
        let p = clearance_resolve(
            &History::new(doc),
            &request(vec![a, b], Axis::Y, vec![overlap(a, WALL)]),
        )?;
        let mut expected = vec![(a, Axis::Y, 14 * IN), (b, Axis::Y, 38 * IN)];
        expected.sort();
        let mut got = moves(&p);
        got.sort();
        assert_eq!(got, expected);
        assert!(p.violations.is_empty(), "{:?}", p.violations);
        Ok(())
    }

    #[test]
    fn a_fixed_item_blocks_the_chain() {
        let (mut doc, a, b) = row();
        if let Some(o) = doc.occurrences.get_mut(&b) {
            o.fixed = true;
        }
        assert_eq!(
            clearance_resolve(
                &History::new(doc),
                &request(vec![a, b], Axis::Y, vec![overlap(a, WALL)])
            ),
            Err(ClearanceError::Blocked {
                chain: vec![a, b],
                blocker: b
            })
        );
        // An item the request does not allow to move blocks just the same.
        let (doc, a, b) = row();
        assert_eq!(
            clearance_resolve(
                &History::new(doc),
                &request(vec![a], Axis::Y, vec![overlap(a, WALL)])
            ),
            Err(ClearanceError::Blocked {
                chain: vec![a, b],
                blocker: b
            })
        );
    }

    #[test]
    fn a_wall_blocks_the_chain() {
        // Pushing the bench toward -Y off a second wall runs it into the first.
        let mut doc = base();
        let (n3, n4) = (uid(1, 3), uid(1, 4));
        doc.nodes.insert(
            n3,
            Node {
                x: len(0),
                y: len(-30 * IN),
                join: JoinType::FreeEnd,
            },
        );
        doc.nodes.insert(
            n4,
            Node {
                x: len(240 * IN),
                y: len(-30 * IN),
                join: JoinType::FreeEnd,
            },
        );
        let mut second = doc
            .walls
            .get(&WALL)
            .cloned()
            .unwrap_or_else(|| unreachable!());
        second.node_a = n3;
        second.node_b = n4;
        let w2 = uid(2, 2);
        doc.walls.insert(w2, second);
        // Center y = -20: footprint y = -32 to -8, into the second wall (y = -32 to -28).
        let a = put(&mut doc, 1, floor(100, -20, 0), (100, -20));
        let mut bad = doc.clone();
        if let Some(o) = bad.occurrences.get_mut(&a) {
            o.center.y = len(-23 * IN);
        }
        // Footprint y = -35 to -11: pushed +Y off w2 by 7 in to -28..-4, clear of the first wall.
        let p = clearance_resolve(
            &History::new(bad),
            &request(vec![a], Axis::Y, vec![overlap(a, w2)]),
        );
        assert_eq!(p.map(|p| moves(&p)), Ok(vec![(a, Axis::Y, -16 * IN)]));
        // A 40 in deep item cannot fit in the 26 in gap: pushing it off w2 hits the first wall.
        if let Some(t) = doc.element_types.get_mut(&Uuid::from_bytes([13; 16])) {
            t.declared_dimensions.depth = len(40 * IN);
        }
        let chain = vec![a, WALL];
        assert_eq!(
            clearance_resolve(
                &History::new(doc),
                &request(vec![a], Axis::Y, vec![overlap(a, w2)])
            ),
            Err(ClearanceError::Blocked {
                chain,
                blocker: WALL
            })
        );
    }

    /// Ties are broken by label order: the later label moves.
    #[test]
    fn a_label_order_tie_moves_the_later_label() -> Result<(), ClearanceError> {
        let mut doc = base();
        let a = put(&mut doc, 1, floor(100, 60, 0), (100, 60));
        let b = put(&mut doc, 2, floor(110, 70, 0), (110, 70));
        label(&mut doc, a, 2);
        label(&mut doc, b, 1);
        // b has the earlier label (EQ-001) and holds; a (EQ-002) moves away along X.
        let p = clearance_resolve(
            &History::new(doc.clone()),
            &request(vec![a, b], Axis::X, vec![overlap(a, b)]),
        )?;
        // a: x 76..124 is left of b (x 86..134): it moves -X by 124 - 86 = 38 in.
        assert_eq!(moves(&p), vec![(a, Axis::X, 62 * IN)]);
        // Swap the labels and the other one moves instead.
        label(&mut doc, a, 1);
        label(&mut doc, b, 2);
        let p = clearance_resolve(
            &History::new(doc),
            &request(vec![a, b], Axis::X, vec![overlap(a, b)]),
        )?;
        assert_eq!(moves(&p), vec![(b, Axis::X, 148 * IN)]);
        Ok(())
    }

    #[test]
    fn min_clearance_pushes_to_the_exact_distance() -> Result<(), ClearanceError> {
        // A: x 76..124, y 88..112. B: x 127..175, y 116..140 (gap 3 by 4: 5 in).
        let mut doc = base();
        let a = put(&mut doc, 1, floor(100, 100, 0), (100, 100));
        let b = put(&mut doc, 2, floor(151, 128, 0), (151, 128));
        label(&mut doc, a, 1);
        label(&mut doc, b, 2);
        let c = uid(3, 50);
        doc.constraints.insert(
            c,
            Constraint {
                kind: ConstraintKind::MinClearance {
                    object: a,
                    other: b,
                    value: len(13 * IN),
                },
                status: ConstraintStatus::Driving,
                source: Source::UserStated,
                provenance: None,
            },
        );
        let key = ViolationKey {
            kind: ViolationKind::MinClearance,
            entities: vec![c],
        };
        // Along X with a 4 in gap across: needs sqrt(13^2 - 4^2) = sqrt(153) = 12.37 in, so 12 3/8 in on the grid.
        let p = clearance_resolve(&History::new(doc), &request(vec![b], Axis::X, vec![key]))?;
        let gap = 12 * IN + 3 * IN / 8;
        assert_eq!(moves(&p), vec![(b, Axis::X, 151 * IN + gap - 3 * IN)]);
        assert!(p.violations.is_empty(), "{:?}", p.violations);
        Ok(())
    }

    #[test]
    fn only_current_violations_can_be_resolved() {
        let mut doc = base();
        let a = put(&mut doc, 1, floor(100, 60, 0), (100, 60));
        let key = overlap(a, WALL);
        assert_eq!(
            clearance_resolve(
                &History::new(doc),
                &request(vec![a], Axis::Y, vec![key.clone()])
            ),
            Err(ClearanceError::NotResolvable(key))
        );
    }
}
