use std::collections::HashMap;
use std::sync::{mpsc::Receiver, Arc, Mutex};

use super::scan;
use bytesize::ByteSize;

type FinalEntry = (String, u64);
pub type Cache = HashMap<String, u64>;

pub enum ScanState {
    Idle,
    Scanning((Receiver<Message>, Cache)),
    Done(Vec<FinalEntry>),
    Error(String),
}

pub enum Message {
    Intermediate(Vec<FinalEntry>),
    Done,
}

/// We derive Deserialize/Serialize so we can persist app state on shutdown.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)] // if we add new fields, give them default values when deserializing old state
pub struct TemplateApp {
    // Path in filesystem to scan
    path: String,
    #[serde(skip)]
    state: ScanState,
    // File size cache
    // TODO: use this cache
    #[serde(skip)]
    cache: Arc<Mutex<Cache>>,
}

impl Default for TemplateApp {
    fn default() -> Self {
        Self {
            path: "C:\\Projects\\rust".into(),
            state: ScanState::Idle,
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl TemplateApp {
    /// Called once before the first frame.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Load previous app state (if any).
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
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let Self { path, state, cache } = self;

        #[cfg(not(target_arch = "wasm32"))] // no File->Quit on web pages!
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
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

            ui.horizontal(|ui| {
                if ui.button("Home").clicked() {
                    if let Some(p) = dirs_next::home_dir() {
                        *path = p.to_str().unwrap().to_owned();
                    }
                }

                ui.text_edit_singleline(path);
                if matches!(state, ScanState::Scanning(_)) {
                    if ui.button("Stop").clicked() {
                        *state = ScanState::Idle;
                    }
                } else {
                    if ui.button("Calculate").clicked() {
                        scan::scan_directory(ctx, state, path, cache.clone());
                    }
                }
            });

            match state {
                ScanState::Idle => {}
                ScanState::Scanning((rx, results)) => {
                    if let Ok(scan_result) = rx.try_recv() {
                        match scan_result {
                            Message::Done => {
                                let dirs = sort_results(results.iter());
                                *state = ScanState::Done(dirs);
                                return;
                            }
                            Message::Intermediate(vec) => {
                                for (p, s) in vec {
                                    results.entry(p).and_modify(|size| *size += s).or_insert(s);
                                }
                            }
                        }
                    }

                    ui.label("Scanning in progress...");

                    // We're sorting and calculating sum every time on each repaint
                    // TODO: needs optimisation
                    let dirs = sort_results(results.iter());
                    display_dirs(ui, &dirs);
                }
                ScanState::Done(dirs) => {
                    ui.label("Done");
                    display_dirs(ui, dirs);
                }
                ScanState::Error(e) => {
                    ui.label(format!("Error: {e}"));
                }
            }
        });
    }
}

fn sort_results<'a, I>(iter: I) -> Vec<FinalEntry>
where
    I: Iterator<Item = (&'a String, &'a u64)>,
{
    let mut res: Vec<_> = iter.map(|(p, &s)| (p.to_owned(), s)).collect();
    res.sort_by(|(_, a), (_, b)| b.cmp(a)); // Descending by size
    res.truncate(10); // Keep only 10 top results

    res
}

fn display_dirs(ui: &mut egui::Ui, vec: &Vec<FinalEntry>) {
    let total = vec.iter().map(|(_, s)| s).sum();

    egui::Grid::new("file_grid")
        .num_columns(3)
        .striped(true)
        .show(ui, |ui| {
            for dir in vec {
                ui.label(&dir.0);
                let fraction = dir.1 as f32 / total as f32;
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .show_percentage()
                        .desired_width(200.0),
                );
                ui.label(ByteSize(dir.1).to_string_as(true));
                ui.end_row();
            }

            let total = ByteSize(total).to_string_as(true);
            ui.label(format!("Total: {total}"));
            ui.end_row();
        });
}
