//! Workspace tooling: cargo xtask check, golden, ue-build, package.

mod float_lint;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "usage: cargo xtask <check | golden>
  check   float lint, fmt, clippy, tests, wasm32 build of core, cargo deny
  golden  regenerate schemas/document.schema.json and the insta snapshots";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("check") => check(),
        Some("golden") => golden(),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

/// Runs every gate in order and stops at the first failure.
fn check() -> Result<(), String> {
    let root = workspace_root();
    step("float lint (crates/core)", || float_lint_step(&root))?;
    cargo(&root, "fmt", &["fmt", "--all", "--", "--check"])?;
    cargo(
        &root,
        "clippy",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    cargo(&root, "test", &["test", "--workspace", "--locked"])?;
    cargo(
        &root,
        "wasm32 build (archviz-core)",
        &[
            "build",
            "-p",
            "archviz-core",
            "--target",
            "wasm32-unknown-unknown",
            "--locked",
        ],
    )?;
    cargo(&root, "cargo deny", &["deny", "check"])?;
    println!("xtask check: all steps passed");
    Ok(())
}

/// Regenerates every golden file. Review the diff by hand before committing.
fn golden() -> Result<(), String> {
    let root = workspace_root();
    step("schemas/document.schema.json", || {
        let schema = archviz_core::document_schema_json().map_err(|e| e.to_string())?;
        let path = root.join("schemas").join("document.schema.json");
        std::fs::write(&path, schema).map_err(|e| format!("{}: {e}", path.display()))
    })?;
    step("insta snapshots (archviz-core)", || {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let status = Command::new(cargo)
            .args(["test", "-p", "archviz-core", "--locked"])
            .env("INSTA_UPDATE", "always")
            .current_dir(&root)
            .status()
            .map_err(|e| format!("could not run cargo test: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("cargo test exited with {status}"))
        }
    })?;
    println!("xtask golden: done; review `git diff` before committing");
    Ok(())
}

fn step(name: &str, run: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    println!("==> {name}");
    run().map_err(|e| format!("xtask check: step `{name}` failed\n{e}"))
}

fn cargo(root: &Path, name: &str, args: &[&str]) -> Result<(), String> {
    step(name, || {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let status = Command::new(cargo)
            .args(args)
            .current_dir(root)
            .status()
            .map_err(|e| format!("could not run cargo {}: {e}", args.join(" ")))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("cargo {} exited with {status}", args.join(" ")))
        }
    })
}

fn float_lint_step(root: &Path) -> Result<(), String> {
    let core = root.join("crates").join("core");
    let findings =
        float_lint::scan_dir(&core).map_err(|e| format!("reading {}: {e}", core.display()))?;
    if findings.is_empty() {
        return Ok(());
    }
    let lines: Vec<String> = findings.iter().map(ToString::to_string).collect();
    Err(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::workspace_root;
    use archviz_core::{
        Action, Building, Command, Document, Length, Project, ProjectUnit, SetStorey, Source,
        Sourced, Uuid, apply, to_canonical_json,
    };

    const CHILD_ENV: &str = "ARCHVIZ_DETERMINISM_CHILD";

    /// T-105: one fixed command applied to one fixed document, as canonical JSON.
    fn determinism_result() -> Result<String, String> {
        let feet = |n: i64| Sourced {
            value: Length::from_nanometers(n * 304_800_000),
            source: Source::UserStated,
        };
        let genesis = Document::new(
            Project {
                id: Uuid::from_bytes([1; 16]),
                name: "Shop".into(),
                default_unit: ProjectUnit::Inches,
            },
            Building {
                id: Uuid::from_bytes([2; 16]),
                name: "Main".into(),
            },
        );
        let command = Command {
            id: Uuid::from_bytes([3; 16]),
            idempotency_key: Uuid::from_bytes([4; 16]),
            action: Action::SetStorey(SetStorey {
                storey: None,
                name: "Ground".into(),
                elevation: feet(0),
                floor_to_floor: feet(10),
                default_ceiling_height: feet(9),
            }),
        };
        let doc = apply(&genesis, &command).map_err(|e| e.to_string())?;
        to_canonical_json(&doc).map_err(|e| e.to_string())
    }

    /// Prints the result when run as a child process; a no-op otherwise.
    #[test]
    fn determinism_child() {
        if std::env::var_os(CHILD_ENV).is_some() {
            match determinism_result() {
                Ok(text) => print!("<<<{text}>>>"),
                Err(e) => print!("<<<error: {e}>>>"),
            }
        }
    }

    /// Runs this test binary as a separate process and returns what
    /// `determinism_child` printed.
    fn child_output() -> Result<String, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let out = std::process::Command::new(exe)
            .args([
                "--exact",
                "tests::determinism_child",
                "--nocapture",
                "--test-threads",
                "1",
            ])
            .env(CHILD_ENV, "1")
            .output()
            .map_err(|e| e.to_string())?;
        let stdout = String::from_utf8(out.stdout).map_err(|e| e.to_string())?;
        let start = stdout.find("<<<").ok_or("child printed no result")? + 3;
        let end = stdout[start..]
            .find(">>>")
            .ok_or("child result not terminated")?
            + start;
        Ok(stdout[start..end].to_string())
    }

    /// T-105: the same command agrees twice in one process and across two
    /// separate processes.
    #[test]
    fn same_command_agrees_across_processes() -> Result<(), String> {
        let in_process = [determinism_result()?, determinism_result()?];
        assert_eq!(in_process[0], in_process[1]);
        let children = [child_output()?, child_output()?];
        assert!(!children[0].starts_with("error"), "{}", children[0]);
        assert_eq!(children[0], children[1]);
        assert_eq!(children[0], in_process[0]);
        Ok(())
    }

    /// T-004: CI runs `cargo xtask check` on Windows for every push.
    #[test]
    fn ci_workflow_runs_check_on_windows_for_every_push() -> Result<(), String> {
        let path = workspace_root().join(".github/workflows/ci.yml");
        let yaml =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let lines: Vec<&str> = yaml.lines().map(str::trim_end).collect();
        assert!(
            lines.contains(&"on:"),
            "workflow has no `on:` trigger block"
        );
        assert!(
            lines.contains(&"  push:"),
            "workflow does not trigger on push"
        );
        assert!(
            !yaml.contains("branches:") && !yaml.contains("paths:"),
            "push trigger must not be filtered by branch or path"
        );
        assert!(
            yaml.contains("runs-on: windows-latest"),
            "job must run on Windows"
        );
        assert!(
            yaml.contains("run: cargo xtask check"),
            "job must run `cargo xtask check`"
        );
        Ok(())
    }
}
