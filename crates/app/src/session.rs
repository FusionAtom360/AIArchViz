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

use crate::review::Confirmed;

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

    /// The solver's report on the committed document (`solve`): positions,
    /// conflicts and underconstrained classes (7.3).
    pub fn solve(&self) -> Result<serde_json::Value, String> {
        self.request("solve", &serde_json::json!({}))
    }

    /// The parameters still to supply or confirm (`required-parameters`, 7.4).
    pub fn required_parameters(&self) -> Result<serde_json::Value, String> {
        self.request("required-parameters", &serde_json::json!({}))
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

    /// Commits a proposal through the command API (`commit`) once the user
    /// has confirmed it in the review panel (DEC-10, T-204), and returns the
    /// new version. Fails, committing nothing, if the project changed since
    /// it was proposed or the proposal is blocked.
    pub fn commit(&self, confirmed: &Confirmed) -> Result<u64, String> {
        let pending = confirmed.pending();
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
/// What the review panel's Confirm gives for `pending` (T-204).
pub(crate) fn confirmed(session: &Session, pending: Pending) -> Confirmed {
    crate::review::Review::new(&session.document(), pending).confirm()
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
            .update(&scene, v.to_world(release), v.nm(8.0), GRID_NM)
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
    use super::{Session, confirmed};
    use crate::draw::tests::empty_project;
    use crate::draw::{Click, WallTool, run_commands, setup_actions, storey_for};
    use crate::scene::Scene;
    use crate::snap::visible_step;
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
        assert_eq!(session.commit(&confirmed(&session, pending.clone())), Ok(1));
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
        // About 1.37 px per inch: the visible step is 1 ft (Q-79).
        v.zoom_about(Screen { x: 611.3, y: 377.9 }, 1.37);
        let step = visible_step(v.scale());
        assert_eq!(step, 304_800_000);
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
            match tool.click(&scene, at, tolerance, step) {
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
        assert_eq!(session.commit(&confirmed(&session, pending.clone())), Ok(2));

        let doc = session.document();
        assert_eq!(doc.walls.len(), 4);
        assert_eq!(doc.nodes.len(), 4);
        // The clicks land on the 1 ft lines the user sees.
        let mut corners: Vec<(String, String)> = doc
            .nodes
            .values()
            .map(|n| (format_length(n.x), format_length(n.y)))
            .collect();
        corners.sort();
        assert_eq!(
            corners,
            [
                ("0'-0\"".to_string(), "0'-0\"".to_string()),
                ("0'-0\"".to_string(), "10'-0\"".to_string()),
                ("12'-0\"".to_string(), "0'-0\"".to_string()),
                ("12'-0\"".to_string(), "10'-0\"".to_string()),
            ]
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
        assert_eq!(session.commit(&confirmed(&session, first)), Ok(1));
        assert!(
            session.commit(&confirmed(&session, second)).is_err(),
            "proposed at version 0"
        );
        assert_eq!(session.version(), 1);
    }
}

#[cfg(test)]
pub(crate) mod dimension_tests {
    use super::{Pending, Session, confirmed};
    use crate::dimension::{
        DimensionTool, Pick, conflicts, distance_action, parse_value, pick, underconstrained_walls,
    };
    use crate::draw::tests::empty_project;
    use crate::draw::{Run, run_commands, setup_actions, storey_for};
    use crate::interview::{set_layers, total, wall_groups, with_total};
    use crate::scene::{Scene, WallShape};
    use archviz_core::{
        Action, Axis, Command, Direction, Document, Length, Reference, Source, WallSide,
        format_length, reference_coordinate,
    };
    use std::collections::BTreeSet;

    const FT: i64 = 304_800_000;
    const IN: i64 = 25_400_000;

    fn commands(session: &Session, actions: Vec<Action>) -> Vec<Command> {
        actions
            .into_iter()
            .map(|action| {
                let (id, idempotency_key) = session.ids();
                Command {
                    id,
                    idempotency_key,
                    action,
                }
            })
            .collect()
    }

    fn confirm(session: &Session, pending: &Pending) -> u64 {
        session
            .commit(&confirmed(session, pending.clone()))
            .unwrap_or_else(|e| panic!("{e}"))
    }

    fn wall(scene: &Scene, direction: Direction) -> WallShape {
        scene
            .walls
            .iter()
            .find(|w| w.direction == direction)
            .cloned()
            .unwrap_or_else(|| panic!("a {direction:?} wall"))
    }

    /// A point 1/32 in inside a wall's right (outer) face, halfway along it.
    fn on_right_face(w: &WallShape) -> (i64, i64) {
        let inward = if w.right < w.left { IN / 32 } else { -IN / 32 };
        let mid = |axis: Axis| {
            let v = w.footprint.iter().map(|p| match axis {
                Axis::X => p.0,
                Axis::Y => p.1,
            });
            (v.clone().min().unwrap_or(0) + v.max().unwrap_or(0)) / 2
        };
        match w.across {
            Axis::X => (w.right + inward, mid(Axis::Y)),
            Axis::Y => (mid(Axis::X), w.right + inward),
        }
    }

    fn face(doc: &Document, wall: &WallShape) -> String {
        let r = Reference::WallFace {
            wall: wall.id,
            side: WallSide::Right,
            boundary: 0,
        };
        format_length(reference_coordinate(doc, &r, wall.across).unwrap_or_else(|e| panic!("{e}")))
    }

    /// Picks two points in the plan, types `text`, checks the parser echo
    /// and proposes the dimension, as the dimension box does.
    fn dimension(
        session: &Session,
        a: (i64, i64),
        b: (i64, i64),
        text: &str,
        echo: &str,
    ) -> Pending {
        let doc = session.document();
        let scene = Scene::build(&doc).unwrap_or_else(|e| panic!("{e}"));
        // 1 px = 1/4 in; the pick tolerance is 8 px.
        let tolerance = 2 * IN;
        let mut tool = DimensionTool::default();
        assert_eq!(
            tool.click(pick(&scene, a, tolerance)),
            Pick::First,
            "first pick at {a:?}"
        );
        let Pick::Ready(pair) = tool.click(pick(&scene, b, tolerance)) else {
            panic!("second pick at {b:?}")
        };
        let value = parse_value(text, doc.project.default_unit).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(format_length(value), echo, "parser echo");
        let action = distance_action(&doc, &pair, value, false).unwrap_or_else(|e| panic!("{e}"));
        session
            .propose(action)
            .unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    /// The room the T-203 test builds, as the app builds it: setup, four
    /// drawn walls, the four typed dimensions and a 6" total, each
    /// confirmed. The project folder and the finished document.
    pub(crate) fn t203_room(tag: &str) -> (std::path::PathBuf, Document) {
        let (dir, _) = empty_project(tag);
        let session = Session::open(&dir, Box::new(|| {})).unwrap_or_else(|e| panic!("{e:?}"));
        let setup = commands(&session, setup_actions(&session.document()));
        let pending = session
            .propose_commands(setup)
            .unwrap_or_else(|e| panic!("{e}"));
        confirm(&session, &pending);
        let storey = storey_for(&session.document()).unwrap_or_else(|| panic!("a storey"));
        let run = Run {
            points: vec![(0, 0), (10 * FT, 0), (10 * FT, 10 * FT), (0, 10 * FT)],
            closed: true,
        };
        let walls = run_commands(&run, storey, || session.ids()).unwrap_or_else(|| panic!());
        let pending = session
            .propose_commands(walls)
            .unwrap_or_else(|e| panic!("{e}"));
        confirm(&session, &pending);
        let typed = [
            (None, Direction::NegY, "0\"", "0'-0\""),
            (None, Direction::PosX, "0 in", "0'-0\""),
            (
                Some(Direction::NegY),
                Direction::PosY,
                "14' 6 1/2\"",
                "14'-6 1/2\"",
            ),
            (Some(Direction::PosX), Direction::NegX, "135in", "11'-3\""),
        ];
        for (from, to, text, echo) in typed {
            let now = Scene::build(&session.document()).unwrap_or_else(|e| panic!("{e}"));
            let a = from.map_or((3, -2), |d| on_right_face(&wall(&now, d)));
            let b = on_right_face(&wall(&now, to));
            let pending = dimension(&session, a, b, text, echo);
            confirm(&session, &pending);
        }
        let doc = session.document();
        let groups = wall_groups(&doc);
        let group = groups.first().unwrap_or_else(|| panic!("a group"));
        let six = parse_value("6\"", doc.project.default_unit).unwrap_or_else(|e| panic!("{e}"));
        let layers = with_total(&group.layers, six).unwrap_or_else(|e| panic!("{e}"));
        let actions = set_layers(group, &layers).unwrap_or_else(|e| panic!("{e}"));
        let pending = session
            .propose_commands(commands(&session, actions))
            .unwrap_or_else(|e| panic!("{e}"));
        confirm(&session, &pending);
        let doc = session.document();
        drop(session);
        (dir, doc)
    }

    /// Done when: a user can build a rectangular room from four typed
    /// dimensions and a wall thickness, and a conflict names both
    /// constraints. Each step is what the app does on that input.
    #[test]
    fn a_rectangular_room_from_four_typed_dimensions_and_a_wall_thickness() {
        let (dir, _) = empty_project("dimension-room");
        let session = Session::open(&dir, Box::new(|| {})).unwrap_or_else(|e| panic!("{e:?}"));

        // The storey and origin datum, then four drawn walls at placeholder
        // positions (T-202b), each confirmed.
        let setup = commands(&session, setup_actions(&session.document()));
        let pending = session
            .propose_commands(setup)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(confirm(&session, &pending), 1);
        let storey = storey_for(&session.document()).unwrap_or_else(|| panic!("a storey"));
        let run = Run {
            points: vec![(0, 0), (10 * FT, 0), (10 * FT, 10 * FT), (0, 10 * FT)],
            closed: true,
        };
        let walls = run_commands(&run, storey, || session.ids()).unwrap_or_else(|| panic!());
        let pending = session
            .propose_commands(walls)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(confirm(&session, &pending), 2);

        // Nothing dimensions the walls yet: all four are underconstrained.
        let doc = session.document();
        let solved = session.solve().unwrap_or_else(|e| panic!("{e}"));
        let all: BTreeSet<_> = doc.walls.keys().copied().collect();
        assert_eq!(underconstrained_walls(&doc, &solved), all);

        let scene = Scene::build(&doc).unwrap_or_else(|e| panic!("{e}"));
        let south = wall(&scene, Direction::PosX);
        let east = wall(&scene, Direction::PosY);
        let north = wall(&scene, Direction::NegX);
        let west = wall(&scene, Direction::NegY);
        // Four typed dimensions: the datum to the west and south outer
        // faces, then the width and the depth between outer faces. Each
        // pick is read off the plan as it is after the previous commit;
        // None is the origin datum.
        let typed = [
            (None, Direction::NegY, "0\"", "0'-0\""),
            (None, Direction::PosX, "0 in", "0'-0\""),
            (
                Some(Direction::NegY),
                Direction::PosY,
                "14' 6 1/2\"",
                "14'-6 1/2\"",
            ),
            (Some(Direction::PosX), Direction::NegX, "135in", "11'-3\""),
        ];
        let mut version = 2;
        for (from, to, text, echo) in typed {
            let now = Scene::build(&session.document()).unwrap_or_else(|e| panic!("{e}"));
            let a = from.map_or((3, -2), |d| on_right_face(&wall(&now, d)));
            let b = on_right_face(&wall(&now, to));
            let pending = dimension(&session, a, b, text, echo);
            let line = pending.echo().join(" ");
            assert!(line.contains(echo), "proposal echo {line:?} shows {echo}");
            assert_eq!(session.version(), version, "nothing commits unconfirmed");
            version += 1;
            assert_eq!(confirm(&session, &pending), version);
        }
        let doc = session.document();
        let solved = session.solve().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(underconstrained_walls(&doc, &solved), BTreeSet::new());

        // The interview panel groups the four catalog walls; a typed 6"
        // total sets the stud to 5" for every wall in the group.
        let groups = wall_groups(&doc);
        assert_eq!(groups.len(), 1, "{groups:?}");
        let group = &groups[0];
        assert_eq!(group.walls.len(), 4);
        assert_eq!(
            total(&group.layers).map(format_length),
            Ok("0'-4 1/2\"".to_string())
        );
        let six = parse_value("6\"", doc.project.default_unit).unwrap_or_else(|e| panic!("{e}"));
        let layers = with_total(&group.layers, six).unwrap_or_else(|e| panic!("{e}"));
        let actions = set_layers(group, &layers).unwrap_or_else(|e| panic!("{e}"));
        let pending = session
            .propose_commands(commands(&session, actions))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(pending.echo().len(), 4, "{:?}", pending.echo());
        version += 1;
        assert_eq!(confirm(&session, &pending), version);

        // The room: outer faces where the dimensions put them, 6" walls,
        // nothing free and nothing in conflict.
        let doc = session.document();
        let solved = session.solve().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(underconstrained_walls(&doc, &solved), BTreeSet::new());
        assert_eq!(solved["conflicts"], serde_json::json!([]));
        assert_eq!(face(&doc, &west), "0'-0\"");
        assert_eq!(face(&doc, &south), "0'-0\"");
        assert_eq!(face(&doc, &east), "14'-6 1/2\"");
        assert_eq!(face(&doc, &north), "11'-3\"");
        for w in doc.walls.values() {
            let set = doc
                .layer_sets
                .get(&w.layer_set)
                .unwrap_or_else(|| panic!("layer set"));
            let sum: i64 = set
                .layers
                .iter()
                .map(|l| l.thickness.value.nanometers())
                .sum();
            assert_eq!(sum, 6 * IN);
            let core = set
                .layers
                .iter()
                .find(|l| l.core)
                .unwrap_or_else(|| panic!());
            assert_eq!(core.thickness.value, Length::from_nanometers(5 * IN));
            assert_eq!(core.thickness.source, Source::UserStated);
            // The reference line stays on the core center.
            assert_eq!(w.left_extent.value, Length::from_nanometers(3 * IN));
            assert_eq!(w.left_extent.source, Source::Derived);
        }
        let mut corners: Vec<(String, String)> = doc
            .nodes
            .values()
            .map(|n| (format_length(n.x), format_length(n.y)))
            .collect();
        corners.sort();
        assert_eq!(
            corners,
            [
                ("0'-3\"".to_string(), "0'-3\"".to_string()),
                ("0'-3\"".to_string(), "11'-0\"".to_string()),
                ("14'-3 1/2\"".to_string(), "0'-3\"".to_string()),
                ("14'-3 1/2\"".to_string(), "11'-0\"".to_string()),
            ]
        );
        // The interview no longer asks for the core or the left extent.
        let rows = session
            .required_parameters()
            .unwrap_or_else(|e| panic!("{e}"));
        for row in rows["parameters"].as_array().into_iter().flatten() {
            let field = row["field"].as_str().unwrap_or_default();
            assert_ne!(field, "/left_extent", "{row}");
            assert_ne!(field, "/layers/1/thickness", "{row}");
        }

        // A fifth dimension that disagrees: a conflict naming both, which
        // cannot be committed.
        let scene = Scene::build(&doc).unwrap_or_else(|e| panic!("{e}"));
        let west = wall(&scene, Direction::NegY);
        let east = wall(&scene, Direction::PosY);
        let pending = dimension(
            &session,
            on_right_face(&west),
            on_right_face(&east),
            "15'",
            "15'-0\"",
        );
        let report = conflicts(&doc, &pending);
        let named = format!(
            "the distance from {} right face to {} right face along X",
            west.label, east.label
        );
        assert_eq!(
            report.lines,
            [format!(
                "{named} (15'-0\") conflicts with {named} (14'-6 1/2\")"
            )]
        );
        assert_eq!(report.walls, BTreeSet::from([west.id, east.id]));
        assert!(
            session
                .commit(&confirmed(&session, pending.clone()))
                .is_err(),
            "a conflict blocks commit"
        );
        assert_eq!(session.version(), version);
    }
}

#[cfg(test)]
mod review_tests {
    use super::Session;
    use crate::draw::tests::empty_project;
    use crate::draw::{Run, run_commands, setup_actions, storey_for};
    use crate::review::{Review, Settings};
    use crate::scene::tests::shop;
    use archviz_core::{
        Action, Command, CreateNode, Length, Motion, MoveWall, Reference, WallSide,
    };

    const FT: i64 = 304_800_000;
    const ON: Settings = Settings {
        auto_commit_unambiguous: true,
    };

    fn commands(session: &Session, actions: Vec<Action>) -> Vec<Command> {
        actions
            .into_iter()
            .map(|action| {
                let (id, idempotency_key) = session.ids();
                Command {
                    id,
                    idempotency_key,
                    action,
                }
            })
            .collect()
    }

    /// Done when: no commit path bypasses the review panel except the
    /// explicit auto_commit_unambiguous setting, off by default. Each kind
    /// of proposal the app makes (setup, a drawn run, a drag move, a plain
    /// edit) waits for Confirm under the default settings; with the setting
    /// on, only the unambiguous one commits without it.
    #[test]
    fn only_confirm_or_the_opt_in_setting_commits_a_proposal() {
        let defaults = Settings::default();
        let (dir, _) = empty_project("review-flow");
        let session = Session::open(&dir, Box::new(|| {})).unwrap_or_else(|e| panic!("{e:?}"));

        // The storey and origin datum: Proposed values, so never automatic.
        let setup = commands(&session, setup_actions(&session.document()));
        let pending = session
            .propose_commands(setup)
            .unwrap_or_else(|e| panic!("{e}"));
        let review = Review::new(&session.document(), pending);
        assert!(!review.proposed.is_empty(), "{review:?}");
        assert!(review.auto(&defaults).is_none());
        assert!(review.auto(&ON).is_none());
        assert_eq!(session.version(), 0, "nothing commits before Confirm");
        assert_eq!(session.commit(&review.confirm()), Ok(1));

        // A drawn run of catalog walls: Proposed layers.
        let storey = storey_for(&session.document()).unwrap_or_else(|| panic!("a storey"));
        let run = Run {
            points: vec![(0, 0), (10 * FT, 0), (10 * FT, 8 * FT)],
            closed: false,
        };
        let walls = run_commands(&run, storey, || session.ids()).unwrap_or_else(|| panic!());
        let pending = session
            .propose_commands(walls)
            .unwrap_or_else(|e| panic!("{e}"));
        let review = Review::new(&session.document(), pending);
        assert_eq!(review.echo.len(), 5, "{:?}", review.echo);
        assert!(review.diff.iter().any(|l| l.starts_with("Added wall W-")));
        assert!(!review.proposed.is_empty());
        assert!(review.auto(&defaults).is_none());
        assert!(review.auto(&ON).is_none());
        assert_eq!(session.version(), 1);
        assert_eq!(session.commit(&review.confirm()), Ok(2));

        // A plain edit with no ambiguity, Proposed value or consequence:
        // still waits under the default settings; commits on its own only
        // once the user turns the setting on.
        let node = Action::CreateNode(CreateNode {
            x: Length::from_nanometers(20 * FT),
            y: Length::from_nanometers(0),
        });
        let pending = session.propose(node).unwrap_or_else(|e| panic!("{e}"));
        let review = Review::new(&session.document(), pending);
        assert!(review.unambiguous(), "{review:?}");
        assert_eq!(review.diff, ["Added node at (20'-0\", 0'-0\")"]);
        assert!(review.auto(&defaults).is_none());
        assert_eq!(session.version(), 2, "off by default: nothing committed");
        let confirmed = review.auto(&ON).unwrap_or_else(|| panic!("unambiguous"));
        assert_eq!(session.commit(&confirmed), Ok(3));

        // A move (what a drag proposes) whose far side moves with it: the
        // panel lists the consequences, and it never commits on its own.
        let (dir, doc) = shop("review-move");
        let session = Session::open(&dir, Box::new(|| {})).unwrap_or_else(|e| panic!("{e:?}"));
        let version = session.version();
        let (id, _) = doc
            .walls
            .iter()
            .find(|(_, w)| w.label.to_string() == "W-013")
            .unwrap_or_else(|| panic!("W-013"));
        let action = Action::MoveWall(MoveWall {
            wall: *id,
            reference: Reference::WallFace {
                wall: *id,
                side: WallSide::Left,
                boundary: 0,
            },
            motion: Motion::Delta {
                delta: Length::from_nanometers(FT),
            },
            edit: None,
            absorb: Vec::new(),
        });
        let pending = session.propose(action).unwrap_or_else(|e| panic!("{e}"));
        let review = Review::new(&doc, pending);
        assert!(
            review.echo[0].starts_with("Move W-013 left face 1'-0\" toward +Y"),
            "{:?}",
            review.echo
        );
        assert_eq!(
            review.consequences,
            ["wall W-014", "wall W-003", "wall W-004"]
        );
        assert_eq!(
            review.diff[..2],
            [
                "Changed the distance from W-001 right face to W-013 right face along Y",
                "  /kind/value: 3'-0\" -> 4'-0\"",
            ]
        );
        assert!(review.blocked.is_empty(), "{:?}", review.blocked);
        assert!(review.auto(&defaults).is_none());
        assert!(review.auto(&ON).is_none(), "consequence moves need review");
        assert_eq!(session.version(), version);
        assert_eq!(session.commit(&review.confirm()), Ok(version + 1));
    }

    /// The panel names a move's alternatives (8.7 (b)) and its violations
    /// with lengths in feet-inches, and a blocked proposal cannot commit.
    #[test]
    fn the_panel_shows_alternatives_and_violations_in_feet_and_inches() {
        let (dir, doc) = shop("review-alternatives");
        let session = Session::open(&dir, Box::new(|| {})).unwrap_or_else(|e| panic!("{e:?}"));
        let version = session.version();
        let (id, _) = doc
            .walls
            .iter()
            .find(|(_, w)| w.label.to_string() == "W-003")
            .unwrap_or_else(|| panic!("W-003"));
        let action = Action::MoveWall(MoveWall {
            wall: *id,
            reference: Reference::WallFace {
                wall: *id,
                side: WallSide::Left,
                boundary: 0,
            },
            motion: Motion::Delta {
                delta: Length::from_nanometers(FT),
            },
            edit: None,
            absorb: Vec::new(),
        });
        let pending = session.propose(action).unwrap_or_else(|e| panic!("{e}"));
        let review = Review::new(&doc, pending);
        assert_eq!(review.alternatives.len(), 3, "{:?}", review.alternatives);
        assert!(
            review.alternatives[1].starts_with(
                "door in W-007: set the distance from W-009 right face to W-003 right face \
                 along X to 19'-0\", the offset of door in W-007 to 10'-0\"; moves "
            ),
            "{:?}",
            review.alternatives
        );
        assert_eq!(
            review.violations,
            [
                "Opening outside host: window in W-002 (opening too close to start clearance -0'-4\")"
            ]
        );
        assert_eq!(review.existing, ["door in W-007"]);
        assert!(review.ambiguities.is_empty());
        assert_eq!(review.blocked, ["it leaves violations"]);
        assert!(review.auto(&ON).is_none());
        assert!(session.commit(&review.confirm()).is_err(), "blocked");
        assert_eq!(session.version(), version);
    }
}
