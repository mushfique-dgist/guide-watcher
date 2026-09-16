mod artifact_bundle;
mod attention;
mod batch;
mod circuit_capture;
mod claude;
mod codex;
mod completion;
mod config;
mod context_tree;
mod course_plan;
mod headless;
mod history;
mod job_events;
mod png_validation;
mod process_registry;
mod settings;
mod setup;
mod provider_auth;
mod provider_executable;
mod publication;
mod source_context;
mod supplementary;
mod visual_assets;
mod watcher;

use config::ConfigResponse;
use serde::{Deserialize, Serialize};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};

pub use batch::{BatchJobResult, BatchJobStatus};
pub use circuit_capture::{prepare_circuit_lab_week, CircuitPreparationOutcome};
pub use headless::{
    cancel_headless_jobs, run_headless, run_headless_batch, run_headless_check, HeadlessCommand,
    HeadlessError, HeadlessOutcome,
};

/// `guide-watcher-cli supplementary plan|collect|status <course_dir>`: one-time collection
/// of free lecture transcripts for a course, kept in `<course>/_supplementary/`.
pub async fn run_supplementary(
    course_dir: &std::path::Path,
    action: &str,
) -> Result<serde_json::Value, String> {
    let action = supplementary::SupplementaryAction::parse(action)
        .ok_or_else(|| format!("unknown supplementary action: {action}"))?;
    supplementary::run(course_dir, action).await
}

pub async fn inspect_visuals(
    source: &std::path::Path,
    write_skeleton: bool,
) -> Result<serde_json::Value, String> {
    let cancellation = process_registry::cancellation_token();
    let result = inspect_visuals_with_cancellation(source, write_skeleton, cancellation).await;
    process_registry::finish(cancellation);
    result
}

async fn inspect_visuals_with_cancellation(
    source: &std::path::Path,
    write_skeleton: bool,
    cancellation: process_registry::CancellationToken,
) -> Result<serde_json::Value, String> {
    let source = source.to_path_buf();
    let joined = tokio::task::spawn_blocking(move || {
        if process_registry::is_cancelled(cancellation) {
            return Err("visual inspection was cancelled".to_string());
        }
        let source = source.canonicalize().map_err(|error| {
            format!(
                "could not resolve visual-inspection source {}: {error}",
                source.display()
            )
        })?;
        if process_registry::is_cancelled(cancellation) {
            return Err("visual inspection was cancelled".to_string());
        }
        let mut plans = course_plan::plan_guides(std::slice::from_ref(&source), "auto")?;
        if process_registry::is_cancelled(cancellation) {
            return Err("visual inspection was cancelled".to_string());
        }
        if plans.len() != 1 {
            return Err("visual inspection requires exactly one planned guide".to_string());
        }
        let plan = codex::canonicalize_plan(plans.remove(0))?;
        let inspection =
            source_context::inspect_visual_authoring_with_cancellation(&plan, cancellation)?;
        if process_registry::is_cancelled(cancellation) {
            return Err("visual inspection was cancelled".to_string());
        }
        if write_skeleton {
            let path = std::path::Path::new(&inspection.expected_packet_path);
            write_visual_skeleton(path, &inspection.skeleton)?;
        }
        if process_registry::is_cancelled(cancellation) {
            return Err("visual inspection was cancelled".to_string());
        }
        serde_json::to_value(inspection)
            .map_err(|error| format!("could not serialize visual inspection: {error}"))
    })
    .await;
    match joined {
        Ok(result) => result,
        Err(error) => Err(format!("visual inspection worker failed: {error}")),
    }
}

fn write_visual_skeleton(
    path: &std::path::Path,
    skeleton: &serde_json::Value,
) -> Result<(), String> {
    use std::io::Write;
    let bytes = serde_json::to_vec_pretty(skeleton)
        .map_err(|error| format!("could not serialize visual skeleton: {error}"))?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            format!(
                "refusing to overwrite visual packet {}: {error}",
                path.display()
            )
        })?;
    if let Err(error) = file
        .write_all(&bytes)
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.sync_all())
    {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(format!(
            "could not write visual skeleton {}: {error}",
            path.display()
        ));
    }
    Ok(())
}

pub async fn validate_visuals(source: &std::path::Path) -> Result<serde_json::Value, String> {
    let cancellation = process_registry::cancellation_token();
    let source = source.to_path_buf();
    let joined = tokio::task::spawn_blocking(move || {
        if process_registry::is_cancelled(cancellation) {
            return Err("visual validation was cancelled".to_string());
        }
        let source = source.canonicalize().map_err(|error| {
            format!(
                "could not resolve visual-validation source {}: {error}",
                source.display()
            )
        })?;
        if process_registry::is_cancelled(cancellation) {
            return Err("visual validation was cancelled".to_string());
        }
        let mut plans = course_plan::plan_guides(std::slice::from_ref(&source), "auto")?;
        if process_registry::is_cancelled(cancellation) {
            return Err("visual validation was cancelled".to_string());
        }
        if plans.len() != 1 {
            return Err("visual validation requires exactly one planned guide".to_string());
        }
        let plan = codex::canonicalize_plan(plans.remove(0))?;
        let material = source_context::preflight_source_material_with_renderer(
            &plan,
            &source_context::NativeSourceRenderer,
            cancellation,
        )?;
        source_context::recheck_source_material(&material)?;
        Ok(serde_json::json!({
        "schema_version": 1,
        "packet_path": material.visual_material.packet_path,
        "packet_sha256": material.visual_material.packet_sha256,
        "decision": material.visual_material.contract.decision,
        "assets": material.visual_material.compiled_assets.iter().map(|asset| serde_json::json!({
            "id": asset.catalog.id,
            "sha256": asset.catalog.sha256,
            "spec_sha256": asset.catalog.spec_sha256,
            "width_px": asset.catalog.width_px,
            "height_px": asset.catalog.height_px,
            "kind": asset.catalog.kind,
            "need_ids": asset.catalog.need_ids,
        })).collect::<Vec<_>>(),
        }))
    })
    .await;
    process_registry::finish(cancellation);
    match joined {
        Ok(result) => result,
        Err(error) => Err(format!("visual validation worker failed: {error}")),
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct PhaseSelection {
    provider: String,
    model: String,
    effort: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerateOptions {
    prep: PhaseSelection,
    collection_fallbacks: Vec<PhaseSelection>,
    writer: PhaseSelection,
    fallbacks: Vec<PhaseSelection>,
    course_profile: String,
    require_slide_coverage: bool,
    require_visuals: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResumePrepOptions {
    prep: PhaseSelection,
    collection_fallbacks: Vec<PhaseSelection>,
    writer: PhaseSelection,
    fallbacks: Vec<PhaseSelection>,
}

#[tauri::command]
async fn approve_files(
    app: tauri::AppHandle,
    file_paths: Vec<String>,
    options: GenerateOptions,
) -> Result<Vec<course_plan::PlannedJobDto>, String> {
    dispatch_planned_batch(app, file_paths, options).await
}

#[tauri::command]
async fn start_job(
    app: tauri::AppHandle,
    file_path: String,
    options: GenerateOptions,
) -> Result<course_plan::PlannedJobDto, String> {
    let mut jobs = dispatch_planned_batch(app, vec![file_path], options).await?;
    Ok(jobs.remove(0))
}

#[tauri::command]
fn get_config() -> ConfigResponse {
    ConfigResponse::new()
}

#[tauri::command]
async fn get_provider_auth_statuses() -> Vec<provider_auth::ProviderAuthStatus> {
    provider_auth::statuses().await
}

#[tauri::command]
fn start_provider_login(
    app: tauri::AppHandle,
    state: tauri::State<'_, provider_auth::ProviderAuthState>,
    provider: String,
) -> Result<provider_auth::ProviderAuthSession, String> {
    provider_auth::start_login(app, state.inner().clone(), provider)
}

#[tauri::command]
fn cancel_provider_login(
    state: tauri::State<'_, provider_auth::ProviderAuthState>,
    provider: String,
    session_id: String,
) -> Result<(), String> {
    state.cancel(&provider, &session_id)
}

/// Open a folder picker and return supported lecture/material files recursively.
#[tauri::command]
async fn scan_folder(app: tauri::AppHandle) -> Result<Option<Vec<String>>, String> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog().file().pick_folder(move |folder| {
        let _ = tx.send(folder);
    });

    let Some(folder_path) = rx.recv().map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let path = folder_path.into_path().map_err(|e| e.to_string())?;
    configured_scannable_files_in(&path)
}

/// Return supported files from the configured semester root without opening a
/// native dialog. This is the normal batch entry point for the fixed DGIST
/// workspace; `scan_folder` remains available for ad hoc locations.
#[tauri::command]
fn scan_watch_folder() -> Result<Option<Vec<String>>, String> {
    configured_scannable_files_in(std::path::Path::new(&config::watch_dir()))
}

fn configured_scannable_files_in(path: &std::path::Path) -> Result<Option<Vec<String>>, String> {
    if !path.is_dir() {
        return Err(format!(
            "The configured semester folder is unavailable: {}",
            path.display()
        ));
    }
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(path).min_depth(1).max_depth(3) {
        let entry = entry.map_err(|e| format!("Could not scan the course folder: {e}"))?;
        if entry.file_type().is_file() {
            let text = entry.path().to_string_lossy();
            if config::is_watched_extension(&text) && !config::should_skip(&text) {
                files.push(entry.path().to_owned());
            }
        }
    }
    files.sort();
    if files.is_empty() {
        return Ok(None);
    }
    let courses = course_plan::configured_courses()?;
    let wave = course_plan::next_pending_scan_wave_with_courses(&files, &courses)?;
    let wave = wave
        .into_iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect::<Vec<_>>();
    Ok((!wave.is_empty()).then_some(wave))
}

#[cfg(test)]
fn supported_files_in(path: &std::path::Path) -> Option<Vec<String>> {
    let mut files: Vec<String> = walkdir::WalkDir::new(path)
        .min_depth(1)
        .max_depth(3)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let p = e.path().to_owned();
            let s = p.to_string_lossy().into_owned();
            if config::is_watched_extension(&s) && !config::should_skip(&s) {
                Some(s)
            } else {
                None
            }
        })
        .collect();

    files.sort();
    if files.is_empty() {
        None
    } else {
        Some(files)
    }
}

/// Open a multi-select file picker and return supported source paths directly.
#[tauri::command]
async fn pick_files(app: tauri::AppHandle) -> Option<Vec<String>> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .add_filter("Lecture and course files", &["pdf", "html", "pptx", "docx"])
        .pick_files(move |files| {
            let _ = tx.send(files);
        });

    let chosen = rx.recv().ok()??;
    let paths: Vec<String> = chosen
        .into_iter()
        .filter_map(|f| f.into_path().ok())
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();

    if paths.is_empty() {
        None
    } else {
        Some(paths)
    }
}

/// Open a single-file picker and accept only an exact `.prep.md` packet.
#[tauri::command]
async fn pick_prep_file(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .add_filter("Guide Watcher prep packets", &["md"])
        .pick_file(move |file| {
            let _ = tx.send(file);
        });

    let chosen = rx
        .recv()
        .map_err(|e| format!("The file picker did not return a result: {e}"))?;
    let Some(chosen) = chosen else {
        return Ok(None);
    };
    let path = chosen.into_path().map_err(|e| e.to_string())?;
    if !job_events::is_prep_file(&path) {
        return Err("Choose a saved context file ending in .prep.md. Ordinary guide Markdown files cannot be resumed.".into());
    }
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// Start a resume job for an incomplete guide.
#[tauri::command]
async fn resume_guide(
    app: tauri::AppHandle,
    prep_path: String,
    options: ResumePrepOptions,
) -> Result<String, String> {
    start_provider_resume_job(app, prep_path, options).await
}

#[tauri::command]
fn setup_state() -> setup::SetupState {
    setup::state()
}

#[tauri::command]
fn setup_suggestions() -> (String, String) {
    (setup::suggested_study_folder(), setup::bundled_automation_folder())
}

#[tauri::command]
fn save_setup(
    watch_dir: String,
    automation_dir: String,
    subjects: Vec<setup::SubjectDraft>,
) -> Result<setup::SetupState, String> {
    setup::save(setup::draft_settings(watch_dir, automation_dir, subjects))
}

#[tauri::command]
fn cancel_all_jobs() {
    // The user asked for the stop, so the endings it causes are not news.
    attention::user_stopped_jobs();
    process_registry::cancel_all();
}

struct HistoryState(Result<std::sync::Arc<history::HistoryStore>, String>);
fn history_store(app: &tauri::AppHandle) -> Result<std::sync::Arc<history::HistoryStore>, String> {
    app.state::<HistoryState>().0.clone()
}
async fn dispatch_planned_batch(
    app: tauri::AppHandle,
    file_paths: Vec<String>,
    options: GenerateOptions,
) -> Result<Vec<course_plan::PlannedJobDto>, String> {
    dispatch_batch_with_parent(app, file_paths, options, None).await
}
async fn dispatch_batch_with_parent(
    app: tauri::AppHandle,
    file_paths: Vec<String>,
    options: GenerateOptions,
    parent: Option<String>,
) -> Result<Vec<course_plan::PlannedJobDto>, String> {
    let store = history_store(&app)?;
    let cancellation = process_registry::cancellation_token();
    let request = history::HistoryRecord::request(
        file_paths.clone(),
        serde_json::to_value(&options).map_err(|e| e.to_string())?,
        parent,
        store.session_id.clone(),
    );
    if let Err(error) = store.insert(&request, Some(cancellation)) {
        process_registry::finish(cancellation);
        return Err(error);
    }
    let result: Result<Vec<course_plan::PlannedJobDto>, String> = async {
        let config = generate_run_config(&options)?;
        let paths = file_paths
            .into_iter()
            .map(std::path::PathBuf::from)
            .collect::<Vec<_>>();
        let profile = options.course_profile.clone();
        let prepared = tokio::task::spawn_blocking(move || {
            batch::preflight_paths(&paths, &profile, cancellation)
        })
        .await
        .map_err(|e| format!("Source-check worker failed: {e}"))??;
        let jobs = prepared.planned_jobs()?;
        let records = jobs
            .iter()
            .map(|job| {
                let mut record = request.clone();
                record.job_id = job.job_id.clone();
                record.source_path = job.primary_source.clone();
                record.source_paths = job.source_paths.clone();
                record.filename = std::path::Path::new(&job.primary_source)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                record.folder = job.course_label.clone();
                record.output_path = Some(job.output_path.clone());
                record.prep_path = Some(
                    std::path::Path::new(&job.output_path)
                        .with_extension("prep.md")
                        .to_string_lossy()
                        .into_owned(),
                );
                record.status = "queued".into();
                record.event(
                    "info",
                    "Source checks passed. Waiting for context collection and course order.",
                );
                record
            })
            .collect::<Vec<_>>();
        store.replace_request(&request.job_id, &records, cancellation)?;
        let progress: job_events::SharedProgress = std::sync::Arc::new(history::DesktopProgress {
            app: app.clone(),
            store: store.clone(),
            token: cancellation,
        });
        let worker_store = store.clone();
        let ids = jobs.iter().map(|j| j.job_id.clone()).collect::<Vec<_>>();
        tauri::async_runtime::spawn(async move {
            batch::execute_batch(progress, prepared, config, cancellation).await;
            worker_store.release(&ids);
            process_registry::finish(cancellation);
        });
        Ok(jobs)
    }
    .await;
    if let Err(error) = &result {
        let saved = store.update(&request.job_id, |r| {
            r.terminal_rich(
                if process_registry::is_cancelled(cancellation) {
                    "cancelled"
                } else {
                    "failed"
                },
                history::humanize_error(error),
            )
        });
        store.release(std::slice::from_ref(&request.job_id));
        process_registry::finish(cancellation);
        saved?;
    }
    result
}
async fn start_provider_resume_job(
    app: tauri::AppHandle,
    prep_path: String,
    options: ResumePrepOptions,
) -> Result<String, String> {
    resume_with_parent(app, prep_path, options, None).await
}
async fn resume_with_parent(
    app: tauri::AppHandle,
    prep_path: String,
    options: ResumePrepOptions,
    parent: Option<String>,
) -> Result<String, String> {
    let store = history_store(&app)?;
    let cancellation = process_registry::cancellation_token();
    let mut record = history::HistoryRecord::request(
        vec![prep_path.clone()],
        serde_json::to_value(&options).map_err(|e| e.to_string())?,
        parent,
        store.session_id.clone(),
    );
    record.prep_path = Some(prep_path.clone());
    record.cancellation_scope = "run".into();
    if let Err(error) = store.insert(&record, Some(cancellation)) {
        process_registry::finish(cancellation);
        return Err(error);
    }
    let result: Result<String, String> = async {
        let config = resume_run_config(&options)?;
        let prepared = tokio::task::spawn_blocking(move || {
            batch::preflight_resume_path(std::path::Path::new(&prep_path), cancellation)
        })
        .await
        .map_err(|e| format!("Saved-context check failed: {e}"))??;
        let output = prepared.output_path().to_string_lossy().into_owned();
        store.update(&record.job_id, |r| {
            r.output_path = Some(output);
            r.event(
                "info",
                "Saved context and bound sources passed validation. Starting the writer.",
            );
        })?;
        let progress: job_events::SharedProgress = std::sync::Arc::new(history::DesktopProgress {
            app: app.clone(),
            store: store.clone(),
            token: cancellation,
        });
        let worker_store = store.clone();
        let id = record.job_id.clone();
        tauri::async_runtime::spawn(async move {
            batch::execute_resume(progress, id.clone(), prepared, config, cancellation).await;
            worker_store.release(&[id]);
            process_registry::finish(cancellation);
        });
        Ok(record.job_id.clone())
    }
    .await;
    if let Err(error) = &result {
        let saved = store.update(&record.job_id, |r| {
            r.terminal_rich(
                if process_registry::is_cancelled(cancellation) {
                    "cancelled"
                } else {
                    "failed"
                },
                history::humanize_error(error),
            )
        });
        store.release(std::slice::from_ref(&record.job_id));
        process_registry::finish(cancellation);
        saved?;
    }
    result
}

fn generate_run_config(options: &GenerateOptions) -> Result<codex::HybridRunConfig, String> {
    if !options.require_slide_coverage || !options.require_visuals {
        return Err(
            "The canonical workflow requires both exact source-unit coverage and explanatory visuals."
                .to_string(),
        );
    }
    build_run_config(
        &options.prep,
        &options.collection_fallbacks,
        &options.writer,
        &options.fallbacks,
    )
}

fn resume_run_config(options: &ResumePrepOptions) -> Result<codex::HybridRunConfig, String> {
    build_run_config(
        &options.prep,
        &options.collection_fallbacks,
        &options.writer,
        &options.fallbacks,
    )
}

fn build_run_config(
    prep: &PhaseSelection,
    collection_fallbacks: &[PhaseSelection],
    writer: &PhaseSelection,
    fallbacks: &[PhaseSelection],
) -> Result<codex::HybridRunConfig, String> {
    if prep.provider != "codex-chatgpt" {
        return Err("Phase 1 must use Codex through the ChatGPT login.".to_string());
    }
    validate_phase(prep, "Phase 1")?;
    if collection_fallbacks.len() != 2 {
        return Err(
            "Phase 1 requires exactly two ordered Codex fallbacks after the primary collector."
                .to_string(),
        );
    }
    let mut collection_models = std::collections::HashSet::from([prep.model.as_str()]);
    for (index, fallback) in collection_fallbacks.iter().enumerate() {
        validate_phase(fallback, &format!("Collection fallback {}", index + 1))?;
        if fallback.provider != "codex-chatgpt" {
            return Err(format!(
                "Collection fallback {} must use Codex through the ChatGPT login.",
                index + 1
            ));
        }
        if !collection_models.insert(fallback.model.as_str()) {
            return Err(
                "The primary collector and collection fallbacks must use distinct models."
                    .to_string(),
            );
        }
    }
    if writer.provider != "claude-code" {
        return Err(
            "The cumulative guide workflow requires Claude for Phase 2; unsupported provider combinations are not silently substituted."
                .to_string(),
        );
    }
    validate_phase(writer, "Phase 2")?;
    if fallbacks.is_empty() || fallbacks.len() > 2 {
        return Err(
            "A final fallback using Codex is required, with at most one Claude writer fallback before it."
                .to_string(),
        );
    }
    for (index, fallback) in fallbacks.iter().enumerate() {
        validate_phase(fallback, &format!("Fallback {}", index + 1))?;
    }
    let codex_fallback = fallbacks.last().expect("fallbacks checked as non-empty");
    if codex_fallback.provider != "codex-chatgpt" {
        return Err("The final fallback must use Codex through the ChatGPT login.".to_string());
    }
    let claude_fallback = if fallbacks.len() == 2 {
        let candidate = &fallbacks[0];
        if candidate.provider != "claude-code" {
            return Err("The first fallback must be a Claude writer model.".to_string());
        }
        if candidate.model == writer.model {
            return Err(
                "The Claude fallback must use a different model than the primary writer."
                    .to_string(),
            );
        }
        Some(candidate)
    } else {
        None
    };

    Ok(codex::HybridRunConfig {
        prep_model: prep.model.clone(),
        prep_effort: prep.effort.clone(),
        collection_fallbacks: collection_fallbacks
            .iter()
            .map(|fallback| codex::CollectionModelSpec {
                model: fallback.model.clone(),
                effort: fallback.effort.clone(),
            })
            .collect(),
        writer_model: writer.model.clone(),
        writer_effort: writer.effort.clone(),
        writer_fallback_model: claude_fallback
            .map(|fallback| fallback.model.clone())
            .unwrap_or_default(),
        writer_fallback_effort: claude_fallback
            .map(|fallback| fallback.effort.clone())
            .unwrap_or_default(),
        codex_fallback_model: codex_fallback.model.clone(),
        codex_fallback_effort: codex_fallback.effort.clone(),
    })
}

fn validate_phase(phase: &PhaseSelection, label: &str) -> Result<(), String> {
    let (models, efforts): (&[&str], &[&str]) = match phase.provider.as_str() {
        "codex-chatgpt" => (config::CODEX_MODELS, config::CODEX_EFFORTS),
        "claude-code" => (config::CLAUDE_MODELS, config::CLAUDE_EFFORTS),
        _ => {
            return Err(format!(
                "{label} uses an unsupported provider: {}",
                phase.provider
            ))
        }
    };
    if !models.contains(&phase.model.as_str()) {
        return Err(format!(
            "{label} uses an unsupported model: {}",
            phase.model
        ));
    }
    if !efforts.contains(&phase.effort.as_str()) {
        return Err(format!(
            "{label} uses an unsupported effort: {}",
            phase.effort
        ));
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Startup validation
    if !std::path::Path::new(&config::template_file()).exists() {
        eprintln!("Template file not found: {}", &config::template_file());
    }

    tauri::Builder::default()
        // Must be registered first: plugins run in the order they are added. A second launch -
        // from the Start menu, a shortcut, or a build folder - hands its arguments to the
        // running app and exits, so guide jobs are never split across two processes.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                // The running app may be hidden in the tray or minimized.
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED,
                )
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            approve_files,
            start_job,
            get_config,
            get_provider_auth_statuses,
            start_provider_login,
            cancel_provider_login,
            scan_folder,
            scan_watch_folder,
            pick_files,
            pick_prep_file,
            resume_guide,
            cancel_all_jobs,
            setup_state,
            setup_suggestions,
            save_setup,
            list_job_history,
            archive_history_job,
            clear_recent_history,
            restore_history_job,
            delete_history_job,
            retry_history_job,
            resume_history_job,
            cancel_history_job,
            open_history_output,
        ])
        .setup(|app| {
            app.manage(provider_auth::ProviderAuthState::default());
            let store = app
                .path()
                .app_local_data_dir()
                .map_err(|e| e.to_string())
                .and_then(|root| history::HistoryStore::open(&root.join("history")))
                .map(std::sync::Arc::new);
            app.manage(HistoryState(store.clone()));
            if let Ok(store) = store {
                let history_app = app.handle().clone();
                tauri::async_runtime::spawn_blocking(move || {
                    if let Err(error) =
                        store.import_existing(std::path::Path::new(&config::watch_dir()))
                    {
                        use tauri::Emitter;
                        let _ = history_app.emit("history-error", error);
                    }
                });
            }
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
                .on_menu_event(
                    move |app: &tauri::AppHandle, event| match event.id.as_ref() {
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.unminimize();
                                let _ = window.set_focus();
                            }
                        }
                        "quit" => {
                            process_registry::cancel_all();
                            app.exit(0);
                        }
                        _ => {}
                    },
                )
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
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    // Closing stops the running jobs on purpose, so their endings must not
                    // arrive as notifications.
                    attention::user_stopped_jobs();
                    process_registry::cancel_all();
                    api.prevent_close();
                    let _ = window.hide();
                }
                // Looking at the window is what acknowledges everything it was holding.
                WindowEvent::Focused(true) => attention::clear(window.app_handle()),
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn validate_startup(app: &tauri::AppHandle) {
    use tauri_plugin_dialog::DialogExt;

    for (path, label) in required_startup_paths() {
        if !std::path::Path::new(&path).exists() {
            app.dialog()
                .message(format!(
                    "{}: {}\n\nThe app cannot start without this.",
                    label, path
                ))
                .title("Guide Watcher — Startup Error")
                .blocking_show();
            std::process::exit(1);
        }
    }
}

fn required_startup_paths() -> [(String, &'static str); 2] {
    [
        (config::template_file(), "Template file not found"),
        (config::watch_dir(), "Watch directory not found"),
    ]
}

#[tauri::command]
async fn list_job_history(app: tauri::AppHandle) -> Result<Vec<history::HistoryRecord>, String> {
    let store = history_store(&app)?;
    tokio::task::spawn_blocking(move || store.list())
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn archive_history_job(app: tauri::AppHandle, job_id: String) -> Result<(), String> {
    let store = history_store(&app)?;
    tokio::task::spawn_blocking(move || store.archive(&job_id, true))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn restore_history_job(app: tauri::AppHandle, job_id: String) -> Result<(), String> {
    let store = history_store(&app)?;
    tokio::task::spawn_blocking(move || store.archive(&job_id, false))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn clear_recent_history(app: tauri::AppHandle) -> Result<usize, String> {
    let store = history_store(&app)?;
    tokio::task::spawn_blocking(move || store.clear_recents())
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn delete_history_job(app: tauri::AppHandle, job_id: String) -> Result<(), String> {
    let store = history_store(&app)?;
    tokio::task::spawn_blocking(move || store.delete(&job_id))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn retry_history_job(
    app: tauri::AppHandle,
    job_id: String,
) -> Result<serde_json::Value, String> {
    let store = history_store(&app)?;
    store.claim(&job_id)?;
    let result=async{let record=store.list()?.into_iter().find(|r|r.job_id==job_id).ok_or("This attempt is missing from history.")?;
if !record.can_retry{return Err("Retry is unavailable. Check whether a guide or saved context already exists, or restore the required sources.".into());}let options:GenerateOptions=serde_json::from_value(record.options).map_err(|e|format!("Saved generation settings are invalid: {e}"))?;let jobs=dispatch_batch_with_parent(app,record.source_paths,options,Some(job_id.clone())).await?;Ok(serde_json::json!({"jobId":jobs.first().map(|j|&j.job_id)}))}.await;
    store.unclaim(&job_id);
    result
}
#[tauri::command]
async fn resume_history_job(
    app: tauri::AppHandle,
    job_id: String,
    options: Option<ResumePrepOptions>,
) -> Result<serde_json::Value, String> {
    let store = history_store(&app)?;
    store.claim(&job_id)?;
    let result = async {
        let record = store
            .list()?
            .into_iter()
            .find(|r| r.job_id == job_id)
            .ok_or("This attempt is missing from history.")?;
        if !record.can_resume {
            return Err(
                "Resume is unavailable because no valid saved context is available.".into(),
            );
        }
        // The caller passes the user's current generation preferences (so a resume can
        // switch writer model after a quota or login failure); the compiled defaults are
        // only the fallback for callers that send none.
        let options = options.unwrap_or_else(|| ResumePrepOptions {
            prep: PhaseSelection {
                provider: config::DEFAULT_PROVIDER.into(),
                model: config::CODEX_PREP_PHASE_MODEL.into(),
                effort: config::CODEX_PREP_PHASE_EFFORT.into(),
            },
            collection_fallbacks: config::CODEX_COLLECTION_FALLBACKS
                .iter()
                .map(|(model, effort)| PhaseSelection {
                    provider: config::DEFAULT_PROVIDER.into(),
                    model: (*model).into(),
                    effort: (*effort).into(),
                })
                .collect(),
            writer: PhaseSelection {
                provider: config::DEFAULT_WRITER_PROVIDER.into(),
                model: config::DEFAULT_WRITER_MODEL.into(),
                effort: config::DEFAULT_WRITER_EFFORT.into(),
            },
            fallbacks: vec![PhaseSelection {
                provider: config::DEFAULT_PROVIDER.into(),
                model: config::CODEX_FINAL_FALLBACK_MODEL.into(),
                effort: config::CODEX_FINAL_FALLBACK_EFFORT.into(),
            }],
        });
        let id = resume_with_parent(
            app,
            record.prep_path.ok_or("Saved context path is missing.")?,
            options,
            Some(job_id.clone()),
        )
        .await?;
        Ok(serde_json::json!({"jobId":id}))
    }
    .await;
    store.unclaim(&job_id);
    result
}
#[tauri::command]
async fn cancel_history_job(app: tauri::AppHandle, job_id: String) -> Result<(), String> {
    let store = history_store(&app)?;
    tokio::task::spawn_blocking(move || store.cancel(&job_id))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn open_history_output(app: tauri::AppHandle, job_id: String) -> Result<(), String> {
    let store = history_store(&app)?;
    let path=tokio::task::spawn_blocking(move||{let record=store.get(&job_id)?;let path=record.output_path.ok_or("This attempt has no published guide.")?;
if !completion::is_usable_guide(std::path::Path::new(&path)){return Err("The guide or its supporting files are missing or changed. Opening it as verified is unavailable.".to_string());}Ok(path)}).await.map_err(|e|e.to_string())??;
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phase(provider: &str, model: &str, effort: &str) -> PhaseSelection {
        PhaseSelection {
            provider: provider.to_string(),
            model: model.to_string(),
            effort: effort.to_string(),
        }
    }

    fn default_generate_options() -> GenerateOptions {
        GenerateOptions {
            prep: phase("codex-chatgpt", "gpt-5.6-luna", "medium"),
            collection_fallbacks: vec![
                phase("codex-chatgpt", "gpt-5.6-terra", "medium"),
                phase("codex-chatgpt", "gpt-5.6-sol", "medium"),
            ],
            writer: phase("claude-code", "claude-opus-4-8", "high"),
            fallbacks: vec![phase("codex-chatgpt", "gpt-5.6-sol", "medium")],
            course_profile: "computer-networks".to_string(),
            require_slide_coverage: true,
            require_visuals: true,
        }
    }

    #[test]
    fn startup_only_requires_provider_independent_files() {
        let checks = required_startup_paths();
        assert!(checks
            .iter()
            .any(|(path, _)| path == &config::template_file()));
        assert!(checks.iter().any(|(path, _)| path == &config::watch_dir()));
    }

    #[test]
    fn configured_scan_filters_supported_sources_and_skipped_artifacts() {
        let root =
            std::env::temp_dir().join(format!("guide-watcher-scan-{}", uuid::Uuid::new_v4()));
        let nested = root.join("Course");
        let skipped = root.join(".codex_tmp");
        std::fs::create_dir_all(&nested).expect("create course directory");
        std::fs::create_dir_all(&skipped).expect("create skipped directory");
        std::fs::write(nested.join("Lecture.PDF"), b"pdf").expect("write source");
        std::fs::write(nested.join("Worksheet.docx"), b"docx").expect("write source");
        std::fs::write(nested.join("notes.txt"), b"ignore").expect("write ignored file");
        std::fs::write(skipped.join("Hidden.pdf"), b"ignore").expect("write skipped file");

        let files = supported_files_in(&root).expect("supported files");
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|path| path.ends_with("Lecture.PDF")));
        assert!(files.iter().any(|path| path.ends_with("Worksheet.docx")));

        std::fs::remove_dir_all(root).expect("clean test directory");
    }

    #[test]
    fn default_phase_payload_builds_the_exact_hybrid_chain() {
        let config = generate_run_config(&default_generate_options()).expect("valid payload");
        assert_eq!(config.prep_model, "gpt-5.6-luna");
        assert_eq!(config.prep_effort, "medium");
        assert_eq!(
            config.collection_fallbacks,
            [
                codex::CollectionModelSpec {
                    model: "gpt-5.6-terra".to_string(),
                    effort: "medium".to_string(),
                },
                codex::CollectionModelSpec {
                    model: "gpt-5.6-sol".to_string(),
                    effort: "medium".to_string(),
                },
            ]
        );
        assert_eq!(config.writer_model, "claude-opus-4-8");
        assert_eq!(config.writer_effort, "high");
        assert!(config.writer_fallback_model.is_empty());
        assert_eq!(config.codex_fallback_model, "gpt-5.6-sol");
        assert_eq!(config.codex_fallback_effort, "medium");
    }

    #[test]
    fn hybrid_payload_rejects_provider_reordering_and_missing_final_codex() {
        let mut reordered = default_generate_options();
        reordered.prep.provider = "claude-code".to_string();
        let error = generate_run_config(&reordered).unwrap_err();
        assert!(
            error.contains("Phase 1 must use Codex"),
            "unexpected error: {error}"
        );

        let mut no_codex = default_generate_options();
        no_codex.fallbacks.pop();
        let error = generate_run_config(&no_codex).unwrap_err();
        assert!(
            error.contains("final fallback"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn hybrid_payload_honors_allowed_model_and_effort_selections() {
        let mut prep = default_generate_options();
        prep.prep.model = "gpt-5.6-terra".to_string();
        prep.prep.effort = "low".to_string();
        prep.collection_fallbacks[0].model = "gpt-5.6-luna".to_string();
        let config = generate_run_config(&prep).expect("allowed prep selection");
        assert_eq!(config.prep_model, "gpt-5.6-terra");
        assert_eq!(config.prep_effort, "low");

        let mut writer = default_generate_options();
        writer.writer.model = "claude-opus-5".to_string();
        writer.writer.effort = "medium".to_string();
        writer
            .fallbacks
            .insert(0, phase("claude-code", "claude-opus-4-8", "low"));
        let config = generate_run_config(&writer).expect("allowed writer selection");
        assert_eq!(config.writer_model, "claude-opus-5");
        assert_eq!(config.writer_effort, "medium");
        assert_eq!(config.writer_fallback_model, "claude-opus-4-8");
        assert_eq!(config.writer_fallback_effort, "low");

        let mut final_fallback = default_generate_options();
        final_fallback.fallbacks[0].model = "gpt-5.6-luna".to_string();
        final_fallback.fallbacks[0].effort = "max".to_string();
        let config = generate_run_config(&final_fallback).expect("allowed final fallback");
        assert_eq!(config.codex_fallback_model, "gpt-5.6-luna");
        assert_eq!(config.codex_fallback_effort, "max");
    }

    #[test]
    fn hybrid_payload_rejects_unsupported_values() {
        let mut options = default_generate_options();
        options.prep.effort = "ultra".to_string();
        assert!(generate_run_config(&options)
            .unwrap_err()
            .contains("unsupported effort"));

        let mut options = default_generate_options();
        options.collection_fallbacks[0].model = options.prep.model.clone();
        assert!(generate_run_config(&options)
            .unwrap_err()
            .contains("distinct models"));

        let mut options = default_generate_options();
        options.writer.model = "claude-opus-99".to_string();
        assert!(generate_run_config(&options)
            .unwrap_err()
            .contains("unsupported model"));

        let mut options = default_generate_options();
        options
            .fallbacks
            .insert(0, phase("claude-code", "claude-opus-4-8", "low"));
        assert!(generate_run_config(&options)
            .unwrap_err()
            .contains("different model"));
    }

    #[test]
    fn canonical_quality_gates_cannot_be_disabled_by_payload() {
        let mut options = default_generate_options();
        options.require_visuals = false;
        let error = generate_run_config(&options).unwrap_err();
        assert!(error.contains("requires both"), "unexpected error: {error}");
    }

    #[test]
    fn generate_and_resume_payloads_are_structurally_separate() {
        let legacy = serde_json::json!({
            "prepProvider": "codex-chatgpt",
            "prepModel": "gpt-5.6-sol",
            "prepEffort": "xhigh",
            "writerProvider": "claude-code",
            "writerModel": "fable",
            "writerEffort": "max",
            "fallbacks": []
        });
        assert!(serde_json::from_value::<GenerateOptions>(legacy).is_err());

        let resume = serde_json::json!({
            "prep": { "provider": "codex-chatgpt", "model": "gpt-5.6-sol", "effort": "low" },
            "collectionFallbacks": [
                { "provider": "codex-chatgpt", "model": "gpt-5.6-terra", "effort": "medium" },
                { "provider": "codex-chatgpt", "model": "gpt-5.6-luna", "effort": "medium" }
            ],
            "writer": { "provider": "claude-code", "model": "claude-opus-4-8", "effort": "medium" },
            "fallbacks": [
                { "provider": "codex-chatgpt", "model": "gpt-5.6-sol", "effort": "high" }
            ]
        });
        let options: ResumePrepOptions = serde_json::from_value(resume).expect("resume payload");
        let config = resume_run_config(&options).expect("resume config");
        assert_eq!(config.prep_model, "gpt-5.6-sol");
        assert_eq!(config.prep_effort, "low");
        assert_eq!(config.writer_model, "claude-opus-4-8");
        assert_eq!(config.writer_effort, "medium");
    }

    #[test]
    fn visual_skeleton_write_is_explicit_create_new_and_never_overwrites() {
        let root = std::env::temp_dir().join(format!(
            "guide-watcher-visual-skeleton-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("lecture.pdf.guide-visuals.json");
        let skeleton = serde_json::json!({"decision": "authoring-todo"});
        write_visual_skeleton(&path, &skeleton).unwrap();
        let original = std::fs::read(&path).unwrap();
        let error =
            write_visual_skeleton(&path, &serde_json::json!({"different": true})).unwrap_err();
        assert!(error.contains("refusing to overwrite"));
        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn visual_inspection_worker_is_async_and_honors_its_precreated_token() {
        let token = process_registry::cancellation_token();
        process_registry::cancel(token);
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            inspect_visuals_with_cancellation(
                std::path::Path::new("definitely-missing-source.pdf"),
                false,
                token,
            ),
        )
        .await
        .expect("cancelled inspection worker must return promptly");
        process_registry::finish(token);
        let error = result.unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }
}
