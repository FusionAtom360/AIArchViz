//! An open project in the app (T-201, spec 3.1, DEC-13). The app is the host
//! process: while a session is open it holds the project lock and serves the
//! command API on its pipe, so CLI and agent commands sent to the folder run
//! against the project shown here. The session has no business logic: every
//! change goes through the archviz_cli command API.
//!
//! Save and autosave (Q-77 (x)): each commit is flushed to log.jsonl before
//! it is answered, so an open project never holds unsaved committed work;
//! "Save a copy as" writes the project into a new folder.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use archviz_cli::host::{Notify, Server, Shared, serve};
use archviz_cli::{CliError, Host, NewRequest, new_project, store};
use archviz_core::Uuid;

/// The real clock and random ids for commands the app hosts (Q-49).
pub struct System;

impl Host for System {
    fn now_ms(&mut self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }

    fn new_uuid(&mut self) -> Uuid {
        Uuid::from_bytes(*uuid::Uuid::new_v4().as_bytes())
    }
}

/// What the app shows of the open project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub version: u64,
    /// Each entity collection of the document and how many entities it holds.
    pub collections: Vec<(String, usize)>,
    /// Wall labels, in document order.
    pub walls: Vec<String>,
}

/// An open project. Fields drop in order: the server stops (removing
/// host.json) before the project closes and releases its lock.
pub struct Session {
    _server: Server,
    shared: Arc<Mutex<Shared>>,
    dir: PathBuf,
}

impl Session {
    /// Opens the project in `dir` and starts hosting it. `notify` is called
    /// after a client commits through the pipe.
    pub fn open(dir: &Path, notify: Notify) -> Result<Session, CliError> {
        let project = store::open(dir)?;
        let shared = Arc::new(Mutex::new(Shared {
            project,
            host: Box::new(System),
        }));
        let server = serve(&shared, dir, notify)?;
        Ok(Session {
            _server: server,
            shared,
            dir: dir.to_path_buf(),
        })
    }

    /// Creates a project in `dir` (as `archviz new` does) and opens it.
    pub fn create(
        dir: &Path,
        name: &str,
        building: &str,
        notify: Notify,
    ) -> Result<Session, CliError> {
        let req = NewRequest {
            name: name.to_string(),
            building: building.to_string(),
            project_id: None,
            building_id: None,
        };
        new_project(dir, req, &mut System)?;
        Session::open(dir, notify)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The committed version, cheap enough to check every frame.
    pub fn version(&self) -> u64 {
        self.lock().project.history.version()
    }

    /// What the app shows of the project now.
    pub fn view(&self) -> View {
        let shared = self.lock();
        let history = &shared.project.history;
        let doc = history.document();
        let walls = doc.walls.values().map(|w| w.label.to_string()).collect();
        let collections = match serde_json::to_value(doc) {
            Ok(serde_json::Value::Object(fields)) => fields
                .into_iter()
                .filter_map(|(name, value)| match value {
                    serde_json::Value::Object(entities)
                        if entities.values().all(serde_json::Value::is_object) =>
                    {
                        Some((name, entities.len()))
                    }
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        View {
            version: history.version(),
            collections,
            walls,
        }
    }

    /// "Save a copy as": the project's genesis and committed log, written
    /// into `dest`, which must not hold a project already.
    pub fn save_copy_as(&self, dest: &Path) -> Result<(), CliError> {
        self.lock().project.copy_to(dest)
    }
}

#[cfg(test)]
mod tests {
    use super::{Session, View};
    use archviz_cli::host::{HOST_FILE, run_client};
    use archviz_cli::{ErrorCode, Host, store};
    use archviz_core::Uuid;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// The CLI's own host is never used once it attaches.
    struct Unused;

    impl Host for Unused {
        fn now_ms(&mut self) -> i64 {
            0
        }
        fn new_uuid(&mut self) -> Uuid {
            Uuid::from_bytes([0; 16])
        }
    }

    fn cli(sub: &str, dir: &Path, stdin: &str) -> (String, i32) {
        let args = [sub.to_string(), dir.display().to_string()];
        run_client(&args, stdin, &mut Unused)
    }

    fn create_node(n: u32) -> String {
        format!(
            r#"{{"action":{{"kind":"commands","commands":[{{"id":"00000000-0000-4000-8000-{n:012}","idempotency_key":"00000000-0000-4000-9000-{n:012}","kind":"create_node","args":{{"x":0,"y":{y}}}}}]}},"actor":"user"}}"#,
            y = u64::from(n) * 25_400_000
        )
    }

    fn count(view: &View, collection: &str) -> Option<usize> {
        view.collections
            .iter()
            .find(|(name, _)| name == collection)
            .map(|(_, n)| *n)
    }

    /// Done when: CLI commands issued while the app is open appear in the
    /// app. `run_client` is what the `archviz` binary runs.
    #[test]
    fn cli_commands_issued_while_the_app_is_open_appear_in_the_app() {
        let base = std::env::temp_dir().join(format!("archviz-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("project");
        let repaints = Arc::new(AtomicU64::new(0));
        let counter = repaints.clone();
        let session = Session::create(
            &dir,
            "Shop",
            "Main",
            Box::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(dir.join(HOST_FILE).is_file());
        let before = session.view();
        assert_eq!(before.version, 0);
        assert_eq!(count(&before, "nodes"), Some(0));
        assert!(before.walls.is_empty());

        for n in 1..=2 {
            let (out, status) = cli("apply", &dir, &create_node(n));
            assert_eq!(status, 0, "{out}");
        }
        assert_eq!(repaints.load(Ordering::SeqCst), 2, "the app was told twice");
        let after = session.view();
        assert_eq!(after.version, 2);
        assert_eq!(count(&after, "nodes"), Some(2));

        let (out, status) = cli("undo", &dir, "");
        assert_eq!(status, 0, "{out}");
        assert_eq!(session.version(), 3);
        assert_eq!(count(&session.view(), "nodes"), Some(1));

        // Save a copy: the copy replays to the same version.
        let copy = base.join("copy");
        session
            .save_copy_as(&copy)
            .unwrap_or_else(|e| panic!("{e:?}"));
        let copied = store::open(&copy).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(copied.history.version(), 3);
        drop(copied);

        // A second app cannot open the project while this one has it.
        let second = Session::open(&dir, Box::new(|| {})).err().map(|e| e.code);
        assert_eq!(second, Some(ErrorCode::Locked));

        drop(session);
        assert!(!dir.join(HOST_FILE).exists(), "closing removes host.json");
        let (out, status) = cli("query", &dir, r#"{"collection":"nodes"}"#);
        assert_eq!(status, 0, "{out}");
        let _ = std::fs::remove_dir_all(&base);
    }
}
