//! Float lint: fails when crates/core sources use floating point.
//!
//! Comments (line, block, nested block and doc comments) are excluded. Three
//! forms are flagged:
//! - the words `f32` and `f64`, including type names, paths, suffixed literals
//!   such as `1.0f64`, and string contents;
//! - any identifier that contains `f32` or `f64` (`as_f64`, `f64_from`);
//! - decimal and exponent float literals (`0.5`, `1e3`, `2.`), lexed as Rust
//!   does, so ranges (`0..5`), tuple fields (`x.0`) and integer literals with
//!   suffixes (`7u8`, `0xf64`) pass.
//!
//! The last two are checked outside comments and outside string and char
//! literals; the scanner tracks those literals so that `//` or `/*` inside
//! them is not mistaken for a comment.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// One forbidden token found in a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub path: PathBuf,
    /// 1-based line number.
    pub line: usize,
    /// 1-based column, counted in characters.
    pub column: usize,
    /// `f32`, `f64` or [`FLOAT_LITERAL`].
    pub token: &'static str,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = if self.token == FLOAT_LITERAL {
            "a float literal".to_string()
        } else {
            format!("`{}`", self.token)
        };
        write!(
            f,
            "{}:{}:{}: {what} is forbidden in crates/core (spec 5.5)",
            self.path.display(),
            self.line,
            self.column,
        )
    }
}

const TOKENS: [&str; 2] = ["f32", "f64"];

/// Token reported for a decimal or exponent float literal.
pub const FLOAT_LITERAL: &str = "float literal";

/// What a source character belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Region {
    Code,
    Comment,
    /// Inside a string, raw string or char literal, quotes included.
    Literal,
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Positions (line, column, token) of forbidden forms outside comments,
/// in source order.
pub fn scan_source(src: &str) -> Vec<(usize, usize, &'static str)> {
    let chars: Vec<char> = src.chars().collect();
    let region = region_mask(&chars);
    let mut hits: Vec<(usize, &'static str)> = (0..chars.len())
        .filter_map(|i| token_at(&chars, &region, i).map(|t| (i, t)))
        .collect();
    hits.extend(
        float_literals(&chars, &region)
            .into_iter()
            .map(|i| (i, FLOAT_LITERAL)),
    );
    hits.sort();
    let mut hits = hits.into_iter().peekable();
    let mut out = Vec::new();
    let (mut line, mut column) = (1, 1);
    for (i, &c) in chars.iter().enumerate() {
        while let Some((_, token)) = hits.next_if(|&(at, _)| at == i) {
            out.push((line, column, token));
        }
        if c == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    out
}

/// Returns the forbidden `f32` / `f64` token starting at `i`, if any.
fn token_at(chars: &[char], region: &[Region], i: usize) -> Option<&'static str> {
    let token = TOKENS.into_iter().find(|t| {
        let len = t.chars().count();
        i + len <= chars.len()
            && t.chars().zip(&chars[i..i + len]).all(|(a, &b)| a == b)
            && region[i..i + len].iter().all(|&r| r != Region::Comment)
    })?;
    let end = i + token.len();
    let at_end = end >= chars.len() || !is_ident_char(chars[end]);
    let mut start = i;
    while start > 0 && is_ident_char(chars[start - 1]) {
        start -= 1;
    }
    if start == i && at_end {
        return Some(token);
    }
    let first = chars[start];
    if first.is_ascii_digit() {
        // A numeric literal: flag the suffix of `1.0f64`, not a hex integer
        // (`0xf64`).
        let hex = first == '0' && matches!(chars.get(start + 1), Some('x' | 'X'));
        return (at_end && !hex).then_some(token);
    }
    // Part of an identifier (`as_f64`, `f64_from`): flagged in code only.
    (region[i] == Region::Code).then_some(token)
}

/// Start indices of decimal and exponent float literals in code.
fn float_literals(chars: &[char], region: &[Region]) -> Vec<usize> {
    let n = chars.len();
    let at = |i: usize| chars.get(i).copied();
    let digits_end = |mut j: usize| {
        while j < n && (chars[j].is_ascii_digit() || chars[j] == '_') {
            j += 1;
        }
        j
    };
    let word_end = |mut j: usize| {
        while j < n && is_ident_char(chars[j]) {
            j += 1;
        }
        j
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let starts_number = region[i] == Region::Code
            && chars[i].is_ascii_digit()
            && (i == 0 || !is_ident_char(chars[i - 1]));
        if !starts_number {
            i += 1;
            continue;
        }
        // `x.0` is a tuple field; `..2.5` is a range bound.
        let tuple_field = i > 0 && chars[i - 1] == '.' && (i < 2 || chars[i - 2] != '.');
        let radix = chars[i] == '0' && matches!(at(i + 1), Some('x' | 'o' | 'b'));
        if tuple_field || radix {
            i = word_end(i);
            continue;
        }
        let mut j = digits_end(i);
        let mut float = false;
        // As in rustc: `2.` is a float unless a `.`, `_` or identifier follows
        // (`0..5`, `2.max(3)`).
        let dot_ends_number = match at(j + 1) {
            Some(c) => c == '.' || (is_ident_char(c) && !c.is_ascii_digit()),
            None => false,
        };
        if at(j) == Some('.') && !dot_ends_number {
            float = true;
            j = digits_end(j + 1);
        }
        if matches!(at(j), Some('e' | 'E')) {
            let k = if matches!(at(j + 1), Some('+' | '-')) {
                j + 2
            } else {
                j + 1
            };
            if matches!(at(k), Some(c) if c.is_ascii_digit() || c == '_') {
                float = true;
                j = digits_end(k);
            }
        }
        if float {
            out.push(i);
        }
        i = word_end(j);
    }
    out
}

/// Classifies each character as code, comment or literal.
fn region_mask(chars: &[char]) -> Vec<Region> {
    let n = chars.len();
    let at = |i: usize| chars.get(i).copied();
    let mut region = vec![Region::Code; n];
    let mut i = 0;
    while i < n {
        let c = chars[i];
        let start = i;
        let kind = if c == '/' && at(i + 1) == Some('/') {
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            Region::Comment
        } else if c == '/' && at(i + 1) == Some('*') {
            let mut depth = 0usize;
            while i < n {
                if chars[i] == '/' && at(i + 1) == Some('*') {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && at(i + 1) == Some('/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            Region::Comment
        } else if c == '"' {
            i = skip_string(chars, i + 1);
            Region::Literal
        } else if let Some(hashes) = raw_string_hashes(chars, i) {
            i = skip_raw_string(chars, i + 1 + hashes + 1, hashes);
            Region::Literal
        } else if c == '\'' {
            i = skip_char_or_lifetime(chars, i);
            // A lifetime's quote alone is code; its name stays code too.
            if i == start + 1 {
                Region::Code
            } else {
                Region::Literal
            }
        } else {
            i += 1;
            Region::Code
        };
        for slot in &mut region[start..i.min(n)] {
            *slot = kind;
        }
    }
    region
}

/// Index just past the closing quote of a string whose body starts at `i`.
fn skip_string(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '"' => return i + 1,
            _ => i += 1,
        }
    }
    i
}

/// For `r"`, `r#"`, `br"` and `cr"` starting at the `r` at `i`, the number of hashes.
fn raw_string_hashes(chars: &[char], i: usize) -> Option<usize> {
    if chars[i] != 'r' {
        return None;
    }
    let prefix_ok = match i.checked_sub(1).map(|p| chars[p]) {
        None => true,
        Some('b' | 'c') => i < 2 || !is_ident_char(chars[i - 2]),
        Some(p) => !is_ident_char(p),
    };
    if !prefix_ok {
        return None;
    }
    let hashes = chars[i + 1..].iter().take_while(|&&c| c == '#').count();
    (chars.get(i + 1 + hashes) == Some(&'"')).then_some(hashes)
}

/// Index just past a raw string's closing `"` plus `hashes` hashes; body starts at `i`.
fn skip_raw_string(chars: &[char], mut i: usize, hashes: usize) -> usize {
    while i < chars.len() {
        if chars[i] == '"'
            && chars[i + 1..]
                .iter()
                .take(hashes)
                .filter(|&&c| c == '#')
                .count()
                == hashes
        {
            return i + 1 + hashes;
        }
        i += 1;
    }
    i
}

/// Skips a char literal (`'a'`, `'\''`) or just the quote of a lifetime (`'a`).
fn skip_char_or_lifetime(chars: &[char], i: usize) -> usize {
    match chars.get(i + 1) {
        Some('\\') => {
            let mut j = i + 3;
            while j < chars.len() && chars[j] != '\'' {
                j += 1;
            }
            j + 1
        }
        Some(_) if chars.get(i + 2) == Some(&'\'') => i + 3,
        _ => i + 1,
    }
}

/// Scans every `.rs` file under `root`, recursively, in sorted path order.
pub fn scan_dir(root: &Path) -> io::Result<Vec<Finding>> {
    let mut files = Vec::new();
    collect_rust_files(root, &mut files)?;
    files.sort();
    let mut findings = Vec::new();
    for path in files {
        let src = fs::read_to_string(&path)?;
        for (line, column, token) in scan_source(&src) {
            findings.push(Finding {
                path: path.clone(),
                line,
                column,
                token,
            });
        }
    }
    Ok(findings)
}

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            collect_rust_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(src: &str) -> Vec<(usize, usize, &'static str)> {
        scan_source(src)
    }

    #[test]
    fn flags_the_done_when_line() {
        assert_eq!(
            tokens("let x: f64 = 0.0;"),
            vec![(1, 8, "f64"), (1, 14, FLOAT_LITERAL)]
        );
    }

    #[test]
    fn flags_f32_and_suffixed_literals() {
        assert_eq!(
            tokens("fn a(v: f32) {}\nconst B: u8 = 1.0f64 as u8;"),
            vec![(1, 9, "f32"), (2, 15, FLOAT_LITERAL), (2, 18, "f64")]
        );
    }

    #[test]
    fn flags_paths_and_generics() {
        assert_eq!(
            tokens("let v: Vec<f64> = std::f32::consts::PI;"),
            vec![(1, 12, "f64"), (1, 24, "f32")]
        );
    }

    #[test]
    fn ignores_line_and_doc_comments() {
        let src = "// f64 here\n/// doc f32\n//! inner f64\nlet a = 1; // trailing f64\n";
        assert!(tokens(src).is_empty());
    }

    #[test]
    fn ignores_nested_block_comments() {
        let src = "/* outer /* inner f64 */ still comment f32 */ let y: f64 = 0.0;";
        assert_eq!(tokens(src), vec![(1, 54, "f64"), (1, 60, FLOAT_LITERAL)]);
    }

    #[test]
    fn multi_line_block_comment_keeps_line_numbers() {
        let src = "/*\n f64\n*/\nlet z: f32;";
        assert_eq!(tokens(src), vec![(4, 8, "f32")]);
    }

    #[test]
    fn comment_markers_inside_strings_are_not_comments() {
        let src = "let s = \"http://x\"; let t: f64 = 0.0;";
        assert_eq!(tokens(src), vec![(1, 28, "f64"), (1, 34, FLOAT_LITERAL)]);
    }

    #[test]
    fn escaped_quote_does_not_end_string() {
        let src = "let s = \"a\\\"//\"; f64";
        assert_eq!(tokens(src), vec![(1, 18, "f64")]);
    }

    #[test]
    fn raw_strings_are_skipped_for_comment_detection() {
        let src = "let s = r#\"a \"// b\"#; f32";
        assert_eq!(tokens(src), vec![(1, 23, "f32")]);
    }

    #[test]
    fn string_contents_still_count() {
        assert_eq!(tokens("let s = \"f64\";"), vec![(1, 10, "f64")]);
    }

    #[test]
    fn char_literals_and_lifetimes() {
        let src = "fn f<'a>(x: &'a str) -> char { let _ = '\"'; let _ = '/'; '\\'' } // f64";
        assert!(tokens(src).is_empty());
    }

    #[test]
    fn identifiers_containing_the_text_are_flagged() {
        assert_eq!(
            tokens("v.as_f64(); n.to_f64(); f64_from(1); let af32 = 2; let f640 = 3;"),
            vec![
                (1, 6, "f64"),
                (1, 18, "f64"),
                (1, 25, "f64"),
                (1, 43, "f32"),
                (1, 56, "f64"),
            ]
        );
    }

    #[test]
    fn hex_integers_containing_the_text_pass() {
        assert!(tokens("let a = 0xf64; let b = 0xF32_u32; let c = 0x1e3;").is_empty());
    }

    #[test]
    fn flags_decimal_and_exponent_literals() {
        assert_eq!(
            tokens("let a = 0.5; let b = 1e3; let c = 2.; let d = 1_000.25E-3;"),
            vec![
                (1, 9, FLOAT_LITERAL),
                (1, 22, FLOAT_LITERAL),
                (1, 35, FLOAT_LITERAL),
                (1, 47, FLOAT_LITERAL),
            ]
        );
    }

    #[test]
    fn flags_float_literals_at_line_end_and_in_ranges() {
        assert_eq!(
            tokens("let a = 2.\nfor x in 1.5..2.5 {}"),
            vec![
                (1, 9, FLOAT_LITERAL),
                (2, 10, FLOAT_LITERAL),
                (2, 15, FLOAT_LITERAL),
            ]
        );
    }

    #[test]
    fn ranges_tuple_fields_and_integers_pass() {
        let src = "for i in 0..5 {} let _ = 0..=9; let a = x.0; let b = t.0.1; \
                   let c = 7u8; let d = 1_000i64; let e = 0b1010_u8; let f = 0o17; \
                   let g = 2.max(3); let h = 1.pow(2); let k = 3._x;";
        assert!(tokens(src).is_empty(), "{:?}", tokens(src));
    }

    #[test]
    fn new_forms_in_comments_and_strings_pass() {
        let src = "// 0.5 as_f64 1e3\n/* 2. to_f64 */ let s = \"0.5 as_f64 1e3 2.\"; \
                   let r = r#\"f64_from 1.25\"#; let c = '5';";
        assert!(tokens(src).is_empty(), "{:?}", tokens(src));
    }

    #[test]
    fn identifiers_with_digits_are_not_literals() {
        assert!(tokens("let x1 = a2.b3; let v = e1;").is_empty());
    }

    #[test]
    fn scan_dir_walks_recursively_and_skips_non_rust_files() -> io::Result<()> {
        let root = std::env::temp_dir().join(format!("xtask-float-lint-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src/nested"))?;
        fs::write(root.join("src/lib.rs"), "// f64\npub fn ok() {}\n")?;
        fs::write(root.join("src/nested/bad.rs"), "\nlet x: f64 = 0;\n")?;
        fs::write(root.join("notes.md"), "f64 in markdown is fine\n")?;
        let findings = scan_dir(&root);
        fs::remove_dir_all(&root)?;
        let findings = findings?;
        assert_eq!(findings.len(), 1);
        assert!(findings[0].path.ends_with("src/nested/bad.rs"));
        assert_eq!((findings[0].line, findings[0].column), (2, 8));
        Ok(())
    }

    #[test]
    fn core_crate_is_clean() -> io::Result<()> {
        let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/core");
        let findings = scan_dir(&core)?;
        assert!(findings.is_empty(), "{findings:?}");
        Ok(())
    }
}
