mod claude;
mod config;
mod watcher;

use config::ConfigResponse;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};

#[tauri::command]
async fn approve_files(
    app: tauri::AppHandle,
    file_paths: Vec<String>,
    model: String,
    effort: String,
) -> Vec<String> {
    let mut job_ids = Vec::new();
    for fp in file_paths {
        let jid = claude::start_job(app.clone(), fp, model.clone(), effort.clone()).await;
        job_ids.push(jid);
    }
    job_ids
}

#[tauri::command]
async fn start_job(
    app: tauri::AppHandle,
    file_path: String,
    model: String,
    effort: String,
) -> String {
    claude::start_job(app, file_path, model, effort).await
}

#[tauri::command]
fn get_config() -> ConfigResponse {
    ConfigResponse::new()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Startup validation
    if !std::path::Path::new(config::TEMPLATE_FILE).exists() {
        eprintln!("Template file not found: {}", config::TEMPLATE_FILE);
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            approve_files,
            start_job,
            get_config,
        ])
        .setup(|app| {
            // Startup validation with dialogs
            let handle = app.handle().clone();
            validate_startup(&handle);

            // Build system tray
            let show = MenuItem::with_id(app, "show", "Show Window", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;

            TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app: &tauri::AppHandle, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray: &tauri::tray::TrayIcon, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            // Start file watcher
            watcher::start(handle);

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn validate_startup(app: &tauri::AppHandle) {
    use tauri_plugin_dialog::DialogExt;

    let checks = [
        (config::TEMPLATE_FILE, "Template file not found"),
        (config::GIT_BASH, "Git Bash not found"),
        (config::WATCH_DIR, "Watch directory not found"),
    ];

    for (path, label) in checks {
        if !std::path::Path::new(path).exists() {
            app.dialog()
                .message(format!("{}: {}\n\nThe app cannot start without this.", label, path))
                .title("Guide Watcher — Startup Error")
                .blocking_show();
            std::process::exit(1);
        }
    }
}
