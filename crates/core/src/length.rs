//! Length: a signed count of nanometers (spec 5.1, D-3).
//!
//! 1 in = 25,400,000 nm, so millimeters and inch fractions down to 1/64 in
//! are exact. Add, subtract and integer multiply are exact and checked.
//! Integer division goes through i128 and rounds half to even. Every
//! operation that cannot be represented returns an error; nothing wraps.

use std::fmt;

/// Nanometers per inch.
const NM_PER_INCH: i64 = 25_400_000;
/// Nanometers per foot.
const NM_PER_FOOT: i64 = 12 * NM_PER_INCH;
/// Nanometers per millimeter.
const NM_PER_MILLIMETER: i64 = 1_000_000;

/// A length in nanometers. Serialized as a bare JSON integer.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct Length(i64);

/// Length is a named schema definition, so tools can tell lengths from
/// other integers (Q-36).
impl schemars::JsonSchema for Length {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Length".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "integer",
            "format": "int64",
            "description": "A length in nanometers."
        })
    }
}

/// Why a length operation has no result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthError {
    /// The exact result lies outside the i64 nanometer range.
    Overflow,
    /// The divisor was zero.
    DivisionByZero,
}

impl fmt::Display for LengthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LengthError::Overflow => f.write_str("length out of range"),
            LengthError::DivisionByZero => f.write_str("length divided by zero"),
        }
    }
}

impl std::error::Error for LengthError {}

impl Length {
    /// A length of exactly `nm` nanometers.
    pub const fn from_nanometers(nm: i64) -> Self {
        Length(nm)
    }

    /// The length in nanometers.
    pub const fn nanometers(self) -> i64 {
        self.0
    }

    /// `inches` whole inches.
    pub fn from_inches(inches: i64) -> Result<Self, LengthError> {
        Length(inches).checked_mul(NM_PER_INCH)
    }

    /// `feet` whole feet.
    pub fn from_feet(feet: i64) -> Result<Self, LengthError> {
        Length(feet).checked_mul(NM_PER_FOOT)
    }

    /// `mm` whole millimeters.
    pub fn from_millimeters(mm: i64) -> Result<Self, LengthError> {
        Length(mm).checked_mul(NM_PER_MILLIMETER)
    }

    /// `self + rhs`, exact.
    pub fn checked_add(self, rhs: Length) -> Result<Self, LengthError> {
        self.0
            .checked_add(rhs.0)
            .map(Length)
            .ok_or(LengthError::Overflow)
    }

    /// `self - rhs`, exact.
    pub fn checked_sub(self, rhs: Length) -> Result<Self, LengthError> {
        self.0
            .checked_sub(rhs.0)
            .map(Length)
            .ok_or(LengthError::Overflow)
    }

    /// `self * k`, exact.
    pub fn checked_mul(self, k: i64) -> Result<Self, LengthError> {
        self.0
            .checked_mul(k)
            .map(Length)
            .ok_or(LengthError::Overflow)
    }

    /// `self / k` rounded half to even to the nearest nanometer.
    ///
    /// The quotient is computed in i128, where no intermediate can overflow;
    /// only a result outside i64 (`i64::MIN / -1`) is an error.
    pub fn checked_div(self, k: i64) -> Result<Self, LengthError> {
        if k == 0 {
            return Err(LengthError::DivisionByZero);
        }
        // Make the divisor positive so the remainder carries the sign of the quotient.
        let (n, d) = if k < 0 {
            (-i128::from(self.0), -i128::from(k))
        } else {
            (i128::from(self.0), i128::from(k))
        };
        let q = n / d;
        let r = n % d;
        let twice_r = 2 * r.abs();
        let away = twice_r > d || (twice_r == d && q % 2 != 0);
        let q = if away { q + r.signum() } else { q };
        i64::try_from(q)
            .map(Length)
            .map_err(|_| LengthError::Overflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn nm(v: i64) -> Length {
        Length::from_nanometers(v)
    }

    /// T-199b: every profile builds core with overflow checks, so unchecked
    /// arithmetic that overflows panics instead of wrapping. `cargo xtask
    /// check` runs this test in a release build (xtask/src/overflow.rs).
    #[test]
    fn unchecked_arithmetic_panics_on_overflow() {
        use std::hint::black_box;
        use std::panic::catch_unwind;
        let wide = catch_unwind(|| black_box(i128::MAX) + black_box(1));
        assert!(wide.is_err(), "i128::MAX + 1 gave {wide:?}");
        let narrow = catch_unwind(|| black_box(i64::MIN) - black_box(1));
        assert!(narrow.is_err(), "i64::MIN - 1 gave {narrow:?}");
        let product = catch_unwind(|| black_box(i64::MAX) * black_box(2));
        assert!(product.is_err(), "i64::MAX * 2 gave {product:?}");
    }

    #[test]
    fn one_inch_is_25_400_000_nm() {
        assert_eq!(Length::from_inches(1), Ok(nm(25_400_000)));
    }

    #[test]
    fn one_foot_is_twelve_inches() {
        assert_eq!(Length::from_feet(1), Ok(nm(304_800_000)));
        assert_eq!(Length::from_feet(-3), Length::from_inches(-36));
    }

    #[test]
    fn one_millimeter_is_1_000_000_nm() {
        assert_eq!(Length::from_millimeters(1), Ok(nm(1_000_000)));
        assert_eq!(Length::from_millimeters(-254), Length::from_inches(-10));
    }

    #[test]
    fn one_sixty_fourth_inch_is_exact() -> Result<(), LengthError> {
        let inch = Length::from_inches(1)?;
        let sixty_fourth = inch.checked_div(64)?;
        assert_eq!(sixty_fourth, nm(396_875));
        // Exact: no rounding happened, 64 of them rebuild the inch.
        assert_eq!(sixty_fourth.checked_mul(64)?, inch);
        // Every multiple of 1/64 in up to one inch is exact.
        for n in 0..=64 {
            let exact = inch.checked_mul(n)?.checked_div(64)?;
            assert_eq!(exact.checked_mul(64)?, inch.checked_mul(n)?);
        }
        Ok(())
    }

    /// (dividend nm, divisor, expected result).
    const ROUNDING_TABLE: &[(i64, i64, Result<i64, LengthError>)] = &[
        // Exact.
        (10, 2, Ok(5)),
        (-10, 2, Ok(-5)),
        (0, 7, Ok(0)),
        (0, -7, Ok(0)),
        // Below and above one half round to nearest.
        (1, 3, Ok(0)),
        (2, 3, Ok(1)),
        (-1, 3, Ok(0)),
        (-2, 3, Ok(-1)),
        (1, 4, Ok(0)),
        (3, 4, Ok(1)),
        // Halfway: ties go to the even neighbour, for every sign combination.
        (1, 2, Ok(0)),
        (3, 2, Ok(2)),
        (5, 2, Ok(2)),
        (7, 2, Ok(4)),
        (-1, 2, Ok(0)),
        (-3, 2, Ok(-2)),
        (-5, 2, Ok(-2)),
        (-7, 2, Ok(-4)),
        (5, -2, Ok(-2)),
        (7, -2, Ok(-4)),
        (-5, -2, Ok(2)),
        (-7, -2, Ok(4)),
        (6, 4, Ok(2)),
        (10, 4, Ok(2)),
        (14, 4, Ok(4)),
        // Inch fractions: 1/64 exact, 1/128 a tie, 1/3 rounds.
        (25_400_000, 64, Ok(396_875)),
        (25_400_000, 128, Ok(198_438)),
        (25_400_000, 3, Ok(8_466_667)),
        (-25_400_000, 3, Ok(-8_466_667)),
        // Range edges.
        (i64::MAX, 1, Ok(i64::MAX)),
        (i64::MAX, -1, Ok(-i64::MAX)),
        (i64::MIN, 1, Ok(i64::MIN)),
        (i64::MIN, 2, Ok(i64::MIN / 2)),
        (i64::MAX, 2, Ok(4_611_686_018_427_387_904)),
        (i64::MIN, -2, Ok(4_611_686_018_427_387_904)),
        (i64::MAX, i64::MAX, Ok(1)),
        (i64::MIN, i64::MAX, Ok(-1)),
        (1, i64::MIN, Ok(0)),
        (i64::MIN, -1, Err(LengthError::Overflow)),
        // Division by zero.
        (1, 0, Err(LengthError::DivisionByZero)),
        (0, 0, Err(LengthError::DivisionByZero)),
    ];

    #[test]
    fn rounding_table() {
        for &(n, k, expected) in ROUNDING_TABLE {
            assert_eq!(nm(n).checked_div(k), expected.map(nm), "{n} / {k}");
        }
    }

    #[test]
    fn overflow_is_an_error_never_a_wrap() {
        let max = nm(i64::MAX);
        let min = nm(i64::MIN);
        assert_eq!(max.checked_add(nm(1)), Err(LengthError::Overflow));
        assert_eq!(min.checked_sub(nm(1)), Err(LengthError::Overflow));
        assert_eq!(max.checked_mul(2), Err(LengthError::Overflow));
        assert_eq!(min.checked_mul(-1), Err(LengthError::Overflow));
        assert_eq!(Length::from_inches(i64::MAX), Err(LengthError::Overflow));
        assert_eq!(Length::from_feet(i64::MIN), Err(LengthError::Overflow));
        assert_eq!(
            Length::from_millimeters(i64::MAX),
            Err(LengthError::Overflow)
        );
        // The largest whole-inch length still fits.
        let most_inches = i64::MAX / NM_PER_INCH;
        assert_eq!(
            Length::from_inches(most_inches),
            Ok(nm(most_inches * NM_PER_INCH))
        );
        assert_eq!(
            Length::from_inches(most_inches + 1),
            Err(LengthError::Overflow)
        );
    }

    proptest! {
        /// (a + b) - b == a whenever a + b is representable, over all of i64.
        #[test]
        fn add_then_sub_round_trips(a in any::<i64>(), b in any::<i64>()) {
            match nm(a).checked_add(nm(b)) {
                Ok(sum) => prop_assert_eq!(sum.checked_sub(nm(b)), Ok(nm(a))),
                Err(e) => {
                    prop_assert_eq!(e, LengthError::Overflow);
                    prop_assert!(a.checked_add(b).is_none());
                }
            }
        }

        /// Within half the range the sum always exists, so the round trip always runs.
        #[test]
        fn add_then_sub_round_trips_in_half_range(
            a in (i64::MIN / 2)..=(i64::MAX / 2),
            b in (i64::MIN / 2)..=(i64::MAX / 2),
        ) {
            let sum = nm(a).checked_add(nm(b));
            prop_assert!(sum.is_ok());
            prop_assert_eq!(sum.and_then(|s| s.checked_sub(nm(b))), Ok(nm(a)));
        }

        /// Multiplying by k then dividing by k is exact. Both factors stay within
        /// ±2^31 so the product always fits and every case is checked.
        #[test]
        fn mul_then_div_round_trips(
            a in -(1i64 << 31)..=(1i64 << 31),
            k in -(1i64 << 31)..=(1i64 << 31),
        ) {
            prop_assume!(k != 0);
            let product = nm(a).checked_mul(k);
            prop_assert!(product.is_ok());
            prop_assert_eq!(product.and_then(|p| p.checked_div(k)), Ok(nm(a)));
        }

        /// The quotient is the nearest integer: |q*k - n| <= |k| / 2, ties to even.
        #[test]
        fn div_is_nearest_with_ties_to_even(n in any::<i64>(), k in any::<i64>()) {
            prop_assume!(k != 0 && !(n == i64::MIN && k == -1));
            let q = i128::from(nm(n).checked_div(k).map_err(|e| TestCaseError::fail(e.to_string()))?.nanometers());
            let (n, k) = (i128::from(n), i128::from(k));
            let twice_err = 2 * (q * k - n).abs();
            prop_assert!(twice_err <= k.abs());
            if twice_err == k.abs() {
                prop_assert_eq!(q % 2, 0);
            }
        }
    }
}
