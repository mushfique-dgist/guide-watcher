// src-tauri/src/watcher.rs
use crate::config;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

/// Start the file watcher on a background Tokio task.
/// Emits "new-files" events to the frontend when lecture files are detected.
pub fn start(app: AppHandle) {
    let (tx, mut rx) = mpsc::channel::<String>(100);

    // Grab the Tokio runtime handle BEFORE spawning the std thread.
    // The notify crate's watcher is synchronous, so it must live on a std::thread.
    // But the callbacks need to spawn async tasks, so we capture the handle here.
    let rt_handle = tauri::async_runtime::handle();
    let tx_clone = tx.clone();

    std::thread::spawn(move || {
        let rt = rt_handle;
        let tx_inner = tx_clone;

        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    let tx = tx_inner.clone();
                    rt.spawn(async move {
                        handle_event(event, &tx).await;
                    });
                }
            },
            notify::Config::default(),
        )
        .expect("Failed to create file watcher");

        watcher
            .watch(Path::new(config::WATCH_DIR), RecursiveMode::Recursive)
            .expect("Failed to watch directory");

        // Keep the watcher alive (loop prevents drop)
        loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    });

    // Spawn the batching + emission task
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let cooldown: Arc<Mutex<HashMap<String, Instant>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let mut batch: Vec<String> = Vec::new();
        let mut batch_timer: Option<Instant> = None;

        loop {
            // Try to receive with a short timeout for batching
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Some(filepath)) => {
                    // Cooldown check
                    let normalized = filepath.replace('\\', "/").to_lowercase();
                    {
                        let mut cd = cooldown.lock().unwrap();
                        if let Some(last) = cd.get(&normalized) {
                            if last.elapsed().as_secs() < config::COOLDOWN_SECS {
                                continue;
                            }
                        }
                        cd.insert(normalized.clone(), Instant::now());
                    }

                    if !batch.iter().any(|b| {
                        b.replace('\\', "/").to_lowercase() == normalized
                    }) {
                        batch.push(filepath);
                    }

                    if batch_timer.is_none() {
                        batch_timer = Some(Instant::now());
                    }
                }
                Ok(None) => break, // channel closed
                Err(_) => {} // timeout, check batch
            }

            // Emit batch if window elapsed
            if let Some(timer) = batch_timer {
                if timer.elapsed().as_secs() >= config::BATCH_WINDOW_SECS && !batch.is_empty()
                {
                    let files = std::mem::take(&mut batch);
                    batch_timer = None;

                    // Show window and emit event
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = tauri::WebviewWindow::show(&window);
                        let _ = tauri::WebviewWindow::set_focus(&window);
                    }
                    let _ = app_handle.emit("new-files", &files);
                }
            }
        }
    });
}

async fn handle_event(event: Event, tx: &mpsc::Sender<String>) {
    match event.kind {
        EventKind::Create(_) | EventKind::Modify(notify::event::ModifyKind::Name(_)) => {
            for path in &event.paths {
                let path_str = path.to_string_lossy().to_string();

                if path.is_dir() {
                    // Directory created/moved — wait then scan
                    let tx = tx.clone();
                    let dir = path_str.clone();
                    tokio::spawn(async move {
                        sleep(Duration::from_millis(config::DIR_SCAN_DELAY_MS)).await;
                        scan_directory(&dir, &tx).await;
                    });
                } else {
                    process_file(&path_str, tx).await;
                }
            }
        }
        _ => {}
    }
}

async fn scan_directory(dir: &str, tx: &mpsc::Sender<String>) {
    let walker = walkdir::WalkDir::new(dir).into_iter().filter_map(|e| e.ok());
    for entry in walker {
        if entry.file_type().is_file() {
            process_file(&entry.path().to_string_lossy(), tx).await;
        }
    }
}

async fn process_file(filepath: &str, tx: &mpsc::Sender<String>) {
    if !config::is_watched_extension(filepath) {
        return;
    }
    if config::should_skip(filepath) {
        return;
    }

    // Wait for file copy to stabilize
    if !wait_for_stable(filepath).await {
        return;
    }

    let _ = tx.send(filepath.to_string()).await;
}

/// Poll file size until stable or timeout.
async fn wait_for_stable(filepath: &str) -> bool {
    let path = Path::new(filepath);
    let mut prev_size: Option<u64> = None;
    let max_polls = (config::FILE_STABILITY_TIMEOUT_SECS * 1000)
        / config::FILE_STABILITY_POLL_MS;

    for _ in 0..max_polls {
        match std::fs::metadata(path) {
            Ok(meta) => {
                let size = meta.len();
                if size > 0 {
                    if let Some(prev) = prev_size {
                        if prev == size {
                            return true; // stable
                        }
                    }
                    prev_size = Some(size);
                }
            }
            Err(_) => return false, // file gone
        }
        sleep(Duration::from_millis(config::FILE_STABILITY_POLL_MS)).await;
    }

    // Timeout — proceed if file still exists
    path.exists()
}
