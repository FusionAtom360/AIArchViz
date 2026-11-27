//! T-121: the fixtures in fixtures/<name>/commands.txt, replayed through
//! the `archviz` binary exactly as a shell script would run them.
//!
//! Each line of commands.txt is `<subcommand> <request JSON>`; the files
//! are generated from typed builders by `cargo xtask golden` (xtask/src/fixtures.rs).

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

const IN: i64 = 25_400_000;

fn archviz(sub: &str, dir: &Path, stdin: &str) -> (Value, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_archviz"))
        .arg(sub)
        .arg(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn: {e}"));
    if let Some(mut input) = child.stdin.take() {
        let _ = input.write_all(stdin.as_bytes());
    }
    let out = child
        .wait_with_output()
        .unwrap_or_else(|e| panic!("wait: {e}"));
    (
        serde_json::from_slice(&out.stdout).unwrap_or(Value::Null),
        out.status.code().unwrap_or(-1),
    )
}

fn script(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
        .join("commands.txt");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Replays a fixture script into a fresh project folder.
fn replay(name: &str, tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "archviz-fixture-{name}-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    for (i, line) in script(name).lines().enumerate() {
        let (sub, request) = line.split_once(' ').unwrap_or((line, ""));
        let (out, status) = archviz(sub, &dir, request);
        assert_eq!(status, 0, "{name} line {}: {out}", i + 1);
    }
    dir
}

fn solve(dir: &Path) -> Value {
    let (out, status) = archviz("solve", dir, "");
    assert_eq!(status, 0, "{out}");
    out["result"].clone()
}

fn query(dir: &Path) -> Value {
    let (out, status) = archviz("query", dir, "");
    assert_eq!(status, 0, "{out}");
    out["result"].clone()
}

fn no_violations(dir: &Path) {
    let (out, status) = archviz("validate", dir, "");
    assert_eq!(status, 0, "{out}");
    assert_eq!(out["result"]["violations"], json!([]), "{out}");
}

/// The id of the datum, wall or occurrence with this name or label.
fn named(doc: &Value, collection: &str, field: &str, name: &str) -> String {
    doc[collection]
        .as_object()
        .and_then(|m| m.iter().find(|(_, v)| v[field] == name))
        .map(|(id, _)| id.clone())
        .unwrap_or_else(|| panic!("no {collection} {name}"))
}

/// Underconstrained classes as (axis, member kind, member id).
fn free(solution: &Value) -> BTreeSet<(String, String, String)> {
    let mut out = BTreeSet::new();
    for class in solution["underconstrained"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let axis = class["axis"].as_str().unwrap_or_default().to_string();
        for member in class["members"].as_array().into_iter().flatten() {
            for (kind, id) in member.as_object().into_iter().flatten() {
                out.insert((
                    axis.clone(),
                    kind.clone(),
                    id.as_str().unwrap_or_default().to_string(),
                ));
            }
        }
    }
    out
}

#[test]
fn l_room_replays_with_fiducials_in_place() {
    let dir = replay("l-room", "a");
    let solution = solve(&dir);
    assert_eq!(solution["conflicts"], json!([]), "{solution}");
    no_violations(&dir);
    let doc = query(&dir);
    for (name, x, y) in [
        ("F-A", 0, 0),
        ("F-B", 240, 0),
        ("F-C", 240, 96),
        ("F-D", 0, 180),
    ] {
        let id = named(&doc, "datums", "name", name);
        assert_eq!(
            doc["datums"][&id]["geometry"],
            json!({"kind": "point", "x": x * IN, "y": y * IN}),
            "{name}"
        );
    }
    // The walls hold the L, with the arms of different lengths.
    let corners: BTreeSet<(i64, i64)> = solution["nodes"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(_, p)| {
            (
                p["x"].as_i64().unwrap_or(0) / IN,
                p["y"].as_i64().unwrap_or(0) / IN,
            )
        })
        .collect();
    let expected: BTreeSet<(i64, i64)> =
        [(0, 0), (240, 0), (240, 96), (96, 96), (96, 180), (0, 180)]
            .into_iter()
            .collect();
    assert_eq!(corners, expected);
    // Only the marker box is free.
    let marker = named(&doc, "occurrences", "label", "EQ-001");
    let kinds: BTreeSet<_> = free(&solution)
        .into_iter()
        .map(|(_, kind, id)| (kind, id))
        .collect();
    assert_eq!(
        kinds,
        [("occurrence".to_string(), marker)].into_iter().collect()
    );
    assert_eq!(doc["openings"].as_object().map(|m| m.len()), Some(3));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn shop_solves_without_conflicts_and_lists_its_free_items() {
    let dir = replay("shop", "a");
    let solution = solve(&dir);
    assert_eq!(solution["conflicts"], json!([]), "{solution}");
    no_violations(&dir);
    let doc = query(&dir);

    // Underconstrained: exactly the twelve equipment occurrences, each free
    // on both axes; every wall, opening and datum is determined.
    let free = free(&solution);
    let occurrences: BTreeSet<String> = doc["occurrences"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(id, _)| id.clone())
        .collect();
    assert_eq!(occurrences.len(), 12);
    let expected: BTreeSet<(String, String, String)> = ["x", "y"]
        .into_iter()
        .flat_map(|axis| {
            occurrences
                .iter()
                .map(move |id| (axis.to_string(), "occurrence".to_string(), id.clone()))
        })
        .collect();
    assert_eq!(free, expected);

    // G-2 with Q-61 (36 ft N-S x 18 ft E-W), 4 in walls, centerlines.
    let all: Vec<(i64, i64)> = solution["nodes"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(_, p)| (p["x"].as_i64().unwrap_or(0), p["y"].as_i64().unwrap_or(0)))
        .collect();
    let xs: BTreeSet<i64> = all.iter().map(|p| p.0).collect();
    let ys: BTreeSet<i64> = all.iter().map(|p| p.1).collect();
    // West, new wall 2 (east face 144 in from the east face), east, extension.
    assert_eq!(xs, [2, 70, 214, 278].map(|v| v * IN).into_iter().collect());
    // South, extension south, new wall 1 (north face 312 in from the north
    // face), extension north, north.
    assert_eq!(
        ys,
        [2, 38, 118, 394, 430].map(|v| v * IN).into_iter().collect()
    );

    // Openings: north garage door, four on the west wall, south window and
    // the extension door in the east wall.
    let openings = doc["openings"].as_object().cloned().unwrap_or_default();
    assert_eq!(openings.len(), 7);
    let sizes: BTreeSet<(i64, i64, i64)> = openings
        .values()
        .map(|o| {
            (
                o["width"]["value"].as_i64().unwrap_or(0) / IN,
                o["height"]["value"].as_i64().unwrap_or(0) / IN,
                o["sill"]["value"].as_i64().unwrap_or(0) / IN,
            )
        })
        .collect();
    let expected_sizes: BTreeSet<(i64, i64, i64)> = [
        (144, 109, 0),
        (36, 80, 0),
        (60, 45, 35),
        (32, 80, 0),
        (36, 45, 35),
        (48, 96, 0),
    ]
    .into_iter()
    .collect();
    assert_eq!(sizes, expected_sizes);
    assert!(
        openings.values().all(
            |o| o["width"]["source"] == "user_stated" && o["height"]["source"] == "user_stated"
        ),
        "every opening size is from G-2"
    );

    // The extension's shed roof: ledger on the east wall, 108 in to 96 in.
    let roofs = doc["roofs"].as_object().cloned().unwrap_or_default();
    assert_eq!(roofs.len(), 1);
    let roof = roofs.values().next().cloned().unwrap_or_default();
    assert_eq!(roof["slope_direction"]["value"], "pos_x");
    assert_eq!(roof["high_edge_height"]["value"], 108 * IN);
    assert_eq!(roof["low_edge_height"]["value"], 96 * IN);

    // No G-2 value is unsure (Q-65), so no Measured constraint exists, and
    // every constraint is user-stated.
    assert!(
        doc["constraints"]
            .as_object()
            .into_iter()
            .flatten()
            .all(|(_, c)| c["source"] == "user_stated")
    );
    assert_eq!(
        doc["walls"].as_object().map(|m| m.len()),
        Some(15),
        "9 exterior segments (split at every T), 3 for the new walls, 3 for the extension"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fixtures_replay_to_identical_bytes() {
    for name in ["l-room", "shop"] {
        let a = replay(name, "b1");
        let b = replay(name, "b2");
        for file in ["genesis.json", "log.jsonl"] {
            let read = |d: &Path| std::fs::read(d.join(file)).unwrap_or_default();
            assert_eq!(read(&a), read(&b), "{name} {file}");
            assert!(!read(&a).is_empty());
        }
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }
}

/// T-190 (spec 5.3, REVIEW-1 C-1): a user-stated constraint 1 nm off the
/// 1/16 in grid is rejected with the nearest grid value, and nothing commits.
#[test]
fn an_off_grid_user_stated_constraint_is_rejected() {
    let on = "\"value\":6096000000";
    let text = script("l-room");
    let at = text
        .lines()
        .position(|l| l.contains("\"kind\":\"add_constraint\"") && l.contains(on))
        .unwrap_or_else(|| panic!("no add_constraint with {on}"));
    let dir = std::env::temp_dir().join(format!("archviz-fixture-offgrid-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (i, line) in text.lines().enumerate() {
        let (sub, request) = line.split_once(' ').unwrap_or((line, ""));
        if i < at {
            let (out, status) = archviz(sub, &dir, request);
            assert_eq!(status, 0, "line {}: {out}", i + 1);
            continue;
        }
        assert!(request.contains("\"source\":\"user_stated\""), "{request}");
        let log = std::fs::read(dir.join("log.jsonl")).unwrap_or_default();
        let (out, status) = archviz(sub, &dir, &request.replacen(on, "\"value\":6096000001", 1));
        assert_ne!(status, 0, "{out}");
        assert_eq!(out["ok"], json!(false), "{out}");
        let message = out["error"]["message"].as_str().unwrap_or_default();
        assert!(message.contains("/args/kind/value"), "{message}");
        assert!(
            message.contains("nearest grid value is 20'-0\""),
            "{message}"
        );
        assert_eq!(
            std::fs::read(dir.join("log.jsonl")).unwrap_or_default(),
            log
        );
        // The same line on the grid still commits.
        let (out, status) = archviz(sub, &dir, request);
        assert_eq!(status, 0, "{out}");
        break;
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A copy of `from` (genesis.json only) in a fresh folder, with `log` as its
/// log.jsonl.
fn with_log(from: &Path, tag: &str, log: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("archviz-torn-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    std::fs::copy(from.join("genesis.json"), dir.join("genesis.json"))
        .unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(dir.join("log.jsonl"), log).unwrap_or_else(|e| panic!("{e}"));
    dir
}

fn log_of(dir: &Path) -> Vec<u8> {
    std::fs::read(dir.join("log.jsonl")).unwrap_or_else(|e| panic!("{e}"))
}

/// T-193 (spec 8.4, Q-73, REVIEW-1 H-3): a crash during an append leaves a
/// final segment with no newline. It was never committed, so the project
/// opens with every complete entry, read-only subcommands leave the file
/// alone, and the next commit replaces the torn tail.
#[test]
fn a_torn_last_line_is_dropped_and_replaced_by_the_next_commit() {
    let full = replay("l-room", "torn");
    let log = log_of(&full);
    let text = script("l-room");
    let last = text.lines().last().unwrap_or_default();
    let (sub, request) = last.split_once(' ').unwrap_or((last, ""));
    assert_eq!(sub, "apply");
    // Every complete entry but the last one.
    let body = log.strip_suffix(b"\n").unwrap_or(&log);
    let cut = body.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    let complete = &log[..cut];
    assert!(complete.ends_with(b"\n") && complete.len() < log.len());
    let reference = with_log(&full, "reference", complete);
    let expected = query(&reference);

    let tails: [(&str, Vec<u8>); 3] = [
        ("partial", br#"{"kind":"commit","seq":"#.to_vec()),
        ("half-entry", log[cut..cut + (log.len() - cut) / 2].to_vec()),
        // A cut inside a two-byte UTF-8 character.
        ("utf8", b"{\"actor\":\"\xC3".to_vec()),
    ];
    for (tag, tail) in tails {
        let torn = [complete, tail.as_slice()].concat();
        let dir = with_log(&full, tag, &torn);
        assert_eq!(query(&dir), expected, "{tag}");
        solve(&dir);
        no_violations(&dir);
        assert_eq!(
            log_of(&dir),
            torn,
            "{tag}: read-only subcommands wrote the log"
        );

        let (out, status) = archviz(sub, &dir, request);
        assert_eq!(status, 0, "{tag}: {out}");
        assert_eq!(log_of(&dir), log, "{tag}: the torn tail was not replaced");
        assert_eq!(query(&dir), query(&full), "{tag}");
        let _ = std::fs::remove_dir_all(&dir);
    }
    let _ = std::fs::remove_dir_all(&reference);
    let _ = std::fs::remove_dir_all(&full);
}

/// T-193: a bad line that ends in a newline was flushed as committed, so it
/// is corruption, not a torn append.
#[test]
fn a_bad_complete_line_is_still_a_corrupt_project() {
    let full = replay("l-room", "corrupt");
    let log = log_of(&full);
    for (tag, bad) in [
        (
            "last",
            [log.as_slice(), b"{\"kind\":\"commit\",\"seq\":\n"].concat(),
        ),
        (
            "middle",
            [b"not json\n".as_slice(), log.as_slice()].concat(),
        ),
    ] {
        let dir = with_log(&full, tag, &bad);
        let (out, status) = archviz("query", &dir, "");
        assert_eq!(status, 6, "{tag}: {out}");
        assert_eq!(
            out["error"]["code"],
            json!("CORRUPT_PROJECT"),
            "{tag}: {out}"
        );
        assert_eq!(log_of(&dir), bad, "{tag}");
        let _ = std::fs::remove_dir_all(&dir);
    }
    let _ = std::fs::remove_dir_all(&full);
}
