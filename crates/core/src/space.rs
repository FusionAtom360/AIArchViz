//! Spaces, slabs and ceilings: rectilinear boundaries in exact integer
//! arithmetic (spec 6; Q-24 to Q-26).
//!
//! A Space's boundary is the region around its seed point that the
//! footprints of walls on its storey enclose. The region is found on a grid
//! cut at every wall footprint's bounding-rectangle edge, then traced as one
//! polygon. The bounding rectangles give the same region as the exact
//! footprints: at an L join the two walls together fill the corner square,
//! miter or not, and every other join is rectilinear. A free-standing wall
//! inside the region is an error in v1 (Q-25).
//!
//! A boundary from several spaces (DEC-5) is the union of their regions on
//! the same grid, grown across every wall that bounds them: from each room
//! cell straight through the cells of walls running across the step, to
//! the first cell no such wall covers, so each bounding wall is included
//! out to its far face; then each wall cell with grown wall cells beside it
//! on both axes, which fills the corners.
//!
//! Every polygon is returned counterclockwise from its vertex with the lowest
//! x, then the lowest y, with collinear vertices removed. Areas are exact
//! square nanometers in i128.

use std::fmt;

use crate::document::{Boundary, Direction, Document, Point};
use crate::geometry::{GeometryError, wall_geometry};
use crate::identity::Uuid;
use crate::length::Length;

/// Why a polygon is not a simple rectilinear polygon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PolygonError {
    TooFewVertices,
    /// Edge `edge` (from vertex `edge` to the next) is diagonal or has no length.
    NotRectilinear {
        edge: usize,
    },
    /// Two edges cross, touch or overlap.
    SelfIntersecting {
        first: usize,
        second: usize,
    },
    NonPositiveArea,
}

impl fmt::Display for PolygonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PolygonError::TooFewVertices => f.write_str("a polygon needs at least 4 vertices"),
            PolygonError::NotRectilinear { edge } => {
                write!(f, "edge {edge} is not a horizontal or vertical segment")
            }
            PolygonError::SelfIntersecting { first, second } => {
                write!(f, "edges {first} and {second} cross or touch")
            }
            PolygonError::NonPositiveArea => f.write_str("the polygon has no area"),
        }
    }
}

impl std::error::Error for PolygonError {}

/// Why a space, slab or ceiling outline cannot be found.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SpaceError {
    Geometry(GeometryError),
    Polygon(PolygonError),
    UnknownEntity {
        id: Uuid,
    },
    /// The seed lies inside a wall.
    SeedInWall {
        space: Uuid,
    },
    /// The walls do not enclose the seed.
    Open {
        space: Uuid,
    },
    /// Walls stand free inside the room (Q-25).
    Island {
        space: Uuid,
        walls: Vec<Uuid>,
    },
    /// The region touches itself at a corner, so it is not one simple polygon.
    NotSimple {
        space: Uuid,
    },
    /// A boundary from spaces lists none (DEC-5).
    NoSpaces,
    /// A boundary from spaces lists spaces on different storeys (DEC-5).
    StoreyMismatch {
        space: Uuid,
    },
}

impl fmt::Display for SpaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpaceError::Geometry(e) => write!(f, "{e}"),
            SpaceError::Polygon(e) => write!(f, "{e}"),
            SpaceError::UnknownEntity { id } => write!(f, "no entity {id}"),
            SpaceError::SeedInWall { space } => {
                write!(f, "the seed of space {space} is inside a wall")
            }
            SpaceError::Open { space } => write!(f, "the walls do not enclose space {space}"),
            SpaceError::Island { space, walls } => {
                write!(
                    f,
                    "space {space} contains {} free-standing wall(s)",
                    walls.len()
                )
            }
            SpaceError::NotSimple { space } => {
                write!(f, "space {space} touches itself at a corner")
            }
            SpaceError::NoSpaces => f.write_str("the boundary lists no spaces"),
            SpaceError::StoreyMismatch { space } => {
                write!(
                    f,
                    "space {space} is on another storey than the first listed"
                )
            }
        }
    }
}

impl std::error::Error for SpaceError {}

impl From<GeometryError> for SpaceError {
    fn from(e: GeometryError) -> Self {
        SpaceError::Geometry(e)
    }
}

impl From<PolygonError> for SpaceError {
    fn from(e: PolygonError) -> Self {
        SpaceError::Polygon(e)
    }
}

/// Twice the signed area, positive when counterclockwise.
fn twice_signed_area(points: &[Point]) -> i128 {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            i128::from(a.x.nanometers()) * i128::from(b.y.nanometers())
                - i128::from(b.x.nanometers()) * i128::from(a.y.nanometers())
        })
        .sum()
}

/// The exact area enclosed, in square nanometers (orientation ignored).
pub fn polygon_area(points: &[Point]) -> i128 {
    twice_signed_area(points).abs() / 2
}

type Xy = (i128, i128);

fn xy(p: &Point) -> Xy {
    (i128::from(p.x.nanometers()), i128::from(p.y.nanometers()))
}

fn point(v: Xy) -> Result<Point, GeometryError> {
    let c = |n: i128| {
        i64::try_from(n)
            .map(Length::from_nanometers)
            .map_err(|_| GeometryError::Overflow)
    };
    Ok(Point {
        x: c(v.0)?,
        y: c(v.1)?,
    })
}

/// Removes collinear vertices, orients counterclockwise and starts at the
/// lowest x, then lowest y.
fn canonical(mut v: Vec<Xy>) -> Vec<Xy> {
    let mut changed = true;
    while changed && v.len() > 2 {
        changed = false;
        let n = v.len();
        for i in 0..n {
            let (a, b, c) = (v[(i + n - 1) % n], v[i], v[(i + 1) % n]);
            if (a.0 == b.0 && b.0 == c.0) || (a.1 == b.1 && b.1 == c.1) {
                v.remove(i);
                changed = true;
                break;
            }
        }
    }
    let n = v.len();
    let twice: i128 = (0..n)
        .map(|i| {
            let (a, b) = (v[i], v[(i + 1) % n]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum();
    if twice < 0 {
        v.reverse();
    }
    if let Some(start) = (0..v.len()).min_by_key(|&i| v[i]) {
        v.rotate_left(start);
    }
    v
}

/// Whether two closed axis-aligned segments share any point.
fn touch(a: (Xy, Xy), b: (Xy, Xy)) -> bool {
    let (ax0, ax1) = (a.0.0.min(a.1.0), a.0.0.max(a.1.0));
    let (ay0, ay1) = (a.0.1.min(a.1.1), a.0.1.max(a.1.1));
    let (bx0, bx1) = (b.0.0.min(b.1.0), b.0.0.max(b.1.0));
    let (by0, by1) = (b.0.1.min(b.1.1), b.0.1.max(b.1.1));
    ax0.max(bx0) <= ax1.min(bx1) && ay0.max(by0) <= ay1.min(by1)
}

/// Checks a simple rectilinear polygon and returns it in canonical form.
pub fn check_polygon(points: &[Point]) -> Result<Vec<Point>, PolygonError> {
    let v: Vec<Xy> = points.iter().map(xy).collect();
    let n = v.len();
    if n < 4 {
        return Err(PolygonError::TooFewVertices);
    }
    let edge = |i: usize| (v[i], v[(i + 1) % n]);
    for i in 0..n {
        let (a, b) = edge(i);
        if a == b || (a.0 != b.0 && a.1 != b.1) {
            return Err(PolygonError::NotRectilinear { edge: i });
        }
    }
    for i in 0..n {
        let j = (i + 1) % n;
        let ((a, b), (_, c)) = (edge(i), edge(j));
        // Consecutive edges along one line in opposite directions overlap.
        let same_line = (a.0 == b.0 && b.0 == c.0) || (a.1 == b.1 && b.1 == c.1);
        let backward = (b.0 - a.0) * (c.0 - b.0) + (b.1 - a.1) * (c.1 - b.1) < 0;
        if same_line && backward {
            return Err(PolygonError::SelfIntersecting {
                first: i,
                second: j,
            });
        }
    }
    for i in 0..n {
        for j in i + 2..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            if touch(edge(i), edge(j)) {
                return Err(PolygonError::SelfIntersecting {
                    first: i,
                    second: j,
                });
            }
        }
    }
    if twice_signed_area(points) == 0 {
        return Err(PolygonError::NonPositiveArea);
    }
    canonical(v)
        .into_iter()
        .map(|p| point(p).map_err(|_| PolygonError::NonPositiveArea))
        .collect()
}

/// The storey's wall footprints as a grid: cut at every footprint's
/// bounding-rectangle edge, with the walls covering each cell.
struct Grid {
    xs: Vec<i128>,
    ys: Vec<i128>,
    /// Per wall: id and whether it runs along X.
    walls: Vec<(Uuid, bool)>,
    /// Indices into `walls` covering each cell, row by row.
    owners: Vec<Vec<usize>>,
}

impl Grid {
    fn new(document: &Document, storey: Uuid) -> Result<Grid, GeometryError> {
        let geometry = wall_geometry(document)?;
        let mut rects: Vec<(Uuid, bool, Xy, Xy)> = Vec::new();
        for (wall_id, g) in &geometry {
            let Some(wall) = document.walls.get(wall_id).filter(|w| w.storey == storey) else {
                continue;
            };
            let pts: Vec<Xy> = g.footprint.iter().map(xy).collect();
            let (Some(x0), Some(x1)) =
                (pts.iter().map(|p| p.0).min(), pts.iter().map(|p| p.0).max())
            else {
                continue;
            };
            let (Some(y0), Some(y1)) =
                (pts.iter().map(|p| p.1).min(), pts.iter().map(|p| p.1).max())
            else {
                continue;
            };
            let along_x = matches!(wall.direction, Direction::PosX | Direction::NegX);
            rects.push((*wall_id, along_x, (x0, y0), (x1, y1)));
        }
        let mut xs: Vec<i128> = rects.iter().flat_map(|r| [r.2.0, r.3.0]).collect();
        let mut ys: Vec<i128> = rects.iter().flat_map(|r| [r.2.1, r.3.1]).collect();
        xs.sort_unstable();
        xs.dedup();
        ys.sort_unstable();
        ys.dedup();
        let (nx, ny) = (xs.len().saturating_sub(1), ys.len().saturating_sub(1));
        let index = |axis: &[i128], v: i128| axis.binary_search(&v).unwrap_or(0);
        let mut owners: Vec<Vec<usize>> = vec![Vec::new(); nx * ny];
        for (k, r) in rects.iter().enumerate() {
            for j in index(&ys, r.2.1)..index(&ys, r.3.1) {
                for i in index(&xs, r.2.0)..index(&xs, r.3.0) {
                    owners[j * nx + i].push(k);
                }
            }
        }
        Ok(Grid {
            walls: rects.iter().map(|r| (r.0, r.1)).collect(),
            xs,
            ys,
            owners,
        })
    }

    fn nx(&self) -> usize {
        self.xs.len().saturating_sub(1)
    }

    fn ny(&self) -> usize {
        self.ys.len().saturating_sub(1)
    }

    fn cell(&self, i: usize, j: usize) -> usize {
        j * self.nx() + i
    }

    fn covered(&self, i: usize, j: usize) -> bool {
        !self.owners[self.cell(i, j)].is_empty()
    }

    fn on_edge(&self, i: usize, j: usize) -> bool {
        i == 0 || j == 0 || i + 1 == self.nx() || j + 1 == self.ny()
    }

    fn neighbours(&self, i: usize, j: usize) -> Vec<(usize, usize)> {
        let mut out = Vec::with_capacity(4);
        if i > 0 {
            out.push((i - 1, j));
        }
        if i + 1 < self.nx() {
            out.push((i + 1, j));
        }
        if j > 0 {
            out.push((i, j - 1));
        }
        if j + 1 < self.ny() {
            out.push((i, j + 1));
        }
        out
    }

    /// The cells of space `id`: the region around its seed that the walls
    /// enclose, with no free-standing wall inside (Q-24, Q-25).
    fn room(&self, id: Uuid, seed: &Point) -> Result<Vec<bool>, SpaceError> {
        let open = SpaceError::Open { space: id };
        let (nx, ny) = (self.nx(), self.ny());
        if nx == 0 || ny == 0 {
            return Err(open);
        }
        // The seed's cell, half-open on its upper edges.
        let (sx, sy) = xy(seed);
        let find = |axis: &[i128], v: i128| {
            let k = axis.partition_point(|&a| a <= v);
            (k >= 1 && k < axis.len()).then(|| k - 1)
        };
        let (Some(si), Some(sj)) = (find(&self.xs, sx), find(&self.ys, sy)) else {
            return Err(open);
        };
        if self.covered(si, sj) {
            return Err(SpaceError::SeedInWall { space: id });
        }
        // Flood fill the room.
        let mut filled = vec![false; nx * ny];
        let mut stack = vec![(si, sj)];
        filled[self.cell(si, sj)] = true;
        while let Some((i, j)) = stack.pop() {
            if self.on_edge(i, j) {
                return Err(open);
            }
            for (a, b) in self.neighbours(i, j) {
                if !self.covered(a, b) && !filled[self.cell(a, b)] {
                    filled[self.cell(a, b)] = true;
                    stack.push((a, b));
                }
            }
        }
        // Islands: wall regions surrounded by the room on every side (Q-25).
        let mut seen = vec![false; nx * ny];
        let mut island_walls = std::collections::BTreeSet::new();
        for j in 0..ny {
            for i in 0..nx {
                if !self.covered(i, j) || seen[self.cell(i, j)] {
                    continue;
                }
                let mut component = vec![(i, j)];
                let mut enclosed = true;
                seen[self.cell(i, j)] = true;
                let mut k = 0;
                while k < component.len() {
                    let (a, b) = component[k];
                    k += 1;
                    if self.on_edge(a, b) {
                        enclosed = false;
                    }
                    for (c, d) in self.neighbours(a, b) {
                        if self.covered(c, d) {
                            if !seen[self.cell(c, d)] {
                                seen[self.cell(c, d)] = true;
                                component.push((c, d));
                            }
                        } else if !filled[self.cell(c, d)] {
                            enclosed = false;
                        }
                    }
                }
                if enclosed {
                    for (a, b) in component {
                        island_walls.extend(
                            self.owners[self.cell(a, b)]
                                .iter()
                                .map(|&w| self.walls[w].0),
                        );
                    }
                }
            }
        }
        if !island_walls.is_empty() {
            return Err(SpaceError::Island {
                space: id,
                walls: island_walls.into_iter().collect(),
            });
        }
        Ok(filled)
    }

    /// Adds the walls bounding the room cells `cells`, out to their far
    /// faces (DEC-5): from each room cell straight across the cells of walls
    /// that run perpendicular to the step, then the corner cells that have
    /// such grown cells beside them on both axes.
    fn grow(&self, cells: &mut [bool]) {
        let (nx, ny) = (self.nx(), self.ny());
        let room = cells.to_vec();
        // A wall running along X is crossed by a step along Y, and the reverse.
        let crossed = |i: usize, j: usize, step_x: bool| {
            self.owners[self.cell(i, j)]
                .iter()
                .any(|&w| self.walls[w].1 != step_x)
        };
        let step = |i: usize, j: usize, d: usize| -> Option<(usize, usize)> {
            match d {
                0 => (i + 1 < nx).then_some((i + 1, j)),
                1 => i.checked_sub(1).map(|i| (i, j)),
                2 => (j + 1 < ny).then_some((i, j + 1)),
                _ => j.checked_sub(1).map(|j| (i, j)),
            }
        };
        for j in 0..ny {
            for i in 0..nx {
                if !room[self.cell(i, j)] {
                    continue;
                }
                for d in 0..4 {
                    let mut at = step(i, j, d);
                    while let Some((a, b)) = at {
                        if !self.covered(a, b) || !crossed(a, b, d < 2) {
                            break;
                        }
                        cells[self.cell(a, b)] = true;
                        at = step(a, b, d);
                    }
                }
            }
        }
        // Corners, until none is added (a T junction's corner spans two cells).
        loop {
            let grown = cells.to_vec();
            let is_grown = |at: Option<(usize, usize)>| {
                at.is_some_and(|(a, b)| grown[self.cell(a, b)] && !room[self.cell(a, b)])
            };
            let mut added = false;
            for j in 0..ny {
                for i in 0..nx {
                    if grown[self.cell(i, j)] || !self.covered(i, j) {
                        continue;
                    }
                    let x_side = is_grown(step(i, j, 0)) || is_grown(step(i, j, 1));
                    let y_side = is_grown(step(i, j, 2)) || is_grown(step(i, j, 3));
                    if x_side && y_side {
                        cells[self.cell(i, j)] = true;
                        added = true;
                    }
                }
            }
            if !added {
                break;
            }
        }
    }

    /// Traces `cells` as one counterclockwise polygon; `id` names the
    /// space in errors.
    fn trace(&self, cells: &[bool], id: Uuid) -> Result<Vec<Point>, SpaceError> {
        let (nx, ny) = (self.nx(), self.ny());
        let (xs, ys) = (&self.xs, &self.ys);
        // Every cell side between the region and the rest, directed with
        // the region on its left.
        let mut next: std::collections::BTreeMap<Xy, Vec<Xy>> = std::collections::BTreeMap::new();
        let inside = |i: Option<usize>, j: Option<usize>| match (i, j) {
            (Some(i), Some(j)) if i < nx && j < ny => cells[self.cell(i, j)],
            _ => false,
        };
        for j in 0..ny {
            for i in 0..nx {
                if !cells[self.cell(i, j)] {
                    continue;
                }
                let (x0, x1, y0, y1) = (xs[i], xs[i + 1], ys[j], ys[j + 1]);
                let sides = [
                    (inside(Some(i), j.checked_sub(1)), (x0, y0), (x1, y0)),
                    (inside(Some(i + 1), Some(j)), (x1, y0), (x1, y1)),
                    (inside(Some(i), Some(j + 1)), (x1, y1), (x0, y1)),
                    (inside(i.checked_sub(1), Some(j)), (x0, y1), (x0, y0)),
                ];
                for (neighbour_inside, a, b) in sides {
                    if !neighbour_inside {
                        next.entry(a).or_default().push(b);
                    }
                }
            }
        }
        if next.values().any(|v| v.len() != 1) {
            return Err(SpaceError::NotSimple { space: id });
        }
        let Some(&start) = next.keys().next() else {
            return Err(SpaceError::Open { space: id });
        };
        let mut outline = vec![start];
        let mut at = start;
        loop {
            let Some(&to) = next.get(&at).and_then(|v| v.first()) else {
                return Err(SpaceError::NotSimple { space: id });
            };
            if to == start {
                break;
            }
            outline.push(to);
            at = to;
            if outline.len() > next.len() {
                return Err(SpaceError::NotSimple { space: id });
            }
        }
        if outline.len() != next.len() {
            return Err(SpaceError::NotSimple { space: id });
        }
        canonical(outline)
            .into_iter()
            .map(|p| point(p).map_err(SpaceError::Geometry))
            .collect()
    }
}

/// The derived boundary of space `id` (Q-24).
pub fn space_boundary(document: &Document, id: Uuid) -> Result<Vec<Point>, SpaceError> {
    let space = document
        .spaces
        .get(&id)
        .ok_or(SpaceError::UnknownEntity { id })?;
    let grid = Grid::new(document, space.storey)?;
    let cells = grid.room(id, &space.seed)?;
    grid.trace(&cells, id)
}

/// The union of `spaces` and the walls bounding them, out to each bounding
/// wall's far face (DEC-5). The spaces must share a storey; order and
/// repeats do not matter.
pub fn spaces_outline(document: &Document, spaces: &[Uuid]) -> Result<Vec<Point>, SpaceError> {
    let mut ids: Vec<Uuid> = spaces.to_vec();
    ids.sort();
    ids.dedup();
    let Some(&first) = ids.first() else {
        return Err(SpaceError::NoSpaces);
    };
    for id in &ids {
        if !document.spaces.contains_key(id) {
            return Err(SpaceError::UnknownEntity { id: *id });
        }
    }
    let storey = document
        .spaces
        .get(&first)
        .map(|s| s.storey)
        .ok_or(SpaceError::UnknownEntity { id: first })?;
    let grid = Grid::new(document, storey)?;
    let mut cells = vec![false; grid.nx() * grid.ny()];
    for id in &ids {
        let space = document
            .spaces
            .get(id)
            .ok_or(SpaceError::UnknownEntity { id: *id })?;
        if space.storey != storey {
            return Err(SpaceError::StoreyMismatch { space: *id });
        }
        for (c, inside) in cells.iter_mut().zip(grid.room(*id, &space.seed)?) {
            *c |= inside;
        }
    }
    grid.grow(&mut cells);
    grid.trace(&cells, first)
}

/// Stores every space's derived boundary (Q-7).
pub fn update_spaces(document: &mut Document) -> Result<(), SpaceError> {
    let ids: Vec<Uuid> = document.spaces.keys().copied().collect();
    for id in ids {
        let boundary = space_boundary(document, id)?;
        if let Some(space) = document.spaces.get_mut(&id) {
            space.boundary = boundary;
        }
    }
    Ok(())
}

/// The outline of a slab, ceiling or roof footprint.
pub fn boundary_outline(
    document: &Document,
    boundary: &Boundary,
) -> Result<Vec<Point>, SpaceError> {
    match boundary {
        Boundary::Explicit { points } => Ok(check_polygon(points)?),
        Boundary::FromSpace { space } => space_boundary(document, *space),
        Boundary::FromSpaces { spaces } => spaces_outline(document, spaces),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::*;
    use crate::identity::LabelAllocator;
    use crate::length::Length;
    use crate::units::ProjectUnit;
    use proptest::prelude::*;

    const IN: i64 = 25_400_000;
    const SQIN: i128 = (IN as i128) * (IN as i128);

    fn len(nm: i64) -> Length {
        Length::from_nanometers(nm)
    }

    fn p(x: i64, y: i64) -> Point {
        Point {
            x: len(x * IN),
            y: len(y * IN),
        }
    }

    fn uid(kind: u8, n: u8) -> Uuid {
        let mut b = [0u8; 16];
        b[0] = kind;
        b[15] = n;
        Uuid::from_bytes(b)
    }

    const STOREY: Uuid = Uuid::from_bytes([8; 16]);
    const SPACE: Uuid = Uuid::from_bytes([11; 16]);

    fn sourced(nm: i64) -> Sourced<Length> {
        Sourced {
            value: len(nm),
            source: Source::UserStated,
        }
    }

    /// Walls 4 in thick on their reference line, joining the given corner
    /// points (inches) in order and closing the loop; a space seeded at `seed`.
    fn room(corners: &[(i64, i64)], seed: (i64, i64)) -> Document {
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
        let layers = uid(5, 0);
        doc.layer_sets.insert(
            layers,
            LayerSet {
                name: "4 in".into(),
                layers: vec![Layer {
                    material: uid(6, 0),
                    thickness: sourced(4 * IN),
                    core: true,
                }],
            },
        );
        let mut labels = LabelAllocator::new();
        for (i, &(x, y)) in corners.iter().enumerate() {
            let n = u8::try_from(i).unwrap_or(0);
            doc.nodes.insert(
                uid(1, n),
                Node {
                    x: len(x * IN),
                    y: len(y * IN),
                    join: JoinType::Isolated,
                },
            );
        }
        for i in 0..corners.len() {
            let j = (i + 1) % corners.len();
            let (a, b) = (corners[i], corners[j]);
            let direction = match ((b.0 - a.0).signum(), (b.1 - a.1).signum()) {
                (1, _) => Direction::PosX,
                (-1, _) => Direction::NegX,
                (_, 1) => Direction::PosY,
                _ => Direction::NegY,
            };
            add_wall(&mut doc, &mut labels, i, j, direction);
        }
        doc.spaces.insert(
            SPACE,
            Space {
                storey: STOREY,
                name: "Room".into(),
                seed: p(seed.0, seed.1),
                boundary: vec![],
                finishes: Finishes {
                    floor: None,
                    walls: None,
                    ceiling: None,
                },
            },
        );
        doc
    }

    fn add_wall(
        doc: &mut Document,
        labels: &mut LabelAllocator,
        a: usize,
        b: usize,
        direction: Direction,
    ) {
        let n = |i: usize| u8::try_from(i).unwrap_or(0);
        doc.walls.insert(
            uid(2, n(a)),
            Wall {
                label: labels.allocate("W").unwrap_or_else(|_| unreachable!()),
                storey: STOREY,
                node_a: uid(1, n(a)),
                node_b: uid(1, n(b)),
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

    #[test]
    fn rectangular_room() -> Result<(), SpaceError> {
        // 20 ft x 12 ft on wall centerlines; 4 in walls, so the room is 236 x 140 in.
        let doc = room(&[(0, 0), (240, 0), (240, 144), (0, 144)], (120, 72));
        let b = space_boundary(&doc, SPACE)?;
        assert_eq!(b, vec![p(2, 2), p(238, 2), p(238, 142), p(2, 142)]);
        assert_eq!(polygon_area(&b), 236 * 140 * SQIN);
        // The same room traced clockwise (walls reversed) gives the same space.
        let cw = room(&[(0, 0), (0, 144), (240, 144), (240, 0)], (120, 72));
        assert_eq!(space_boundary(&cw, SPACE)?, b);
        Ok(())
    }

    #[test]
    fn l_shaped_room() -> Result<(), SpaceError> {
        let doc = room(
            &[(0, 0), (240, 0), (240, 96), (120, 96), (120, 192), (0, 192)],
            (60, 150),
        );
        let b = space_boundary(&doc, SPACE)?;
        assert_eq!(
            b,
            vec![
                p(2, 2),
                p(238, 2),
                p(238, 94),
                p(118, 94),
                p(118, 190),
                p(2, 190)
            ]
        );
        // 236 x 92 below the step, plus 116 x 96 above it.
        assert_eq!(polygon_area(&b), (236 * 92 + 116 * 96) * SQIN);
        Ok(())
    }

    #[test]
    fn update_spaces_stores_boundaries() -> Result<(), SpaceError> {
        let mut doc = room(&[(0, 0), (240, 0), (240, 144), (0, 144)], (10, 10));
        update_spaces(&mut doc)?;
        assert_eq!(doc.spaces.get(&SPACE).map(|s| s.boundary.len()), Some(4));
        Ok(())
    }

    #[test]
    fn bad_seeds_and_open_rooms() {
        // Seed inside the south wall.
        let doc = room(&[(0, 0), (240, 0), (240, 144), (0, 144)], (120, 1));
        assert_eq!(
            space_boundary(&doc, SPACE),
            Err(SpaceError::SeedInWall { space: SPACE })
        );
        // Seed outside the building.
        let doc = room(&[(0, 0), (240, 0), (240, 144), (0, 144)], (300, 72));
        assert_eq!(
            space_boundary(&doc, SPACE),
            Err(SpaceError::Open { space: SPACE })
        );
        // A missing wall leaves the room open.
        let mut doc = room(&[(0, 0), (240, 0), (240, 144), (0, 144)], (120, 72));
        let first = doc.walls.keys().next().copied();
        if let Some(id) = first {
            doc.walls.remove(&id);
        }
        assert_eq!(
            space_boundary(&doc, SPACE),
            Err(SpaceError::Open { space: SPACE })
        );
        assert_eq!(
            space_boundary(&doc, uid(11, 9)),
            Err(SpaceError::UnknownEntity { id: uid(11, 9) })
        );
    }

    #[test]
    fn a_free_standing_wall_is_an_island() {
        let mut doc = room(&[(0, 0), (240, 0), (240, 144), (0, 144)], (20, 20));
        doc.nodes.insert(
            uid(1, 20),
            Node {
                x: len(100 * IN),
                y: len(60 * IN),
                join: JoinType::Isolated,
            },
        );
        doc.nodes.insert(
            uid(1, 21),
            Node {
                x: len(150 * IN),
                y: len(60 * IN),
                join: JoinType::Isolated,
            },
        );
        let mut labels = LabelAllocator::new();
        let _ = labels.allocate("W");
        add_wall(&mut doc, &mut labels, 20, 21, Direction::PosX);
        let island = uid(2, 20);
        assert_eq!(
            space_boundary(&doc, SPACE),
            Err(SpaceError::Island {
                space: SPACE,
                walls: vec![island]
            })
        );
    }

    #[test]
    fn walls_on_other_storeys_do_not_bound_the_space() {
        let mut doc = room(&[(0, 0), (240, 0), (240, 144), (0, 144)], (120, 72));
        let other = uid(8, 1);
        if let Some(w) = doc.walls.values_mut().next() {
            w.storey = other;
        }
        assert_eq!(
            space_boundary(&doc, SPACE),
            Err(SpaceError::Open { space: SPACE })
        );
    }

    fn pts(v: &[(i64, i64)]) -> Vec<Point> {
        v.iter().map(|&(x, y)| p(x, y)).collect()
    }

    #[test]
    fn explicit_polygons_are_checked_and_normalized() {
        // Clockwise, starting elsewhere, with a collinear vertex: normalized.
        let cw = pts(&[(10, 5), (10, 0), (0, 0), (0, 5), (5, 5)]);
        assert_eq!(
            check_polygon(&cw),
            Ok(pts(&[(0, 0), (10, 0), (10, 5), (0, 5)]))
        );
        let l = pts(&[(0, 0), (6, 0), (6, 2), (2, 2), (2, 6), (0, 6)]);
        assert_eq!(check_polygon(&l), Ok(l.clone()));
        assert_eq!(polygon_area(&l), (6 * 2 + 2 * 4) * SQIN);
    }

    #[test]
    fn self_intersecting_input_is_rejected() {
        let bad = |v: &[(i64, i64)]| check_polygon(&pts(v));
        // A figure eight: two squares crossing at one point.
        assert!(matches!(
            bad(&[
                (0, 0),
                (2, 0),
                (2, 2),
                (4, 2),
                (4, 4),
                (2, 4),
                (2, 2),
                (0, 2)
            ]),
            Err(PolygonError::SelfIntersecting { .. })
        ));
        // Edges crossing: a rectilinear bow tie.
        assert!(matches!(
            bad(&[(0, 0), (4, 0), (4, 4), (2, 4), (2, -2), (0, -2)]),
            Err(PolygonError::SelfIntersecting { .. })
        ));
        // A vertex touching another edge.
        assert!(matches!(
            bad(&[
                (0, 0),
                (4, 0),
                (4, 2),
                (2, 2),
                (2, 0),
                (1, 0),
                (1, 4),
                (0, 4)
            ]),
            Err(PolygonError::SelfIntersecting { .. })
        ));
        // A spike that doubles back along itself.
        assert!(matches!(
            bad(&[(0, 0), (4, 0), (6, 0), (4, 0), (4, 4), (0, 4)]),
            Err(PolygonError::SelfIntersecting { .. })
        ));
        assert_eq!(
            bad(&[(0, 0), (4, 0), (4, 4), (1, 3)]),
            Err(PolygonError::NotRectilinear { edge: 2 })
        );
        assert_eq!(
            bad(&[(0, 0), (4, 0), (4, 4)]),
            Err(PolygonError::TooFewVertices)
        );
        assert_eq!(
            bad(&[(0, 0), (4, 0), (4, 0), (0, 0)]),
            Err(PolygonError::NotRectilinear { edge: 1 })
        );
    }

    #[test]
    fn slabs_and_ceilings_take_their_outline() -> Result<(), SpaceError> {
        let doc = room(&[(0, 0), (240, 0), (240, 144), (0, 144)], (120, 72));
        let from_space = boundary_outline(&doc, &Boundary::FromSpace { space: SPACE })?;
        assert_eq!(from_space, space_boundary(&doc, SPACE)?);
        let explicit = Boundary::Explicit {
            points: pts(&[(0, 5), (0, 0), (9, 0), (9, 5)]),
        };
        assert_eq!(
            boundary_outline(&doc, &explicit)?,
            pts(&[(0, 0), (9, 0), (9, 5), (0, 5)])
        );
        let crossing = Boundary::Explicit {
            points: pts(&[(0, 0), (4, 0), (4, 4), (2, 4), (2, -2), (0, -2)]),
        };
        assert!(matches!(
            boundary_outline(&doc, &crossing),
            Err(SpaceError::Polygon(PolygonError::SelfIntersecting { .. }))
        ));
        assert_eq!(
            boundary_outline(&doc, &Boundary::FromSpace { space: uid(11, 7) }),
            Err(SpaceError::UnknownEntity { id: uid(11, 7) })
        );
        Ok(())
    }

    fn from_spaces(spaces: &[Uuid]) -> Boundary {
        Boundary::FromSpaces {
            spaces: spaces.to_vec(),
        }
    }

    /// Two rooms side by side: a 200 x 80 in rectangle on centerlines split
    /// by an interior wall at x = 100 that butts the outer walls (T joins).
    fn two_rooms() -> Document {
        let mut doc = room(
            &[(0, 0), (100, 0), (200, 0), (200, 80), (100, 80), (0, 80)],
            (50, 40),
        );
        if let Some(mut w) = doc.walls.get(&uid(2, 0)).cloned() {
            w.node_a = uid(1, 1);
            w.node_b = uid(1, 4);
            w.direction = Direction::PosY;
            w.join_priority = 1;
            doc.walls.insert(uid(2, 6), w);
        }
        doc
    }

    /// DEC-5: a slab from one space reaches its walls' exterior faces.
    #[test]
    fn from_spaces_reaches_the_walls_far_faces() -> Result<(), SpaceError> {
        let doc = room(&[(0, 0), (240, 0), (240, 144), (0, 144)], (120, 72));
        assert_eq!(
            boundary_outline(&doc, &from_spaces(&[SPACE]))?,
            vec![p(-2, -2), p(242, -2), p(242, 146), p(-2, 146)]
        );
        // An L: the corners at the step are filled too.
        let l = room(
            &[(0, 0), (240, 0), (240, 96), (120, 96), (120, 192), (0, 192)],
            (60, 150),
        );
        assert_eq!(
            boundary_outline(&l, &from_spaces(&[SPACE]))?,
            vec![
                p(-2, -2),
                p(242, -2),
                p(242, 98),
                p(122, 98),
                p(122, 194),
                p(-2, 194)
            ]
        );
        Ok(())
    }

    /// DEC-5: listed spaces join across the walls between them; an unlisted
    /// neighbour's shared wall is included out to its far face only.
    #[test]
    fn from_spaces_joins_rooms_across_interior_walls() -> Result<(), SpaceError> {
        let mut doc = two_rooms();
        let east = uid(11, 1);
        let mut second = doc
            .spaces
            .get(&SPACE)
            .cloned()
            .ok_or(SpaceError::Open { space: SPACE })?;
        second.seed = p(150, 40);
        doc.spaces.insert(east, second);
        assert_eq!(
            space_boundary(&doc, SPACE)?,
            vec![p(2, 2), p(98, 2), p(98, 78), p(2, 78)]
        );
        assert_eq!(
            boundary_outline(&doc, &from_spaces(&[SPACE, east]))?,
            vec![p(-2, -2), p(202, -2), p(202, 82), p(-2, 82)]
        );
        // The west room alone: out to the interior wall's east face at 102.
        assert_eq!(
            boundary_outline(&doc, &from_spaces(&[SPACE]))?,
            vec![p(-2, -2), p(102, -2), p(102, 82), p(-2, 82)]
        );
        // Order and repeats do not matter.
        assert_eq!(
            boundary_outline(&doc, &from_spaces(&[east, SPACE, east]))?,
            boundary_outline(&doc, &from_spaces(&[SPACE, east]))?
        );
        Ok(())
    }

    #[test]
    fn from_spaces_errors() {
        let doc = room(&[(0, 0), (240, 0), (240, 144), (0, 144)], (120, 72));
        assert_eq!(
            boundary_outline(&doc, &from_spaces(&[])),
            Err(SpaceError::NoSpaces)
        );
        assert_eq!(
            boundary_outline(&doc, &from_spaces(&[SPACE, uid(11, 7)])),
            Err(SpaceError::UnknownEntity { id: uid(11, 7) })
        );
        let mut other = doc.clone();
        if let Some(s) = other.spaces.get_mut(&SPACE) {
            s.seed = p(500, 500);
        }
        assert_eq!(
            boundary_outline(&other, &from_spaces(&[SPACE])),
            Err(SpaceError::Open { space: SPACE })
        );
    }

    proptest! {
        /// A rectangle given from any vertex, in either direction, is the
        /// same canonical polygon with the exact area.
        #[test]
        fn rectangles_normalize(x in -1000i64..1000, y in -1000i64..1000, w in 1i64..500, h in 1i64..500, start in 0usize..4, reverse in any::<bool>()) {
            let mut v = vec![(x, y), (x + w, y), (x + w, y + h), (x, y + h)];
            if reverse {
                v.reverse();
            }
            v.rotate_left(start);
            let got = check_polygon(&pts(&v));
            prop_assert_eq!(got.clone(), Ok(pts(&[(x, y), (x + w, y), (x + w, y + h), (x, y + h)])));
            prop_assert_eq!(got.map(|g| polygon_area(&g)), Ok(i128::from(w) * i128::from(h) * SQIN));
        }

        /// A room built from four walls has exactly the interior area.
        #[test]
        fn rectangular_rooms_have_exact_area(w in 10i64..600, h in 10i64..600) {
            let doc = room(&[(0, 0), (w, 0), (w, h), (0, h)], (w / 2, h / 2));
            let b = space_boundary(&doc, SPACE);
            prop_assert_eq!(b.map(|b| polygon_area(&b)), Ok(i128::from(w - 4) * i128::from(h - 4) * SQIN));
        }
    }
}
