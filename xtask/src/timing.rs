//! The solver timing step of `cargo xtask check` (T-106, T-198).
//!
//! The step runs `timing::tests::solver_500_mixed_elements_under_50_ms` in a
//! release build and fails unless that one test ran, passed and printed a
//! maximum solve time under [`LIMIT_MICROS`].

use std::path::Path;
use std::process::{Command, Stdio};

/// The T-106 bound on one solve of the 500-element workload, in microseconds.
pub const LIMIT_MICROS: u128 = 50_000;

/// The test the step runs, as libtest names it.
pub const TEST: &str = "timing::tests::solver_500_mixed_elements_under_50_ms";

/// The line the timing test prints starts with this.
const PREFIX: &str = "solver timing:";

/// Runs the timing test in release and judges its output.
pub fn step(root: &Path) -> Result<(), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let args = [
        "test",
        "--release",
        "-p",
        "archviz-xtask",
        "--locked",
        "--",
        "--exact",
        TEST,
        "--nocapture",
    ];
    let out = Command::new(cargo)
        .args(args)
        .current_dir(root)
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("could not run cargo {}: {e}", args.join(" ")))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    match verdict(out.status.success(), &stdout) {
        Ok(line) => {
            println!("{line}");
            Ok(())
        }
        Err(e) => Err(format!("{e}\n--- test output ---\n{stdout}")),
    }
}

/// Passes only when the test exited successfully, exactly one test passed,
/// and the printed maximum is under the limit. Returns the timing line.
pub fn verdict(success: bool, stdout: &str) -> Result<String, String> {
    let line = stdout
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with(PREFIX))
        .ok_or("the timing test printed no measurement; did it run?")?;
    let max = micros_after(line, "max ")
        .ok_or_else(|| format!("no maximum time in the timing line: {line}"))?;
    if max >= LIMIT_MICROS {
        return Err(format!(
            "solver took {max} us, not under the {LIMIT_MICROS} us limit: {line}"
        ));
    }
    if !success {
        return Err(format!("the timing test failed: {line}"));
    }
    if !stdout.contains("test result: ok. 1 passed;") {
        return Err(format!("expected exactly one passing timing test: {line}"));
    }
    Ok(line.to_string())
}

/// The integer before " us" that follows `key` in `line`.
fn micros_after(line: &str, key: &str) -> Option<u128> {
    let rest = &line[line.find(key)? + key.len()..];
    let digits = rest.strip_suffix(rest.trim_start_matches(|c: char| c.is_ascii_digit()))?;
    let after = &rest[digits.len()..];
    if digits.is_empty() || !after.starts_with(" us") {
        return None;
    }
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{LIMIT_MICROS, PREFIX, verdict};
    use archviz_core::{
        Authority, Axis, Building, Clearance, Constraint, ConstraintKind, ConstraintStatus, Datum,
        DatumGeometry, DeclaredDimensions, Direction, Document, ElementType, Fidelity, Function,
        JoinType, LabelAllocator, Layer, LayerA, LayerSet, Length, Node, Occurrence, Opening,
        OpeningEdge, OpeningKind, Placement, Point, Project, ProjectUnit, QuarterTurns, Reference,
        Solver, Source, Sourced, Uuid, Wall, WallSide, WallTop, to_canonical_json,
    };
    use std::collections::BTreeMap;

    const WALLS: usize = 250;
    const OPENINGS: usize = 125;
    const OCCURRENCES: usize = 125;

    fn id(kind: &str, n: usize) -> Uuid {
        Uuid::derive(Uuid::from_bytes([7; 16]), kind, &n.to_string())
    }

    fn ft(n: i64) -> Length {
        Length::from_nanometers(n * 304_800_000)
    }

    fn user(l: Length) -> Sourced<Length> {
        Sourced {
            value: l,
            source: Source::UserStated,
        }
    }

    fn count(i: usize) -> i64 {
        i64::try_from(i).unwrap_or(0)
    }

    fn distance(a: Reference, b: Reference, axis: Axis, value: Length) -> ConstraintKind {
        ConstraintKind::Distance {
            a,
            b,
            axis,
            value,
            tolerance: None,
        }
    }

    /// T-198: 500 mixed elements with 1,500 driving constraints, built
    /// deterministically.
    ///
    /// - 250 walls, 10 ft long, alternating +X and +Y. Each places node a from
    ///   the origin datum, node b 10 ft from node a, and its left face either
    ///   fixed (the first two) or 1 ft from the left face of wall i - 2.
    /// - 125 windows, one in each of walls 0 to 124, 3 ft wide with the start
    ///   edge 1 ft from node a. Each has three redundant distances: node a to
    ///   start, start to end, end to node b.
    /// - 125 floor occurrences on a row 12 ft apart at y = -20 ft. Each has y
    ///   from the datum, x from the previous center (the first from the
    ///   datum), and a redundant x from the datum.
    fn five_hundred_mixed() -> Result<Document, String> {
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
        let datum = Reference::Datum { datum: origin };
        let mut labels = LabelAllocator::new();
        let mut constraints = Vec::new();

        for i in 0..WALLS {
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
            let (direction, along, across) = if horizontal {
                (Direction::PosX, Axis::X, Axis::Y)
            } else {
                (Direction::PosY, Axis::Y, Axis::X)
            };
            let wall = id("wall", i);
            doc.walls.insert(
                wall,
                Wall {
                    label: labels.allocate("W").map_err(|e| e.to_string())?,
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
            let (node_a, node_b) = (Reference::Node { node: a }, Reference::Node { node: b });
            constraints.push(distance(datum, node_a, along, ft(12 * count(i))));
            constraints.push(distance(node_a, node_b, along, ft(10)));
            let face = Reference::WallFace {
                wall,
                side: WallSide::Left,
                boundary: 0,
            };
            if i < 2 {
                constraints.push(ConstraintKind::Fixed {
                    reference: face,
                    axis: across,
                    value: ft(0),
                });
            } else {
                let prev = Reference::WallFace {
                    wall: id("wall", i - 2),
                    side: WallSide::Left,
                    boundary: 0,
                };
                constraints.push(distance(prev, face, across, ft(1)));
            }

            if i < OPENINGS {
                let opening = id("opening", i);
                doc.openings.insert(
                    opening,
                    Opening {
                        kind: OpeningKind::Window,
                        host: wall,
                        reference: node_a,
                        anchor: OpeningEdge::Start,
                        offset: user(ft(1)),
                        start: ft(0),
                        width: user(ft(3)),
                        height: user(ft(4)),
                        sill: user(ft(3)),
                    },
                );
                let edge = |edge| Reference::OpeningEdge { opening, edge };
                constraints.push(distance(node_a, edge(OpeningEdge::Start), along, ft(1)));
                constraints.push(distance(
                    edge(OpeningEdge::Start),
                    edge(OpeningEdge::End),
                    along,
                    ft(3),
                ));
                constraints.push(distance(edge(OpeningEdge::End), node_b, along, ft(6)));
            }
        }

        let cabinet = id("element_type", 0);
        doc.element_types.insert(
            cabinet,
            ElementType {
                name: "Cabinet 24 x 24".into(),
                declared_dimensions: DeclaredDimensions {
                    width: ft(2),
                    depth: ft(2),
                    height: ft(3),
                    authority: Authority::UserStatedExact,
                },
                fidelity: Fidelity::Proxy,
                model: None,
                properties: BTreeMap::new(),
                source: "bench".into(),
                license: "n/a".into(),
            },
        );
        let rotation = QuarterTurns::new(0).ok_or("no zero rotation")?;
        for k in 0..OCCURRENCES {
            let item = id("occurrence", k);
            let x = ft(12 * count(k));
            doc.occurrences.insert(
                item,
                Occurrence {
                    label: labels.allocate("EQ").map_err(|e| e.to_string())?,
                    storey: id("storey", 0),
                    element_type: cabinet,
                    placement: Placement::Floor {
                        x: user(ft(0)),
                        y: user(ft(0)),
                        rotation,
                        elevation: user(ft(0)),
                    },
                    clearance: Clearance {
                        front: user(ft(0)),
                        back: user(ft(0)),
                        left: user(ft(0)),
                        right: user(ft(0)),
                    },
                    fixed: false,
                    overlap_allowed: false,
                    light: None,
                    center: Point { x: ft(0), y: ft(0) },
                },
            );
            let center = Reference::ItemCenter { item };
            constraints.push(distance(datum, center, Axis::Y, ft(-20)));
            if k == 0 {
                constraints.push(distance(datum, center, Axis::X, ft(0)));
            } else {
                let prev = Reference::ItemCenter {
                    item: id("occurrence", k - 1),
                };
                constraints.push(distance(prev, center, Axis::X, ft(12)));
            }
            constraints.push(distance(datum, center, Axis::X, x));
        }

        for (n, kind) in constraints.into_iter().enumerate() {
            doc.constraints.insert(
                id("constraint", n),
                Constraint {
                    kind,
                    status: ConstraintStatus::Driving,
                    source: Source::UserStated,
                    provenance: None,
                },
            );
        }
        Ok(doc)
    }

    /// The workload has the stated mix, is byte-identical when rebuilt, and
    /// solves fully determined with no conflicts.
    #[test]
    fn workload_is_500_mixed_elements_with_1500_constraints() -> Result<(), String> {
        let doc = five_hundred_mixed()?;
        assert_eq!(
            (doc.walls.len(), doc.openings.len(), doc.occurrences.len()),
            (WALLS, OPENINGS, OCCURRENCES)
        );
        assert_eq!(WALLS + OPENINGS + OCCURRENCES, 500);
        assert_eq!(doc.constraints.len(), 1500);
        let again = five_hundred_mixed()?;
        assert_eq!(
            to_canonical_json(&doc).map_err(|e| e.to_string())?,
            to_canonical_json(&again).map_err(|e| e.to_string())?
        );
        let solution = LayerA::default().solve(&doc).map_err(|e| e.to_string())?;
        assert!(solution.conflicts.is_empty(), "{:?}", solution.conflicts);
        assert!(
            solution.underconstrained.is_empty(),
            "{:?}",
            solution.underconstrained
        );
        assert_eq!(solution.openings.len(), OPENINGS);
        assert_eq!(solution.occurrences.len(), OCCURRENCES);
        // Spot checks: opening 3 starts 1 ft past wall 3's node a (y = 36 ft);
        // occurrence 4 sits at (48 ft, -20 ft).
        assert_eq!(
            solution.openings.get(&id("opening", 3)).copied(),
            Some(ft(37))
        );
        assert_eq!(
            solution.occurrences.get(&id("occurrence", 4)).copied(),
            Some(Point {
                x: ft(48),
                y: ft(-20)
            })
        );
        Ok(())
    }

    /// T-106, T-198: one solve of the mixed workload takes under 50 ms in a
    /// release build. `cargo xtask check` runs it through `timing::step`.
    #[test]
    #[cfg_attr(
        debug_assertions,
        ignore = "timing is only meaningful in release builds; cargo xtask check runs it"
    )]
    fn solver_500_mixed_elements_under_50_ms() -> Result<(), String> {
        let doc = five_hundred_mixed()?;
        let solver = LayerA::default();
        let mut times = Vec::new();
        for _ in 0..10 {
            let start = std::time::Instant::now();
            let solution = solver.solve(&doc).map_err(|e| e.to_string())?;
            times.push(start.elapsed().as_micros());
            assert!(solution.conflicts.is_empty() && solution.underconstrained.is_empty());
        }
        times.sort_unstable();
        let (min, median, max) = (times[0], times[times.len() / 2], times[times.len() - 1]);
        println!(
            "{PREFIX} {WALLS} walls, {OPENINGS} openings, {OCCURRENCES} occurrences, {} constraints, {} runs: min {min} us, median {median} us, max {max} us (limit {LIMIT_MICROS} us)",
            doc.constraints.len(),
            times.len()
        );
        assert!(max < LIMIT_MICROS, "max {max} us");
        Ok(())
    }

    fn output(max: u128, result: &str) -> String {
        format!(
            "running 1 test\n{PREFIX} 250 walls, 125 openings, 125 occurrences, 1500 constraints, 10 runs: min 900 us, median 1200 us, max {max} us (limit 50000 us)\ntest timing::tests::solver_500_mixed_elements_under_50_ms ... ok\n\ntest result: {result}\n"
        )
    }

    const OK: &str =
        "ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.05s";
    const FAILED: &str =
        "FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.60s";

    #[test]
    fn step_passes_a_run_under_the_bound() -> Result<(), String> {
        let line = verdict(true, &output(2_345, OK))?;
        assert!(line.starts_with(PREFIX) && line.contains("max 2345 us"));
        Ok(())
    }

    #[test]
    fn step_fails_at_or_over_the_bound() {
        for max in [LIMIT_MICROS, LIMIT_MICROS + 1, 612_000] {
            let e = verdict(false, &output(max, FAILED)).err();
            assert!(
                e.as_deref()
                    .is_some_and(|e| e.contains(&format!("solver took {max} us"))),
                "{e:?}"
            );
        }
        // Even if the test's own assert were lost, the step judges the number.
        assert!(verdict(true, &output(LIMIT_MICROS, OK)).is_err());
    }

    #[test]
    fn step_fails_when_the_test_fails() {
        assert!(verdict(false, &output(1_000, FAILED)).is_err());
    }

    #[test]
    fn step_fails_when_the_test_did_not_run() {
        let ignored = "running 1 test\ntest timing::tests::solver_500_mixed_elements_under_50_ms ... ignored\n\ntest result: ok. 0 passed; 0 failed; 1 ignored\n";
        let filtered = "running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 8 filtered out\n";
        for stdout in [ignored, filtered, ""] {
            assert!(verdict(true, stdout).is_err(), "{stdout}");
        }
        let line_without_pass = output(1_000, "ok. 0 passed; 0 failed; 1 ignored");
        assert!(verdict(true, &line_without_pass).is_err());
    }

    #[test]
    fn step_fails_on_an_unreadable_time() {
        for bad in ["max us", "max 12ms", "max -3 us", "min 5 us"] {
            let stdout = format!("{PREFIX} {bad}\ntest result: {OK}\n");
            assert!(verdict(true, &stdout).is_err(), "{bad}");
        }
    }
}
