//! The plan view widget (T-202, spec 3.1, 5.3): draws the scene through the
//! 64-bit viewport, pans (middle or right drag), zooms about the pointer
//! (scroll), selects (click) and drags entities into move commands (left
//! drag), and in wall mode draws runs of walls from snapped clicks
//! (T-202b, draw.rs), and in dimension mode picks references for a
//! dimension (T-203, dimension.rs). Every Distance constraint is drawn as a
//! dimension where dimension_layout.rs places it; a click selects one and
//! Delete proposes its removal (T-204d). Floats appear only between the viewport and the painter; every
//! value that leaves this widget as a command argument is snapped first.

use archviz_core::{Axis, ConstraintStatus, DatumGeometry, Function, Length, format_length};
use eframe::egui::{
    self, Align2, Color32, FontId, PointerButton, Pos2, Sense, Shape, Stroke, StrokeKind,
};

use std::collections::BTreeSet;

use archviz_core::Uuid;

use crate::dimension::{DimensionTool, Mark, Pick, Picked, mark, pick};
use crate::dimension_layout::Side;
use crate::drag::{Drag, Preview};
use crate::draw::{Click, Run, WallTool};
use crate::scene::{Entity, Scene};
use crate::snap::{SnapKind, to_grid, visible_step};
use crate::viewport::{Screen, Viewport, World, clip_polygon, clip_segment};

/// Pixels within which the pointer picks an entity or snaps to a reference.
const PICK_PX: f64 = 6.0;
const SNAP_PX: f64 = 8.0;
/// Pixels outside the canvas that clipped shapes extend to, so clip edges
/// never show.
const PAD: f64 = 64.0;
/// Dimension mode: the reference a click would pick, and the first pick.
pub const HOVER: Color32 = Color32::from_rgb(0, 140, 255);
pub const PICKED: Color32 = Color32::from_rgb(0, 190, 160);
/// A selected dimension, and one named by a proposal's conflicts.
pub const SELECTED: Color32 = Color32::from_rgb(0, 140, 255);
pub const CONFLICT: Color32 = Color32::from_rgb(230, 40, 40);
/// Pixels from the plan's bounding box to the first row of dimensions, and
/// between rows.
const DIMENSION_GAP: f64 = 28.0;
const DIMENSION_ROW: f64 = 26.0;

/// What the widget reports to the app after a frame.
pub enum Event {
    None,
    /// A drag ended with a move; the app proposes it.
    Move(Box<archviz_core::Action>),
    /// A run of walls was closed or finished; the app proposes it.
    Run(Run),
    /// A click in wall mode was refused, and why.
    Refused(&'static str),
    /// A click in dimension mode picked a reference, or was refused.
    Picked(Pick),
    /// Delete was pressed with this dimension selected; the app proposes
    /// RemoveConstraint (T-204d).
    Remove(Uuid),
}

pub struct PlanView {
    viewport: Option<Viewport>,
    pub selected: Option<Entity>,
    drag: Option<Drag>,
    preview: Option<Preview>,
    /// The snapped pointer position, for the status line.
    pointer: Option<World>,
    /// Wall mode: the run being drawn. None in select mode.
    pub walls: Option<WallTool>,
    /// In wall mode, where a click would place the next point.
    next: Option<World>,
    /// Dimension mode: the references being picked. None otherwise.
    pub dimensions: Option<DimensionTool>,
    /// Dimension mode: the reference under the pointer, the one a click
    /// there picks (T-204c).
    hover: Option<Picked>,
    /// The first reference of a pair whose value is being typed; the app
    /// holds the pair and sets this each frame.
    pub held: Option<Picked>,
    /// Walls the solver reports underconstrained (7.3).
    pub free: BTreeSet<Uuid>,
    /// Walls named by a pending proposal's conflicts (7.5).
    pub conflicting: BTreeSet<Uuid>,
    /// Constraints named by a pending proposal's conflicts; their
    /// dimensions are drawn red.
    pub conflicting_dimensions: BTreeSet<Uuid>,
    /// The selected dimension (its constraint), in select mode.
    pub selected_dimension: Option<Uuid>,
    /// Where each dimension was drawn last frame, for clicks.
    dimension_hits: Vec<(Uuid, egui::Rect)>,
}

fn pos(s: Screen) -> Pos2 {
    Pos2::new(s.x as f32, s.y as f32)
}

fn screen(p: Pos2) -> Screen {
    Screen {
        x: f64::from(p.x),
        y: f64::from(p.y),
    }
}

fn length(nm: i64) -> String {
    format_length(Length::from_nanometers(nm))
}

impl PlanView {
    pub fn new() -> PlanView {
        PlanView {
            viewport: None,
            selected: None,
            drag: None,
            preview: None,
            pointer: None,
            walls: None,
            next: None,
            dimensions: None,
            hover: None,
            held: None,
            free: BTreeSet::new(),
            conflicting: BTreeSet::new(),
            conflicting_dimensions: BTreeSet::new(),
            selected_dimension: None,
            dimension_hits: Vec::new(),
        }
    }

    /// Fits the whole scene on the next frame.
    pub fn fit(&mut self) {
        self.viewport = None;
    }

    /// What the current mode asks of the user; first in the status line.
    pub fn instruction(&self) -> String {
        if self.walls.is_some() {
            return "Draw walls: click to place each point; click the first point to close, \
                    Enter or double-click to finish, Esc to cancel"
                .to_string();
        }
        if let Some(tool) = &self.dimensions {
            let mut s = match tool.first() {
                None => "Dimension: pick the first reference (wall face, wall center or datum)"
                    .to_string(),
                Some(p) => format!(
                    "Dimension from {}: pick the second reference, Esc to cancel",
                    p.name
                ),
            };
            if let Some(h) = &self.hover {
                s.push_str(&format!("; under the pointer: {}", h.name));
            }
            return s;
        }
        let mut s = "Select: click to select, drag a wall, opening or item to move it; \
                     scroll to zoom, right or middle drag to pan"
            .to_string();
        if self.selected_dimension.is_some() {
            s.push_str("; Delete removes the selected dimension");
        }
        s
    }

    /// The first pick, highlighted until the dimension is proposed or
    /// cancelled: the tool's while the second is picked, else the held one.
    pub fn first_pick(&self) -> Option<&Picked> {
        self.dimensions
            .as_ref()
            .and_then(DimensionTool::first)
            .or(self.held.as_ref())
    }

    /// The pointer readout; last in the status line.
    pub fn coordinates(&self) -> String {
        let mut s = match self.pointer {
            Some((x, y)) => format!("x {}  y {}", length(to_grid(x)), length(to_grid(y))),
            None => String::new(),
        };
        if let Some(p) = &self.preview {
            let axis = match p.axis {
                Axis::X => "X",
                Axis::Y => "Y",
            };
            let to = match &p.snap.kind {
                SnapKind::Grid => "grid".to_string(),
                SnapKind::Reference(name) => name.clone(),
            };
            s.push_str(&format!(
                "   move {} along {axis} to {} ({to})",
                length(p.delta),
                length(p.snap.value)
            ));
        }
        if let (Some(tool), Some(next)) = (&self.walls, self.next) {
            s = format!("next point x {}  y {}", length(next.0), length(next.1));
            if let Some(last) = tool.points().last() {
                let d = (i128::from(next.0) - i128::from(last.0)).abs()
                    + (i128::from(next.1) - i128::from(last.1)).abs();
                s.push_str(&format!(
                    "   wall {}",
                    length(i64::try_from(d).unwrap_or(i64::MAX))
                ));
            }
        }
        if let Some(v) = &self.viewport {
            s.push_str(&format!(
                "   1 px = {}",
                length(v.nm(1.0)).trim_start_matches('~')
            ));
        }
        s.trim_start().to_string()
    }

    pub fn show(&mut self, ui: &mut egui::Ui, scene: &Scene) -> Event {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        let origin = screen(rect.center());
        // Fit leaves room for the rows of dimensions outside the plan.
        let rows = scene.dimensions.iter().map(|d| d.row + 1).max();
        let margin = 40.0
            + rows.map_or(0.0, |n| {
                DIMENSION_GAP + DIMENSION_ROW * f64::from(u32::try_from(n).unwrap_or(u32::MAX))
            });
        let viewport = self.viewport.get_or_insert_with(|| match scene.bounds() {
            Some((min, max)) => Viewport::fit(
                min,
                max,
                (f64::from(rect.width()), f64::from(rect.height())),
                margin,
                origin,
            ),
            None => Viewport::new((0, 0), 1.0 / 25_400_000.0, origin),
        });
        viewport.set_origin(origin);

        // Navigation.
        if response.dragged_by(PointerButton::Middle)
            || response.dragged_by(PointerButton::Secondary)
        {
            let d = response.drag_delta();
            viewport.pan(f64::from(d.x), f64::from(d.y));
        }
        if let Some(hover) = response.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            let pinch = ui.input(|i| i.zoom_delta());
            let factor = f64::from(pinch) * (f64::from(scroll) / 200.0).exp();
            if factor != 1.0 {
                viewport.zoom_about(screen(hover), factor);
            }
        }
        let viewport = viewport.clone();
        self.pointer = response.hover_pos().map(|p| viewport.to_world(screen(p)));

        // A dimension whose constraint is gone is no longer selected.
        if let Some(id) = self.selected_dimension
            && !scene.dimensions.iter().any(|d| d.constraint == id)
        {
            self.selected_dimension = None;
        }
        let mut event = Event::None;
        let at = response
            .interact_pointer_pos()
            .map(|p| viewport.to_world(screen(p)));
        if self.dimensions.is_none() {
            self.hover = None;
        }
        if let Some(tool) = &mut self.walls {
            // Wall mode: clicks place points; nothing is selected or dragged.
            self.drag = None;
            self.preview = None;
            let tolerance = viewport.nm(SNAP_PX);
            let step = visible_step(viewport.scale());
            if response.clicked_by(PointerButton::Primary)
                && let Some(at) = at
            {
                event = match tool.click(scene, at, tolerance, step) {
                    Click::Added => Event::None,
                    Click::Closed(run) => Event::Run(run),
                    Click::Ignored(why) => Event::Refused(why),
                };
            }
            let (enter, escape) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::Enter),
                    i.key_pressed(egui::Key::Escape),
                )
            });
            if (response.double_clicked_by(PointerButton::Primary) || enter)
                && let Some(run) = tool.finish()
            {
                event = Event::Run(run);
            }
            if escape {
                tool.cancel();
            }
            self.next = self
                .pointer
                .map(|p| tool.next_point(scene, p, tolerance, step));
        } else if let Some(tool) = &mut self.dimensions {
            // Dimension mode: clicks pick references; the one a click at
            // the pointer would pick is highlighted, found the same way.
            self.next = None;
            self.drag = None;
            self.preview = None;
            let under = |p: World| pick(scene, p, viewport.nm(SNAP_PX));
            self.hover = self.pointer.and_then(under);
            if response.clicked_by(PointerButton::Primary)
                && let Some(at) = at
            {
                event = Event::Picked(tool.click(under(at)));
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                tool.cancel();
            }
        } else {
            self.next = None;
            event = self.select_and_drag(ui, &response, scene, &viewport, at);
        }

        let painter = painter.with_clip_rect(rect);
        let bg = ui.visuals().extreme_bg_color;
        painter.rect_filled(rect, 0.0, bg);
        self.dimension_hits =
            self.paint(&painter, &viewport, scene, rect, ui.visuals().dark_mode, bg);
        event
    }

    /// Select mode: click selects, left drag moves.
    fn select_and_drag(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        scene: &Scene,
        viewport: &Viewport,
        at: Option<World>,
    ) -> Event {
        let mut event = Event::None;
        if response.clicked_by(PointerButton::Primary) {
            // A dimension, drawn over the plan, is picked first.
            let on = response.interact_pointer_pos();
            self.selected_dimension = on.and_then(|p| {
                self.dimension_hits
                    .iter()
                    .find(|(_, r)| r.contains(p))
                    .map(|(id, _)| *id)
            });
            self.selected = match self.selected_dimension {
                Some(_) => None,
                None => at.and_then(|w| scene.pick(w, viewport.nm(PICK_PX))),
            };
        }
        if let Some(id) = self.selected_dimension
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input(|i| i.key_pressed(egui::Key::Delete))
        {
            event = Event::Remove(id);
        }
        if response.drag_started_by(PointerButton::Primary) {
            // The press position, not where the drag threshold was crossed.
            let press = ui
                .input(|i| i.pointer.press_origin())
                .map(|p| viewport.to_world(screen(p)));
            let picked = press.and_then(|w| scene.pick(w, viewport.nm(PICK_PX)).map(|e| (e, w)));
            self.selected = picked.map(|(e, _)| e);
            self.selected_dimension = None;
            self.drag = picked.and_then(|(e, w)| Drag::begin(scene, e, w));
        }
        if let (Some(drag), Some(at)) = (&self.drag, at)
            && response.dragged_by(PointerButton::Primary)
        {
            self.preview = drag.update(
                scene,
                at,
                viewport.nm(SNAP_PX),
                visible_step(viewport.scale()),
            );
        }
        if response.drag_stopped_by(PointerButton::Primary) {
            if let Some(action) = self.preview.take().and_then(|p| p.action) {
                event = Event::Move(Box::new(action));
            }
            self.drag = None;
        }
        event
    }

    fn paint(
        &self,
        painter: &egui::Painter,
        v: &Viewport,
        scene: &Scene,
        rect: egui::Rect,
        dark: bool,
        bg: Color32,
    ) -> Vec<(Uuid, egui::Rect)> {
        let lo = Screen {
            x: f64::from(rect.min.x) - PAD,
            y: f64::from(rect.min.y) - PAD,
        };
        let hi = Screen {
            x: f64::from(rect.max.x) + PAD,
            y: f64::from(rect.max.y) + PAD,
        };
        let ink = if dark {
            Color32::from_gray(220)
        } else {
            Color32::from_gray(30)
        };
        let faint = if dark {
            Color32::from_gray(45)
        } else {
            Color32::from_gray(225)
        };
        let accent = Color32::from_rgb(0, 140, 255);
        let font = FontId::proportional(12.0);

        // Grid: the step a click snaps to (Q-79).
        {
            let step = visible_step(v.scale());
            let a = v.to_world(screen(rect.left_top()));
            let b = v.to_world(screen(rect.right_bottom()));
            let line = |p: World, q: World| {
                if let Some((p, q)) = clip_segment(v.to_screen(p), v.to_screen(q), lo, hi) {
                    painter.line_segment([pos(p), pos(q)], Stroke::new(1.0, faint));
                }
            };
            let first = |x: i64| x.div_euclid(step).saturating_mul(step);
            let mut x = first(a.0);
            while x <= b.0 {
                line((x, b.1), (x, a.1));
                x = x.saturating_add(step);
                if x == i64::MAX {
                    break;
                }
            }
            let mut y = first(b.1);
            while y <= a.1 {
                line((a.0, y), (b.0, y));
                y = y.saturating_add(step);
                if y == i64::MAX {
                    break;
                }
            }
        }

        let outline = |points: &[World], stroke: Stroke, fill: Option<Color32>| {
            let s: Vec<Screen> = points.iter().map(|p| v.to_screen(*p)).collect();
            let clipped: Vec<Pos2> = clip_polygon(&s, lo, hi).into_iter().map(pos).collect();
            if clipped.len() >= 3 {
                if let Some(fill) = fill {
                    painter.add(Shape::convex_polygon(clipped.clone(), fill, Stroke::NONE));
                }
                painter.add(Shape::closed_line(clipped, stroke));
            }
        };
        let rect_of = |r: &(World, World)| vec![r.0, (r.1.0, r.0.1), r.1, (r.0.0, r.1.1)];
        let selected = |e: Entity| self.selected == Some(e);
        let stroke = |e: Entity, color: Color32| {
            if selected(e) {
                Stroke::new(2.5, accent)
            } else {
                Stroke::new(1.0, color)
            }
        };

        for s in &scene.slabs {
            outline(
                &s.outline,
                stroke(Entity::Slab(s.id), Color32::from_rgb(150, 150, 110)),
                None,
            );
        }
        for w in &scene.walls {
            let color = match w.function {
                Function::Existing => Color32::from_rgb(120, 120, 120),
                Function::Demolished => Color32::from_rgb(200, 80, 80),
                _ => ink,
            };
            // A conflict outranks underconstrained, which outranks selection.
            let s = if self.conflicting.contains(&w.id) {
                Stroke::new(3.0, Color32::from_rgb(230, 40, 40))
            } else if self.free.contains(&w.id) {
                Stroke::new(2.0, Color32::from_rgb(240, 150, 0))
            } else {
                stroke(Entity::Wall(w.id), color)
            };
            outline(&w.footprint, s, None);
        }
        for o in &scene.openings {
            outline(
                &rect_of(&o.rect),
                stroke(Entity::Opening(o.id), ink),
                Some(bg),
            );
        }
        for o in &scene.occurrences {
            outline(
                &rect_of(&o.rect),
                stroke(Entity::Occurrence(o.id), Color32::from_rgb(40, 150, 90)),
                None,
            );
        }
        for d in &scene.datums {
            let color = Color32::from_rgb(190, 60, 190);
            match d.geometry {
                DatumGeometry::Point { x, y } => {
                    let p = v.to_screen((x.nanometers(), y.nanometers()));
                    if clip_segment(p, p, lo, hi).is_some() {
                        painter.circle_filled(pos(p), 3.0, color);
                        painter.text(
                            pos(p) + egui::vec2(5.0, -5.0),
                            Align2::LEFT_BOTTOM,
                            &d.name,
                            font.clone(),
                            color,
                        );
                    }
                }
                DatumGeometry::XLine { x } => {
                    let a = v.to_screen((x.nanometers(), 0));
                    let (p, q) = (Screen { x: a.x, y: lo.y }, Screen { x: a.x, y: hi.y });
                    if let Some((p, q)) = clip_segment(p, q, lo, hi) {
                        painter.line_segment([pos(p), pos(q)], Stroke::new(1.0, color));
                    }
                }
                DatumGeometry::YLine { y } => {
                    let a = v.to_screen((0, y.nanometers()));
                    let (p, q) = (Screen { x: lo.x, y: a.y }, Screen { x: hi.x, y: a.y });
                    if let Some((p, q)) = clip_segment(p, q, lo, hi) {
                        painter.line_segment([pos(p), pos(q)], Stroke::new(1.0, color));
                    }
                }
            }
        }

        // Labels, where their anchor is on the canvas.
        let label = |at: World, text: &str, color: Color32| {
            let p = v.to_screen(at);
            if rect.contains(pos(p)) {
                painter.text(pos(p), Align2::CENTER_CENTER, text, font.clone(), color);
            }
        };
        for w in &scene.walls {
            let (min, max) = w
                .footprint
                .iter()
                .fold(((i64::MAX, i64::MAX), (i64::MIN, i64::MIN)), |(a, b), p| {
                    ((a.0.min(p.0), a.1.min(p.1)), (b.0.max(p.0), b.1.max(p.1)))
                });
            let mid = |a: i64, b: i64| a / 2 + b / 2;
            label((mid(min.0, max.0), mid(min.1, max.1)), &w.label, ink);
        }
        for o in &scene.occurrences {
            label(o.center, &o.label, Color32::from_rgb(40, 150, 90));
        }

        // The drag preview: the grabbed reference at its snapped target.
        if let (Some(drag), Some(p)) = (&self.drag, &self.preview) {
            let shape: Option<Vec<World>> = match drag.entity {
                Entity::Wall(id) => scene
                    .walls
                    .iter()
                    .find(|w| w.id == id)
                    .map(|w| w.footprint.clone()),
                Entity::Opening(id) => scene
                    .openings
                    .iter()
                    .find(|o| o.id == id)
                    .map(|o| rect_of(&o.rect)),
                Entity::Occurrence(id) => scene
                    .occurrences
                    .iter()
                    .find(|o| o.id == id)
                    .map(|o| rect_of(&o.rect)),
                Entity::Slab(_) => None,
            };
            if let Some(shape) = shape {
                let moved: Vec<World> = shape
                    .iter()
                    .map(|&(x, y)| match p.axis {
                        Axis::X => (x.saturating_add(p.delta), y),
                        Axis::Y => (x, y.saturating_add(p.delta)),
                    })
                    .collect();
                outline(&moved, Stroke::new(1.5, accent), None);
            }
            let guide = match p.axis {
                Axis::X => {
                    let x = v.to_screen((p.snap.value, 0)).x;
                    (Screen { x, y: lo.y }, Screen { x, y: hi.y })
                }
                Axis::Y => {
                    let y = v.to_screen((0, p.snap.value)).y;
                    (Screen { x: lo.x, y }, Screen { x: hi.x, y })
                }
            };
            if let Some((a, b)) = clip_segment(guide.0, guide.1, lo, hi) {
                painter.add(Shape::dashed_line(
                    &[pos(a), pos(b)],
                    Stroke::new(1.0, accent),
                    6.0,
                    4.0,
                ));
            }
        }
        // Wall mode: the run so far, the next segment and the next point.
        if let Some(tool) = &self.walls {
            let mut run: Vec<World> = tool.points().to_vec();
            run.extend(self.next);
            for w in run.windows(2) {
                if let [a, b] = w
                    && let Some((p, q)) = clip_segment(v.to_screen(*a), v.to_screen(*b), lo, hi)
                {
                    painter.line_segment([pos(p), pos(q)], Stroke::new(2.0, accent));
                }
            }
            for p in &run {
                let s = v.to_screen(*p);
                if clip_segment(s, s, lo, hi).is_some() {
                    painter.circle_stroke(pos(s), 4.0, Stroke::new(1.5, accent));
                }
            }
        }
        // Dimension mode: the reference under the pointer, then the first
        // pick over it.
        let highlight = |picked: &Picked, color: Color32| {
            let stroke = Stroke::new(3.0, color);
            let line = |p: Screen, q: Screen| {
                if let Some((p, q)) = clip_segment(p, q, lo, hi) {
                    painter.line_segment([pos(p), pos(q)], stroke);
                }
            };
            match mark(scene, picked) {
                Some(Mark::Segment(a, b)) => line(v.to_screen(a), v.to_screen(b)),
                Some(Mark::XLine(x)) => {
                    let x = v.to_screen((x, 0)).x;
                    line(Screen { x, y: lo.y }, Screen { x, y: hi.y });
                }
                Some(Mark::YLine(y)) => {
                    let y = v.to_screen((0, y)).y;
                    line(Screen { x: lo.x, y }, Screen { x: hi.x, y });
                }
                Some(Mark::Point(p)) => {
                    let s = v.to_screen(p);
                    if clip_segment(s, s, lo, hi).is_some() {
                        painter.circle_stroke(pos(s), 7.0, stroke);
                    }
                }
                None => {}
            }
        };
        if let Some(h) = &self.hover {
            highlight(h, HOVER);
        }
        if let Some(f) = self.first_pick() {
            highlight(f, PICKED);
        }
        let gray = if dark {
            Color32::from_gray(140)
        } else {
            Color32::from_gray(120)
        };
        let hits = self.paint_dimensions(painter, v, scene, rect, (lo, hi), (ink, gray), &font);
        painter.rect_stroke(rect, 0.0, Stroke::new(1.0, faint), StrokeKind::Inside);
        hits
    }

    /// Draws every dimension of the scene where the layout puts it: rows
    /// outside the plan's bounding box, `DIMENSION_GAP` px out and then
    /// `DIMENSION_ROW` px apart. A driving dimension is solid in ink with
    /// its value; a reference one dashed in gray with its measured value in
    /// parentheses; a selected one blue; a conflicting one red. Returns the
    /// rectangle each one occupies, for clicks.
    #[allow(clippy::too_many_arguments)]
    fn paint_dimensions(
        &self,
        painter: &egui::Painter,
        v: &Viewport,
        scene: &Scene,
        rect: egui::Rect,
        (lo, hi): (Screen, Screen),
        (ink, gray): (Color32, Color32),
        font: &FontId,
    ) -> Vec<(Uuid, egui::Rect)> {
        let mut hits = Vec::new();
        let Some((min, max)) = scene.bounds() else {
            return hits;
        };
        // min is the bottom left on screen, max the top right.
        let (smin, smax) = (v.to_screen(min), v.to_screen(max));
        for d in &scene.dimensions {
            let reference = d.status == ConstraintStatus::Reference;
            let selected = self.selected_dimension == Some(d.constraint);
            let color = if self.conflicting_dimensions.contains(&d.constraint) {
                CONFLICT
            } else if selected {
                SELECTED
            } else if reference {
                gray
            } else {
                ink
            };
            let stroke = Stroke::new(if selected { 2.0 } else { 1.0 }, color);
            let segment = |p: Screen, q: Screen| {
                if let Some((p, q)) = clip_segment(p, q, lo, hi) {
                    if reference {
                        painter.add(Shape::dashed_line(&[pos(p), pos(q)], stroke, 5.0, 3.0));
                    } else {
                        painter.line_segment([pos(p), pos(q)], stroke);
                    }
                }
            };
            let horizontal = d.axis == Axis::X;
            let offset =
                DIMENSION_GAP + DIMENSION_ROW * f64::from(u32::try_from(d.row).unwrap_or(u32::MAX));
            // Screen coordinates across the line: the box edge, and which
            // way is outward.
            let (edge, outward) = match d.side {
                Side::Below => (smin.y, 1.0),
                Side::Above => (smax.y, -1.0),
                Side::Left => (smin.x, -1.0),
                Side::Right => (smax.x, 1.0),
            };
            let base = edge + outward * offset;
            // The screen point at `along` on the axis and `across` pixels.
            let at = |along: i64, across: f64| {
                if horizontal {
                    Screen {
                        x: v.to_screen((along, 0)).x,
                        y: across,
                    }
                } else {
                    Screen {
                        x: across,
                        y: v.to_screen((0, along)).y,
                    }
                }
            };
            for (end, foot) in d.ends.iter().zip(d.feet) {
                let start = match foot {
                    Some(f) if horizontal => v.to_screen((0, f)).y + outward * 3.0,
                    Some(f) => v.to_screen((f, 0)).x + outward * 3.0,
                    None => edge,
                };
                segment(at(*end, start), at(*end, base + outward * 4.0));
                // The tick: a short slash through the dimension line.
                let c = at(*end, base);
                segment(
                    Screen {
                        x: c.x - 4.0,
                        y: c.y + 4.0,
                    },
                    Screen {
                        x: c.x + 4.0,
                        y: c.y - 4.0,
                    },
                );
            }
            let (a, b) = (at(d.ends[0], base), at(d.ends[1], base));
            segment(a, b);
            let mid = (i128::from(d.ends[0]) + i128::from(d.ends[1])) / 2;
            let mid = at(i64::try_from(mid).unwrap_or(d.ends[0]), base);
            let galley = painter.layout_no_wrap(d.text.clone(), font.clone(), color);
            let size = galley.size();
            let text_rect = if horizontal {
                let r = Align2::CENTER_BOTTOM.anchor_size(pos(mid) - egui::vec2(0.0, 2.0), size);
                if rect.contains(r.center()) {
                    painter.galley(r.min, galley, color);
                }
                r
            } else {
                // Read from below, left of the line.
                let anchor = pos(mid) - egui::vec2(2.0, 0.0);
                let r = egui::Rect::from_center_size(
                    anchor - egui::vec2(size.y / 2.0, 0.0),
                    egui::vec2(size.y, size.x),
                );
                if rect.contains(r.center()) {
                    painter.add(
                        // `pos` is the unrotated top left; the bottom center
                        // is the pivot, placed at `anchor`.
                        egui::epaint::TextShape::new(
                            anchor - egui::vec2(size.x / 2.0, size.y),
                            galley,
                            color,
                        )
                        .with_angle_and_anchor(-std::f32::consts::FRAC_PI_2, Align2::CENTER_BOTTOM),
                    );
                }
                r
            };
            let line = egui::Rect::from_two_pos(pos(a), pos(b)).expand(PICK_PX as f32);
            hits.push((d.constraint, line.union(text_rect)));
        }
        hits
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::viewport::{MAX_SCALE, MIN_SCALE};
    use eframe::egui::{Context, RawInput, Rect, epaint::ClippedShape, vec2};
    use proptest::prelude::*;
    use std::sync::OnceLock;

    /// The shop fixture and the T-203 room, built once.
    fn scenes() -> &'static [Scene; 2] {
        static SCENES: OnceLock<[Scene; 2]> = OnceLock::new();
        SCENES.get_or_init(|| {
            let (_, shop) = crate::scene::tests::shop("highlight");
            let (_, room) = crate::session::dimension_tests::t203_room("highlight-room");
            let build = |d| Scene::build(d).unwrap_or_else(|e| panic!("{e}"));
            [build(&shop), build(&room)]
        })
    }

    fn context() -> Context {
        static FONTS: OnceLock<egui::FontDefinitions> = OnceLock::new();
        let ctx = Context::default();
        ctx.set_fonts(
            FONTS
                .get_or_init(|| crate::system_fonts().unwrap_or_else(|| panic!("no font")))
                .clone(),
        );
        ctx
    }

    /// One frame of the plan filling a 1600 x 1000 window; what it picked.
    fn frame(
        ctx: &Context,
        plan: &mut PlanView,
        scene: &Scene,
        events: Vec<egui::Event>,
    ) -> (Option<Pick>, Vec<ClippedShape>) {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1000.0))),
            events,
            ..RawInput::default()
        };
        let mut picked = None;
        let mut o = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                if let Event::Picked(p) = plan.show(ui, scene) {
                    picked = Some(p);
                }
            });
        });
        o.textures_delta.clear();
        (picked, o.shapes)
    }

    /// The lines and circles in `shapes` stroked in `color`: segments as
    /// their two ends, circles as their center twice.
    fn strokes(shapes: &[ClippedShape], color: Color32) -> Vec<[Pos2; 2]> {
        fn walk(shape: &Shape, color: Color32, out: &mut Vec<[Pos2; 2]>) {
            match shape {
                Shape::LineSegment { points, stroke } if stroke.color == color => {
                    out.push(*points);
                }
                Shape::Circle(c) if c.stroke.color == color => out.push([c.center, c.center]),
                Shape::Vec(v) => v.iter().for_each(|s| walk(s, color, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        shapes.iter().for_each(|c| walk(&c.shape, color, &mut out));
        out
    }

    /// Whether any line or circle in `shapes` is stroked in `color`.
    pub(crate) fn drawn(shapes: &[ClippedShape], color: Color32) -> bool {
        !strokes(shapes, color).is_empty()
    }

    /// Checks that what is stroked in `color` is `picked`'s mark: every
    /// stroke lies on it, and it is drawn if the mark passes within 300 px
    /// of `pointer` (so on the canvas). A mark far off the canvas (the
    /// nearest face of a thick wall, zoomed in close) draws nothing.
    fn shows(
        shapes: &[ClippedShape],
        color: Color32,
        scene: &Scene,
        v: &Viewport,
        picked: &Picked,
        pointer: Pos2,
    ) -> Result<(), String> {
        let drawn = strokes(shapes, color);
        let on = |a: f32, b: f64| (f64::from(a) - b).abs() < 0.01;
        // The mark's screen line: vertical at x, or horizontal at y.
        let (vertical, at, distance) = match mark(scene, picked) {
            Some(Mark::Segment(a, b)) if a.0 == b.0 => {
                let x = v.to_screen(a).x;
                (true, x, (f64::from(pointer.x) - x).abs())
            }
            Some(Mark::Segment(a, _)) => {
                let y = v.to_screen(a).y;
                (false, y, (f64::from(pointer.y) - y).abs())
            }
            Some(Mark::XLine(x)) => {
                let x = v.to_screen((x, 0)).x;
                (true, x, (f64::from(pointer.x) - x).abs())
            }
            Some(Mark::YLine(y)) => {
                let y = v.to_screen((0, y)).y;
                (false, y, (f64::from(pointer.y) - y).abs())
            }
            Some(Mark::Point(p)) => {
                let s = v.to_screen(p);
                for [a, b] in &drawn {
                    if !(on(a.x, s.x) && on(a.y, s.y) && a == b) {
                        return Err(format!("{a:?} is not the point {s:?}"));
                    }
                }
                let d = (f64::from(pointer.x) - s.x).hypot(f64::from(pointer.y) - s.y);
                if d <= 300.0 && drawn.is_empty() {
                    return Err(format!("{} not drawn", picked.name));
                }
                return Ok(());
            }
            None => return Err(format!("{} has no mark", picked.name)),
        };
        for [a, b] in &drawn {
            let ok = if vertical {
                on(a.x, at) && on(b.x, at)
            } else {
                on(a.y, at) && on(b.y, at)
            };
            if !ok {
                return Err(format!("{a:?}-{b:?} is not on {} at {at}", picked.name));
            }
        }
        if distance <= 300.0 && drawn.is_empty() {
            return Err(format!("{} not drawn, {distance} px away", picked.name));
        }
        Ok(())
    }

    fn button(at: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }
    }

    /// A world point on a wall or datum of `scene`: the `i`th of them, at
    /// fractions `s`, `t` of its extent.
    fn target(scene: &Scene, i: usize, s: f64, t: f64) -> World {
        let lerp = |a: i64, b: i64, f: f64| a + ((b - a) as f64 * f) as i64;
        let n = scene.walls.len() + scene.datums.len();
        let i = i % n;
        if let Some(w) = scene.walls.get(i) {
            let xs = w.footprint.iter().map(|p| p.0);
            let ys = w.footprint.iter().map(|p| p.1);
            let (x0, x1) = (xs.clone().min().unwrap_or(0), xs.max().unwrap_or(0));
            let (y0, y1) = (ys.clone().min().unwrap_or(0), ys.max().unwrap_or(0));
            return (lerp(x0, x1, s), lerp(y0, y1, t));
        }
        let far = 30 * 304_800_000;
        match scene.datums.get(i - scene.walls.len()).map(|d| d.geometry) {
            Some(DatumGeometry::Point { x, y }) => (x.nanometers(), y.nanometers()),
            Some(DatumGeometry::XLine { x }) => (x.nanometers(), lerp(-far, far, t)),
            Some(DatumGeometry::YLine { y }) => (lerp(-far, far, s), y.nanometers()),
            None => (0, 0),
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(96))]

        /// Done when: the highlighted reference is always the one a click
        /// at that point picks, over the shop fixture and the T-203 room,
        /// at any zoom and pointer position. The first pick then stays
        /// highlighted and the status line names both.
        #[test]
        fn the_highlighted_reference_is_the_one_a_click_picks(
            room in any::<bool>(),
            i in 0usize..1000,
            s in 0.0f64..=1.0,
            t in 0.0f64..=1.0,
            zoom in 0.0f64..=1.0,
            dx in -20.0f32..20.0,
            dy in -20.0f32..20.0,
        ) {
            let scene = &scenes()[usize::from(room)];
            let at = target(scene, i, s, t);
            let scale = MIN_SCALE * (MAX_SCALE / MIN_SCALE).powf(zoom);
            let ctx = context();
            let mut plan = PlanView::new();
            plan.dimensions = Some(DimensionTool::default());
            plan.viewport = Some(Viewport::new(at, scale, Screen { x: 0.0, y: 0.0 }));
            frame(&ctx, &mut plan, scene, Vec::new());
            let v = plan.viewport.clone().unwrap_or_else(|| panic!("viewport"));
            let p = pos(v.to_screen(at)) + vec2(dx, dy);

            // Hovering: the highlight is drawn exactly when a reference is
            // under the pointer, and the status line names it.
            frame(&ctx, &mut plan, scene, vec![egui::Event::PointerMoved(p)]);
            let (_, shapes) = frame(&ctx, &mut plan, scene, vec![egui::Event::PointerMoved(p)]);
            let hovered = plan.hover.clone();
            prop_assert!(!drawn(&shapes, PICKED));
            match &hovered {
                Some(h) => {
                    let shown = shows(&shapes, HOVER, scene, &v, h, p);
                    prop_assert!(shown.is_ok(), "{:?}", shown);
                }
                None => prop_assert!(!drawn(&shapes, HOVER)),
            }
            if let Some(h) = &hovered {
                let named = format!("under the pointer: {}", h.name);
                prop_assert!(plan.instruction().ends_with(&named), "{}", plan.instruction());
            }

            // A click there picks that reference, or is refused if none.
            frame(&ctx, &mut plan, scene, vec![button(p, true)]);
            let (picked, _) = frame(&ctx, &mut plan, scene, vec![button(p, false)]);
            let first = plan.dimensions.as_ref().and_then(DimensionTool::first).cloned();
            prop_assert_eq!(&first, &hovered, "at {:?} scale {}", p, scale);
            match &hovered {
                Some(_) => prop_assert_eq!(picked, Some(Pick::First)),
                None => prop_assert!(matches!(picked, Some(Pick::Refused(_)))),
            }

            // The first pick stays highlighted; the status line names both.
            let (_, shapes) = frame(&ctx, &mut plan, scene, vec![egui::Event::PointerMoved(p)]);
            match &hovered {
                Some(h) => {
                    let shown = shows(&shapes, PICKED, scene, &v, h, p);
                    prop_assert!(shown.is_ok(), "{:?}", shown);
                }
                None => prop_assert!(!drawn(&shapes, PICKED)),
            }
            if let Some(h) = &hovered {
                prop_assert_eq!(
                    plan.instruction(),
                    format!(
                        "Dimension from {}: pick the second reference, Esc to cancel; \
                         under the pointer: {}",
                        h.name, h.name
                    )
                );
            }
        }
    }

    #[test]
    fn nothing_is_highlighted_outside_dimension_mode() {
        let scene = &scenes()[1];
        let ctx = context();
        let mut plan = PlanView::new();
        let wall = scene.walls.first().unwrap_or_else(|| panic!("walls"));
        let at = wall.footprint.first().copied().unwrap_or((0, 0));
        plan.viewport = Some(Viewport::new(at, 1e-6, Screen { x: 0.0, y: 0.0 }));
        frame(&ctx, &mut plan, scene, Vec::new());
        let v = plan.viewport.clone().unwrap_or_else(|| panic!("viewport"));
        let p = pos(v.to_screen(at));
        plan.dimensions = Some(DimensionTool::default());
        frame(&ctx, &mut plan, scene, vec![egui::Event::PointerMoved(p)]);
        frame(&ctx, &mut plan, scene, vec![egui::Event::PointerMoved(p)]);
        assert!(plan.hover.is_some(), "a reference at the wall corner");
        plan.dimensions = None;
        let (_, shapes) = frame(&ctx, &mut plan, scene, vec![egui::Event::PointerMoved(p)]);
        assert!(plan.hover.is_none());
        assert!(!drawn(&shapes, HOVER) && !drawn(&shapes, PICKED));
        assert!(!plan.instruction().contains("under the pointer"));
    }
}
