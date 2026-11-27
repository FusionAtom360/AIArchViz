//! T-120b Done when: a script of CLI commands alone builds a document with
//! datums, walls, openings, constraints, a space, a slab, a shed roof and
//! box occurrences, and it solves.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use archviz_core::Uuid;
use serde_json::Value;

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

/// Builds a project one `archviz apply` per command, as a script would.
struct Script {
    dir: PathBuf,
    next: u32,
}

impl Script {
    fn key(n: u32) -> String {
        format!("00000000-0000-4000-9000-{n:012}")
    }

    /// Applies one command and returns the id of the entity it creates.
    fn apply(&mut self, kind: &str, args: &str) -> String {
        self.next += 1;
        let n = self.next;
        let command = format!(
            r#"{{"id":"00000000-0000-4000-8000-{n:012}","idempotency_key":"{}","kind":"{kind}","args":{args}}}"#,
            Self::key(n)
        );
        let request = format!(
            r#"{{"action":{{"kind":"commands","commands":[{command}]}},"timestamp_ms":{n},"actor":"script"}}"#
        );
        let (out, status) = archviz("apply", &self.dir, &request);
        assert_eq!(status, 0, "{kind} {args}: {out}");
        let key: Uuid = serde_json::from_value(Value::String(Self::key(n)))
            .unwrap_or_else(|e| panic!("key: {e}"));
        Uuid::derive(key, "command", "0").to_string()
    }
}

fn user(inches: i64) -> String {
    format!(r#"{{"value":{},"source":"user_stated"}}"#, inches * IN)
}

fn node_ref(id: &str) -> String {
    format!(r#"{{"kind":"node","node":"{id}"}}"#)
}

fn distance(a: &str, b: &str, axis: &str, nm: i64, source: &str) -> String {
    format!(
        r#"{{"kind":{{"kind":"distance","a":{a},"b":{b},"axis":"{axis}","value":{nm},"tolerance":null}},"source":"{source}"}}"#
    )
}

#[test]
fn a_script_of_cli_commands_builds_a_room_that_solves() {
    let dir = std::env::temp_dir().join(format!("archviz-script-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (out, status) = archviz(
        "new",
        &dir,
        r#"{"name":"Script","building":"Main","project_id":"11111111-1111-4111-8111-111111111111","building_id":"22222222-2222-4222-8222-222222222222"}"#,
    );
    assert_eq!(status, 0, "{out}");
    let mut s = Script { dir, next: 0 };

    let storey = s.apply(
        "set_storey",
        &format!(
            r#"{{"storey":null,"name":"Ground","elevation":{},"floor_to_floor":{},"default_ceiling_height":{}}}"#,
            user(0),
            user(120),
            user(108)
        ),
    );
    let datum = s.apply(
        "create_datum",
        r#"{"name":"SW","geometry":{"kind":"point","x":0,"y":0}}"#,
    );
    let datum = format!(r#"{{"kind":"datum","datum":"{datum}"}}"#);
    // Node positions start wrong on purpose; the dimensions place them.
    let corner = |s: &mut Script, x: i64, y: i64| {
        s.apply(
            "create_node",
            &format!(r#"{{"x":{},"y":{}}}"#, x * IN, y * IN),
        )
    };
    let sw = corner(&mut s, 0, 0);
    let se = corner(&mut s, 100, 0);
    let ne = corner(&mut s, 100, 100);
    let nw = corner(&mut s, 0, 100);
    let wall = |s: &mut Script, a: &str, b: &str, direction: &str| {
        s.apply(
            "create_wall",
            &format!(
                r#"{{"storey":"{storey}","node_a":"{a}","node_b":"{b}","direction":"{direction}","function":"existing"}}"#
            ),
        )
    };
    let south = wall(&mut s, &sw, &se, "pos_x");
    wall(&mut s, &se, &ne, "pos_y");
    wall(&mut s, &ne, &nw, "neg_x");
    wall(&mut s, &nw, &sw, "neg_y");
    let (swr, ser, nwr) = (node_ref(&sw), node_ref(&se), node_ref(&nw));
    s.apply(
        "add_constraint",
        &distance(&datum, &swr, "x", 0, "user_stated"),
    );
    s.apply(
        "add_constraint",
        &distance(&datum, &swr, "y", 0, "user_stated"),
    );
    s.apply(
        "add_constraint",
        &distance(&swr, &ser, "x", 120 * IN, "user_stated"),
    );
    s.apply(
        "add_constraint",
        &distance(&swr, &nwr, "y", 96 * IN, "user_stated"),
    );
    s.apply(
        "add_constraint",
        &distance(&swr, &ser, "x", 120 * IN + IN / 8, "measured"),
    );
    s.apply(
        "create_opening",
        &format!(
            r#"{{"kind":"door","host":"{south}","reference":{swr},"anchor":"start","offset":{},"width":null,"height":null,"sill":null}}"#,
            user(24)
        ),
    );
    let space = s.apply(
        "create_space",
        &format!(
            r#"{{"storey":"{storey}","name":"Room","seed":{{"x":{},"y":{}}}}}"#,
            60 * IN,
            48 * IN
        ),
    );
    s.apply(
        "create_slab",
        &format!(r#"{{"storey":"{storey}","boundary":{{"kind":"from_space","space":"{space}"}}}}"#),
    );
    s.apply(
        "create_roof",
        &format!(
            r#"{{"storey":"{storey}","footprint":{{"kind":"from_space","space":"{space}"}},"slope_direction":"neg_y","high_edge_height":{},"low_edge_height":{}}}"#,
            user(120),
            user(108)
        ),
    );
    let bench = s.apply(
        "create_element_type",
        &format!(
            r#"{{"name":"Bench","declared_dimensions":{{"width":{},"depth":{},"height":{},"authority":"user_stated_exact"}},"source":"parametric box","license":"n/a"}}"#,
            24 * IN,
            24 * IN,
            30 * IN
        ),
    );
    let item = s.apply(
        "place_occurrence",
        &format!(
            r#"{{"storey":"{storey}","element_type":"{bench}","placement":{{"kind":"floor","x":{},"y":{},"rotation":0,"elevation":{}}}}}"#,
            user(60),
            user(30),
            user(0)
        ),
    );

    let (solution, status) = archviz("solve", &s.dir, "");
    assert_eq!(status, 0, "{solution}");
    let result = &solution["result"];
    assert_eq!(result["conflicts"], serde_json::json!([]), "{solution}");
    // Only the bench is free: nothing dimensions it to the room.
    assert_eq!(
        result["underconstrained"],
        serde_json::json!([
            {"axis": "x", "members": [{"occurrence": item}]},
            {"axis": "y", "members": [{"occurrence": item}]}
        ]),
        "{solution}"
    );
    assert_eq!(result["nodes"][&ne]["x"], 120 * IN);
    assert_eq!(result["nodes"][&ne]["y"], 96 * IN);

    let (validity, status) = archviz("validate", &s.dir, "");
    assert_eq!(status, 0, "{validity}");
    assert_eq!(
        validity["result"]["violations"],
        serde_json::json!([]),
        "{validity}"
    );

    let (doc, _) = archviz("query", &s.dir, "");
    let count = |c: &str| doc["result"][c].as_object().map_or(0, |m| m.len());
    let counts = [
        "datums",
        "walls",
        "openings",
        "constraints",
        "spaces",
        "slabs",
        "roofs",
        "element_types",
        "occurrences",
    ]
    .map(count);
    assert_eq!(counts, [1, 4, 1, 5, 1, 1, 1, 1, 1]);
    let _ = std::fs::remove_dir_all(&s.dir);
}
