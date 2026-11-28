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

mod dimension;
mod drag;
mod draw;
mod interview;
mod plan_view;
mod review;
mod scene;
mod session;
mod snap;
mod viewport;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use archviz_core::{Action, Axis, Length, ProjectUnit, Source, Uuid, format_length};

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
            message: "Open a project folder, or create one.".to_string(),
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
        self.pending = Some((what.to_string(), review));
    }

    /// Drops the proposal under review and its highlights.
    fn release(&mut self) {
        self.pending = None;
        self.plan.conflicting.clear();
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
            ui.text_edit_singleline(&mut self.folder);
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
            ui.text_edit_singleline(&mut self.name);
            ui.label("");
            ui.end_row();
            ui.label("Building name");
            ui.text_edit_singleline(&mut self.building);
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
            ui.text_edit_singleline(&mut self.copy_to);
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
        ui.label(&self.message);
    }

    fn info(&mut self, ui: &mut egui::Ui) {
        let Some(session) = &self.session else {
            ui.label("No project is open.");
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
        ui.label(format!(
            "Selection: {}",
            describe(&self.scene, self.plan.selected)
        ));
        self.dimension_box(ui);
        self.review(ui);
        self.interview(ui);
    }

    fn plan(&mut self, ui: &mut egui::Ui) {
        let Some(session) = &self.session else {
            ui.label("No project is open.");
            return;
        };
        let version = session.version();
        if self.scene.as_ref().is_none_or(|(v, _)| *v != version) {
            self.scene = Some((version, Scene::build(&session.document())));
        }
        let scene = match &self.scene {
            Some((_, Ok(scene))) => scene,
            Some((_, Err(e))) => {
                ui.label(format!("The plan cannot be drawn: {e}"));
                return;
            }
            None => return,
        };
        match self.plan.show(ui, scene) {
            Event::None => {}
            Event::Move(action) => match session.propose(*action) {
                Ok(pending) => self.hold("Move", pending),
                Err(e) => self.message = format!("Move: {e}"),
            },
            Event::Run(run) => self.propose_run(&run),
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

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
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
                ui.label(self.plan.status());
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
