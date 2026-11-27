//! Occurrence geometry: frames, footprints, heights, clearance zones, and
//! the parametric box type (spec 6.3, 10.1; Q-30 to Q-33).
//!
//! An item's footprint is its declared width along its local X and depth
//! along its local Y. Its front faces local -Y, back +Y, left -X, right +X.
//! Rotation turns that frame counterclockwise in quarter turns. A floor or
//! ceiling item's x, y and its stored `center` are the footprint center; the
//! footprint runs from center - half(size) (half rounded half to even) for
//! the full size. A wall-mounted item has its back against the named face
//! of its wall and its front facing away, so its rotation follows the wall.
//! Elevation is the item's bottom above the storey elevation; a
//! ceiling-mounted item hangs with its top at the storey's default ceiling
//! height.

use crate::document::{
    Authority, DeclaredDimensions, Direction, Document, ElementType, Fidelity, ItemSide, Placement,
    Point, WallSide,
};
use crate::geometry::GeometryError;
use crate::identity::Uuid;
use crate::length::Length;

/// An axis-aligned rectangle in plan, `min` to `max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub min: Point,
    pub max: Point,
}

/// `n / 2` rounded half to even.
pub(crate) fn half(n: i128) -> i128 {
    let q = n / 2;
    let r = n % 2;
    if r != 0 && q % 2 != 0 { q + r } else { q }
}

/// The world direction a side of an item faces at `rotation` quarter turns.
pub(crate) fn side_direction(rotation: u8, side: ItemSide) -> Direction {
    use Direction::{NegX, NegY, PosX, PosY};
    let front = [NegY, PosX, PosY, NegX];
    let left = [NegX, NegY, PosX, PosY];
    let q = usize::from(rotation % 4);
    let flip = |d: Direction| match d {
        PosX => NegX,
        NegX => PosX,
        PosY => NegY,
        NegY => PosY,
    };
    match side {
        ItemSide::Front => front[q],
        ItemSide::Back => flip(front[q]),
        ItemSide::Left => left[q],
        ItemSide::Right => flip(left[q]),
    }
}

fn occurrence(
    document: &Document,
    id: Uuid,
) -> Result<&crate::document::Occurrence, GeometryError> {
    document
        .occurrences
        .get(&id)
        .ok_or(GeometryError::UnknownEntity { id })
}

/// The item's quarter turns: stored for floor items, 0 for ceiling items,
/// and facing away from the wall face for wall-mounted items.
pub(crate) fn rotation(document: &Document, id: Uuid) -> Result<u8, GeometryError> {
    match &occurrence(document, id)?.placement {
        Placement::Floor { rotation, .. } => Ok(rotation.turns()),
        Placement::CeilingMounted { .. } => Ok(0),
        Placement::WallMounted { wall, side, .. } => {
            let w = document
                .walls
                .get(wall)
                .ok_or(GeometryError::UnknownEntity { id: *wall })?;
            // The face's outward normal, which the item's front faces.
            let left = match w.direction {
                Direction::PosX => Direction::PosY,
                Direction::NegX => Direction::NegY,
                Direction::PosY => Direction::NegX,
                Direction::NegY => Direction::PosX,
            };
            let normal = match (side, left) {
                (WallSide::Left, d) => d,
                (WallSide::Right, Direction::PosX) => Direction::NegX,
                (WallSide::Right, Direction::NegX) => Direction::PosX,
                (WallSide::Right, Direction::PosY) => Direction::NegY,
                (WallSide::Right, Direction::NegY) => Direction::PosY,
            };
            Ok(match normal {
                Direction::NegY => 0,
                Direction::PosX => 1,
                Direction::PosY => 2,
                Direction::NegX => 3,
            })
        }
    }
}

fn element_type(document: &Document, id: Uuid) -> Result<&ElementType, GeometryError> {
    let o = occurrence(document, id)?;
    document
        .element_types
        .get(&o.element_type)
        .ok_or(GeometryError::UnknownEntity { id: o.element_type })
}

/// The item's size along world X and Y.
pub(crate) fn world_size(document: &Document, id: Uuid) -> Result<(i128, i128), GeometryError> {
    let d = &element_type(document, id)?.declared_dimensions;
    let (w, depth) = (
        i128::from(d.width.nanometers()),
        i128::from(d.depth.nanometers()),
    );
    Ok(if rotation(document, id)? % 2 == 0 {
        (w, depth)
    } else {
        (depth, w)
    })
}

fn to_length(v: i128) -> Result<Length, GeometryError> {
    i64::try_from(v)
        .map(Length::from_nanometers)
        .map_err(|_| GeometryError::Overflow)
}

fn rect(min: (i128, i128), max: (i128, i128)) -> Result<Rect, GeometryError> {
    Ok(Rect {
        min: Point {
            x: to_length(min.0)?,
            y: to_length(min.1)?,
        },
        max: Point {
            x: to_length(max.0)?,
            y: to_length(max.1)?,
        },
    })
}

/// The footprint of an item of world size `size` centered at `center`.
pub(crate) fn footprint_at(
    center: (i128, i128),
    size: (i128, i128),
) -> ((i128, i128), (i128, i128)) {
    let min = (center.0 - half(size.0), center.1 - half(size.1));
    (min, (min.0 + size.0, min.1 + size.1))
}

/// The item's footprint around its stored center.
pub fn footprint(document: &Document, id: Uuid) -> Result<Rect, GeometryError> {
    let c = occurrence(document, id)?.center;
    let center = (i128::from(c.x.nanometers()), i128::from(c.y.nanometers()));
    let (min, max) = footprint_at(center, world_size(document, id)?);
    rect(min, max)
}

/// The absolute bottom and top elevations of the item.
pub fn height_range(document: &Document, id: Uuid) -> Result<(Length, Length), GeometryError> {
    let o = occurrence(document, id)?;
    let storey = document
        .storeys
        .get(&o.storey)
        .ok_or(GeometryError::UnknownEntity { id: o.storey })?;
    let level = i128::from(storey.elevation.value.nanometers());
    let height = i128::from(
        element_type(document, id)?
            .declared_dimensions
            .height
            .nanometers(),
    );
    let bottom = match &o.placement {
        Placement::Floor { elevation, .. } | Placement::WallMounted { elevation, .. } => {
            level + i128::from(elevation.value.nanometers())
        }
        Placement::CeilingMounted { .. } => {
            level + i128::from(storey.default_ceiling_height.value.nanometers()) - height
        }
    };
    Ok((to_length(bottom)?, to_length(bottom + height)?))
}

/// The footprint grown by the clearance extents on each side (Q-32).
pub fn clearance_zone(document: &Document, id: Uuid) -> Result<Rect, GeometryError> {
    let o = occurrence(document, id)?;
    let f = footprint(document, id)?;
    let q = rotation(document, id)?;
    let n = |l: Length| i128::from(l.nanometers());
    let (mut min, mut max) = ((n(f.min.x), n(f.min.y)), (n(f.max.x), n(f.max.y)));
    for (side, extent) in [
        (ItemSide::Front, &o.clearance.front),
        (ItemSide::Back, &o.clearance.back),
        (ItemSide::Left, &o.clearance.left),
        (ItemSide::Right, &o.clearance.right),
    ] {
        let e = n(extent.value);
        match side_direction(q, side) {
            Direction::PosX => max.0 += e,
            Direction::NegX => min.0 -= e,
            Direction::PosY => max.1 += e,
            Direction::NegY => min.1 -= e,
        }
    }
    rect(min, max)
}

/// A parametric box element type at the Proxy tier (10.1, Q-33).
pub fn proxy_box(
    name: &str,
    width: Length,
    depth: Length,
    height: Length,
    authority: Authority,
) -> ElementType {
    ElementType {
        name: name.to_string(),
        declared_dimensions: DeclaredDimensions {
            width,
            depth,
            height,
            authority,
        },
        fidelity: Fidelity::Proxy,
        model: None,
        properties: Default::default(),
        source: "parametric box".to_string(),
        license: "n/a".to_string(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
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

    pub(crate) const STOREY: Uuid = Uuid::from_bytes([8; 16]);
    pub(crate) const BOX: Uuid = Uuid::from_bytes([13; 16]);

    pub(crate) fn sourced(nm: i64) -> Sourced<Length> {
        Sourced {
            value: len(nm),
            source: Source::UserStated,
        }
    }

    pub(crate) fn pt(x: i64, y: i64) -> Point {
        Point {
            x: len(x * IN),
            y: len(y * IN),
        }
    }

    /// A storey (elevation 12 in, ceiling 108 in above it), a 4 in layer set,
    /// a +X wall from (0,0) to (240,0) in, and a 48 x 24 x 30 in box type.
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
                elevation: sourced(12 * IN),
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
        for (n, x) in [(1, 0), (2, 240)] {
            doc.nodes.insert(
                uid(1, n),
                Node {
                    x: len(x * IN),
                    y: len(0),
                    join: JoinType::FreeEnd,
                },
            );
        }
        let mut labels = LabelAllocator::new();
        doc.walls.insert(
            uid(2, 1),
            Wall {
                label: labels.allocate("W").unwrap_or_else(|_| unreachable!()),
                storey: STOREY,
                node_a: uid(1, 1),
                node_b: uid(1, 2),
                direction: Direction::PosX,
                layer_set: uid(5, 0),
                left_extent: sourced(2 * IN),
                base_offset: sourced(0),
                top: WallTop::Storey,
                join_priority: 0,
                function: Function::New,
            },
        );
        doc.element_types.insert(
            BOX,
            proxy_box(
                "Bench",
                len(48 * IN),
                len(24 * IN),
                len(30 * IN),
                Authority::UserStatedExact,
            ),
        );
        doc
    }

    fn zero_clearance() -> Clearance {
        Clearance {
            front: sourced(0),
            back: sourced(0),
            left: sourced(0),
            right: sourced(0),
        }
    }

    /// Adds an occurrence with the box type, its center stored at `center` (in).
    pub(crate) fn put(doc: &mut Document, n: u8, placement: Placement, center: (i64, i64)) -> Uuid {
        let id = uid(14, n);
        let mut labels = LabelAllocator::new();
        doc.occurrences.insert(
            id,
            Occurrence {
                label: labels.allocate("EQ").unwrap_or_else(|_| unreachable!()),
                storey: STOREY,
                element_type: BOX,
                placement,
                clearance: zero_clearance(),
                fixed: false,
                overlap_allowed: false,
                light: None,
                center: pt(center.0, center.1),
            },
        );
        id
    }

    pub(crate) fn floor(x: i64, y: i64, turns: u8) -> Placement {
        Placement::Floor {
            x: sourced(x * IN),
            y: sourced(y * IN),
            rotation: QuarterTurns::new(turns).unwrap_or_else(|| unreachable!()),
            elevation: sourced(0),
        }
    }

    pub(crate) fn on_wall(side: WallSide, offset: i64, elevation: i64) -> Placement {
        Placement::WallMounted {
            wall: uid(2, 1),
            side,
            reference: Reference::Node { node: uid(1, 1) },
            offset: sourced(offset * IN),
            elevation: sourced(elevation * IN),
        }
    }

    fn rect(min: (i64, i64), max: (i64, i64)) -> Rect {
        Rect {
            min: pt(min.0, min.1),
            max: pt(max.0, max.1),
        }
    }

    #[test]
    fn sides_turn_with_rotation() {
        use Direction::*;
        let table = [
            (0, [NegY, PosY, NegX, PosX]),
            (1, [PosX, NegX, NegY, PosY]),
            (2, [PosY, NegY, PosX, NegX]),
            (3, [NegX, PosX, PosY, NegY]),
        ];
        for (q, expected) in table {
            let got = [
                ItemSide::Front,
                ItemSide::Back,
                ItemSide::Left,
                ItemSide::Right,
            ]
            .map(|s| side_direction(q, s));
            assert_eq!(got, expected, "rotation {q}");
        }
    }

    #[test]
    fn floor_footprints_rotate_about_the_center() -> Result<(), GeometryError> {
        let mut doc = base();
        let a = put(&mut doc, 1, floor(100, 60, 0), (100, 60));
        let b = put(&mut doc, 2, floor(100, 60, 1), (100, 60));
        assert_eq!(footprint(&doc, a)?, rect((76, 48), (124, 72)));
        assert_eq!(footprint(&doc, b)?, rect((88, 36), (112, 84)));
        assert_eq!(
            world_size(&doc, b)?,
            (i128::from(24 * IN), i128::from(48 * IN))
        );
        Ok(())
    }

    #[test]
    fn wall_mounted_items_face_away_from_their_face() -> Result<(), GeometryError> {
        let mut doc = base();
        // Left of a +X wall is +Y, so the front faces +Y: rotation 2.
        let left = put(&mut doc, 1, on_wall(WallSide::Left, 60, 48), (60, 14));
        let right = put(&mut doc, 2, on_wall(WallSide::Right, 60, 48), (60, -14));
        assert_eq!(rotation(&doc, left)?, 2);
        assert_eq!(rotation(&doc, right)?, 0);
        assert_eq!(footprint(&doc, left)?, rect((36, 2), (84, 26)));
        assert_eq!(footprint(&doc, right)?, rect((36, -26), (84, -2)));
        Ok(())
    }

    #[test]
    fn heights_by_placement() -> Result<(), GeometryError> {
        let mut doc = base();
        let f = put(&mut doc, 1, floor(100, 60, 0), (100, 60));
        let w = put(&mut doc, 2, on_wall(WallSide::Left, 60, 48), (60, 14));
        let c = put(
            &mut doc,
            3,
            Placement::CeilingMounted {
                x: sourced(100 * IN),
                y: sourced(60 * IN),
            },
            (100, 60),
        );
        let inches = |r: (Length, Length)| (r.0.nanometers() / IN, r.1.nanometers() / IN);
        assert_eq!(height_range(&doc, f).map(inches), Ok((12, 42)));
        assert_eq!(height_range(&doc, w).map(inches), Ok((60, 90)));
        assert_eq!(height_range(&doc, c).map(inches), Ok((90, 120)));
        assert_eq!(rotation(&doc, c), Ok(0));
        Ok(())
    }

    #[test]
    fn clearance_zones_grow_in_the_item_frame() -> Result<(), GeometryError> {
        let mut doc = base();
        let a = put(&mut doc, 1, floor(100, 60, 1), (100, 60));
        if let Some(o) = doc.occurrences.get_mut(&a) {
            o.clearance = Clearance {
                front: sourced(36 * IN),
                back: sourced(0),
                left: sourced(6 * IN),
                right: sourced(0),
            };
        }
        // Rotation 1: front faces +X, left faces -Y. Footprint (88,36)-(112,84).
        assert_eq!(clearance_zone(&doc, a)?, rect((88, 30), (148, 84)));
        Ok(())
    }

    #[test]
    fn proxy_box_is_a_proxy() {
        let t = proxy_box(
            "Column",
            len(12 * IN),
            len(12 * IN),
            len(120 * IN),
            Authority::MeasuredWithTolerance,
        );
        assert_eq!(t.name, "Column");
        assert_eq!(t.fidelity, Fidelity::Proxy);
        assert_eq!(t.model, None);
        assert_eq!(t.source, "parametric box");
        assert_eq!(t.license, "n/a");
        assert_eq!(
            t.declared_dimensions,
            DeclaredDimensions {
                width: len(12 * IN),
                depth: len(12 * IN),
                height: len(120 * IN),
                authority: Authority::MeasuredWithTolerance
            }
        );
    }

    #[test]
    fn unknown_entities_are_errors() {
        let doc = base();
        assert_eq!(
            footprint(&doc, uid(14, 9)),
            Err(GeometryError::UnknownEntity { id: uid(14, 9) })
        );
    }
}
