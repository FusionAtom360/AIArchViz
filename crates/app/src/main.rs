//! C5 app: egui app and wgpu viewer (spec 3, 3.1, D-14).
//!
//! T-201 is the shell: open, create, close and save a copy of a project
//! folder. While a project is open the app is its host process (3.1): CLI
//! and agent commands sent to that folder run here, and the window repaints
//! when one commits.
//!
//! T-202 adds the plan view: walls, openings, slabs, occurrences and labels
//! drawn through a 64-bit transform, pan and zoom, selection, and dragging
//! an entity into a move command snapped to the 1/16 in grid (3.1). A drag
//! is proposed through the command API and reviewed like any proposal.
//!
//! T-202b adds the wall tool (draw.rs). Each drawn run, and on first use the
//! storey and origin datum, is proposed through the command API, its echo
//! shown, and committed only when the user confirms it (DEC-10).
//!
//! T-203 adds dimensions (dimension.rs): pick two references, type a value,
//! check its parser echo, then confirm the proposal echo to commit a
//! Distance constraint. The plan highlights underconstrained walls and the
//! walls of a proposal's conflicts, whose constraints are named. The
//! interview panel (interview.rs) lists required parameters and sets wall
//! thicknesses by group (Q-78 (i), (x)).
//!
//! T-204 adds the review panel (review.rs): every proposal (a drawn run, a
//! drag, a dimension, a thickness) is shown with its echo, diff, violations,
//! ambiguities, Proposed flags, consequences and alternatives, and commits
//! only on Confirm, unless the user turns on auto_commit_unambiguous (off by
//! default) and the proposal is unambiguous (9.4).
//!
//! T-204b (Q-81 (a)): the top bar holds only the project controls, with
//! text boxes four times egui's default width; every message goes to the
//! one status line in the bottom bar, after the mode's instruction and
//! before the pointer coordinates (status.rs).
//!
//! T-204d (Q-81 (b)(i), (c)(i), Q-82): every Distance constraint is drawn on
//! the plan as a dimension (dimension_layout.rs); a click selects one and
//! Delete proposes RemoveConstraint for it, reviewed in the panel.

mod dimension;
mod dimension_layout;
mod drag;
mod draw;
mod interview;
mod plan_view;
mod review;
mod scene;
mod session;
mod snap;
mod status;
mod viewport;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use archviz_core::{
    Action, Axis, Length, ProjectUnit, RemoveConstraint, Source, Uuid, format_length,
};

use eframe::egui;
use plan_view::{Event, PlanView};
use review::Review;
use scene::{Entity, Scene};
use session::{Pending, Session, View};

/// Fonts installed with Windows; the first one found is used. No font ships
/// with the app (DEC-12).
const FONTS: [&str; 3] = [
    r"C:\Windows\Fonts\segoeui.ttf",
    r"C:\Windows\Fonts\arial.ttf",
    r"C:\Windows\Fonts\tahoma.ttf",
];

fn system_fonts() -> Option<egui::FontDefinitions> {
    let bytes = FONTS.iter().find_map(|path| std::fs::read(path).ok())?;
    let mut fonts = egui::FontDefinitions::empty();
    fonts.font_data.insert(
        "system".to_string(),
        Arc::new(egui::FontData::from_owned(bytes)),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("system".to_string());
    }
    Some(fonts)
}

/// The width of each text box in the top bar: four times egui's default
/// `text_edit_width` (280 px), the width T-204 used (Q-81 (a)).
const TOP_TEXT_WIDTH: f32 = 4.0 * 280.0;

/// The ids of the top bar's text boxes, in the order shown.
const TOP_TEXT_BOXES: [&str; 4] = [
    "project-folder",
    "project-name",
    "building-name",
    "save-copy-as",
];

/// A text box in the top bar, `TOP_TEXT_WIDTH` wide. Its space is
/// allocated at that size: a grid cell otherwise offers a text box only the
/// column's width from the previous frame, so the box never grows.
fn top_text(ui: &mut egui::Ui, id: &str, text: &mut String) {
    let height = ui.spacing().interact_size.y;
    ui.add_sized(
        [TOP_TEXT_WIDTH, height],
        egui::TextEdit::singleline(text)
            .id(egui::Id::unique(id))
            .desired_width(TOP_TEXT_WIDTH),
    );
}

/// The plan scene and the version it was built at.
type Built = Option<(u64, Result<Scene, String>)>;

struct App {
    folder: String,
    name: String,
    building: String,
    copy_to: String,
    message: String,
    session: Option<Session>,
    /// The view and the version it was built at.
    view: Option<View>,
    scene: Built,
    plan: PlanView,
    /// A proposal in the review panel, and what it is.
    pending: Option<(String, Review)>,
    /// Review settings; auto-commit is off until the user turns it on.
    settings: review::Settings,
    /// Two picked references and the value being typed for them.
    dimension: Option<(dimension::Pair, String)>,
    /// Required parameters and underconstrained walls, and the version they
    /// were read at.
    checked: Option<(u64, serde_json::Value, BTreeSet<Uuid>)>,
    /// Text typed in the interview panel, by row key.
    typed: BTreeMap<String, String>,
}

const WARN: egui::Color32 = egui::Color32::from_rgb(220, 60, 60);

/// A text box for a length, its parser echo (5.2) and a button; the parsed
/// value when the button is clicked.
fn length_input(
    ui: &mut egui::Ui,
    text: &mut String,
    unit: ProjectUnit,
    button: &str,
) -> Option<Length> {
    let mut out = None;
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(text).desired_width(90.0));
        let parsed = dimension::parse_value(text, unit);
        if !text.trim().is_empty() {
            match &parsed {
                Ok(v) => ui.label(format!("reads as {}", format_length(*v))),
                Err(e) => ui.colored_label(WARN, e),
            };
        }
        if ui
            .add_enabled(parsed.is_ok(), egui::Button::new(button))
            .clicked()
        {
            out = parsed.ok();
        }
    });
    out
}

impl App {
    fn new() -> App {
        App {
            folder: String::new(),
            name: String::new(),
            building: String::new(),
            copy_to: String::new(),
            message: String::new(),
            session: None,
            view: None,
            scene: None,
            plan: PlanView::new(),
            pending: None,
            settings: review::Settings::default(),
            dimension: None,
            checked: None,
            typed: BTreeMap::new(),
        }
    }

    /// Forgets everything shown of the previous project.
    fn reset(&mut self) {
        self.view = None;
        self.scene = None;
        self.plan = PlanView::new();
        self.pending = None;
        self.dimension = None;
        self.checked = None;
        self.typed.clear();
    }

    /// Puts `pending` in the review panel and highlights its conflicts. With
    /// auto_commit_unambiguous on, an unambiguous proposal commits at once.
    fn hold(&mut self, what: &str, pending: Pending) {
        let Some(session) = &self.session else {
            return;
        };
        let review = Review::new(&session.document(), pending);
        if let Some(confirmed) = review.auto(&self.settings) {
            self.message = match session.commit(&confirmed) {
                Ok(version) => format!("{what}: committed as version {version} (unambiguous)."),
                Err(e) => format!("{what}: not committed: {e}"),
            };
            self.release();
            return;
        }
        self.plan.conflicting = review.conflicts.walls.clone();
        self.plan.conflicting_dimensions = review.conflicts.constraints.clone();
        self.pending = Some((what.to_string(), review));
    }

    /// Drops the proposal under review and its highlights.
    fn release(&mut self) {
        self.pending = None;
        self.plan.conflicting.clear();
        self.plan.conflicting_dimensions.clear();
    }

    /// Proposes `commands`; on success it waits for confirmation.
    fn propose(&mut self, what: &str, commands: Vec<archviz_core::Command>) {
        let Some(session) = &self.session else {
            return;
        };
        match session.propose_commands(commands) {
            Ok(pending) => self.hold(what, pending),
            Err(e) => self.message = format!("{what}: {e}"),
        }
    }

    /// Turns wall mode on or off. On first use in a project without a
    /// storey or datum, proposes them (one proposal).
    fn toggle_walls(&mut self) {
        if self.plan.walls.take().is_some() {
            return;
        }
        self.plan.dimensions = None;
        self.plan.walls = Some(draw::WallTool::new());
        let Some(session) = &self.session else {
            return;
        };
        let setup = draw::setup_actions(&session.document());
        if setup.is_empty() {
            return;
        }
        let commands = setup
            .into_iter()
            .map(|action| {
                let (id, idempotency_key) = session.ids();
                archviz_core::Command {
                    id,
                    idempotency_key,
                    action,
                }
            })
            .collect();
        self.propose("Storey and origin datum", commands);
    }

    /// Turns dimension mode on or off.
    fn toggle_dimensions(&mut self) {
        if self.plan.dimensions.take().is_some() {
            return;
        }
        self.plan.walls = None;
        self.plan.dimensions = Some(dimension::DimensionTool::default());
    }

    /// Proposes `actions`, one command each, as one transaction.
    fn propose_actions(&mut self, what: &str, actions: Vec<Action>) {
        let Some(session) = &self.session else {
            return;
        };
        let commands = actions
            .into_iter()
            .map(|action| {
                let (id, idempotency_key) = session.ids();
                archviz_core::Command {
                    id,
                    idempotency_key,
                    action,
                }
            })
            .collect();
        self.propose(what, commands);
    }

    /// The dimension box: the two references, the typed value with its
    /// parser echo (5.2), and Propose once the value parses.
    fn dimension_box(&mut self, ui: &mut egui::Ui) {
        let (Some(session), Some((pair, text))) = (&self.session, &mut self.dimension) else {
            return;
        };
        ui.separator();
        let axis = match pair.axis {
            Axis::X => "X",
            Axis::Y => "Y",
        };
        ui.strong(format!(
            "Dimension from {} to {} along {axis}",
            pair.a.name, pair.b.name
        ));
        let doc = session.document();
        let value = length_input(ui, text, doc.project.default_unit, "Propose");
        let cancel = ui.button("Cancel").clicked();
        let action = value.map(|v| (v, dimension::distance_action(&doc, pair, v)));
        if cancel {
            self.dimension = None;
        }
        match action {
            Some((v, Ok(action))) => {
                self.dimension = None;
                self.propose_actions(&format!("Dimension {}", format_length(v)), vec![action]);
            }
            Some((_, Err(e))) => self.message = format!("Dimension: {e}"),
            None => {}
        }
    }

    /// The interview panel (7.4): each group of walls with the same layers,
    /// its total and each layer, then every required parameter with its
    /// proposed default. Underconstrained walls are counted and drawn orange.
    fn interview(&mut self, ui: &mut egui::Ui) {
        let Some(session) = &self.session else {
            return;
        };
        let version = session.version();
        if self.checked.as_ref().is_none_or(|(v, _, _)| *v != version) {
            let doc = session.document();
            let params = session.required_parameters().unwrap_or_default();
            let free = session
                .solve()
                .map(|s| dimension::underconstrained_walls(&doc, &s))
                .unwrap_or_default();
            self.checked = Some((version, params, free));
        }
        let Some((_, params, free)) = &self.checked else {
            return;
        };
        self.plan.free = free.clone();
        let rows = interview::rows(params);
        let free = free.len();
        let doc = session.document();
        let unit = doc.project.default_unit;
        let mut set: Option<(String, Result<Vec<Action>, String>)> = None;
        ui.separator();
        egui::CollapsingHeader::new(format!("Required parameters ({})", rows.len()))
            .default_open(true)
            .show(ui, |ui| {
                if free > 0 {
                    ui.colored_label(
                        egui::Color32::from_rgb(240, 150, 0),
                        format!("{free} walls are underconstrained (orange): dimension them."),
                    );
                }
                for group in interview::wall_groups(&doc) {
                    let names: Vec<&str> = group.walls.iter().map(|(_, l)| l.as_str()).collect();
                    let names = names.join(", ");
                    let first = group.walls.first().map_or("", |w| w.1.as_str());
                    let total = interview::total(&group.layers)
                        .map(format_length)
                        .unwrap_or_default();
                    let mark = if interview::proposed(&group.layers) {
                        " (Proposed)"
                    } else {
                        ""
                    };
                    ui.strong(format!("Walls {names}: {total} thick{mark}"));
                    ui.label("Total thickness (sets the core layer)");
                    let text = self.typed.entry(format!("{first}/total")).or_default();
                    if let Some(v) = length_input(ui, text, unit, "Set") {
                        let result = interview::with_total(&group.layers, v)
                            .and_then(|layers| interview::set_layers(&group, &layers));
                        set = Some((format!("Thickness of {names}"), result));
                    }
                    for (i, layer) in group.layers.iter().enumerate() {
                        let core = if layer.core { ", core" } else { "" };
                        let source = if layer.thickness.source == Source::Proposed {
                            " (Proposed)"
                        } else {
                            ""
                        };
                        ui.label(format!(
                            "Layer {}{core}: {}{source}",
                            i + 1,
                            format_length(layer.thickness.value)
                        ));
                        let text = self.typed.entry(format!("{first}/{i}")).or_default();
                        if let Some(v) = length_input(ui, text, unit, "Set") {
                            let result = interview::with_layer(&group.layers, i, v)
                                .and_then(|layers| interview::set_layers(&group, &layers));
                            set = Some((format!("Layer {} of {names}", i + 1), result));
                        }
                    }
                }
                ui.separator();
                egui::Grid::new("required").striped(true).show(ui, |ui| {
                    for (what, field, default) in &rows {
                        ui.label(what);
                        ui.label(field);
                        ui.label(default);
                        ui.end_row();
                    }
                });
            });
        match set {
            Some((what, Ok(actions))) => self.propose_actions(&what, actions),
            Some((what, Err(e))) => self.message = format!("{what}: {e}"),
            None => {}
        }
    }

    /// Proposes a drawn run of walls.
    fn propose_run(&mut self, run: &draw::Run) {
        let Some(session) = &self.session else {
            return;
        };
        let Some(storey) = draw::storey_for(&session.document()) else {
            self.message = "Confirm the storey and origin datum before drawing walls.".to_string();
            return;
        };
        match draw::run_commands(run, storey, || session.ids()) {
            Some(commands) => self.propose("Walls", commands),
            None => self.message = "Walls must be horizontal or vertical.".to_string(),
        }
    }

    /// The review panel (8.3, 9.4): the echo the user confirms, then every
    /// part of the proposal, then Confirm or Discard. Confirm is the only
    /// way to commit unless auto_commit_unambiguous is on.
    fn review(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.checkbox(
            &mut self.settings.auto_commit_unambiguous,
            "Commit unambiguous proposals without review",
        )
        .on_hover_text(
            "Off by default. When on, a proposal with no ambiguities, no Proposed \
             values and no consequence moves commits without this panel.",
        );
        let Some((what, review)) = &self.pending else {
            return;
        };
        ui.strong(format!("Review: {what} (proposed, not yet committed)"));
        ui.label("You confirm this text:");
        for line in &review.echo {
            ui.label(egui::RichText::new(line).strong());
        }
        let section = |ui: &mut egui::Ui, title: &str, lines: &[String], warn: bool| {
            if lines.is_empty() {
                return;
            }
            egui::CollapsingHeader::new(format!("{title} ({})", lines.len()))
                .default_open(true)
                .show(ui, |ui| {
                    for line in lines {
                        if warn {
                            ui.colored_label(WARN, line);
                        } else {
                            ui.label(line);
                        }
                    }
                });
        };
        section(ui, "Conflicts", &review.conflicts.lines, true);
        section(ui, "Violations", &review.violations, true);
        section(ui, "Ambiguities", &review.ambiguities, true);
        section(ui, "Moves as a consequence", &review.consequences, false);
        section(ui, "Alternatives", &review.alternatives, false);
        section(ui, "Proposed values", &review.proposed, false);
        section(ui, "Existing elements changed", &review.existing, false);
        section(ui, "Changes", &review.diff, false);
        if review.provisional {
            ui.label("The result rests on Proposed values.");
        }
        let blocked = !review.blocked.is_empty();
        if blocked {
            ui.colored_label(
                WARN,
                format!("Cannot be committed: {}.", review.blocked.join("; ")),
            );
        }
        let mut done = None;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!blocked, egui::Button::new("Confirm"))
                .clicked()
            {
                done = Some(true);
            }
            if ui.button("Discard").clicked() {
                done = Some(false);
            }
        });
        let Some(confirm) = done else {
            return;
        };
        let Some((what, review)) = self.pending.take() else {
            return;
        };
        self.message = match (&self.session, confirm) {
            (Some(session), true) => match session.commit(&review.confirm()) {
                Ok(version) => format!("{what}: committed as version {version}."),
                Err(e) => format!("{what}: not committed: {e}"),
            },
            _ => format!("{what}: discarded."),
        };
        self.release();
    }

    fn opened(&mut self, result: Result<Session, archviz_cli::CliError>) {
        match result {
            Ok(session) => {
                self.message = format!("Opened {}", session.dir().display());
                self.session = Some(session);
                self.reset();
            }
            Err(e) => self.message = e.message,
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let repaint = {
            let ctx = ui.ctx().clone();
            move || -> archviz_cli::host::Notify {
                let ctx = ctx.clone();
                Box::new(move || ctx.request_repaint())
            }
        };
        egui::Grid::new("project").num_columns(3).show(ui, |ui| {
            ui.label("Project folder");
            top_text(ui, TOP_TEXT_BOXES[0], &mut self.folder);
            ui.horizontal(|ui| {
                if ui.button("Open").clicked() {
                    self.session = None;
                    let result = Session::open(Path::new(self.folder.trim()), repaint());
                    self.opened(result);
                }
                if ui
                    .add_enabled(self.session.is_some(), egui::Button::new("Close"))
                    .clicked()
                {
                    self.session = None;
                    self.reset();
                    self.message = "Closed.".to_string();
                }
            });
            ui.end_row();

            ui.label("New project name");
            top_text(ui, TOP_TEXT_BOXES[1], &mut self.name);
            ui.label("");
            ui.end_row();
            ui.label("Building name");
            top_text(ui, TOP_TEXT_BOXES[2], &mut self.building);
            if ui.button("Create in project folder").clicked() {
                self.session = None;
                let result = Session::create(
                    Path::new(self.folder.trim()),
                    self.name.trim(),
                    self.building.trim(),
                    repaint(),
                );
                self.opened(result);
            }
            ui.end_row();

            ui.label("Save a copy as");
            top_text(ui, TOP_TEXT_BOXES[3], &mut self.copy_to);
            if let Some(session) = &self.session {
                if ui.button("Save copy").clicked() {
                    let dest = Path::new(self.copy_to.trim());
                    self.message = match session.save_copy_as(dest) {
                        Ok(()) => format!("Saved a copy in {}", dest.display()),
                        Err(e) => e.message,
                    };
                }
            } else {
                ui.add_enabled(false, egui::Button::new("Save copy"));
            }
            ui.end_row();
        });
    }

    fn info(&mut self, ui: &mut egui::Ui) {
        let Some(session) = &self.session else {
            return;
        };
        let version = session.version();
        let view = match &self.view {
            Some(view) if view.version == version => view,
            _ => self.view.insert(session.view()),
        };
        ui.heading(session.dir().display().to_string());
        ui.label(format!(
            "Version {} (each commit is saved as it is made)",
            view.version
        ));
        ui.label("Commands sent to this folder from the command line run here while it is open.");
        ui.separator();
        egui::Grid::new("collections").striped(true).show(ui, |ui| {
            for (name, n) in &view.collections {
                ui.label(name);
                ui.label(n.to_string());
                ui.end_row();
            }
        });
        ui.separator();
        let selection = match self.plan.selected_dimension {
            Some(id) => describe_dimension(&self.scene, id),
            None => describe(&self.scene, self.plan.selected),
        };
        ui.label(format!("Selection: {selection}"));
        self.dimension_box(ui);
        self.review(ui);
        self.interview(ui);
    }

    fn plan(&mut self, ui: &mut egui::Ui) {
        // Esc cancels a picked pair as it cancels a first pick; until then
        // its first reference stays highlighted (T-204c).
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.dimension = None;
        }
        self.plan.held = self.dimension.as_ref().map(|(pair, _)| pair.a.clone());
        let Some(session) = &self.session else {
            return;
        };
        let version = session.version();
        if self.scene.as_ref().is_none_or(|(v, _)| *v != version) {
            self.scene = Some((version, Scene::build(&session.document())));
        }
        let scene = match &self.scene {
            Some((_, Ok(scene))) => scene,
            Some((_, Err(_))) | None => return,
        };
        match self.plan.show(ui, scene) {
            Event::None => {}
            Event::Move(action) => match session.propose(*action) {
                Ok(pending) => self.hold("Move", pending),
                Err(e) => self.message = format!("Move: {e}"),
            },
            Event::Run(run) => self.propose_run(&run),
            Event::Remove(constraint) => {
                let action = Action::RemoveConstraint(RemoveConstraint { constraint });
                match session.propose(action) {
                    Ok(pending) => self.hold("Remove dimension", pending),
                    Err(e) => self.message = format!("Remove dimension: {e}"),
                }
            }
            Event::Refused(why) => self.message = format!("Point not placed: {why}."),
            Event::Picked(dimension::Pick::First) => {}
            Event::Picked(dimension::Pick::Ready(pair)) => {
                self.dimension = Some((pair, String::new()));
            }
            Event::Picked(dimension::Pick::Refused(why)) => {
                self.message = format!("Not picked: {why}.");
            }
        }
    }
}

/// A selected dimension in words: the constraint and its value.
fn describe_dimension(scene: &Built, id: Uuid) -> String {
    let Some((_, Ok(s))) = scene else {
        return "nothing".to_string();
    };
    s.dimensions
        .iter()
        .find(|d| d.constraint == id)
        .map_or_else(
            || "nothing".to_string(),
            |d| format!("dimension {} = {} (Delete removes it)", d.name, d.text),
        )
}

/// The selection in words.
fn describe(scene: &Built, selected: Option<Entity>) -> String {
    let Some((_, Ok(s))) = scene else {
        return "nothing".to_string();
    };
    let text = match selected {
        None => None,
        Some(Entity::Wall(id)) => s
            .walls
            .iter()
            .find(|w| w.id == id)
            .map(|w| format!("wall {}", w.label)),
        Some(Entity::Opening(id)) => s.openings.iter().find(|o| o.id == id).map(|o| {
            let host = s.walls.iter().find(|w| w.id == o.host);
            format!("opening in {}", host.map_or("?", |w| w.label.as_str()))
        }),
        Some(Entity::Occurrence(id)) => s
            .occurrences
            .iter()
            .find(|o| o.id == id)
            .map(|o| format!("item {}", o.label)),
        Some(Entity::Slab(_)) => Some("slab".to_string()),
    };
    text.unwrap_or_else(|| "nothing".to_string())
}

impl App {
    /// What the app asks of the user now: open a project, review a
    /// proposal, type a dimension's value, or the plan tool's instruction.
    fn instruction(&self) -> String {
        if self.session.is_none() {
            return "Open a project folder, or create one".to_string();
        }
        if let Some((what, _)) = &self.pending {
            return format!("Review {what} in the panel at the right: Confirm or Discard");
        }
        if let Some((pair, _)) = &self.dimension {
            return format!(
                "Type the dimension from {} to {} in the panel at the right, then Propose",
                pair.a.name, pair.b.name
            );
        }
        self.plan.instruction()
    }

    /// The last message, and why the plan cannot be drawn if it cannot.
    fn last_message(&self) -> String {
        match (&self.scene, &self.session) {
            (Some((_, Err(e))), Some(_)) if self.message.is_empty() => {
                format!("The plan cannot be drawn: {e}")
            }
            (Some((_, Err(e))), Some(_)) => {
                format!("{}; the plan cannot be drawn: {e}", self.message)
            }
            _ => self.message.clone(),
        }
    }

    /// The one status line (Q-81 (a)).
    fn status(&self) -> String {
        let coordinates = if self.session.is_some() {
            self.plan.coordinates()
        } else {
            String::new()
        };
        status::line(&self.instruction(), &self.last_message(), &coordinates)
    }

    /// The whole window: project controls on top, tools and the status line
    /// at the bottom, the info and review panel at the right, the plan in
    /// the middle.
    fn layout(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Fit").clicked() {
                    self.plan.fit();
                }
                let drawing = self.plan.walls.is_some();
                if ui
                    .add_enabled(
                        self.session.is_some(),
                        egui::Button::selectable(drawing, "Draw walls"),
                    )
                    .clicked()
                {
                    self.toggle_walls();
                }
                let dimensioning = self.plan.dimensions.is_some();
                if ui
                    .add_enabled(
                        self.session.is_some(),
                        egui::Button::selectable(dimensioning, "Dimension"),
                    )
                    .clicked()
                {
                    self.toggle_dimensions();
                }
                ui.label(self.status());
            });
        });
        egui::Panel::right("info")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.info(ui));
            });
        egui::CentralPanel::default().show(ui, |ui| self.plan(ui));
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.layout(ui);
    }
}

fn main() -> std::process::ExitCode {
    let Some(fonts) = system_fonts() else {
        eprintln!("archviz-app: no usable font found in C:\\Windows\\Fonts");
        return std::process::ExitCode::FAILURE;
    };
    let result = eframe::run_native(
        "ArchViz",
        eframe::NativeOptions::default(),
        Box::new(move |cc| {
            cc.egui_ctx.set_fonts(fonts);
            Ok(Box::new(App::new()))
        }),
    );
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("archviz-app: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::tests::shop;
    use archviz_core::CreateNode;
    use eframe::egui::{Context, Pos2, RawInput, Rect, Shape, vec2};

    const WIDTH: f32 = 3000.0;
    const HEIGHT: f32 = 1200.0;

    fn context() -> Context {
        let ctx = Context::default();
        ctx.set_fonts(system_fonts().unwrap_or_else(|| panic!("no system font")));
        ctx
    }

    fn input(pointer: Pos2) -> RawInput {
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(WIDTH, HEIGHT))),
            events: vec![egui::Event::PointerMoved(pointer)],
            ..RawInput::default()
        }
    }

    fn collect(shape: &Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
            Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }

    /// Runs the whole window for a few frames with the pointer over the
    /// plan; every text drawn in the last frame, with where it was drawn.
    fn texts(ctx: &Context, app: &mut App) -> Vec<(String, Rect)> {
        let pointer = Pos2::new(1200.0, 600.0);
        let mut output = None;
        for _ in 0..4 {
            let mut o = ctx.run_ui(input(pointer), |ui| app.layout(ui));
            o.textures_delta.clear();
            output = Some(o);
        }
        let mut out = Vec::new();
        for clipped in output.map(|o| o.shapes).unwrap_or_default() {
            collect(&clipped.shape, &mut out);
        }
        out
    }

    /// Shows `message` in the mode `app` is in and checks it is drawn once,
    /// in the status line at the bottom of the window, after the mode's
    /// instruction and before the coordinates. Returns the status line.
    fn check(ctx: &Context, app: &mut App, message: &str, instruction: &str) -> String {
        app.message = message.to_string();
        let drawn = texts(ctx, app);
        let with: Vec<&(String, Rect)> =
            drawn.iter().filter(|(t, _)| t.contains(message)).collect();
        assert_eq!(
            with.len(),
            1,
            "{message} drawn {} times: {drawn:?}",
            with.len()
        );
        let Some((line, rect)) = with.first() else {
            panic!("not drawn");
        };
        assert_eq!(*line, app.status());
        assert!(
            rect.min.y > HEIGHT - 60.0,
            "not in the bottom bar: {rect:?}"
        );
        let at = |part: &str| {
            line.find(part)
                .unwrap_or_else(|| panic!("{part:?} not in {line:?}"))
        };
        assert_eq!(at(instruction), 0, "{line}");
        assert!(at(instruction) < at(message), "{line}");
        let coordinates = app.plan.coordinates();
        if app.session.is_some() {
            assert!(!coordinates.is_empty(), "no coordinates over the plan");
            assert!(at(message) < at(&coordinates), "{line}");
            assert!(line.ends_with(&coordinates), "{line}");
        } else {
            assert!(line.ends_with(message), "{line}");
        }
        line.clone()
    }

    #[test]
    fn every_message_is_in_the_status_line_after_the_instruction_and_before_the_coordinates() {
        let ctx = context();
        let mut app = App::new();

        // No project: the instruction to open one, then the message.
        check(
            &ctx,
            &mut app,
            "Message 1 (no project).",
            "Open a project folder",
        );

        let (dir, doc) = shop("status-line");
        app.opened(Session::open(&dir, Box::new(|| {})));
        assert!(app.session.is_some(), "{}", app.message);

        // Select mode: the pointer is over the plan, so coordinates show.
        let line = check(&ctx, &mut app, "Message 2 (select).", "Select:");
        assert!(line.contains("x ") && line.contains("1 px ="), "{line}");

        // Wall mode: the next point and the scale.
        app.toggle_walls();
        assert!(app.pending.is_none(), "the shop has a storey and datum");
        let line = check(&ctx, &mut app, "Message 3 (walls).", "Draw walls:");
        assert!(line.contains("next point x "), "{line}");

        // Dimension mode, first pick.
        app.toggle_dimensions();
        check(
            &ctx,
            &mut app,
            "Message 4 (first pick).",
            "Dimension: pick the first",
        );

        // Dimension mode, second pick: the first reference is named.
        let scene = Scene::build(&doc).unwrap_or_else(|e| panic!("{e}"));
        let wall = scene.walls.first().unwrap_or_else(|| panic!("walls"));
        let first = dimension::Picked {
            reference: archviz_core::Reference::WallCenter { wall: wall.id },
            axis: Some(wall.across),
            name: format!("{} center", wall.label),
        };
        let tool = app
            .plan
            .dimensions
            .as_mut()
            .unwrap_or_else(|| panic!("mode"));
        assert!(matches!(
            tool.click(Some(first.clone())),
            dimension::Pick::First
        ));
        let line = check(
            &ctx,
            &mut app,
            "Message 5 (second pick).",
            "Dimension from ",
        );
        assert!(line.contains(&first.name), "{line}");

        // A pair picked: type its value.
        app.dimension = Some((
            dimension::Pair {
                a: first.clone(),
                b: first.clone(),
                axis: wall.across,
            },
            String::new(),
        ));
        check(
            &ctx,
            &mut app,
            "Message 6 (value).",
            "Type the dimension from ",
        );
        app.dimension = None;

        // A proposal under review.
        app.propose_actions(
            "Node",
            vec![Action::CreateNode(CreateNode {
                x: Length::from_nanometers(0),
                y: Length::from_nanometers(0),
            })],
        );
        assert!(app.pending.is_some(), "{}", app.message);
        check(
            &ctx,
            &mut app,
            "Message 7 (review).",
            "Review Node in the panel",
        );
    }

    /// Runs a few frames with the pointer at `pointer` and `events` in the
    /// first; whether the first pick's highlight is drawn in the last.
    fn highlighted(ctx: &Context, app: &mut App, pointer: Pos2, events: Vec<egui::Event>) -> bool {
        let mut shapes = Vec::new();
        for n in 0..3 {
            let mut i = input(pointer);
            if n == 0 {
                i.events.extend(events.clone());
            }
            let mut o = ctx.run_ui(i, |ui| app.layout(ui));
            o.textures_delta.clear();
            shapes = o.shapes;
        }
        crate::plan_view::tests::drawn(&shapes, crate::plan_view::PICKED)
    }

    /// A press and release of the primary button on the text `label`.
    fn click_on(ctx: &Context, app: &mut App, label: &str) -> bool {
        let drawn = texts(ctx, app);
        let (_, rect) = drawn
            .iter()
            .find(|(t, _)| t == label)
            .unwrap_or_else(|| panic!("{label} not drawn"));
        let at = rect.center();
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        highlighted(ctx, app, at, vec![press(true)]);
        highlighted(ctx, app, at, vec![press(false)])
    }

    /// `highlighted` with the pointer over the plan.
    fn first_pick_drawn(ctx: &Context, app: &mut App, events: Vec<egui::Event>) -> bool {
        highlighted(ctx, app, Pos2::new(1200.0, 600.0), events)
    }

    fn escape() -> egui::Event {
        egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        }
    }

    /// Done when: the first pick's highlight clears on Propose, Cancel and
    /// Esc. It stays while the second reference is picked and while the
    /// value is typed.
    #[test]
    fn the_first_pick_highlight_clears_on_propose_cancel_and_esc() {
        let ctx = context();
        let mut app = App::new();
        let (dir, doc) = shop("first-pick");
        app.opened(Session::open(&dir, Box::new(|| {})));
        app.toggle_dimensions();
        let scene = Scene::build(&doc).unwrap_or_else(|e| panic!("{e}"));
        let wall = scene.walls.first().unwrap_or_else(|| panic!("walls"));
        let other = scene
            .walls
            .iter()
            .find(|w| w.id != wall.id && w.across == wall.across)
            .unwrap_or_else(|| panic!("a parallel wall"));
        let center = |w: &crate::scene::WallShape| dimension::Picked {
            reference: archviz_core::Reference::WallCenter { wall: w.id },
            axis: Some(w.across),
            name: format!("{} center", w.label),
        };
        // Both picks, as the plan reports them to the app.
        let pick_both = |app: &mut App| {
            let tool = app.plan.dimensions.as_mut().unwrap_or_else(|| panic!());
            assert_eq!(tool.click(Some(center(wall))), dimension::Pick::First);
            let dimension::Pick::Ready(pair) = tool.click(Some(center(other))) else {
                panic!("a pair")
            };
            app.dimension = Some((pair, String::new()));
        };
        assert!(!first_pick_drawn(&ctx, &mut app, Vec::new()), "nothing yet");

        // Esc while the second reference is picked.
        let tool = app.plan.dimensions.as_mut().unwrap_or_else(|| panic!());
        tool.click(Some(center(wall)));
        assert!(first_pick_drawn(&ctx, &mut app, Vec::new()));
        assert!(!first_pick_drawn(&ctx, &mut app, vec![escape()]));
        assert!(app.plan.first_pick().is_none());

        // Esc while the value is typed.
        pick_both(&mut app);
        assert!(first_pick_drawn(&ctx, &mut app, Vec::new()));
        assert_eq!(app.plan.first_pick(), Some(&center(wall)));
        assert!(!first_pick_drawn(&ctx, &mut app, vec![escape()]));
        assert!(app.dimension.is_none() && app.plan.first_pick().is_none());

        // Cancel.
        pick_both(&mut app);
        assert!(first_pick_drawn(&ctx, &mut app, Vec::new()));
        assert!(!click_on(&ctx, &mut app, "Cancel"));
        assert!(app.dimension.is_none() && app.plan.first_pick().is_none());

        // Propose: the proposal goes to review and the highlight clears.
        pick_both(&mut app);
        if let Some((_, text)) = &mut app.dimension {
            *text = "10'".to_string();
        }
        assert!(first_pick_drawn(&ctx, &mut app, Vec::new()));
        assert!(!click_on(&ctx, &mut app, "Propose"));
        assert!(app.pending.is_some(), "{}", app.message);
        assert!(app.dimension.is_none() && app.plan.first_pick().is_none());
    }

    /// Runs a few frames with the pointer at `pointer` and `events` in the
    /// first; the shapes of the last.
    fn shapes_after(
        ctx: &Context,
        app: &mut App,
        pointer: Pos2,
        events: Vec<egui::Event>,
    ) -> Vec<egui::epaint::ClippedShape> {
        let mut shapes = Vec::new();
        for n in 0..3 {
            let mut i = input(pointer);
            if n == 0 {
                i.events.extend(events.clone());
            }
            let mut o = ctx.run_ui(i, |ui| app.layout(ui));
            o.textures_delta.clear();
            shapes = o.shapes;
        }
        shapes
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        }
    }

    /// The texts drawn in the plan (left of the side panel, between the
    /// top bar and the status line), sorted.
    fn plan_texts(ctx: &Context, app: &mut App) -> Vec<(String, Rect)> {
        let mut out: Vec<(String, Rect)> = texts(ctx, app)
            .into_iter()
            .filter(|(_, r)| r.max.x < WIDTH - 400.0 && r.min.y > 200.0 && r.max.y < HEIGHT - 60.0)
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    fn values(texts: &[(String, Rect)]) -> Vec<&str> {
        texts
            .iter()
            .map(|(t, _)| t.as_str())
            .filter(|t| t.contains('\''))
            .collect()
    }

    /// Done when: the T-203 room shows its four dimensions with the values
    /// typed; selecting one and pressing Delete opens a review whose echo
    /// names the constraint, and Confirm removes it and leaves its wall
    /// underconstrained (orange).
    #[test]
    fn the_t203_room_shows_its_dimensions_and_delete_removes_one() {
        let ctx = context();
        let mut app = App::new();
        let (dir, doc) = crate::session::dimension_tests::t203_room("dimensions-on-plan");
        app.opened(Session::open(&dir, Box::new(|| {})));
        assert!(app.session.is_some(), "{}", app.message);

        let drawn = plan_texts(&ctx, &mut app);
        assert_eq!(
            values(&drawn),
            ["0'-0\"", "0'-0\"", "11'-3\"", "14'-6 1/2\""],
            "{drawn:?}"
        );
        assert!(
            app.plan.free.is_empty(),
            "the room starts fully constrained"
        );

        // Click the 14'-6 1/2" dimension: it is selected, nothing else is.
        let (id, kind) = doc
            .constraints
            .iter()
            .find(|(_, c)| {
                matches!(c.kind, archviz_core::ConstraintKind::Distance { value, .. }
                    if value.nanometers() == (14 * 12 + 6) * 25_400_000 + 12_700_000)
            })
            .map(|(id, c)| (*id, c.kind.clone()))
            .unwrap_or_else(|| panic!("the 14'-6 1/2\" constraint"));
        click_on(&ctx, &mut app, "14'-6 1/2\"");
        assert_eq!(app.plan.selected_dimension, Some(id));
        assert_eq!(app.plan.selected, None);
        let at = drawn
            .iter()
            .find(|(t, _)| t == "14'-6 1/2\"")
            .map(|(_, r)| r.center())
            .unwrap_or_else(|| panic!("drawn"));
        let shapes = shapes_after(&ctx, &mut app, at, Vec::new());
        assert!(crate::plan_view::tests::drawn(
            &shapes,
            crate::plan_view::SELECTED
        ));

        // Delete opens a review whose echo names the constraint.
        shapes_after(&ctx, &mut app, at, vec![key(egui::Key::Delete)]);
        let (what, review) = app
            .pending
            .as_ref()
            .unwrap_or_else(|| panic!("{}", app.message));
        assert_eq!(what, "Remove dimension");
        assert_eq!(
            review.echo,
            ["Remove the distance from W-004 right face to W-002 right face along X (14'-6 1/2\")"]
        );
        assert_eq!(
            review.echo,
            [format!(
                "Remove {} (14'-6 1/2\")",
                archviz_core::describe_constraint(&doc, &kind)
            )]
        );
        let version = app.session.as_ref().map(Session::version);
        assert!(
            app.session
                .as_ref()
                .is_some_and(|s| s.document().constraints.contains_key(&id)),
            "nothing is removed before Confirm"
        );

        // Confirm removes it; W-002 (the east wall) is underconstrained and
        // drawn orange; the other three dimensions remain.
        click_on(&ctx, &mut app, "Confirm");
        assert!(app.pending.is_none(), "{}", app.message);
        let session = app.session.as_ref().unwrap_or_else(|| panic!("open"));
        assert_eq!(Some(session.version()), version.map(|v| v + 1));
        let after = session.document();
        assert!(!after.constraints.contains_key(&id));
        assert_eq!(after.constraints.len(), doc.constraints.len() - 1);
        let east = after
            .walls
            .iter()
            .find(|(_, w)| w.label.to_string() == "W-002")
            .map(|(id, _)| *id)
            .unwrap_or_else(|| panic!("W-002"));
        let drawn = plan_texts(&ctx, &mut app);
        assert_eq!(values(&drawn), ["0'-0\"", "0'-0\"", "11'-3\""], "{drawn:?}");
        assert!(app.plan.free.contains(&east), "{:?}", app.plan.free);
        assert_eq!(app.plan.selected_dimension, None, "its dimension is gone");
        let orange = shapes_after(&ctx, &mut app, Pos2::new(1200.0, 600.0), Vec::new())
            .iter()
            .any(|c| match &c.shape {
                Shape::Path(p) => {
                    p.stroke.color
                        == egui::epaint::ColorMode::Solid(egui::Color32::from_rgb(240, 150, 0))
                }
                _ => false,
            });
        assert!(orange, "an underconstrained wall is drawn orange");
    }

    /// A proposal that conflicts with a dimension draws that dimension red
    /// until it is discarded; Delete while a text box has focus removes
    /// nothing.
    #[test]
    fn a_conflicting_dimension_is_red_and_delete_in_a_text_box_does_nothing() {
        let ctx = context();
        let mut app = App::new();
        let (dir, doc) = crate::session::dimension_tests::t203_room("dimensions-conflict");
        app.opened(Session::open(&dir, Box::new(|| {})));
        let (id, kind) = doc
            .constraints
            .iter()
            .find(|(_, c)| {
                matches!(c.kind, archviz_core::ConstraintKind::Distance { value, .. }
                    if value.nanometers() == 135 * 25_400_000)
            })
            .map(|(id, c)| (*id, c.kind.clone()))
            .unwrap_or_else(|| panic!("the 11'-3\" constraint"));
        let pointer = Pos2::new(1200.0, 600.0);
        let red = |shapes: &[egui::epaint::ClippedShape]| {
            crate::plan_view::tests::drawn(shapes, crate::plan_view::CONFLICT)
        };
        assert!(!red(&shapes_after(&ctx, &mut app, pointer, Vec::new())));
        let archviz_core::ConstraintKind::Distance { a, b, axis, .. } = kind else {
            panic!("a distance")
        };
        app.propose_actions(
            "Dimension 12'-0\"",
            vec![Action::AddConstraint(archviz_core::AddConstraint {
                kind: archviz_core::ConstraintKind::Distance {
                    a,
                    b,
                    axis,
                    value: Length::from_nanometers(144 * 25_400_000),
                    tolerance: None,
                },
                status: None,
                source: Source::UserStated,
                provenance: None,
            })],
        );
        assert!(app.pending.is_some(), "{}", app.message);
        assert!(app.plan.conflicting_dimensions.contains(&id));
        assert!(red(&shapes_after(&ctx, &mut app, pointer, Vec::new())));
        click_on(&ctx, &mut app, "Discard");
        assert!(app.plan.conflicting_dimensions.is_empty());
        assert!(!red(&shapes_after(&ctx, &mut app, pointer, Vec::new())));

        // Select the 11'-3" dimension, then focus the project folder box
        // and press Delete: no proposal.
        click_on(&ctx, &mut app, "11'-3\"");
        assert_eq!(app.plan.selected_dimension, Some(id));
        ctx.memory_mut(|m| m.request_focus(egui::Id::unique(TOP_TEXT_BOXES[0])));
        shapes_after(&ctx, &mut app, pointer, vec![key(egui::Key::Delete)]);
        assert!(
            app.pending.is_none(),
            "Delete in a text box proposes nothing"
        );
    }

    #[test]
    fn the_message_is_read_only_by_the_status_line() {
        // The app's code: everything before this test module.
        let source = include_str!("main.rs")
            .split("mod tests {")
            .next()
            .unwrap_or_default();
        let reads: Vec<&str> = source
            .lines()
            .filter(|l| l.contains("self.message") && !l.contains("self.message ="))
            .map(str::trim)
            .collect();
        // last_message reads it three times; nothing else shows it.
        assert_eq!(
            reads,
            [
                "(Some((_, Err(e))), Some(_)) if self.message.is_empty() => {",
                "format!(\"{}; the plan cannot be drawn: {e}\", self.message)",
                "_ => self.message.clone(),",
            ]
        );
    }

    #[test]
    fn no_text_box_in_the_top_bar_is_narrower_than_four_times_its_t204_width() {
        // T-204 put each box in the project grid with `text_edit_singleline`
        // (egui's default width, 280 px); measure that layout as drawn.
        let ctx = context();
        let mut text = String::from("shop");
        let mut t204 = 0.0_f32;
        for _ in 0..4 {
            let mut o = ctx.run_ui(input(Pos2::ZERO), |ui| {
                egui::Panel::top("t204").show(ui, |ui| {
                    egui::Grid::new("t204-grid").num_columns(3).show(ui, |ui| {
                        ui.label("Project folder");
                        t204 = ui.text_edit_singleline(&mut text).rect.width();
                        let _ = ui.button("Open");
                        ui.end_row();
                    });
                });
            });
            o.textures_delta.clear();
        }
        // The grid gave it only its 40 px minimum cell width; hold the new
        // boxes to four times the larger of that and the 280 px asked for.
        assert_eq!(t204, 40.0, "T-204 layout as drawn");
        let t204 = t204.max(egui::Style::default().spacing.text_edit_width);
        let mut app = App::new();
        texts(&ctx, &mut app);
        for id in TOP_TEXT_BOXES {
            let response = ctx
                .read_response(egui::Id::unique(id))
                .unwrap_or_else(|| panic!("{id} not drawn"));
            let width = response.rect.width();
            assert!(width >= 4.0 * t204, "{id}: {width} px < 4 x {t204} px");
            assert!(response.rect.max.y < 200.0, "{id} not in the top bar");
        }
        // Every text box in the top bar is one of those four.
        let source = include_str!("main.rs");
        let body = source
            .split("fn top_bar(")
            .nth(1)
            .and_then(|s| s.split("\n    fn ").next())
            .unwrap_or_else(|| panic!("top_bar not found"));
        assert_eq!(body.matches("top_text(ui, ").count(), 4);
        assert!(!body.contains("text_edit") && !body.contains("TextEdit"));
    }
}
