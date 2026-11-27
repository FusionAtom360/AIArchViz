//! Snapping a pointer coordinate before it becomes a command argument
//! (T-202, spec 3.1, 5.3).
//!
//! A drag value is user-stated (5.3), so it must lie on the 1/16 in grid.
//! The raw pointer value snaps to the nearest reference within the snap
//! tolerance when one lies on the grid, otherwise to the nearest grid value.
//! References off the grid (a derived midpoint, a converted metric
//! dimension) are never snap targets: snapping to one would either commit an
//! off-grid value or land a hair beside it. Either way the result is an
//! integer multiple of the grid, taken from integer arithmetic only.

use archviz_core::GRID_NM;

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
    let (n, d) = (i128::from(v), i128::from(GRID_NM));
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
/// nanometers, else to the grid. `references` are (coordinate, description).
pub fn snap(raw: i64, references: &[(i64, String)], tolerance: i64) -> Snap {
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
            value: to_grid(raw),
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
        let s = snap(10 * G + G / 3, &refs, G);
        assert_eq!(s.value, 10 * G);
        assert_eq!(s.kind, SnapKind::Reference("W-001 left face".to_string()));
        // Out of reach: the grid.
        let s = snap(12 * G + 100, &refs, G);
        assert_eq!((s.value, s.kind), (12 * G, SnapKind::Grid));
    }

    #[test]
    fn an_off_grid_reference_is_never_a_target() {
        let refs = vec![(10 * G + 7, "a derived midpoint".to_string())];
        let s = snap(10 * G + 5, &refs, 10 * G);
        assert_eq!((s.value, s.kind), (10 * G, SnapKind::Grid));
    }

    #[test]
    fn the_nearest_reference_wins_and_ties_break_low() {
        let refs = vec![
            (12 * G, "b".to_string()),
            (8 * G, "a".to_string()),
            (11 * G, "c".to_string()),
        ];
        assert_eq!(snap(10 * G, &refs, 5 * G).value, 11 * G);
        let tied = vec![(12 * G, "b".to_string()), (8 * G, "a".to_string())];
        assert_eq!(snap(10 * G, &tied, 5 * G).value, 8 * G);
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
            let s = snap(raw, &refs, tolerance);
            prop_assert_eq!(s.value.rem_euclid(G), 0);
            if s.kind == SnapKind::Grid && raw.checked_abs().is_some_and(|a| a < i64::MAX - G) {
                prop_assert!((i128::from(s.value) - i128::from(raw)).abs() <= i128::from(G / 2));
            }
        }
    }
}
