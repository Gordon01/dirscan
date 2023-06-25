use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::app::{Cache, Message, ScanState};
use dirwiz::DirWiz;

pub fn scan_directory(
    ctx: &egui::Context,
    state: &mut ScanState,
    path: &str,
    cache: Arc<Mutex<Cache>>,
) {
    let (tx_total, rx_total) = mpsc::channel();
    *state = ScanState::Scanning((rx_total, HashMap::new()));

    // Not used right now
    let mut cache = cache.lock().unwrap();
    cache.insert("Test".to_string(), 2);

    let ctx = ctx.clone();
    let dirwiz = DirWiz::new(path).into_iter();
    thread::spawn(move || {
        let mut start = Instant::now();
        let mut intermediate = Vec::new();
        for (p, s) in dirwiz {
            intermediate.push((p.to_str().unwrap().to_owned(), s));
            if start.elapsed() > Duration::from_millis(100) {
                tx_total
                    .send(Message::Intermediate(intermediate.clone()))
                    .unwrap();
                ctx.request_repaint();
                intermediate.clear();
                start = Instant::now();
            }
        }

        let _ = tx_total.send(Message::Done);
        ctx.request_repaint();
    });
}
