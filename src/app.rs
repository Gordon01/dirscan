use std::collections::HashMap;
use std::sync::{mpsc::Receiver, Arc, Mutex};
use std::thread;

use bytesize::ByteSize;
use walkdir::WalkDir;

type FinalEntry = (String, u64);
type Cache = HashMap<String, u64>;

enum ScanState {
    Idle,
    Scanning((Receiver<Message>, Cache)),
    Done(Vec<FinalEntry>),
    Error(String),
}

enum Message {
    Intermediate(FinalEntry),
    Done,
}

impl ScanState {
    fn is_scanning(&self) -> bool {
        if let ScanState::Scanning(_) = self {
            true
        } else {
            false
        }
    }
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

            ui.horizontal(|ui| {
                if ui.button("Home").clicked() {
                    if let Some(p) = dirs_next::home_dir() {
                        *path = p.to_str().unwrap().to_owned();
                    }
                }

                ui.text_edit_singleline(path);
                if ui.button("Stop").clicked() {
                    *state = ScanState::Idle;
                }
                if !state.is_scanning() {
                    add_scan_button(ui, ctx, state, path, cache.clone());
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
                            Message::Intermediate((p, s)) => {
                                results.entry(p).and_modify(|size| *size += s).or_insert(s);
                            }
                        }
                    }

                    ui.label("Scanning in progress...");

                    // We're sorting and calculating sum every time
                    // Probably needs optimisation
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

fn add_scan_button(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    state: &mut ScanState,
    path: &str,
    cache: Arc<Mutex<Cache>>,
) {
    if ui.button("Calculate").clicked() {
        let sub_paths = if let Ok(p) = std::fs::read_dir(path) {
            p
        } else {
            *state = ScanState::Error(format!("On open root dir: {path}"));
            return;
        };

        use std::sync::mpsc::channel;
        let (tx, rx) = channel();
        *state = ScanState::Scanning((rx, HashMap::new()));

        let ctx = ctx.clone();
        thread::spawn(move || {
            let mut cache = cache.lock().unwrap();
            cache.insert("Test".to_string(), 2);

            let mut sub_iters: Vec<_> = sub_paths
                .filter_map(|p| p.ok()) // Ignore subdirectory if it fails on read
                .map(|p| {
                    (
                        WalkDir::new(p.path())
                            .into_iter()
                            .filter_map(|e| e.ok()) // Ignore any entry on a way
                            .filter(|e| e.file_type().is_file()),
                        p.file_name().to_str().unwrap().to_owned(),
                    )
                })
                .collect();

            let mut flags = vec![true; sub_iters.len()];

            while !flags.iter().all(|&f| f == false) {
                for (i, (iter, name)) in sub_iters.iter_mut().enumerate() {
                    if flags[i] == false {
                        continue;
                    }

                    //let (len, size) = iter.take(2).map(|e| e.metadata().unwrap().len()).enumerate().fold((0, 0),
                    //    |(len, acc), (i, s)| (len.max(i), acc+s));
                    let two: Vec<_> = iter.take(1024).collect();

                    if !two.is_empty() {
                        let size = two.iter().map(|e| e.metadata().unwrap().len()).sum::<u64>();
                        // Fighting the borrow checker here
                        if tx
                            .send(Message::Intermediate((name.to_owned(), size)))
                            .is_err()
                        {
                            println!("Nowhere to send");
                            return;
                        }
                        ctx.request_repaint();
                    } else {
                        flags[i] = false;
                        println!("Iter {i} done: {:?}", iter);
                    }
                }
            }

            // Don't care for this message to be received
            let _ = tx.send(Message::Done);
            ctx.request_repaint();
        });
    }
}
