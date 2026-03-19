// src-tauri/src/claude.rs
use crate::config;
use serde::Serialize;
use std::path::Path;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use uuid::Uuid;

#[derive(Clone, Serialize)]
pub struct JobStarted {
    pub job_id: String,
}

#[derive(Clone, Serialize)]
pub struct JobOutput {
    pub job_id: String,
    pub line: String,
}

#[derive(Clone, Serialize)]
pub struct JobDone {
    pub job_id: String,
    pub exit_code: i32,
    pub output_file_exists: bool,
}

/// Start a Claude guide generation job. Returns the job_id.
pub async fn start_job(
    app: AppHandle,
    filepath: String,
    model: String,
    effort: String,
) -> String {
    let job_id = Uuid::new_v4().to_string();

    let app_clone = app.clone();
    let jid = job_id.clone();

    tauri::async_runtime::spawn(async move {
        run_claude_job(app_clone, jid, filepath, model, effort).await;
    });

    job_id
}

async fn run_claude_job(
    app: AppHandle,
    job_id: String,
    filepath: String,
    model: String,
    effort: String,
) {
    // Validate Git Bash exists
    if !Path::new(config::GIT_BASH).exists() {
        emit_error(
            &app,
            &job_id,
            &format!(
                "Error: Git Bash not found at {}. Install Git for Windows.",
                config::GIT_BASH
            ),
        );
        return;
    }

    // Build paths
    let filepath_posix = filepath.replace('\\', "/");
    let output_dir = Path::new(&filepath)
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| ".".to_string());
    let filename = Path::new(&filepath)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    let output_name = Path::new(&filename)
        .file_stem()
        .map(|s| s.to_string_lossy().replace(' ', "_"))
        .unwrap_or_default()
        + "_Guide.md";
    let output_path = format!("{}/{}", output_dir, output_name);

    let ext = Path::new(&filepath)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    // Per-extension read instruction
    let read_part = match ext.as_str() {
        "pdf" => format!(
            "Then extract text from the PDF at '{}' using pymupdf (fitz) via the Bash tool — \
             the Read tool cannot render PDFs on this Windows machine.",
            filepath_posix
        ),
        "html" => format!(
            "Then extract content from the HTML lecture at '{}' using the regex extraction method \
             described in the template (JS template literals, not BeautifulSoup — the content is in \
             JavaScript const slides = [...] with template literals, not in the DOM).",
            filepath_posix
        ),
        _ => format!(
            "Then read the lecture file at '{}' using the Read tool.",
            filepath_posix
        ),
    };

    let template_posix = config::TEMPLATE_FILE.replace('\\', "/");
    let dgist_posix = config::DGIST_ROOT.replace('\\', "/");

    let prompt = format!(
        "Read the study guide generation instructions from '{}' using the Read tool. \
         {} \
         Follow every instruction in that template to create a comprehensive study guide. \
         Also check for textbook PDFs in '{}' (filenames containing 'libgen.li' are textbooks) \
         and weave relevant textbook content into the guide as described in the template. \
         You have access to the full DGIST folder at '{}' including all semesters — \
         if prior course material is relevant, you may reference it. \
         Save the guide as '{}' in '{}'.",
        template_posix, read_part, output_dir, dgist_posix, output_name, output_dir
    );

    // Build Claude flags
    let flags = format!(
        "--model {} --effort {} --dangerously-skip-permissions --add-dir \"{}\"",
        model, effort, dgist_posix
    );

    // Write temp shell script
    let sh_content = format!(
        "#!/bin/bash\ncd \"{}\"\nclaude -p --verbose --output-format stream-json {} <<'GUIDEPROMPT'\n{}\nGUIDEPROMPT\n",
        output_dir, flags, prompt
    );

    let sh_path = std::env::temp_dir().join(format!(
        "guide_gen_{}_{}.sh",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        &job_id[..8]
    ));

    if let Err(e) = std::fs::write(&sh_path, &sh_content) {
        emit_error(
            &app,
            &job_id,
            &format!("Error: Failed to write temp script: {}", e),
        );
        return;
    }

    // Emit job-started
    let _ = app.emit("job-started", JobStarted { job_id: job_id.clone() });

    // Spawn Claude via Git Bash
    // --login is REQUIRED: it sources ~/.bash_profile which puts `claude` (npm global) on PATH.
    // Without --login, `claude` command is not found. Never remove this flag.
    #[cfg(windows)]
    let child = {
        Command::new(config::GIT_BASH)
            .args(["--login", &sh_path.to_string_lossy()])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .spawn()
    };

    #[cfg(not(windows))]
    let child = Command::new(config::GIT_BASH)
        .args(["--login", &sh_path.to_string_lossy()])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn();

    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            emit_error(
                &app,
                &job_id,
                &format!("Error: Failed to spawn process: {}", e),
            );
            return;
        }
    };

    // Stream stdout line by line
    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            let _ = app.emit(
                "job-output",
                JobOutput {
                    job_id: job_id.clone(),
                    line,
                },
            );
        }
    }

    // Wait for process to exit
    let exit_code = match child.wait().await {
        Ok(status) => status.code().unwrap_or(-1),
        Err(_) => -1,
    };

    let output_file_exists = Path::new(&output_path).exists();

    let _ = app.emit(
        "job-done",
        JobDone {
            job_id: job_id.clone(),
            exit_code,
            output_file_exists,
        },
    );

    // Clean up temp script
    let _ = std::fs::remove_file(&sh_path);
}

fn emit_error(app: &AppHandle, job_id: &str, message: &str) {
    let _ = app.emit(
        "job-output",
        JobOutput {
            job_id: job_id.to_string(),
            line: message.to_string(),
        },
    );
    let _ = app.emit(
        "job-done",
        JobDone {
            job_id: job_id.to_string(),
            exit_code: -1,
            output_file_exists: false,
        },
    );
}
