//! Float lint: fails when `f32` or `f64` appears in crates/core sources.
//!
//! Comments (line, block, nested block and doc comments) are excluded. Every
//! other occurrence counts, including type names, suffixed literals such as
//! `1.0f64`, and string contents. The scanner tracks string, raw string and
//! char literals only so that `//` or `/*` inside them is not mistaken for a
//! comment.

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
    pub token: &'static str,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: `{}` is forbidden in crates/core (spec 5.5)",
            self.path.display(),
            self.line,
            self.column,
            self.token
        )
    }
}

const TOKENS: [&str; 2] = ["f32", "f64"];

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Positions (line, column, token) of `f32` / `f64` outside comments.
pub fn scan_source(src: &str) -> Vec<(usize, usize, &'static str)> {
    let chars: Vec<char> = src.chars().collect();
    let code = code_mask(&chars);
    let mut out = Vec::new();
    let (mut line, mut column) = (1, 1);
    for i in 0..chars.len() {
        if let Some(token) = token_at(&chars, &code, i) {
            out.push((line, column, token));
        }
        if chars[i] == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    out
}

/// Returns the forbidden token starting at `i`, if any.
fn token_at(chars: &[char], code: &[bool], i: usize) -> Option<&'static str> {
    let token = TOKENS.into_iter().find(|t| {
        let len = t.chars().count();
        i + len <= chars.len()
            && t.chars().zip(&chars[i..i + len]).all(|(a, &b)| a == b)
            && code[i..i + len].iter().all(|&c| c)
    })?;
    let end = i + token.len();
    if end < chars.len() && is_ident_char(chars[end]) {
        return None;
    }
    if i > 0 && is_ident_char(chars[i - 1]) {
        // Part of a longer word: flag only a numeric literal suffix (`1.0f64`),
        // not an identifier (`af64`) or a hex integer (`0xf64`).
        let mut start = i;
        while start > 0 && is_ident_char(chars[start - 1]) {
            start -= 1;
        }
        let first = chars[start];
        let hex = first == '0' && matches!(chars.get(start + 1), Some('x' | 'X'));
        if !first.is_ascii_digit() || hex {
            return None;
        }
    }
    Some(token)
}

/// Marks each character `true` when it is outside every comment.
fn code_mask(chars: &[char]) -> Vec<bool> {
    let n = chars.len();
    let at = |i: usize| chars.get(i).copied();
    let mut code = vec![true; n];
    let mut i = 0;
    while i < n {
        let c = chars[i];
        if c == '/' && at(i + 1) == Some('/') {
            while i < n && chars[i] != '\n' {
                code[i] = false;
                i += 1;
            }
        } else if c == '/' && at(i + 1) == Some('*') {
            let mut depth = 0usize;
            while i < n {
                if chars[i] == '/' && at(i + 1) == Some('*') {
                    depth += 1;
                    code[i] = false;
                    code[i + 1] = false;
                    i += 2;
                } else if chars[i] == '*' && at(i + 1) == Some('/') {
                    depth -= 1;
                    code[i] = false;
                    code[i + 1] = false;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    code[i] = false;
                    i += 1;
                }
            }
        } else if c == '"' {
            i = skip_string(chars, i + 1);
        } else if let Some(hashes) = raw_string_hashes(chars, i) {
            i = skip_raw_string(chars, i + 1 + hashes + 1, hashes);
        } else if c == '\'' {
            i = skip_char_or_lifetime(chars, i);
        } else {
            i += 1;
        }
    }
    code
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
        assert_eq!(tokens("let x: f64 = 0.0;"), vec![(1, 8, "f64")]);
    }

    #[test]
    fn flags_f32_and_suffixed_literals() {
        assert_eq!(
            tokens("fn a(v: f32) {}\nconst B: u8 = 1.0f64 as u8;"),
            vec![(1, 9, "f32"), (2, 18, "f64")]
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
        assert_eq!(tokens(src), vec![(1, 54, "f64")]);
    }

    #[test]
    fn multi_line_block_comment_keeps_line_numbers() {
        let src = "/*\n f64\n*/\nlet z: f32;";
        assert_eq!(tokens(src), vec![(4, 8, "f32")]);
    }

    #[test]
    fn comment_markers_inside_strings_are_not_comments() {
        let src = "let s = \"http://x\"; let t: f64 = 0.0;";
        assert_eq!(tokens(src), vec![(1, 28, "f64")]);
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
    fn identifiers_containing_the_text_are_not_flagged() {
        assert!(tokens("let af64 = 1; let f64x = 2; let my_f32 = 3; let f640 = 4;").is_empty());
    }

    #[test]
    fn scan_dir_walks_recursively_and_skips_non_rust_files() -> io::Result<()> {
        let root = std::env::temp_dir().join(format!("xtask-float-lint-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src/nested"))?;
        fs::write(root.join("src/lib.rs"), "// f64\npub fn ok() {}\n")?;
        fs::write(root.join("src/nested/bad.rs"), "\nlet x: f64 = 0.0;\n")?;
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
