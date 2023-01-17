use std::collections::{BTreeMap, HashMap};
use std::fs::DirEntry;
use std::sync::{mpsc::Receiver, Arc, Mutex};
use std::thread;

use bytesize::ByteSize;
use walkdir::WalkDir;

type CacheEntry = (String, f32, u64);
type Cache = HashMap<String, u64>;
type Intermediate = BTreeMap<String, (f32, u64)>;

//#[derive(PartialEq)]
enum ScanState {
    Idle,
    Scanning((Receiver<Message>, Intermediate)),
    Done(Vec<CacheEntry>),
    Error(String),
}

enum Message {
    Intermediate(CacheEntry),
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
    #[serde(skip)]
    cache: Arc<Mutex<Cache>>,
}

impl Default for TemplateApp {
    fn default() -> Self {
        Self {
            path: "C:\\Projects\\rust".to_owned(),
            state: ScanState::Idle,
            cache: Arc::new(Mutex::new(HashMap::new())),
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
        let Self { path, state, cache } = self;

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
                    add_scan_button(ui, ctx, state, path, cache.clone());
                }
                ScanState::Scanning((rx, results)) => {
                    if let Ok(scan_result) = rx.try_recv() {
                        match scan_result {
                            Message::Done => {
                                let res = results
                                    .iter()
                                    .map(|(p, (f, s))| (p.to_owned(), *f, *s))
                                    .collect();
                                *state = ScanState::Done(res);
                            }
                            Message::Intermediate((p, f, s)) => {
                                results.insert(p, (f, s));
                            }
                        }
                    } else {
                        ui.label("Scanning in progress...");
                        display_dirs(ui, results.iter().map(|(p, (f, s))| (p.to_owned(), *f, *s)));
                    }
                }
                ScanState::Done(dirs) => {
                    display_dirs(ui, dirs.iter().cloned());
                    add_scan_button(ui, ctx, state, path, cache.clone());
                }
                ScanState::Error(e) => {
                    ui.label(format!("Error: {e}"));
                    add_scan_button(ui, ctx, state, path, cache.clone());
                }
            }

            ui.label("<END>");
        });
    }
}

fn display_dirs<I>(ui: &mut egui::Ui, iter: I)
where
    I: Iterator<Item = (String, f32, u64)>,
{
    egui::Grid::new("file_grid")
        .num_columns(3)
        .striped(true)
        .show(ui, |ui| {
            for dir in iter {
                ui.label(&dir.0);
                ui.add(egui::ProgressBar::new(dir.1).show_percentage());
                ui.label(ByteSize(dir.2).to_string_as(true));
                ui.end_row();
            }
        });
}

fn add_scan_button(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    state: &mut ScanState,
    path: &str,
    cache: Arc<Mutex<Cache>>,
) {
    if ui.button("Calculate").clicked() {
        use std::sync::mpsc::channel;

        let paths = std::fs::read_dir(path);
        if let Err(e) = paths {
            *state = ScanState::Error(e.to_string());
            return;
        }

        let (tx, rx) = channel();
        *state = ScanState::Scanning((rx, BTreeMap::new()));

        let ctx = ctx.clone();
        thread::spawn(move || {
            let mut cache = cache.lock().unwrap();
            cache.insert("Test".to_string(), 2);

            let mut total = 0;

            // Safe because it's checked for errror before
            for path in paths.unwrap() {
                // TODO: Use cache
                let path = path.unwrap();

                let size = calc_dir_size(&path);
                total += size;
                let fraction = size as f32 / total as f32;

                let dir = path.file_name().to_str().unwrap().to_owned();
                tx.send(Message::Intermediate((dir, fraction, size)))
                    .unwrap();
                thread::sleep(std::time::Duration::from_millis(250));
                ctx.request_repaint();
            }

            thread::sleep(std::time::Duration::from_millis(1000));
            tx.send(Message::Done).unwrap();

            ctx.request_repaint();
        });
    }
}

fn calc_dir_size(path: &DirEntry) -> u64 {
    let mut total = 0;
    for e in WalkDir::new(path.path()).into_iter().filter_map(|e| e.ok()) {
        let metadata = e.metadata().unwrap();
        if metadata.is_file() {
            total += metadata.len();
        }
    }

    total
}
