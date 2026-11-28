//! Underlay commands and calibration arithmetic (6.6, T-205, Q-83, DEC-14).
//!
//! An underlay is display-only: these commands write the `assets` and
//! `underlays` collections and nothing else, so no geometry can change.
//!
//! The scale maps a pixel segment to a real length as
//! `distance x |segment| / |calibration segment|`, a ratio of two square
//! roots. It is computed exactly in integers and rounded half to even to the
//! nanometer (5.1), with 256-bit intermediates.

use serde::{Deserialize, Serialize};

use crate::command::CommandError;
use crate::document::{
    Asset, CalibrationCheck, ContentHash, Direction, Document, Fidelity, KnownDistance,
    Millidegrees, PixelPoint, Point, Underlay,
};
use crate::identity::Uuid;
use crate::length::Length;

/// The largest pixel coordinate an underlay point may have (Q-83 (4)(i)).
pub const PIXEL_MAX: i64 = 65_535;

/// The license recorded for an underlay image (DEC-14).
pub const UNDERLAY_LICENSE: &str = "client-provided";

/// Creates an underlay and the Asset of its image (DEC-14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateUnderlay {
    pub name: String,
    pub storey: Uuid,
    /// SHA-256 of the image file the app copied into the project.
    pub image: ContentHash,
    /// Sets the scale.
    pub calibration: KnownDistance,
    /// Where `calibration.pixel_a` lies in the world.
    pub origin: Point,
    /// The world direction of `calibration.pixel_a` to `pixel_b`.
    pub heading: Direction,
    /// A second known distance; its residual is derived and stored.
    #[serde(default)]
    pub check: Option<KnownDistance>,
}

/// Replaces an underlay's calibration and placement (DEC-14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetUnderlayCalibration {
    pub underlay: Uuid,
    pub calibration: KnownDistance,
    pub origin: Point,
    pub heading: Direction,
    #[serde(default)]
    pub check: Option<KnownDistance>,
}

/// Why a calibration cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalibrationError {
    /// A pixel coordinate outside 0..=PIXEL_MAX.
    PixelOutOfRange(PixelPoint),
    /// The two points of a known distance are the same pixel.
    SamePoint(PixelPoint),
    /// A known distance that is not positive.
    NotPositive(Length),
    /// The scaled length or the residual does not fit a Length.
    OutOfRange,
}

impl std::fmt::Display for CalibrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CalibrationError::PixelOutOfRange(p) => {
                write!(f, "pixel ({}, {}) is outside 0 to {PIXEL_MAX}", p.x, p.y)
            }
            CalibrationError::SamePoint(p) => {
                write!(
                    f,
                    "both points of a known distance are pixel ({}, {})",
                    p.x, p.y
                )
            }
            CalibrationError::NotPositive(l) => write!(
                f,
                "a known distance must be positive, not {}",
                crate::units::format_length(*l)
            ),
            CalibrationError::OutOfRange => write!(f, "the scaled length is out of range"),
        }
    }
}

impl From<CalibrationError> for CommandError {
    fn from(e: CalibrationError) -> Self {
        CommandError::InvalidArguments(e.to_string())
    }
}

/// The stored heading of an axis direction (Q-5: 0 = +X, counterclockwise).
pub fn heading_of(direction: Direction) -> Millidegrees {
    Millidegrees::of_direction(direction)
}

/// The axis direction of a stored heading, if it is a quarter turn.
pub fn direction_of(heading: Millidegrees) -> Option<Direction> {
    match heading.value() {
        0 => Some(Direction::PosX),
        90_000 => Some(Direction::PosY),
        180_000 => Some(Direction::NegX),
        270_000 => Some(Direction::NegY),
        _ => None,
    }
}

fn pixel(p: PixelPoint) -> Result<PixelPoint, CalibrationError> {
    if (0..=PIXEL_MAX).contains(&p.x) && (0..=PIXEL_MAX).contains(&p.y) {
        Ok(p)
    } else {
        Err(CalibrationError::PixelOutOfRange(p))
    }
}

/// The squared pixel length of a segment, both ends checked.
fn squared(a: PixelPoint, b: PixelPoint) -> Result<u128, CalibrationError> {
    let (a, b) = (pixel(a)?, pixel(b)?);
    let dx = u128::from((a.x - b.x).unsigned_abs());
    let dy = u128::from((a.y - b.y).unsigned_abs());
    Ok(dx * dx + dy * dy)
}

/// Checks one known distance: both pixels in range, distinct, and a
/// positive distance.
pub fn check_known(k: &KnownDistance) -> Result<(), CalibrationError> {
    if squared(k.pixel_a, k.pixel_b)? == 0 {
        return Err(CalibrationError::SamePoint(k.pixel_a));
    }
    if k.distance.value.nanometers() <= 0 {
        return Err(CalibrationError::NotPositive(k.distance.value));
    }
    Ok(())
}

/// A 256-bit unsigned integer, high half first so the derived order is
/// numeric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Wide {
    hi: u128,
    lo: u128,
}

const HALF: u32 = 64;
const LOW: u128 = u64::MAX as u128;

/// The full product of two u128 values.
fn mul_wide(a: u128, b: u128) -> Wide {
    let (a0, a1) = (a & LOW, a >> HALF);
    let (b0, b1) = (b & LOW, b >> HALF);
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;
    let mid = (p00 >> HALF) + (p01 & LOW) + (p10 & LOW);
    Wide {
        hi: p11 + (p01 >> HALF) + (p10 >> HALF) + (mid >> HALF),
        lo: (p00 & LOW) | ((mid & LOW) << HALF),
    }
}

/// `w x k`, or None if it does not fit 256 bits.
fn mul_small(w: Wide, k: u128) -> Option<Wide> {
    let low = mul_wide(w.lo, k);
    let hi = w.hi.checked_mul(k)?.checked_add(low.hi)?;
    Some(Wide { hi, lo: low.lo })
}

/// The real length of the pixel segment `a` to `b` under `calibration`:
/// round half to even of `distance x sqrt(|ab|^2 / |calibration|^2)`.
pub fn scaled_length(
    calibration: &KnownDistance,
    a: PixelPoint,
    b: PixelPoint,
) -> Result<Length, CalibrationError> {
    check_known(calibration)?;
    let k2 = squared(calibration.pixel_a, calibration.pixel_b)?;
    let c2 = squared(a, b)?;
    let d = u128::from(calibration.distance.value.nanometers().unsigned_abs());
    // N = d^2 c2; the result r is the integer nearest sqrt(N / k2).
    let n = mul_wide(d * d, c2);
    let fits = |f: u128| mul_small(mul_wide(f, f), k2).is_some_and(|v| v <= n);
    // r < 2^63 x 2^17, so the floor lies below 2^81.
    let (mut lo, mut hi) = (0_u128, 1_u128 << 81);
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let floor = lo;
    let odd = 2 * floor + 1;
    let four_n = mul_small(n, 4).ok_or(CalibrationError::OutOfRange)?;
    let half = mul_small(mul_wide(odd, odd), k2).ok_or(CalibrationError::OutOfRange)?;
    let r = match four_n.cmp(&half) {
        std::cmp::Ordering::Greater => floor + 1,
        std::cmp::Ordering::Less => floor,
        std::cmp::Ordering::Equal if floor % 2 == 0 => floor,
        std::cmp::Ordering::Equal => floor + 1,
    };
    i64::try_from(r)
        .map(Length::from_nanometers)
        .map_err(|_| CalibrationError::OutOfRange)
}

/// The check's distance minus what the calibration reads for it: positive
/// when the image reads short (Q-83 (4)(i)).
pub fn residual(
    calibration: &KnownDistance,
    check: &KnownDistance,
) -> Result<Length, CalibrationError> {
    check_known(check)?;
    let scaled = scaled_length(calibration, check.pixel_a, check.pixel_b)?;
    check
        .distance
        .value
        .checked_sub(scaled)
        .map_err(|_| CalibrationError::OutOfRange)
}

fn derive_check(
    calibration: &KnownDistance,
    check: &Option<KnownDistance>,
) -> Result<Option<CalibrationCheck>, CalibrationError> {
    check_known(calibration)?;
    check
        .as_ref()
        .map(|known| {
            Ok(CalibrationCheck {
                known: known.clone(),
                residual: residual(calibration, known)?,
            })
        })
        .transpose()
}

/// Inserts the image Asset as `asset` and the underlay as `id`.
pub fn create_underlay(
    doc: &mut Document,
    args: &CreateUnderlay,
    asset: Uuid,
    id: Uuid,
) -> Result<(), CommandError> {
    if !doc.storeys.contains_key(&args.storey) {
        return Err(CommandError::UnknownEntity(args.storey));
    }
    for new in [asset, id] {
        if doc.assets.contains_key(&new) || doc.underlays.contains_key(&new) {
            return Err(CommandError::IdCollision(new));
        }
    }
    let check = derive_check(&args.calibration, &args.check)?;
    doc.assets.insert(
        asset,
        Asset {
            content_hash: args.image.clone(),
            source_url: None,
            license: UNDERLAY_LICENSE.into(),
            fidelity: Fidelity::Proxy,
            converter_log: Vec::new(),
        },
    );
    doc.underlays.insert(
        id,
        Underlay {
            name: args.name.clone(),
            storey: args.storey,
            image: asset,
            calibration: args.calibration.clone(),
            origin: args.origin,
            heading: heading_of(args.heading),
            check,
        },
    );
    Ok(())
}

/// Replaces the calibration, placement and check of an underlay.
pub fn set_underlay_calibration(
    doc: &mut Document,
    args: &SetUnderlayCalibration,
) -> Result<(), CommandError> {
    let check = derive_check(&args.calibration, &args.check)?;
    let u = doc
        .underlays
        .get_mut(&args.underlay)
        .ok_or(CommandError::UnknownEntity(args.underlay))?;
    u.calibration = args.calibration.clone();
    u.origin = args.origin;
    u.heading = heading_of(args.heading);
    u.check = check;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Source, Sourced};
    use proptest::prelude::*;

    const IN: i64 = 25_400_000;
    const FT: i64 = 12 * IN;

    fn px(x: i64, y: i64) -> PixelPoint {
        PixelPoint { x, y }
    }

    fn known(a: PixelPoint, b: PixelPoint, nm: i64) -> KnownDistance {
        KnownDistance {
            pixel_a: a,
            pixel_b: b,
            distance: Sourced {
                value: Length::from_nanometers(nm),
                source: Source::Measured,
            },
        }
    }

    fn nm(l: Result<Length, CalibrationError>) -> Result<i64, CalibrationError> {
        l.map(Length::nanometers)
    }

    /// The fixture scan: 2,400 px = 20 ft, so a 1,440 px check is 12 ft.
    #[test]
    fn an_axis_segment_scales_exactly() {
        let cal = known(px(120, 88), px(2520, 88), 20 * FT);
        assert_eq!(
            nm(scaled_length(&cal, px(120, 88), px(120, 1528))),
            Ok(12 * FT)
        );
        let check = known(px(120, 88), px(120, 1528), 12 * FT + IN / 4);
        assert_eq!(nm(residual(&cal, &check)), Ok(IN / 4));
        let check = known(px(120, 88), px(120, 1528), 12 * FT - IN / 2);
        assert_eq!(nm(residual(&cal, &check)), Ok(-IN / 2));
    }

    #[test]
    fn scaling_rounds_half_to_even_and_handles_irrational_ratios() {
        // 2 px = 1 nm: 1 px is 0.5 nm (to 0), 3 px is 1.5 nm (to 2), 5 px 2.5 (to 2).
        let cal = known(px(0, 0), px(2, 0), 1);
        assert_eq!(nm(scaled_length(&cal, px(0, 0), px(1, 0))), Ok(0));
        assert_eq!(nm(scaled_length(&cal, px(0, 0), px(3, 0))), Ok(2));
        assert_eq!(nm(scaled_length(&cal, px(0, 0), px(5, 0))), Ok(2));
        assert_eq!(nm(scaled_length(&cal, px(0, 0), px(7, 0))), Ok(4));
        // A 3-4-5 triangle is exact.
        let cal = known(px(0, 0), px(3, 4), 5_000);
        assert_eq!(nm(scaled_length(&cal, px(0, 0), px(0, 1))), Ok(1_000));
        // 1 px = 1 mm: the diagonal of one pixel is 1,414,213.562 nm.
        let cal = known(px(0, 0), px(1, 0), 1_000_000);
        assert_eq!(nm(scaled_length(&cal, px(0, 0), px(1, 1))), Ok(1_414_214));
        // sqrt(2) / sqrt(5) x 10^9 = 632,455,532.03...
        let cal = known(px(0, 0), px(1, 2), 1_000_000_000);
        assert_eq!(nm(scaled_length(&cal, px(5, 5), px(6, 6))), Ok(632_455_532));
    }

    #[test]
    fn the_largest_inputs_compute_or_report_out_of_range() {
        let max = PIXEL_MAX;
        let cal = known(px(0, 0), px(max, max), i64::MAX);
        assert_eq!(
            nm(scaled_length(&cal, px(0, 0), px(max, max))),
            Ok(i64::MAX)
        );
        assert_eq!(
            nm(scaled_length(&cal, px(max, max), px(0, 0))),
            Ok(i64::MAX)
        );
        let cal = known(px(0, 0), px(0, 1), i64::MAX);
        assert_eq!(
            scaled_length(&cal, px(0, 0), px(max, max)),
            Err(CalibrationError::OutOfRange)
        );
        let check = known(px(0, 0), px(0, 1), i64::MAX);
        let cal = known(px(0, 0), px(0, 1), 1);
        assert_eq!(nm(residual(&cal, &check)), Ok(i64::MAX - 1));
    }

    #[test]
    fn bad_known_distances_are_refused() {
        let ok = known(px(0, 0), px(10, 0), FT);
        assert_eq!(
            scaled_length(&known(px(3, 3), px(3, 3), FT), px(0, 0), px(1, 0)),
            Err(CalibrationError::SamePoint(px(3, 3)))
        );
        assert_eq!(
            scaled_length(&known(px(0, 0), px(1, 0), 0), px(0, 0), px(1, 0)),
            Err(CalibrationError::NotPositive(Length::from_nanometers(0)))
        );
        assert_eq!(
            scaled_length(&ok, px(0, 0), px(PIXEL_MAX + 1, 0)),
            Err(CalibrationError::PixelOutOfRange(px(PIXEL_MAX + 1, 0)))
        );
        assert_eq!(
            scaled_length(&known(px(-1, 0), px(1, 0), FT), px(0, 0), px(1, 0)),
            Err(CalibrationError::PixelOutOfRange(px(-1, 0)))
        );
        assert_eq!(
            residual(&ok, &known(px(0, 0), px(10, 0), -FT)),
            Err(CalibrationError::NotPositive(Length::from_nanometers(-FT)))
        );
    }

    #[test]
    fn headings_are_quarter_turns() {
        for (d, v) in [
            (Direction::PosX, 0),
            (Direction::PosY, 90_000),
            (Direction::NegX, 180_000),
            (Direction::NegY, 270_000),
        ] {
            assert_eq!(heading_of(d).value(), v);
            assert_eq!(direction_of(heading_of(d)), Some(d));
        }
        assert_eq!(Millidegrees::new(45_000).and_then(direction_of), None);
    }

    fn pixel_point() -> impl Strategy<Value = PixelPoint> {
        (0..=PIXEL_MAX, 0..=PIXEL_MAX).prop_map(|(x, y)| px(x, y))
    }

    proptest! {
        /// The calibration segment reads its own distance; an integer
        /// multiple of it reads that multiple; the order of the ends does not
        /// matter; the result lies within half a nanometer of the true value
        /// (checked exactly: (2r - 1)^2 k2 <= 4 d^2 c2 <= (2r + 1)^2 k2).
        #[test]
        fn scaling_is_exact_and_symmetric(
            a in pixel_point(),
            v in (-200_i64..=200, -200_i64..=200),
            m in 1_i64..=50,
            d in 1_i64..=(1_i64 << 45),
            c in (pixel_point(), pixel_point()),
        ) {
            let b = px(a.x + v.0, a.y + v.1);
            prop_assume!(v != (0, 0) && pixel(b).is_ok());
            let cal = known(a, b, d);
            prop_assert_eq!(nm(scaled_length(&cal, a, b)), Ok(d));
            let far = px(a.x + m * v.0, a.y + m * v.1);
            if pixel(far).is_ok() {
                prop_assert_eq!(nm(scaled_length(&cal, a, far)), Ok(m * d));
            }
            let (p, q) = c;
            let r = scaled_length(&cal, p, q);
            prop_assert_eq!(&r, &scaled_length(&cal, q, p));
            let r = u128::try_from(nm(r).unwrap_or(-1)).unwrap_or(0);
            let k2 = squared(a, b).unwrap_or(0);
            let c2 = squared(p, q).unwrap_or(0);
            let d = u128::try_from(d).unwrap_or(0);
            let four_n = mul_small(mul_wide(d * d, c2), 4);
            let bound = |s: u128| mul_small(mul_wide(s, s), k2);
            prop_assert!(r == 0 || bound(2 * r - 1) <= four_n);
            prop_assert!(four_n <= bound(2 * r + 1));
        }
    }

    #[test]
    fn wide_multiplication_matches_known_products() {
        let max = u128::MAX;
        // (2^128 - 1)^2 = 2^256 - 2^129 + 1.
        assert_eq!(mul_wide(max, max), Wide { hi: max - 1, lo: 1 });
        assert_eq!(mul_wide(1 << 64, 1 << 64), Wide { hi: 1, lo: 0 });
        assert_eq!(
            mul_wide(12_345, 678),
            Wide {
                hi: 0,
                lo: 8_369_910
            }
        );
        assert_eq!(mul_small(Wide { hi: max, lo: 0 }, 2), None);
        assert_eq!(
            mul_small(Wide { hi: 0, lo: max }, 2),
            Some(Wide { hi: 1, lo: max - 1 })
        );
    }

    mod document {
        use super::*;
        use crate::command::tests::cmd;
        use crate::command::{Action, CommandError, apply};
        use crate::echo::tests::labeled_chain;
        use crate::geometry::wall_geometry;
        use crate::log::History;
        use crate::proposal::{ProposedAction, propose};
        use crate::solver::{LayerA, Solver};
        use crate::validity::validate;

        fn hash(c: char) -> ContentHash {
            ContentHash::parse(&format!("sha256:{}", c.to_string().repeat(64)))
                .unwrap_or_else(|| unreachable!())
        }

        fn storey(doc: &Document) -> Uuid {
            doc.storeys
                .keys()
                .next()
                .copied()
                .unwrap_or_else(|| unreachable!())
        }

        fn point(x: i64, y: i64) -> Point {
            Point {
                x: Length::from_nanometers(x),
                y: Length::from_nanometers(y),
            }
        }

        fn create(doc: &Document) -> CreateUnderlay {
            CreateUnderlay {
                name: "Paper plan".into(),
                storey: storey(doc),
                image: hash('a'),
                calibration: known(px(120, 88), px(2520, 88), 20 * FT),
                origin: point(0, 0),
                heading: Direction::PosX,
                check: Some(known(px(120, 88), px(120, 1528), 12 * FT + IN / 4)),
            }
        }

        #[test]
        fn create_underlay_adds_the_image_asset_and_the_underlay() {
            let (doc, ..) = labeled_chain();
            let c = cmd(1, Action::CreateUnderlay(create(&doc)));
            let after = apply(&doc, &c).unwrap_or_else(|e| unreachable!("{e}"));
            let asset_id = Uuid::derive(c.idempotency_key, "command", "0");
            let id = Uuid::derive(c.idempotency_key, "command", "1");
            assert_eq!(
                after.assets.get(&asset_id).cloned(),
                Some(Asset {
                    content_hash: hash('a'),
                    source_url: None,
                    license: "client-provided".into(),
                    fidelity: Fidelity::Proxy,
                    converter_log: Vec::new(),
                })
            );
            let u = after
                .underlays
                .get(&id)
                .cloned()
                .unwrap_or_else(|| unreachable!());
            assert_eq!(u.image, asset_id);
            assert_eq!(u.heading.value(), 0);
            assert_eq!(u.check.map(|k| k.residual.nanometers()), Some(IN / 4));
            // The same command again collides on its derived ids.
            assert_eq!(apply(&after, &c), Err(CommandError::IdCollision(asset_id)));
        }

        #[test]
        fn set_underlay_calibration_replaces_the_placement() {
            let (doc, ..) = labeled_chain();
            let c = cmd(1, Action::CreateUnderlay(create(&doc)));
            let doc = apply(&doc, &c).unwrap_or_else(|e| unreachable!("{e}"));
            let id = Uuid::derive(c.idempotency_key, "command", "1");
            let set = SetUnderlayCalibration {
                underlay: id,
                calibration: known(px(0, 0), px(0, 100), 10 * FT),
                origin: point(FT, -FT),
                heading: Direction::NegY,
                check: None,
            };
            let after = apply(&doc, &cmd(2, Action::SetUnderlayCalibration(set.clone())))
                .unwrap_or_else(|e| unreachable!("{e}"));
            let u = after
                .underlays
                .get(&id)
                .cloned()
                .unwrap_or_else(|| unreachable!());
            assert_eq!(u.calibration, set.calibration);
            assert_eq!(u.origin, set.origin);
            assert_eq!(u.heading.value(), 270_000);
            assert_eq!(u.check, None);
            assert_eq!(u.name, "Paper plan");
            let unknown = SetUnderlayCalibration {
                underlay: storey(&doc),
                ..set
            };
            assert_eq!(
                apply(&doc, &cmd(3, Action::SetUnderlayCalibration(unknown))),
                Err(CommandError::UnknownEntity(storey(&doc)))
            );
        }

        #[test]
        fn bad_underlay_arguments_are_refused() {
            let (doc, ..) = labeled_chain();
            let nowhere = Uuid::from_bytes([9; 16]);
            let mut a = create(&doc);
            a.storey = nowhere;
            assert_eq!(
                apply(&doc, &cmd(1, Action::CreateUnderlay(a))),
                Err(CommandError::UnknownEntity(nowhere))
            );
            let mut a = create(&doc);
            a.calibration.pixel_b = a.calibration.pixel_a;
            assert!(matches!(
                apply(&doc, &cmd(1, Action::CreateUnderlay(a))),
                Err(CommandError::InvalidArguments(t)) if t.contains("both points")
            ));
            let mut a = create(&doc);
            a.check = Some(known(px(0, 0), px(PIXEL_MAX + 1, 0), FT));
            assert!(matches!(
                apply(&doc, &cmd(1, Action::CreateUnderlay(a))),
                Err(CommandError::InvalidArguments(t)) if t.contains("outside 0 to 65535")
            ));
        }

        /// `after` without the underlays and assets `before` does not have.
        fn without_new_underlays(after: &Document, before: &Document) -> Document {
            let mut d = after.clone();
            d.underlays
                .retain(|id, _| before.underlays.contains_key(id));
            d.assets.retain(|id, _| before.assets.contains_key(id));
            d
        }

        fn direction() -> impl Strategy<Value = Direction> {
            prop_oneof![
                Just(Direction::PosX),
                Just(Direction::NegX),
                Just(Direction::PosY),
                Just(Direction::NegY),
            ]
        }

        fn known_distance() -> impl Strategy<Value = KnownDistance> {
            (pixel_point(), pixel_point(), 1_i64..=(1_i64 << 50))
                .prop_filter("distinct", |(a, b, _)| a != b)
                .prop_map(|(a, b, d)| known(a, b, d))
        }

        fn coordinate() -> impl Strategy<Value = i64> {
            -(1_i64 << 50)..(1_i64 << 50)
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(64))]
            /// Done when: an underlay never changes geometry. Creating one and
            /// recalibrating it leave every other collection, the solve, the
            /// wall geometry and the violations exactly as they were, and a
            /// proposal's diff lists only the underlay and its image.
            #[test]
            fn an_underlay_never_changes_geometry(
                calibration in known_distance(),
                check in proptest::option::of(known_distance()),
                again in known_distance(),
                origin in (coordinate(), coordinate()),
                moved in (coordinate(), coordinate()),
                heading in direction(),
                turn in direction(),
            ) {
                let (doc, ..) = labeled_chain();
                let mut args = create(&doc);
                args.calibration = calibration;
                args.check = check;
                args.origin = point(origin.0, origin.1);
                args.heading = heading;
                let c = cmd(1, Action::CreateUnderlay(args));
                let created = match apply(&doc, &c) {
                    Ok(d) => d,
                    // A check whose scaled length does not fit is refused.
                    Err(CommandError::InvalidArguments(_)) => return Ok(()),
                    Err(e) => return Err(TestCaseError::fail(e.to_string())),
                };
                let id = Uuid::derive(c.idempotency_key, "command", "1");
                let set = cmd(2, Action::SetUnderlayCalibration(SetUnderlayCalibration {
                    underlay: id,
                    calibration: again,
                    origin: point(moved.0, moved.1),
                    heading: turn,
                    check: None,
                }));
                let recalibrated = apply(&created, &set)
                    .map_err(|e| TestCaseError::fail(e.to_string()))?;
                let solver = LayerA::default();
                let min_jamb = crate::defaults::MIN_JAMB_DISTANCE;
                for after in [&created, &recalibrated] {
                    prop_assert_eq!(without_new_underlays(after, &doc), doc.clone());
                    prop_assert_eq!(solver.solve(after), solver.solve(&doc));
                    prop_assert_eq!(wall_geometry(after), wall_geometry(&doc));
                    prop_assert_eq!(validate(after, min_jamb), validate(&doc, min_jamb));
                }
                let history = History::new(doc.clone());
                let p = propose(&history, &ProposedAction::Commands { commands: vec![c] })
                    .map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
                let collections: Vec<&str> =
                    p.diff.iter().map(|e| e.collection.as_str()).collect();
                prop_assert_eq!(collections, vec!["assets", "underlays"]);
                prop_assert!(p.conflicts.is_empty());
            }
        }
    }
}
