//! The plan view widget (T-202, spec 3.1, 5.3): draws the scene through the
//! 64-bit viewport, pans (middle or right drag), zooms about the pointer
//! (scroll), selects (click) and drags entities into move commands (left
//! drag), and in wall mode draws runs of walls from snapped clicks
//! (T-202b, draw.rs), and in dimension mode picks references for a
//! dimension (T-203, dimension.rs). Floats appear only between the viewport and the painter; every
//! value that leaves this widget as a command argument is snapped first.

use archviz_core::{Axis, DatumGeometry, Function, Length, format_length};
use eframe::egui::{
    self, Align2, Color32, FontId, PointerButton, Pos2, Sense, Shape, Stroke, StrokeKind,
};

use std::collections::BTreeSet;

use archviz_core::Uuid;

use crate::dimension::{DimensionTool, Pick, pick};
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
    /// Walls the solver reports underconstrained (7.3).
    pub free: BTreeSet<Uuid>,
    /// Walls named by a pending proposal's conflicts (7.5).
    pub conflicting: BTreeSet<Uuid>,
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
            free: BTreeSet::new(),
            conflicting: BTreeSet::new(),
        }
    }

    /// Fits the whole scene on the next frame.
    pub fn fit(&mut self) {
        self.viewport = None;
    }

    pub fn status(&self) -> String {
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
            s.push_str(
                "   click the first point to close, Enter or double-click to finish, Esc to cancel",
            );
        }
        if let Some(tool) = &self.dimensions {
            s.push_str(&match tool.first() {
                None => "   pick the first reference (wall face, wall center or datum)".to_string(),
                Some(p) => format!(
                    "   from {}: pick the second reference, Esc to cancel",
                    p.name
                ),
            });
        }
        if let Some(v) = &self.viewport {
            s.push_str(&format!(
                "   1 px = {}",
                length(v.nm(1.0)).trim_start_matches('~')
            ));
        }
        s
    }

    pub fn show(&mut self, ui: &mut egui::Ui, scene: &Scene) -> Event {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        let origin = screen(rect.center());
        let viewport = self.viewport.get_or_insert_with(|| match scene.bounds() {
            Some((min, max)) => Viewport::fit(
                min,
                max,
                (f64::from(rect.width()), f64::from(rect.height())),
                40.0,
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

        let mut event = Event::None;
        let at = response
            .interact_pointer_pos()
            .map(|p| viewport.to_world(screen(p)));
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
            // Dimension mode: clicks pick references.
            self.next = None;
            self.drag = None;
            self.preview = None;
            if response.clicked_by(PointerButton::Primary)
                && let Some(at) = at
            {
                event = Event::Picked(tool.click(pick(scene, at, viewport.nm(SNAP_PX))));
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
            self.selected = at.and_then(|w| scene.pick(w, viewport.nm(PICK_PX)));
        }
        if response.drag_started_by(PointerButton::Primary) {
            // The press position, not where the drag threshold was crossed.
            let press = ui
                .input(|i| i.pointer.press_origin())
                .map(|p| viewport.to_world(screen(p)));
            let picked = press.and_then(|w| scene.pick(w, viewport.nm(PICK_PX)).map(|e| (e, w)));
            self.selected = picked.map(|(e, _)| e);
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
    ) {
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
        painter.rect_stroke(rect, 0.0, Stroke::new(1.0, faint), StrokeKind::Inside);
    }
}
