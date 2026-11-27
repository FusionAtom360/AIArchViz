//! The shed roof (spec 6.5; Q-44, Q-45).
//!
//! A roof is one inclined plane over a rectilinear footprint. Heights are
//! above the storey elevation and measured at the footprint (the bearing
//! lines); overhangs extend the same plane beyond it. The high edge is the
//! footprint side facing up-slope, or the ledger face when there is one, and
//! the run is the horizontal distance from it to the footprint's low side
//! along the slope direction. With a pitch, rise = run x rise / run rounded
//! half to even to the nanometer (5.1), and the low edge height is derived.
//!
//! A ledger must be a face of an existing wall; the roof slopes down away
//! from it, so its slope direction is the face's outward normal (Q-45).

use std::fmt;

use crate::document::{Axis, Direction, Document, Function, Reference, Source, Sourced, WallSide};
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
    let high_height = roof.high_edge_height.value;
    let low_height = match &roof.pitch {
        Some(pitch) => {
            if pitch.value.run == 0 {
                return Err(RoofProblem::ZeroPitchRun);
            }
            let rise = round_half_even(
                run * i128::from(pitch.value.rise),
                i128::from(pitch.value.run),
            );
            let derived = to_length(i128::from(high_height.nanometers()) - rise)?;
            if let Some(stated) = &roof.low_edge_height {
                let both_stated =
                    stated.source != Source::Derived && pitch.source != Source::Derived;
                if both_stated && stated.value != derived {
                    return Err(RoofProblem::PitchDisagrees {
                        stated: stated.value,
                        derived,
                    });
                }
            }
            Some(derived)
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

/// Stores each roof's derived slope direction and low edge height (Q-7);
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
            }
            if let (Some(_), Some(low)) = (&roof.pitch, g.low_height) {
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
}
