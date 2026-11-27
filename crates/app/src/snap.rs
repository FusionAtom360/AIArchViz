//! Snapping a pointer coordinate before it becomes a command argument
//! (T-202, spec 3.1, 5.3).
//!
//! A drag value is user-stated (5.3), so it must lie on the 1/16 in grid.
//! The raw pointer value snaps to the nearest reference within the snap
//! tolerance when one lies on the grid, otherwise to the nearest multiple of
//! the grid step drawn at the current zoom (T-202c, Q-79 (a)). Every step is
//! a multiple of 1/16 in, so either way the value is on the grid.
//! References off the grid (a derived midpoint, a converted metric
//! dimension) are never snap targets: snapping to one would either commit an
//! off-grid value or land a hair beside it. Either way the result is an
//! integer multiple of the grid, taken from integer arithmetic only.

use archviz_core::GRID_NM;

const IN: i64 = 25_400_000;
const FT: i64 = 12 * IN;

/// The plan-view grid steps, finest first: powers of 2 of the inch up to
/// 1 in, then 3 in and 6 in (1/4 and 1/2 ft), then powers of 2 of the foot
/// up to 2048 ft. Each divides the next, so every coarser line is also a
/// finer one.
pub const STEPS: [i64; 19] = [
    IN / 16,
    IN / 8,
    IN / 4,
    IN / 2,
    IN,
    3 * IN,
    6 * IN,
    FT,
    2 * FT,
    4 * FT,
    8 * FT,
    16 * FT,
    32 * FT,
    64 * FT,
    128 * FT,
    256 * FT,
    512 * FT,
    1024 * FT,
    2048 * FT,
];

/// The fewest pixels between drawn grid lines.
pub const MIN_STEP_PX: f64 = 12.0;

/// The grid step drawn and snapped to at `scale` pixels per nanometer: the
/// finest step at least MIN_STEP_PX apart, else the coarsest.
pub fn visible_step(scale: f64) -> i64 {
    STEPS
        .iter()
        .copied()
        .find(|s| (*s as f64) * scale >= MIN_STEP_PX)
        .unwrap_or(STEPS[STEPS.len() - 1])
}

/// What a value snapped to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapKind {
    Grid,
    /// A reference, by its description.
    Reference(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snap {
    /// Nanometers, a multiple of GRID_NM.
    pub value: i64,
    pub kind: SnapKind,
}

/// The multiple of GRID_NM nearest `v`, ties to even (as the parser and core
/// round, 5.3); at the ends of the range, the next one toward zero.
pub fn to_grid(v: i64) -> i64 {
    to_step(v, GRID_NM)
}

/// The multiple of `step` (positive) nearest `v`, ties to the even multiple;
/// at the ends of the range, the next one toward zero.
pub fn to_step(v: i64, step: i64) -> i64 {
    let (n, d) = (i128::from(v), i128::from(step.max(1)));
    let q = n.div_euclid(d);
    let r = n.rem_euclid(d);
    let q = match (2 * r).cmp(&d) {
        std::cmp::Ordering::Less => q,
        std::cmp::Ordering::Greater => q + 1,
        std::cmp::Ordering::Equal => q + (q & 1),
    };
    i64::try_from(q * d)
        .or_else(|_| i64::try_from((n / d) * d))
        .unwrap_or(0)
}

/// Snaps `raw` to the nearest on-grid reference within `tolerance`
/// nanometers, else to the nearest multiple of `step`, one of STEPS.
/// `references` are (coordinate, description).
pub fn snap(raw: i64, references: &[(i64, String)], tolerance: i64, step: i64) -> Snap {
    let near = references
        .iter()
        .filter(|(v, _)| v.rem_euclid(GRID_NM) == 0)
        .map(|(v, name)| ((i128::from(*v) - i128::from(raw)).abs(), *v, name))
        .filter(|(d, _, _)| *d <= i128::from(tolerance.max(0)))
        .min_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    match near {
        Some((_, value, name)) => Snap {
            value,
            kind: SnapKind::Reference(name.clone()),
        },
        None => Snap {
            value: to_step(raw, step),
            kind: SnapKind::Grid,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const G: i64 = GRID_NM;

    #[test]
    fn rounds_to_the_nearest_sixteenth_ties_to_even() {
        assert_eq!(to_grid(0), 0);
        assert_eq!(to_grid(G / 2 - 1), 0);
        assert_eq!(to_grid(G / 2 + 1), G);
        assert_eq!(to_grid(G / 2), 0, "tie to the even multiple");
        assert_eq!(to_grid(3 * G / 2), 2 * G);
        assert_eq!(to_grid(-G / 2 - 1), -G);
        assert_eq!(to_grid(-3 * G / 2), -2 * G);
        assert_eq!(to_grid(i64::MAX) % G, 0);
        assert_eq!(to_grid(i64::MIN) % G, 0);
    }

    #[test]
    fn an_on_grid_reference_in_reach_wins() {
        let refs = vec![(10 * G, "W-001 left face".to_string())];
        let s = snap(10 * G + G / 3, &refs, G, G);
        assert_eq!(s.value, 10 * G);
        assert_eq!(s.kind, SnapKind::Reference("W-001 left face".to_string()));
        // Out of reach: the grid.
        let s = snap(12 * G + 100, &refs, G, G);
        assert_eq!((s.value, s.kind), (12 * G, SnapKind::Grid));
    }

    #[test]
    fn an_off_grid_reference_is_never_a_target() {
        let refs = vec![(10 * G + 7, "a derived midpoint".to_string())];
        let s = snap(10 * G + 5, &refs, 10 * G, G);
        assert_eq!((s.value, s.kind), (10 * G, SnapKind::Grid));
    }

    #[test]
    fn the_nearest_reference_wins_and_ties_break_low() {
        let refs = vec![
            (12 * G, "b".to_string()),
            (8 * G, "a".to_string()),
            (11 * G, "c".to_string()),
        ];
        assert_eq!(snap(10 * G, &refs, 5 * G, G).value, 11 * G);
        let tied = vec![(12 * G, "b".to_string()), (8 * G, "a".to_string())];
        assert_eq!(snap(10 * G, &tied, 5 * G, G).value, 8 * G);
    }

    #[test]
    fn every_step_is_a_usable_value() {
        let shown: Vec<String> = STEPS
            .iter()
            .map(|s| archviz_core::format_length(archviz_core::Length::from_nanometers(*s)))
            .collect();
        assert_eq!(
            shown,
            [
                "0'-0 1/16\"",
                "0'-0 1/8\"",
                "0'-0 1/4\"",
                "0'-0 1/2\"",
                "0'-1\"",
                "0'-3\"",
                "0'-6\"",
                "1'-0\"",
                "2'-0\"",
                "4'-0\"",
                "8'-0\"",
                "16'-0\"",
                "32'-0\"",
                "64'-0\"",
                "128'-0\"",
                "256'-0\"",
                "512'-0\"",
                "1024'-0\"",
                "2048'-0\""
            ]
        );
        let ratios: Vec<i64> = STEPS.windows(2).map(|w| w[1] / w[0]).collect();
        for (i, w) in STEPS.windows(2).enumerate() {
            assert_eq!(w[1] % w[0], 0, "each step divides the next");
            // Powers of 2 everywhere but 1 in to 3 in (a foot is 3 x 4 in).
            assert_eq!(ratios[i], if w[0] == 25_400_000 { 3 } else { 2 }, "{i}");
        }
        assert!(STEPS.iter().all(|s| s % G == 0));
    }

    #[test]
    fn a_step_at_least_twelve_pixels_apart_exists_at_every_zoom() {
        use crate::viewport::{MAX_SCALE, MIN_SCALE};
        assert_eq!(visible_step(MAX_SCALE), G, "closest zoom: 1/16 in");
        let far = visible_step(MIN_SCALE);
        assert!(far as f64 * MIN_SCALE >= MIN_STEP_PX, "farthest zoom");
        assert_eq!(far, 2048 * 304_800_000);
        // 1 px = 1 in: 1 ft is the finest step 12 px apart.
        assert_eq!(visible_step(1.0 / 25_400_000.0), 304_800_000);
        // Each step is chosen where it first reaches 12 px.
        for w in STEPS.windows(2) {
            let at = MIN_STEP_PX / w[1] as f64;
            assert_eq!(visible_step(at * 1.000_001), w[1]);
            // Just below, a coarser step; below the coarsest, the coarsest.
            let below = visible_step(at * 0.999_999);
            assert!(below > w[1] || below == STEPS[STEPS.len() - 1], "{below}");
        }
    }

    #[test]
    fn without_a_reference_a_value_snaps_to_the_visible_step() {
        const FT: i64 = 304_800_000;
        let s = snap(12 * FT + 5 * 25_400_000 + 7, &[], 0, FT);
        assert_eq!((s.value, s.kind), (12 * FT, SnapKind::Grid));
        assert_eq!(snap(12 * FT + 7 * 25_400_000, &[], 0, FT).value, 13 * FT);
        assert_eq!(
            snap(FT / 2, &[], 0, FT).value,
            0,
            "tie to the even multiple"
        );
        assert_eq!(snap(3 * FT / 2, &[], 0, FT).value, 2 * FT);
        assert_eq!(snap(-3 * FT / 2, &[], 0, FT).value, -2 * FT);
        // An on-grid reference off the step still wins within reach.
        let refs = vec![(12 * FT + 5 * G, "W-001 left face".to_string())];
        assert_eq!(
            snap(12 * FT + 3 * G, &refs, 4 * G, FT).value,
            12 * FT + 5 * G
        );
        assert_eq!(to_step(i64::MAX, 2048 * FT) % (2048 * FT), 0);
        assert_eq!(to_step(i64::MIN, 2048 * FT) % (2048 * FT), 0);
    }

    proptest! {
        /// Done when (T-202c): at any step, a value snapped to the grid is a
        /// multiple of that step within half a step of the pointer, and every
        /// snapped value is on the 1/16 in grid.
        #[test]
        fn every_snap_is_on_the_visible_step(
            raw in any::<i64>(),
            which in 0_usize..STEPS.len(),
            refs in proptest::collection::vec(any::<i64>(), 0..6),
            tolerance in 0_i64..(1 << 40),
        ) {
            let step = STEPS[which];
            let refs: Vec<(i64, String)> = refs.into_iter().map(|v| (v, String::new())).collect();
            let s = snap(raw, &refs, tolerance, step);
            prop_assert_eq!(s.value.rem_euclid(G), 0);
            match s.kind {
                SnapKind::Grid => {
                    prop_assert_eq!(s.value.rem_euclid(step), 0);
                    if raw.checked_abs().is_some_and(|a| a < i64::MAX - step) {
                        prop_assert!((i128::from(s.value) - i128::from(raw)).abs() <= i128::from(step / 2));
                    }
                }
                SnapKind::Reference(_) => {
                    prop_assert!(refs.iter().any(|(v, _)| *v == s.value));
                }
            }
        }
    }

    proptest! {
        /// Whatever the pointer and the references, the value is a multiple
        /// of the grid and within half a step of the pointer or a reference.
        #[test]
        fn every_snap_is_on_the_grid(
            raw in any::<i64>(),
            refs in proptest::collection::vec(any::<i64>(), 0..6),
            tolerance in 0_i64..(1 << 40),
        ) {
            let refs: Vec<(i64, String)> = refs.into_iter().map(|v| (v, String::new())).collect();
            let s = snap(raw, &refs, tolerance, G);
            prop_assert_eq!(s.value.rem_euclid(G), 0);
            if s.kind == SnapKind::Grid && raw.checked_abs().is_some_and(|a| a < i64::MAX - G) {
                prop_assert!((i128::from(s.value) - i128::from(raw)).abs() <= i128::from(G / 2));
            }
        }
    }
}
