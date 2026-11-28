//! Where the plan draws each Distance constraint (T-204d, spec 5.3, 6.2, 7.1;
//! Q-81 (b)(i), Q-82).
//!
//! Every Distance in the document is drawn as an architectural dimension:
//! extension lines from its two references, a dimension line along its axis
//! and its value in feet-inches-fractions (with ~ off the 1/16 in grid). The
//! placement is computed here from the document alone and stored nowhere.
//! A dimension sits outside the plan's bounding box on the side nearer its
//! references: an X dimension above or below, a Y dimension right or left.
//! "Nearer" compares the midpoint of the references' extents across the
//! axis with the center of the box; a tie, or references with no extent
//! (datum lines), go below or left (Q-82). On each side the dimensions are
//! stacked outward one per row, ordered by value, then by name (the echo's
//! words), then by id. Rows are a count; the plan turns them into pixels.

use archviz_core::{
    Axis, ConstraintKind, ConstraintStatus, DatumGeometry, Document, Reference, Uuid,
    describe_constraint, format_length, reference_coordinate,
};

use crate::scene::Scene;
use crate::viewport::World;

/// Which side of the plan's bounding box a dimension is drawn on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side {
    Below,
    Above,
    Left,
    Right,
}

/// One Distance constraint laid out on the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub constraint: Uuid,
    /// The axis the distance is measured along.
    pub axis: Axis,
    pub side: Side,
    /// 0 is the row nearest the bounding box.
    pub row: usize,
    /// The coordinates on `axis` of references `a` and `b`.
    pub ends: [i64; 2],
    /// Where each extension line starts, across `axis`: the end of the
    /// reference nearest `side`; None for a datum line (it starts at the box).
    pub feet: [Option<i64>; 2],
    /// The value shown: a driving constraint's value, or in parentheses a
    /// reference constraint's measured distance (7.2).
    pub text: String,
    pub status: ConstraintStatus,
    /// The constraint in the echo's words.
    pub name: String,
}

fn on(axis: Axis, p: World) -> i64 {
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

fn extent(values: impl Iterator<Item = i64> + Clone) -> Option<(i64, i64)> {
    Some((values.clone().min()?, values.max()?))
}

/// The extent of `r` on axis `perp`, read from the scene and the document;
/// None for a datum line, which has no end.
fn span(doc: &Document, scene: &Scene, r: &Reference, perp: Axis) -> Option<(i64, i64)> {
    let rect = |r: &(World, World)| {
        let (a, b) = (on(perp, r.0), on(perp, r.1));
        Some((a.min(b), a.max(b)))
    };
    match *r {
        Reference::WallFace { wall, .. } | Reference::WallCenter { wall } => {
            let w = scene.walls.iter().find(|w| w.id == wall)?;
            extent(w.footprint.iter().map(|p| on(perp, *p)))
        }
        Reference::Node { node } => {
            let n = doc.nodes.get(&node)?;
            let v = on(perp, (n.x.nanometers(), n.y.nanometers()));
            Some((v, v))
        }
        Reference::OpeningEdge { opening, .. } => {
            rect(&scene.openings.iter().find(|o| o.id == opening)?.rect)
        }
        Reference::ItemEdge { item, .. } | Reference::ItemCenter { item } => {
            rect(&scene.occurrences.iter().find(|o| o.id == item)?.rect)
        }
        Reference::Datum { datum } => match doc.datums.get(&datum)?.geometry {
            DatumGeometry::Point { x, y } => {
                let v = on(perp, (x.nanometers(), y.nanometers()));
                Some((v, v))
            }
            DatumGeometry::XLine { .. } | DatumGeometry::YLine { .. } => None,
        },
    }
}

/// Lays out every Distance constraint of `doc`; `scene` is built from the
/// same document. A constraint whose references cannot be located is left
/// out. Sorted by side, then row.
pub fn layout(doc: &Document, scene: &Scene) -> Vec<Placed> {
    let Some((min, max)) = scene.bounds() else {
        return Vec::new();
    };
    // (side, sort key, placed) before rows are numbered.
    let mut items: Vec<(Side, (i64, String, Uuid), Placed)> = Vec::new();
    for (id, c) in &doc.constraints {
        let ConstraintKind::Distance {
            a, b, axis, value, ..
        } = &c.kind
        else {
            continue;
        };
        let (Ok(ca), Ok(cb)) = (
            reference_coordinate(doc, a, *axis),
            reference_coordinate(doc, b, *axis),
        ) else {
            continue;
        };
        let perp = across(*axis);
        let spans = [span(doc, scene, a, perp), span(doc, scene, b, perp)];
        let merged = spans
            .iter()
            .flatten()
            .fold(None, |acc: Option<(i64, i64)>, &(lo, hi)| {
                Some(acc.map_or((lo, hi), |(l, h)| (l.min(lo), h.max(hi))))
            });
        // Compared doubled, so nothing is divided.
        let center = i128::from(on(perp, min)) + i128::from(on(perp, max));
        let high = merged.is_some_and(|(lo, hi)| i128::from(lo) + i128::from(hi) > center);
        let side = match (axis, high) {
            (Axis::X, false) => Side::Below,
            (Axis::X, true) => Side::Above,
            (Axis::Y, false) => Side::Left,
            (Axis::Y, true) => Side::Right,
        };
        let foot = |s: Option<(i64, i64)>| s.map(|(lo, hi)| if high { hi } else { lo });
        let (ca, cb) = (ca.nanometers(), cb.nanometers());
        let text = match c.status {
            ConstraintStatus::Driving => format_length(*value),
            ConstraintStatus::Reference => {
                let measured = i64::try_from(i128::from(cb) - i128::from(ca)).unwrap_or(i64::MAX);
                format!(
                    "({})",
                    format_length(archviz_core::Length::from_nanometers(measured))
                )
            }
        };
        let name = describe_constraint(doc, &c.kind);
        items.push((
            side,
            (value.nanometers(), name.clone(), *id),
            Placed {
                constraint: *id,
                axis: *axis,
                side,
                row: 0,
                ends: [ca, cb],
                feet: [foot(spans[0]), foot(spans[1])],
                text,
                status: c.status,
                name,
            },
        ));
    }
    items.sort_by(|x, y| (x.0, &x.1).cmp(&(y.0, &y.1)));
    let mut out: Vec<Placed> = Vec::with_capacity(items.len());
    for (side, _, mut placed) in items {
        placed.row = out.iter().filter(|p| p.side == side).count();
        out.push(placed);
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::session::dimension_tests::t203_room;
    use archviz_core::{Constraint, Length, Source};

    const IN: i64 = 25_400_000;

    /// A layout without ids: what two documents built the same way share.
    fn shape(l: &[Placed]) -> Vec<Placed> {
        l.iter()
            .map(|p| Placed {
                constraint: Uuid::from_bytes([0; 16]),
                ..p.clone()
            })
            .collect()
    }

    fn room(tag: &str) -> (Document, Vec<Placed>) {
        let (_, doc) = t203_room(tag);
        let scene = Scene::build(&doc).unwrap_or_else(|e| panic!("{e}"));
        let placed = scene.dimensions.clone();
        assert_eq!(placed, layout(&doc, &scene), "the scene holds the layout");
        (doc, placed)
    }

    /// Done when: the same document always gives the same layout. The
    /// layout of the T-203 room is computed twice, from the document read
    /// back from its canonical JSON, and from a second room built by the
    /// same steps (other ids, other insertion order of constraints).
    #[test]
    fn the_same_document_always_gives_the_same_layout() {
        let (doc, placed) = room("layout-a");
        assert_eq!(placed.len(), 4);
        let again = Scene::build(&doc).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(layout(&doc, &again), placed);
        let json = archviz_core::to_canonical_json(&doc).unwrap_or_else(|e| panic!("{e}"));
        let back: Document = serde_json::from_str(&json).unwrap_or_else(|e| panic!("{e}"));
        let scene = Scene::build(&back).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(layout(&back, &scene), placed);
        let (other, other_placed) = room("layout-b");
        assert_ne!(
            doc.constraints.keys().collect::<Vec<_>>(),
            other.constraints.keys().collect::<Vec<_>>(),
            "the second room has its own ids"
        );
        assert_eq!(shape(&other_placed), shape(&placed));
    }

    /// The T-203 room: the two zero dimensions from the origin datum lie
    /// below (X, from the west wall) and left (Y, from the south wall); the
    /// 14'-6 1/2" one is below, stacked outside the 0'-0" one, and the
    /// 11'-3" one left of the 0'-0" one. Each value as typed.
    #[test]
    fn the_t203_room_places_its_four_dimensions_outside_the_plan() {
        let (doc, placed) = room("layout-room");
        let scene = Scene::build(&doc).unwrap_or_else(|e| panic!("{e}"));
        let ((x0, y0), (x1, y1)) = scene.bounds().unwrap_or_else(|| panic!("bounds"));
        let got: Vec<(Axis, Side, usize, &str)> = placed
            .iter()
            .map(|p| (p.axis, p.side, p.row, p.text.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                (Axis::X, Side::Below, 0, "0'-0\""),
                (Axis::X, Side::Below, 1, "14'-6 1/2\""),
                (Axis::Y, Side::Left, 0, "0'-0\""),
                (Axis::Y, Side::Left, 1, "11'-3\""),
            ]
        );
        for p in &placed {
            assert_eq!(p.status, ConstraintStatus::Driving);
            let (lo, hi) = match p.axis {
                Axis::X => (x0, x1),
                Axis::Y => (y0, y1),
            };
            for e in p.ends {
                assert!(lo <= e && e <= hi, "{} ends inside the box", p.name);
            }
            for f in p.feet.iter().flatten() {
                let base = match p.side {
                    Side::Below => y0,
                    Side::Left => x0,
                    Side::Above => y1,
                    Side::Right => x1,
                };
                assert!(
                    matches!(p.side, Side::Below | Side::Left) && *f >= base
                        || matches!(p.side, Side::Above | Side::Right) && *f <= base,
                    "{} starts inside the box",
                    p.name
                );
            }
        }
        let wide = placed.iter().find(|p| p.text == "14'-6 1/2\"");
        assert_eq!(
            wide.map(|p| p.ends[1] - p.ends[0]),
            Some((14 * 12 + 6) * IN + IN / 2)
        );
    }

    /// Rows go by value then name; a reference constraint shows its measured
    /// value in parentheses; a constraint nearer the top goes above; an off
    /// grid value shows the ~.
    #[test]
    fn rows_sides_and_reference_constraints() {
        let (mut doc, _) = room("layout-rows");
        let scene = Scene::build(&doc).unwrap_or_else(|e| panic!("{e}"));
        let (south, north) = {
            let mut ws: Vec<_> = scene.walls.iter().filter(|w| w.across == Axis::Y).collect();
            ws.sort_by_key(|w| w.left.min(w.right));
            (ws[0].id, ws[1].id)
        };
        let (west, east) = {
            let mut ws: Vec<_> = scene.walls.iter().filter(|w| w.across == Axis::X).collect();
            ws.sort_by_key(|w| w.left.min(w.right));
            (ws[0].id, ws[1].id)
        };
        let center = |wall| Reference::WallCenter { wall };
        let add = |doc: &mut Document, n: u8, a, b, axis, value: i64, status| {
            doc.constraints.insert(
                Uuid::from_bytes([n; 16]),
                Constraint {
                    kind: ConstraintKind::Distance {
                        a,
                        b,
                        axis,
                        value: Length::from_nanometers(value),
                        tolerance: None,
                    },
                    status,
                    source: Source::UserStated,
                    provenance: None,
                },
            );
        };
        // West to east center along X, a reference constraint, and north
        // to south center along Y, driving and off the grid.
        add(
            &mut doc,
            0xF1,
            center(west),
            center(east),
            Axis::X,
            IN + 1,
            ConstraintStatus::Reference,
        );
        add(
            &mut doc,
            0xF2,
            center(north),
            center(south),
            Axis::Y,
            IN + 1,
            ConstraintStatus::Driving,
        );
        let scene = Scene::build(&doc).unwrap_or_else(|e| panic!("{e}"));
        let l = layout(&doc, &scene);
        let f1 = l
            .iter()
            .find(|p| p.constraint == Uuid::from_bytes([0xF1; 16]))
            .unwrap_or_else(|| panic!());
        // Both walls span the whole height: a tie goes below; its value
        // (1 in + 1 nm) sorts after 0'-0" and before 14'-6 1/2".
        assert_eq!((f1.side, f1.row), (Side::Below, 1));
        assert!(
            f1.text.starts_with('(') && f1.text.ends_with(')'),
            "{}",
            f1.text
        );
        let measured = f1.ends[1] - f1.ends[0];
        assert_eq!(
            f1.text,
            format!("({})", format_length(Length::from_nanometers(measured)))
        );
        let f2 = l
            .iter()
            .find(|p| p.constraint == Uuid::from_bytes([0xF2; 16]))
            .unwrap_or_else(|| panic!());
        assert_eq!((f2.side, f2.row), (Side::Left, 1));
        assert!(f2.text.starts_with('~'), "{}", f2.text);
        // Rows on each side are 0, 1, 2, ... in value order.
        for side in [Side::Below, Side::Left] {
            let on_side: Vec<&Placed> = l.iter().filter(|p| p.side == side).collect();
            for (i, p) in on_side.iter().enumerate() {
                assert_eq!(p.row, i);
            }
        }
        // A dimension between two point datums at the top goes above.
        let top = scene
            .walls
            .iter()
            .find(|w| w.id == north)
            .map(|w| w.left.max(w.right))
            .unwrap_or_default();
        let point = |doc: &mut Document, n: u8, x: i64| {
            let id = Uuid::from_bytes([n; 16]);
            doc.datums.insert(
                id,
                archviz_core::Datum {
                    name: format!("High {n}"),
                    geometry: DatumGeometry::Point {
                        x: Length::from_nanometers(x),
                        y: Length::from_nanometers(top),
                    },
                },
            );
            Reference::Datum { datum: id }
        };
        let (d1, d2) = (point(&mut doc, 0xF3, 0), point(&mut doc, 0xF5, 60 * IN));
        add(
            &mut doc,
            0xF4,
            d1,
            d2,
            Axis::X,
            60 * IN,
            ConstraintStatus::Driving,
        );
        let scene = Scene::build(&doc).unwrap_or_else(|e| panic!("{e}"));
        let l = layout(&doc, &scene);
        let f4 = l
            .iter()
            .find(|p| p.constraint == Uuid::from_bytes([0xF4; 16]))
            .unwrap_or_else(|| panic!());
        assert_eq!((f4.side, f4.row), (Side::Above, 0));
        assert_eq!(f4.feet[0], Some(top), "a point datum's foot is the point");
    }
}
