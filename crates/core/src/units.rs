//! Imperial parsing, formatting and echo (spec 5.2, 5.3; Q-1 to Q-3).
//!
//! Accepted input (case-insensitive, spaces between parts optional):
//!
//! - an optional leading `-`;
//! - a feet part: a number followed by `'`, `′`, `ft`, `foot` or `feet`;
//! - an inch part: a number followed by `"`, `″`, `in`, `inch` or `inches`.
//!   After a feet part the inch unit may be left off (`10'-4`), and an
//!   optional `-` may separate the two parts;
//! - a bare number, read in the project unit;
//! - in the asset-specification context only, a number followed by `mm`,
//!   `cm` or `m`.
//!
//! A number is whole (`4`), decimal (`4.5`, `.5`), a fraction (`1/2`) or a
//! whole number and a fraction separated by spaces (`4 1/2`).
//!
//! Values are computed as exact rationals. In the user context a value off
//! the 1/16 in grid is rejected with the nearest grid value. In the
//! asset-specification context a value finer than a nanometer is rounded
//! half to even and the rounding is reported (Q-1).

use std::fmt;

use crate::length::Length;

/// 1/16 in, the display and user-input grid, in nanometers.
pub const GRID_NM: i64 = 1_587_500;

/// Where the text being parsed came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputContext {
    /// Typed or spoken by the user: imperial only, on the 1/16 in grid.
    User,
    /// A manufacturer or CAD specification: metric allowed, any precision.
    AssetSpec,
}

/// The unit of a bare number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProjectUnit {
    #[default]
    Inches,
    Feet,
}

/// The stored value minus the exact input, in nanometers, as a reduced
/// fraction. Its magnitude is at most one half.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputRounding {
    numerator: i128,
    denominator: i128,
}

impl InputRounding {
    /// Numerator of the reduced fraction (the sign of the rounding).
    pub const fn numerator(self) -> i128 {
        self.numerator
    }

    /// Denominator of the reduced fraction, always positive.
    pub const fn denominator(self) -> i128 {
        self.denominator
    }
}

/// A parsed length and, in the asset context, how far input was rounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedLength {
    pub length: Length,
    pub input_rounding: Option<InputRounding>,
}

/// Why text is not a length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// The text is empty or blank.
    Empty,
    /// The text does not follow the grammar; `at` is the character index.
    Syntax { at: usize },
    /// A fraction has a zero denominator.
    ZeroDenominator,
    /// Metric units are allowed only in asset specifications.
    MetricNotAllowed,
    /// The value is outside the representable range.
    OutOfRange,
    /// User input finer than 1/16 in; `nearest` is the nearest grid value.
    OffGrid { nearest: Length },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Empty => f.write_str("no length given"),
            ParseError::Syntax { at } => {
                write!(f, "not a length: unexpected input at character {at}")
            }
            ParseError::ZeroDenominator => f.write_str("fraction has a zero denominator"),
            ParseError::MetricNotAllowed => {
                f.write_str("metric units are accepted only in asset specifications")
            }
            ParseError::OutOfRange => f.write_str("length out of range"),
            ParseError::OffGrid { nearest } => write!(
                f,
                "input is finer than 1/16 in; nearest grid value is {}",
                format_length(*nearest)
            ),
        }
    }
}

impl std::error::Error for ParseError {}

const NM_PER_INCH: i128 = 25_400_000;
const NM_PER_FOOT: i128 = 12 * NM_PER_INCH;
const GRID: i128 = GRID_NM as i128;
const SIXTEENTHS_PER_FOOT: u128 = 192;

/// An exact rational number with a positive denominator, kept reduced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ratio {
    n: i128,
    d: i128,
}

fn gcd(a: u128, b: u128) -> u128 {
    let (mut a, mut b) = (a, b);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Ratio {
    fn new(n: i128, d: i128) -> Result<Self, ParseError> {
        if d == 0 {
            return Err(ParseError::ZeroDenominator);
        }
        let (n, d) = if d < 0 {
            (n.checked_neg(), d.checked_neg())
        } else {
            (Some(n), Some(d))
        };
        let (n, d) = n.zip(d).ok_or(ParseError::OutOfRange)?;
        // g divides d > 0, so 1 <= g <= d and it fits in i128.
        let g = i128::try_from(gcd(n.unsigned_abs(), d.unsigned_abs()))
            .map_err(|_| ParseError::OutOfRange)?;
        Ok(Ratio { n: n / g, d: d / g })
    }

    fn int(n: i128) -> Self {
        Ratio { n, d: 1 }
    }

    fn add(self, o: Ratio) -> Result<Self, ParseError> {
        let n = self
            .n
            .checked_mul(o.d)
            .zip(o.n.checked_mul(self.d))
            .and_then(|(a, b)| a.checked_add(b))
            .ok_or(ParseError::OutOfRange)?;
        let d = self.d.checked_mul(o.d).ok_or(ParseError::OutOfRange)?;
        Ratio::new(n, d)
    }

    fn mul_int(self, k: i128) -> Result<Self, ParseError> {
        let n = self.n.checked_mul(k).ok_or(ParseError::OutOfRange)?;
        Ratio::new(n, self.d)
    }

    fn neg(self) -> Result<Self, ParseError> {
        let n = self.n.checked_neg().ok_or(ParseError::OutOfRange)?;
        Ratio::new(n, self.d)
    }
}

/// `n / d` rounded half to even; `d` must be positive.
fn round_half_even(n: i128, d: i128) -> i128 {
    let q = n / d;
    let r = n % d;
    // |r| < d, so 2|r| fits in u128.
    let twice_r = 2 * r.unsigned_abs();
    let d_abs = d.unsigned_abs();
    if twice_r > d_abs || (twice_r == d_abs && q % 2 != 0) {
        q + r.signum()
    } else {
        q
    }
}

fn to_length(v: i128) -> Result<Length, ParseError> {
    i64::try_from(v)
        .map(Length::from_nanometers)
        .map_err(|_| ParseError::OutOfRange)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Feet,
    Inches,
    Metric(i128),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumberKind {
    Whole,
    Decimal,
    Fraction,
}

/// Unit marks and words, longest first where one is a prefix of another.
const UNITS: &[(&str, Unit)] = &[
    ("'", Unit::Feet),
    ("′", Unit::Feet),
    ("feet", Unit::Feet),
    ("foot", Unit::Feet),
    ("ft", Unit::Feet),
    ("\"", Unit::Inches),
    ("″", Unit::Inches),
    ("inches", Unit::Inches),
    ("inch", Unit::Inches),
    ("in", Unit::Inches),
    ("mm", Unit::Metric(1_000_000)),
    ("cm", Unit::Metric(10_000_000)),
    ("m", Unit::Metric(1_000_000_000)),
];

struct Parser {
    chars: Vec<char>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.i).copied()
    }

    fn at_end(&self) -> bool {
        self.i >= self.chars.len()
    }

    fn syntax(&self) -> ParseError {
        ParseError::Syntax { at: self.i }
    }

    /// Skips whitespace; true when any was skipped.
    fn skip_ws(&mut self) -> bool {
        let start = self.i;
        while self.peek().is_some_and(char::is_whitespace) {
            self.i += 1;
        }
        self.i > start
    }

    fn eat(&mut self, c: char) -> bool {
        let found = self.peek() == Some(c);
        if found {
            self.i += 1;
        }
        found
    }

    /// Decimal digits as an integer, and how many there were.
    fn digits(&mut self) -> Result<(i128, u32), ParseError> {
        let (mut v, mut count) = (0i128, 0u32);
        while let Some(d) = self.peek().and_then(|c| c.to_digit(10)) {
            v = v
                .checked_mul(10)
                .and_then(|v| v.checked_add(i128::from(d)))
                .ok_or(ParseError::OutOfRange)?;
            count = count.saturating_add(1);
            self.i += 1;
        }
        Ok((v, count))
    }

    /// One number: `4`, `4.5`, `.5` or `1/2`.
    fn number(&mut self) -> Result<(Ratio, NumberKind), ParseError> {
        let (whole, count) = self.digits()?;
        if count > 0 && self.eat('/') {
            let (den, den_count) = self.digits()?;
            if den_count == 0 {
                return Err(self.syntax());
            }
            return Ok((Ratio::new(whole, den)?, NumberKind::Fraction));
        }
        if self.eat('.') {
            let (frac, places) = self.digits()?;
            if places == 0 {
                return Err(self.syntax());
            }
            let scale = 10i128.checked_pow(places).ok_or(ParseError::OutOfRange)?;
            let n = whole
                .checked_mul(scale)
                .and_then(|w| w.checked_add(frac))
                .ok_or(ParseError::OutOfRange)?;
            return Ok((Ratio::new(n, scale)?, NumberKind::Decimal));
        }
        if count == 0 {
            return Err(self.syntax());
        }
        Ok((Ratio::int(whole), NumberKind::Whole))
    }

    /// A number, or a whole number followed by spaces and a fraction.
    fn mixed(&mut self) -> Result<Ratio, ParseError> {
        let (value, kind) = self.number()?;
        if kind != NumberKind::Whole {
            return Ok(value);
        }
        let save = self.i;
        if self.skip_ws() && self.peek().is_some_and(|c| c.is_ascii_digit()) {
            let (frac, frac_kind) = self.number()?;
            if frac_kind == NumberKind::Fraction {
                return value.add(frac);
            }
        }
        self.i = save;
        Ok(value)
    }

    /// The unit at the cursor, consumed if present.
    fn unit(&mut self) -> Option<Unit> {
        for &(word, unit) in UNITS {
            let len = word.chars().count();
            let matches = self
                .chars
                .get(self.i..self.i + len)
                .is_some_and(|s| s.iter().copied().eq(word.chars()));
            if !matches {
                continue;
            }
            let is_word = word.chars().all(char::is_alphabetic);
            let letter_follows = self
                .chars
                .get(self.i + len)
                .is_some_and(|c| c.is_alphabetic());
            if is_word && letter_follows {
                continue;
            }
            self.i += len;
            return Some(unit);
        }
        None
    }
}

/// The exact value of `text` in nanometers.
fn parse_exact(
    text: &str,
    context: InputContext,
    project_unit: ProjectUnit,
) -> Result<Ratio, ParseError> {
    let mut p = Parser {
        chars: text.chars().map(|c| c.to_ascii_lowercase()).collect(),
        i: 0,
    };
    p.skip_ws();
    if p.at_end() {
        return Err(ParseError::Empty);
    }
    let negative = p.eat('-');
    p.skip_ws();
    let first = p.mixed()?;
    p.skip_ws();
    let total = match p.unit() {
        Some(Unit::Feet) => {
            let feet = first.mul_int(NM_PER_FOOT)?;
            p.skip_ws();
            let dashed = p.eat('-');
            p.skip_ws();
            if p.at_end() && !dashed {
                feet
            } else {
                let inches = p.mixed()?;
                p.skip_ws();
                let unit_at = p.i;
                match p.unit() {
                    None | Some(Unit::Inches) => {}
                    Some(_) => return Err(ParseError::Syntax { at: unit_at }),
                }
                feet.add(inches.mul_int(NM_PER_INCH)?)?
            }
        }
        Some(Unit::Inches) => first.mul_int(NM_PER_INCH)?,
        Some(Unit::Metric(nm)) => {
            if context == InputContext::User {
                return Err(ParseError::MetricNotAllowed);
            }
            first.mul_int(nm)?
        }
        None => first.mul_int(match project_unit {
            ProjectUnit::Inches => NM_PER_INCH,
            ProjectUnit::Feet => NM_PER_FOOT,
        })?,
    };
    p.skip_ws();
    if !p.at_end() {
        return Err(p.syntax());
    }
    if negative { total.neg() } else { Ok(total) }
}

/// Parses a length typed in `context`; bare numbers are in `unit`.
pub fn parse_length(
    text: &str,
    context: InputContext,
    unit: ProjectUnit,
) -> Result<ParsedLength, ParseError> {
    let value = parse_exact(text, context, unit)?;
    match context {
        InputContext::User => {
            let grid_den = value.d.checked_mul(GRID).ok_or(ParseError::OutOfRange)?;
            if value.n % grid_den != 0 {
                let g = round_half_even(value.n, grid_den);
                let nearest = g.checked_mul(GRID).ok_or(ParseError::OutOfRange)?;
                return Err(ParseError::OffGrid {
                    nearest: to_length(nearest)?,
                });
            }
            Ok(ParsedLength {
                length: to_length(value.n)?,
                input_rounding: None,
            })
        }
        InputContext::AssetSpec => {
            if value.d == 1 {
                return Ok(ParsedLength {
                    length: to_length(value.n)?,
                    input_rounding: None,
                });
            }
            let stored = round_half_even(value.n, value.d);
            let residual = Ratio::int(stored).add(value.neg()?)?;
            Ok(ParsedLength {
                length: to_length(stored)?,
                input_rounding: Some(InputRounding {
                    numerator: residual.n,
                    denominator: residual.d,
                }),
            })
        }
    }
}

/// Canonical feet-inches-fraction text at 1/16 in, `~` first when off grid (Q-2).
pub fn format_length(length: Length) -> String {
    let v = i128::from(length.nanometers());
    let g = round_half_even(v, GRID);
    let tilde = if v % GRID != 0 { "~" } else { "" };
    let sign = if g < 0 { "-" } else { "" };
    let m = g.unsigned_abs();
    let feet = m / SIXTEENTHS_PER_FOOT;
    let rest = m % SIXTEENTHS_PER_FOOT;
    let inches = rest / 16;
    let sixteenths = rest % 16;
    let fraction = if sixteenths == 0 {
        String::new()
    } else {
        let k = gcd(sixteenths, 16);
        format!(" {}/{}", sixteenths / k, 16 / k)
    };
    format!("{tilde}{sign}{feet}'-{inches}{fraction}\"")
}

/// `whole.frac` with `frac` zero-padded to `places` digits and trailing zeros removed.
fn decimal(whole: u128, frac: u128, places: usize) -> String {
    let digits = format!("{frac:0places$}");
    let digits = digits.trim_end_matches('0');
    if digits.is_empty() {
        format!("{whole}")
    } else {
        format!("{whole}.{digits}")
    }
}

/// The exact stored value in millimeters: `12`, `-0.198438`.
fn exact_millimeters(length: Length) -> String {
    let v = length.nanometers();
    let sign = if v < 0 { "-" } else { "" };
    let m = u128::from(v.unsigned_abs());
    format!("{sign}{}", decimal(m / 1_000_000, m % 1_000_000, 6))
}

/// The size of an input rounding in nanometers to three decimals, `~` first when inexact.
fn rounding_nanometers(r: InputRounding) -> String {
    let n = r.numerator.unsigned_abs();
    let d = r.denominator.unsigned_abs();
    // |n / d| <= 1/2, so n < d and n * 1000 stays far below u128::MAX / 2.
    let scaled = n.saturating_mul(1000);
    let q = scaled / d;
    let twice_r = 2 * (scaled % d);
    let exact = twice_r == 0;
    let thousandths = if twice_r > d || (twice_r == d && !q.is_multiple_of(2)) {
        q + 1
    } else {
        q
    };
    let number = decimal(thousandths / 1000, thousandths % 1000, 3);
    if exact { number } else { format!("~{number}") }
}

/// The confirmation text for a parsed value (Q-3).
pub fn echo_length(parsed: &ParsedLength) -> String {
    let canonical = format_length(parsed.length);
    let on_grid = parsed.length.nanometers() % GRID_NM == 0;
    if on_grid && parsed.input_rounding.is_none() {
        return canonical;
    }
    let mut notes = vec![format!("exact {} mm", exact_millimeters(parsed.length))];
    if let Some(r) = parsed.input_rounding {
        notes.push(format!("rounded {} nm from input", rounding_nanometers(r)));
    }
    format!("{canonical} ({})", notes.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const IN: i64 = 25_400_000;
    const FT: i64 = 12 * IN;
    const SIXTEENTH: i64 = IN / 16;

    fn nm(v: i64) -> Length {
        Length::from_nanometers(v)
    }

    fn exact(v: i64) -> Result<ParsedLength, ParseError> {
        Ok(ParsedLength {
            length: nm(v),
            input_rounding: None,
        })
    }

    fn user(text: &str) -> Result<ParsedLength, ParseError> {
        parse_length(text, InputContext::User, ProjectUnit::Inches)
    }

    fn asset(text: &str) -> Result<ParsedLength, ParseError> {
        parse_length(text, InputContext::AssetSpec, ProjectUnit::Inches)
    }

    #[test]
    fn grid_is_one_sixteenth_inch() {
        assert_eq!(GRID_NM, SIXTEENTH);
    }

    /// (input, expected nm) in the user context with inches as project unit.
    const USER_FORMS: &[(&str, i64)] = &[
        // The three examples from spec 5.2 and the task.
        ("10'-4 1/2\"", 10 * FT + 4 * IN + 8 * SIXTEENTH),
        ("10 ft 4.5 in", 10 * FT + 4 * IN + 8 * SIXTEENTH),
        ("124.5in", 124 * IN + 8 * SIXTEENTH),
        // Bare numbers in the project unit (inches).
        ("124.5", 124 * IN + 8 * SIXTEENTH),
        ("124", 124 * IN),
        ("4 1/2", 4 * IN + 8 * SIXTEENTH),
        ("3/8", 6 * SIXTEENTH),
        // Feet only, inches only.
        ("10'", 10 * FT),
        ("10 ft", 10 * FT),
        ("10 feet", 10 * FT),
        ("1 foot", FT),
        ("10.5'", 10 * FT + 6 * IN),
        ("4\"", 4 * IN),
        ("4 in", 4 * IN),
        ("4 inch", 4 * IN),
        ("4 inches", 4 * IN),
        ("1/2\"", 8 * SIXTEENTH),
        (".5\"", 8 * SIXTEENTH),
        ("4 15/16\"", 4 * IN + 15 * SIXTEENTH),
        // Feet and inches with each separator style.
        ("10' 4\"", 10 * FT + 4 * IN),
        ("10'4\"", 10 * FT + 4 * IN),
        ("10' - 4\"", 10 * FT + 4 * IN),
        ("10'-4", 10 * FT + 4 * IN),
        ("10 ft 4", 10 * FT + 4 * IN),
        ("10 feet 4 inches", 10 * FT + 4 * IN),
        ("10′-4 1/2″", 10 * FT + 4 * IN + 8 * SIXTEENTH),
        // Case, spacing, sign and zero.
        ("10 FT 4 IN", 10 * FT + 4 * IN),
        ("  10'-4\"  ", 10 * FT + 4 * IN),
        ("-10'-4 1/2\"", -(10 * FT + 4 * IN + 8 * SIXTEENTH)),
        ("- 4\"", -4 * IN),
        ("0'-0\"", 0),
        ("0", 0),
        // Canonical output form parses back.
        ("0'-0 1/16\"", SIXTEENTH),
    ];

    #[test]
    fn user_forms_table() {
        for &(text, expected) in USER_FORMS {
            assert_eq!(user(text), exact(expected), "{text:?}");
        }
    }

    #[test]
    fn bare_number_uses_the_project_unit() {
        let feet = |t| parse_length(t, InputContext::User, ProjectUnit::Feet);
        assert_eq!(feet("10"), exact(10 * FT));
        assert_eq!(feet("10.5"), exact(10 * FT + 6 * IN));
        // Explicit units ignore the project unit.
        assert_eq!(feet("10\""), exact(10 * IN));
        assert_eq!(ProjectUnit::default(), ProjectUnit::Inches);
    }

    #[test]
    fn metric_only_in_asset_context() {
        for text in ["12 mm", "1.5 cm", "2 m", "12mm"] {
            assert_eq!(user(text), Err(ParseError::MetricNotAllowed), "{text:?}");
        }
        assert_eq!(asset("12 mm"), exact(12_000_000));
        assert_eq!(asset("12mm"), exact(12_000_000));
        assert_eq!(asset("1.5 cm"), exact(15_000_000));
        assert_eq!(asset("2 m"), exact(2_000_000_000));
        assert_eq!(asset("25.4 MM"), exact(IN));
        assert_eq!(asset("0.000001 mm"), exact(1));
    }

    #[test]
    fn asset_context_accepts_imperial_finer_than_the_grid() {
        assert_eq!(
            asset("10'-4 1/2\""),
            exact(10 * FT + 4 * IN + 8 * SIXTEENTH)
        );
        assert_eq!(asset("1/64\""), exact(396_875));
        assert_eq!(asset("1/100 in"), exact(254_000));
        assert_eq!(asset("0.001 in"), exact(25_400));
    }

    #[test]
    fn asset_context_rounds_to_the_nanometer_and_reports_it() {
        // 1/128 in = 198,437.5 nm: a tie, rounded to the even 198,438.
        let r = asset("1/128 in");
        assert_eq!(
            r,
            Ok(ParsedLength {
                length: nm(198_438),
                input_rounding: Some(InputRounding {
                    numerator: 1,
                    denominator: 2
                }),
            })
        );
        // 1/3 in = 8,466,666 2/3 nm, rounded up by 1/3 nm.
        assert_eq!(
            asset("1/3\""),
            Ok(ParsedLength {
                length: nm(8_466_667),
                input_rounding: Some(InputRounding {
                    numerator: 1,
                    denominator: 3
                }),
            })
        );
        // Negative: -1/3 in rounds to -8,466,667, i.e. down by 1/3 nm.
        assert_eq!(
            asset("-1/3\"").map(|p| p.input_rounding),
            Ok(Some(InputRounding {
                numerator: -1,
                denominator: 3
            }))
        );
        // 0.0000005 mm = 0.5 nm: a tie, rounded to the even 0.
        assert_eq!(
            asset("0.0000005 mm"),
            Ok(ParsedLength {
                length: nm(0),
                input_rounding: Some(InputRounding {
                    numerator: -1,
                    denominator: 2
                }),
            })
        );
    }

    /// (input, nearest grid value in sixteenths) rejected in the user context.
    const OFF_GRID: &[(&str, i64)] = &[
        // 4 1/32 is a tie between 64 and 65 sixteenths: even wins.
        ("4 1/32\"", 64),
        ("4 3/32\"", 66),
        ("0.1\"", 2),
        ("1/64\"", 0),
        ("4 1/3\"", 69),
        ("10'-4 1/64\"", 10 * 192 + 64),
        ("-4 1/32\"", -64),
        ("-4 3/32\"", -66),
        ("10.01 ft", 1922),
    ];

    #[test]
    fn user_context_rejects_input_off_the_grid() {
        for &(text, sixteenths) in OFF_GRID {
            assert_eq!(
                user(text),
                Err(ParseError::OffGrid {
                    nearest: nm(sixteenths * SIXTEENTH)
                }),
                "{text:?}"
            );
        }
    }

    #[test]
    fn off_grid_error_message_shows_the_nearest_value() {
        assert_eq!(
            user("4 3/32\"").map_err(|e| e.to_string()),
            Err("input is finer than 1/16 in; nearest grid value is 0'-4 1/8\"".to_string())
        );
    }

    #[test]
    fn malformed_input_is_an_error() {
        let cases: &[(&str, ParseError)] = &[
            ("", ParseError::Empty),
            ("   ", ParseError::Empty),
            ("abc", ParseError::Syntax { at: 0 }),
            ("10 4", ParseError::Syntax { at: 3 }),
            ("4.5 1/2", ParseError::Syntax { at: 4 }),
            ("10 ft 4 ft", ParseError::Syntax { at: 8 }),
            ("4 in 2", ParseError::Syntax { at: 5 }),
            ("5 inx", ParseError::Syntax { at: 2 }),
            ("10'-", ParseError::Syntax { at: 4 }),
            ("--4", ParseError::Syntax { at: 1 }),
            ("4.", ParseError::Syntax { at: 2 }),
            ("1/", ParseError::Syntax { at: 2 }),
            ("1/0", ParseError::ZeroDenominator),
            ("10 ft 4 mm", ParseError::Syntax { at: 8 }),
            ("4\"\"", ParseError::Syntax { at: 2 }),
        ];
        for (text, expected) in cases {
            assert_eq!(user(text), Err(*expected), "{text:?}");
        }
    }

    #[test]
    fn out_of_range_is_an_error_never_a_wrap() {
        assert_eq!(user("1000000000000 ft"), Err(ParseError::OutOfRange));
        assert_eq!(
            user("99999999999999999999999999999999999999999"),
            Err(ParseError::OutOfRange)
        );
        assert_eq!(
            asset("0.00000000000000000000000000000000000000001 mm"),
            Err(ParseError::OutOfRange)
        );
    }

    /// (nm, canonical text).
    const FORMAT_TABLE: &[(i64, &str)] = &[
        (0, "0'-0\""),
        (SIXTEENTH, "0'-0 1/16\""),
        (8 * SIXTEENTH, "0'-0 1/2\""),
        (6 * SIXTEENTH, "0'-0 3/8\""),
        (15 * SIXTEENTH, "0'-0 15/16\""),
        (4 * IN + 8 * SIXTEENTH, "0'-4 1/2\""),
        (FT, "1'-0\""),
        (10 * FT, "10'-0\""),
        (10 * FT + 4 * IN + 8 * SIXTEENTH, "10'-4 1/2\""),
        (11 * IN + 15 * SIXTEENTH, "0'-11 15/16\""),
        (-(10 * FT + 4 * IN + 8 * SIXTEENTH), "-10'-4 1/2\""),
        (-SIXTEENTH, "-0'-0 1/16\""),
        // Off grid: nearest 1/16 in, ties to even, `~` before the sign.
        (1, "~0'-0\""),
        (-1, "~0'-0\""),
        (SIXTEENTH / 2, "~0'-0\""),
        (3 * SIXTEENTH / 2, "~0'-0 1/8\""),
        (12_000_000, "~0'-0 1/2\""),
        (-SIXTEENTH - 1, "~-0'-0 1/16\""),
        (10 * FT + 4 * IN + 8 * SIXTEENTH + 1, "~10'-4 1/2\""),
        // Range edges.
        (i64::MAX, "~30260406945'-0 3/4\""),
        (i64::MIN, "~-30260406945'-0 3/4\""),
    ];

    #[test]
    fn format_table() {
        for &(v, text) in FORMAT_TABLE {
            assert_eq!(format_length(nm(v)), text, "{v} nm");
        }
    }

    #[test]
    fn echo_on_grid_is_the_canonical_text() {
        for text in ["10'-4 1/2\"", "10 ft 4.5 in", "124.5in", "124.5"] {
            let p = user(text).map(|p| echo_length(&p));
            assert_eq!(p, Ok("10'-4 1/2\"".to_string()), "{text:?}");
        }
    }

    #[test]
    fn echo_off_grid_adds_exact_millimeters() {
        let echo = |t| asset(t).map(|p| echo_length(&p));
        assert_eq!(echo("12 mm"), Ok("~0'-0 1/2\" (exact 12 mm)".to_string()));
        assert_eq!(
            echo("-12 mm"),
            Ok("~-0'-0 1/2\" (exact -12 mm)".to_string())
        );
        assert_eq!(echo("25.4 mm"), Ok("0'-1\"".to_string()));
        assert_eq!(echo("1/100 in"), Ok("~0'-0\" (exact 0.254 mm)".to_string()));
        assert_eq!(
            echo("1/128 in"),
            Ok("~0'-0\" (exact 0.198438 mm, rounded 0.5 nm from input)".to_string())
        );
        assert_eq!(
            echo("1/3 in"),
            Ok("~0'-0 5/16\" (exact 8.466667 mm, rounded ~0.333 nm from input)".to_string())
        );
    }

    proptest! {
        /// Every on-grid value prints and parses back to itself.
        #[test]
        fn on_grid_values_round_trip(g in (i64::MIN / GRID_NM)..=(i64::MAX / GRID_NM)) {
            let v = nm(g * GRID_NM);
            let text = format_length(v);
            prop_assert!(!text.starts_with('~'), "{}", text);
            prop_assert_eq!(user(&text), exact(g * GRID_NM), "{}", text);
        }

        /// `~` appears exactly when the stored value is off the grid, and the
        /// displayed value is the nearest grid value.
        #[test]
        fn tilde_marks_off_grid_values(v in any::<i64>()) {
            let text = format_length(nm(v));
            let off_grid = v % GRID_NM != 0;
            prop_assert_eq!(text.starts_with('~'), off_grid, "{}", text);
            let shown = user(text.trim_start_matches('~'));
            let nearest = nm(v).checked_div(GRID_NM).map(|g| g.nanometers() * GRID_NM);
            prop_assert_eq!(shown.map(|p| p.length.nanometers()), nearest.map_err(|_| ParseError::OutOfRange));
        }
    }
}
