//! Golden transcripts of the `archviz` binary, and the project lock (T-117).

use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// `text` with the folder's path (as it appears inside JSON) replaced by
/// `<dir>`, so transcripts do not depend on where tests run.
fn redact(text: &str, dir: &Path) -> String {
    let quoted = serde_json::to_string(&dir.display().to_string()).unwrap_or_default();
    text.replace(quoted.trim_matches('"'), "<dir>")
}

/// Runs `archviz <sub> <dir>` with `stdin`; returns (stdout with the folder
/// path redacted, exit status).
fn archviz(sub: &str, dir: &Path, stdin: &str) -> (String, i32) {
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
        redact(String::from_utf8_lossy(&out.stdout).trim_end(), dir),
        out.status.code().unwrap_or(-1),
    )
}

fn fresh_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("archviz-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Records each step as `$ archviz <sub>`, `< stdin`, `> stdout`, `= status`.
struct Transcript {
    dir: PathBuf,
    text: String,
}

impl Transcript {
    fn step(&mut self, sub: &str, stdin: &str) -> String {
        let (out, status) = archviz(sub, &self.dir, stdin);
        let _ = writeln!(
            self.text,
            "$ archviz {sub}\n< {}\n> {out}\n= {status}\n",
            stdin.trim()
        );
        out
    }
}

const PROJECT: &str = "11111111-1111-4111-8111-111111111111";
const BUILDING: &str = "22222222-2222-4222-8222-222222222222";

fn command(n: u8, kind: &str, args: &str) -> String {
    format!(
        r#"{{"id":"00000000-0000-4000-8000-0000000000{n:02}","idempotency_key":"00000000-0000-4000-9000-0000000000{n:02}","kind":"{kind}","args":{args}}}"#
    )
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or(serde_json::Value::Null)
}

#[test]
fn golden_transcript() {
    let mut t = Transcript {
        dir: fresh_dir("transcript"),
        text: String::new(),
    };
    let new = format!(
        r#"{{"name":"Shop","building":"Main","project_id":"{PROJECT}","building_id":"{BUILDING}"}}"#
    );
    t.step("new", &new);
    t.step("new", &new);
    let storey = command(
        1,
        "set_storey",
        r#"{"storey":null,"name":"Ground","elevation":{"value":0,"source":"user_stated"},"floor_to_floor":{"value":3048000000,"source":"user_stated"},"default_ceiling_height":{"value":2743200000,"source":"proposed"}}"#,
    );
    let node = command(2, "create_node", r#"{"x":0,"y":0}"#);
    t.step("apply", &format!(r#"{{"action":{{"kind":"commands","commands":[{storey},{node}]}},"timestamp_ms":1000,"actor":"user"}}"#));
    let feet = command(3, "set_project_units", r#"{"default_unit":"feet"}"#);
    let action = format!(r#"{{"kind":"commands","commands":[{feet}]}}"#);
    let proposal = t.step("propose", &format!(r#"{{"action":{action}}}"#));
    let id = json(&proposal)["result"]["proposal_id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let commit = format!(
        r#"{{"proposal_id":"{id}","base_version":1,"action":{action},"timestamp_ms":2000,"actor":"user"}}"#
    );
    t.step("commit", &commit);
    t.step("commit", &commit);
    t.step("query", r#"{"collection":"storeys"}"#);
    t.step(
        "query",
        r#"{"collection":"walls","id":"99999999-9999-4999-8999-999999999999"}"#,
    );
    t.step("solve", "");
    t.step("validate", "");
    t.step("required-parameters", "");
    t.step("undo", r#"{"timestamp_ms":3000,"actor":"user"}"#);
    t.step("query", r#"{"collection":"project"}"#);
    t.step("validate", "{not json");
    t.step("teleport", "");
    let log = std::fs::read_to_string(t.dir.join("log.jsonl")).unwrap_or_default();
    let _ = writeln!(t.text, "log.jsonl:\n{log}");
    let empty = fresh_dir("not-a-project");
    let (out, status) = archviz("validate", &empty, "");
    let _ = writeln!(
        t.text,
        "$ archviz validate <empty folder>\n> {out}\n= {status}"
    );
    let _ = std::fs::remove_dir_all(&t.dir);
    insta::assert_snapshot!(t.text);
}

fn hold_lock(dir: &Path) -> File {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("project.lock"))
        .unwrap_or_else(|e| panic!("open lock: {e}"));
    file.try_lock().unwrap_or_else(|e| panic!("lock: {e}"));
    file
}

/// Done when: a second process receives the lock error.
#[test]
fn a_second_process_receives_the_lock_error() {
    let dir = fresh_dir("lock");
    let new = format!(
        r#"{{"name":"Shop","building":"Main","project_id":"{PROJECT}","building_id":"{BUILDING}"}}"#
    );
    assert_eq!(archviz("new", &dir, &new).1, 0);
    // This test process holds the project open, as a host would.
    let held = hold_lock(&dir);
    let (out, status) = archviz("validate", &dir, "");
    assert_eq!(status, 5, "{out}");
    assert_eq!(json(&out)["error"]["code"], "LOCKED");
    assert_eq!(json(&out)["ok"], false);
    drop(held);
    // Once released, the project opens again.
    let (out, status) = archviz("validate", &dir, "");
    assert_eq!(status, 0, "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}
