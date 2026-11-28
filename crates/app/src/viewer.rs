//! The 3D viewer (T-206, 13.1, D-14): a verification view, not a
//! visualization product. Orbit and walk navigation, picking, an
//! axis-aligned section plane and point-to-point measurement in
//! feet-inches, drawn with wgpu (gpu.rs) from the export crate's exact
//! solids (model3d.rs). It also provides the view context (9.1, 9.3):
//! camera pose, heading, selection and hovered entity.
//!
//! Orbit: left drag turns about the target, right or middle drag pans,
//! the wheel zooms, a double click orbits about the vertex clicked. Walk:
//! left drag looks around, W A S D walk, Q E go down and up (Shift is
//! faster), the wheel steps forward and back. A click selects; with
//! Measure on, two clicks measure between the vertices nearest the pointer.

use archviz_core::{Document, ProjectUnit, Uuid, ViewContext, format_length};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke};
use eframe::egui_wgpu::RenderState;

use crate::camera::{Camera, Mode, P3, V3, cross, dot};
use crate::gpu::{Gpu, Vertex};
use crate::model3d::{Measure, Model, Section};

/// Walking speed: 4 ft per second, four times that with Shift.
const WALK_SPEED: f64 = 4.0 * 304_800_000.0;
const MEASURE: Color32 = Color32::from_rgb(200, 40, 160);
const CUT: [f32; 4] = [0.85, 0.1, 0.1, 1.0];
const EDGE: [f32; 4] = [0.15, 0.15, 0.18, 1.0];

/// What a frame of the viewer asks of the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    None,
    /// The user clicked an element (its id and kind) or empty space.
    Select(Option<(Uuid, &'static str)>),
}

pub struct Viewer {
    pub camera: Camera,
    /// The model and the version it was built at.
    built: Option<(u64, Result<Model, String>)>,
    /// The model cut by the section, and the version and section it was cut at.
    cut: Option<(u64, Section, Model)>,
    /// The camera has been fitted to a model.
    fitted: bool,
    /// The section plane is on.
    pub section_on: bool,
    /// 0 = X, 1 = Y, 2 = Z.
    pub section_axis: usize,
    pub section_text: String,
    /// Keep the side below (or west, or south of) the plane.
    pub keep_below: bool,
    /// Measure mode: clicks pick vertices.
    pub measuring: bool,
    first: Option<P3>,
    pub measure: Option<Measure>,
    pub selected: Option<Uuid>,
    pub hovered: Option<Uuid>,
    /// Measure mode: the vertex a click would pick.
    snap: Option<P3>,
    gpu: Option<Gpu>,
    /// Where the canvas was drawn last frame.
    pub rect: Option<Rect>,
}

fn color(kind: &str) -> [f32; 4] {
    match kind {
        "wall" => [0.92, 0.9, 0.86, 1.0],
        "door" => [0.62, 0.45, 0.3, 1.0],
        "window" => [0.55, 0.75, 0.9, 1.0],
        "slab" => [0.6, 0.6, 0.62, 1.0],
        "ceiling" => [0.95, 0.95, 0.95, 1.0],
        "roof" => [0.7, 0.42, 0.36, 1.0],
        _ => [0.5, 0.72, 0.55, 1.0],
    }
}

fn meters(v: V3) -> [f32; 3] {
    [
        (v[0] / 1e9) as f32,
        (v[1] / 1e9) as f32,
        (v[2] / 1e9) as f32,
    ]
}

impl Viewer {
    pub fn new() -> Viewer {
        Viewer {
            camera: Camera::default(),
            built: None,
            cut: None,
            fitted: false,
            section_on: false,
            section_axis: 2,
            section_text: "4'".to_string(),
            keep_below: true,
            measuring: false,
            first: None,
            measure: None,
            selected: None,
            hovered: None,
            snap: None,
            gpu: None,
            rect: None,
        }
    }

    /// The view context (9.1): the camera pose, the selection and the
    /// element under the pointer.
    pub fn context(&self) -> ViewContext {
        let selection: Vec<Uuid> = self.selected.into_iter().collect();
        self.camera.context(&selection, self.hovered)
    }

    /// The section as typed, if it is on and the offset reads.
    pub fn section(&self, unit: ProjectUnit) -> Option<Result<Section, String>> {
        if !self.section_on {
            return None;
        }
        Some(
            crate::dimension::parse_signed(&self.section_text, unit).map(|at| Section {
                axis: self.section_axis,
                at: at.nanometers(),
                keep_below: self.keep_below,
            }),
        )
    }

    /// What the viewer asks of the user now.
    pub fn instruction(&self) -> String {
        match (self.measuring, self.first) {
            (true, None) => "Measure: click the first point (the nearest corner is used)".to_string(),
            (true, Some(_)) => "Measure: click the second point; Esc starts over".to_string(),
            (false, _) => match self.camera.mode {
                Mode::Orbit => {
                    "3D: drag to orbit, right drag to pan, wheel to zoom, double click to orbit about a point"
                        .to_string()
                }
                Mode::Walk => {
                    "3D walk: drag to look, W A S D to walk, Q E down and up, Shift faster"
                        .to_string()
                }
            },
        }
    }

    /// The measurement, or the element under the pointer, for the status line.
    pub fn readout(&self) -> String {
        if let Some(m) = &self.measure {
            return format!("Measured {}", m.text());
        }
        let hovered = self.hovered.and_then(|id| {
            self.model()
                .and_then(|m| m.items.iter().find(|i| i.id == id))
                .map(|i| format!("{} {}", i.kind, i.name))
        });
        hovered.unwrap_or_default()
    }

    fn model(&self) -> Option<&Model> {
        match &self.built {
            Some((_, Ok(m))) => Some(m),
            _ => None,
        }
    }

    /// Builds the model when the document changed; fits the camera the
    /// first time there is something to see.
    fn refresh(&mut self, version: u64, document: impl FnOnce() -> Document) {
        if self.built.as_ref().is_none_or(|(v, _)| *v != version) {
            self.built = Some((version, Model::build(&document())));
            self.cut = None;
        }
        if !self.fitted
            && let Some((lo, hi)) = self.model().and_then(Model::bounds)
        {
            self.camera.fit(lo, hi);
            self.fitted = true;
        }
    }

    /// The model as drawn: cut by the section when one is on.
    fn shown(&mut self, version: u64, section: Option<Section>) -> Option<&Model> {
        let Some(section) = section else {
            return self.model();
        };
        let stale = self
            .cut
            .as_ref()
            .is_none_or(|(v, s, _)| *v != version || *s != section);
        if stale {
            let cut = self.model()?.cut(&section);
            self.cut = Some((version, section, cut));
        }
        self.cut.as_ref().map(|(_, _, m)| m)
    }

    pub fn fit(&mut self) {
        if let Some((lo, hi)) = self.model().and_then(Model::bounds) {
            self.camera.fit(lo, hi);
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, unit: ProjectUnit) {
        ui.horizontal(|ui| {
            for (mode, name) in [(Mode::Orbit, "Orbit"), (Mode::Walk, "Walk")] {
                if ui
                    .add(egui::Button::selectable(self.camera.mode == mode, name))
                    .clicked()
                {
                    self.camera.mode = mode;
                }
            }
            if ui.button("Fit 3D").clicked() {
                self.fit();
            }
            if ui
                .add(egui::Button::selectable(self.measuring, "Measure"))
                .clicked()
            {
                self.measuring = !self.measuring;
                self.first = None;
                self.snap = None;
            }
            ui.separator();
            ui.checkbox(&mut self.section_on, "Section");
            egui::ComboBox::from_id_salt("section-axis")
                .selected_text(["X", "Y", "Z"][self.section_axis.min(2)])
                .width(40.0)
                .show_ui(ui, |ui| {
                    for (k, name) in ["X", "Y", "Z"].iter().enumerate() {
                        ui.selectable_value(&mut self.section_axis, k, *name);
                    }
                });
            ui.label("at");
            ui.add(egui::TextEdit::singleline(&mut self.section_text).desired_width(90.0));
            match crate::dimension::parse_signed(&self.section_text, unit) {
                Ok(v) => ui.label(format!("reads as {}", format_length(v))),
                Err(e) => ui.colored_label(crate::WARN, e),
            };
            let keep = if self.keep_below {
                ["Keep below", "Keep west", "Keep south"]
            } else {
                ["Keep above", "Keep east", "Keep north"]
            };
            let keep = match self.section_axis {
                0 => keep[1],
                1 => keep[2],
                _ => keep[0],
            };
            if ui.button(keep).clicked() {
                self.keep_below = !self.keep_below;
            }
        });
    }

    /// Draws the toolbar and the view; handles navigation, picking and
    /// measuring. `render` is None without a wgpu renderer (in tests).
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        render: Option<&RenderState>,
        version: u64,
        unit: ProjectUnit,
        document: impl FnOnce() -> Document,
    ) -> Event {
        self.refresh(version, document);
        self.toolbar(ui, unit);
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.first = None;
            self.measure = None;
        }
        let section = match self.section(unit) {
            Some(Ok(s)) => Some(s),
            _ => None,
        };
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        self.rect = Some(rect);
        let size = [f64::from(rect.width()), f64::from(rect.height())];
        self.navigate(ui, &response, size);

        let pointer = response
            .hover_pos()
            .map(|p| [f64::from(p.x - rect.min.x), f64::from(p.y - rect.min.y)]);
        let camera = self.camera.clone();
        let picked = match (self.shown(version, section), pointer) {
            (Some(model), Some(at)) => model
                .pick_vertex(&camera, at, size)
                .map(|(item, vertex)| (item.id, item.kind, vertex)),
            _ => None,
        };
        self.hovered = picked.map(|(id, _, _)| id);
        self.snap = if self.measuring {
            picked.map(|(_, _, v)| v)
        } else {
            None
        };

        // In Measure mode every click picks a point, however quick the
        // second follows the first; elsewhere a double click also orbits
        // about the vertex clicked.
        let mut event = Event::None;
        if response.clicked() && self.measuring {
            if let Some((_, _, v)) = picked {
                match self.first.take() {
                    None => {
                        self.first = Some(v);
                        self.measure = None;
                    }
                    Some(a) => self.measure = Some(Measure { a, b: v }),
                }
            }
        } else if response.clicked() {
            self.selected = picked.map(|(id, _, _)| id);
            event = Event::Select(picked.map(|(id, kind, _)| (id, kind)));
            if response.double_clicked()
                && let Some((_, _, v)) = picked
            {
                let distance = camera.relative(v).iter().map(|c| c * c).sum::<f64>().sqrt();
                self.camera.look_at(v, distance);
            }
        }

        self.paint(ui, render, rect, version, section);
        event
    }

    fn navigate(&mut self, ui: &egui::Ui, response: &egui::Response, size: [f64; 2]) {
        let delta = response.drag_delta();
        let (dx, dy) = (f64::from(delta.x), f64::from(delta.y));
        if response.dragged_by(egui::PointerButton::Primary) {
            self.camera.drag(dx, dy);
        } else if response.dragged_by(egui::PointerButton::Secondary)
            || response.dragged_by(egui::PointerButton::Middle)
        {
            self.camera.pan(dx, dy, size[1]);
        }
        if !response.hovered() {
            return;
        }
        let scroll = ui.input(|i| f64::from(i.smooth_scroll_delta.y));
        if scroll != 0.0 {
            self.camera.zoom((-scroll * 0.002).exp());
        }
        if self.camera.mode == Mode::Walk {
            let (step, keys) = ui.input(|i| {
                let fast = if i.modifiers.shift { 4.0 } else { 1.0 };
                let down = |k| if i.key_down(k) { 1.0 } else { 0.0 };
                (
                    WALK_SPEED * fast * f64::from(i.stable_dt.min(0.1)),
                    [
                        down(egui::Key::W) - down(egui::Key::S),
                        down(egui::Key::D) - down(egui::Key::A),
                        down(egui::Key::E) - down(egui::Key::Q),
                    ],
                )
            });
            if keys.iter().any(|k| *k != 0.0) {
                self.camera
                    .walk(keys[0] * step, keys[1] * step, keys[2] * step);
                ui.ctx().request_repaint();
            }
        }
    }

    /// The vertices to draw, relative to the eye in meters.
    fn vertices(&self, model: &Model) -> (Vec<Vertex>, Vec<Vertex>) {
        let cam = &self.camera;
        let (mut fills, mut lines) = (Vec::new(), Vec::new());
        for item in &model.items {
            let mut c = color(item.kind);
            if Some(item.id) == self.selected {
                c = [1.0, 0.6, 0.2, 1.0];
            } else if Some(item.id) == self.hovered {
                c = [
                    c[0] * 0.85 + 0.15,
                    c[1] * 0.85 + 0.15,
                    c[2] * 0.85 + 0.15,
                    1.0,
                ];
            }
            for t in &item.triangles {
                // The normal from exact edge vectors.
                let e = |p: &P3, q: &P3| [0, 1, 2].map(|k| (q[k] - p[k]) as f64);
                let n = cross(e(&t[0], &t[1]), e(&t[0], &t[2]));
                let len = dot(n, n).sqrt().max(f64::MIN_POSITIVE);
                let normal = n.map(|v| (v / len) as f32);
                for p in t {
                    fills.push(Vertex {
                        pos: meters(cam.relative(*p)),
                        normal,
                        color: c,
                    });
                }
            }
            for (segments, c) in [(&item.edges, EDGE), (&item.cuts, CUT)] {
                for s in segments {
                    for p in s {
                        lines.push(Vertex {
                            pos: meters(cam.relative(*p)),
                            normal: [0.0; 3],
                            color: c,
                        });
                    }
                }
            }
        }
        (fills, lines)
    }

    fn paint(
        &mut self,
        ui: &egui::Ui,
        render: Option<&RenderState>,
        rect: Rect,
        version: u64,
        section: Option<Section>,
    ) {
        let painter = ui.painter_at(rect);
        let size = [f64::from(rect.width()), f64::from(rect.height())];
        let pixels = ui.ctx().pixels_per_point();
        let drawn = match render {
            Some(rs) => {
                let model = self.shown(version, section).cloned();
                model.map(|m| {
                    let (fills, lines) = self.vertices(&m);
                    let light = {
                        let (f, u, r) =
                            (self.camera.forward(), self.camera.up(), self.camera.right());
                        let l = [0, 1, 2].map(|k| -f[k] + 0.6 * u[k] + 0.3 * r[k]);
                        let n = dot(l, l).sqrt();
                        l.map(|v| (v / n) as f32)
                    };
                    let px = [
                        (rect.width() * pixels).round() as u32,
                        (rect.height() * pixels).round() as u32,
                    ];
                    let gpu = self.gpu.get_or_insert_with(|| Gpu::new(rs));
                    gpu.render(rs, px, self.camera.rows(size), light, &fills, &lines)
                })
            }
            None => None,
        };
        match drawn {
            Some(texture) => {
                painter.image(
                    texture,
                    rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => {
                painter.rect_filled(rect, 0.0, Color32::from_gray(220));
                let why = match &self.built {
                    Some((_, Err(e))) => format!("The model cannot be drawn: {e}"),
                    _ if render.is_none() => "3D needs the wgpu renderer".to_string(),
                    _ => "Nothing to show yet".to_string(),
                };
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    why,
                    egui::FontId::proportional(14.0),
                    Color32::DARK_GRAY,
                );
            }
        }
        self.overlay(&painter, rect, size);
    }

    /// Measurement points, the line between them and its length.
    fn overlay(&self, painter: &egui::Painter, rect: Rect, size: [f64; 2]) {
        let at = |p: P3| {
            self.camera
                .project(p, size)
                .map(|s| Pos2::new(rect.min.x + s[0] as f32, rect.min.y + s[1] as f32))
        };
        let stroke = Stroke::new(2.0, MEASURE);
        if let Some(p) = self.snap.and_then(at) {
            painter.circle_stroke(p, 6.0, stroke);
        }
        if let Some(p) = self.first.and_then(at) {
            painter.circle_filled(p, 4.0, MEASURE);
        }
        if let Some(m) = &self.measure
            && let (Some(a), Some(b)) = (at(m.a), at(m.b))
        {
            painter.line_segment([a, b], stroke);
            painter.circle_filled(a, 4.0, MEASURE);
            painter.circle_filled(b, 4.0, MEASURE);
            let text = m
                .distance()
                .map_or_else(|| "too far".to_string(), |d| d.text());
            painter.text(
                Pos2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0 - 8.0),
                egui::Align2::CENTER_BOTTOM,
                text,
                egui::FontId::proportional(16.0),
                MEASURE,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_section_reads_its_offset_on_the_grid() {
        let mut v = Viewer::new();
        assert_eq!(v.section(ProjectUnit::Inches), None);
        v.section_on = true;
        v.section_text = "-2'-6 1/2\"".to_string();
        v.section_axis = 1;
        v.keep_below = false;
        assert_eq!(
            v.section(ProjectUnit::Inches),
            Some(Ok(Section {
                axis: 1,
                at: -(30 * 25_400_000 + 12_700_000),
                keep_below: false,
            }))
        );
        v.section_text = "1/32\"".to_string();
        assert!(matches!(v.section(ProjectUnit::Inches), Some(Err(e)) if e.contains("1/16")));
    }

    /// The renderer runs on this machine's adapter without a validation
    /// error, and draws the shop: the clear color is not everywhere, and the
    /// pixel at the model's center is not background.
    #[test]
    fn the_shop_renders_on_the_gpu() {
        let Some(rs) = crate::gpu::tests::headless() else {
            eprintln!("no wgpu adapter here: skipped");
            return;
        };
        let (_, doc) = crate::scene::tests::shop("viewer-gpu");
        let mut v = Viewer::new();
        v.refresh(1, || doc);
        let model = v
            .shown(1, None)
            .cloned()
            .unwrap_or_else(|| panic!("no model"));
        let (fills, lines) = v.vertices(&model);
        assert!(!fills.is_empty() && !lines.is_empty());
        let size = [640.0, 400.0];
        let scope = rs
            .device
            .push_error_scope(eframe::egui_wgpu::wgpu::ErrorFilter::Validation);
        let mut gpu = Gpu::new(&rs);
        gpu.render(
            &rs,
            [640, 400],
            v.camera.rows(size),
            [0.0, 0.0, 1.0],
            &fills,
            &lines,
        );
        let (pixels, [w, h]) = gpu.read(&rs).unwrap_or_else(|| panic!("no readback"));
        let error = crate::gpu::tests::block_on(scope.pop());
        assert!(error.is_none(), "{error:?}");
        assert_eq!((w, h), (640, 400));
        let clear = pixels[0..4].to_vec();
        let drawn = pixels.chunks(4).filter(|p| *p != clear.as_slice()).count();
        assert!(
            drawn > (w * h / 10) as usize,
            "{drawn} of {} pixels drawn",
            w * h
        );
        let center = ((h / 2 * w + w / 2) * 4) as usize;
        assert_ne!(&pixels[center..center + 4], clear.as_slice());
        // Resizing reuses the egui texture id.
        let id = gpu.render(
            &rs,
            [320, 200],
            v.camera.rows([320.0, 200.0]),
            [0.0, 0.0, 1.0],
            &fills,
            &lines,
        );
        let again = gpu.render(
            &rs,
            [300, 200],
            v.camera.rows([300.0, 200.0]),
            [0.0, 0.0, 1.0],
            &fills,
            &lines,
        );
        assert_eq!(id, again);
    }
}
