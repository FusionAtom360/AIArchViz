//! Wall geometry: layer polygons, footprints and joins (spec 6.1; Q-17 to Q-19).
//!
//! Each wall runs along X or Y from node_a to node_b in its stored
//! direction. Its layers are ordered left to right (left is 90 degrees
//! counterclockwise from the direction); the left face is `left_extent` from
//! the reference line. Each end is cut by the join at its node:
//!
//! - one wall at the node (free end), two collinear walls (straight), or a
//!   wall in a through run of two: square at the node;
//! - two perpendicular walls (L): mitered along the line from the corner of
//!   the inner faces to the corner of the outer faces. Footprint vertices are
//!   exact; an interior layer boundary crossing that line at a non-integer
//!   point is rounded half to even to the nanometer (Q-17);
//! - three or four walls (T, X): walls from opposite sides form a run. The
//!   run holding the lowest join_priority (ties: lowest wall id) runs
//!   through. A through run of one wall (a T stem) extends to the far face of
//!   the crossing walls. Every other wall butts, ending at the through run's
//!   face that reaches farthest toward it (Q-18).
//!
//! Polygons are computed on demand, never stored (Q-19). Each is listed
//! counterclockwise from its vertex with the lowest x, then the lowest y.
//!
//! A wall's elevation profile runs along its axis in its direction and up
//! from its base to its top; each opening voids a rectangle from its start
//! to its end edge and from its sill (above the storey elevation) to its
//! head. An opening must lie in the stretch where its host has full
//! thickness on both faces, the minimum jamb distance from either end, and
//! between the wall's base and top (7.7, Q-22).

use std::collections::BTreeMap;
use std::fmt;

use crate::document::{Direction, Document, JoinType, Point, WallTop};
use crate::identity::Uuid;
use crate::length::Length;

/// A wall's outline and its layers' outlines, layers listed left to right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallGeometry {
    pub footprint: Vec<Point>,
    pub layers: Vec<Vec<Point>>,
}

/// Why wall geometry cannot be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeometryError {
    UnknownEntity {
        id: Uuid,
    },
    /// The wall's nodes are not on one line in its direction, or it has no length.
    BadWall {
        wall: Uuid,
    },
    /// Two walls leave this node in the same direction (Q-19).
    SameDirection {
        node: Uuid,
    },
    /// A coordinate is outside the Length range.
    Overflow,
}

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GeometryError::UnknownEntity { id } => write!(f, "no entity {id}"),
            GeometryError::BadWall { wall } => {
                write!(
                    f,
                    "wall {wall} does not run from node_a to node_b in its direction"
                )
            }
            GeometryError::SameDirection { node } => {
                write!(f, "two walls leave node {node} in the same direction")
            }
            GeometryError::Overflow => f.write_str("a wall coordinate is out of range"),
        }
    }
}

impl std::error::Error for GeometryError {}

/// A wall's frame: which axis it runs along, the sign of its direction on
/// that axis, and the sign of its left normal on the other axis (6.1).
#[derive(Debug, Clone, Copy)]
struct Frame {
    along_x: bool,
    dir: i128,
    left: i128,
}

fn frame(direction: Direction) -> Frame {
    match direction {
        Direction::PosX => Frame {
            along_x: true,
            dir: 1,
            left: 1,
        },
        Direction::NegX => Frame {
            along_x: true,
            dir: -1,
            left: -1,
        },
        Direction::PosY => Frame {
            along_x: false,
            dir: 1,
            left: -1,
        },
        Direction::NegY => Frame {
            along_x: false,
            dir: -1,
            left: 1,
        },
    }
}

/// A wall in its own (along, across) coordinates.
struct WallInfo {
    id: Uuid,
    frame: Frame,
    priority: u32,
    nodes: [Uuid; 2],
    /// Along coordinates of node_a and node_b.
    ends: [i128; 2],
    /// Across coordinates of the layer boundaries, left face first.
    bounds: Vec<i128>,
    lo: i128,
    hi: i128,
}

/// How a wall end is cut, in the wall's (along, across) coordinates.
#[derive(Debug, Clone, Copy)]
enum Cut {
    Square(i128),
    /// The line through `inner` and `outer`, two (along, across) points.
    Miter {
        inner: (i128, i128),
        outer: (i128, i128),
    },
}

/// `n / d` rounded half to even; `d` must not be zero.
fn round_div(n: i128, d: i128) -> i128 {
    let (n, d) = if d < 0 { (-n, -d) } else { (n, d) };
    let q = n / d;
    let r = n % d;
    let twice = 2 * r.abs();
    if twice > d || (twice == d && q % 2 != 0) {
        q + r.signum()
    } else {
        q
    }
}

impl Cut {
    /// The along coordinate of the cut at across coordinate `v`.
    fn at(self, v: i128) -> i128 {
        match self {
            Cut::Square(u) => u,
            Cut::Miter { inner, outer } => {
                let dv = outer.1 - inner.1;
                if dv == 0 {
                    inner.0
                } else {
                    inner.0 + round_div((v - inner.1) * (outer.0 - inner.0), dv)
                }
            }
        }
    }
}

fn walls(document: &Document) -> Result<Vec<WallInfo>, GeometryError> {
    let mut out = Vec::new();
    for (id, w) in &document.walls {
        let f = frame(w.direction);
        let node = |n: Uuid| {
            document
                .nodes
                .get(&n)
                .ok_or(GeometryError::UnknownEntity { id: n })
        };
        let (a, b) = (node(w.node_a)?, node(w.node_b)?);
        let split = |p: &crate::document::Node| {
            let (x, y) = (i128::from(p.x.nanometers()), i128::from(p.y.nanometers()));
            if f.along_x { (x, y) } else { (y, x) }
        };
        let ((ua, va), (ub, vb)) = (split(a), split(b));
        if va != vb || (ub - ua) * f.dir <= 0 {
            return Err(GeometryError::BadWall { wall: *id });
        }
        let set = document
            .layer_sets
            .get(&w.layer_set)
            .ok_or(GeometryError::UnknownEntity { id: w.layer_set })?;
        let mut bounds = Vec::with_capacity(set.layers.len() + 1);
        let mut from_left = i128::from(w.left_extent.value.nanometers());
        bounds.push(va + f.left * from_left);
        for layer in &set.layers {
            from_left -= i128::from(layer.thickness.value.nanometers());
            bounds.push(va + f.left * from_left);
        }
        let (first, last) = (bounds[0], bounds[bounds.len() - 1]);
        out.push(WallInfo {
            id: *id,
            frame: f,
            priority: w.join_priority,
            nodes: [w.node_a, w.node_b],
            ends: [ua, ub],
            lo: first.min(last),
            hi: first.max(last),
            bounds,
        });
    }
    Ok(out)
}

/// One wall arriving at a node: its index, which end, and the sign of the
/// direction its body extends from the node along its own axis.
#[derive(Debug, Clone, Copy)]
struct Arm {
    wall: usize,
    end: usize,
    body: i128,
}

/// The cut at each (wall index, end) pair.
type Cuts = BTreeMap<(usize, usize), Cut>;

/// Join types per node and the cut at each wall end.
fn joins(
    document: &Document,
    walls: &[WallInfo],
) -> Result<(BTreeMap<Uuid, JoinType>, Cuts), GeometryError> {
    let mut arms: BTreeMap<Uuid, Vec<Arm>> =
        document.nodes.keys().map(|id| (*id, Vec::new())).collect();
    for (i, w) in walls.iter().enumerate() {
        for end in 0..2 {
            let body = if end == 0 { w.frame.dir } else { -w.frame.dir };
            arms.entry(w.nodes[end])
                .or_default()
                .push(Arm { wall: i, end, body });
        }
    }
    let mut types = BTreeMap::new();
    let mut cuts = BTreeMap::new();
    for (node, arms) in &arms {
        let mut seen = std::collections::BTreeSet::new();
        for a in arms {
            if !seen.insert((walls[a.wall].frame.along_x, a.body)) {
                return Err(GeometryError::SameDirection { node: *node });
            }
        }
        let at_node = |a: &Arm| walls[a.wall].ends[a.end];
        let join = match arms.as_slice() {
            [] => JoinType::Isolated,
            [a] => {
                cuts.insert((a.wall, a.end), Cut::Square(at_node(a)));
                JoinType::FreeEnd
            }
            [a, b] if walls[a.wall].frame.along_x == walls[b.wall].frame.along_x => {
                cuts.insert((a.wall, a.end), Cut::Square(at_node(a)));
                cuts.insert((b.wall, b.end), Cut::Square(at_node(b)));
                JoinType::Straight
            }
            [a, b] => {
                let (wa, wb) = (&walls[a.wall], &walls[b.wall]);
                // In A's frame: along = B's across, across = B's along.
                let (outer_u, inner_u) = if a.body > 0 {
                    (wb.lo, wb.hi)
                } else {
                    (wb.hi, wb.lo)
                };
                let (outer_v, inner_v) = if b.body > 0 {
                    (wa.lo, wa.hi)
                } else {
                    (wa.hi, wa.lo)
                };
                cuts.insert(
                    (a.wall, a.end),
                    Cut::Miter {
                        inner: (inner_u, inner_v),
                        outer: (outer_u, outer_v),
                    },
                );
                cuts.insert(
                    (b.wall, b.end),
                    Cut::Miter {
                        inner: (inner_v, inner_u),
                        outer: (outer_v, outer_u),
                    },
                );
                JoinType::L
            }
            _ => {
                let lowest = arms
                    .iter()
                    .min_by_key(|a| (walls[a.wall].priority, walls[a.wall].id))
                    .map(|a| walls[a.wall].frame.along_x);
                let (through, crossing): (Vec<&Arm>, Vec<&Arm>) = arms
                    .iter()
                    .partition(|a| Some(walls[a.wall].frame.along_x) == lowest);
                let extreme = |set: &[&Arm], low_side: bool| {
                    let it = set.iter().map(|a| {
                        if low_side {
                            walls[a.wall].lo
                        } else {
                            walls[a.wall].hi
                        }
                    });
                    if low_side { it.min() } else { it.max() }
                };
                for c in &crossing {
                    let end = extreme(&through, c.body < 0).unwrap_or(at_node(c));
                    cuts.insert((c.wall, c.end), Cut::Square(end));
                }
                for t in &through {
                    let end = if through.len() == 1 {
                        extreme(&crossing, t.body > 0).unwrap_or(at_node(t))
                    } else {
                        at_node(t)
                    };
                    cuts.insert((t.wall, t.end), Cut::Square(end));
                }
                if arms.len() == 3 {
                    JoinType::T
                } else {
                    JoinType::X
                }
            }
        };
        types.insert(*node, join);
    }
    Ok((types, cuts))
}

/// Orders a polygon counterclockwise from its lowest-x, then lowest-y vertex.
fn normalize(points: Vec<(i128, i128)>) -> Result<Vec<Point>, GeometryError> {
    let n = points.len();
    let twice_area: i128 = (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum();
    let mut points = points;
    if twice_area < 0 {
        points.reverse();
    }
    if let Some(start) = (0..n).min_by_key(|&i| points[i]) {
        points.rotate_left(start);
    }
    points
        .into_iter()
        .map(|(x, y)| {
            let x = i64::try_from(x).map_err(|_| GeometryError::Overflow)?;
            let y = i64::try_from(y).map_err(|_| GeometryError::Overflow)?;
            Ok(Point {
                x: Length::from_nanometers(x),
                y: Length::from_nanometers(y),
            })
        })
        .collect()
}

/// The join type at every node (6.1, Q-19).
pub fn node_joins(document: &Document) -> Result<BTreeMap<Uuid, JoinType>, GeometryError> {
    let walls = walls(document)?;
    Ok(joins(document, &walls)?.0)
}

/// Stores every node's derived join type (Q-7).
pub fn update_joins(document: &mut Document) -> Result<(), GeometryError> {
    let joins = node_joins(document)?;
    for (id, join) in joins {
        if let Some(node) = document.nodes.get_mut(&id) {
            node.join = join;
        }
    }
    Ok(())
}

/// Every wall's footprint and layer polygons.
pub fn wall_geometry(document: &Document) -> Result<BTreeMap<Uuid, WallGeometry>, GeometryError> {
    let walls = walls(document)?;
    let (_, cuts) = joins(document, &walls)?;
    let mut out = BTreeMap::new();
    for (i, w) in walls.iter().enumerate() {
        let cut = |end: usize| {
            cuts.get(&(i, end))
                .copied()
                .unwrap_or(Cut::Square(w.ends[end]))
        };
        let (ca, cb) = (cut(0), cut(1));
        let quad = |v1: i128, v2: i128| {
            let uv = [
                (ca.at(v1), v1),
                (cb.at(v1), v1),
                (cb.at(v2), v2),
                (ca.at(v2), v2),
            ];
            let xy = uv
                .iter()
                .map(|&(u, v)| if w.frame.along_x { (u, v) } else { (v, u) })
                .collect();
            normalize(xy)
        };
        let footprint = quad(w.bounds[0], w.bounds[w.bounds.len() - 1])?;
        let layers = w
            .bounds
            .windows(2)
            .map(|pair| quad(pair[0], pair[1]))
            .collect::<Result<Vec<_>, _>>()?;
        out.insert(w.id, WallGeometry { footprint, layers });
    }
    Ok(out)
}

/// The void an opening cuts in its host's elevation profile (Q-22).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Void {
    pub opening: Uuid,
    /// Start and end edges on the host's along axis (start nearer node_a).
    pub start: Length,
    pub end: Length,
    /// Absolute elevations: storey elevation + sill, and sill + height.
    pub sill: Length,
    pub head: Length,
}

/// A wall's elevation profile: along its axis in its direction, and up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallProfile {
    /// The outermost extent of the wall's end cuts, at node_a's and node_b's ends.
    pub start: Length,
    pub end: Length,
    /// The stretch where the wall has its full thickness on both faces.
    pub full_start: Length,
    pub full_end: Length,
    /// Absolute elevations of the wall's base and top.
    pub base: Length,
    pub top: Length,
    /// Ordered along the wall.
    pub voids: Vec<Void>,
}

/// What is wrong with an opening's fit in its host (7.7, Q-22).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpeningProblem {
    /// Width or height is zero or negative.
    NonPositiveSize,
    /// Closer than the minimum jamb distance to the start of the
    /// full-thickness stretch; negative clearance means outside it.
    TooCloseToStart { clearance: Length },
    /// The same at the end.
    TooCloseToEnd { clearance: Length },
    /// The sill is below the wall's base by this much.
    BelowBase { by: Length },
    /// The head is above the wall's top by this much.
    AboveTop { by: Length },
}

/// One problem with one opening.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpeningViolation {
    pub opening: Uuid,
    pub problem: OpeningProblem,
}

/// Every wall's elevation profile with its openings' voids.
pub fn wall_profiles(document: &Document) -> Result<BTreeMap<Uuid, WallProfile>, GeometryError> {
    let walls = walls(document)?;
    let (_, cuts) = joins(document, &walls)?;
    let to_len = |v: i128| {
        i64::try_from(v)
            .map(Length::from_nanometers)
            .map_err(|_| GeometryError::Overflow)
    };
    let mut out = BTreeMap::new();
    for (i, w) in walls.iter().enumerate() {
        let wall = document
            .walls
            .get(&w.id)
            .ok_or(GeometryError::UnknownEntity { id: w.id })?;
        let storey = document
            .storeys
            .get(&wall.storey)
            .ok_or(GeometryError::UnknownEntity { id: wall.storey })?;
        let elevation = i128::from(storey.elevation.value.nanometers());
        let base = elevation + i128::from(wall.base_offset.value.nanometers());
        let top = match &wall.top {
            WallTop::Storey => elevation + i128::from(storey.floor_to_floor.value.nanometers()),
            WallTop::Height { height } => base + i128::from(height.value.nanometers()),
        };
        let cut = |end: usize| {
            cuts.get(&(i, end))
                .copied()
                .unwrap_or(Cut::Square(w.ends[end]))
        };
        let faces = [w.bounds[0], w.bounds[w.bounds.len() - 1]];
        let dir = w.frame.dir;
        // Outermost and innermost of the two faces' cuts, seen along the direction.
        let span = |c: Cut, toward: i128| {
            let (p, q) = (c.at(faces[0]), c.at(faces[1]));
            let (lo, hi) = (p.min(q), p.max(q));
            if toward > 0 { (hi, lo) } else { (lo, hi) }
        };
        let (start, full_start) = span(cut(0), -dir);
        let (end, full_end) = span(cut(1), dir);
        let mut voids: Vec<(i128, Void)> = Vec::new();
        for (id, o) in document.openings.iter().filter(|(_, o)| o.host == w.id) {
            let s0 = i128::from(o.start.nanometers());
            let sill = elevation + i128::from(o.sill.value.nanometers());
            let void = Void {
                opening: *id,
                start: o.start,
                end: to_len(s0 + dir * i128::from(o.width.value.nanometers()))?,
                sill: to_len(sill)?,
                head: to_len(sill + i128::from(o.height.value.nanometers()))?,
            };
            voids.push((s0 * dir, void));
        }
        voids.sort_by_key(|a| (a.0, a.1.opening));
        out.insert(
            w.id,
            WallProfile {
                start: to_len(start)?,
                end: to_len(end)?,
                full_start: to_len(full_start)?,
                full_end: to_len(full_end)?,
                base: to_len(base)?,
                top: to_len(top)?,
                voids: voids.into_iter().map(|(_, v)| v).collect(),
            },
        );
    }
    Ok(out)
}

/// Every opening that does not lie fully inside its host (7.7, Q-22), in
/// opening id order.
pub fn opening_violations(
    document: &Document,
    min_jamb: Length,
) -> Result<Vec<OpeningViolation>, GeometryError> {
    let profiles = wall_profiles(document)?;
    let jamb = i128::from(min_jamb.nanometers());
    let n = |l: Length| i128::from(l.nanometers());
    let to_len = |v: i128| {
        i64::try_from(v)
            .map(Length::from_nanometers)
            .map_err(|_| GeometryError::Overflow)
    };
    let mut out = Vec::new();
    for (id, o) in &document.openings {
        let host = document
            .walls
            .get(&o.host)
            .ok_or(GeometryError::UnknownEntity { id: o.host })?;
        let p = profiles
            .get(&o.host)
            .ok_or(GeometryError::UnknownEntity { id: o.host })?;
        let mut push = |problem| {
            out.push(OpeningViolation {
                opening: *id,
                problem,
            })
        };
        if o.width.value.nanometers() <= 0 || o.height.value.nanometers() <= 0 {
            push(OpeningProblem::NonPositiveSize);
            continue;
        }
        let Some(v) = p.voids.iter().find(|v| v.opening == *id) else {
            continue;
        };
        let dir = frame(host.direction).dir;
        let before = (n(v.start) - n(p.full_start)) * dir;
        if before < jamb {
            push(OpeningProblem::TooCloseToStart {
                clearance: to_len(before)?,
            });
        }
        let after = (n(p.full_end) - n(v.end)) * dir;
        if after < jamb {
            push(OpeningProblem::TooCloseToEnd {
                clearance: to_len(after)?,
            });
        }
        if v.sill < p.base {
            push(OpeningProblem::BelowBase {
                by: to_len(n(p.base) - n(v.sill))?,
            });
        }
        if v.head > p.top {
            push(OpeningProblem::AboveTop {
                by: to_len(n(v.head) - n(p.top))?,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::*;
    use crate::identity::LabelAllocator;
    use crate::length::Length;
    use crate::units::{ProjectUnit, format_length};
    use proptest::prelude::*;
    use std::fmt::Write as _;

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
    fn layers_id(n: u8) -> Uuid {
        uid(5, n)
    }

    fn sourced(nm: i64) -> Sourced<Length> {
        Sourced {
            value: len(nm),
            source: Source::UserStated,
        }
    }

    /// A wall spec in inches: thicknesses left to right and the left extent.
    #[derive(Debug, Clone)]
    struct Spec {
        a: u8,
        b: u8,
        direction: Direction,
        layers: Vec<i64>,
        left: i64,
        priority: u32,
    }

    fn spec(a: u8, b: u8, direction: Direction, layers: &[i64], left: i64, priority: u32) -> Spec {
        Spec {
            a,
            b,
            direction,
            layers: layers.to_vec(),
            left,
            priority,
        }
    }

    /// Builds a document from node positions (inches) and wall specs. Each
    /// wall gets its own layer set; `order` lists the walls in insertion order.
    fn build(nodes: &[(u8, i64, i64)], walls: &[(u8, Spec)]) -> Document {
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
            uid(8, 0),
            Storey {
                name: "Ground".into(),
                elevation: sourced(0),
                floor_to_floor: sourced(120 * IN),
                default_ceiling_height: sourced(108 * IN),
            },
        );
        for &(n, x, y) in nodes {
            doc.nodes.insert(
                node_id(n),
                Node {
                    x: len(x * IN),
                    y: len(y * IN),
                    join: JoinType::Isolated,
                },
            );
        }
        let mut labels = LabelAllocator::new();
        for (n, s) in walls {
            doc.layer_sets.insert(
                layers_id(*n),
                LayerSet {
                    name: format!("set {n}"),
                    layers: s
                        .layers
                        .iter()
                        .map(|t| Layer {
                            material: uid(6, 0),
                            thickness: sourced(t * IN),
                            core: false,
                        })
                        .collect(),
                },
            );
            doc.walls.insert(
                wall_id(*n),
                Wall {
                    label: labels.allocate("W").unwrap_or_else(|_| unreachable!()),
                    storey: uid(8, 0),
                    node_a: node_id(s.a),
                    node_b: node_id(s.b),
                    direction: s.direction,
                    layer_set: layers_id(*n),
                    left_extent: sourced(s.left * IN),
                    base_offset: sourced(0),
                    top: WallTop::Storey,
                    join_priority: s.priority,
                    function: Function::New,
                },
            );
        }
        doc
    }

    use Direction::{NegX, NegY, PosX, PosY};

    /// L: a 4 in wall along +X from (0,0) to (100,0), then along +Y to (100,80).
    fn l_join() -> Document {
        build(
            &[(1, 0, 0), (2, 100, 0), (3, 100, 80)],
            &[
                (1, spec(1, 2, PosX, &[4], 2, 0)),
                (2, spec(2, 3, PosY, &[4], 2, 0)),
            ],
        )
    }

    /// T: walls 1 and 2 collinear along X through node 2; wall 3 the stem to +Y.
    fn t_join(stem_priority: u32) -> Document {
        build(
            &[(1, 0, 0), (2, 100, 0), (3, 200, 0), (4, 100, 60)],
            &[
                (1, spec(1, 2, PosX, &[6], 3, 1)),
                (2, spec(2, 3, PosX, &[6], 3, 1)),
                (3, spec(2, 4, PosY, &[4], 2, stem_priority)),
            ],
        )
    }

    /// X: four walls meeting at node 5; the Y pair has the lower priority.
    fn x_join() -> Document {
        build(
            &[(1, -50, 0), (2, 50, 0), (3, 0, -40), (4, 0, 40), (5, 0, 0)],
            &[
                (1, spec(1, 5, PosX, &[6], 3, 2)),
                (2, spec(5, 2, PosX, &[6], 3, 2)),
                (3, spec(3, 5, PosY, &[4], 2, 1)),
                (4, spec(5, 4, PosY, &[4], 2, 1)),
            ],
        )
    }

    /// A free-standing three-layer wall: 1/2 in gypsum, 3 1/2 in stud, 1/2 in
    /// gypsum, reference line on the stud center.
    fn three_layer() -> Document {
        let doc = build(
            &[(1, 0, 0), (2, 0, 120)],
            &[(1, spec(1, 2, PosY, &[0, 0, 0], 0, 0))],
        );
        let mut doc = doc;
        if let Some(set) = doc.layer_sets.get_mut(&layers_id(1)) {
            let t = [IN / 2, 3 * IN + IN / 2, IN / 2];
            for (layer, t) in set.layers.iter_mut().zip(t) {
                layer.thickness = sourced(t);
            }
        }
        if let Some(w) = doc.walls.get_mut(&wall_id(1)) {
            w.left_extent = sourced(IN / 2 + (3 * IN + IN / 2) / 2);
        }
        doc
    }

    /// L between a 6 in and a 4 in wall, each with two layers: the interior
    /// boundary points on the miter are not exact (Q-17).
    fn unequal_l() -> Document {
        build(
            &[(1, 0, 0), (2, 100, 0), (3, 100, 80)],
            // Wall 2's boundary at x = 98 meets the miter at y = 3 - 6/7 in.
            &[
                (1, spec(1, 2, PosX, &[3, 3], 3, 0)),
                (2, spec(2, 3, PosY, &[1, 6], 3, 0)),
            ],
        )
    }

    fn pt(x: i64, y: i64) -> Point {
        Point {
            x: len(x * IN),
            y: len(y * IN),
        }
    }

    fn polygon_text(p: &[Point]) -> String {
        p.iter()
            .map(|v| format!("({}, {})", format_length(v.x), format_length(v.y)))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// A readable golden: per wall, the footprint and each layer, in
    /// feet-inches (with ~ where off the 1/16 in grid) and in nanometers.
    fn golden(doc: &Document) -> String {
        let mut out = String::new();
        let joins = node_joins(doc);
        let geometry = wall_geometry(doc);
        let (Ok(joins), Ok(geometry)) = (joins, geometry) else {
            return format!(
                "error: {:?} {:?}",
                node_joins(doc).err(),
                wall_geometry(doc).err()
            );
        };
        for (id, j) in &joins {
            let _ = writeln!(out, "node {id}: {j:?}");
        }
        for (id, g) in &geometry {
            let label = doc
                .walls
                .get(id)
                .map(|w| w.label.to_string())
                .unwrap_or_default();
            let _ = writeln!(out, "{label} footprint: {}", polygon_text(&g.footprint));
            let _ = writeln!(
                out,
                "  nm: {:?}",
                g.footprint
                    .iter()
                    .map(|p| (p.x.nanometers(), p.y.nanometers()))
                    .collect::<Vec<_>>()
            );
            for (k, layer) in g.layers.iter().enumerate() {
                let _ = writeln!(out, "  layer {k}: {}", polygon_text(layer));
                let _ = writeln!(
                    out,
                    "    nm: {:?}",
                    layer
                        .iter()
                        .map(|p| (p.x.nanometers(), p.y.nanometers()))
                        .collect::<Vec<_>>()
                );
            }
        }
        out
    }

    #[test]
    fn golden_l_join() {
        insta::assert_snapshot!(golden(&l_join()));
    }

    #[test]
    fn golden_t_join_pair_through() {
        insta::assert_snapshot!(golden(&t_join(5)));
    }

    #[test]
    fn golden_t_join_stem_through() {
        insta::assert_snapshot!(golden(&t_join(0)));
    }

    #[test]
    fn golden_x_join() {
        insta::assert_snapshot!(golden(&x_join()));
    }

    #[test]
    fn golden_three_layer_wall() {
        insta::assert_snapshot!(golden(&three_layer()));
    }

    #[test]
    fn golden_unequal_l_join() {
        insta::assert_snapshot!(golden(&unequal_l()));
    }

    #[test]
    fn l_join_is_mitered_exactly() -> Result<(), GeometryError> {
        let g = wall_geometry(&l_join())?;
        // Wall 1 faces y = -2 and 2; wall 2 faces x = 98 and 102.
        // Outer corner (102, -2), inner corner (98, 2).
        assert_eq!(
            g.get(&wall_id(1)).map(|w| w.footprint.clone()),
            Some(vec![pt(0, -2), pt(102, -2), pt(98, 2), pt(0, 2)])
        );
        assert_eq!(
            g.get(&wall_id(2)).map(|w| w.footprint.clone()),
            Some(vec![pt(98, 2), pt(102, -2), pt(102, 80), pt(98, 80)])
        );
        Ok(())
    }

    #[test]
    fn t_join_butts_the_stem_against_the_through_pair() -> Result<(), GeometryError> {
        let g = wall_geometry(&t_join(5))?;
        // The pair's faces are y = -3 and 3; the stem (x 98..102) starts at y = 3.
        assert_eq!(
            g.get(&wall_id(3)).map(|w| w.footprint.clone()),
            Some(vec![pt(98, 3), pt(102, 3), pt(102, 60), pt(98, 60)])
        );
        assert_eq!(
            g.get(&wall_id(1)).map(|w| w.footprint.clone()),
            Some(vec![pt(0, -3), pt(100, -3), pt(100, 3), pt(0, 3)])
        );
        // With the stem lowest, it runs through to y = -3 and the pair butts at x = 98 and 102.
        let g = wall_geometry(&t_join(0))?;
        assert_eq!(
            g.get(&wall_id(3)).map(|w| w.footprint.clone()),
            Some(vec![pt(98, -3), pt(102, -3), pt(102, 60), pt(98, 60)])
        );
        assert_eq!(
            g.get(&wall_id(1)).map(|w| w.footprint.clone()),
            Some(vec![pt(0, -3), pt(98, -3), pt(98, 3), pt(0, 3)])
        );
        assert_eq!(
            g.get(&wall_id(2)).map(|w| w.footprint.clone()),
            Some(vec![pt(102, -3), pt(200, -3), pt(200, 3), pt(102, 3)])
        );
        Ok(())
    }

    #[test]
    fn join_types() -> Result<(), GeometryError> {
        let mut doc = build(
            &[(1, 0, 0), (2, 10, 0), (3, 20, 0), (9, 50, 50)],
            &[
                (1, spec(1, 2, PosX, &[4], 2, 0)),
                (2, spec(2, 3, PosX, &[4], 2, 0)),
            ],
        );
        let joins = node_joins(&doc)?;
        assert_eq!(joins.get(&node_id(1)), Some(&JoinType::FreeEnd));
        assert_eq!(joins.get(&node_id(2)), Some(&JoinType::Straight));
        assert_eq!(joins.get(&node_id(9)), Some(&JoinType::Isolated));
        assert_eq!(node_joins(&l_join())?.get(&node_id(2)), Some(&JoinType::L));
        assert_eq!(node_joins(&t_join(5))?.get(&node_id(2)), Some(&JoinType::T));
        assert_eq!(node_joins(&x_join())?.get(&node_id(5)), Some(&JoinType::X));
        update_joins(&mut doc)?;
        assert_eq!(
            doc.nodes.get(&node_id(2)).map(|n| n.join),
            Some(JoinType::Straight)
        );
        // Collinear walls meet square at the node.
        let g = wall_geometry(&doc)?;
        assert_eq!(
            g.get(&wall_id(1)).map(|w| w.footprint.clone()),
            Some(vec![pt(0, -2), pt(10, -2), pt(10, 2), pt(0, 2)])
        );
        Ok(())
    }

    #[test]
    fn bad_input_is_an_error() {
        // Two walls leaving node 1 in the same direction.
        let doc = build(
            &[(1, 0, 0), (2, 10, 0), (3, 20, 0)],
            &[
                (1, spec(1, 2, PosX, &[4], 2, 0)),
                (2, spec(1, 3, PosX, &[4], 2, 0)),
            ],
        );
        assert_eq!(
            wall_geometry(&doc),
            Err(GeometryError::SameDirection { node: node_id(1) })
        );
        // node_b is behind node_a for a +X wall.
        let doc = build(
            &[(1, 10, 0), (2, 0, 0)],
            &[(1, spec(1, 2, PosX, &[4], 2, 0))],
        );
        assert_eq!(
            wall_geometry(&doc),
            Err(GeometryError::BadWall { wall: wall_id(1) })
        );
        // Nodes not on one horizontal line.
        let doc = build(
            &[(1, 0, 0), (2, 10, 1)],
            &[(1, spec(1, 2, PosX, &[4], 2, 0))],
        );
        assert_eq!(
            wall_geometry(&doc),
            Err(GeometryError::BadWall { wall: wall_id(1) })
        );
        // Zero length.
        let doc = build(
            &[(1, 0, 0), (2, 0, 0)],
            &[(1, spec(1, 2, PosX, &[4], 2, 0))],
        );
        assert_eq!(
            wall_geometry(&doc),
            Err(GeometryError::BadWall { wall: wall_id(1) })
        );
    }

    fn opening_id(n: u8) -> Uuid {
        uid(10, n)
    }

    /// An opening with its solved start already stored, all sizes in inches.
    fn put_opening(
        doc: &mut Document,
        n: u8,
        host: u8,
        start: i64,
        width: i64,
        height: i64,
        sill: i64,
    ) {
        doc.openings.insert(
            opening_id(n),
            Opening {
                kind: OpeningKind::Window,
                host: wall_id(host),
                reference: Reference::Node { node: node_id(1) },
                anchor: OpeningEdge::Start,
                offset: sourced(0),
                width: sourced(width * IN),
                height: sourced(height * IN),
                sill: sourced(sill * IN),
                start: len(start * IN),
            },
        );
    }

    fn free_wall(direction: Direction) -> Document {
        let (a, b) = if direction == PosX {
            (0, 100)
        } else {
            (100, 0)
        };
        build(
            &[(1, a, 0), (2, b, 0)],
            &[(1, spec(1, 2, direction, &[4], 2, 0))],
        )
    }

    fn inches(l: Length) -> i64 {
        l.nanometers() / IN
    }

    #[test]
    fn profile_and_voids_of_a_free_wall() -> Result<(), GeometryError> {
        let mut doc = free_wall(PosX);
        put_opening(&mut doc, 1, 1, 10, 36, 80, 0);
        put_opening(&mut doc, 2, 1, 60, 30, 48, 36);
        let p = wall_profiles(&doc)?.remove(&wall_id(1));
        let p = p.ok_or(GeometryError::UnknownEntity { id: wall_id(1) })?;
        assert_eq!(
            [p.start, p.end, p.full_start, p.full_end, p.base, p.top].map(inches),
            [0, 100, 0, 100, 0, 120]
        );
        let voids: Vec<_> = p
            .voids
            .iter()
            .map(|v| (v.opening, [v.start, v.end, v.sill, v.head].map(inches)))
            .collect();
        assert_eq!(
            voids,
            vec![
                (opening_id(1), [10, 46, 0, 80]),
                (opening_id(2), [60, 90, 36, 84])
            ]
        );
        Ok(())
    }

    #[test]
    fn profile_runs_in_the_wall_direction() -> Result<(), GeometryError> {
        let mut doc = free_wall(NegX);
        put_opening(&mut doc, 1, 1, 90, 36, 80, 0);
        let p = wall_profiles(&doc)?
            .remove(&wall_id(1))
            .ok_or(GeometryError::Overflow)?;
        assert_eq!([p.start, p.end].map(inches), [100, 0]);
        assert_eq!(
            p.voids.first().map(|v| [v.start, v.end].map(inches)),
            Some([90, 54])
        );
        Ok(())
    }

    #[test]
    fn profile_heights_follow_storey_base_and_top() -> Result<(), GeometryError> {
        let mut doc = free_wall(PosX);
        if let Some(st) = doc.storeys.get_mut(&uid(8, 0)) {
            st.elevation = sourced(12 * IN);
        }
        if let Some(w) = doc.walls.get_mut(&wall_id(1)) {
            w.base_offset = sourced(-6 * IN);
            w.top = WallTop::Height {
                height: sourced(90 * IN),
            };
        }
        put_opening(&mut doc, 1, 1, 10, 36, 48, 30);
        let p = wall_profiles(&doc)?
            .remove(&wall_id(1))
            .ok_or(GeometryError::Overflow)?;
        assert_eq!([p.base, p.top].map(inches), [6, 96]);
        // The sill is measured from the storey elevation, not the wall base.
        assert_eq!(
            p.voids.first().map(|v| [v.sill, v.head].map(inches)),
            Some([42, 90])
        );
        Ok(())
    }

    #[test]
    fn full_thickness_stretch_stops_at_joins() -> Result<(), GeometryError> {
        // L: wall 1's faces end at x = 102 (outer) and 98 (inner).
        let p = wall_profiles(&l_join())?
            .remove(&wall_id(1))
            .ok_or(GeometryError::Overflow)?;
        assert_eq!(
            [p.start, p.end, p.full_start, p.full_end].map(inches),
            [0, 102, 0, 98]
        );
        // T with the stem through: wall 1 butts at x = 98.
        let p = wall_profiles(&t_join(0))?
            .remove(&wall_id(1))
            .ok_or(GeometryError::Overflow)?;
        assert_eq!(
            [p.start, p.end, p.full_start, p.full_end].map(inches),
            [0, 98, 0, 98]
        );
        Ok(())
    }

    #[test]
    fn an_opening_inside_its_host_is_valid() -> Result<(), GeometryError> {
        let mut doc = free_wall(PosX);
        put_opening(&mut doc, 1, 1, 4, 36, 80, 0);
        put_opening(&mut doc, 2, 1, 60, 36, 48, 72);
        assert_eq!(opening_violations(&doc, len(4 * IN))?, vec![]);
        Ok(())
    }

    #[test]
    fn an_opening_outside_its_host_fails_validation() -> Result<(), GeometryError> {
        let v = |doc: &Document| opening_violations(doc, len(4 * IN));
        let one = |opening, problem| vec![OpeningViolation { opening, problem }];
        // Past the end of a free wall.
        let mut doc = free_wall(PosX);
        put_opening(&mut doc, 1, 1, 80, 36, 80, 0);
        assert_eq!(
            v(&doc)?,
            one(
                opening_id(1),
                OpeningProblem::TooCloseToEnd {
                    clearance: len(-16 * IN)
                }
            )
        );
        // Inside, but within the 4 in jamb distance of the start.
        let mut doc = free_wall(PosX);
        put_opening(&mut doc, 1, 1, 2, 36, 80, 0);
        assert_eq!(
            v(&doc)?,
            one(
                opening_id(1),
                OpeningProblem::TooCloseToStart {
                    clearance: len(2 * IN)
                }
            )
        );
        // Into the miter of an L: full thickness ends at 98, the opening at 96.
        let mut doc = l_join();
        put_opening(&mut doc, 1, 1, 60, 36, 80, 0);
        assert_eq!(
            v(&doc)?,
            one(
                opening_id(1),
                OpeningProblem::TooCloseToEnd {
                    clearance: len(2 * IN)
                }
            )
        );
        // Into the crossing wall at a T: wall 1 butts at 98, the opening ends at 100.
        let mut doc = t_join(0);
        put_opening(&mut doc, 1, 1, 64, 36, 80, 0);
        assert_eq!(
            v(&doc)?,
            one(
                opening_id(1),
                OpeningProblem::TooCloseToEnd {
                    clearance: len(-2 * IN)
                }
            )
        );
        // Above the top and below the base.
        let mut doc = free_wall(PosX);
        put_opening(&mut doc, 1, 1, 10, 30, 48, 80);
        put_opening(&mut doc, 2, 1, 50, 30, 48, -2);
        assert_eq!(
            v(&doc)?,
            vec![
                OpeningViolation {
                    opening: opening_id(1),
                    problem: OpeningProblem::AboveTop { by: len(8 * IN) }
                },
                OpeningViolation {
                    opening: opening_id(2),
                    problem: OpeningProblem::BelowBase { by: len(2 * IN) }
                },
            ]
        );
        // No size.
        let mut doc = free_wall(PosX);
        put_opening(&mut doc, 1, 1, 10, 0, 48, 0);
        assert_eq!(
            v(&doc)?,
            one(opening_id(1), OpeningProblem::NonPositiveSize)
        );
        // Unknown host.
        let mut doc = free_wall(PosX);
        put_opening(&mut doc, 1, 9, 10, 30, 48, 0);
        assert_eq!(
            v(&doc),
            Err(GeometryError::UnknownEntity { id: wall_id(9) })
        );
        Ok(())
    }

    /// The same wall described from the other end: nodes swapped, direction
    /// reversed, layers reversed, left extent measured from the other side.
    fn reversed(s: &Spec) -> Spec {
        let total: i64 = s.layers.iter().sum();
        Spec {
            a: s.b,
            b: s.a,
            direction: match s.direction {
                PosX => NegX,
                NegX => PosX,
                PosY => NegY,
                NegY => PosY,
            },
            layers: s.layers.iter().rev().copied().collect(),
            left: total - s.left,
            priority: s.priority,
        }
    }

    /// Node positions in inches and wall specs, as `build` takes them.
    type Fixture = (Vec<(u8, i64, i64)>, Vec<(u8, Spec)>);

    fn fixtures() -> Vec<Fixture> {
        let parts = |d: Document| -> Fixture {
            let nodes = d
                .nodes
                .iter()
                .map(|(id, n)| {
                    (
                        id.as_bytes()[15],
                        n.x.nanometers() / IN,
                        n.y.nanometers() / IN,
                    )
                })
                .collect();
            let walls = d
                .walls
                .iter()
                .map(|(id, w)| {
                    let set = d
                        .layer_sets
                        .get(&w.layer_set)
                        .map(|s| {
                            s.layers
                                .iter()
                                .map(|l| l.thickness.value.nanometers() / IN)
                                .collect()
                        })
                        .unwrap_or_default();
                    (
                        id.as_bytes()[15],
                        Spec {
                            a: w.node_a.as_bytes()[15],
                            b: w.node_b.as_bytes()[15],
                            direction: w.direction,
                            layers: set,
                            left: w.left_extent.value.nanometers() / IN,
                            priority: w.join_priority,
                        },
                    )
                })
                .collect();
            (nodes, walls)
        };
        vec![
            parts(l_join()),
            parts(t_join(5)),
            parts(t_join(0)),
            parts(x_join()),
            parts(unequal_l()),
        ]
    }

    proptest! {
        /// Creating the walls in any order, and describing any of them from
        /// the other end, gives the same footprints and the same layers.
        #[test]
        fn independent_of_creation_order_and_wall_sense(
            which in 0usize..5,
            rotate in 0usize..8,
            flips in proptest::collection::vec(any::<bool>(), 4),
        ) {
            let (nodes, walls) = fixtures().swap_remove(which);
            let reference = wall_geometry(&build(&nodes, &walls));
            prop_assert!(reference.is_ok(), "{:?}", reference);
            let reference = reference.map_err(|e| TestCaseError::fail(e.to_string()))?;
            let mut changed: Vec<(u8, Spec)> = walls
                .iter()
                .enumerate()
                .map(|(i, (n, s))| (*n, if flips[i] { reversed(s) } else { s.clone() }))
                .collect();
            let k = rotate % changed.len();
            changed.rotate_left(k);
            let got = wall_geometry(&build(&nodes, &changed)).map_err(|e| TestCaseError::fail(e.to_string()))?;
            for (i, (n, _)) in walls.iter().enumerate() {
                let (a, b) = (reference.get(&wall_id(*n)), got.get(&wall_id(*n)));
                prop_assert_eq!(a.map(|g| &g.footprint), b.map(|g| &g.footprint));
                let mut expected_layers = a.map(|g| g.layers.clone()).unwrap_or_default();
                if flips[i] {
                    expected_layers.reverse();
                }
                prop_assert_eq!(Some(expected_layers), b.map(|g| g.layers.clone()));
            }
        }
    }
}
