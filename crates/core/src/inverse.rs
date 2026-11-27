//! Inverse deltas of move and set-value commands (spec 13.2, 8.3; Q-75).
//!
//! The inverse of a committed MoveWall, MoveOpening or MoveOccurrence is the
//! same move (same entity, reference, axis, `edit` and `absorb`) by minus
//! the displacement it made, measured in the document before the commit; a
//! target becomes the delta that reached it. The inverse of a
//! SetConstraintValue sets the value its target held before. A commit of
//! several commands is inverted command by command, each against the state
//! the commit had reached when it ran, and the inverses are returned in
//! reverse order.
//!
//! The inverse restores values only (Q-75 (a), DEC-11). A move stamps the
//! offsets it sets UserStated, and `derive` prunes an acknowledgment whose
//! violation is gone; the inverse brings back neither. Undo (8.4) is the
//! exact restore.

use std::fmt;

use crate::command::{
    Action, Command, CommandError, DimensionTarget, Motion, SetConstraintValue, apply,
};
use crate::document::{Axis, ConstraintKind, Document, Placement};
use crate::edit::{MoveError, MoveOutcome, displacement, request_for, resolve_move};
use crate::identity::Uuid;
use crate::length::Length;
use crate::proposal::{DeriveError, derive};

/// Why a commit's commands have no inverse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InverseError {
    /// Command `index` is not a move or a SetConstraintValue.
    NotInvertible {
        index: usize,
        kind: &'static str,
    },
    /// Command `index` does not resolve to edits in this document (an
    /// ambiguity without a pick, or an obstruction): it cannot have been
    /// committed.
    NotCommitted {
        index: usize,
    },
    Move {
        index: usize,
        error: MoveError,
    },
    Command {
        index: usize,
        error: CommandError,
    },
    Derive(DeriveError),
    /// Minus the displacement is out of range.
    Overflow {
        index: usize,
    },
}

impl fmt::Display for InverseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InverseError::NotInvertible { index, kind } => {
                write!(f, "command {index}: {kind} has no inverse delta")
            }
            InverseError::NotCommitted { index } => {
                write!(f, "command {index} does not resolve to edits")
            }
            InverseError::Move { index, error } => write!(f, "command {index}: {error}"),
            InverseError::Command { index, error } => write!(f, "command {index}: {error}"),
            InverseError::Derive(e) => write!(f, "{e}"),
            InverseError::Overflow { index } => {
                write!(
                    f,
                    "command {index}: the inverse displacement is out of range"
                )
            }
        }
    }
}

impl std::error::Error for InverseError {}

/// The value `target` holds in `doc`.
fn prior_value(doc: &Document, target: DimensionTarget) -> Result<Length, CommandError> {
    let unknown = CommandError::UnknownEntity(target.id());
    let not = CommandError::NotADimension(target.id());
    match target {
        DimensionTarget::Constraint { constraint } => {
            match &doc.constraints.get(&constraint).ok_or(unknown)?.kind {
                ConstraintKind::Distance { value, .. } | ConstraintKind::Fixed { value, .. } => {
                    Ok(*value)
                }
                _ => Err(not),
            }
        }
        DimensionTarget::OpeningOffset { opening } => {
            Ok(doc.openings.get(&opening).ok_or(unknown)?.offset.value)
        }
        DimensionTarget::OccurrenceOffset { occurrence } => {
            match &doc.occurrences.get(&occurrence).ok_or(unknown)?.placement {
                Placement::WallMounted { offset, .. } => Ok(offset.value),
                _ => Err(not),
            }
        }
        DimensionTarget::NodeCoordinate { node, axis } => {
            let n = doc.nodes.get(&node).ok_or(unknown)?;
            Ok(match axis {
                Axis::X => n.x,
                Axis::Y => n.y,
            })
        }
        DimensionTarget::OccurrenceCoordinate { occurrence, axis } => {
            match &doc.occurrences.get(&occurrence).ok_or(unknown)?.placement {
                Placement::Floor { x, y, .. } | Placement::CeilingMounted { x, y } => {
                    Ok(match axis {
                        Axis::X => x.value,
                        Axis::Y => y.value,
                    })
                }
                Placement::WallMounted { .. } => Err(not),
            }
        }
    }
}

/// The inverses of `commands`, committed together on `before`, in the order
/// they undo the commit (the last command's inverse first). Each inverse
/// takes its id and idempotency key from the command it inverts.
pub fn inverse(before: &Document, commands: &[Command]) -> Result<Vec<Command>, InverseError> {
    let mut state = before.clone();
    let mut out = Vec::new();
    for (index, c) in commands.iter().enumerate() {
        let command_error = |error| InverseError::Command { index, error };
        let move_error = |error| InverseError::Move { index, error };
        let (action, edits) = match &c.action {
            Action::SetConstraintValue(args) => {
                let value = prior_value(&state, args.target).map_err(command_error)?;
                let undo = Action::SetConstraintValue(SetConstraintValue {
                    target: args.target,
                    value,
                });
                (undo, vec![*args])
            }
            Action::MoveWall(_) | Action::MoveOpening(_) | Action::MoveOccurrence(_) => {
                let Some(request) = request_for(&state, &c.action) else {
                    return Err(InverseError::NotInvertible {
                        index,
                        kind: c.action.kind(),
                    });
                };
                let request = request.map_err(move_error)?;
                // Measured as `propose` resolves it: on the solved state (8.7).
                let current = derive(state.clone())
                    .map_err(InverseError::Derive)?
                    .document;
                let back = displacement(&current, &request)
                    .map_err(move_error)?
                    .checked_mul(-1)
                    .map_err(|_| InverseError::Overflow { index })?;
                let edits = match resolve_move(&current, &request).map_err(move_error)? {
                    MoveOutcome::Edits { edits, .. } => edits,
                    MoveOutcome::Ambiguous { .. } | MoveOutcome::Obstructed { .. } => {
                        return Err(InverseError::NotCommitted { index });
                    }
                };
                let motion = Motion::Delta { delta: back };
                let mut undo = c.action.clone();
                match &mut undo {
                    Action::MoveWall(m) => m.motion = motion,
                    Action::MoveOpening(m) => m.motion = motion,
                    Action::MoveOccurrence(m) => m.motion = motion,
                    _ => {}
                }
                (undo, edits)
            }
            other => {
                return Err(InverseError::NotInvertible {
                    index,
                    kind: other.kind(),
                });
            }
        };
        // Advance to the state the next command saw, as `propose` does.
        for e in edits {
            let edit = Command {
                id: c.id,
                idempotency_key: c.idempotency_key,
                action: Action::SetConstraintValue(e),
            };
            state = apply(&state, &edit).map_err(command_error)?;
        }
        out.push(Command {
            id: Uuid::derive(c.id, "inverse", ""),
            idempotency_key: Uuid::derive(c.idempotency_key, "inverse", ""),
            action,
        });
    }
    out.reverse();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::to_canonical_json;
    use crate::clearance::{ClearanceRequest, clearance_resolve};
    use crate::command::{MoveOccurrence, MoveOpening, MoveWall};
    use crate::document::*;
    use crate::edit::tests::{
        EAST, IN, STOREY, WEST, base, center, chain, datum, len, uid, wall_at,
    };
    use crate::identity::Label;
    use crate::log::{Event, History, LogEntry};
    use crate::proposal::{Proposal, ProposedAction, commit, propose};
    use proptest::prelude::*;

    const SIXTEENTH: i64 = IN / 16;

    fn bytes(doc: &Document) -> String {
        to_canonical_json(doc).unwrap_or_else(|e| e.to_string())
    }

    fn cmd(n: u8, action: Action) -> Command {
        Command {
            id: uid(40, n),
            idempotency_key: uid(41, n),
            action,
        }
    }

    fn sourced(nm: i64) -> Sourced<Length> {
        Sourced {
            value: len(nm),
            source: Source::UserStated,
        }
    }

    fn commands(cs: Vec<Command>) -> ProposedAction {
        ProposedAction::Commands { commands: cs }
    }

    /// A history whose head is `doc` derived, as a committed document is.
    fn history(doc: Document) -> Result<History, String> {
        Ok(History::new(
            derive(doc).map_err(|e| e.to_string())?.document,
        ))
    }

    /// Proposes and commits; the error says why when it cannot.
    fn commit_all(h: &mut History, cs: Vec<Command>) -> Result<Proposal, String> {
        let p = propose(h, &commands(cs)).map_err(|e| e.to_string())?;
        if p.blocked() {
            return Err(format!("blocked: {:?} {:?}", p.violations, p.conflicts));
        }
        commit(h, &p, 0, "user").map_err(|e| e.to_string())?;
        Ok(p)
    }

    /// Applies a proposal's resolved commands as a commit event, the way
    /// commit and replay do, without the commit's blocking check.
    fn apply_resolved(h: &mut History, p: &Proposal) -> Result<(), String> {
        let ProposedAction::Commands { commands } = &p.resolved else {
            return Err("not commands".into());
        };
        let entry = LogEntry {
            sequence: h.version() + 1,
            timestamp_ms: 0,
            actor: "user".into(),
            event: Event::Commit {
                commands: commands.clone(),
            },
        };
        h.apply_entry(&entry).map_err(|e| e.to_string())
    }

    fn fix_y(doc: &mut Document, n: u8, node: Uuid) {
        doc.constraints.insert(
            uid(3, n),
            Constraint {
                kind: ConstraintKind::Fixed {
                    reference: Reference::Node { node },
                    axis: Axis::Y,
                    value: len(0),
                },
                status: ConstraintStatus::Driving,
                source: Source::UserStated,
                provenance: None,
            },
        );
    }

    fn move_wall(wall: Uuid, motion: Motion, edit: Option<Uuid>, absorb: Vec<Uuid>) -> Action {
        Action::MoveWall(MoveWall {
            wall,
            reference: center(wall),
            motion,
            edit,
            absorb,
        })
    }

    fn delta(nm: i64) -> Motion {
        Motion::Delta { delta: len(nm) }
    }

    fn actions(cs: &[Command]) -> Vec<Action> {
        cs.iter().map(|c| c.action.clone()).collect()
    }

    #[test]
    fn a_set_value_inverse_sets_the_prior_value() -> Result<(), String> {
        let (doc, _, [_, c2, _], _) = chain();
        let mut h = history(doc)?;
        let before = h.document().clone();
        let target = DimensionTarget::Constraint { constraint: c2 };
        let forward = cmd(
            1,
            Action::SetConstraintValue(SetConstraintValue {
                target,
                value: len(100 * IN),
            }),
        );
        commit_all(&mut h, vec![forward.clone()])?;
        let undo = inverse(&before, &[forward]).map_err(|e| e.to_string())?;
        assert_eq!(
            actions(&undo),
            vec![Action::SetConstraintValue(SetConstraintValue {
                target,
                value: len(96 * IN)
            })]
        );
        commit_all(&mut h, undo)?;
        assert_eq!(bytes(h.document()), bytes(&before));
        Ok(())
    }

    /// A case (b) chain move carries C and its window along (8.7); the
    /// inverse brings all three back.
    #[test]
    fn a_chain_move_and_its_consequences_come_back() -> Result<(), String> {
        let (doc, [_, b, c], _, window) = chain();
        let mut h = history(doc)?;
        let before = h.document().clone();
        let forward = cmd(1, move_wall(b, delta(12 * IN), None, vec![]));
        let p = commit_all(&mut h, vec![forward.clone()])?;
        let mut moved = vec![c, window];
        moved.sort();
        assert_eq!(p.consequences, moved);
        let undo = inverse(&before, &[forward]).map_err(|e| e.to_string())?;
        assert_eq!(
            actions(&undo),
            vec![move_wall(b, delta(-12 * IN), None, vec![])]
        );
        commit_all(&mut h, undo)?;
        assert_eq!(bytes(h.document()), bytes(&before));
        Ok(())
    }

    /// A target becomes the displacement that reached it.
    #[test]
    fn a_target_move_inverts_by_its_measured_displacement() -> Result<(), String> {
        let (doc, [a, _, _], _, _) = chain();
        let mut h = history(doc)?;
        let before = h.document().clone();
        let target = Motion::Target {
            target: len(100 * IN),
        };
        let forward = cmd(1, move_wall(a, target, None, vec![]));
        commit_all(&mut h, vec![forward.clone()])?;
        let undo = inverse(&before, &[forward]).map_err(|e| e.to_string())?;
        assert_eq!(
            actions(&undo),
            vec![move_wall(a, delta(20 * IN), None, vec![])]
        );
        commit_all(&mut h, undo)?;
        assert_eq!(bytes(h.document()), bytes(&before));
        Ok(())
    }

    /// A bench (48 x 24 in) on the items base, whose wall runs +X at y = 0
    /// with footprint y = -2 to 2 in.
    fn bench(doc: &mut Document, n: u8, x: i64, y: i64) -> Uuid {
        let id = crate::items::tests::put(doc, n, crate::items::tests::floor(x, y, 0), (x, y));
        if let (Some(o), Ok(l)) = (
            doc.occurrences.get_mut(&id),
            Label::try_from(format!("EQ-{n:03}")),
        ) {
            o.label = l;
        }
        id
    }

    const ITEMS_WALL: Uuid = Uuid::from_bytes([2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);

    fn overlap(a: Uuid, b: Uuid) -> ViolationKey {
        let mut entities = vec![a, b];
        entities.sort();
        ViolationKey {
            kind: ViolationKind::OccurrenceOverlap,
            entities,
        }
    }

    fn move_occurrence(occurrence: Uuid, axis: Axis, motion: Motion) -> Action {
        Action::MoveOccurrence(MoveOccurrence {
            occurrence,
            axis,
            motion,
            edit: None,
            absorb: vec![],
        })
    }

    /// 8.8: pushing A off the wall pushes B. The inverse of the commit moves
    /// both back and the document is byte-identical; the overlap the push
    /// fixed is back, so the inverse proposal is blocked like any proposal
    /// that creates a violation (Q-35).
    #[test]
    fn a_push_chain_comes_back() -> Result<(), String> {
        let mut doc = crate::items::tests::base();
        let a = bench(&mut doc, 1, 100, 10);
        let b = bench(&mut doc, 2, 100, 36);
        let mut h = history(doc)?;
        let before = h.document().clone();
        let p = clearance_resolve(
            &h,
            &ClearanceRequest {
                movable: vec![a, b],
                axis: Axis::Y,
                violations: vec![overlap(a, ITEMS_WALL)],
            },
        )
        .map_err(|e| e.to_string())?;
        commit(&mut h, &p, 0, "user").map_err(|e| e.to_string())?;
        let ProposedAction::Commands { commands: forward } = &p.action else {
            return Err("not commands".into());
        };
        assert_eq!(forward.len(), 2, "A and the item it pushed");
        let undo = inverse(&before, forward).map_err(|e| e.to_string())?;
        let mut got = actions(&undo);
        got.sort_by_key(|a| format!("{a:?}"));
        let mut want = vec![
            move_occurrence(a, Axis::Y, delta(-4 * IN)),
            move_occurrence(b, Axis::Y, delta(-2 * IN)),
        ];
        want.sort_by_key(|a| format!("{a:?}"));
        assert_eq!(got, want);
        let q = propose(&h, &commands(undo)).map_err(|e| e.to_string())?;
        assert!(q.blocked());
        assert_eq!(
            q.violations
                .iter()
                .map(|v| v.key.clone())
                .collect::<Vec<_>>(),
            vec![overlap(a, ITEMS_WALL)]
        );
        apply_resolved(&mut h, &q)?;
        assert_eq!(bytes(h.document()), bytes(&before));
        Ok(())
    }

    /// Q-75 exception 1: a Measured offset comes back with its value but
    /// stated by the user, because a move states the dimension (Q-39).
    #[test]
    fn a_moved_offset_comes_back_user_stated() -> Result<(), String> {
        let (mut doc, _, _, window) = chain();
        fix_y(&mut doc, 10, uid(1, 6));
        if let Some(o) = doc.openings.get_mut(&window) {
            o.offset.source = Source::Measured;
        }
        let mut h = history(doc)?;
        let before = h.document().clone();
        let forward = cmd(
            1,
            Action::MoveOpening(MoveOpening {
                opening: window,
                motion: delta(6 * IN),
                edit: None,
                absorb: vec![],
            }),
        );
        commit_all(&mut h, vec![forward.clone()])?;
        let undo = inverse(&before, &[forward]).map_err(|e| e.to_string())?;
        commit_all(&mut h, undo)?;
        let mut restored = h.document().clone();
        let offset = restored.openings.get_mut(&window).map(|o| &mut o.offset);
        assert_eq!(offset.as_deref(), Some(&sourced(24 * IN)));
        if let Some(o) = offset {
            o.source = Source::Measured;
        }
        assert_eq!(bytes(&restored), bytes(&before), "only the source differs");
        Ok(())
    }

    /// Q-75 exception 2: a move that clears an acknowledged violation prunes
    /// the acknowledgment; the inverse brings the violation back
    /// unacknowledged, so its proposal is blocked.
    #[test]
    fn a_pruned_acknowledgment_does_not_come_back() -> Result<(), String> {
        let mut doc = crate::items::tests::base();
        let a = bench(&mut doc, 1, 100, 10);
        doc.acknowledged.insert(overlap(a, ITEMS_WALL));
        let mut h = history(doc)?;
        let before = h.document().clone();
        assert_eq!(before.acknowledged.len(), 1);
        let forward = cmd(1, move_occurrence(a, Axis::Y, delta(4 * IN)));
        commit_all(&mut h, vec![forward.clone()])?;
        assert!(h.document().acknowledged.is_empty());
        let undo = inverse(&before, &[forward]).map_err(|e| e.to_string())?;
        let q = propose(&h, &commands(undo)).map_err(|e| e.to_string())?;
        assert!(q.blocked());
        apply_resolved(&mut h, &q)?;
        let mut restored = h.document().clone();
        assert!(restored.acknowledged.is_empty());
        restored.acknowledged = before.acknowledged.clone();
        assert_eq!(
            bytes(&restored),
            bytes(&before),
            "only the acknowledgment differs"
        );
        Ok(())
    }

    #[test]
    fn other_commands_have_no_inverse_delta() {
        let c = cmd(
            1,
            Action::CreateNode(crate::command::CreateNode {
                x: len(0),
                y: len(0),
            }),
        );
        assert_eq!(
            inverse(&base(), &[c]),
            Err(InverseError::NotInvertible {
                index: 0,
                kind: "create_node"
            })
        );
    }

    // Property test: random documents and random commands of each kind.

    /// A random plan: walls running +Y in a chain from the west datum (each
    /// spacing in sixteenths), a window and a 24 in panel on chain walls, a
    /// free bench north of the walls, and optionally an east pin (case
    /// (c)), a free wall (Q-38) and a Fixed y on the first wall's start node.
    #[derive(Debug, Clone)]
    struct Plan {
        spacings: Vec<i64>,
        east_pin: bool,
        free_wall: bool,
        fix_y: bool,
        opening: (usize, i64),
        panel: (usize, i64),
        bench_x: i64,
    }

    const BENCH_Y: i64 = 140 * 16;
    const FREE_X: i64 = -60 * 16;
    const OPENING: Uuid = Uuid::from_bytes([10; 16]);
    const PANEL: Uuid = Uuid::from_bytes([14; 16]);
    const BENCH: Uuid = Uuid::from_bytes([15; 16]);
    const FREE_WALL: u8 = 9;

    fn plan() -> impl Strategy<Value = Plan> {
        (
            proptest::collection::vec(48 * 16..=72 * 16i64, 1..=3),
            any::<bool>(),
            any::<bool>(),
            any::<bool>(),
            (0usize..3, 6 * 16..=30 * 16i64),
            (0usize..3, 72 * 16..=80 * 16i64),
            30 * 16..=210 * 16i64,
        )
            .prop_map(
                |(spacings, east_pin, free_wall, fix_y, opening, panel, bench_x)| Plan {
                    spacings,
                    east_pin,
                    free_wall,
                    fix_y,
                    opening,
                    panel,
                    bench_x,
                },
            )
    }

    fn small(n: usize) -> u8 {
        u8::try_from(n).unwrap_or(u8::MAX)
    }

    fn wall_id(i: usize) -> Uuid {
        uid(2, small(i + 1))
    }

    /// Chain link i joins wall i - 1 (or the west datum) to wall i.
    fn link_id(i: usize) -> Uuid {
        uid(3, small(i + 1))
    }

    fn start_node(i: usize) -> Uuid {
        uid(1, 2 * small(i + 1))
    }

    fn sixteenths(s: i64) -> Length {
        len(s * SIXTEENTH)
    }

    fn distance(doc: &mut Document, id: Uuid, a: Reference, b: Reference, s: i64) {
        doc.constraints.insert(
            id,
            Constraint {
                kind: ConstraintKind::Distance {
                    a,
                    b,
                    axis: Axis::X,
                    value: sixteenths(s),
                    tolerance: None,
                },
                status: ConstraintStatus::Driving,
                source: Source::UserStated,
                provenance: None,
            },
        );
    }

    fn occurrence(
        label: &str,
        element_type: Uuid,
        placement: Placement,
        center: Point,
    ) -> Result<Occurrence, String> {
        Ok(Occurrence {
            label: Label::try_from(label.to_string()).map_err(|e| e.to_string())?,
            storey: STOREY,
            element_type,
            placement,
            clearance: Clearance {
                front: sourced(0),
                back: sourced(0),
                left: sourced(0),
                right: sourced(0),
            },
            fixed: false,
            overlap_allowed: false,
            light: None,
            center,
        })
    }

    fn build(p: &Plan) -> Result<Document, String> {
        let mut doc = base();
        let k = p.spacings.len();
        let mut x = 0;
        for (i, s) in p.spacings.iter().enumerate() {
            x += s;
            let w = wall_at(&mut doc, small(i + 1), x / 16);
            let from = match i {
                0 => datum(WEST),
                _ => center(wall_id(i - 1)),
            };
            distance(&mut doc, link_id(i), from, center(w), *s);
        }
        if p.east_pin {
            let last = center(wall_id(k - 1));
            distance(&mut doc, link_id(k), last, datum(EAST), 240 * 16 - x);
        }
        if p.free_wall {
            wall_at(&mut doc, FREE_WALL, FREE_X / 16);
        }
        if p.fix_y {
            fix_y(&mut doc, 20, start_node(0));
        }
        for (i, w) in doc.walls.values_mut().enumerate() {
            w.label = Label::try_from(format!("W-{:03}", i + 1)).map_err(|e| e.to_string())?;
        }
        let (ow, offset) = (p.opening.0 % k, p.opening.1);
        doc.openings.insert(
            OPENING,
            Opening {
                kind: OpeningKind::Window,
                host: wall_id(ow),
                reference: Reference::Node {
                    node: start_node(ow),
                },
                anchor: OpeningEdge::Start,
                offset: sourced(offset * SIXTEENTH),
                width: sourced(36 * IN),
                height: sourced(48 * IN),
                sill: sourced(36 * IN),
                start: sixteenths(offset),
            },
        );
        let (panel_type, bench_type) = (uid(13, 1), uid(13, 2));
        let exact = Authority::UserStatedExact;
        let (w24, d6, h12) = (len(24 * IN), len(6 * IN), len(12 * IN));
        let panel = crate::items::proxy_box("Panel", w24, d6, h12, exact);
        doc.element_types.insert(panel_type, panel);
        let (w48, h30) = (len(48 * IN), len(30 * IN));
        let bench = crate::items::proxy_box("Bench", w48, w24, h30, exact);
        doc.element_types.insert(bench_type, bench);
        let (pw, poff) = (p.panel.0 % k, p.panel.1);
        let mounted = Placement::WallMounted {
            wall: wall_id(pw),
            side: WallSide::Right,
            reference: Reference::Node {
                node: start_node(pw),
            },
            offset: sourced(poff * SIXTEENTH),
            elevation: sourced(0),
        };
        let at = Point {
            x: len(0),
            y: sixteenths(poff),
        };
        doc.occurrences
            .insert(PANEL, occurrence("EQ-001", panel_type, mounted, at)?);
        let floor = Placement::Floor {
            x: sourced(p.bench_x * SIXTEENTH),
            y: sourced(BENCH_Y * SIXTEENTH),
            rotation: QuarterTurns::new(0).ok_or("rotation")?,
            elevation: sourced(0),
        };
        let at = Point {
            x: sixteenths(p.bench_x),
            y: sixteenths(BENCH_Y),
        };
        doc.occurrences
            .insert(BENCH, occurrence("EQ-002", bench_type, floor, at)?);
        Ok(doc)
    }

    /// A command of one of the four kinds in plan terms; `d` is in sixteenths.
    #[derive(Debug, Clone)]
    enum Op {
        Wall {
            which: usize,
            target: bool,
            absorb: bool,
            d: i64,
        },
        Opening {
            target: bool,
            d: i64,
        },
        Occurrence {
            bench: bool,
            x: bool,
            target: bool,
            d: i64,
        },
        Set {
            which: usize,
            link: usize,
            d: i64,
        },
    }

    fn op() -> impl Strategy<Value = Op> {
        let d = -12 * 16..=12 * 16i64;
        prop_oneof![
            (0usize..4, any::<bool>(), any::<bool>(), d.clone()).prop_map(
                |(which, target, absorb, d)| Op::Wall {
                    which,
                    target,
                    absorb,
                    d
                }
            ),
            (any::<bool>(), d.clone()).prop_map(|(target, d)| Op::Opening { target, d }),
            (any::<bool>(), any::<bool>(), any::<bool>(), d.clone()).prop_map(
                |(bench, x, target, d)| Op::Occurrence {
                    bench,
                    x,
                    target,
                    d
                }
            ),
            (0usize..5, 0usize..3, d).prop_map(|(which, link, d)| Op::Set { which, link, d }),
        ]
    }

    fn motion(target: bool, at: i64, d: i64) -> Motion {
        if target {
            Motion::Target {
                target: sixteenths(at + d),
            }
        } else {
            delta(d * SIXTEENTH)
        }
    }

    fn set(target: DimensionTarget, s: i64) -> Action {
        Action::SetConstraintValue(SetConstraintValue {
            target,
            value: sixteenths(s),
        })
    }

    fn action(p: &Plan, op: &Op) -> Action {
        let k = p.spacings.len();
        match *op {
            Op::Wall {
                which,
                target,
                absorb,
                d,
            } => {
                let i = which % (k + usize::from(p.free_wall));
                if i == k {
                    let free = uid(2, FREE_WALL);
                    return move_wall(free, motion(target, FREE_X, d), None, vec![]);
                }
                let at: i64 = p.spacings.iter().take(i + 1).sum();
                let edit = p.east_pin.then(|| link_id(i));
                let absorb = if absorb && !p.east_pin && i + 1 < k {
                    vec![link_id(i + 1)]
                } else {
                    vec![]
                };
                move_wall(wall_id(i), motion(target, at, d), edit, absorb)
            }
            Op::Opening { target, d } => Action::MoveOpening(MoveOpening {
                opening: OPENING,
                motion: motion(target, p.opening.1, d),
                edit: None,
                absorb: vec![],
            }),
            Op::Occurrence {
                bench,
                x,
                target,
                d,
            } => {
                let (occurrence, axis, at) = match (bench, x) {
                    (true, true) => (BENCH, Axis::X, p.bench_x),
                    (true, false) => (BENCH, Axis::Y, BENCH_Y),
                    (false, _) => (PANEL, Axis::Y, p.panel.1),
                };
                move_occurrence(occurrence, axis, motion(target, at, d))
            }
            Op::Set { which, link, d } => {
                let l = link % k;
                let spacing = p.spacings.get(l).copied().unwrap_or(0);
                match which {
                    0 => set(
                        DimensionTarget::Constraint {
                            constraint: link_id(l),
                        },
                        spacing + d,
                    ),
                    1 => set(
                        DimensionTarget::OpeningOffset { opening: OPENING },
                        p.opening.1 + d,
                    ),
                    2 => set(
                        DimensionTarget::OccurrenceOffset { occurrence: PANEL },
                        p.panel.1 + d,
                    ),
                    3 => set(
                        DimensionTarget::OccurrenceCoordinate {
                            occurrence: BENCH,
                            axis: Axis::X,
                        },
                        p.bench_x + d,
                    ),
                    _ if p.free_wall => set(
                        DimensionTarget::NodeCoordinate {
                            node: uid(1, 2 * FREE_WALL + 1),
                            axis: Axis::Y,
                        },
                        96 * 16 + d,
                    ),
                    _ => set(
                        DimensionTarget::OccurrenceCoordinate {
                            occurrence: BENCH,
                            axis: Axis::Y,
                        },
                        BENCH_Y + d,
                    ),
                }
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(96))]

        /// 13.2: committing a move or set-value command and then its inverse
        /// gives back the document byte for byte (offsets UserStated and no
        /// acknowledgments: Q-75 (a)).
        #[test]
        fn a_command_then_its_inverse_is_byte_identical(p in plan(), op in op()) {
            let doc = build(&p).map_err(TestCaseError::fail)?;
            let derived = derive(doc).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assume!(derived.violations.is_empty());
            let mut h = History::new(derived.document);
            let before = h.document().clone();
            let forward = cmd(1, action(&p, &op));
            prop_assume!(commit_all(&mut h, vec![forward.clone()]).is_ok());
            let undo = inverse(&before, std::slice::from_ref(&forward))
                .map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert_eq!(actions(&undo).iter().map(Action::kind).collect::<Vec<_>>(), vec![forward.action.kind()]);
            commit_all(&mut h, undo).map_err(TestCaseError::fail)?;
            prop_assert_eq!(bytes(h.document()), bytes(&before));
        }
    }
}
