use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::app::{Cache, Message, ScanState};
use walkdir::WalkDir;

pub fn scan_directory(
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
