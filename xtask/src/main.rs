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
