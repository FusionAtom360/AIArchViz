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
//! Each opening has one variable on its host's along axis: its start edge,
//! placed at coordinate(reference) + direction * offset, less the anchor
//! edge's shift. Placements are processed before constraints, in opening id
//! order, and a placement that disagrees is a Conflict naming the opening
//! (Q-20).
//!
//! Each occurrence has a center variable (Q-31): on both axes for floor and
//! ceiling items, which stay put unless a constraint moves them; on its
//! host's along axis for a wall-mounted item, whose across coordinate is the
//! host's face, so it follows the wall. A Fixed occurrence is anchored at its
//! stored center before anything else, so whatever would move it is a
//! Conflict naming it.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use crate::document::{
    Axis, Constraint, ConstraintKind, ConstraintStatus, DatumGeometry, Direction, Document,
    OpeningEdge, Point, Reference, Source, WallSide,
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
    /// An opening's position along its host (Q-20).
    Opening(Uuid),
    /// An occurrence's center on the class's axis (Q-31).
    Occurrence(Uuid),
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
    /// Every opening's start-edge coordinate on its host's along axis (Q-20).
    pub openings: BTreeMap<Uuid, Length>,
    /// Every occurrence's footprint center (Q-31).
    pub occurrences: BTreeMap<Uuid, Point>,
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
        for (id, center) in &self.occurrences {
            if let Some(o) = document.occurrences.get_mut(id) {
                o.center = *center;
                match &mut o.placement {
                    crate::document::Placement::Floor { x, y, .. }
                    | crate::document::Placement::CeilingMounted { x, y } => {
                        x.value = center.x;
                        y.value = center.y;
                    }
                    crate::document::Placement::WallMounted { .. } => {}
                }
            }
        }
        for (id, start) in &self.openings {
            if let Some(o) = document.openings.get_mut(id) {
                o.start = *start;
            }
        }
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

/// What a solver variable stands for (used by move resolution, T-113).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Owner {
    Ground,
    Node(Uuid),
    Opening(Uuid),
    Occurrence(Uuid),
}

/// One relation between two variables on an axis: value(b) - value(a) is
/// fixed. An editable edge's dimension enters it with coefficient `coef`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EdgeKind {
    /// A wall tie, an Align, or a value copied through an Equal.
    Rigid,
    /// A Fixed occurrence held at its stored position.
    Anchor(Uuid),
    Editable {
        target: crate::command::DimensionTarget,
        value: i128,
        coef: i128,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GraphEdge {
    pub a: usize,
    pub b: usize,
    pub kind: EdgeKind,
}

/// The solver's relations on one axis, with every variable's solved value.
pub(crate) struct AxisGraph {
    pub owners: Vec<Owner>,
    pub values: Vec<i128>,
    pub edges: Vec<GraphEdge>,
}

/// The relations Layer A solves on `axis`, leaving out conflicting and
/// demoted constraints and non-driving ones.
pub(crate) fn axis_graph(doc: &Document, axis: Axis) -> Result<AxisGraph, SolveError> {
    use crate::command::DimensionTarget;
    let solution = LayerA::default().solve(doc)?;
    let engine = Engine::new(doc, Length::from_nanometers(0))?;
    let conflicted: BTreeSet<Uuid> = solution.conflicts.iter().map(|c| c.constraint).collect();
    let n = engine.nodes.len() + engine.openings.len() + engine.items.len() + 1;
    let mut owners = vec![Owner::Ground; n];
    let mut values = vec![0i128; n];
    let pick = |p: &Point| {
        i128::from(match axis {
            Axis::X => p.x.nanometers(),
            Axis::Y => p.y.nanometers(),
        })
    };
    for (id, &v) in &engine.nodes {
        owners[v] = Owner::Node(*id);
        values[v] = solution.nodes.get(id).map_or(0, pick);
    }
    for (id, &v) in &engine.openings {
        owners[v] = Owner::Opening(*id);
        values[v] = solution
            .openings
            .get(id)
            .map_or(0, |l| i128::from(l.nanometers()));
    }
    for (id, &v) in &engine.items {
        owners[v] = Owner::Occurrence(*id);
        values[v] = solution.occurrences.get(id).map_or(0, pick);
    }
    let mut edges = Vec::new();
    for w in doc.walls.values() {
        if wall_frame(w.direction).0 == axis {
            edges.push(GraphEdge {
                a: engine.node(w.node_a)?,
                b: engine.node(w.node_b)?,
                kind: EdgeKind::Rigid,
            });
        }
    }
    for (id, o) in &doc.occurrences {
        if o.fixed {
            let (v, _) = engine.item_center(*id, axis)?;
            edges.push(GraphEdge {
                a: 0,
                b: v,
                kind: EdgeKind::Anchor(*id),
            });
        }
    }
    for (id, o) in &doc.openings {
        let (var, along, dir, _) = engine.opening(*id)?;
        if along != axis || conflicted.contains(id) {
            continue;
        }
        let (rv, _) = engine.resolve(&o.reference, along, *id)?;
        edges.push(GraphEdge {
            a: rv,
            b: var,
            kind: EdgeKind::Editable {
                target: DimensionTarget::OpeningOffset { opening: *id },
                value: i128::from(o.offset.value.nanometers()),
                coef: dir,
            },
        });
    }
    for (id, o) in &doc.occurrences {
        let crate::document::Placement::WallMounted {
            wall,
            reference,
            offset,
            ..
        } = &o.placement
        else {
            continue;
        };
        let w = doc
            .walls
            .get(wall)
            .ok_or(SolveError::UnknownEntity { id: *wall })?;
        let (along, dir) = along_frame(w.direction);
        if along != axis || conflicted.contains(id) {
            continue;
        }
        let var = engine
            .items
            .get(id)
            .copied()
            .ok_or(SolveError::UnknownEntity { id: *id })?;
        let (rv, _) = engine.resolve(reference, along, *id)?;
        edges.push(GraphEdge {
            a: rv,
            b: var,
            kind: EdgeKind::Editable {
                target: DimensionTarget::OccurrenceOffset { occurrence: *id },
                value: i128::from(offset.value.nanometers()),
                coef: dir,
            },
        });
    }
    for (id, c) in &doc.constraints {
        if c.status != ConstraintStatus::Driving {
            continue;
        }
        let report = solution.constraints.get(id);
        if !matches!(
            report.map(|r| r.outcome),
            Some(Outcome::Driving | Outcome::Redundant)
        ) {
            continue;
        }
        let copied = report.is_some_and(|r| r.copied_from.is_some());
        match &c.kind {
            ConstraintKind::Distance {
                a,
                b,
                axis: ax,
                value,
                ..
            } if *ax == axis => {
                let (va, _) = engine.resolve(a, axis, *id)?;
                let (vb, _) = engine.resolve(b, axis, *id)?;
                let kind = if copied {
                    EdgeKind::Rigid
                } else {
                    EdgeKind::Editable {
                        target: DimensionTarget::Constraint { constraint: *id },
                        value: i128::from(value.nanometers()),
                        coef: 1,
                    }
                };
                edges.push(GraphEdge { a: va, b: vb, kind });
            }
            ConstraintKind::Align { a, b, axis: ax } if *ax == axis => {
                let (va, _) = engine.resolve(a, axis, *id)?;
                let (vb, _) = engine.resolve(b, axis, *id)?;
                edges.push(GraphEdge {
                    a: va,
                    b: vb,
                    kind: EdgeKind::Rigid,
                });
            }
            ConstraintKind::Fixed {
                reference,
                axis: ax,
                value,
            } if *ax == axis => {
                let (v, _) = engine.resolve(reference, axis, *id)?;
                edges.push(GraphEdge {
                    a: 0,
                    b: v,
                    kind: EdgeKind::Editable {
                        target: DimensionTarget::Constraint { constraint: *id },
                        value: i128::from(value.nanometers()),
                        coef: 1,
                    },
                });
            }
            _ => {}
        }
    }
    Ok(AxisGraph {
        owners,
        values,
        edges,
    })
}

/// The variable and constant that give `reference`'s coordinate on `axis`.
pub(crate) fn resolve_reference(
    doc: &Document,
    reference: &Reference,
    axis: Axis,
) -> Result<(usize, i128), SolveError> {
    Engine::new(doc, Length::from_nanometers(0))?.resolve(
        reference,
        axis,
        Uuid::from_bytes([0; 16]),
    )
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
    /// Each opening's variable: its start edge on the host's along axis.
    openings: BTreeMap<Uuid, usize>,
    /// Each occurrence's variable: its center (both axes for floor and
    /// ceiling items, the host's along axis for wall-mounted items).
    items: BTreeMap<Uuid, usize>,
    axes: [Axis1; 2],
}

fn from_geometry(e: crate::geometry::GeometryError) -> SolveError {
    match e {
        crate::geometry::GeometryError::UnknownEntity { id } => SolveError::UnknownEntity { id },
        _ => SolveError::Overflow,
    }
}

fn direction_axis(d: Direction) -> (Axis, i128) {
    along_frame(d)
}

/// The axis a wall runs along and the sign of its direction on it.
fn along_frame(direction: Direction) -> (Axis, i128) {
    match direction {
        Direction::PosX => (Axis::X, 1),
        Direction::NegX => (Axis::X, -1),
        Direction::PosY => (Axis::Y, 1),
        Direction::NegY => (Axis::Y, -1),
    }
}

/// How far an opening edge is from its start edge, along the host's axis.
fn edge_shift(edge: OpeningEdge, dir: i128, width: i128) -> i128 {
    match edge {
        OpeningEdge::Start => 0,
        OpeningEdge::End => dir * width,
        OpeningEdge::Center => dir * half(width),
    }
}

impl<'d> Engine<'d> {
    fn new(doc: &'d Document, tolerance: Length) -> Result<Self, SolveError> {
        let nodes: BTreeMap<Uuid, usize> = doc
            .nodes
            .keys()
            .enumerate()
            .map(|(i, id)| (*id, i + 1))
            .collect();
        let openings: BTreeMap<Uuid, usize> = doc
            .openings
            .keys()
            .enumerate()
            .map(|(i, id)| (*id, nodes.len() + 1 + i))
            .collect();
        let items: BTreeMap<Uuid, usize> = doc
            .occurrences
            .keys()
            .enumerate()
            .map(|(i, id)| (*id, nodes.len() + openings.len() + 1 + i))
            .collect();
        let n = nodes.len() + openings.len() + items.len() + 1;
        let mut engine = Engine {
            doc,
            tolerance: i128::from(tolerance.nanometers()),
            nodes,
            openings,
            items,
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

    /// The variable and constant giving an item's center on `axis`.
    fn item_center(&self, id: Uuid, axis: Axis) -> Result<(usize, i128), SolveError> {
        let o = self
            .doc
            .occurrences
            .get(&id)
            .ok_or(SolveError::UnknownEntity { id })?;
        let var = self
            .items
            .get(&id)
            .copied()
            .ok_or(SolveError::UnknownEntity { id })?;
        let crate::document::Placement::WallMounted { wall, side, .. } = &o.placement else {
            return Ok((var, 0));
        };
        let w = self
            .doc
            .walls
            .get(wall)
            .ok_or(SolveError::UnknownEntity { id: *wall })?;
        let (line_axis, sign) = wall_frame(w.direction);
        if axis != line_axis {
            return Ok((var, 0));
        }
        let set = self
            .doc
            .layer_sets
            .get(&w.layer_set)
            .ok_or(SolveError::UnknownEntity { id: w.layer_set })?;
        let total: i128 = set
            .layers
            .iter()
            .map(|l| i128::from(l.thickness.value.nanometers()))
            .sum();
        let left = i128::from(w.left_extent.value.nanometers());
        let (face, normal) = match side {
            WallSide::Left => (sign * left, sign),
            WallSide::Right => (-sign * (total - left), -sign),
        };
        let (sx, sy) = crate::items::world_size(self.doc, id).map_err(from_geometry)?;
        let depth = if axis == Axis::X { sx } else { sy };
        let min = if normal > 0 { face } else { face - depth };
        Ok((self.node(w.node_a)?, min + half(depth)))
    }

    /// An opening's variable, its host's along axis and direction, and its width.
    fn opening(&self, id: Uuid) -> Result<(usize, Axis, i128, i128), SolveError> {
        let o = self
            .doc
            .openings
            .get(&id)
            .ok_or(SolveError::UnknownEntity { id })?;
        let host = self
            .doc
            .walls
            .get(&o.host)
            .ok_or(SolveError::UnknownEntity { id: o.host })?;
        let var = self
            .openings
            .get(&id)
            .copied()
            .ok_or(SolveError::UnknownEntity { id })?;
        let (along, dir) = along_frame(host.direction);
        Ok((var, along, dir, i128::from(o.width.value.nanometers())))
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
            Reference::OpeningEdge { opening, edge } => {
                let (var, along, dir, width) = self.opening(opening)?;
                if along != axis {
                    return Err(wrong);
                }
                Ok((var, edge_shift(edge, dir, width)))
            }
            Reference::ItemCenter { item } => self.item_center(item, axis),
            Reference::ItemEdge { item, side } => {
                let (var, c) = self.item_center(item, axis)?;
                let q = crate::items::rotation(self.doc, item).map_err(from_geometry)?;
                let (edge_axis, sign) = direction_axis(crate::items::side_direction(q, side));
                if edge_axis != axis {
                    return Err(wrong);
                }
                let (sx, sy) = crate::items::world_size(self.doc, item).map_err(from_geometry)?;
                let size = if axis == Axis::X { sx } else { sy };
                Ok((
                    var,
                    if sign > 0 {
                        c - half(size) + size
                    } else {
                        c - half(size)
                    },
                ))
            }
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

        let mut conflicts = Vec::new();

        // Fixed occurrences are anchored at their stored center first, so
        // anything that would move one conflicts with it (6.3, Q-31).
        for (id, o) in &doc.occurrences {
            if !o.fixed {
                continue;
            }
            for (axis, stored) in [(Axis::X, o.center.x), (Axis::Y, o.center.y)] {
                let (v, c) = self.item_center(*id, axis)?;
                let d = i128::from(stored.nanometers()) - c;
                let ax = &mut self.axes[axis_index(axis)];
                match ax.relation(0, v) {
                    None => ax.union(0, v, d, Label::Constraint(*id)),
                    Some(current) if current == d => {}
                    Some(current) => conflicts.push(Conflict {
                        constraint: *id,
                        with: ax.path(0, v),
                        residual: Some(to_length(current - d)?),
                    }),
                }
            }
        }

        // Wall-mounted placements: center = reference + direction * offset (Q-30).
        for (id, o) in &doc.occurrences {
            let crate::document::Placement::WallMounted {
                wall,
                reference,
                offset,
                ..
            } = &o.placement
            else {
                continue;
            };
            let w = doc
                .walls
                .get(wall)
                .ok_or(SolveError::UnknownEntity { id: *wall })?;
            let (along, dir) = along_frame(w.direction);
            let var = self
                .items
                .get(id)
                .copied()
                .ok_or(SolveError::UnknownEntity { id: *id })?;
            let (rv, rc) = self.resolve(reference, along, *id)?;
            let d = rc + dir * i128::from(offset.value.nanometers());
            let ax = &mut self.axes[axis_index(along)];
            match ax.relation(rv, var) {
                None => ax.union(rv, var, d, Label::Constraint(*id)),
                Some(current) if current == d => {}
                Some(current) => conflicts.push(Conflict {
                    constraint: *id,
                    with: ax.path(rv, var),
                    residual: Some(to_length(current - d)?),
                }),
            }
        }

        // Opening placements (Q-20), in id order, before any constraint:
        // start = coordinate(reference) + direction * offset - anchor shift.
        for (id, o) in &doc.openings {
            let (var, along, dir, width) = self.opening(*id)?;
            let (rv, rc) = self.resolve(&o.reference, along, *id)?;
            let offset = i128::from(o.offset.value.nanometers());
            let d = rc + dir * offset - edge_shift(o.anchor, dir, width);
            let ax = &mut self.axes[axis_index(along)];
            match ax.relation(rv, var) {
                None => ax.union(rv, var, d, Label::Constraint(*id)),
                Some(current) if current == d => {}
                Some(current) => conflicts.push(Conflict {
                    constraint: *id,
                    with: ax.path(rv, var),
                    residual: Some(to_length(current - d)?),
                }),
            }
        }

        // Equal (Q-14): which Distance copies which, in processing order.
        let mut copy_from: BTreeMap<Uuid, (Uuid, Uuid)> = BTreeMap::new();
        let mut reports: BTreeMap<Uuid, ConstraintReport> = BTreeMap::new();
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
        let mut opening_starts: BTreeMap<Uuid, i128> = BTreeMap::new();
        let mut item_centers: [BTreeMap<Uuid, i128>; 2] = [BTreeMap::new(), BTreeMap::new()];
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
            for (id, &var) in &self.openings {
                let Some(o) = doc.openings.get(id) else {
                    continue;
                };
                let host = doc
                    .walls
                    .get(&o.host)
                    .ok_or(SolveError::UnknownEntity { id: o.host })?;
                if along_frame(host.direction).0 != axis {
                    continue;
                }
                let (root, off) = self.axes[k].find(var);
                let value = if root == ground_root {
                    off - ground_off
                } else {
                    let stored = i128::from(o.start.nanometers());
                    let base = *anchors.entry(root).or_insert(stored - off);
                    classes[k]
                        .entry(root)
                        .or_default()
                        .insert(Member::Opening(*id));
                    base + off
                };
                opening_starts.insert(*id, value);
            }
            for (id, &var) in &self.items {
                let Some(o) = doc.occurrences.get(id) else {
                    continue;
                };
                let stored = i128::from(match axis {
                    Axis::X => o.center.x.nanometers(),
                    Axis::Y => o.center.y.nanometers(),
                });
                if let crate::document::Placement::WallMounted { wall, .. } = &o.placement {
                    let w = doc
                        .walls
                        .get(wall)
                        .ok_or(SolveError::UnknownEntity { id: *wall })?;
                    if along_frame(w.direction).0 != axis {
                        // Across the wall: the host's line plus a constant.
                        let (node_var, c) = self.item_center(*id, axis)?;
                        let node_id = self
                            .nodes
                            .iter()
                            .find(|(_, v)| **v == node_var)
                            .map(|(n, _)| *n)
                            .ok_or(SolveError::UnknownEntity { id: *wall })?;
                        let line = coords[k].get(&node_id).copied().unwrap_or(0);
                        item_centers[k].insert(*id, line + c);
                        continue;
                    }
                }
                let ax = &mut self.axes[k];
                let (root, off) = ax.find(var);
                let value = if root == ground_root {
                    off - ground_off
                } else {
                    let base = *anchors.entry(root).or_insert(stored - off);
                    classes[k]
                        .entry(root)
                        .or_default()
                        .insert(Member::Occurrence(*id));
                    base + off
                };
                item_centers[k].insert(*id, value);
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
            occurrences: doc
                .occurrences
                .keys()
                .map(|id| {
                    let x = item_centers[0].get(id).copied().unwrap_or(0);
                    let y = item_centers[1].get(id).copied().unwrap_or(0);
                    Ok((
                        *id,
                        Point {
                            x: to_length(x)?,
                            y: to_length(y)?,
                        },
                    ))
                })
                .collect::<Result<_, SolveError>>()?,
            openings: opening_starts
                .into_iter()
                .map(|(id, v)| Ok((id, to_length(v)?)))
                .collect::<Result<_, SolveError>>()?,
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
        ] {
            let mut doc = l_shape();
            add(&mut doc, 1, fixed(r, Axis::X, 0), Source::UserStated);
            // Item references are solved since T-111; a missing item is unknown.
            assert_eq!(
                solve(&doc),
                Err(SolveError::UnknownEntity { id: uid(7, 0) })
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

    fn opening_id(n: u8) -> Uuid {
        uid(10, n)
    }

    fn add_opening(
        doc: &mut Document,
        n: u8,
        host: u8,
        reference: Reference,
        anchor: OpeningEdge,
        offset: i64,
        width: i64,
    ) {
        doc.openings.insert(
            opening_id(n),
            Opening {
                kind: OpeningKind::Door,
                host: wall_id(host),
                reference,
                anchor,
                offset: sourced(offset),
                width: sourced(width),
                height: sourced(80 * IN),
                sill: sourced(0),
                start: len(0),
            },
        );
    }

    fn start(s: &Solution, n: u8) -> Option<i64> {
        s.openings.get(&opening_id(n)).map(|l| l.nanometers())
    }

    /// A +X wall from (0,0) to (20 ft, 0) and a -X wall from (20 ft, 10 ft)
    /// back to (0, 10 ft), both fully anchored.
    fn two_walls() -> Document {
        let mut doc = base();
        add_node(&mut doc, 1, 0, 0);
        add_node(&mut doc, 2, 20 * FT, 0);
        add_node(&mut doc, 3, 20 * FT, 10 * FT);
        add_node(&mut doc, 4, 0, 10 * FT);
        add_wall(&mut doc, 1, 1, 2, Direction::PosX);
        add_wall(&mut doc, 2, 3, 4, Direction::NegX);
        let mut n = 0;
        for (node, x, y) in [
            (1, 0, 0),
            (2, 20 * FT, 0),
            (3, 20 * FT, 10 * FT),
            (4, 0, 10 * FT),
        ] {
            for (axis, v) in [(Axis::X, x), (Axis::Y, y)] {
                n += 1;
                add(
                    &mut doc,
                    n,
                    fixed(
                        Reference::Node {
                            node: node_id(node),
                        },
                        axis,
                        v,
                    ),
                    Source::UserStated,
                );
            }
        }
        doc
    }

    /// The anchor edge sits at reference + offset in the host's direction;
    /// start is the edge nearer node_a (Q-20).
    #[test]
    fn opening_is_placed_from_each_anchor() -> Result<(), SolveError> {
        let w = 36 * IN;
        for (anchor, on_pos_x, on_neg_x) in [
            // +X wall from x = 0: anchor at 48 in. -X wall from x = 20 ft: anchor at 20 ft - 48 in.
            (OpeningEdge::Start, 48 * IN, 20 * FT - 48 * IN),
            (OpeningEdge::End, 48 * IN - w, 20 * FT - 48 * IN + w),
            (
                OpeningEdge::Center,
                48 * IN - w / 2,
                20 * FT - 48 * IN + w / 2,
            ),
        ] {
            let mut doc = two_walls();
            add_opening(&mut doc, 1, 1, node(1), anchor, 48 * IN, w);
            add_opening(&mut doc, 2, 2, node(3), anchor, 48 * IN, w);
            let s = solve(&doc)?;
            assert_eq!(start(&s, 1), Some(on_pos_x), "{anchor:?} on +X");
            assert_eq!(start(&s, 2), Some(on_neg_x), "{anchor:?} on -X");
        }
        Ok(())
    }

    /// Constraints can name each opening edge as a reference.
    #[test]
    fn opening_edges_position_other_things() -> Result<(), SolveError> {
        // Opening: start 2 ft from n1, 3 ft wide. Each edge drives n1 through a Fixed.
        for (edge, edge_offset) in [
            (OpeningEdge::Start, 2 * FT),
            (OpeningEdge::End, 5 * FT),
            (OpeningEdge::Center, 2 * FT + 18 * IN),
        ] {
            let mut doc = l_shape();
            add_opening(&mut doc, 1, 1, node(1), OpeningEdge::Start, 2 * FT, 3 * FT);
            add(
                &mut doc,
                1,
                fixed(
                    Reference::OpeningEdge {
                        opening: opening_id(1),
                        edge,
                    },
                    Axis::X,
                    10 * FT,
                ),
                Source::UserStated,
            );
            let s = solve(&doc)?;
            assert_eq!(
                pos(&s, 1).map(|p| p.0),
                Some(10 * FT - edge_offset),
                "{edge:?}"
            );
            assert_eq!(
                start(&s, 1),
                Some(10 * FT - edge_offset + 2 * FT),
                "{edge:?}"
            );
        }
        // One opening placed from another opening's End edge.
        let mut doc = two_walls();
        add_opening(&mut doc, 1, 1, node(1), OpeningEdge::Start, 2 * FT, 3 * FT);
        add_opening(
            &mut doc,
            2,
            1,
            Reference::OpeningEdge {
                opening: opening_id(1),
                edge: OpeningEdge::End,
            },
            OpeningEdge::Start,
            FT,
            2 * FT,
        );
        assert_eq!(start(&solve(&doc)?, 2), Some(6 * FT));
        // Placed from the face of a perpendicular wall.
        let mut doc = l_shape();
        add(&mut doc, 1, fixed(node(1), Axis::X, 0), Source::UserStated);
        add(
            &mut doc,
            2,
            dist(node(1), node(2), Axis::X, 10 * FT),
            Source::UserStated,
        );
        // Wall 2 runs +Y at x = 10 ft; its left face (toward -X) is at 10 ft - 1 in.
        let face = Reference::WallFace {
            wall: wall_id(2),
            side: WallSide::Left,
            boundary: 0,
        };
        add_opening(&mut doc, 1, 1, face, OpeningEdge::End, -6 * IN, 3 * FT);
        assert_eq!(
            start(&solve(&doc)?, 1),
            Some(10 * FT - IN - 6 * IN - 3 * FT)
        );
        Ok(())
    }

    #[test]
    fn opening_edge_has_no_across_coordinate() {
        let mut doc = l_shape();
        add_opening(&mut doc, 1, 1, node(1), OpeningEdge::Start, 0, 3 * FT);
        add(
            &mut doc,
            1,
            fixed(
                Reference::OpeningEdge {
                    opening: opening_id(1),
                    edge: OpeningEdge::Start,
                },
                Axis::Y,
                0,
            ),
            Source::UserStated,
        );
        assert_eq!(
            solve(&doc),
            Err(SolveError::WrongAxis {
                constraint: con_id(1)
            })
        );
        let mut doc = l_shape();
        add(
            &mut doc,
            1,
            fixed(
                Reference::OpeningEdge {
                    opening: opening_id(9),
                    edge: OpeningEdge::Start,
                },
                Axis::X,
                0,
            ),
            Source::UserStated,
        );
        assert_eq!(
            solve(&doc),
            Err(SolveError::UnknownEntity { id: opening_id(9) })
        );
    }

    #[test]
    fn clashing_placements_are_conflicts_naming_the_openings() -> Result<(), SolveError> {
        let mut doc = l_shape();
        let edge = |o| Reference::OpeningEdge {
            opening: opening_id(o),
            edge: OpeningEdge::Start,
        };
        add_opening(&mut doc, 1, 1, edge(2), OpeningEdge::Start, IN, FT);
        add_opening(&mut doc, 2, 1, edge(1), OpeningEdge::Start, IN, FT);
        let s = solve(&doc)?;
        assert_eq!(
            s.conflicts,
            vec![Conflict {
                constraint: opening_id(2),
                with: vec![opening_id(1)],
                residual: Some(len(-2 * IN))
            }]
        );
        // A constraint that disagrees with a placement names the opening.
        let mut doc = l_shape();
        add_opening(&mut doc, 1, 1, node(1), OpeningEdge::Start, 2 * FT, FT);
        add(
            &mut doc,
            1,
            dist(
                node(1),
                Reference::OpeningEdge {
                    opening: opening_id(1),
                    edge: OpeningEdge::Start,
                },
                Axis::X,
                FT,
            ),
            Source::UserStated,
        );
        assert_eq!(
            solve(&doc)?.conflicts,
            vec![Conflict {
                constraint: con_id(1),
                with: vec![opening_id(1)],
                residual: Some(len(FT))
            }]
        );
        Ok(())
    }

    #[test]
    fn free_openings_are_reported_and_stored() -> Result<(), SolveError> {
        let mut doc = l_shape();
        add_opening(&mut doc, 1, 1, node(1), OpeningEdge::Start, 2 * FT, FT);
        let s = solve(&doc)?;
        let x_class = s
            .underconstrained
            .iter()
            .find(|c| c.axis == Axis::X)
            .map(|c| c.members.clone());
        assert_eq!(
            x_class,
            Some(vec![
                Member::Node(node_id(1)),
                Member::Opening(opening_id(1))
            ])
        );
        // n1 keeps its stored x = 0, so the opening starts at 2 ft.
        assert_eq!(start(&s, 1), Some(2 * FT));
        s.apply_to(&mut doc);
        assert_eq!(
            doc.openings.get(&opening_id(1)).map(|o| o.start),
            Some(len(2 * FT))
        );
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

#[cfg(test)]
mod occurrence_tests {
    use super::*;
    use crate::document::*;
    use crate::items::tests::{IN, base, floor, len, on_wall, put, uid};

    fn fixed(doc: &mut Document, n: u8, reference: Reference, axis: Axis, value: i64) {
        doc.constraints.insert(
            uid(3, n),
            Constraint {
                kind: ConstraintKind::Fixed {
                    reference,
                    axis,
                    value: len(value * IN),
                },
                status: ConstraintStatus::Driving,
                source: Source::UserStated,
                provenance: None,
            },
        );
    }

    fn node(n: u8) -> Reference {
        Reference::Node { node: uid(1, n) }
    }

    /// The +X wall of items::tests::base, anchored with its line at `y` in.
    fn anchored(y: i64) -> Document {
        let mut doc = base();
        fixed(&mut doc, 1, node(1), Axis::X, 0);
        fixed(&mut doc, 2, node(1), Axis::Y, y);
        fixed(&mut doc, 3, node(2), Axis::X, 240);
        doc
    }

    fn center(s: &Solution, id: Uuid) -> Option<(i64, i64)> {
        s.occurrences
            .get(&id)
            .map(|p| (p.x.nanometers() / IN, p.y.nanometers() / IN))
    }

    fn solve(doc: &Document) -> Result<Solution, SolveError> {
        LayerA::default().solve(doc)
    }

    /// Done when: wall-mounted items follow a moved host wall.
    #[test]
    fn wall_mounted_items_follow_a_moved_host_wall() -> Result<(), SolveError> {
        let mut doc = anchored(0);
        let item = put(&mut doc, 1, on_wall(WallSide::Left, 60, 48), (0, 0));
        let s = solve(&doc)?;
        // Back on the left face (y = 2), 24 in deep: center y = 14.
        assert_eq!(center(&s, item), Some((60, 14)));
        s.apply_to(&mut doc);
        assert_eq!(
            doc.occurrences.get(&item).map(|o| o.center),
            Some(Point {
                x: len(60 * IN),
                y: len(14 * IN)
            })
        );
        // Move the wall 36 in north: the item moves with it.
        fixed(&mut doc, 2, node(1), Axis::Y, 36);
        assert_eq!(center(&solve(&doc)?, item), Some((60, 50)));
        // Move the wall's start node east: the item keeps its offset from it.
        fixed(&mut doc, 1, node(1), Axis::X, 30);
        assert_eq!(center(&solve(&doc)?, item), Some((90, 50)));
        Ok(())
    }

    /// Done when: floor items stay (and are rechecked; see validity).
    #[test]
    fn floor_items_stay_when_a_wall_moves() -> Result<(), SolveError> {
        let mut doc = anchored(0);
        let item = put(&mut doc, 1, floor(100, 60, 0), (100, 60));
        assert_eq!(center(&solve(&doc)?, item), Some((100, 60)));
        fixed(&mut doc, 2, node(1), Axis::Y, 50);
        let s = solve(&doc)?;
        assert_eq!(center(&s, item), Some((100, 60)));
        // Free on both axes: listed in the degrees-of-freedom report.
        let members: Vec<_> = s
            .underconstrained
            .iter()
            .map(|c| c.members.clone())
            .collect();
        assert_eq!(
            members,
            vec![
                vec![Member::Occurrence(item)],
                vec![Member::Occurrence(item)]
            ]
        );
        s.apply_to(&mut doc);
        let xy = doc.occurrences.get(&item).map(|o| match &o.placement {
            Placement::Floor { x, y, .. } => (x.value.nanometers() / IN, y.value.nanometers() / IN),
            _ => (0, 0),
        });
        assert_eq!(xy, Some((100, 60)));
        Ok(())
    }

    /// Done when: a Fixed item refuses to move.
    #[test]
    fn a_fixed_item_refuses_to_move() -> Result<(), SolveError> {
        // A fixed wall-mounted item pins its host wall: moving the wall conflicts.
        let mut doc = anchored(0);
        let item = put(&mut doc, 1, on_wall(WallSide::Left, 60, 48), (60, 14));
        if let Some(o) = doc.occurrences.get_mut(&item) {
            o.fixed = true;
        }
        assert!(solve(&doc)?.conflicts.is_empty());
        fixed(&mut doc, 2, node(1), Axis::Y, 36);
        let s = solve(&doc)?;
        assert_eq!(
            s.conflicts,
            vec![Conflict {
                constraint: uid(3, 2),
                with: vec![item],
                residual: Some(len(-36 * IN))
            }]
        );
        assert_eq!(center(&s, item), Some((60, 14)));
        // A fixed floor item: a constraint that would push it is a Conflict naming it.
        let mut doc = anchored(0);
        let column = put(&mut doc, 2, floor(100, 60, 0), (100, 60));
        if let Some(o) = doc.occurrences.get_mut(&column) {
            o.fixed = true;
        }
        let face = Reference::WallFace {
            wall: uid(2, 1),
            side: WallSide::Left,
            boundary: 0,
        };
        let front = Reference::ItemEdge {
            item: column,
            side: ItemSide::Front,
        };
        doc.constraints.insert(
            uid(3, 9),
            Constraint {
                kind: ConstraintKind::Distance {
                    a: face,
                    b: front,
                    axis: Axis::Y,
                    value: len(36 * IN),
                    tolerance: None,
                },
                status: ConstraintStatus::Driving,
                source: Source::UserStated,
                provenance: None,
            },
        );
        let s = solve(&doc)?;
        // Front edge is at 60 - 12 = 48, face at 2: the distance is 46, not 36.
        assert_eq!(s.conflicts.len(), 1);
        assert_eq!(
            s.conflicts
                .first()
                .map(|c| (c.constraint, c.with.contains(&column))),
            Some((uid(3, 9), true))
        );
        assert_eq!(center(&s, column), Some((100, 60)));
        Ok(())
    }

    #[test]
    fn item_edges_follow_rotation() -> Result<(), SolveError> {
        let mut doc = anchored(0);
        // Rotation 1: front faces +X; world size 24 x 48.
        let item = put(&mut doc, 1, floor(0, 0, 1), (0, 0));
        fixed(
            &mut doc,
            5,
            Reference::ItemEdge {
                item,
                side: ItemSide::Front,
            },
            Axis::X,
            200,
        );
        fixed(&mut doc, 6, Reference::ItemCenter { item }, Axis::Y, 50);
        assert_eq!(center(&solve(&doc)?, item), Some((188, 50)));
        // The front edge has no Y coordinate at this rotation.
        fixed(
            &mut doc,
            7,
            Reference::ItemEdge {
                item,
                side: ItemSide::Front,
            },
            Axis::Y,
            0,
        );
        assert_eq!(
            solve(&doc),
            Err(SolveError::WrongAxis {
                constraint: uid(3, 7)
            })
        );
        Ok(())
    }

    #[test]
    fn wall_mounted_back_edge_is_the_wall_face() -> Result<(), SolveError> {
        let mut doc = base();
        fixed(&mut doc, 1, node(1), Axis::X, 0);
        let item = put(&mut doc, 1, on_wall(WallSide::Right, 60, 48), (0, 0));
        // The item's back lies on the wall's right face (line - 2); put it at y = 10.
        fixed(
            &mut doc,
            2,
            Reference::ItemEdge {
                item,
                side: ItemSide::Back,
            },
            Axis::Y,
            10,
        );
        let s = solve(&doc)?;
        assert_eq!(
            s.nodes.get(&uid(1, 1)).map(|p| p.y.nanometers() / IN),
            Some(12)
        );
        assert_eq!(center(&s, item), Some((60, -2)));
        Ok(())
    }

    #[test]
    fn ceiling_items_are_free_like_floor_items() -> Result<(), SolveError> {
        let mut doc = anchored(0);
        let light = put(
            &mut doc,
            1,
            Placement::CeilingMounted {
                x: len(0).into_sourced(),
                y: len(0).into_sourced(),
            },
            (80, 90),
        );
        fixed(
            &mut doc,
            4,
            Reference::ItemCenter { item: light },
            Axis::X,
            120,
        );
        assert_eq!(center(&solve(&doc)?, light), Some((120, 90)));
        Ok(())
    }

    trait IntoSourced {
        fn into_sourced(self) -> Sourced<Length>;
    }

    impl IntoSourced for Length {
        fn into_sourced(self) -> Sourced<Length> {
            Sourced {
                value: self,
                source: Source::UserStated,
            }
        }
    }
}
