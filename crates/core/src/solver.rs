//! Solver Layer A: exact axis-offset solving (spec 7.1 to 7.5, D-6; Q-12 to Q-16).
//!
//! Each axis is solved on its own. Every node has one coordinate variable per
//! axis, and each axis has a ground variable fixed at 0 that carries absolute
//! values (Datum, Fixed). A wall ties its nodes' coordinates on the axis
//! perpendicular to its direction, which is its reference line; wall faces
//! and the wall center are that coordinate plus a constant offset. Every
//! driving Distance, Align and Fixed constraint merges two variable classes
//! in a weighted union-find holding the exact integer offset between them.
//!
//! Driving constraints are processed in a fixed order, by source authority
//! (UserStated, Imported, Measured, Derived, Proposed) and then by id
//! (Q-15), so the result never depends on insertion order. A constraint whose
//! references are already related is redundant: consistent if it agrees,
//! demoted to Reference if it is Measured and within tolerance (7.5), and
//! otherwise a [`Conflict`] naming every constraint on the path it closes.
//!
//! A class without ground is underconstrained by one translation. For a
//! provisional model (7.4) it keeps the stored position of its lowest-id node.
//!
//! References to openings and items are added by T-108 and T-111 (Q-16).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use crate::document::{
    Axis, Constraint, ConstraintKind, ConstraintStatus, DatumGeometry, Direction, Document, Point,
    Reference, Source, WallSide,
};
use crate::identity::Uuid;
use crate::length::Length;

/// A constraint solver (D-6). Implementations must be pure.
pub trait Solver {
    fn solve(&self, document: &Document) -> Result<Solution, SolveError>;
}

/// The exact axis-offset solver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerA {
    /// Tolerance for Measured constraints that state none (7.5).
    pub default_tolerance: Length,
}

impl Default for LayerA {
    /// A default tolerance of 1/4 in.
    fn default() -> Self {
        LayerA {
            default_tolerance: Length::from_nanometers(6_350_000),
        }
    }
}

/// What the solver made of one constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Merged two classes: it determines something.
    Driving,
    /// Driving, but its references were already related and it agrees.
    Redundant,
    /// A redundant Measured constraint within tolerance: now Reference (7.5).
    Demoted,
    /// Redundant and disagreeing beyond tolerance; see [`Solution::conflicts`].
    Conflict,
    /// A Reference-status constraint: reported, constrains nothing (7.2).
    Reference,
    /// Not solved by Layer A (MinClearance is checked after solving, 7.1).
    NotSolved,
}

/// The solver's report for one constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConstraintReport {
    pub outcome: Outcome,
    /// The value in force: the stored value, or the value copied by Equal.
    pub value: Option<Length>,
    /// The value the solved geometry gives, when both references are related.
    pub computed: Option<Length>,
    /// `computed - value`, when both are known.
    pub residual: Option<Length>,
    /// The Distance whose value this one takes through an Equal (Q-14).
    pub copied_from: Option<Uuid>,
}

/// A redundant constraint that disagrees (7.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// The constraint later in processing order.
    pub constraint: Uuid,
    /// The constraints it disagrees with: every one on the path it closes, sorted.
    pub with: Vec<Uuid>,
    /// How far it is from agreeing, when it has a value.
    pub residual: Option<Length>,
}

/// Something whose coordinate lies in an underconstrained class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Member {
    /// A node's coordinate on the class's axis.
    Node(Uuid),
    /// A wall's reference line (walls perpendicular to the class's axis).
    Wall(Uuid),
}

/// One underconstrained class: its members move together by one translation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreeClass {
    pub axis: Axis,
    pub members: Vec<Member>,
}

/// The result of a solve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Solution {
    /// Every node's position: exact where determined, provisional otherwise.
    pub nodes: BTreeMap<Uuid, Point>,
    pub constraints: BTreeMap<Uuid, ConstraintReport>,
    /// Sorted by constraint id.
    pub conflicts: Vec<Conflict>,
    /// The degrees-of-freedom report, sorted by axis (X first) then members.
    pub underconstrained: Vec<FreeClass>,
}

impl Solution {
    /// Writes the solved node positions, Equal-copied values and demotions
    /// back into `document`.
    pub fn apply_to(&self, document: &mut Document) {
        for (id, p) in &self.nodes {
            if let Some(n) = document.nodes.get_mut(id) {
                n.x = p.x;
                n.y = p.y;
            }
        }
        for (id, report) in &self.constraints {
            let Some(c) = document.constraints.get_mut(id) else {
                continue;
            };
            if report.outcome == Outcome::Demoted {
                c.status = ConstraintStatus::Reference;
            }
            if let (Some(copied), Some(_), ConstraintKind::Distance { value, .. }) =
                (report.value, report.copied_from, &mut c.kind)
            {
                *value = copied;
            }
        }
    }
}

/// Why the document cannot be solved at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SolveError {
    /// An entity named by a wall or constraint does not exist.
    UnknownEntity { id: Uuid },
    /// A reference kind T-106 does not solve yet (Q-16).
    UnsupportedReference { constraint: Uuid },
    /// A reference has no coordinate on the constraint's axis.
    WrongAxis { constraint: Uuid },
    /// A WallFace boundary index beyond the wall's layers.
    NoSuchBoundary { constraint: Uuid },
    /// An Equal names something that is not a Distance constraint.
    NotADistance { constraint: Uuid, target: Uuid },
    /// A solved coordinate is outside the Length range.
    Overflow,
}

impl fmt::Display for SolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SolveError::UnknownEntity { id } => write!(f, "no entity {id}"),
            SolveError::UnsupportedReference { constraint } => {
                write!(
                    f,
                    "constraint {constraint} uses a reference Layer A does not solve yet"
                )
            }
            SolveError::WrongAxis { constraint } => {
                write!(
                    f,
                    "constraint {constraint} names a reference with no coordinate on its axis"
                )
            }
            SolveError::NoSuchBoundary { constraint } => {
                write!(
                    f,
                    "constraint {constraint} names a wall layer boundary that does not exist"
                )
            }
            SolveError::NotADistance { constraint, target } => {
                write!(
                    f,
                    "Equal {constraint} names {target}, which is not a Distance constraint"
                )
            }
            SolveError::Overflow => f.write_str("a solved coordinate is out of range"),
        }
    }
}

impl std::error::Error for SolveError {}

impl Solver for LayerA {
    fn solve(&self, document: &Document) -> Result<Solution, SolveError> {
        Engine::new(document, self.default_tolerance)?.run()
    }
}

/// What a union-find edge came from, for naming conflicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Label {
    Wall,
    Constraint(Uuid),
}

/// A weighted union-find over one axis. Variable 0 is ground (value 0).
struct Axis1 {
    parent: Vec<usize>,
    /// value(i) - value(parent(i)).
    offset: Vec<i128>,
    rank: Vec<u8>,
    /// Accepted edges, in acceptance order, for path search.
    edges: Vec<Vec<(usize, Label)>>,
}

impl Axis1 {
    fn new(n: usize) -> Self {
        Axis1 {
            parent: (0..n).collect(),
            offset: vec![0; n],
            rank: vec![0; n],
            edges: vec![Vec::new(); n],
        }
    }

    /// The root of `i` and value(i) - value(root).
    fn find(&mut self, i: usize) -> (usize, i128) {
        let mut path = Vec::new();
        let mut root = i;
        while self.parent[root] != root {
            path.push(root);
            root = self.parent[root];
        }
        // Compress from the top of the path down, so each offset is to root.
        for &node in path.iter().rev() {
            let p = self.parent[node];
            if p != root {
                self.offset[node] += self.offset[p];
            }
            self.parent[node] = root;
        }
        (root, if i == root { 0 } else { self.offset[i] })
    }

    /// value(b) - value(a), if `a` and `b` are in one class.
    fn relation(&mut self, a: usize, b: usize) -> Option<i128> {
        let (ra, oa) = self.find(a);
        let (rb, ob) = self.find(b);
        (ra == rb).then_some(ob - oa)
    }

    /// Records value(b) - value(a) = d; `a` and `b` must be in different classes.
    fn union(&mut self, a: usize, b: usize, d: i128, label: Label) {
        let (ra, oa) = self.find(a);
        let (rb, ob) = self.find(b);
        // value(rb) - value(ra) = d + oa - ob
        let delta = d + oa - ob;
        if self.rank[ra] >= self.rank[rb] {
            self.parent[rb] = ra;
            self.offset[rb] = delta;
            if self.rank[ra] == self.rank[rb] {
                self.rank[ra] = self.rank[ra].saturating_add(1);
            }
        } else {
            self.parent[ra] = rb;
            self.offset[ra] = -delta;
        }
        self.edges[a].push((b, label));
        self.edges[b].push((a, label));
    }

    /// The constraints on the accepted path from `a` to `b`, sorted.
    fn path(&self, a: usize, b: usize) -> Vec<Uuid> {
        let mut prev: BTreeMap<usize, (usize, Label)> = BTreeMap::new();
        let mut queue = VecDeque::from([a]);
        let mut seen = BTreeSet::from([a]);
        while let Some(v) = queue.pop_front() {
            if v == b {
                break;
            }
            for &(w, label) in &self.edges[v] {
                if seen.insert(w) {
                    prev.insert(w, (v, label));
                    queue.push_back(w);
                }
            }
        }
        let mut out = BTreeSet::new();
        let mut v = b;
        while let Some(&(p, label)) = prev.get(&v) {
            if let Label::Constraint(id) = label {
                out.insert(id);
            }
            v = p;
        }
        out.into_iter().collect()
    }
}

fn axis_index(axis: Axis) -> usize {
    match axis {
        Axis::X => 0,
        Axis::Y => 1,
    }
}

/// The axis a wall's reference line lies on, and the sign of its left
/// normal along that axis (6.1).
fn wall_frame(direction: Direction) -> (Axis, i128) {
    match direction {
        Direction::PosX => (Axis::Y, 1),
        Direction::NegX => (Axis::Y, -1),
        Direction::PosY => (Axis::X, -1),
        Direction::NegY => (Axis::X, 1),
    }
}

/// Processing rank of a source: authority order of D-11 (Q-15).
fn authority(source: Source) -> u8 {
    match source {
        Source::UserStated => 0,
        Source::Imported => 1,
        Source::Measured => 2,
        Source::Derived => 3,
        Source::Proposed => 4,
    }
}

/// `n / 2` rounded half to even.
fn half(n: i128) -> i128 {
    let q = n / 2;
    let r = n % 2;
    if r != 0 && q % 2 != 0 { q + r } else { q }
}

/// A linear relation between two references: value(b) - value(a) = d.
struct Relation {
    axis: usize,
    a: usize,
    b: usize,
    /// Constant parts: coordinate(ref) = value(var) + constant.
    ca: i128,
    cb: i128,
}

struct Engine<'d> {
    doc: &'d Document,
    tolerance: i128,
    nodes: BTreeMap<Uuid, usize>,
    axes: [Axis1; 2],
}

impl<'d> Engine<'d> {
    fn new(doc: &'d Document, tolerance: Length) -> Result<Self, SolveError> {
        let nodes: BTreeMap<Uuid, usize> = doc
            .nodes
            .keys()
            .enumerate()
            .map(|(i, id)| (*id, i + 1))
            .collect();
        let n = nodes.len() + 1;
        let mut engine = Engine {
            doc,
            tolerance: i128::from(tolerance.nanometers()),
            nodes,
            axes: [Axis1::new(n), Axis1::new(n)],
        };
        for wall in doc.walls.values() {
            let a = engine.node(wall.node_a)?;
            let b = engine.node(wall.node_b)?;
            let (axis, _) = wall_frame(wall.direction);
            let ax = &mut engine.axes[axis_index(axis)];
            if ax.relation(a, b).is_none() {
                ax.union(a, b, 0, Label::Wall);
            }
        }
        Ok(engine)
    }

    fn node(&self, id: Uuid) -> Result<usize, SolveError> {
        self.nodes
            .get(&id)
            .copied()
            .ok_or(SolveError::UnknownEntity { id })
    }

    /// The variable and constant that give `r`'s coordinate on `axis`.
    fn resolve(
        &self,
        r: &Reference,
        axis: Axis,
        constraint: Uuid,
    ) -> Result<(usize, i128), SolveError> {
        let wrong = SolveError::WrongAxis { constraint };
        match *r {
            Reference::Node { node } => Ok((self.node(node)?, 0)),
            Reference::Datum { datum } => {
                let d = self
                    .doc
                    .datums
                    .get(&datum)
                    .ok_or(SolveError::UnknownEntity { id: datum })?;
                let value = match (d.geometry, axis) {
                    (DatumGeometry::Point { x, .. }, Axis::X)
                    | (DatumGeometry::XLine { x }, Axis::X) => x,
                    (DatumGeometry::Point { y, .. }, Axis::Y)
                    | (DatumGeometry::YLine { y }, Axis::Y) => y,
                    _ => return Err(wrong),
                };
                Ok((0, i128::from(value.nanometers())))
            }
            Reference::WallCenter { wall } | Reference::WallFace { wall, .. } => {
                let w = self
                    .doc
                    .walls
                    .get(&wall)
                    .ok_or(SolveError::UnknownEntity { id: wall })?;
                let (line_axis, sign) = wall_frame(w.direction);
                if line_axis != axis {
                    return Err(wrong);
                }
                let set = self
                    .doc
                    .layer_sets
                    .get(&w.layer_set)
                    .ok_or(SolveError::UnknownEntity { id: w.layer_set })?;
                let t: Vec<i128> = set
                    .layers
                    .iter()
                    .map(|l| i128::from(l.thickness.value.nanometers()))
                    .collect();
                let total: i128 = t.iter().sum();
                let left = i128::from(w.left_extent.value.nanometers());
                let toward_left = match *r {
                    Reference::WallFace { side, boundary, .. } => {
                        let k = usize::try_from(boundary).unwrap_or(usize::MAX);
                        if k > t.len() {
                            return Err(SolveError::NoSuchBoundary { constraint });
                        }
                        match side {
                            WallSide::Left => left - t[..k].iter().sum::<i128>(),
                            WallSide::Right => left - total + t[t.len() - k..].iter().sum::<i128>(),
                        }
                    }
                    _ => left - half(total),
                };
                Ok((self.node(w.node_a)?, sign * toward_left))
            }
            Reference::OpeningEdge { .. }
            | Reference::ItemEdge { .. }
            | Reference::ItemCenter { .. } => Err(SolveError::UnsupportedReference { constraint }),
        }
    }

    /// The relation a constraint states, if Layer A solves it.
    fn relation(&self, id: Uuid, c: &Constraint) -> Result<Option<Relation>, SolveError> {
        let rel = |a: &Reference, b: &Reference, axis: Axis| -> Result<Relation, SolveError> {
            let (va, ca) = self.resolve(a, axis, id)?;
            let (vb, cb) = self.resolve(b, axis, id)?;
            Ok(Relation {
                axis: axis_index(axis),
                a: va,
                b: vb,
                ca,
                cb,
            })
        };
        Ok(match &c.kind {
            ConstraintKind::Distance { a, b, axis, .. } | ConstraintKind::Align { a, b, axis } => {
                Some(rel(a, b, *axis)?)
            }
            ConstraintKind::Fixed {
                reference, axis, ..
            } => {
                let (v, cv) = self.resolve(reference, *axis, id)?;
                Some(Relation {
                    axis: axis_index(*axis),
                    a: 0,
                    b: v,
                    ca: 0,
                    cb: cv,
                })
            }
            ConstraintKind::Equal { .. } | ConstraintKind::MinClearance { .. } => None,
        })
    }

    fn run(mut self) -> Result<Solution, SolveError> {
        let doc = self.doc;
        let mut order: Vec<(&Uuid, &Constraint)> = doc.constraints.iter().collect();
        order.sort_by_key(|(id, c)| (authority(c.source), **id));

        // Equal (Q-14): which Distance copies which, in processing order.
        let mut copy_from: BTreeMap<Uuid, (Uuid, Uuid)> = BTreeMap::new();
        let mut reports: BTreeMap<Uuid, ConstraintReport> = BTreeMap::new();
        let mut conflicts = Vec::new();
        let report = |outcome| ConstraintReport {
            outcome,
            value: None,
            computed: None,
            residual: None,
            copied_from: None,
        };
        for (id, c) in &order {
            let ConstraintKind::Equal { first, second } = c.kind else {
                continue;
            };
            for target in [first, second] {
                let is_distance = matches!(
                    doc.constraints.get(&target).map(|t| &t.kind),
                    Some(ConstraintKind::Distance { .. })
                );
                if !is_distance {
                    return Err(SolveError::NotADistance {
                        constraint: **id,
                        target,
                    });
                }
            }
            if c.status != ConstraintStatus::Driving {
                reports.insert(**id, report(Outcome::Reference));
                continue;
            }
            // Already copying, or copying would close a cycle: Conflict.
            let mut with = BTreeSet::new();
            let clash = if let Some(&(_, equal)) = copy_from.get(&second) {
                with.insert(equal);
                true
            } else {
                let mut cur = first;
                let mut cycle = false;
                while let Some(&(src, equal)) = copy_from.get(&cur) {
                    with.insert(equal);
                    if src == second {
                        cycle = true;
                        break;
                    }
                    cur = src;
                }
                cycle || first == second
            };
            if clash {
                reports.insert(**id, report(Outcome::Conflict));
                conflicts.push(Conflict {
                    constraint: **id,
                    with: with.into_iter().collect(),
                    residual: None,
                });
            } else {
                copy_from.insert(second, (first, **id));
                reports.insert(**id, report(Outcome::Driving));
            }
        }
        let stated = |id: Uuid, c: &Constraint| -> (Option<i128>, Option<Uuid>) {
            let mut root = id;
            let mut copied = None;
            while let Some(&(src, _)) = copy_from.get(&root) {
                root = src;
                copied = Some(src);
            }
            let kind = doc.constraints.get(&root).map_or(&c.kind, |r| &r.kind);
            let v = match kind {
                ConstraintKind::Distance { value, .. } | ConstraintKind::Fixed { value, .. } => {
                    Some(i128::from(value.nanometers()))
                }
                ConstraintKind::Align { .. } => Some(0),
                _ => None,
            };
            (v, copied)
        };

        // Driving constraints first, then Reference-status ones (report only).
        for driving in [true, false] {
            for (id, c) in &order {
                if (c.status == ConstraintStatus::Driving) != driving {
                    continue;
                }
                let (value, copied_from) = stated(**id, c);
                let Some(rel) = self.relation(**id, c)? else {
                    if matches!(c.kind, ConstraintKind::MinClearance { .. }) {
                        reports.insert(**id, report(Outcome::NotSolved));
                    }
                    continue;
                };
                let v = value.unwrap_or(0);
                let ax = &mut self.axes[rel.axis];
                let computed = ax.relation(rel.a, rel.b).map(|d| d + rel.cb - rel.ca);
                let residual = computed.map(|x| x - v);
                let outcome = if !driving {
                    Outcome::Reference
                } else {
                    match residual {
                        None => {
                            ax.union(rel.a, rel.b, v - rel.cb + rel.ca, Label::Constraint(**id));
                            Outcome::Driving
                        }
                        Some(0) => Outcome::Redundant,
                        Some(r) => {
                            let tolerance = match c.kind {
                                ConstraintKind::Distance {
                                    tolerance: Some(t), ..
                                } => i128::from(t.nanometers()),
                                _ => self.tolerance,
                            };
                            if c.source == Source::Measured && r.abs() <= tolerance {
                                Outcome::Demoted
                            } else {
                                conflicts.push(Conflict {
                                    constraint: **id,
                                    with: ax.path(rel.a, rel.b),
                                    residual: Some(to_length(r)?),
                                });
                                Outcome::Conflict
                            }
                        }
                    }
                };
                let computed = match outcome {
                    Outcome::Driving => Some(v),
                    _ => computed,
                };
                let residual = match outcome {
                    Outcome::Driving => Some(0),
                    _ => residual,
                };
                reports.insert(
                    **id,
                    ConstraintReport {
                        outcome,
                        value: value.map(to_length).transpose()?,
                        computed: computed.map(to_length).transpose()?,
                        residual: residual.map(to_length).transpose()?,
                        copied_from,
                    },
                );
            }
        }
        conflicts.sort_by_key(|c| c.constraint);

        // Positions: exact in grounded classes; otherwise anchored at the
        // stored coordinate of the class's lowest-id node (7.4).
        let mut coords: [BTreeMap<Uuid, i128>; 2] = [BTreeMap::new(), BTreeMap::new()];
        let mut classes: [BTreeMap<usize, BTreeSet<Member>>; 2] =
            [BTreeMap::new(), BTreeMap::new()];
        for (k, axis) in [Axis::X, Axis::Y].into_iter().enumerate() {
            let ax = &mut self.axes[k];
            let (ground_root, ground_off) = ax.find(0);
            let mut anchors: BTreeMap<usize, i128> = BTreeMap::new();
            for (id, &var) in &self.nodes {
                let (root, off) = ax.find(var);
                let value = if root == ground_root {
                    off - ground_off
                } else {
                    let stored = doc.nodes.get(id).map_or(0, |n| match axis {
                        Axis::X => n.x.nanometers(),
                        Axis::Y => n.y.nanometers(),
                    });
                    let base = *anchors.entry(root).or_insert(i128::from(stored) - off);
                    classes[k]
                        .entry(root)
                        .or_default()
                        .insert(Member::Node(*id));
                    base + off
                };
                coords[k].insert(*id, value);
            }
            for (id, wall) in &doc.walls {
                let (line_axis, _) = wall_frame(wall.direction);
                if line_axis != axis {
                    continue;
                }
                let var = self.node(wall.node_a)?;
                let (root, _) = self.axes[k].find(var);
                if let Some(members) = classes[k].get_mut(&root) {
                    members.insert(Member::Wall(*id));
                }
            }
        }
        let mut nodes = BTreeMap::new();
        for id in doc.nodes.keys() {
            let x = coords[0].get(id).copied().unwrap_or(0);
            let y = coords[1].get(id).copied().unwrap_or(0);
            nodes.insert(
                *id,
                Point {
                    x: to_length(x)?,
                    y: to_length(y)?,
                },
            );
        }
        let mut underconstrained = Vec::new();
        for (k, axis) in [Axis::X, Axis::Y].into_iter().enumerate() {
            let mut list: Vec<Vec<Member>> = classes[k]
                .values()
                .map(|m| m.iter().copied().collect())
                .collect();
            list.sort();
            underconstrained.extend(list.into_iter().map(|members| FreeClass { axis, members }));
        }
        Ok(Solution {
            nodes,
            constraints: reports,
            conflicts,
            underconstrained,
        })
    }
}

fn to_length(v: i128) -> Result<Length, SolveError> {
    i64::try_from(v)
        .map(Length::from_nanometers)
        .map_err(|_| SolveError::Overflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::*;
    use proptest::prelude::*;

    const IN: i64 = 25_400_000;
    const FT: i64 = 12 * IN;

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
    fn con_id(n: u8) -> Uuid {
        uid(3, n)
    }
    fn datum_id(n: u8) -> Uuid {
        uid(4, n)
    }
    const LAYERS: Uuid = Uuid::from_bytes([5; 16]);

    fn sourced(nm: i64) -> Sourced<Length> {
        Sourced {
            value: len(nm),
            source: Source::UserStated,
        }
    }

    /// An empty document with one layer set: 1 in, 2 in, 1 in (4 in total).
    fn base() -> Document {
        let mut doc = Document::new(
            Project {
                id: uid(9, 0),
                name: "T".into(),
                default_unit: crate::units::ProjectUnit::Inches,
            },
            Building {
                id: uid(9, 1),
                name: "B".into(),
            },
        );
        let layer = |t| Layer {
            material: uid(6, 0),
            thickness: sourced(t),
            core: false,
        };
        doc.layer_sets.insert(
            LAYERS,
            LayerSet {
                name: "L".into(),
                layers: vec![layer(IN), layer(2 * IN), layer(IN)],
            },
        );
        doc
    }

    fn add_node(doc: &mut Document, n: u8, x: i64, y: i64) {
        doc.nodes.insert(
            node_id(n),
            Node {
                x: len(x),
                y: len(y),
                join: JoinType::Isolated,
            },
        );
    }

    fn add_wall(doc: &mut Document, n: u8, a: u8, b: u8, direction: Direction) {
        let mut labels = crate::identity::LabelAllocator::new();
        let label = labels.allocate("W").unwrap_or_else(|_| unreachable!());
        doc.walls.insert(
            wall_id(n),
            Wall {
                label,
                storey: uid(8, 0),
                node_a: node_id(a),
                node_b: node_id(b),
                direction,
                layer_set: LAYERS,
                left_extent: sourced(IN),
                base_offset: sourced(0),
                top: WallTop::Storey,
                join_priority: 0,
                function: Function::New,
            },
        );
    }

    fn add(doc: &mut Document, n: u8, kind: ConstraintKind, source: Source) {
        doc.constraints.insert(
            con_id(n),
            Constraint {
                kind,
                status: ConstraintStatus::Driving,
                source,
                provenance: None,
            },
        );
    }

    fn node(n: u8) -> Reference {
        Reference::Node { node: node_id(n) }
    }

    fn fixed(reference: Reference, axis: Axis, value: i64) -> ConstraintKind {
        ConstraintKind::Fixed {
            reference,
            axis,
            value: len(value),
        }
    }

    fn dist(a: Reference, b: Reference, axis: Axis, value: i64) -> ConstraintKind {
        ConstraintKind::Distance {
            a,
            b,
            axis,
            value: len(value),
            tolerance: None,
        }
    }

    fn solve(doc: &Document) -> Result<Solution, SolveError> {
        LayerA::default().solve(doc)
    }

    fn pos(s: &Solution, n: u8) -> Option<(i64, i64)> {
        s.nodes
            .get(&node_id(n))
            .map(|p| (p.x.nanometers(), p.y.nanometers()))
    }

    /// An L of two walls: n1 -> n2 along +X, n2 -> n3 along +Y.
    fn l_shape() -> Document {
        let mut doc = base();
        add_node(&mut doc, 1, 0, 0);
        add_node(&mut doc, 2, 1, 1);
        add_node(&mut doc, 3, 2, 2);
        add_wall(&mut doc, 1, 1, 2, Direction::PosX);
        add_wall(&mut doc, 2, 2, 3, Direction::PosY);
        doc
    }

    #[test]
    fn l_shape_is_fully_determined() -> Result<(), SolveError> {
        let mut doc = l_shape();
        add(&mut doc, 1, fixed(node(1), Axis::X, 0), Source::UserStated);
        add(&mut doc, 2, fixed(node(1), Axis::Y, 0), Source::UserStated);
        add(
            &mut doc,
            3,
            dist(node(1), node(2), Axis::X, 10 * FT),
            Source::UserStated,
        );
        add(
            &mut doc,
            4,
            dist(node(2), node(3), Axis::Y, 8 * FT),
            Source::UserStated,
        );
        let s = solve(&doc)?;
        assert_eq!(pos(&s, 1), Some((0, 0)));
        assert_eq!(pos(&s, 2), Some((10 * FT, 0)));
        assert_eq!(pos(&s, 3), Some((10 * FT, 8 * FT)));
        assert!(s.underconstrained.is_empty(), "{:?}", s.underconstrained);
        assert!(s.conflicts.is_empty());
        assert!(
            s.constraints
                .values()
                .all(|r| r.outcome == Outcome::Driving)
        );
        Ok(())
    }

    #[test]
    fn distance_is_signed_and_align_is_zero() -> Result<(), SolveError> {
        let mut doc = l_shape();
        add(&mut doc, 1, fixed(node(1), Axis::X, 0), Source::UserStated);
        add(&mut doc, 2, fixed(node(1), Axis::Y, 0), Source::UserStated);
        // n2 is 6 ft to the WEST of n1: b - a = -6 ft.
        add(
            &mut doc,
            3,
            dist(node(1), node(2), Axis::X, -6 * FT),
            Source::UserStated,
        );
        add(
            &mut doc,
            4,
            ConstraintKind::Align {
                a: node(1),
                b: node(3),
                axis: Axis::Y,
            },
            Source::UserStated,
        );
        let s = solve(&doc)?;
        assert_eq!(pos(&s, 2), Some((-6 * FT, 0)));
        assert_eq!(pos(&s, 3), Some((-6 * FT, 0)));
        Ok(())
    }

    #[test]
    fn datums_anchor_axes() -> Result<(), SolveError> {
        let mut doc = l_shape();
        doc.datums.insert(
            datum_id(1),
            Datum {
                name: "P".into(),
                geometry: DatumGeometry::Point {
                    x: len(5),
                    y: len(7),
                },
            },
        );
        doc.datums.insert(
            datum_id(2),
            Datum {
                name: "X".into(),
                geometry: DatumGeometry::XLine { x: len(100) },
            },
        );
        add(
            &mut doc,
            1,
            dist(Reference::Datum { datum: datum_id(1) }, node(1), Axis::Y, 3),
            Source::UserStated,
        );
        add(
            &mut doc,
            2,
            dist(Reference::Datum { datum: datum_id(2) }, node(1), Axis::X, 0),
            Source::UserStated,
        );
        add(
            &mut doc,
            3,
            dist(
                Reference::Datum { datum: datum_id(1) },
                node(3),
                Axis::X,
                50,
            ),
            Source::UserStated,
        );
        let s = solve(&doc)?;
        assert_eq!(pos(&s, 1), Some((100, 10)));
        // n3 shares X with n2 (wall 2 runs along Y); its Y is free and keeps its stored 2.
        assert_eq!(pos(&s, 3), Some((55, 2)));
        assert_eq!(pos(&s, 2), Some((55, 10)));
        // XLine has no Y coordinate.
        add(
            &mut doc,
            4,
            dist(Reference::Datum { datum: datum_id(2) }, node(1), Axis::Y, 0),
            Source::UserStated,
        );
        assert_eq!(
            solve(&doc),
            Err(SolveError::WrongAxis {
                constraint: con_id(4)
            })
        );
        Ok(())
    }

    /// Offset of each wall reference from the reference line, in inches, for a
    /// wall with left_extent 1 in and layers 1, 2, 1 in, per direction.
    /// Left of +X is +Y, of -X is -Y, of +Y is -X, of -Y is +X (6.1).
    #[test]
    fn wall_face_and_center_offsets() -> Result<(), SolveError> {
        // (side, boundary, offset toward the left normal, in inches)
        let faces: &[(Option<(WallSide, u32)>, i64)] = &[
            (Some((WallSide::Left, 0)), 1),
            (Some((WallSide::Left, 1)), 0),
            (Some((WallSide::Left, 2)), -2),
            (Some((WallSide::Left, 3)), -3),
            (Some((WallSide::Right, 0)), -3),
            (Some((WallSide::Right, 1)), -2),
            (Some((WallSide::Right, 3)), 1),
            (None, -1), // center: 1 - 4/2
        ];
        let dirs = [
            (Direction::PosX, Axis::Y, 1),
            (Direction::NegX, Axis::Y, -1),
            (Direction::PosY, Axis::X, -1),
            (Direction::NegY, Axis::X, 1),
        ];
        for (dir, axis, sign) in dirs {
            for &(face, offset) in faces {
                let mut doc = base();
                add_node(&mut doc, 1, 0, 0);
                add_node(&mut doc, 2, 0, 0);
                add_wall(&mut doc, 1, 1, 2, dir);
                let reference = match face {
                    Some((side, boundary)) => Reference::WallFace {
                        wall: wall_id(1),
                        side,
                        boundary,
                    },
                    None => Reference::WallCenter { wall: wall_id(1) },
                };
                // Put the reference at 0; the reference line lands at -offset.
                add(&mut doc, 1, fixed(reference, axis, 0), Source::UserStated);
                let s = solve(&doc)?;
                let line = match axis {
                    Axis::X => pos(&s, 2).map(|p| p.0),
                    Axis::Y => pos(&s, 2).map(|p| p.1),
                };
                assert_eq!(line, Some(-sign * offset * IN), "{dir:?} {face:?}");
            }
        }
        Ok(())
    }

    #[test]
    fn invalid_references_are_errors() {
        let mut doc = l_shape();
        add(
            &mut doc,
            1,
            fixed(Reference::WallCenter { wall: wall_id(1) }, Axis::X, 0),
            Source::UserStated,
        );
        assert_eq!(
            solve(&doc),
            Err(SolveError::WrongAxis {
                constraint: con_id(1)
            })
        );

        let mut doc = l_shape();
        let face = Reference::WallFace {
            wall: wall_id(1),
            side: WallSide::Left,
            boundary: 4,
        };
        add(&mut doc, 1, fixed(face, Axis::Y, 0), Source::UserStated);
        assert_eq!(
            solve(&doc),
            Err(SolveError::NoSuchBoundary {
                constraint: con_id(1)
            })
        );

        for r in [
            Reference::ItemCenter { item: uid(7, 0) },
            Reference::ItemEdge {
                item: uid(7, 0),
                side: ItemSide::Front,
            },
            Reference::OpeningEdge {
                opening: uid(7, 0),
                edge: OpeningEdge::Start,
            },
        ] {
            let mut doc = l_shape();
            add(&mut doc, 1, fixed(r, Axis::X, 0), Source::UserStated);
            assert_eq!(
                solve(&doc),
                Err(SolveError::UnsupportedReference {
                    constraint: con_id(1)
                })
            );
        }

        let mut doc = l_shape();
        add(&mut doc, 1, fixed(node(9), Axis::X, 0), Source::UserStated);
        assert_eq!(
            solve(&doc),
            Err(SolveError::UnknownEntity { id: node_id(9) })
        );
    }

    #[test]
    fn conflicting_constraints_name_both() -> Result<(), SolveError> {
        let mut doc = l_shape();
        add(
            &mut doc,
            1,
            dist(node(1), node(2), Axis::X, 10 * FT),
            Source::UserStated,
        );
        add(
            &mut doc,
            2,
            dist(node(1), node(2), Axis::X, 12 * FT),
            Source::UserStated,
        );
        let s = solve(&doc)?;
        assert_eq!(
            s.conflicts,
            vec![Conflict {
                constraint: con_id(2),
                with: vec![con_id(1)],
                residual: Some(len(-2 * FT))
            }]
        );
        assert_eq!(
            s.constraints.get(&con_id(1)).map(|r| r.outcome),
            Some(Outcome::Driving)
        );
        assert_eq!(
            s.constraints.get(&con_id(2)).map(|r| r.outcome),
            Some(Outcome::Conflict)
        );
        // The conflicting constraint is skipped; the first one holds.
        assert_eq!(
            pos(&s, 2).map(|p| p.0 - pos(&s, 1).map_or(0, |q| q.0)),
            Some(10 * FT)
        );
        Ok(())
    }

    #[test]
    fn conflict_names_every_constraint_on_the_cycle() -> Result<(), SolveError> {
        let mut doc = l_shape();
        add(&mut doc, 1, fixed(node(1), Axis::X, 0), Source::UserStated);
        add(
            &mut doc,
            2,
            fixed(node(2), Axis::X, 10 * FT),
            Source::UserStated,
        );
        add(
            &mut doc,
            3,
            dist(node(1), node(2), Axis::X, 5 * FT),
            Source::UserStated,
        );
        let s = solve(&doc)?;
        assert_eq!(
            s.conflicts,
            vec![Conflict {
                constraint: con_id(3),
                with: vec![con_id(1), con_id(2)],
                residual: Some(len(5 * FT))
            }]
        );
        Ok(())
    }

    #[test]
    fn measured_within_tolerance_becomes_reference() -> Result<(), SolveError> {
        let mut doc = l_shape();
        add(
            &mut doc,
            1,
            dist(node(1), node(2), Axis::X, 10 * FT),
            Source::UserStated,
        );
        add(
            &mut doc,
            2,
            dist(node(1), node(2), Axis::X, 10 * FT + IN / 8),
            Source::Measured,
        );
        let s = solve(&doc)?;
        let r = s.constraints.get(&con_id(2)).copied();
        assert_eq!(r.map(|r| r.outcome), Some(Outcome::Demoted));
        assert_eq!(r.and_then(|r| r.residual), Some(len(-IN / 8)));
        assert!(s.conflicts.is_empty());

        // Beyond the default 1/4 in: a Conflict.
        let mut doc = l_shape();
        add(
            &mut doc,
            1,
            dist(node(1), node(2), Axis::X, 10 * FT),
            Source::UserStated,
        );
        add(
            &mut doc,
            2,
            dist(node(1), node(2), Axis::X, 10 * FT + IN / 2),
            Source::Measured,
        );
        assert_eq!(solve(&doc)?.conflicts.len(), 1);

        // A constraint's own tolerance overrides the default.
        let mut doc = l_shape();
        add(
            &mut doc,
            1,
            dist(node(1), node(2), Axis::X, 10 * FT),
            Source::UserStated,
        );
        let kind = ConstraintKind::Distance {
            a: node(1),
            b: node(2),
            axis: Axis::X,
            value: len(10 * FT + IN / 2),
            tolerance: Some(len(IN)),
        };
        add(&mut doc, 2, kind, Source::Measured);
        assert_eq!(
            solve(&doc)?.constraints.get(&con_id(2)).map(|r| r.outcome),
            Some(Outcome::Demoted)
        );

        // A user-stated disagreement is a Conflict however small.
        let mut doc = l_shape();
        add(
            &mut doc,
            1,
            dist(node(1), node(2), Axis::X, 10 * FT),
            Source::UserStated,
        );
        add(
            &mut doc,
            2,
            dist(node(1), node(2), Axis::X, 10 * FT + 1),
            Source::UserStated,
        );
        assert_eq!(solve(&doc)?.conflicts.len(), 1);
        Ok(())
    }

    #[test]
    fn authority_decides_which_is_redundant() -> Result<(), SolveError> {
        // The Measured constraint has the smaller id but lower authority.
        let mut doc = l_shape();
        add(
            &mut doc,
            1,
            dist(node(1), node(2), Axis::X, 10 * FT + IN / 8),
            Source::Measured,
        );
        add(
            &mut doc,
            2,
            dist(node(1), node(2), Axis::X, 10 * FT),
            Source::UserStated,
        );
        let s = solve(&doc)?;
        assert_eq!(
            s.constraints.get(&con_id(2)).map(|r| r.outcome),
            Some(Outcome::Driving)
        );
        assert_eq!(
            s.constraints.get(&con_id(1)).map(|r| r.outcome),
            Some(Outcome::Demoted)
        );
        Ok(())
    }

    #[test]
    fn consistent_redundancy_is_redundant() -> Result<(), SolveError> {
        let mut doc = l_shape();
        add(
            &mut doc,
            1,
            dist(node(1), node(2), Axis::X, 10 * FT),
            Source::UserStated,
        );
        add(
            &mut doc,
            2,
            dist(node(2), node(1), Axis::X, -10 * FT),
            Source::UserStated,
        );
        let s = solve(&doc)?;
        assert_eq!(
            s.constraints.get(&con_id(2)).map(|r| r.outcome),
            Some(Outcome::Redundant)
        );
        assert_eq!(
            s.constraints.get(&con_id(2)).and_then(|r| r.residual),
            Some(len(0))
        );
        Ok(())
    }

    fn equal(first: u8, second: u8) -> ConstraintKind {
        ConstraintKind::Equal {
            first: con_id(first),
            second: con_id(second),
        }
    }

    #[test]
    fn equal_copies_the_first_value() -> Result<(), SolveError> {
        let mut doc = l_shape();
        add(&mut doc, 1, fixed(node(1), Axis::X, 0), Source::UserStated);
        add(
            &mut doc,
            2,
            dist(node(1), node(2), Axis::X, 10 * FT),
            Source::UserStated,
        );
        add(
            &mut doc,
            3,
            dist(node(2), node(3), Axis::Y, 3 * FT),
            Source::UserStated,
        );
        add(&mut doc, 4, equal(2, 3), Source::UserStated);
        let s = solve(&doc)?;
        let r3 = s.constraints.get(&con_id(3)).copied();
        assert_eq!(r3.and_then(|r| r.value), Some(len(10 * FT)));
        assert_eq!(r3.and_then(|r| r.copied_from), Some(con_id(2)));
        assert_eq!(
            pos(&s, 3).map(|p| p.1 - pos(&s, 2).map_or(0, |q| q.1)),
            Some(10 * FT)
        );
        Ok(())
    }

    #[test]
    fn equal_chains_and_cycles() -> Result<(), SolveError> {
        let mut doc = l_shape();
        add(
            &mut doc,
            1,
            dist(node(1), node(2), Axis::X, 7),
            Source::UserStated,
        );
        add(
            &mut doc,
            2,
            dist(node(2), node(3), Axis::Y, 1),
            Source::UserStated,
        );
        add(
            &mut doc,
            3,
            dist(node(1), node(3), Axis::Y, 2),
            Source::Proposed,
        );
        add(&mut doc, 4, equal(1, 2), Source::UserStated);
        add(&mut doc, 5, equal(2, 3), Source::UserStated);
        let s = solve(&doc)?;
        assert_eq!(
            s.constraints.get(&con_id(3)).and_then(|r| r.value),
            Some(len(7))
        );
        assert_eq!(
            s.constraints.get(&con_id(3)).and_then(|r| r.copied_from),
            Some(con_id(1))
        );

        // Closing the loop 3 -> 1 is a cycle: Conflict naming the other Equals.
        add(&mut doc, 6, equal(3, 1), Source::UserStated);
        let s = solve(&doc)?;
        assert_eq!(
            s.conflicts,
            vec![Conflict {
                constraint: con_id(6),
                with: vec![con_id(4), con_id(5)],
                residual: None
            }]
        );

        // Equal must name Distances.
        let mut doc = l_shape();
        add(&mut doc, 1, fixed(node(1), Axis::X, 0), Source::UserStated);
        add(
            &mut doc,
            2,
            dist(node(1), node(2), Axis::X, 7),
            Source::UserStated,
        );
        add(&mut doc, 3, equal(1, 2), Source::UserStated);
        assert_eq!(
            solve(&doc),
            Err(SolveError::NotADistance {
                constraint: con_id(3),
                target: con_id(1)
            })
        );
        Ok(())
    }

    #[test]
    fn reference_status_reports_without_constraining() -> Result<(), SolveError> {
        let mut doc = l_shape();
        add(
            &mut doc,
            1,
            dist(node(1), node(2), Axis::X, 10 * FT),
            Source::UserStated,
        );
        add(
            &mut doc,
            2,
            dist(node(1), node(2), Axis::X, 9 * FT),
            Source::Measured,
        );
        add(
            &mut doc,
            3,
            dist(node(1), node(3), Axis::Y, 4 * FT),
            Source::Measured,
        );
        for n in [2, 3] {
            if let Some(c) = doc.constraints.get_mut(&con_id(n)) {
                c.status = ConstraintStatus::Reference;
            }
        }
        let s = solve(&doc)?;
        let r2 = s.constraints.get(&con_id(2)).copied();
        assert_eq!(
            r2.map(|r| (r.outcome, r.computed, r.residual)),
            Some((Outcome::Reference, Some(len(10 * FT)), Some(len(FT))))
        );
        // n1 and n3 are not related on Y: nothing to compute, and no effect.
        let r3 = s.constraints.get(&con_id(3)).copied();
        assert_eq!(
            r3.map(|r| (r.outcome, r.computed)),
            Some((Outcome::Reference, None))
        );
        assert!(s.conflicts.is_empty());
        // X: {n1, n2, n3}; Y: {n1, n2} and {n3}.
        assert_eq!(s.underconstrained.len(), 3, "{:?}", s.underconstrained);
        Ok(())
    }

    /// A rectangle of four walls with nothing anchored: per axis, one class per
    /// pair of parallel walls' reference lines, plus every node's free coordinate.
    #[test]
    fn underconstrained_classes_list_exactly_their_members() -> Result<(), SolveError> {
        let mut doc = base();
        add_node(&mut doc, 1, 0, 0);
        add_node(&mut doc, 2, 10, 0);
        add_node(&mut doc, 3, 10, 8);
        add_node(&mut doc, 4, 0, 8);
        add_wall(&mut doc, 1, 1, 2, Direction::PosX); // south, line y
        add_wall(&mut doc, 2, 2, 3, Direction::PosY); // east, line x
        add_wall(&mut doc, 3, 3, 4, Direction::NegX); // north, line y
        add_wall(&mut doc, 4, 4, 1, Direction::NegY); // west, line x
        // Anchor the west wall's line on X, and tie south to north on Y.
        add(&mut doc, 1, fixed(node(1), Axis::X, 0), Source::UserStated);
        add(
            &mut doc,
            2,
            dist(node(1), node(4), Axis::Y, 8),
            Source::UserStated,
        );
        let s = solve(&doc)?;
        use Member::{Node as N, Wall as W};
        assert_eq!(
            s.underconstrained,
            vec![
                FreeClass {
                    axis: Axis::X,
                    members: vec![N(node_id(2)), N(node_id(3)), W(wall_id(2))]
                },
                FreeClass {
                    axis: Axis::Y,
                    members: vec![
                        N(node_id(1)),
                        N(node_id(2)),
                        N(node_id(3)),
                        N(node_id(4)),
                        W(wall_id(1)),
                        W(wall_id(3))
                    ]
                },
            ]
        );
        // Provisional positions: free classes keep their lowest-id node's stored coordinate.
        assert_eq!(pos(&s, 1), Some((0, 0)));
        assert_eq!(pos(&s, 2), Some((10, 0)));
        assert_eq!(pos(&s, 3), Some((10, 8)));
        assert_eq!(pos(&s, 4), Some((0, 8)));
        Ok(())
    }

    #[test]
    fn apply_to_writes_positions_demotions_and_copies() -> Result<(), SolveError> {
        let mut doc = l_shape();
        add(&mut doc, 1, fixed(node(1), Axis::X, 0), Source::UserStated);
        add(
            &mut doc,
            2,
            dist(node(1), node(2), Axis::X, 10 * FT),
            Source::UserStated,
        );
        add(
            &mut doc,
            3,
            dist(node(1), node(2), Axis::X, 10 * FT + 1),
            Source::Measured,
        );
        add(
            &mut doc,
            4,
            dist(node(2), node(3), Axis::Y, 5),
            Source::UserStated,
        );
        add(&mut doc, 5, equal(2, 4), Source::UserStated);
        let s = solve(&doc)?;
        s.apply_to(&mut doc);
        assert_eq!(doc.nodes.get(&node_id(2)).map(|n| n.x), Some(len(10 * FT)));
        assert_eq!(
            doc.constraints.get(&con_id(3)).map(|c| c.status),
            Some(ConstraintStatus::Reference)
        );
        let copied = doc.constraints.get(&con_id(4)).map(|c| match &c.kind {
            ConstraintKind::Distance { value, .. } => *value,
            _ => len(0),
        });
        assert_eq!(copied, Some(len(10 * FT)));
        // Solving the updated document again changes nothing.
        let again = solve(&doc)?;
        assert_eq!(again.nodes, s.nodes);
        Ok(())
    }

    /// A row of `n` nodes joined by +X walls, with constraints derived from
    /// true x positions, so every constraint set is consistent.
    fn row(n: u8, xs: &[i64], extra: &[(usize, usize)]) -> (Document, Vec<(Uuid, Constraint)>) {
        let mut doc = base();
        for i in 0..n {
            add_node(&mut doc, i, 0, 0);
        }
        for i in 1..n {
            add_wall(&mut doc, i, i - 1, i, Direction::PosX);
        }
        let mut cs = Vec::new();
        let mk = |kind, source| Constraint {
            kind,
            status: ConstraintStatus::Driving,
            source,
            provenance: None,
        };
        cs.push(mk(fixed(node(0), Axis::X, xs[0]), Source::UserStated));
        cs.push(mk(fixed(node(0), Axis::Y, 0), Source::UserStated));
        for i in 1..usize::from(n) {
            let src = if i % 3 == 0 {
                Source::Measured
            } else {
                Source::UserStated
            };
            cs.push(mk(
                dist(node(i as u8 - 1), node(i as u8), Axis::X, xs[i] - xs[i - 1]),
                src,
            ));
        }
        for &(a, b) in extra {
            cs.push(mk(
                dist(node(a as u8), node(b as u8), Axis::X, xs[b] - xs[a]),
                Source::Measured,
            ));
        }
        let with_ids = cs
            .into_iter()
            .enumerate()
            .map(|(i, c)| (con_id(i as u8), c))
            .collect();
        (doc, with_ids)
    }

    fn arb_row() -> impl Strategy<Value = (u8, Vec<i64>, Vec<(usize, usize)>)> {
        (2u8..12).prop_flat_map(|n| {
            let m = usize::from(n);
            (
                Just(n),
                proptest::collection::vec(-1_000_000i64..1_000_000, m),
                proptest::collection::vec((0..m, 0..m), 0..6),
            )
        })
    }

    proptest! {
        /// Shuffling the order constraints are added in changes nothing at all.
        #[test]
        fn shuffled_insertion_gives_identical_results(
            (n, xs, extra) in arb_row(),
            seed in any::<u64>(),
            conflict in any::<bool>(),
        ) {
            let (mut doc, mut cs) = row(n, &xs, &extra);
            if conflict {
                let k = ConstraintKind::Distance { a: node(0), b: node(n - 1), axis: Axis::X, value: len(xs[usize::from(n) - 1] - xs[0] + 1), tolerance: None };
                cs.push((con_id(200), Constraint { kind: k, status: ConstraintStatus::Driving, source: Source::UserStated, provenance: None }));
            }
            let mut shuffled = cs.clone();
            // A deterministic Fisher-Yates driven by the seed.
            let mut state = seed;
            for i in (1..shuffled.len()).rev() {
                state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                let j = usize::try_from(state >> 33).unwrap_or(0) % (i + 1);
                shuffled.swap(i, j);
            }
            let mut doc2 = doc.clone();
            doc.constraints.extend(cs);
            for (id, c) in shuffled {
                doc2.constraints.insert(id, c);
            }
            prop_assert_eq!(solve(&doc), solve(&doc2));
        }

        /// For consistent sets, even renaming the constraints (which changes
        /// processing order among equals) gives the same geometry.
        #[test]
        fn relabeled_constraints_give_identical_geometry(
            (n, xs, extra) in arb_row(),
            rotate in 0usize..64,
        ) {
            let (base_doc, cs) = row(n, &xs, &extra);
            let mut a = base_doc.clone();
            a.constraints.extend(cs.iter().cloned());
            let mut b = base_doc;
            let ids: Vec<Uuid> = cs.iter().map(|(id, _)| *id).collect();
            let k = rotate % ids.len();
            for (i, (_, c)) in cs.into_iter().enumerate() {
                b.constraints.insert(ids[(i + k) % ids.len()], c);
            }
            let (sa, sb) = (solve(&a), solve(&b));
            prop_assert!(sa.is_ok());
            let (sa, sb) = (sa.map_err(|e| TestCaseError::fail(e.to_string()))?, sb.map_err(|e| TestCaseError::fail(e.to_string()))?);
            prop_assert!(sa.conflicts.is_empty() && sb.conflicts.is_empty());
            prop_assert_eq!(&sa.nodes, &sb.nodes);
            prop_assert_eq!(&sa.underconstrained, &sb.underconstrained);
            for (i, x) in xs.iter().enumerate() {
                prop_assert_eq!(sa.nodes.get(&node_id(i as u8)).map(|p| p.x.nanometers()), Some(*x));
            }
        }
    }
}
