//! The overflow step of `cargo xtask check` (T-199b).
//!
//! The step runs `length::tests::unchecked_arithmetic_panics_on_overflow`
//! from archviz-core in a release build and fails unless that one test ran
//! and passed, which shows a release build of core panics on overflow
//! instead of wrapping.

use std::path::Path;
use std::process::{Command, Stdio};

/// The test the step runs, as libtest names it.
pub const TEST: &str = "length::tests::unchecked_arithmetic_panics_on_overflow";

/// Runs the overflow test in release and judges its output.
pub fn step(root: &Path) -> Result<(), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let args = [
        "test",
        "--release",
        "-p",
        "archviz-core",
        "--lib",
        "--locked",
        "--",
        "--exact",
        TEST,
    ];
    let out = Command::new(cargo)
        .args(args)
        .current_dir(root)
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("could not run cargo {}: {e}", args.join(" ")))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    match verdict(out.status.success(), &stdout) {
        Ok(()) => {
            println!("overflow: a release build of core panics on overflow");
            Ok(())
        }
        Err(e) => Err(format!("{e}\n--- test output ---\n{stdout}")),
    }
}

/// Passes only when the test run succeeded and exactly one test passed.
pub fn verdict(success: bool, stdout: &str) -> Result<(), String> {
    if !success {
        return Err(format!(
            "{TEST} failed: a release build of core wraps on overflow"
        ));
    }
    if !stdout.contains("test result: ok. 1 passed;") {
        return Err(format!("expected exactly one passing test named {TEST}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{TEST, verdict};

    const PASSED: &str = "running 1 test\n\
        test length::tests::unchecked_arithmetic_panics_on_overflow ... ok\n\n\
        test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 180 filtered out\n";

    #[test]
    fn one_passing_test_passes() {
        assert_eq!(verdict(true, PASSED), Ok(()));
    }

    #[test]
    fn a_failed_test_fails() {
        let failed = "running 1 test\n\
            test length::tests::unchecked_arithmetic_panics_on_overflow ... FAILED\n\n\
            test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 180 filtered out\n";
        let err = verdict(false, failed).err().unwrap_or_default();
        assert!(err.contains("wraps on overflow"), "{err}");
    }

    #[test]
    fn no_test_run_fails() {
        let none = "running 0 tests\n\n\
            test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 181 filtered out\n";
        let err = verdict(true, none).err().unwrap_or_default();
        assert!(err.contains("exactly one passing test"), "{err}");
    }

    #[test]
    fn an_ignored_test_fails() {
        let ignored = "running 1 test\n\
            test length::tests::unchecked_arithmetic_panics_on_overflow ... ignored\n\n\
            test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 180 filtered out\n";
        assert!(verdict(true, ignored).is_err());
    }

    /// The named test exists in core, so the filter cannot silently match nothing.
    #[test]
    fn the_test_exists_in_core() -> Result<(), String> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../crates/core/src/length.rs");
        let source = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let name = TEST.rsplit("::").next().unwrap_or(TEST);
        assert!(
            source.contains(&format!("fn {name}()")),
            "{name} not in {path}"
        );
        Ok(())
    }

    /// The release profile keeps overflow checks on (T-199b).
    #[test]
    fn release_profile_checks_overflow() -> Result<(), String> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../Cargo.toml");
        let manifest = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let release = manifest
            .split("[profile.release]")
            .nth(1)
            .ok_or("no [profile.release] in the workspace Cargo.toml")?;
        let section = release.split("\n[").next().unwrap_or(release);
        assert!(
            section
                .lines()
                .any(|l| l.split_whitespace().collect::<String>() == "overflow-checks=true"),
            "[profile.release] does not set overflow-checks = true"
        );
        Ok(())
    }
}
