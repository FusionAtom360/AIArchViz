//! C5 app: egui app and wgpu viewer (spec 3, 3.1, D-14).
//!
//! T-201 is the shell: open, create, close and save a copy of a project
//! folder. While a project is open the app is its host process (3.1): CLI
//! and agent commands sent to that folder run here, and the window repaints
//! when one commits.

mod session;

use std::path::Path;
use std::sync::Arc;

use eframe::egui;
use session::{Session, View};

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

struct App {
    folder: String,
    name: String,
    building: String,
    copy_to: String,
    message: String,
    session: Option<Session>,
    /// The view and the version it was built at.
    view: Option<View>,
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
        }
    }

    fn opened(&mut self, result: Result<Session, archviz_cli::CliError>) {
        match result {
            Ok(session) => {
                self.message = format!("Opened {}", session.dir().display());
                self.session = Some(session);
                self.view = None;
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
                    self.view = None;
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

    fn project(&mut self, ui: &mut egui::Ui) {
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
        if !view.walls.is_empty() {
            ui.separator();
            ui.label(format!("Walls: {}", view.walls.join(", ")));
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.project(ui));
        });
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
