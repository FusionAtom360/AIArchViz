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
use archviz_cli::{CliError, Host, NewRequest, new_project, run_on, store};
use archviz_core::{Action, Command, Document, ProposedAction, Uuid};

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

/// A proposal made through the command API, waiting for the user to confirm
/// or discard it.
#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    /// The action as sent, which a commit sends again.
    pub action: serde_json::Value,
    /// The proposal as the command API returned it.
    pub result: serde_json::Value,
}

impl Pending {
    /// The echo text the user confirms (9.4).
    pub fn echo(&self) -> Vec<String> {
        self.result["echo"]
            .as_array()
            .map(|lines| {
                lines
                    .iter()
                    .filter_map(|l| l.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }
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

    /// The committed document, for the plan view.
    pub fn document(&self) -> Document {
        self.lock().project.history.document().clone()
    }

    /// A fresh (command id, idempotency key) pair.
    pub fn ids(&self) -> (Uuid, Uuid) {
        let mut shared = self.lock();
        (shared.host.new_uuid(), shared.host.new_uuid())
    }

    /// Runs one command API request on the hosted project and returns its
    /// result, or the error message.
    fn request(&self, sub: &str, body: &serde_json::Value) -> Result<serde_json::Value, String> {
        let mut shared = self.lock();
        let args = [sub.to_string(), self.dir.display().to_string()];
        let Shared { project, host } = &mut *shared;
        let (out, _) = run_on(project, &args, &body.to_string(), host.as_mut());
        let response: serde_json::Value = serde_json::from_str(&out).map_err(|e| e.to_string())?;
        if response["ok"] == serde_json::Value::Bool(true) {
            Ok(response["result"].clone())
        } else {
            Err(response["error"]["message"]
                .as_str()
                .unwrap_or("the request failed")
                .to_string())
        }
    }

    /// Proposes `action` as one command (see `propose_commands`).
    pub fn propose(&self, action: Action) -> Result<Pending, String> {
        let (id, idempotency_key) = self.ids();
        self.propose_commands(vec![Command {
            id,
            idempotency_key,
            action,
        }])
    }

    /// Proposes `commands` as one transaction through the command API
    /// (`propose`, which stores nothing).
    pub fn propose_commands(&self, commands: Vec<Command>) -> Result<Pending, String> {
        let action = serde_json::to_value(ProposedAction::Commands { commands })
            .map_err(|e| e.to_string())?;
        let result = self.request("propose", &serde_json::json!({ "action": action }))?;
        Ok(Pending { action, result })
    }

    /// Commits `pending` through the command API (`commit`) once the user
    /// has confirmed its echo (DEC-10), and returns the new version. Fails,
    /// committing nothing, if the project changed since it was proposed or
    /// the proposal is blocked.
    pub fn commit(&self, pending: &Pending) -> Result<u64, String> {
        let request = serde_json::json!({
            "proposal_id": pending.result["proposal_id"],
            "base_version": pending.result["base_version"],
            "action": pending.action,
            "actor": "user",
        });
        self.request("commit", &request)?;
        Ok(self.version())
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

#[cfg(test)]
mod plan_tests {
    use super::Session;
    use crate::drag::Drag;
    use crate::scene::tests::shop;
    use crate::scene::{Entity, Scene};
    use crate::viewport::{Screen, Viewport};
    use archviz_core::GRID_NM;

    /// Done when (part 2): a drag in the plan view, from fractional screen
    /// positions, yields a command whose value is a snapped integer, and the
    /// command API accepts it as a proposal without committing anything.
    #[test]
    fn a_drag_yields_a_snapped_move_the_command_api_proposes() {
        let (dir, _) = shop("session-drag");
        let session = Session::open(&dir, Box::new(|| {})).unwrap_or_else(|e| panic!("{e:?}"));
        let version = session.version();
        let scene = Scene::build(&session.document()).unwrap_or_else(|e| panic!("{e}"));
        let ((x0, y0), (x1, y1)) = scene.bounds().unwrap_or(((0, 0), (0, 0)));
        let mut v = Viewport::fit(
            (x0, y0),
            (x1, y1),
            (1280.0, 800.0),
            40.0,
            Screen { x: 640.0, y: 400.0 },
        );
        // Zoom in on a wall, then grab its face and drag by an awkward amount.
        let w = scene
            .walls
            .iter()
            .find(|w| w.across == archviz_core::Axis::X)
            .unwrap_or_else(|| panic!("a north-south wall"));
        let y_mid = w.footprint.iter().map(|p| p.1).sum::<i64>() / w.footprint.len() as i64;
        let face = v.to_screen((w.left, y_mid));
        v.zoom_about(face, 37.3);
        let press = v.to_screen((w.left, y_mid));
        let press = Screen {
            x: press.x + 0.37,
            y: press.y - 0.21,
        };
        let release = Screen {
            x: press.x + 123.456,
            y: press.y + 7.7,
        };
        let at = v.to_world(press);
        assert_eq!(scene.pick(at, v.nm(6.0)), Some(Entity::Wall(w.id)));
        let drag = Drag::begin(&scene, Entity::Wall(w.id), at).unwrap_or_else(|| panic!());
        let preview = drag
            .update(&scene, v.to_world(release), v.nm(8.0))
            .unwrap_or_else(|| panic!());
        let action = preview.action.unwrap_or_else(|| panic!("the wall moved"));
        let proposal = session
            .propose(action)
            .unwrap_or_else(|e| panic!("{e}"))
            .result;
        let command = &proposal["action"]["commands"][0];
        assert_eq!(command["kind"], "move_wall", "{proposal}");
        let target = command["args"]["motion"]["target"]
            .as_i64()
            .unwrap_or_else(|| panic!("{command}"));
        assert_eq!(target % GRID_NM, 0, "{target}");
        assert_eq!(target, preview.snap.value);
        let echo = proposal["echo"][0].as_str().unwrap_or_default();
        assert!(
            echo.starts_with(&format!("Move {} left face", w.label)),
            "{echo}"
        );
        assert_eq!(session.version(), version, "proposing commits nothing");
    }
}

#[cfg(test)]
mod draw_tests {
    use super::Session;
    use crate::draw::tests::empty_project;
    use crate::draw::{Click, WallTool, run_commands, setup_actions, storey_for};
    use crate::scene::Scene;
    use crate::viewport::{Screen, Viewport};
    use archviz_core::{Command, DatumGeometry, GRID_NM, JoinType, Length, Source, format_length};

    /// Done when: a user draws a closed rectangle of four walls in the app,
    /// every node coordinate committed is on the 1/16 in grid (from
    /// fractional pointer positions), and the walls join as an L at each
    /// corner (6.1). Each step is what the app does on that input.
    #[test]
    fn a_closed_rectangle_drawn_from_fractional_pointers_commits_on_the_grid_with_l_joins() {
        let (dir, _) = empty_project("draw-rectangle");
        let session = Session::open(&dir, Box::new(|| {})).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(session.version(), 0);

        // First use: one proposal adds the storey and the origin datum, and
        // commits once its echo is confirmed.
        let setup = setup_actions(&session.document());
        assert_eq!(setup.len(), 2);
        let commands: Vec<Command> = setup
            .into_iter()
            .map(|action| {
                let (id, idempotency_key) = session.ids();
                Command {
                    id,
                    idempotency_key,
                    action,
                }
            })
            .collect();
        let pending = session
            .propose_commands(commands)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(pending.echo().len(), 2, "{:?}", pending.echo());
        assert_eq!(session.version(), 0, "proposing commits nothing");
        assert_eq!(session.commit(&pending), Ok(1));
        let doc = session.document();
        assert!(setup_actions(&doc).is_empty(), "set up once");
        assert!(doc.datums.values().any(|d| d.geometry
            == DatumGeometry::Point {
                x: Length::from_nanometers(0),
                y: Length::from_nanometers(0)
            }));

        // Draw at an awkward zoom, clicking at fractional screen positions
        // that sit between grid values.
        let scene = Scene::build(&doc).unwrap_or_else(|e| panic!("{e}"));
        let mut v = Viewport::new((0, 0), 1.0 / 25_400_000.0, Screen { x: 640.0, y: 400.0 });
        v.zoom_about(Screen { x: 611.3, y: 377.9 }, 0.173);
        let px = v.scale() * 304_800_000.0; // pixels per foot
        let o = v.to_screen((0, 0));
        let clicks = [
            (o.x + 0.37, o.y - 0.41),
            (o.x + 12.0 * px + 0.29, o.y + 0.83),
            (o.x + 12.0 * px - 0.61, o.y - 10.0 * px + 0.17),
            (o.x - 0.77, o.y - 10.0 * px - 0.53),
            (o.x + 0.45, o.y + 0.31),
        ];
        let tolerance = v.nm(8.0);
        let mut tool = WallTool::new();
        let mut run = None;
        for (i, (x, y)) in clicks.into_iter().enumerate() {
            let at = v.to_world(Screen { x, y });
            assert!(
                at.0 % GRID_NM != 0 || at.1 % GRID_NM != 0,
                "click {i} is off the grid"
            );
            match tool.click(&scene, at, tolerance) {
                Click::Added => assert!(i < 4),
                Click::Closed(r) => run = Some(r),
                Click::Ignored(why) => panic!("click {i}: {why}"),
            }
        }
        let run = run.unwrap_or_else(|| panic!("the last click closes the run"));
        let storey = storey_for(&doc).unwrap_or_else(|| panic!("a storey"));
        let commands =
            run_commands(&run, storey, || session.ids()).unwrap_or_else(|| panic!("aligned"));
        let pending = session
            .propose_commands(commands)
            .unwrap_or_else(|e| panic!("{e}"));
        let echo = pending.echo();
        assert_eq!(echo.len(), 8, "{echo:?}");
        assert_eq!(session.version(), 1, "nothing commits before confirmation");
        assert_eq!(session.commit(&pending), Ok(2));

        let doc = session.document();
        assert_eq!(doc.walls.len(), 4);
        assert_eq!(doc.nodes.len(), 4);
        // A rectangle: two x values and two y values, the near corner on the
        // origin datum (snapped to it) and the far one within a pixel of
        // where the user clicked (at this zoom a pixel is about 6 in).
        let xs: std::collections::BTreeSet<i64> =
            doc.nodes.values().map(|n| n.x.nanometers()).collect();
        let ys: std::collections::BTreeSet<i64> =
            doc.nodes.values().map(|n| n.y.nanometers()).collect();
        let (xs, ys): (Vec<i64>, Vec<i64>) = (xs.into_iter().collect(), ys.into_iter().collect());
        assert_eq!((xs.len(), ys.len()), (2, 2), "{xs:?} {ys:?}");
        assert_eq!((xs[0], ys[0]), (0, 0));
        let pixel = v.nm(1.0);
        assert!(
            (xs[1] - 12 * 304_800_000).abs() <= pixel,
            "{}",
            format_length(Length::from_nanometers(xs[1]))
        );
        assert!(
            (ys[1] - 10 * 304_800_000).abs() <= pixel,
            "{}",
            format_length(Length::from_nanometers(ys[1]))
        );
        for n in doc.nodes.values() {
            assert_eq!(n.x.nanometers() % GRID_NM, 0, "{n:?}");
            assert_eq!(n.y.nanometers() % GRID_NM, 0, "{n:?}");
            assert_eq!(n.join, JoinType::L, "{n:?}");
        }
        for w in doc.walls.values() {
            assert_eq!(w.left_extent.source, Source::Proposed);
            let set = doc
                .layer_sets
                .get(&w.layer_set)
                .unwrap_or_else(|| panic!("layer set"));
            assert_eq!(set.layers.len(), 3);
            assert!(
                set.layers
                    .iter()
                    .all(|l| l.thickness.source == Source::Proposed)
            );
        }
        // The committed document draws.
        assert_eq!(Scene::build(&doc).map(|s| s.walls.len()), Ok(4));
    }

    #[test]
    fn a_discarded_or_stale_proposal_commits_nothing() {
        let (dir, _) = empty_project("draw-stale");
        let session = Session::open(&dir, Box::new(|| {})).unwrap_or_else(|e| panic!("{e:?}"));
        let doc = session.document();
        let setup = |s: &Session| {
            let commands: Vec<Command> = setup_actions(&doc)
                .into_iter()
                .map(|action| {
                    let (id, idempotency_key) = s.ids();
                    Command {
                        id,
                        idempotency_key,
                        action,
                    }
                })
                .collect();
            s.propose_commands(commands)
                .unwrap_or_else(|e| panic!("{e}"))
        };
        let first = setup(&session);
        let second = setup(&session);
        assert_eq!(session.commit(&first), Ok(1));
        assert!(session.commit(&second).is_err(), "proposed at version 0");
        assert_eq!(session.version(), 1);
    }
}
