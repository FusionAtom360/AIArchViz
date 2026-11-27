//! Editing under constraints (spec 8.7; Q-37 to Q-40).
//!
//! A move is resolved on the solver's graph for its axis (`axis_graph`):
//! variables, editable dimensions (constraint values and placement offsets),
//! rigid ties (walls, Align, Equal copies) and Fixed-item anchors. Rigid ties
//! are contracted first, so collinear joined walls move as one. Then:
//!
//! - nothing links the moved reference to ground: translate its whole free
//!   class (Q-38);
//! - a Fixed anchor holds the moved entity: case (d), obstructed;
//! - the editable bridge nearest the moved reference (an edge whose removal
//!   cuts it off from ground) touches ground: case (a), change it by the
//!   delta; it does not: case (b), change it by the delta, everything beyond
//!   it moves, and each editable edge leaving the moved entity inside that
//!   moving part is offered as a PolicyChoice that absorbs the move (changes
//!   by minus the delta) so what lies beyond it holds still;
//! - no bridge: case (c), a ReferenceSemantics ambiguity over the edges
//!   joining the moved entity to its routes to ground; once one is picked,
//!   every such edge is updated so all routes stay consistent (Q-40).
//!
//! Every resolution is a list of SetConstraintValue edits (Q-39). The
//! entities that move are found by solving the edited document.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use crate::command::{
    Action, Command, CommandError, DimensionTarget, Motion, SetConstraintValue, apply,
};
use crate::document::{Axis, Direction, Document, OpeningEdge, Placement, Reference};
use crate::identity::Uuid;
use crate::length::Length;
use crate::solver::{
    AxisGraph, EdgeKind, LayerA, Owner, SolveError, Solver, axis_graph, resolve_reference,
};

/// Which rule of 8.7 applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Case {
    /// (a) one dimension ties the moved reference straight to a datum or Fixed value.
    A,
    /// (b) the reference is fixed through a chain.
    B,
    /// (c) two or more independent routes; resolved by the user's pick.
    C,
    /// Nothing fixes the reference (Q-38).
    Free,
}

/// One way to resolve a move, with its resulting dimensions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The dimension this option is about (a constraint, opening or occurrence id).
    pub choice: Uuid,
    pub edits: Vec<SetConstraintValue>,
    /// Entities other than the moved one that move under this option.
    pub moved: Vec<Uuid>,
}

/// How a move resolves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveOutcome {
    Edits {
        case: Case,
        edits: Vec<SetConstraintValue>,
        moved: Vec<Uuid>,
        /// Case (b): PolicyChoices that absorb the move instead.
        alternatives: Vec<Candidate>,
    },
    /// Case (c) without a pick: the user must say which dimension changes.
    Ambiguous { candidates: Vec<Candidate> },
    /// Case (d): these Fixed occurrences would have to move.
    Obstructed { fixed: Vec<Uuid>, delta: Length },
}

/// Why a move cannot be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveError {
    Solve(SolveError),
    Command(CommandError),
    /// The reference does not belong to the moved entity.
    WrongReference {
        entity: Uuid,
    },
    /// The entity cannot move on that axis.
    WrongAxis {
        entity: Uuid,
    },
    UnknownEntity {
        id: Uuid,
    },
    /// `edit` or `absorb` names a dimension that is not one of the options.
    InvalidChoice(Uuid),
    /// The chosen dimensions cannot absorb the move without moving a datum.
    Infeasible,
    Overflow,
}

impl fmt::Display for MoveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MoveError::Solve(e) => write!(f, "{e}"),
            MoveError::Command(e) => write!(f, "{e}"),
            MoveError::WrongReference { entity } => {
                write!(f, "the reference does not belong to {entity}")
            }
            MoveError::WrongAxis { entity } => write!(f, "{entity} cannot move on that axis"),
            MoveError::UnknownEntity { id } => write!(f, "no entity {id}"),
            MoveError::InvalidChoice(id) => write!(f, "{id} is not an option for this move"),
            MoveError::Infeasible => f.write_str("the chosen dimensions cannot absorb the move"),
            MoveError::Overflow => f.write_str("a dimension is out of range"),
        }
    }
}

impl std::error::Error for MoveError {}

/// A move in solver terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MoveRequest {
    pub entity: Uuid,
    pub reference: Reference,
    pub axis: Axis,
    pub motion: Motion,
    pub edit: Option<Uuid>,
    pub absorb: Vec<Uuid>,
}

/// The request for a Move command, or None for any other command.
pub(crate) fn request_for(
    doc: &Document,
    action: &Action,
) -> Option<Result<MoveRequest, MoveError>> {
    Some(match action {
        Action::MoveWall(m) => (|| {
            let w = doc
                .walls
                .get(&m.wall)
                .ok_or(MoveError::UnknownEntity { id: m.wall })?;
            let own = match m.reference {
                Reference::WallCenter { wall } | Reference::WallFace { wall, .. } => wall == m.wall,
                _ => false,
            };
            if !own {
                return Err(MoveError::WrongReference { entity: m.wall });
            }
            let axis = match w.direction {
                Direction::PosX | Direction::NegX => Axis::Y,
                Direction::PosY | Direction::NegY => Axis::X,
            };
            Ok(MoveRequest {
                entity: m.wall,
                reference: m.reference,
                axis,
                motion: m.motion,
                edit: m.edit,
                absorb: m.absorb.clone(),
            })
        })(),
        Action::MoveOpening(m) => (|| {
            let o = doc
                .openings
                .get(&m.opening)
                .ok_or(MoveError::UnknownEntity { id: m.opening })?;
            let host = doc
                .walls
                .get(&o.host)
                .ok_or(MoveError::UnknownEntity { id: o.host })?;
            Ok(MoveRequest {
                entity: m.opening,
                reference: Reference::OpeningEdge {
                    opening: m.opening,
                    edge: OpeningEdge::Start,
                },
                axis: along(host.direction),
                motion: m.motion,
                edit: m.edit,
                absorb: m.absorb.clone(),
            })
        })(),
        Action::MoveOccurrence(m) => (|| {
            let o = doc
                .occurrences
                .get(&m.occurrence)
                .ok_or(MoveError::UnknownEntity { id: m.occurrence })?;
            if let Placement::WallMounted { wall, .. } = &o.placement {
                let host = doc
                    .walls
                    .get(wall)
                    .ok_or(MoveError::UnknownEntity { id: *wall })?;
                if along(host.direction) != m.axis {
                    return Err(MoveError::WrongAxis {
                        entity: m.occurrence,
                    });
                }
            }
            Ok(MoveRequest {
                entity: m.occurrence,
                reference: Reference::ItemCenter { item: m.occurrence },
                axis: m.axis,
                motion: m.motion,
                edit: m.edit,
                absorb: m.absorb.clone(),
            })
        })(),
        _ => return None,
    })
}

fn along(direction: Direction) -> Axis {
    match direction {
        Direction::PosX | Direction::NegX => Axis::X,
        Direction::PosY | Direction::NegY => Axis::Y,
    }
}

/// The graph with rigid ties contracted: each variable's group, and the
/// non-rigid edges between groups.
struct Contracted {
    group: Vec<usize>,
    /// (edge index, group a, group b).
    edges: Vec<(usize, usize, usize)>,
}

fn contract(g: &AxisGraph) -> Contracted {
    let n = g.owners.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    for e in &g.edges {
        if e.kind == EdgeKind::Rigid {
            let (a, b) = (find(&mut parent, e.a), find(&mut parent, e.b));
            if a != b {
                let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                parent[hi] = lo;
            }
        }
    }
    let group: Vec<usize> = (0..n).map(|i| find(&mut parent, i)).collect();
    let edges = g
        .edges
        .iter()
        .enumerate()
        .filter(|(_, e)| e.kind != EdgeKind::Rigid)
        .map(|(i, e)| (i, group[e.a], group[e.b]))
        .filter(|(_, a, b)| a != b)
        .collect();
    Contracted { group, edges }
}

/// Groups reachable from `start` without crossing `skip` edges or entering `avoid`.
fn reach(
    c: &Contracted,
    start: usize,
    skip: &BTreeSet<usize>,
    avoid: Option<usize>,
) -> BTreeSet<usize> {
    let mut seen = BTreeSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(v) = queue.pop_front() {
        for &(i, a, b) in &c.edges {
            if skip.contains(&i) {
                continue;
            }
            let w = if a == v {
                b
            } else if b == v {
                a
            } else {
                continue;
            };
            if Some(w) != avoid && seen.insert(w) {
                queue.push_back(w);
            }
        }
    }
    seen
}

/// A path of edge indices from `from` to `to`, by breadth-first search.
fn path(c: &Contracted, from: usize, to: usize) -> Vec<usize> {
    let mut prev: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    let mut queue = VecDeque::from([from]);
    let mut seen = BTreeSet::from([from]);
    while let Some(v) = queue.pop_front() {
        if v == to {
            break;
        }
        for &(i, a, b) in &c.edges {
            let w = if a == v {
                b
            } else if b == v {
                a
            } else {
                continue;
            };
            if seen.insert(w) {
                prev.insert(w, (v, i));
                queue.push_back(w);
            }
        }
    }
    let mut out = Vec::new();
    let mut v = to;
    while let Some(&(p, i)) = prev.get(&v) {
        out.push(i);
        v = p;
    }
    out.reverse();
    out
}

/// What moves when `change` edges take up the move: the groups pulled
/// along with `r`, or the Fixed items or datum that would have to move.
fn propagate(
    g: &AxisGraph,
    c: &Contracted,
    r: usize,
    change: &BTreeSet<usize>,
) -> Result<BTreeSet<usize>, Result<Vec<Uuid>, MoveError>> {
    let ground = c.group[0];
    let mut moving = BTreeSet::from([r]);
    let mut queue = VecDeque::from([r]);
    let mut fixed = BTreeSet::new();
    while let Some(v) = queue.pop_front() {
        for &(i, a, b) in &c.edges {
            let w = if a == v {
                b
            } else if b == v {
                a
            } else {
                continue;
            };
            if change.contains(&i) || moving.contains(&w) {
                continue;
            }
            if let EdgeKind::Anchor(item) = g.edges[i].kind {
                fixed.insert(item);
                continue;
            }
            if w == ground {
                return Err(Err(MoveError::Infeasible));
            }
            moving.insert(w);
            queue.push_back(w);
        }
    }
    if fixed.is_empty() {
        Ok(moving)
    } else {
        Err(Ok(fixed.into_iter().collect()))
    }
}

/// The edits that make `change` edges take up a move of `delta`.
fn edits_for(
    g: &AxisGraph,
    c: &Contracted,
    moving: &BTreeSet<usize>,
    change: &BTreeSet<usize>,
    delta: i128,
) -> Result<Vec<SetConstraintValue>, MoveError> {
    let mut out = Vec::new();
    for &i in change {
        let e = g.edges[i];
        let EdgeKind::Editable {
            target,
            value,
            coef,
        } = e.kind
        else {
            continue;
        };
        let (in_a, in_b) = (
            moving.contains(&c.group[e.a]),
            moving.contains(&c.group[e.b]),
        );
        if in_a == in_b {
            continue;
        }
        let side = if in_b { 1 } else { -1 };
        let new = value + side * coef * delta;
        let new = i64::try_from(new).map_err(|_| MoveError::Overflow)?;
        out.push(SetConstraintValue {
            target,
            value: Length::from_nanometers(new),
        });
    }
    out.sort();
    Ok(out)
}

/// Entities other than `entity` whose solved position changes under `edits`.
fn moved(
    doc: &Document,
    entity: Uuid,
    edits: &[SetConstraintValue],
) -> Result<Vec<Uuid>, MoveError> {
    let mut after = doc.clone();
    for (i, e) in edits.iter().enumerate() {
        let cmd = Command {
            id: Uuid::derive(entity, "edit", &i.to_string()),
            idempotency_key: Uuid::derive(entity, "edit-key", &i.to_string()),
            action: Action::SetConstraintValue(*e),
        };
        after = apply(&after, &cmd).map_err(MoveError::Command)?;
    }
    let solver = LayerA::default();
    let (s0, s1) = (
        solver.solve(doc).map_err(MoveError::Solve)?,
        solver.solve(&after).map_err(MoveError::Solve)?,
    );
    let mut out = BTreeSet::new();
    let changed_nodes: BTreeSet<Uuid> = s0
        .nodes
        .iter()
        .filter(|(id, p)| s1.nodes.get(id) != Some(p))
        .map(|(id, _)| *id)
        .collect();
    for (id, w) in &doc.walls {
        if changed_nodes.contains(&w.node_a) || changed_nodes.contains(&w.node_b) {
            out.insert(*id);
        }
    }
    for (id, o) in &doc.openings {
        if s0.openings.get(id) != s1.openings.get(id) || out.contains(&o.host) {
            out.insert(*id);
        }
    }
    for (id, o) in &doc.occurrences {
        let on_moved_wall =
            matches!(&o.placement, Placement::WallMounted { wall, .. } if out.contains(wall));
        if s0.occurrences.get(id) != s1.occurrences.get(id) || on_moved_wall {
            out.insert(*id);
        }
    }
    out.remove(&entity);
    Ok(out.into_iter().collect())
}

fn choice_edges(g: &AxisGraph, ids: &[Uuid]) -> Result<BTreeSet<usize>, MoveError> {
    let mut out = BTreeSet::new();
    for id in ids {
        let found: Vec<usize> = g
            .edges
            .iter()
            .enumerate()
            .filter(
                |(_, e)| matches!(e.kind, EdgeKind::Editable { target, .. } if target.id() == *id),
            )
            .map(|(i, _)| i)
            .collect();
        if found.is_empty() {
            return Err(MoveError::InvalidChoice(*id));
        }
        out.extend(found);
    }
    Ok(out)
}

fn edge_id(g: &AxisGraph, i: usize) -> Option<Uuid> {
    match g.edges[i].kind {
        EdgeKind::Editable { target, .. } => Some(target.id()),
        _ => None,
    }
}

/// Resolves a move into edits (8.7).
pub(crate) fn resolve_move(doc: &Document, req: &MoveRequest) -> Result<MoveOutcome, MoveError> {
    let g = axis_graph(doc, req.axis).map_err(MoveError::Solve)?;
    let (rv, rc) = resolve_reference(doc, &req.reference, req.axis).map_err(MoveError::Solve)?;
    let current = g.values.get(rv).copied().unwrap_or(0) + rc;
    let delta = match req.motion {
        Motion::Delta { delta } => i128::from(delta.nanometers()),
        Motion::Target { target } => i128::from(target.nanometers()) - current,
    };
    let delta_len = Length::from_nanometers(i64::try_from(delta).map_err(|_| MoveError::Overflow)?);
    let c = contract(&g);
    let r = c.group[rv];
    let ground = c.group[0];
    let none = BTreeSet::new();

    // Nothing links it to ground: translate the free class (Q-38).
    let class = reach(&c, r, &none, None);
    if !class.contains(&ground) {
        let mut edits = Vec::new();
        for (var, owner) in g.owners.iter().enumerate() {
            if !class.contains(&c.group[var]) {
                continue;
            }
            let target = match owner {
                Owner::Node(node) => DimensionTarget::NodeCoordinate {
                    node: *node,
                    axis: req.axis,
                },
                Owner::Occurrence(id) => match doc.occurrences.get(id).map(|o| &o.placement) {
                    Some(Placement::Floor { .. } | Placement::CeilingMounted { .. }) => {
                        DimensionTarget::OccurrenceCoordinate {
                            occurrence: *id,
                            axis: req.axis,
                        }
                    }
                    _ => continue,
                },
                _ => continue,
            };
            let value = i64::try_from(g.values[var] + delta).map_err(|_| MoveError::Overflow)?;
            edits.push(SetConstraintValue {
                target,
                value: Length::from_nanometers(value),
            });
        }
        edits.sort();
        let moved = moved(doc, req.entity, &edits)?;
        return Ok(MoveOutcome::Edits {
            case: Case::Free,
            edits,
            moved,
            alternatives: vec![],
        });
    }

    // (d) A Fixed anchor holds the moved entity.
    let anchored: Vec<Uuid> = c
        .edges
        .iter()
        .filter(|(_, a, b)| *a == r || *b == r)
        .filter_map(|(i, _, _)| match g.edges[*i].kind {
            EdgeKind::Anchor(item) => Some(item),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if !anchored.is_empty() {
        return Ok(MoveOutcome::Obstructed {
            fixed: anchored,
            delta: delta_len,
        });
    }

    let absorb = choice_edges(&g, &req.absorb)?;
    // The editable bridge nearest the moved reference on a path to ground.
    let route = path(&c, r, ground);
    let bridge = route.iter().copied().find(|&i| {
        matches!(g.edges[i].kind, EdgeKind::Editable { .. })
            && !reach(&c, r, &BTreeSet::from([i]), None).contains(&ground)
    });
    let finish = |change: BTreeSet<usize>| -> Result<(Vec<SetConstraintValue>, BTreeSet<usize>), MoveOutcome> {
        match propagate(&g, &c, r, &change) {
            Ok(moving) => Ok((edits_for(&g, &c, &moving, &change, delta).map_err(|_| MoveOutcome::Obstructed { fixed: vec![], delta: delta_len })?, moving)),
            Err(Ok(fixed)) => Err(MoveOutcome::Obstructed { fixed, delta: delta_len }),
            Err(Err(_)) => Err(MoveOutcome::Obstructed { fixed: vec![], delta: delta_len }),
        }
    };
    match bridge {
        Some(e) => {
            let (ea, eb) = (c.group[g.edges[e].a], c.group[g.edges[e].b]);
            let case = if ea == ground || eb == ground {
                Case::A
            } else {
                Case::B
            };
            let mut change = absorb.clone();
            change.insert(e);
            let (edits, moving) = match finish(change.clone()) {
                Ok(x) => x,
                Err(outcome) => return Ok(outcome),
            };
            let mut alternatives = Vec::new();
            if case == Case::B && absorb.is_empty() {
                // Each editable edge leaving the moved entity inside the moving part.
                for &(i, a, b) in &c.edges {
                    let inside = moving.contains(&a) && moving.contains(&b);
                    if i == e || !(a == r || b == r) || !inside {
                        continue;
                    }
                    let Some(choice) = edge_id(&g, i) else {
                        continue;
                    };
                    let mut alt = change.clone();
                    alt.insert(i);
                    if let Ok((alt_edits, _)) = finish(alt) {
                        let alt_moved = moved(doc, req.entity, &alt_edits)?;
                        alternatives.push(Candidate {
                            choice,
                            edits: alt_edits,
                            moved: alt_moved,
                        });
                    }
                }
                alternatives.sort_by_key(|a| a.choice);
            }
            let moved = moved(doc, req.entity, &edits)?;
            Ok(MoveOutcome::Edits {
                case,
                edits,
                moved,
                alternatives,
            })
        }
        None => {
            // (c) Several routes: the editable edges at the moved entity that
            // still reach ground without passing through it.
            let routes: BTreeSet<usize> = c
                .edges
                .iter()
                .filter(|(i, a, b)| {
                    let other = if *a == r {
                        *b
                    } else if *b == r {
                        *a
                    } else {
                        return false;
                    };
                    matches!(g.edges[*i].kind, EdgeKind::Editable { .. })
                        && (other == ground || reach(&c, other, &none, Some(r)).contains(&ground))
                })
                .map(|(i, _, _)| *i)
                .collect();
            let mut change = routes.clone();
            change.extend(absorb.iter().copied());
            let (edits, _) = match finish(change) {
                Ok(x) => x,
                Err(outcome) => return Ok(outcome),
            };
            let moved = moved(doc, req.entity, &edits)?;
            match req.edit {
                Some(pick) => {
                    if !routes.iter().any(|&i| edge_id(&g, i) == Some(pick)) {
                        return Err(MoveError::InvalidChoice(pick));
                    }
                    Ok(MoveOutcome::Edits {
                        case: Case::C,
                        edits,
                        moved,
                        alternatives: vec![],
                    })
                }
                None => {
                    let choices: BTreeSet<Uuid> =
                        routes.iter().filter_map(|&i| edge_id(&g, i)).collect();
                    let candidates = choices
                        .into_iter()
                        .map(|choice| Candidate {
                            choice,
                            edits: edits.clone(),
                            moved: moved.clone(),
                        })
                        .collect();
                    Ok(MoveOutcome::Ambiguous { candidates })
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::command::{MoveOccurrence, MoveOpening, MoveWall};
    use crate::document::*;
    use crate::identity::LabelAllocator;
    use crate::units::ProjectUnit;

    pub(crate) const IN: i64 = 25_400_000;

    pub(crate) fn len(nm: i64) -> Length {
        Length::from_nanometers(nm)
    }

    pub(crate) fn uid(kind: u8, n: u8) -> Uuid {
        let mut b = [0u8; 16];
        b[0] = kind;
        b[15] = n;
        Uuid::from_bytes(b)
    }

    fn sourced(nm: i64) -> Sourced<Length> {
        Sourced {
            value: len(nm),
            source: Source::UserStated,
        }
    }

    pub(crate) const STOREY: Uuid = Uuid::from_bytes([8; 16]);
    pub(crate) const WEST: Uuid =
        Uuid::from_bytes([4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    pub(crate) const EAST: Uuid =
        Uuid::from_bytes([4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]);

    /// A storey, a 4 in layer set, and datum lines x = 0 (west) and x = 240 in (east).
    pub(crate) fn base() -> Document {
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
        doc.datums.insert(
            WEST,
            Datum {
                name: "West".into(),
                geometry: DatumGeometry::XLine { x: len(0) },
            },
        );
        doc.datums.insert(
            EAST,
            Datum {
                name: "East".into(),
                geometry: DatumGeometry::XLine { x: len(240 * IN) },
            },
        );
        doc
    }

    /// A wall running +Y at x (inches), from y = 0 to 96, with its own nodes.
    pub(crate) fn wall_at(doc: &mut Document, n: u8, x: i64) -> Uuid {
        let (a, b) = (uid(1, 2 * n), uid(1, 2 * n + 1));
        doc.nodes.insert(
            a,
            Node {
                x: len(x * IN),
                y: len(0),
                join: JoinType::FreeEnd,
            },
        );
        doc.nodes.insert(
            b,
            Node {
                x: len(x * IN),
                y: len(96 * IN),
                join: JoinType::FreeEnd,
            },
        );
        let mut labels = LabelAllocator::new();
        let id = uid(2, n);
        doc.walls.insert(
            id,
            Wall {
                label: labels.allocate("W").unwrap_or_else(|_| unreachable!()),
                storey: STOREY,
                node_a: a,
                node_b: b,
                direction: Direction::PosY,
                layer_set: uid(5, 0),
                left_extent: sourced(2 * IN),
                base_offset: sourced(0),
                top: WallTop::Storey,
                join_priority: 0,
                function: Function::New,
            },
        );
        id
    }

    pub(crate) fn center(wall: Uuid) -> Reference {
        Reference::WallCenter { wall }
    }

    pub(crate) fn datum(d: Uuid) -> Reference {
        Reference::Datum { datum: d }
    }

    pub(crate) fn dist(doc: &mut Document, n: u8, a: Reference, b: Reference, inches: i64) -> Uuid {
        let id = uid(3, n);
        doc.constraints.insert(
            id,
            Constraint {
                kind: ConstraintKind::Distance {
                    a,
                    b,
                    axis: Axis::X,
                    value: len(inches * IN),
                    tolerance: None,
                },
                status: ConstraintStatus::Driving,
                source: Source::UserStated,
                provenance: None,
            },
        );
        id
    }

    fn fix_y(doc: &mut Document, n: u8, node: Uuid, inches: i64) -> Uuid {
        let id = uid(3, n);
        doc.constraints.insert(
            id,
            Constraint {
                kind: ConstraintKind::Fixed {
                    reference: Reference::Node { node },
                    axis: Axis::Y,
                    value: len(inches * IN),
                },
                status: ConstraintStatus::Driving,
                source: Source::UserStated,
                provenance: None,
            },
        );
        id
    }

    fn set(target: DimensionTarget, inches: i64) -> SetConstraintValue {
        SetConstraintValue {
            target,
            value: len(inches * IN),
        }
    }

    fn con(id: Uuid) -> DimensionTarget {
        DimensionTarget::Constraint { constraint: id }
    }

    pub(crate) fn move_wall(
        doc: &Document,
        wall: Uuid,
        delta: i64,
        edit: Option<Uuid>,
        absorb: Vec<Uuid>,
    ) -> Result<MoveOutcome, MoveError> {
        let action = Action::MoveWall(MoveWall {
            wall,
            reference: center(wall),
            motion: Motion::Delta {
                delta: len(delta * IN),
            },
            edit,
            absorb,
        });
        let req = request_for(doc, &action).unwrap_or(Err(MoveError::Infeasible))?;
        resolve_move(doc, &req)
    }

    /// (a) A wall placed straight from a datum: that one distance changes.
    #[test]
    fn case_a_edits_the_one_dimension() -> Result<(), MoveError> {
        let mut doc = base();
        let a = wall_at(&mut doc, 1, 120);
        let c1 = dist(&mut doc, 1, datum(WEST), center(a), 120);
        assert_eq!(
            move_wall(&doc, a, 12, None, vec![])?,
            MoveOutcome::Edits {
                case: Case::A,
                edits: vec![set(con(c1), 132)],
                moved: vec![],
                alternatives: vec![]
            }
        );
        Ok(())
    }

    /// A chain: west datum -120- A -96- B -72- C, with a window on C.
    pub(crate) fn chain() -> (Document, [Uuid; 3], [Uuid; 3], Uuid) {
        let mut doc = base();
        let a = wall_at(&mut doc, 1, 120);
        let b = wall_at(&mut doc, 2, 216);
        let c = wall_at(&mut doc, 3, 288);
        let c1 = dist(&mut doc, 1, datum(WEST), center(a), 120);
        let c2 = dist(&mut doc, 2, center(a), center(b), 96);
        let c3 = dist(&mut doc, 3, center(b), center(c), 72);
        let window = uid(10, 1);
        doc.openings.insert(
            window,
            Opening {
                kind: OpeningKind::Window,
                host: c,
                reference: Reference::Node { node: uid(1, 6) },
                anchor: OpeningEdge::Start,
                offset: sourced(24 * IN),
                width: sourced(36 * IN),
                height: sourced(48 * IN),
                sill: sourced(36 * IN),
                start: len(24 * IN),
            },
        );
        (doc, [a, b, c], [c1, c2, c3], window)
    }

    /// (b) Done when: a chain edit lists every downstream entity that moves
    /// and offers each alternative with its resulting dimensions.
    #[test]
    fn case_b_chain_lists_consequences_and_alternatives() -> Result<(), MoveError> {
        let (doc, [_, b, c], [_, c2, c3], window) = chain();
        let mut downstream = vec![c, window];
        downstream.sort();
        assert_eq!(
            move_wall(&doc, b, 12, None, vec![])?,
            MoveOutcome::Edits {
                case: Case::B,
                edits: vec![set(con(c2), 108)],
                moved: downstream,
                alternatives: vec![Candidate {
                    choice: c3,
                    edits: vec![set(con(c2), 108), set(con(c3), 60)],
                    moved: vec![]
                }],
            }
        );
        // Taking the alternative: C and its window hold still.
        assert_eq!(
            move_wall(&doc, b, 12, None, vec![c3])?,
            MoveOutcome::Edits {
                case: Case::B,
                edits: vec![set(con(c2), 108), set(con(c3), 60)],
                moved: vec![],
                alternatives: vec![]
            }
        );
        assert_eq!(
            move_wall(&doc, b, 12, None, vec![uid(3, 99)]),
            Err(MoveError::InvalidChoice(uid(3, 99)))
        );
        Ok(())
    }

    /// (c) A wall held from both datums: the user must pick; then both update (Q-40).
    #[test]
    fn case_c_asks_which_dimension_changes() -> Result<(), MoveError> {
        let mut doc = base();
        let a = wall_at(&mut doc, 1, 120);
        let c1 = dist(&mut doc, 1, datum(WEST), center(a), 120);
        let c4 = dist(&mut doc, 4, center(a), datum(EAST), 120);
        let both = vec![set(con(c1), 132), set(con(c4), 108)];
        assert_eq!(
            move_wall(&doc, a, 12, None, vec![])?,
            MoveOutcome::Ambiguous {
                candidates: vec![
                    Candidate {
                        choice: c1,
                        edits: both.clone(),
                        moved: vec![]
                    },
                    Candidate {
                        choice: c4,
                        edits: both.clone(),
                        moved: vec![]
                    },
                ]
            }
        );
        assert_eq!(
            move_wall(&doc, a, 12, Some(c4), vec![])?,
            MoveOutcome::Edits {
                case: Case::C,
                edits: both,
                moved: vec![],
                alternatives: vec![]
            }
        );
        assert_eq!(
            move_wall(&doc, a, 12, Some(uid(3, 77)), vec![]),
            Err(MoveError::InvalidChoice(uid(3, 77)))
        );
        Ok(())
    }

    /// (d) A Fixed obstruction: moving it, or a wall holding one, is a Conflict.
    #[test]
    fn case_d_fixed_obstructions_refuse() -> Result<(), MoveError> {
        let mut doc = base();
        let a = wall_at(&mut doc, 1, 120);
        dist(&mut doc, 1, datum(WEST), center(a), 120);
        let box_type = uid(13, 0);
        doc.element_types.insert(
            box_type,
            crate::items::proxy_box(
                "Panel",
                len(24 * IN),
                len(6 * IN),
                len(36 * IN),
                Authority::UserStatedExact,
            ),
        );
        let panel = uid(14, 1);
        let mut labels = LabelAllocator::new();
        doc.occurrences.insert(
            panel,
            Occurrence {
                label: labels.allocate("EQ").unwrap_or_else(|_| unreachable!()),
                storey: STOREY,
                element_type: box_type,
                placement: Placement::WallMounted {
                    wall: a,
                    side: WallSide::Right,
                    reference: Reference::Node { node: uid(1, 2) },
                    offset: sourced(48 * IN),
                    elevation: sourced(48 * IN),
                },
                clearance: Clearance {
                    front: sourced(0),
                    back: sourced(0),
                    left: sourced(0),
                    right: sourced(0),
                },
                fixed: true,
                overlap_allowed: false,
                light: None,
                // On the right face (x = 122), 6 in deep toward +X: center x = 125.
                center: Point {
                    x: len(125 * IN),
                    y: len(48 * IN),
                },
            },
        );
        assert_eq!(
            move_wall(&doc, a, 12, None, vec![])?,
            MoveOutcome::Obstructed {
                fixed: vec![panel],
                delta: len(12 * IN)
            }
        );
        let column = uid(14, 2);
        let mut labels = LabelAllocator::new();
        doc.occurrences.insert(
            column,
            Occurrence {
                label: labels.allocate("EQ").unwrap_or_else(|_| unreachable!()),
                storey: STOREY,
                element_type: box_type,
                placement: Placement::Floor {
                    x: sourced(60 * IN),
                    y: sourced(40 * IN),
                    rotation: QuarterTurns::new(0).unwrap_or_else(|| unreachable!()),
                    elevation: sourced(0),
                },
                clearance: Clearance {
                    front: sourced(0),
                    back: sourced(0),
                    left: sourced(0),
                    right: sourced(0),
                },
                fixed: true,
                overlap_allowed: false,
                light: None,
                center: Point {
                    x: len(60 * IN),
                    y: len(40 * IN),
                },
            },
        );
        let action = Action::MoveOccurrence(MoveOccurrence {
            occurrence: column,
            axis: Axis::X,
            motion: Motion::Delta { delta: len(6 * IN) },
            edit: None,
            absorb: vec![],
        });
        let req = request_for(&doc, &action).unwrap_or(Err(MoveError::Infeasible))?;
        assert_eq!(
            resolve_move(&doc, &req)?,
            MoveOutcome::Obstructed {
                fixed: vec![column],
                delta: len(6 * IN)
            }
        );
        Ok(())
    }

    /// Q-38: nothing fixes the wall, so its free class is translated.
    #[test]
    fn free_classes_are_translated() -> Result<(), MoveError> {
        let mut doc = base();
        let f = wall_at(&mut doc, 1, 120);
        let node = |n| DimensionTarget::NodeCoordinate {
            node: uid(1, n),
            axis: Axis::X,
        };
        assert_eq!(
            move_wall(&doc, f, -12, None, vec![])?,
            MoveOutcome::Edits {
                case: Case::Free,
                edits: vec![set(node(2), 108), set(node(3), 108)],
                moved: vec![],
                alternatives: vec![]
            }
        );
        Ok(())
    }

    #[test]
    fn openings_and_occurrences_move_to_a_target() -> Result<(), MoveError> {
        let (mut doc, _, _, window) = chain();
        fix_y(&mut doc, 10, uid(1, 6), 0);
        let action = Action::MoveOpening(MoveOpening {
            opening: window,
            motion: Motion::Target {
                target: len(36 * IN),
            },
            edit: None,
            absorb: vec![],
        });
        let req = request_for(&doc, &action).unwrap_or(Err(MoveError::Infeasible))?;
        let outcome = resolve_move(&doc, &req)?;
        assert_eq!(
            outcome,
            MoveOutcome::Edits {
                case: Case::B,
                edits: vec![set(DimensionTarget::OpeningOffset { opening: window }, 36)],
                moved: vec![],
                alternatives: vec![]
            }
        );
        // A free floor item moves to a target center.
        let mut doc = base();
        let box_type = uid(13, 0);
        doc.element_types.insert(
            box_type,
            crate::items::proxy_box(
                "Bench",
                len(48 * IN),
                len(24 * IN),
                len(30 * IN),
                Authority::UserStatedExact,
            ),
        );
        let bench = uid(14, 3);
        let mut labels = LabelAllocator::new();
        doc.occurrences.insert(
            bench,
            Occurrence {
                label: labels.allocate("EQ").unwrap_or_else(|_| unreachable!()),
                storey: STOREY,
                element_type: box_type,
                placement: Placement::Floor {
                    x: sourced(60 * IN),
                    y: sourced(40 * IN),
                    rotation: QuarterTurns::new(0).unwrap_or_else(|| unreachable!()),
                    elevation: sourced(0),
                },
                clearance: Clearance {
                    front: sourced(0),
                    back: sourced(0),
                    left: sourced(0),
                    right: sourced(0),
                },
                fixed: false,
                overlap_allowed: false,
                light: None,
                center: Point {
                    x: len(60 * IN),
                    y: len(40 * IN),
                },
            },
        );
        let action = Action::MoveOccurrence(MoveOccurrence {
            occurrence: bench,
            axis: Axis::Y,
            motion: Motion::Target {
                target: len(50 * IN),
            },
            edit: None,
            absorb: vec![],
        });
        let req = request_for(&doc, &action).unwrap_or(Err(MoveError::Infeasible))?;
        assert_eq!(
            resolve_move(&doc, &req)?,
            MoveOutcome::Edits {
                case: Case::Free,
                edits: vec![set(
                    DimensionTarget::OccurrenceCoordinate {
                        occurrence: bench,
                        axis: Axis::Y
                    },
                    50
                )],
                moved: vec![],
                alternatives: vec![]
            }
        );
        Ok(())
    }

    #[test]
    fn a_wall_moves_only_by_its_own_reference_across_its_axis() {
        let (doc, [a, b, _], _, _) = chain();
        let action = Action::MoveWall(MoveWall {
            wall: a,
            reference: center(b),
            motion: Motion::Delta { delta: len(IN) },
            edit: None,
            absorb: vec![],
        });
        assert_eq!(
            request_for(&doc, &action),
            Some(Err(MoveError::WrongReference { entity: a }))
        );
    }
}
