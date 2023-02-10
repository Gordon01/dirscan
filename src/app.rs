use std::collections::HashMap;
use std::sync::{
    mpsc::{self, Receiver},
    Arc, Mutex,
};
use std::thread;
use std::time::Duration;

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
    Intermediate(Vec<FinalEntry>),
    Done,
}

impl ScanState {
    fn is_scanning(&self) -> bool {
        matches!(self, ScanState::Scanning(_))
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
                // TODO: Show as needed
                if ui.button("Stop").clicked() {
                    *state = ScanState::Idle;
                }
                if !state.is_scanning() {
                    if ui.button("Calculate").clicked() {
                        scan_directory(ctx, state, path, cache.clone());
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
fn scan_directory(
    ctx: &egui::Context,
    state: &mut ScanState,
    path: &str,
    cache: Arc<Mutex<Cache>>,
) {
    let sub_paths = if let Ok(p) = std::fs::read_dir(path) {
        p
    } else {
        *state = ScanState::Error(format!("On open root dir: {path}"));
        return;
    };

    /* We're sending intermediate results every 100 ms to not overwhelm UI with frequent updates here.
       First thread is sending events to UI, second — to the first thread (and `Done` message).
       Ideally, we want to give each directory iterator an adjustable amount of time to work (100 ms default),
       not number of entries (1024 now) because time may be different for different types of filesystems.
       TODO: simplify code to one thread and adjustable amount of time to work
       TODO: Always have different iterators for each subdir of subdir (currently we have iters only for root dir)
    */
    let (tx_total, rx_total) = mpsc::channel();
    let (tx_inter, rx_inter) = mpsc::channel();
    *state = ScanState::Scanning((rx_total, HashMap::new()));

    let ctx = ctx.clone();
    let tx = tx_total.clone();
    thread::spawn(move || loop {
        // TODO: Stop thread on `rx_inter` or `tx_total` hung-up
        let intermediate: Vec<_> = rx_inter.try_iter().collect();
        if !intermediate.is_empty() {
            tx.send(Message::Intermediate(intermediate)).unwrap();
            ctx.request_repaint();
        }

        thread::sleep(Duration::from_millis(100));
    });

    thread::spawn(move || {
        let mut cache = cache.lock().unwrap();
        cache.insert("Test".to_string(), 2);

        let mut sub_iters: Vec<_> = sub_paths
            .filter_map(|p| p.ok()) // Ignore subdirectory if it fails on read
            .map(|p| {
                (
                    WalkDir::new(p.path())
                        .into_iter()
                        .filter_map(|e| e.ok()) // Ignore any entry on the way
                        .filter(|e| e.file_type().is_file()),
                    p.file_name().to_str().unwrap().to_owned(),
                )
            })
            .collect();

        // Calculating the size of the 1024 items on each subdirectory sequentally
        let mut flags = vec![true; sub_iters.len()];
        while !flags.iter().all(|&f| !f) {
            for (i, (iter, name)) in sub_iters.iter_mut().enumerate() {
                // Ugly, needs refactoring
                if !flags[i] {
                    continue;
                }

                // Get next 1024 (or less) filesystem entries
                let two: Vec<_> = iter.take(1024).collect();

                if !two.is_empty() {
                    let size = two.iter().map(|e| e.metadata().unwrap().len()).sum::<u64>();
                    // Fighting the borrow checker here
                    if tx_inter.send((name.to_owned(), size)).is_err() {
                        println!("Nowhere to send");
                        return;
                    }
                    //ctx.request_repaint();
                } else {
                    flags[i] = false;
                    println!("Iter {i} done: {:?}", iter);
                }
            }
        }

        // Don't care for this message to be received
        let _ = tx_total.send(Message::Done);
        //ctx.request_repaint();
    });
}
