//! The host process (T-201, spec 3.1, DEC-13): while a host has a project
//! open, `archviz` attaches to it over its named pipe instead of opening the
//! project files, and concurrent clients cannot corrupt the project.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use archviz_cli::host::{HOST_FILE, Server, Shared, request, serve};
use archviz_cli::{Host, store};
use archviz_core::Uuid;

const PROJECT: &str = "11111111-1111-4111-8111-111111111111";
const BUILDING: &str = "22222222-2222-4222-8222-222222222222";

/// A fixed clock and counting ids, so nothing here depends on the time.
struct Fixed(u128);

impl Host for Fixed {
    fn now_ms(&mut self) -> i64 {
        5000
    }
    fn new_uuid(&mut self) -> Uuid {
        self.0 += 1;
        Uuid::from_bytes(self.0.to_be_bytes())
    }
}

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
        String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
        out.status.code().unwrap_or(-1),
    )
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or(serde_json::Value::Null)
}

fn fresh_project(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("archviz-host-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let new = format!(
        r#"{{"name":"Shop","building":"Main","project_id":"{PROJECT}","building_id":"{BUILDING}"}}"#
    );
    let (out, status) = archviz("new", &dir, &new);
    assert_eq!(status, 0, "{out}");
    dir
}

/// An `apply` request creating one node, with ids unique to `n`.
fn create_node(n: u32) -> String {
    format!(
        r#"{{"action":{{"kind":"commands","commands":[{{"id":"00000000-0000-4000-8000-{n:012}","idempotency_key":"00000000-0000-4000-9000-{n:012}","kind":"create_node","args":{{"x":{x},"y":0}}}}]}},"actor":"user"}}"#,
        x = u64::from(n) * 25_400_000
    )
}

/// Starts a host on `dir` in this process, counting its change notices.
fn start(dir: &Path) -> (Arc<Mutex<Shared>>, Server, Arc<AtomicU64>) {
    let project = store::open(dir).unwrap_or_else(|e| panic!("{e:?}"));
    let shared = Arc::new(Mutex::new(Shared {
        project,
        host: Box::new(Fixed(0)),
    }));
    let changes = Arc::new(AtomicU64::new(0));
    let counter = changes.clone();
    let server = serve(
        &shared,
        dir,
        Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    (shared, server, changes)
}

fn version(shared: &Arc<Mutex<Shared>>) -> u64 {
    shared
        .lock()
        .unwrap_or_else(|e| panic!("{e}"))
        .project
        .history
        .version()
}

/// Done when: CLI commands issued while the host is open reach the host's
/// project (the app shows that project; see the app's own test).
#[test]
fn the_cli_attaches_to_a_running_host() {
    let dir = fresh_project("attach");
    let (shared, server, changes) = start(&dir);
    assert!(dir.join(HOST_FILE).is_file());
    let (out, status) = archviz("apply", &dir, &create_node(1));
    assert_eq!(status, 0, "{out}");
    assert_eq!(json(&out)["result"]["version"], 1);
    assert_eq!(version(&shared), 1, "the host's own project has the commit");
    assert_eq!(changes.load(Ordering::SeqCst), 1);
    let (out, status) = archviz("query", &dir, r#"{"collection":"nodes"}"#);
    assert_eq!(status, 0, "{out}");
    assert_eq!(json(&out)["result"].as_object().map(|m| m.len()), Some(1));
    assert_eq!(changes.load(Ordering::SeqCst), 1, "a query changes nothing");
    // `new` never goes through the host.
    let (out, status) = archviz("new", &dir, r#"{"name":"X","building":"Y"}"#);
    assert_ne!(status, 0, "{out}");
    drop(server);
    assert!(!dir.join(HOST_FILE).exists(), "closing removes host.json");
    drop(shared);
    let (out, status) = archviz("query", &dir, r#"{"collection":"nodes"}"#);
    assert_eq!(status, 0, "{out}");
    assert_eq!(json(&out)["result"].as_object().map(|m| m.len()), Some(1));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Done when: two clients cannot corrupt the project. Eight `archviz`
/// processes apply five commits each at once through the host; two clients
/// race to commit proposals made at the same version; a direct open is
/// refused while the host holds the project. The log replays to exactly the
/// commits that were answered.
#[test]
fn concurrent_clients_cannot_corrupt_the_project() {
    let dir = fresh_project("race");
    let (shared, server, _) = start(&dir);
    let clients: Vec<_> = (0..8u32)
        .map(|c| {
            let dir = dir.clone();
            std::thread::spawn(move || {
                (0..5u32)
                    .map(|i| archviz("apply", &dir, &create_node(c * 10 + i + 1)))
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let mut versions = Vec::new();
    for client in clients {
        for (out, status) in client.join().unwrap_or_else(|_| panic!("client panicked")) {
            assert_eq!(status, 0, "{out}");
            versions.push(json(&out)["result"]["version"].as_u64().unwrap_or(0));
        }
    }
    versions.sort_unstable();
    assert_eq!(versions, (1..=40).collect::<Vec<u64>>(), "each commit once");

    // Two proposals at version 40, committed at once: one wins, one is stale.
    let actions: Vec<String> = [101u32, 102]
        .iter()
        .map(|&n| {
            let req = json(&create_node(n));
            serde_json::to_string(&req["action"]).unwrap_or_default()
        })
        .collect();
    let commits: Vec<String> = actions
        .iter()
        .map(|action| {
            let (out, status) = archviz("propose", &dir, &format!(r#"{{"action":{action}}}"#));
            assert_eq!(status, 0, "{out}");
            let id = json(&out)["result"]["proposal_id"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            format!(r#"{{"proposal_id":"{id}","base_version":40,"action":{action}}}"#)
        })
        .collect();
    let racers: Vec<_> = commits
        .into_iter()
        .map(|commit| {
            let dir = dir.clone();
            std::thread::spawn(move || archviz("commit", &dir, &commit))
        })
        .collect();
    let mut codes: Vec<String> = racers
        .into_iter()
        .map(|r| {
            let (out, _) = r.join().unwrap_or_else(|_| panic!("racer panicked"));
            match json(&out)["ok"].as_bool() {
                Some(true) => "OK".to_string(),
                _ => json(&out)["error"]["code"]
                    .as_str()
                    .unwrap_or("?")
                    .to_string(),
            }
        })
        .collect();
    codes.sort();
    assert_eq!(codes, ["OK", "STALE"]);

    // Nobody opens the files directly while the host holds them.
    let direct = store::open(&dir).err().map(|e| e.code);
    assert_eq!(direct, Some(archviz_cli::ErrorCode::Locked));

    assert_eq!(version(&shared), 41);
    drop(server);
    drop(shared);
    let project = store::open(&dir).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(project.history.version(), 41);
    assert_eq!(project.history.document().nodes.len(), 41);
    let log = std::fs::read_to_string(dir.join("log.jsonl")).unwrap_or_default();
    assert_eq!(log.lines().count(), 41);
    drop(project);
    let _ = std::fs::remove_dir_all(&dir);
}

/// DEC-13: a request with the wrong token is refused and changes nothing.
#[test]
fn a_wrong_token_is_refused() {
    let dir = fresh_project("token");
    let (shared, server, changes) = start(&dir);
    let file = json(&std::fs::read_to_string(dir.join(HOST_FILE)).unwrap_or_default());
    let pipe = file["pipe"].as_str().unwrap_or_default().to_string();
    let token = file["token"].as_str().unwrap_or_default().to_string();
    assert_eq!(token.len(), 64, "256 bits in hex");
    let args = vec!["apply".to_string(), dir.display().to_string()];
    let (out, status) =
        request(&pipe, &"0".repeat(64), &args, &create_node(1)).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(json(&out)["error"]["code"], "UNAUTHORIZED", "{out}");
    assert_eq!(status, 14);
    assert_eq!(version(&shared), 0);
    assert_eq!(changes.load(Ordering::SeqCst), 0);
    let (out, status) =
        request(&pipe, &token, &args, &create_node(1)).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(status, 0, "{out}");
    assert_eq!(version(&shared), 1);
    drop(server);
    drop(shared);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A host.json left by a host that is gone (its pipe answers nobody and the
/// lock is free) does not block the CLI: it opens the project directly and
/// removes the stale file.
#[test]
fn a_stale_host_file_falls_back_to_the_files() {
    let dir = fresh_project("stale");
    std::fs::write(
        dir.join(HOST_FILE),
        format!(
            r#"{{"pipe":"archviz-gone-{}","token":"{}"}}"#,
            std::process::id(),
            "0".repeat(64)
        ),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let (out, status) = archviz("apply", &dir, &create_node(1));
    assert_eq!(status, 0, "{out}");
    assert!(!dir.join(HOST_FILE).exists());
    let _ = std::fs::remove_dir_all(&dir);
}
