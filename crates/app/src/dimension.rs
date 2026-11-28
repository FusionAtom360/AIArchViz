//! The dimension tool (T-203, spec 5.2, 5.3, 6.2, 7.1, 7.5).
//!
//! The user picks two references in the plan (a wall face, a wall center or
//! a datum), types a value, sees it echoed in canonical feet-inches, and the
//! app proposes a driving Distance constraint with source UserStated through
//! the command API. The constraint's references and axis come from what was
//! picked; its sign follows the plan: `a` is the reference with the lower
//! coordinate on the axis, as core computes it, so the typed value is the
//! positive distance between them. Core solves; the app computes nothing.
//!
//! The plan highlights underconstrained walls (from the solver's report) and
//! the walls of a proposal's conflicting constraints, and names every
//! constraint in a conflict in the echo's words (7.5).

use std::collections::{BTreeMap, BTreeSet};

use archviz_core::{
    Action, AddConstraint, Axis, ConstraintKind, DatumGeometry, Document, InputContext, Length,
    ParseError, ProjectUnit, ProposedAction, Provenance, Reference, Source, Uuid, WallSide,
    describe_constraint, format_length, parse_length, reference_coordinate,
};

use crate::scene::{Scene, WallShape};
use crate::session::Pending;
use crate::viewport::World;

/// A reference the user picked in the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picked {
    pub reference: Reference,
    /// The one axis it has a coordinate on; None for a point datum (both).
    pub axis: Option<Axis>,
    /// How the plan names it.
    pub name: String,
}

/// Two references measured along one axis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    pub a: Picked,
    pub b: Picked,
    pub axis: Axis,
}

/// What a pick did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// The first reference is held; pick the second.
    First,
    /// Both are picked: type the value.
    Ready(Pair),
    /// The pick was refused, and why; the first pick, if any, is kept.
    Refused(&'static str),
}

/// The dimension being picked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DimensionTool {
    first: Option<Picked>,
}

fn on(axis: Axis, p: World) -> i64 {
    match axis {
        Axis::X => p.0,
        Axis::Y => p.1,
    }
}

fn distance(a: i64, b: i64) -> i128 {
    (i128::from(a) - i128::from(b)).abs()
}

/// The reference under `at`, within `tolerance` nanometers: a datum first,
/// then the wall face or center nearest the pointer across the wall.
pub fn pick(scene: &Scene, at: World, tolerance: i64) -> Option<Picked> {
    let t = i128::from(tolerance.max(0));
    for d in &scene.datums {
        let name = format!("datum \"{}\"", d.name);
        let reference = Reference::Datum { datum: d.id };
        let (near, axis) = match d.geometry {
            DatumGeometry::Point { x, y } => (
                distance(at.0, x.nanometers()) <= t && distance(at.1, y.nanometers()) <= t,
                None,
            ),
            DatumGeometry::XLine { x } => (distance(at.0, x.nanometers()) <= t, Some(Axis::X)),
            DatumGeometry::YLine { y } => (distance(at.1, y.nanometers()) <= t, Some(Axis::Y)),
        };
        if near {
            return Some(Picked {
                reference,
                axis,
                name,
            });
        }
    }
    let w = scene.wall_at(at, tolerance)?;
    let v = on(w.across, at);
    // The nearest; on a tie the first listed (a face before the center).
    let (_, reference, what) = wall_references(w)?
        .into_iter()
        .min_by_key(|(c, _, _)| distance(*c, v))?;
    Some(Picked {
        reference,
        axis: Some(w.across),
        name: format!("{} {what}", w.label),
    })
}

/// A wall's pickable references with their coordinates across it: the left
/// face, the right face and the center, in that order.
fn wall_references(w: &WallShape) -> Option<[(i64, Reference, &'static str); 3]> {
    let center = i64::try_from((i128::from(w.left) + i128::from(w.right)) / 2).ok()?;
    Some([
        (
            w.left,
            Reference::WallFace {
                wall: w.id,
                side: WallSide::Left,
                boundary: 0,
            },
            "left face",
        ),
        (
            w.right,
            Reference::WallFace {
                wall: w.id,
                side: WallSide::Right,
                boundary: 0,
            },
            "right face",
        ),
        (center, Reference::WallCenter { wall: w.id }, "center"),
    ])
}

/// Where a picked reference lies in the plan, for its highlight (T-204c).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// A wall face or center: the line across the wall at its coordinate,
    /// along the wall's length.
    Segment(World, World),
    /// A datum line at this X.
    XLine(i64),
    /// A datum line at this Y.
    YLine(i64),
    /// A point datum.
    Point(World),
}

/// The mark of `picked` in `scene`, read from the same coordinates `pick`
/// chooses between; None if the scene no longer holds it.
pub fn mark(scene: &Scene, picked: &Picked) -> Option<Mark> {
    match picked.reference {
        Reference::Datum { datum } => {
            let d = scene.datums.iter().find(|d| d.id == datum)?;
            Some(match d.geometry {
                DatumGeometry::Point { x, y } => Mark::Point((x.nanometers(), y.nanometers())),
                DatumGeometry::XLine { x } => Mark::XLine(x.nanometers()),
                DatumGeometry::YLine { y } => Mark::YLine(y.nanometers()),
            })
        }
        Reference::WallFace { wall, .. } | Reference::WallCenter { wall } => {
            let w = scene.walls.iter().find(|w| w.id == wall)?;
            let (c, _, _) = wall_references(w)?
                .into_iter()
                .find(|(_, r, _)| *r == picked.reference)?;
            let along = w.footprint.iter().map(|p| match w.across {
                Axis::X => p.1,
                Axis::Y => p.0,
            });
            let (lo, hi) = (along.clone().min()?, along.max()?);
            Some(match w.across {
                Axis::X => Mark::Segment((c, lo), (c, hi)),
                Axis::Y => Mark::Segment((lo, c), (hi, c)),
            })
        }
        _ => None,
    }
}

impl DimensionTool {
    /// The first reference, while the second is being picked.
    pub fn first(&self) -> Option<&Picked> {
        self.first.as_ref()
    }

    pub fn cancel(&mut self) {
        self.first = None;
    }

    /// Takes the next pick (`pick`'s result for a click).
    pub fn click(&mut self, picked: Option<Picked>) -> Pick {
        let Some(picked) = picked else {
            return Pick::Refused("no reference there: pick a wall face, wall center or datum");
        };
        let Some(first) = &self.first else {
            self.first = Some(picked);
            return Pick::First;
        };
        if first.reference == picked.reference {
            return Pick::Refused("pick a second, different reference");
        }
        let axis = match (first.axis, picked.axis) {
            (Some(x), Some(y)) if x != y => {
                return Pick::Refused("the two references lie on different axes");
            }
            (Some(x), _) | (None, Some(x)) => x,
            (None, None) => return Pick::Refused("two points: pick a wall for one of them"),
        };
        let a = first.clone();
        self.first = None;
        Pick::Ready(Pair { a, b: picked, axis })
    }
}

/// Parses a typed distance (5.2): bare numbers in the project unit, the
/// 1/16 in grid enforced with the nearest grid value offered (5.3), never
/// negative. The caller echoes the result with `format_length`.
pub fn parse_value(text: &str, unit: ProjectUnit) -> Result<Length, String> {
    match parse_length(text, InputContext::User, unit) {
        Ok(parsed) if parsed.length.nanometers() < 0 => {
            Err("a distance cannot be negative".to_string())
        }
        Ok(parsed) => Ok(parsed.length),
        Err(ParseError::OffGrid { nearest }) => Err(format!(
            "finer than 1/16 in; the nearest value is {}",
            format_length(nearest)
        )),
        Err(e) => Err(e.to_string()),
    }
}

/// The driving Distance between the picked references, with `a` the one at
/// the lower coordinate now so `value` is the distance between them (7.1,
/// Q-13): user-stated, or Measured with provenance Transcribed when read off
/// the underlay (6.6).
pub fn distance_action(
    doc: &Document,
    pair: &Pair,
    value: Length,
    transcribed: bool,
) -> Result<Action, String> {
    let at = |r: &Reference| reference_coordinate(doc, r, pair.axis).map_err(|e| format!("{e:?}"));
    let (a, b) = if at(&pair.b.reference)? < at(&pair.a.reference)? {
        (&pair.b, &pair.a)
    } else {
        (&pair.a, &pair.b)
    };
    Ok(Action::AddConstraint(AddConstraint {
        kind: ConstraintKind::Distance {
            a: a.reference,
            b: b.reference,
            axis: pair.axis,
            value,
            tolerance: None,
        },
        status: None,
        // Read off the paper plan: Measured, provenance Transcribed (6.6).
        source: if transcribed {
            Source::Measured
        } else {
            Source::UserStated
        },
        provenance: transcribed.then_some(Provenance::Transcribed),
    }))
}

fn wall_of(r: &Reference) -> Option<Uuid> {
    match *r {
        Reference::WallFace { wall, .. } | Reference::WallCenter { wall } => Some(wall),
        _ => None,
    }
}

fn ids(v: &serde_json::Value) -> Option<Uuid> {
    serde_json::from_value(v.clone()).ok()
}

/// Walls with a free coordinate in the solver's report (`solve` result,
/// 7.3): a wall listed itself, or a wall with a free end node.
pub fn underconstrained_walls(doc: &Document, solution: &serde_json::Value) -> BTreeSet<Uuid> {
    let mut walls = BTreeSet::new();
    let mut nodes = BTreeSet::new();
    let classes = solution["underconstrained"]
        .as_array()
        .into_iter()
        .flatten();
    for member in classes.flat_map(|c| c["members"].as_array().into_iter().flatten()) {
        if let Some(id) = ids(&member["wall"]) {
            walls.insert(id);
        }
        if let Some(id) = ids(&member["node"]) {
            nodes.insert(id);
        }
    }
    for (id, w) in &doc.walls {
        if nodes.contains(&w.node_a) || nodes.contains(&w.node_b) {
            walls.insert(*id);
        }
    }
    walls
}

/// The conflicts of a proposal, named, and the walls they touch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConflictReport {
    /// One line per conflict: the constraint, then each it disagrees with.
    pub lines: Vec<String>,
    pub walls: BTreeSet<Uuid>,
    /// Every constraint named, the proposed ones included; the plan draws
    /// those the document holds in red (T-204d).
    pub constraints: BTreeSet<Uuid>,
}

/// Names every constraint in `pending`'s conflicts (7.5): those already in
/// `doc`, and those the proposal adds (a command's constraint id derives
/// from its idempotency key, as core derives it).
pub fn conflicts(doc: &Document, pending: &Pending) -> ConflictReport {
    let mut kinds: BTreeMap<Uuid, ConstraintKind> = doc
        .constraints
        .iter()
        .map(|(id, c)| (*id, c.kind.clone()))
        .collect();
    if let Ok(ProposedAction::Commands { commands }) =
        serde_json::from_value::<ProposedAction>(pending.action.clone())
    {
        for c in commands {
            if let Action::AddConstraint(add) = c.action {
                kinds.insert(Uuid::derive(c.idempotency_key, "command", "0"), add.kind);
            }
        }
    }
    let mut report = ConflictReport::default();
    let mut name = |id: Uuid| match kinds.get(&id) {
        Some(kind) => {
            let refs = match kind {
                ConstraintKind::Distance { a, b, .. } | ConstraintKind::Align { a, b, .. } => {
                    vec![*a, *b]
                }
                ConstraintKind::Fixed { reference, .. } => vec![*reference],
                _ => Vec::new(),
            };
            report.walls.extend(refs.iter().filter_map(wall_of));
            let value = match kind {
                ConstraintKind::Distance { value, .. } | ConstraintKind::Fixed { value, .. } => {
                    format!(" ({})", format_length(*value))
                }
                _ => String::new(),
            };
            format!("{}{value}", describe_constraint(doc, kind))
        }
        None => "a constraint".to_string(),
    };
    // The solver reports whichever constraint it processes later (by id);
    // the line names the proposed ones first, so it reads the same each time.
    let mut lines = Vec::new();
    let mut named = BTreeSet::new();
    for c in pending.result["conflicts"].as_array().into_iter().flatten() {
        let Some(id) = ids(&c["constraint"]) else {
            continue;
        };
        let mut involved: Vec<Uuid> = vec![id];
        involved.extend(c["with"].as_array().into_iter().flatten().filter_map(ids));
        involved.sort_by_key(|id| (doc.constraints.contains_key(id), *id));
        named.extend(involved.iter().copied());
        let mut names = involved.into_iter().map(&mut name);
        let first = names.next().unwrap_or_default();
        let rest: Vec<String> = names.collect();
        lines.push(format!("{first} conflicts with {}", rest.join(" and ")));
    }
    report.lines = lines;
    report.constraints = named;
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{DatumShape, WallShape};
    use archviz_core::{Direction, Function, GRID_NM};

    const IN: i64 = 25_400_000;

    fn uid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    /// A +X wall from x 0 to 10 ft, left face at y 2 1/4 in, right at -2 1/4
    /// in; a north-south wall; a point datum at the origin and an X line.
    fn scene() -> Scene {
        let ft = 12 * IN;
        let h = 9 * IN / 4;
        Scene {
            walls: vec![
                WallShape {
                    id: uid(1),
                    label: "W-001".into(),
                    function: Function::New,
                    direction: Direction::PosX,
                    footprint: vec![(0, -h), (10 * ft, -h), (10 * ft, h), (0, h)],
                    across: Axis::Y,
                    left: h,
                    right: -h,
                },
                WallShape {
                    id: uid(2),
                    label: "W-002".into(),
                    function: Function::New,
                    direction: Direction::PosY,
                    footprint: vec![
                        (20 * ft - h, 0),
                        (20 * ft + h, 0),
                        (20 * ft + h, ft),
                        (20 * ft - h, ft),
                    ],
                    across: Axis::X,
                    left: 20 * ft - h,
                    right: 20 * ft + h,
                },
            ],
            datums: vec![
                DatumShape {
                    id: uid(3),
                    name: "Origin".into(),
                    geometry: DatumGeometry::Point {
                        x: Length::from_nanometers(-5 * ft),
                        y: Length::from_nanometers(-5 * ft),
                    },
                },
                DatumShape {
                    id: uid(4),
                    name: "Grid A".into(),
                    geometry: DatumGeometry::XLine {
                        x: Length::from_nanometers(30 * ft),
                    },
                },
            ],
            ..Scene::default()
        }
    }

    #[test]
    fn a_pick_takes_the_nearest_face_or_center() {
        let s = scene();
        let at = |y: i64| pick(&s, (5 * 12 * IN, y), IN).map(|p| p.name);
        assert_eq!(at(2 * IN), Some("W-001 left face".into()));
        assert_eq!(at(-2 * IN), Some("W-001 right face".into()));
        assert_eq!(at(IN / 4), Some("W-001 center".into()));
        assert_eq!(at(3 * IN), Some("W-001 left face".into()), "just outside");
        assert_eq!(at(5 * IN), None, "beyond the tolerance");
        let p = pick(&s, (5 * 12 * IN, -2 * IN), IN).unwrap_or_else(|| panic!());
        assert_eq!(
            p.reference,
            Reference::WallFace {
                wall: uid(1),
                side: WallSide::Right,
                boundary: 0
            }
        );
        assert_eq!(p.axis, Some(Axis::Y));
    }

    #[test]
    fn a_datum_is_picked_before_a_wall() {
        let s = scene();
        let p = pick(&s, (-60 * IN + 7, -60 * IN - 3), IN).unwrap_or_else(|| panic!());
        assert_eq!(p.reference, Reference::Datum { datum: uid(3) });
        assert_eq!(p.axis, None);
        assert_eq!(p.name, "datum \"Origin\"");
        let line = pick(&s, (360 * IN + 5, 1_000 * IN), IN).unwrap_or_else(|| panic!());
        assert_eq!(line.axis, Some(Axis::X));
    }

    #[test]
    fn the_mark_lies_on_the_picked_reference() {
        let s = scene();
        let ft = 12 * IN;
        let h = 9 * IN / 4;
        let mark_at = |p: World| {
            let picked = pick(&s, p, IN).unwrap_or_else(|| panic!("nothing at {p:?}"));
            (picked.name.clone(), mark(&s, &picked))
        };
        assert_eq!(
            mark_at((5 * ft, 2 * IN)),
            (
                "W-001 left face".into(),
                Some(Mark::Segment((0, h), (10 * ft, h)))
            )
        );
        assert_eq!(
            mark_at((5 * ft, -2 * IN)),
            (
                "W-001 right face".into(),
                Some(Mark::Segment((0, -h), (10 * ft, -h)))
            )
        );
        assert_eq!(
            mark_at((5 * ft, IN / 4)),
            (
                "W-001 center".into(),
                Some(Mark::Segment((0, 0), (10 * ft, 0)))
            )
        );
        assert_eq!(
            mark_at((20 * ft + 2 * IN, 6 * IN)),
            (
                "W-002 right face".into(),
                Some(Mark::Segment((20 * ft + h, 0), (20 * ft + h, ft)))
            )
        );
        assert_eq!(
            mark_at((-5 * ft, -5 * ft + 3)),
            (
                "datum \"Origin\"".into(),
                Some(Mark::Point((-5 * ft, -5 * ft)))
            )
        );
        assert_eq!(
            mark_at((30 * ft, 7)),
            ("datum \"Grid A\"".into(), Some(Mark::XLine(30 * ft)))
        );
        let gone = Picked {
            reference: Reference::WallCenter { wall: uid(9) },
            axis: Some(Axis::X),
            name: "W-009 center".into(),
        };
        assert_eq!(mark(&s, &gone), None, "a reference the scene lacks");
    }

    #[test]
    fn two_picks_make_a_pair_on_one_axis() {
        let s = scene();
        let datum = pick(&s, (-60 * IN, -60 * IN), IN);
        let face = pick(&s, (60 * IN, 2 * IN), IN);
        let east = pick(&s, (240 * IN + 2 * IN, 6 * IN), IN);
        let line = pick(&s, (360 * IN, 0), IN);
        let mut t = DimensionTool::default();
        assert_eq!(
            t.click(None),
            Pick::Refused("no reference there: pick a wall face, wall center or datum")
        );
        assert_eq!(t.click(datum.clone()), Pick::First);
        assert_eq!(
            t.click(datum.clone()),
            Pick::Refused("pick a second, different reference")
        );
        let Pick::Ready(pair) = t.click(face.clone()) else {
            panic!()
        };
        assert_eq!(pair.axis, Axis::Y, "the datum takes the wall's axis");
        assert!(t.first().is_none());
        assert_eq!(t.click(face), Pick::First);
        assert_eq!(
            t.click(east.clone()),
            Pick::Refused("the two references lie on different axes")
        );
        assert!(t.first().is_some(), "the first pick is kept");
        t.cancel();
        t.click(east);
        assert!(matches!(
            t.click(line),
            Pick::Ready(Pair { axis: Axis::X, .. })
        ));
        let mut u = DimensionTool::default();
        u.click(datum);
        let other_point = Some(Picked {
            reference: Reference::Datum { datum: uid(9) },
            axis: None,
            name: "datum \"B\"".into(),
        });
        assert_eq!(
            u.click(other_point),
            Pick::Refused("two points: pick a wall for one of them")
        );
    }

    #[test]
    fn the_parser_echoes_or_refuses() {
        let echo = |t: &str| parse_value(t, ProjectUnit::Feet).map(format_length);
        assert_eq!(echo("10'-4 1/2\""), Ok("10'-4 1/2\"".into()));
        assert_eq!(echo("10 ft 4.5 in"), Ok("10'-4 1/2\"".into()));
        assert_eq!(echo("124.5in"), Ok("10'-4 1/2\"".into()));
        assert_eq!(
            echo("12"),
            Ok("12'-0\"".into()),
            "bare numbers in the project unit"
        );
        assert_eq!(
            parse_value("12", ProjectUnit::Inches).map(format_length),
            Ok("1'-0\"".into())
        );
        assert_eq!(echo("0"), Ok("0'-0\"".into()));
        assert_eq!(
            echo("1/32\""),
            Err("finer than 1/16 in; the nearest value is 0'-0\"".into())
        );
        assert_eq!(
            echo("3.03in"),
            Err("finer than 1/16 in; the nearest value is 0'-3\"".into())
        );
        assert_eq!(echo("-2'"), Err("a distance cannot be negative".into()));
        assert!(echo("").is_err());
        assert!(echo("ten feet").is_err());
        assert!(echo("300mm").is_err(), "metric only in asset fields");
        for t in ["7 13/16\"", "1'", "99' 11 15/16\""] {
            let v = parse_value(t, ProjectUnit::Feet).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(v.nanometers() % GRID_NM, 0);
        }
    }

    #[test]
    fn underconstrained_walls_include_walls_with_a_free_end() {
        let doc = crate::interview::tests::four_walls();
        let mut ids = doc.walls.iter();
        let (w1, wall1) = ids
            .next()
            .map(|(k, v)| (*k, v.clone()))
            .unwrap_or_else(|| panic!());
        let (w2, _) = ids
            .next()
            .map(|(k, v)| (*k, v.clone()))
            .unwrap_or_else(|| panic!());
        let solution = serde_json::json!({
            "underconstrained": [
                {"axis": "x", "members": [{"wall": w2}]},
                {"axis": "y", "members": [{"node": wall1.node_a}]},
            ]
        });
        let free = underconstrained_walls(&doc, &solution);
        assert!(free.contains(&w1) && free.contains(&w2));
        let none = underconstrained_walls(&doc, &serde_json::json!({"underconstrained": []}));
        assert!(none.is_empty());
    }
}
