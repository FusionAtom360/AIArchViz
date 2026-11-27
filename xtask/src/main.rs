//! Workspace tooling: cargo xtask check, golden, ue-build, package.

mod float_lint;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "usage: cargo xtask <check | golden>
  check   float lint, fmt, clippy, tests, wasm32 build of core, cargo deny
  golden  regenerate schemas/ (document and cli) and the insta snapshots";

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
    step("schemas/scene.schema.json", || {
        let schema = archviz_export::scene_schema_json().map_err(|e| e.to_string())?;
        let path = root.join("schemas").join("scene.schema.json");
        std::fs::write(&path, schema).map_err(|e| format!("{}: {e}", path.display()))
    })?;
    step("schemas/cli/*.schema.json", || {
        let dir = root.join("schemas").join("cli");
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for (name, text) in archviz_cli::schemas()? {
            let path = dir.join(name);
            std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Ok(())
    })?;
    step(
        "insta snapshots (archviz-core, archviz-cli, archviz-export)",
        || {
            let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
            let status = Command::new(cargo)
                .args([
                    "test",
                    "-p",
                    "archviz-core",
                    "-p",
                    "archviz-cli",
                    "-p",
                    "archviz-export",
                    "--locked",
                ])
                .env("INSTA_UPDATE", "always")
                .current_dir(&root)
                .status()
                .map_err(|e| format!("could not run cargo test: {e}"))?;
            if status.success() {
                Ok(())
            } else {
                Err(format!("cargo test exited with {status}"))
            }
        },
    )?;
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

    /// T-106: 500 walls (1,000 nodes), each placed from a datum by two
    /// distances, with its left face fixed or set 1 ft from the face of the
    /// previous parallel wall: 1,500 constraints in all.
    fn five_hundred_walls() -> Document {
        use archviz_core::{
            Axis, Constraint, ConstraintKind, ConstraintStatus, Datum, DatumGeometry, Direction,
            Function, JoinType, LabelAllocator, Layer, LayerSet, Node, Reference, Wall, WallSide,
            WallTop,
        };
        let id =
            |kind: &str, n: usize| Uuid::derive(Uuid::from_bytes([7; 16]), kind, &n.to_string());
        let ft = |n: i64| Length::from_nanometers(n * 304_800_000);
        let user = |l: Length| Sourced {
            value: l,
            source: Source::UserStated,
        };
        let mut doc = Document::new(
            Project {
                id: id("project", 0),
                name: "Bench".into(),
                default_unit: ProjectUnit::Inches,
            },
            Building {
                id: id("building", 0),
                name: "Bench".into(),
            },
        );
        let layers = id("layers", 0);
        doc.layer_sets.insert(
            layers,
            LayerSet {
                name: "Stud".into(),
                layers: vec![Layer {
                    material: id("material", 0),
                    thickness: user(Length::from_nanometers(114_300_000)),
                    core: true,
                }],
            },
        );
        let origin = id("datum", 0);
        doc.datums.insert(
            origin,
            Datum {
                name: "Origin".into(),
                geometry: DatumGeometry::Point { x: ft(0), y: ft(0) },
            },
        );
        let mut labels = LabelAllocator::new();
        let constraint = |doc: &mut Document, n: &mut usize, kind| {
            *n += 1;
            doc.constraints.insert(
                id("constraint", *n),
                Constraint {
                    kind,
                    status: ConstraintStatus::Driving,
                    source: Source::UserStated,
                    provenance: None,
                },
            );
        };
        let mut n = 0;
        let datum = Reference::Datum { datum: origin };
        for i in 0..500 {
            let (a, b) = (id("node", 2 * i), id("node", 2 * i + 1));
            for node in [a, b] {
                doc.nodes.insert(
                    node,
                    Node {
                        x: ft(0),
                        y: ft(0),
                        join: JoinType::Isolated,
                    },
                );
            }
            let horizontal = i % 2 == 0;
            let direction = if horizontal {
                Direction::PosX
            } else {
                Direction::PosY
            };
            let (along, across) = if horizontal {
                (Axis::X, Axis::Y)
            } else {
                (Axis::Y, Axis::X)
            };
            let wall = id("wall", i);
            let label = labels.allocate("W").unwrap_or_else(|_| unreachable!());
            doc.walls.insert(
                wall,
                Wall {
                    label,
                    storey: id("storey", 0),
                    node_a: a,
                    node_b: b,
                    direction,
                    layer_set: layers,
                    left_extent: user(Length::from_nanometers(57_150_000)),
                    base_offset: user(ft(0)),
                    top: WallTop::Storey,
                    join_priority: 0,
                    function: Function::New,
                },
            );
            let offset = i64::try_from(i).unwrap_or(0) * 3;
            let node_a = Reference::Node { node: a };
            constraint(
                &mut doc,
                &mut n,
                ConstraintKind::Distance {
                    a: datum,
                    b: node_a,
                    axis: along,
                    value: ft(offset),
                    tolerance: None,
                },
            );
            constraint(
                &mut doc,
                &mut n,
                ConstraintKind::Distance {
                    a: node_a,
                    b: Reference::Node { node: b },
                    axis: along,
                    value: ft(2),
                    tolerance: None,
                },
            );
            let face = Reference::WallFace {
                wall,
                side: WallSide::Left,
                boundary: 0,
            };
            if i < 2 {
                constraint(
                    &mut doc,
                    &mut n,
                    ConstraintKind::Fixed {
                        reference: face,
                        axis: across,
                        value: ft(0),
                    },
                );
            } else {
                // Same orientation as wall i - 2: align their left faces, 1 ft apart.
                let prev = Reference::WallFace {
                    wall: id("wall", i - 2),
                    side: WallSide::Left,
                    boundary: 0,
                };
                constraint(
                    &mut doc,
                    &mut n,
                    ConstraintKind::Distance {
                        a: prev,
                        b: face,
                        axis: across,
                        value: ft(1),
                        tolerance: None,
                    },
                );
            }
        }
        doc
    }

    /// T-106 Done when: solving 500 elements takes under 50 ms in release mode.
    /// Run with `cargo test --release -p archviz-xtask solver_500 -- --nocapture`.
    #[test]
    #[cfg_attr(
        debug_assertions,
        ignore = "timing is only meaningful in release builds"
    )]
    fn solver_500_elements_under_50_ms() -> Result<(), String> {
        use archviz_core::{LayerA, Solver};
        let doc = five_hundred_walls();
        assert_eq!((doc.walls.len(), doc.nodes.len()), (500, 1000));
        let solver = LayerA::default();
        let mut times = Vec::new();
        let mut last = None;
        for _ in 0..10 {
            let start = std::time::Instant::now();
            let solution = solver.solve(&doc).map_err(|e| e.to_string())?;
            times.push(start.elapsed());
            last = Some(solution);
        }
        let solution = last.ok_or("no run")?;
        assert!(solution.conflicts.is_empty() && solution.underconstrained.is_empty());
        times.sort();
        println!(
            "solver: 500 walls, {} constraints: min {:?}, median {:?}, max {:?}",
            doc.constraints.len(),
            times[0],
            times[times.len() / 2],
            times[times.len() - 1]
        );
        assert!(times[times.len() - 1] < std::time::Duration::from_millis(50));
        Ok(())
    }

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
