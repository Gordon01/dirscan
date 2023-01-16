use std::sync::mpsc::Receiver;
use std::thread;

//#[derive(PartialEq)]
enum ScanState {
    Idle,
    Scanning(Receiver<Option<Vec<(String, f32)>>>),
    Done(Vec<(String, f32)>),
    Error(String),
}

/// We derive Deserialize/Serialize so we can persist app state on shutdown.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)] // if we add new fields, give them default values when deserializing old state
pub struct TemplateApp {
    // Path in filesystem to scan
    path: String,
    #[serde(skip)]
    state: ScanState,
    // this how you opt-out of serialization of a member
    #[serde(skip)]
    value: f32,
}

impl Default for TemplateApp {
    fn default() -> Self {
        Self {
            path: "C:\\Projects\\rust".to_owned(),
            state: ScanState::Idle,
            value: 2.7,
        }
    }
}

impl TemplateApp {
    /// Called once before the first frame.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // This is also where you can customize the look and feel of egui using
        // `cc.egui_ctx.set_visuals` and `cc.egui_ctx.set_fonts`.

        // Load previous app state (if any).
        // Note that you must enable the `persistence` feature for this to work.
        if let Some(storage) = cc.storage {
            return eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default();
        }

        Default::default()
    }
}

impl eframe::App for TemplateApp {
    /// Called by the frame work to save state before shutdown.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, self);
    }

    /// Called each time the UI needs repainting, which may be many times per second.
    /// Put your widgets into a `SidePanel`, `TopPanel`, `CentralPanel`, `Window` or `Area`.
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let Self {
            path,
            state,
            value: _,
        } = self;

        // Examples of how to create different panels and windows.
        // Pick whichever suits you.
        // Tip: a good default choice is to just keep the `CentralPanel`.
        // For inspiration and more examples, go to https://emilk.github.io/egui

        #[cfg(not(target_arch = "wasm32"))] // no File->Quit on web pages!
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            // The top panel is often a good place for a menu bar:
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Quit").clicked() {
                        frame.close();
                    }
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Dir scan");

            ui.text_edit_singleline(path);
            match state {
                ScanState::Idle => {
                    add_scan_button(ui, ctx, state, path);
                }
                ScanState::Scanning(rx) => {
                    if let Ok(scan_result) = rx.try_recv() {
                        match scan_result {
                            Some(dirs) => *state = ScanState::Done(dirs),
                            None => *state = ScanState::Error(String::from("unknown")),
                        }
                    } else {
                        ui.label("Scanning in progress...");
                    }
                }
                ScanState::Done(dirs) => {
                    for dir in dirs {
                        ui.horizontal(|ui| {
                            ui.label(&dir.0);
                            ui.add(egui::ProgressBar::new(dir.1).show_percentage());
                        });
                    }
                    add_scan_button(ui, ctx, state, path);
                }
                ScanState::Error(e) => {
                    ui.label(format!("Error: {e}"));
                    add_scan_button(ui, ctx, state, path);
                }
            }

            ui.label("<END>");
        });

        if false {
            egui::Window::new("Window").show(ctx, |ui| {
                ui.label("Windows can be moved by dragging them.");
                ui.label("They are automatically sized based on contents.");
                ui.label("You can turn on resizing and scrolling if you like.");
                ui.label("You would normally choose either panels OR windows.");
            });
        }
    }
}

fn add_scan_button(ui: &mut egui::Ui, ctx: &egui::Context, state: &mut ScanState, path: &str) {
    if ui.button("Calculate").clicked() {
        use std::sync::mpsc::channel;

        let paths = std::fs::read_dir(path);
        if let Err(e) = paths {
            *state = ScanState::Error(e.to_string());
            return;
        }

        let (tx, rx) = channel();
        *state = ScanState::Scanning(rx);

        let ctx = ctx.clone();
        thread::spawn(move || {
            thread::sleep(std::time::Duration::from_secs(1));

            let res = paths
                .unwrap()
                .map(|p| (p.unwrap().file_name().to_str().unwrap().to_owned(), 0.5))
                .collect();

            tx.send(Some(res)).unwrap();
            ctx.request_repaint();
        });
    }
}
