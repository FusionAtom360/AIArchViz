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
//!
//! T-205 (6.6, Q-83, DEC-14): a PNG or JPEG of the paper plan is imported
//! into the project's assets folder and calibrated (underlay.rs): two image
//! points and a known distance, the axis direction they run and where the
//! first lies on the plan, then optionally a check distance whose residual
//! is shown. The calibration is a CreateUnderlay or SetUnderlayCalibration
//! proposal, reviewed like any other; the image is drawn under the plan and
//! never drives geometry. A dimension can be marked transcribed from the
//! underlay: source Measured, provenance Transcribed.
//!
//! T-206 (13.1, D-14) adds the 3D viewer (viewer.rs): the "3D view" button
//! swaps the plan for a wgpu view of the export crate's exact solids, with
//! orbit and walk navigation, picking, a section plane and point-to-point
//! measurement in feet-inches between exact vertices. It provides the view
//! context (9.1): camera pose, heading, selection and hovered entity, shown
//! in the info panel as get_view_context will return it.

mod camera;
mod dimension;
mod dimension_layout;
mod drag;
mod draw;
mod gpu;
mod interview;
mod model3d;
mod plan_view;
mod review;
mod scene;
mod session;
mod snap;
mod status;
mod underlay;
mod viewer;
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

/// An uploaded underlay image and its size in pixels.
type Uploaded = (egui::TextureHandle, (u32, u32));

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
    /// The underlay image file to import, and the name to give it.
    underlay_path: String,
    underlay_name: String,
    /// The underlay being calibrated, if any (T-205).
    calibration: Option<underlay::Calibration>,
    /// Underlay images uploaded for drawing, by content hash, with their
    /// size in pixels; or why one could not be loaded.
    textures: BTreeMap<String, Result<Uploaded, String>>,
    /// A typed dimension is transcribed from the underlay (6.6).
    transcribed: bool,
    /// The 3D view is shown in place of the plan (T-206).
    view3d: bool,
    viewer: viewer::Viewer,
    /// The window's wgpu renderer; None in tests.
    render: Option<eframe::egui_wgpu::RenderState>,
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

/// A text box for a length and its parser echo (5.2).
fn length_text(ui: &mut egui::Ui, text: &mut String, unit: ProjectUnit) {
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(text).desired_width(90.0));
        if !text.trim().is_empty() {
            match dimension::parse_value(text, unit) {
                Ok(v) => ui.label(format!("reads as {}", format_length(v))),
                Err(e) => ui.colored_label(WARN, e),
            };
        }
    });
}

fn direction_label(d: archviz_core::Direction) -> &'static str {
    match d {
        archviz_core::Direction::PosX => "+X",
        archviz_core::Direction::NegX => "-X",
        archviz_core::Direction::PosY => "+Y",
        archviz_core::Direction::NegY => "-Y",
    }
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
            underlay_path: String::new(),
            underlay_name: String::new(),
            calibration: None,
            textures: BTreeMap::new(),
            transcribed: false,
            view3d: false,
            viewer: viewer::Viewer::new(),
            render: None,
        }
    }

    /// Forgets everything shown of the previous project.
    fn reset(&mut self) {
        self.view = None;
        self.scene = None;
        self.plan = PlanView::new();
        self.viewer = viewer::Viewer::new();
        self.pending = None;
        self.dimension = None;
        self.checked = None;
        self.typed.clear();
        self.calibration = None;
        self.textures.clear();
        self.transcribed = false;
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
        let transcribed = &mut self.transcribed;
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
        if doc.underlays.is_empty() {
            *transcribed = false;
        } else {
            ui.checkbox(transcribed, "Transcribed from the underlay")
                .on_hover_text("Read off the paper plan: source Measured, provenance Transcribed.");
        }
        let transcribed = *transcribed;
        let value = length_input(ui, text, doc.project.default_unit, "Propose");
        let cancel = ui.button("Cancel").clicked();
        let action = value.map(|v| (v, dimension::distance_action(&doc, pair, v, transcribed)));
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

    /// Uploads `image` for drawing, scaled down to fit the renderer's
    /// largest texture if it must be (display only; picks use its pixels).
    fn upload(ctx: &egui::Context, key: &str, image: &underlay::Image) -> egui::TextureHandle {
        let max = u32::try_from(ctx.input(|i| i.max_texture_side)).unwrap_or(u32::MAX);
        let (w, h) = image.size();
        let full = egui::ColorImage::from_rgba_unmultiplied(
            [w as usize, h as usize],
            image.rgba.as_slice(),
        );
        let color = if w <= max && h <= max {
            full
        } else {
            let raw = ::image::RgbaImage::from_raw(w, h, image.rgba.clone());
            let small = raw.map(|r| ::image::imageops::thumbnail(&r, max.min(w), max.min(h)));
            small.map_or(full, |s| {
                egui::ColorImage::from_rgba_unmultiplied(
                    [s.width() as usize, s.height() as usize],
                    s.as_raw(),
                )
            })
        };
        ctx.load_texture(key, color, egui::TextureOptions::LINEAR)
    }

    /// The texture of the image with `hash`, loaded from the project's
    /// assets folder on first use.
    fn texture(
        &mut self,
        ctx: &egui::Context,
        hash: &archviz_core::ContentHash,
    ) -> Result<(egui::TextureId, (u32, u32)), String> {
        let Some(session) = &self.session else {
            return Err("no project is open".to_string());
        };
        let entry = self
            .textures
            .entry(hash.as_str().to_string())
            .or_insert_with(|| {
                underlay::load(session.dir(), hash)
                    .map(|image| (App::upload(ctx, hash.as_str(), &image), image.size()))
            });
        entry
            .as_ref()
            .map(|(t, size)| (t.id(), *size))
            .map_err(Clone::clone)
    }

    /// Hands the plan every underlay to draw (the one being calibrated where
    /// the calibration puts it) and the calibration's points.
    fn show_underlays(&mut self, ctx: &egui::Context) {
        self.plan.underlays.clear();
        self.plan.calibration_marks.clear();
        self.plan.calibrating = self.calibration.is_some() && self.pending.is_none();
        let Some(session) = &self.session else {
            return;
        };
        let doc = session.document();
        let unit = doc.project.default_unit;
        let editing = match &self.calibration {
            Some(c) => match &c.target {
                underlay::Target::Existing(id) => Some(*id),
                underlay::Target::New { .. } => None,
            },
            None => None,
        };
        let mut drawn = Vec::new();
        for (id, u) in &doc.underlays {
            let Some(hash) = doc.assets.get(&u.image).map(|a| a.content_hash.clone()) else {
                continue;
            };
            let placement = match (&self.calibration, editing == Some(*id)) {
                (Some(c), true) => Some(c.placement(unit)),
                _ => underlay::Placement::of(u),
            };
            if let (Ok((texture, size)), Some(p)) = (self.texture(ctx, &hash), placement) {
                drawn.push(plan_view::Drawn {
                    texture,
                    corners: p.corners(size),
                });
            }
        }
        if let Some(c) = self.calibration.clone() {
            let placement = c.placement(unit);
            if let underlay::Target::New { image, .. } = &c.target
                && let Ok((texture, _)) = self.texture(ctx, image)
            {
                drawn.push(plan_view::Drawn {
                    texture,
                    corners: placement.corners(c.size),
                });
            }
            let at = |p: archviz_core::PixelPoint| placement.to_world((p.x as f64, p.y as f64));
            self.plan.calibration_marks = [c.a, c.b, c.check_a, c.check_b]
                .into_iter()
                .flatten()
                .map(at)
                .collect();
        }
        self.plan.underlays = drawn;
    }

    /// Where a new image is first drawn: the plan's visible area, else the
    /// scene, else 40 ft square at the origin.
    fn start_area(&self) -> (viewport::World, viewport::World) {
        const FORTY_FEET: i64 = 40 * 12 * 25_400_000;
        let scene = match &self.scene {
            Some((_, Ok(s))) => s.bounds(),
            _ => None,
        };
        self.plan
            .visible
            .or(scene)
            .unwrap_or(((0, 0), (FORTY_FEET, FORTY_FEET)))
    }

    /// Imports the image file typed in the panel and starts its calibration.
    fn import_underlay(&mut self, ctx: &egui::Context) {
        let Some(session) = &self.session else {
            return;
        };
        let source = std::path::PathBuf::from(self.underlay_path.trim());
        match underlay::import(session.dir(), &source) {
            Ok(imported) => {
                let name = match self.underlay_name.trim() {
                    "" => imported.stem.clone(),
                    name => name.to_string(),
                };
                let key = imported.hash.as_str().to_string();
                let size = imported.image.size();
                let texture = App::upload(ctx, &key, &imported.image);
                self.textures.insert(key, Ok((texture, size)));
                let (min, max) = self.start_area();
                self.calibration = Some(underlay::Calibration::new(
                    underlay::Target::New {
                        name: name.clone(),
                        image: imported.hash,
                    },
                    size,
                    underlay::Placement::fit(size, min, max),
                ));
                self.dimension = None;
                self.plan.walls = None;
                self.plan.dimensions = None;
                self.message = format!(
                    "Underlay {name:?}: imported, {} x {} px; calibrate it.",
                    size.0, size.1
                );
            }
            Err(e) => self.message = format!("Underlay not imported: {e}"),
        }
    }

    /// Starts recalibrating the underlay `id`, drawn where it is now.
    fn recalibrate(&mut self, ctx: &egui::Context, id: Uuid) {
        let Some(session) = &self.session else {
            return;
        };
        let doc = session.document();
        let Some(u) = doc.underlays.get(&id) else {
            return;
        };
        let Some(hash) = doc.assets.get(&u.image).map(|a| a.content_hash.clone()) else {
            return;
        };
        match self.texture(ctx, &hash) {
            Ok((_, size)) => {
                let (min, max) = self.start_area();
                let start = underlay::Placement::of(u)
                    .unwrap_or_else(|| underlay::Placement::fit(size, min, max));
                self.calibration = Some(underlay::Calibration::new(
                    underlay::Target::Existing(id),
                    size,
                    start,
                ));
                self.dimension = None;
                self.plan.walls = None;
                self.plan.dimensions = None;
            }
            Err(e) => self.message = format!("Underlay {:?}: {e}", u.name),
        }
    }

    /// The underlay panel (6.6): each underlay with its calibration and
    /// residual, the image import, and the calibration being made with its
    /// typed distances, direction and residual, then Propose.
    fn underlay_panel(&mut self, ui: &mut egui::Ui) {
        let Some(session) = &self.session else {
            return;
        };
        let doc = session.document();
        let unit = doc.project.default_unit;
        let mut recalibrate = None;
        let mut import = false;
        let mut propose = None;
        let mut cancel = false;
        ui.separator();
        egui::CollapsingHeader::new(format!("Underlays ({})", doc.underlays.len()))
            .default_open(true)
            .show(ui, |ui| {
                for (id, u) in &doc.underlays {
                    for line in underlay::summary(u) {
                        ui.label(line);
                    }
                    if ui.button("Recalibrate").clicked() {
                        recalibrate = Some(*id);
                    }
                }
                if let Some(c) = &mut self.calibration {
                    App::calibration_box(ui, c, unit, &doc, &mut propose, &mut cancel);
                    return;
                }
                ui.label("Image of the paper plan (PNG or JPEG file)");
                ui.text_edit_singleline(&mut self.underlay_path);
                ui.label("Name (default: the file name)");
                ui.text_edit_singleline(&mut self.underlay_name);
                import = ui.button("Import image").clicked();
            });
        if import {
            self.import_underlay(ui.ctx());
        }
        if let Some(id) = recalibrate {
            self.recalibrate(ui.ctx(), id);
        }
        if cancel {
            self.calibration = None;
            self.message = "Underlay: calibration cancelled.".to_string();
        }
        match propose {
            Some(Ok(action)) => {
                self.calibration = None;
                self.propose_actions("Underlay calibration", vec![action]);
            }
            Some(Err(e)) => self.message = format!("Underlay: {e}"),
            None => {}
        }
    }

    /// The calibration being made: its points, distances, direction and the
    /// check's residual as core computes it.
    fn calibration_box(
        ui: &mut egui::Ui,
        c: &mut underlay::Calibration,
        unit: ProjectUnit,
        doc: &archviz_core::Document,
        propose: &mut Option<Result<Action, String>>,
        cancel: &mut bool,
    ) {
        let what = match &c.target {
            underlay::Target::New { name, .. } => format!("Calibrate new underlay {name:?}"),
            underlay::Target::Existing(id) => format!(
                "Recalibrate underlay {:?}",
                doc.underlays.get(id).map_or("", |u| u.name.as_str())
            ),
        };
        ui.strong(what);
        let pixel = |p: Option<archviz_core::PixelPoint>| {
            p.map_or("not picked".to_string(), |p| format!("({}, {})", p.x, p.y))
        };
        ui.label(format!(
            "Known distance: pixels {} to {}",
            pixel(c.a),
            pixel(c.b)
        ));
        length_text(ui, &mut c.distance, unit);
        let nearest = match (c.a, c.b) {
            (Some(a), Some(b)) => direction_label(underlay::nearest_direction(a, b)),
            _ => "?",
        };
        egui::ComboBox::from_label("runs toward")
            .selected_text(c.direction.map_or_else(
                || format!("nearest ({nearest})"),
                |d| direction_label(d).to_string(),
            ))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut c.direction, None, format!("nearest ({nearest})"));
                for d in [
                    archviz_core::Direction::PosX,
                    archviz_core::Direction::PosY,
                    archviz_core::Direction::NegX,
                    archviz_core::Direction::NegY,
                ] {
                    ui.selectable_value(&mut c.direction, Some(d), direction_label(d));
                }
            });
        ui.label(match c.origin {
            Some((x, y)) => format!(
                "First point on the plan: ({}, {})",
                format_length(Length::from_nanometers(x)),
                format_length(Length::from_nanometers(y))
            ),
            None => "First point on the plan: click it".to_string(),
        });
        ui.label(format!(
            "Check distance (optional): pixels {} to {}",
            pixel(c.check_a),
            pixel(c.check_b)
        ));
        length_text(ui, &mut c.check_distance, unit);
        match (c.residual(unit), c.known(unit), c.check(unit)) {
            (Some(Ok(r)), Some(known), Some(check)) => {
                ui.label(underlay::residual_line(&known, &check, r));
            }
            (Some(Err(e)), ..) => {
                ui.colored_label(WARN, e);
            }
            _ => {}
        }
        let storey = draw::storey_for(doc);
        let action = match storey {
            Some(storey) => c.action(storey, unit),
            None => Err("the project has no storey yet: Draw walls proposes one".to_string()),
        };
        ui.horizontal(|ui| {
            if ui
                .add_enabled(action.is_ok(), egui::Button::new("Propose"))
                .on_disabled_hover_text(action.as_ref().err().cloned().unwrap_or_default())
                .clicked()
            {
                *propose = Some(action.clone());
            }
            if ui.button("Clear check").clicked() {
                c.clear_check();
            }
            if ui.button("Cancel").clicked() {
                *cancel = true;
            }
        });
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
        if self.view3d {
            self.view_context(ui);
        }
        self.dimension_box(ui);
        self.review(ui);
        self.underlay_panel(ui);
        self.interview(ui);
    }

    /// Swaps the plan and the 3D view; the plan's selection carries over.
    fn toggle_3d(&mut self) {
        self.view3d = !self.view3d;
        if self.view3d {
            self.viewer.selected = self.plan.selected.map(|e| match e {
                Entity::Wall(id)
                | Entity::Opening(id)
                | Entity::Occurrence(id)
                | Entity::Slab(id) => id,
            });
        }
    }

    /// The view context as get_view_context returns it (9.1).
    fn view_context(&self, ui: &mut egui::Ui) {
        let context = self.viewer.context();
        let c = &context.camera;
        ui.label(format!(
            "View: eye ({}, {}, {}), heading {}.{:03} deg, pitch {}{}.{:03} deg",
            format_length(c.x),
            format_length(c.y),
            format_length(c.z),
            c.heading.value() / 1000,
            c.heading.value() % 1000,
            if c.pitch.value() < 0 { "-" } else { "" },
            c.pitch.value().unsigned_abs() / 1000,
            c.pitch.value().unsigned_abs() % 1000,
        ));
        egui::CollapsingHeader::new("View context").show(ui, |ui| {
            let json = archviz_core::to_canonical_json(&context).unwrap_or_else(|e| e.to_string());
            ui.monospace(json);
        });
    }

    fn view_3d(&mut self, ui: &mut egui::Ui) {
        let Some(session) = &self.session else {
            return;
        };
        let (version, unit) = (session.version(), session.document().project.default_unit);
        let event = self
            .viewer
            .show(ui, self.render.as_ref(), version, unit, || {
                session.document()
            });
        if let viewer::Event::Select(picked) = event {
            self.plan.selected_dimension = None;
            self.plan.selected = picked.and_then(|(id, kind)| match kind {
                "wall" => Some(Entity::Wall(id)),
                "door" | "window" => Some(Entity::Opening(id)),
                "slab" => Some(Entity::Slab(id)),
                "occurrence" => Some(Entity::Occurrence(id)),
                _ => None,
            });
        }
    }

    fn plan(&mut self, ui: &mut egui::Ui) {
        // Esc cancels a picked pair as it cancels a first pick; until then
        // its first reference stays highlighted (T-204c).
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.dimension = None;
            if self.calibration.take().is_some() {
                self.message = "Underlay: calibration cancelled.".to_string();
            }
        }
        self.plan.held = self.dimension.as_ref().map(|(pair, _)| pair.a.clone());
        self.show_underlays(ui.ctx());
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
            Event::Calibrate { at, snapped } => {
                let unit = session.document().project.default_unit;
                if let Some(c) = &mut self.calibration
                    && let Err(why) = c.click(unit, at, snapped)
                {
                    self.message = format!("Underlay: {why}.");
                }
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
        if let Some(c) = &self.calibration {
            return c.instruction();
        }
        if self.view3d {
            return self.viewer.instruction();
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
        let coordinates = if self.session.is_some() && self.view3d {
            self.viewer.readout()
        } else if self.session.is_some() {
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
                if ui
                    .add(egui::Button::selectable(self.view3d, "3D view"))
                    .clicked()
                {
                    self.toggle_3d();
                }
                if ui.button("Fit").clicked() {
                    self.plan.fit();
                    self.viewer.fit();
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
        egui::CentralPanel::default().show(ui, |ui| {
            if self.view3d {
                self.view_3d(ui);
            } else {
                self.plan(ui);
            }
        });
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
            let mut app = App::new();
            app.render = cc.wgpu_render_state.clone();
            Ok(Box::new(app))
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

    /// A press and release of the primary button at `at` in the plan.
    fn click_at(ctx: &Context, app: &mut App, at: Pos2) {
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        shapes_after(ctx, app, at, vec![press(true)]);
        shapes_after(ctx, app, at, vec![press(false)]);
    }

    /// Clicks image pixel `p` where the plan draws it now.
    fn click_pixel(ctx: &Context, app: &mut App, p: (i64, i64)) {
        let unit = ProjectUnit::Inches;
        let c = app
            .calibration
            .as_ref()
            .unwrap_or_else(|| panic!("calibrating"));
        let world = c.placement(unit).to_world((p.0 as f64, p.1 as f64));
        let at = app
            .plan
            .screen_of(world)
            .unwrap_or_else(|| panic!("no view"));
        click_at(ctx, app, at);
    }

    /// Done when: the residual is displayed, and an underlay never changes
    /// geometry. A PNG is imported into the shop, calibrated by clicks on
    /// the plan (20 ft over 480 px along +X, then a 12'-0 1/4" check over
    /// 288 px), proposed, reviewed and confirmed; the panel shows the
    /// residual, the image is drawn, and the plan, the solve and every
    /// collection but underlays and assets are exactly as before.
    #[test]
    fn an_underlay_is_calibrated_its_residual_shown_and_no_geometry_changes() {
        let ctx = context();
        let mut app = App::new();
        let (dir, doc) = shop("underlay");
        app.opened(Session::open(&dir, Box::new(|| {})));
        let before_solve = app.session.as_ref().map(Session::solve);
        let source = dir.with_file_name("Paper plan.png");
        std::fs::write(&source, underlay::tests::png(600, 400)).unwrap_or_else(|e| panic!("{e}"));
        app.underlay_path = source.display().to_string();
        texts(&ctx, &mut app);
        click_on(&ctx, &mut app, "Import image");
        assert!(app.calibration.is_some(), "{}", app.status());
        assert!(app.status().starts_with("Underlay: click the first end"));

        // A click off the image (fitted to the plan's height, anchored at
        // its top left) is refused with the reason.
        let corner = app
            .plan
            .visible
            .map(|(min, max)| (max.0, min.1))
            .unwrap_or_default();
        let off = app.plan.screen_of(corner).unwrap_or_default() + vec2(-5.0, -5.0);
        click_at(&ctx, &mut app, off);
        assert!(
            app.status().contains("Underlay: click on the image."),
            "{}",
            app.status()
        );
        assert_eq!(app.calibration.as_ref().and_then(|c| c.a), None);

        click_pixel(&ctx, &mut app, (20, 10));
        click_pixel(&ctx, &mut app, (500, 10));
        if let Some(c) = &mut app.calibration {
            c.distance = "20'".to_string();
            c.check_distance = "12' 1/4\"".to_string();
        }
        assert!(
            app.status()
                .starts_with("Underlay: click where the first point lies")
        );
        click_at(&ctx, &mut app, Pos2::new(1200.0, 600.0));
        click_pixel(&ctx, &mut app, (20, 10));
        click_pixel(&ctx, &mut app, (20, 298));
        let c = app
            .calibration
            .clone()
            .unwrap_or_else(|| panic!("calibrating"));
        assert_eq!(c.step(), underlay::Step::Ready);
        assert_eq!(c.a, Some(archviz_core::PixelPoint { x: 20, y: 10 }));
        assert_eq!(c.b, Some(archviz_core::PixelPoint { x: 500, y: 10 }));
        assert_eq!(c.check_b, Some(archviz_core::PixelPoint { x: 20, y: 298 }));
        let origin = c.origin.unwrap_or_else(|| panic!("origin"));
        assert_eq!(origin.0 % archviz_core::GRID_NM, 0, "the origin is snapped");
        assert_eq!(origin.1 % archviz_core::GRID_NM, 0, "the origin is snapped");

        // Before proposing, the panel shows the residual as core computes it.
        let line = "Residual 0'-0 1/4\" (0.2%): check 12'-0 1/4\" reads 12'-0\"";
        let shown = |app: &mut App| texts(&ctx, app).iter().filter(|(t, _)| t == line).count();
        assert_eq!(shown(&mut app), 1);

        click_on(&ctx, &mut app, "Propose");
        let (what, review) = app
            .pending
            .as_ref()
            .unwrap_or_else(|| panic!("{}", app.status()));
        assert_eq!(what, "Underlay calibration");
        let echo = review.echo.join("\n");
        assert!(
            echo.starts_with("Create underlay \"Paper plan\" on storey"),
            "{echo}"
        );
        assert!(
            echo.contains("20'-0\" between pixels (20, 10) and (500, 10), toward +X"),
            "{echo}"
        );
        assert!(echo.ends_with("residual 0'-0 1/4\""), "{echo}");
        assert!(review.blocked.is_empty(), "{:?}", review.blocked);
        click_on(&ctx, &mut app, "Confirm");
        assert!(app.pending.is_none(), "{}", app.status());

        // The residual is displayed for the committed underlay.
        assert_eq!(shown(&mut app), 1);
        let shapes = shapes_after(&ctx, &mut app, Pos2::new(1200.0, 600.0), Vec::new());
        let image_drawn = shapes.iter().any(|c| match &c.shape {
            Shape::Mesh(m) => m.texture_id != egui::TextureId::default(),
            _ => false,
        });
        assert!(image_drawn, "the underlay image is drawn");

        // No geometry changed.
        let session = app.session.as_ref().unwrap_or_else(|| panic!("open"));
        let after = session.document();
        assert_eq!(after.underlays.len(), 1);
        let mut without = after.clone();
        without.underlays.clear();
        without.assets.retain(|id, _| doc.assets.contains_key(id));
        assert_eq!(without, doc);
        assert_eq!(Scene::build(&after), Scene::build(&doc));
        assert_eq!(Some(session.solve()), before_solve);
        let u = after.underlays.values().next().unwrap_or_else(|| panic!());
        assert_eq!(
            u.check.as_ref().map(|k| k.residual.nanometers()),
            Some(6_350_000)
        );
        let image = after
            .assets
            .get(&u.image)
            .unwrap_or_else(|| panic!("asset"));
        let file = underlay::asset_file(&dir, &image.content_hash, underlay::Format::Png);
        assert!(file.is_file(), "{}", file.display());

        // A transcribed dimension is proposed as Measured, Transcribed.
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
        let pair = dimension::Pair {
            a: center(wall),
            b: center(other),
            axis: wall.across,
        };
        app.dimension = Some((pair, "10'".to_string()));
        texts(&ctx, &mut app);
        click_on(&ctx, &mut app, "Transcribed from the underlay");
        assert!(app.transcribed);
        click_on(&ctx, &mut app, "Propose");
        let (_, review) = app
            .pending
            .as_ref()
            .unwrap_or_else(|| panic!("{}", app.status()));
        let echo = review.echo.join("\n");
        assert!(echo.contains("transcribed from the underlay"), "{echo}");
        assert!(
            echo.contains("tolerance"),
            "Measured takes a tolerance: {echo}"
        );
        let _ = std::fs::remove_file(&source);
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

    /// T-206 done when: measuring a known wall length in the 3D view reads
    /// exactly. Each end of a wall's longest footprint edge (a length read
    /// from core) is framed and clicked a few pixels inside the wall's top
    /// face, at fractional pointer positions; the status line shows the
    /// length with no ~.
    #[test]
    fn measuring_a_known_wall_length_in_the_3d_view_reads_exactly() {
        use crate::camera::P3;
        let ctx = context();
        let mut app = App::new();
        let (dir, doc) = shop("viewer-measure");
        app.opened(Session::open(&dir, Box::new(|| {})));
        assert!(app.session.is_some(), "{}", app.message);
        app.toggle_3d();
        texts(&ctx, &mut app);
        assert!(app.viewer.rect.is_some(), "the 3D view is drawn");
        app.viewer.measuring = true;

        let model = crate::model3d::Model::build(&doc).unwrap_or_else(|e| panic!("{e}"));
        let geometry = archviz_core::wall_geometry(&doc).unwrap_or_else(|e| panic!("{e}"));
        let mut measured = Vec::new();
        for (id, wall) in &doc.walls {
            let Some(g) = geometry.get(id) else { continue };
            let fp: Vec<(i128, i128)> = g
                .footprint
                .iter()
                .map(|p| (i128::from(p.x.nanometers()), i128::from(p.y.nanometers())))
                .collect();
            let n = fp.len();
            let Some((p, q)) = (0..n)
                .map(|i| (fp[i], fp[(i + 1) % n]))
                .max_by_key(|(p, q)| ((q.0 - p.0).abs() + (q.1 - p.1).abs(), *p))
            else {
                continue;
            };
            let known = (q.0 - p.0).abs() + (q.1 - p.1).abs();
            let Some(item) = model.items.iter().find(|i| i.id == *id) else {
                panic!("{} has no solid", wall.label);
            };
            let verts: Vec<P3> = item.triangles.iter().flatten().copied().collect();
            let top = |xy: (i128, i128)| {
                verts
                    .iter()
                    .filter(|v| (v[0], v[1]) == xy)
                    .map(|v| v[2])
                    .max()
                    .unwrap_or(0)
            };
            let (a, b) = ([p.0, p.1, top(p)], [q.0, q.1, top(q)]);
            let tops: Vec<&P3> = verts.iter().filter(|v| v[2] == a[2]).collect();
            let middle =
                [0, 1, 2].map(|k| tops.iter().map(|v| v[k]).sum::<i128>() / tops.len() as i128);
            app.viewer.measure = None;
            for (end, frac) in [(a, (0.37, -0.29)), (b, (-0.41, 0.13))] {
                app.viewer.camera.look_at(end, 8.0 * 304_800_000.0);
                let rect = app.viewer.rect.unwrap_or_else(|| panic!("no 3D view"));
                let size = [f64::from(rect.width()), f64::from(rect.height())];
                let cam = &app.viewer.camera;
                let span: Vec<f64> = (0..3).map(|k| (middle[k] - end[k]) as f64).collect();
                let k = 6.0 * 25_400_000.0 / span.iter().map(|v| v * v).sum::<f64>().sqrt();
                let toward = [0, 1, 2].map(|i| end[i] + (span[i] * k).round() as i128);
                let (Some(e), Some(t)) = (cam.project(end, size), cam.project(toward, size)) else {
                    panic!("behind the eye");
                };
                let s = 3.0 / (t[0] - e[0]).hypot(t[1] - e[1]).max(3.0);
                let at = Pos2::new(
                    rect.min.x + (e[0] + (t[0] - e[0]) * s + frac.0) as f32,
                    rect.min.y + (e[1] + (t[1] - e[1]) * s + frac.1) as f32,
                );
                let press = |pressed| egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                };
                shapes_after(&ctx, &mut app, at, vec![press(true)]);
                shapes_after(&ctx, &mut app, at, vec![press(false)]);
            }
            let Some(m) = app.viewer.measure else {
                continue;
            };
            if (m.a, m.b) != (a, b) {
                // The pointer landed on a neighbour at a corner.
                continue;
            }
            let length = archviz_core::Length::from_nanometers(known as i64);
            let want = format!("Measured {} (", format_length(length));
            assert!(!want.contains('~'));
            let status = app.status();
            assert!(status.contains(&want), "{}: {status}", wall.label);
            measured.push(format!("{} {}", wall.label, format_length(length)));
        }
        eprintln!("measured exactly in the 3D view: {measured:?}");
        assert!(measured.len() >= 4, "{measured:?}");
    }

    /// T-206 done when: the view context matches the camera. The info panel
    /// shows the context get_view_context returns, and its selection is
    /// what a click in the 3D view selected.
    #[test]
    fn the_view_context_follows_the_3d_camera_and_selection() {
        let ctx = context();
        let mut app = App::new();
        let (dir, doc) = shop("viewer-context");
        app.opened(Session::open(&dir, Box::new(|| {})));
        app.toggle_3d();
        texts(&ctx, &mut app);
        let rect = app.viewer.rect.unwrap_or_else(|| panic!("no 3D view"));
        let at = rect.center();
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        shapes_after(&ctx, &mut app, at, vec![press(true)]);
        shapes_after(&ctx, &mut app, at, vec![press(false)]);
        let context = app.viewer.context();
        let selected = app
            .viewer
            .selected
            .unwrap_or_else(|| panic!("nothing picked"));
        assert_eq!(context.selection, vec![selected]);
        assert_eq!(context.hovered, Some(selected));
        assert_eq!(context.camera, app.viewer.camera.pose());
        assert!(
            doc.walls.contains_key(&selected)
                || doc.slabs.contains_key(&selected)
                || doc.openings.contains_key(&selected)
                || doc.occurrences.contains_key(&selected)
                || doc.ceilings.contains_key(&selected)
                || doc.roofs.contains_key(&selected)
        );
        // Dragging turns the camera; the context follows.
        let before = app.viewer.context().camera;
        let drag = vec![
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
            egui::Event::PointerMoved(at + vec2(120.0, 0.0)),
        ];
        shapes_after(&ctx, &mut app, at, drag);
        let after = app.viewer.context().camera;
        assert_ne!(before.heading, after.heading);
        assert_eq!(after, app.viewer.camera.pose());
        let json = archviz_core::to_canonical_json(&app.viewer.context())
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(json.starts_with("{\"camera\":{\"heading\":"), "{json}");
    }
}
