//! The shed roof (spec 6.5; Q-44, Q-45).
//!
//! A roof is one inclined plane over a rectilinear footprint. Heights are
//! the underside (the bearing surface) above the storey elevation, measured
//! at the footprint (the bearing lines); the thickness sits above the
//! underside, and overhangs extend the same plane beyond the footprint
//! (Q-68, superseding Q-44 and Q-55 on these points). The high edge is the
//! footprint side facing up-slope, or the ledger face when there is one, and
//! the run is the horizontal distance from it to the footprint's low side
//! along the slope direction. With a pitch, rise = run x rise / run rounded
//! half to even to the nanometer (5.1): the low edge height is derived from
//! the high edge, or, when the high edge is not given (stored as Derived),
//! the high edge from a stated low edge.
//!
//! A ledger must be a face of an existing wall; the roof slopes down away
//! from it, so its slope direction is the face's outward normal (Q-45). A
//! ledger edge is not an eave, so it takes no overhang (Q-68).
//!
//! The underside at a coordinate s on the slope axis is high - d x rise /
//! run, d the distance from the high edge down the slope, in i128 and
//! rounded half to even to the nanometer (DEC-6).

use std::fmt;

use crate::document::{
    Axis, Direction, Document, Function, Point, Reference, Source, Sourced, WallSide,
};
use crate::identity::Uuid;
use crate::length::Length;
use crate::solver::{SolveError, axis_graph, resolve_reference};
use crate::space::{SpaceError, boundary_outline};

/// A roof's solved shape, along its slope axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoofGeometry {
    /// The direction the roof slopes down.
    pub slope_direction: Direction,
    /// Coordinates of the high edge and the low side on the slope axis.
    pub high_edge: Length,
    pub low_edge: Length,
    /// Horizontal distance from the high edge to the low side.
    pub run: Length,
    pub high_height: Length,
    /// Derived from the pitch, or as stated; None when neither is given.
    pub low_height: Option<Length>,
}

/// Why a roof is not well defined.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoofProblem {
    /// The ledger is not a face of an existing wall (Q-45).
    LedgerNotExistingWallFace,
    /// The footprint's outline cannot be found.
    Footprint(SpaceError),
    /// The high edge is not before the low side along the slope.
    NoRun,
    /// A pitch with a zero run.
    ZeroPitchRun,
    /// A stated low edge height differs from the one the pitch gives.
    PitchDisagrees {
        stated: Length,
        derived: Length,
    },
    /// The ledger face could not be solved.
    Solve(SolveError),
    /// Neither a low edge height nor a pitch is given.
    NoLowHeight,
    Overflow,
}

impl fmt::Display for RoofProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RoofProblem::LedgerNotExistingWallFace => {
                f.write_str("the ledger must be a face of an existing wall")
            }
            RoofProblem::Footprint(e) => write!(f, "roof footprint: {e}"),
            RoofProblem::NoRun => f.write_str("the roof has no run along its slope"),
            RoofProblem::ZeroPitchRun => f.write_str("the pitch has a zero run"),
            RoofProblem::PitchDisagrees { .. } => {
                f.write_str("the stated low edge height does not match the pitch")
            }
            RoofProblem::Solve(e) => write!(f, "{e}"),
            RoofProblem::NoLowHeight => {
                f.write_str("the roof has neither a low edge height nor a pitch")
            }
            RoofProblem::Overflow => f.write_str("a roof dimension is out of range"),
        }
    }
}

impl std::error::Error for RoofProblem {}

/// The solved shape of roof `id`.
pub fn roof_geometry(document: &Document, id: Uuid) -> Result<RoofGeometry, RoofProblem> {
    let roof = document
        .roofs
        .get(&id)
        .ok_or(RoofProblem::Footprint(SpaceError::UnknownEntity { id }))?;
    let (slope_direction, ledger_at) = match &roof.ledger {
        None => (roof.slope_direction.value, None),
        Some(ledger) => {
            let Reference::WallFace { wall, side, .. } = ledger else {
                return Err(RoofProblem::LedgerNotExistingWallFace);
            };
            let w = document
                .walls
                .get(wall)
                .filter(|w| w.function == Function::Existing)
                .ok_or(RoofProblem::LedgerNotExistingWallFace)?;
            // The face's outward normal: the roof slopes down away from it.
            let left = match w.direction {
                Direction::PosX => Direction::PosY,
                Direction::NegX => Direction::NegY,
                Direction::PosY => Direction::NegX,
                Direction::NegY => Direction::PosX,
            };
            let normal = match side {
                WallSide::Left => left,
                WallSide::Right => opposite(left),
            };
            let axis = axis_of(normal);
            let (var, constant) =
                resolve_reference(document, ledger, axis).map_err(RoofProblem::Solve)?;
            let graph = axis_graph(document, axis).map_err(RoofProblem::Solve)?;
            let at = graph.values.get(var).copied().unwrap_or(0) + constant;
            (normal, Some(at))
        }
    };
    let outline = boundary_outline(document, &roof.footprint).map_err(RoofProblem::Footprint)?;
    let axis = axis_of(slope_direction);
    let coords: Vec<i128> = outline
        .iter()
        .map(|p| {
            i128::from(match axis {
                Axis::X => p.x.nanometers(),
                Axis::Y => p.y.nanometers(),
            })
        })
        .collect();
    let (Some(&min), Some(&max)) = (coords.iter().min(), coords.iter().max()) else {
        return Err(RoofProblem::NoRun);
    };
    let sign: i128 = match slope_direction {
        Direction::PosX | Direction::PosY => 1,
        Direction::NegX | Direction::NegY => -1,
    };
    let (up_side, low_side) = if sign > 0 { (min, max) } else { (max, min) };
    let high = ledger_at.unwrap_or(up_side);
    let run = (low_side - high) * sign;
    if run <= 0 {
        return Err(RoofProblem::NoRun);
    }
    let mut high_height = roof.high_edge_height.value;
    let low_height = match &roof.pitch {
        Some(pitch) => {
            if pitch.value.run == 0 {
                return Err(RoofProblem::ZeroPitchRun);
            }
            let rise = round_half_even(
                run * i128::from(pitch.value.rise),
                i128::from(pitch.value.run),
            );
            let stated_low = roof
                .low_edge_height
                .as_ref()
                .filter(|l| l.source != Source::Derived);
            match stated_low {
                // The high edge is not given: derive it (6.5, Q-68).
                Some(low) if roof.high_edge_height.source == Source::Derived => {
                    high_height = to_length(i128::from(low.value.nanometers()) + rise)?;
                    Some(low.value)
                }
                _ => {
                    let derived = to_length(i128::from(high_height.nanometers()) - rise)?;
                    if let Some(stated) = stated_low
                        && pitch.source != Source::Derived
                        && stated.value != derived
                    {
                        return Err(RoofProblem::PitchDisagrees {
                            stated: stated.value,
                            derived,
                        });
                    }
                    Some(derived)
                }
            }
        }
        None => roof.low_edge_height.as_ref().map(|l| l.value),
    };
    Ok(RoofGeometry {
        slope_direction,
        high_edge: to_length(high)?,
        low_edge: to_length(low_side)?,
        run: to_length(run)?,
        high_height,
        low_height,
    })
}

impl RoofGeometry {
    /// The underside above the storey elevation at coordinate `s` on the
    /// slope axis, anywhere on the plane (DEC-6).
    pub fn underside(&self, s: Length) -> Result<Length, RoofProblem> {
        let low = self.low_height.ok_or(RoofProblem::NoLowHeight)?;
        let sign: i128 = match self.slope_direction {
            Direction::PosX | Direction::PosY => 1,
            Direction::NegX | Direction::NegY => -1,
        };
        let down = (i128::from(s.nanometers()) - i128::from(self.high_edge.nanometers())) * sign;
        let rise = i128::from(self.high_height.nanometers()) - i128::from(low.nanometers());
        let run = i128::from(self.run.nanometers());
        if run <= 0 {
            return Err(RoofProblem::NoRun);
        }
        to_length(i128::from(self.high_height.nanometers()) - round_half_even(down * rise, run))
    }

    /// The axis the roof slopes along.
    pub fn axis(&self) -> Axis {
        axis_of(self.slope_direction)
    }
}

/// Roof `id`'s footprint grown by its overhangs, counterclockwise from its
/// lowest-x, lowest-y vertex. A ledger edge takes no overhang (Q-68).
pub fn roof_outline(document: &Document, id: Uuid) -> Result<Vec<Point>, RoofProblem> {
    let g = roof_geometry(document, id)?;
    let roof = document
        .roofs
        .get(&id)
        .ok_or(RoofProblem::Footprint(SpaceError::UnknownEntity { id }))?;
    let outline = boundary_outline(document, &roof.footprint).map_err(RoofProblem::Footprint)?;
    let plan: Vec<(i128, i128)> = outline
        .iter()
        .map(|p| (i128::from(p.x.nanometers()), i128::from(p.y.nanometers())))
        .collect();
    // The side facing up-slope is the ledger edge when there is a ledger.
    let ledger_side = roof.ledger.as_ref().map(|_| opposite(g.slope_direction));
    let overhang = |side: Direction| {
        if Some(side) == ledger_side {
            return 0;
        }
        let o = &roof.overhang;
        i128::from(
            match side {
                Direction::PosX => &o.pos_x,
                Direction::NegX => &o.neg_x,
                Direction::PosY => &o.pos_y,
                Direction::NegY => &o.neg_y,
            }
            .value
            .nanometers(),
        )
    };
    // Each edge of the counterclockwise outline faces right of its
    // direction and moves out by that side's overhang.
    let n = plan.len();
    let offsets: Vec<(bool, i128)> = (0..n)
        .map(|i| {
            let (p, q) = (plan[i], plan[(i + 1) % n]);
            if p.1 == q.1 {
                if q.0 > p.0 {
                    (true, p.1 - overhang(Direction::NegY))
                } else {
                    (true, p.1 + overhang(Direction::PosY))
                }
            } else if q.1 > p.1 {
                (false, p.0 + overhang(Direction::PosX))
            } else {
                (false, p.0 - overhang(Direction::NegX))
            }
        })
        .collect();
    let grown: Vec<Point> = (0..n)
        .map(|i| {
            let (mut x, mut y) = plan[i];
            for (horizontal, v) in [offsets[(i + n - 1) % n], offsets[i]] {
                if horizontal {
                    y = v;
                } else {
                    x = v;
                }
            }
            Ok(Point {
                x: to_length(x)?,
                y: to_length(y)?,
            })
        })
        .collect::<Result<_, RoofProblem>>()?;
    crate::space::check_polygon(&grown).map_err(|e| RoofProblem::Footprint(SpaceError::Polygon(e)))
}

fn opposite(d: Direction) -> Direction {
    match d {
        Direction::PosX => Direction::NegX,
        Direction::NegX => Direction::PosX,
        Direction::PosY => Direction::NegY,
        Direction::NegY => Direction::PosY,
    }
}

fn axis_of(d: Direction) -> Axis {
    match d {
        Direction::PosX | Direction::NegX => Axis::X,
        Direction::PosY | Direction::NegY => Axis::Y,
    }
}

/// `n / d` rounded half to even (5.1); `d` must be positive.
fn round_half_even(n: i128, d: i128) -> i128 {
    let q = n / d;
    let r = n % d;
    let twice = 2 * r.abs();
    if twice > d || (twice == d && q % 2 != 0) {
        q + r.signum()
    } else {
        q
    }
}

fn to_length(v: i128) -> Result<Length, RoofProblem> {
    i64::try_from(v)
        .map(Length::from_nanometers)
        .map_err(|_| RoofProblem::Overflow)
}

/// Stores each roof's derived slope direction, ledger-edge overhang and
/// high or low edge height (Q-7);
/// roofs that are not well defined are left as they are and reported by
/// validation.
pub fn derive_roofs(document: &mut Document) {
    let ids: Vec<Uuid> = document.roofs.keys().copied().collect();
    for id in ids {
        let Ok(g) = roof_geometry(document, id) else {
            continue;
        };
        if let Some(roof) = document.roofs.get_mut(&id) {
            if roof.ledger.is_some() {
                roof.slope_direction = Sourced {
                    value: g.slope_direction,
                    source: Source::Derived,
                };
                // A ledger edge takes no overhang (Q-68).
                let o = &mut roof.overhang;
                let side = match opposite(g.slope_direction) {
                    Direction::PosX => &mut o.pos_x,
                    Direction::NegX => &mut o.neg_x,
                    Direction::PosY => &mut o.pos_y,
                    Direction::NegY => &mut o.neg_y,
                };
                *side = Sourced {
                    value: Length::from_nanometers(0),
                    source: Source::Derived,
                };
            }
            if roof.high_edge_height.source == Source::Derived {
                roof.high_edge_height.value = g.high_height;
            } else if let (Some(_), Some(low)) = (&roof.pitch, g.low_height) {
                roof.low_edge_height = Some(Sourced {
                    value: low,
                    source: Source::Derived,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::*;
    use crate::items::tests::{IN, STOREY, base, len, uid};
    use crate::proposal::derive;

    const FT: i64 = 12 * IN;

    fn user<T>(value: T) -> Sourced<T> {
        Sourced {
            value,
            source: Source::UserStated,
        }
    }

    fn proposed(nm: i64) -> Sourced<Length> {
        Sourced {
            value: len(nm),
            source: Source::Proposed,
        }
    }

    fn rect(x0: i64, y0: i64, x1: i64, y1: i64) -> Boundary {
        let p = |x, y| Point {
            x: len(x),
            y: len(y),
        };
        Boundary::Explicit {
            points: vec![p(x0, y0), p(x1, y0), p(x1, y1), p(x0, y1)],
        }
    }

    const ROOF: Uuid = Uuid::from_bytes([15; 16]);

    fn put_roof(
        doc: &mut Document,
        footprint: Boundary,
        slope: Direction,
        ledger: Option<Reference>,
        pitch: Option<(u32, u32)>,
        low: Option<Sourced<Length>>,
    ) {
        doc.roofs.insert(
            ROOF,
            Roof {
                storey: STOREY,
                footprint,
                slope_direction: user(slope),
                high_edge_height: user(len(10 * FT)),
                low_edge_height: low,
                pitch: pitch.map(|(rise, run)| user(Pitch { rise, run })),
                ledger,
                overhang: Overhang {
                    pos_x: proposed(12 * IN),
                    neg_x: proposed(12 * IN),
                    pos_y: proposed(12 * IN),
                    neg_y: proposed(12 * IN),
                },
                thickness: proposed(8 * IN),
                fascia_depth: proposed(6 * IN),
                material_slots: Default::default(),
            },
        );
    }

    fn low(doc: &Document) -> Option<Sourced<Length>> {
        doc.roofs.get(&ROOF).and_then(|r| r.low_edge_height.clone())
    }

    /// Done when: a 3:12 pitch over a 12 ft run gives exactly a 3 ft rise.
    #[test]
    fn a_3_in_12_pitch_over_12_ft_rises_3_ft() -> Result<(), RoofProblem> {
        let mut doc = base();
        // Slopes down toward +X across x = 0 to 12 ft.
        put_roof(
            &mut doc,
            rect(0, 0, 12 * FT, 20 * FT),
            Direction::PosX,
            None,
            Some((3, 12)),
            None,
        );
        let g = roof_geometry(&doc, ROOF)?;
        assert_eq!(
            (g.high_edge, g.low_edge, g.run),
            (len(0), len(12 * FT), len(12 * FT))
        );
        assert_eq!(g.low_height, Some(len(7 * FT)));
        assert_eq!(
            g.high_height.nanometers() - len(7 * FT).nanometers(),
            3 * FT
        );
        derive_roofs(&mut doc);
        assert_eq!(
            low(&doc),
            Some(Sourced {
                value: len(7 * FT),
                source: Source::Derived
            })
        );
        Ok(())
    }

    #[test]
    fn inexact_rises_round_half_to_even() -> Result<(), RoofProblem> {
        // 1:3 over 10 in: 3 1/3 in = 84,666,666.67 nm, rounded to 84,666,667.
        let mut doc = base();
        put_roof(
            &mut doc,
            rect(0, 0, 10 * IN, 20 * FT),
            Direction::NegX,
            None,
            Some((1, 3)),
            None,
        );
        let g = roof_geometry(&doc, ROOF)?;
        assert_eq!((g.high_edge, g.low_edge), (len(10 * IN), len(0)));
        assert_eq!(g.low_height, Some(len(10 * FT - 84_666_667)));
        Ok(())
    }

    /// Done when: the ledger case derives its direction.
    #[test]
    fn a_ledger_derives_the_slope_direction() -> Result<(), RoofProblem> {
        let mut doc = base();
        if let Some(w) = doc.walls.get_mut(&uid(2, 1)) {
            w.function = Function::Existing;
        }
        // The +X wall's left face is at y = 2 in and faces +Y: a lean-to off it.
        let ledger = Reference::WallFace {
            wall: uid(2, 1),
            side: WallSide::Left,
            boundary: 0,
        };
        put_roof(
            &mut doc,
            rect(0, 2 * IN, 20 * FT, 2 * IN + 12 * FT),
            Direction::NegX,
            Some(ledger),
            Some((3, 12)),
            None,
        );
        let g = roof_geometry(&doc, ROOF)?;
        assert_eq!(g.slope_direction, Direction::PosY);
        assert_eq!((g.high_edge, g.run), (len(2 * IN), len(12 * FT)));
        assert_eq!(g.low_height, Some(len(7 * FT)));
        derive_roofs(&mut doc);
        let slope = doc.roofs.get(&ROOF).map(|r| r.slope_direction.clone());
        assert_eq!(
            slope,
            Some(Sourced {
                value: Direction::PosY,
                source: Source::Derived
            })
        );
        // The right face of the same wall faces -Y.
        let mut doc2 = doc.clone();
        if let Some(r) = doc2.roofs.get_mut(&ROOF) {
            r.ledger = Some(Reference::WallFace {
                wall: uid(2, 1),
                side: WallSide::Right,
                boundary: 0,
            });
            r.footprint = rect(0, -2 * IN - 12 * FT, 20 * FT, -2 * IN);
        }
        assert_eq!(
            roof_geometry(&doc2, ROOF).map(|g| g.slope_direction),
            Ok(Direction::NegY)
        );
        Ok(())
    }

    #[test]
    fn a_ledger_must_be_on_an_existing_wall() {
        let mut doc = base();
        let ledger = Reference::WallFace {
            wall: uid(2, 1),
            side: WallSide::Left,
            boundary: 0,
        };
        put_roof(
            &mut doc,
            rect(0, 2 * IN, 20 * FT, 2 * IN + 12 * FT),
            Direction::PosY,
            Some(ledger),
            Some((3, 12)),
            None,
        );
        assert_eq!(
            roof_geometry(&doc, ROOF),
            Err(RoofProblem::LedgerNotExistingWallFace)
        );
        let violations = derive(doc).map(|d| d.violations).unwrap_or_default();
        assert_eq!(
            violations.iter().map(|v| v.key.kind).collect::<Vec<_>>(),
            vec![ViolationKind::RoofDefinition]
        );
    }

    #[test]
    fn a_stated_low_edge_must_match_the_pitch() {
        let mut doc = base();
        put_roof(
            &mut doc,
            rect(0, 0, 12 * FT, 20 * FT),
            Direction::PosX,
            None,
            Some((3, 12)),
            Some(user(len(8 * FT))),
        );
        assert_eq!(
            roof_geometry(&doc, ROOF),
            Err(RoofProblem::PitchDisagrees {
                stated: len(8 * FT),
                derived: len(7 * FT)
            })
        );
        // Without a pitch, a stated low edge is simply used.
        let mut doc = base();
        put_roof(
            &mut doc,
            rect(0, 0, 12 * FT, 20 * FT),
            Direction::PosX,
            None,
            None,
            Some(user(len(8 * FT))),
        );
        assert_eq!(
            roof_geometry(&doc, ROOF).map(|g| g.low_height),
            Ok(Some(len(8 * FT)))
        );
    }

    /// Done when: a missing pitch or low edge appears in required_parameters().
    #[test]
    fn a_missing_pitch_or_low_edge_is_required() -> Result<(), String> {
        let mut doc = base();
        put_roof(
            &mut doc,
            rect(0, 0, 12 * FT, 20 * FT),
            Direction::PosX,
            None,
            None,
            None,
        );
        let r = crate::params::required_parameters(&doc).map_err(|e| e.to_string())?;
        let missing: Vec<_> = r
            .iter()
            .filter(|p| p.reason == crate::params::Reason::Missing)
            .collect();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].collection, "roofs");
        assert_eq!(missing[0].field, "/pitch");
        assert_eq!(
            missing[0].alternatives,
            vec!["/low_edge_height".to_string()]
        );
        assert_eq!(missing[0].unit, crate::params::Unit::Pitch);
        assert_eq!(missing[0].proposed, None);
        assert!(crate::params::is_provisional(&doc));
        // Supplying the low edge clears it.
        if let Some(roof) = doc.roofs.get_mut(&ROOF) {
            roof.low_edge_height = Some(user(len(8 * FT)));
        }
        let r = crate::params::required_parameters(&doc).map_err(|e| e.to_string())?;
        assert!(r.iter().all(|p| p.reason != crate::params::Reason::Missing));
        Ok(())
    }

    fn ledger_roof(doc: &mut Document) {
        if let Some(w) = doc.walls.get_mut(&uid(2, 1)) {
            w.function = Function::Existing;
        }
        // A lean-to off the +X wall's left face at y = 2 in, sloping to +Y.
        put_roof(
            doc,
            rect(0, 2 * IN, 20 * FT, 2 * IN + 12 * FT),
            Direction::NegX,
            Some(Reference::WallFace {
                wall: uid(2, 1),
                side: WallSide::Left,
                boundary: 0,
            }),
            Some((3, 12)),
            None,
        );
    }

    fn pt(x: i64, y: i64) -> Point {
        Point {
            x: len(x),
            y: len(y),
        }
    }

    /// Q-68: overhangs grow every edge but the ledger edge, which stays at
    /// the ledger face; derive_roofs records that overhang as a derived 0.
    #[test]
    fn a_ledger_edge_takes_no_overhang() -> Result<(), RoofProblem> {
        let mut doc = base();
        ledger_roof(&mut doc);
        assert_eq!(
            roof_outline(&doc, ROOF)?,
            vec![
                pt(-12 * IN, 2 * IN),
                pt(20 * FT + 12 * IN, 2 * IN),
                pt(20 * FT + 12 * IN, 2 * IN + 13 * FT),
                pt(-12 * IN, 2 * IN + 13 * FT),
            ]
        );
        derive_roofs(&mut doc);
        let o = doc.roofs.get(&ROOF).map(|r| r.overhang.clone());
        assert_eq!(
            o.map(|o| (o.neg_y, o.pos_y.source)),
            Some((
                Sourced {
                    value: len(0),
                    source: Source::Derived
                },
                Source::Proposed
            ))
        );
        // Without a ledger, all four edges grow.
        let mut free = base();
        put_roof(
            &mut free,
            rect(0, 0, 12 * FT, 20 * FT),
            Direction::PosX,
            None,
            Some((3, 12)),
            None,
        );
        assert_eq!(
            roof_outline(&free, ROOF)?,
            vec![
                pt(-12 * IN, -12 * IN),
                pt(13 * FT, -12 * IN),
                pt(13 * FT, 21 * FT),
                pt(-12 * IN, 21 * FT),
            ]
        );
        Ok(())
    }

    /// 6.5, Q-68: with a pitch and a stated low edge, the high edge is
    /// derived when it is not given.
    #[test]
    fn the_high_edge_derives_from_the_low_edge_and_pitch() -> Result<(), RoofProblem> {
        let mut doc = base();
        put_roof(
            &mut doc,
            rect(0, 0, 12 * FT, 20 * FT),
            Direction::PosX,
            None,
            Some((3, 12)),
            Some(user(len(7 * FT))),
        );
        if let Some(r) = doc.roofs.get_mut(&ROOF) {
            r.high_edge_height = Sourced {
                value: len(0),
                source: Source::Derived,
            };
        }
        let g = roof_geometry(&doc, ROOF)?;
        assert_eq!(
            (g.high_height, g.low_height),
            (len(10 * FT), Some(len(7 * FT)))
        );
        derive_roofs(&mut doc);
        let r = doc.roofs.get(&ROOF).cloned();
        assert_eq!(
            r.map(|r| (r.high_edge_height, r.low_edge_height)),
            Some((
                Sourced {
                    value: len(10 * FT),
                    source: Source::Derived
                },
                Some(user(len(7 * FT)))
            ))
        );
        Ok(())
    }

    /// DEC-6: the underside at s is high - (s - high_edge) x rise / run,
    /// rounded half to even, beyond the footprint too.
    #[test]
    fn the_underside_rounds_half_to_even() {
        let g = RoofGeometry {
            slope_direction: Direction::PosX,
            high_edge: len(0),
            low_edge: len(4),
            run: len(4),
            high_height: len(10),
            low_height: Some(len(9)),
        };
        let at = |s: i64| g.underside(len(s));
        assert_eq!(at(0), Ok(len(10)));
        assert_eq!(at(2), Ok(len(10)), "0.5 rounds to 0");
        assert_eq!(at(6), Ok(len(8)), "1.5 rounds to 2");
        assert_eq!(at(-2), Ok(len(10)), "-0.5 rounds to 0");
        assert_eq!(at(4), Ok(len(9)));
        let down_x = RoofGeometry {
            slope_direction: Direction::NegX,
            high_edge: len(4),
            low_edge: len(0),
            ..g
        };
        assert_eq!(down_x.underside(len(1)), Ok(len(9)), "3/4 rounds to 1");
        let undefined = RoofGeometry {
            low_height: None,
            ..g
        };
        assert_eq!(undefined.underside(len(1)), Err(RoofProblem::NoLowHeight));
    }
}
