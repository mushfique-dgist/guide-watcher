# Guide Watcher v2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Tauri v2 desktop app that watches for lecture files, prompts the user, runs Claude Code in the background, and streams live output — replacing the current Python/tkinter version.

**Architecture:** Rust backend handles file watching (`notify` crate), Claude process management (`tokio::process`), and system tray. Svelte 5 frontend renders a sidebar + log panel layout with Raycast-inspired glass dark theme. IPC via Tauri commands and events.

**Tech Stack:** Rust, Tauri v2, Svelte 5, Tailwind CSS 4, Vite, notify, tokio

**Spec:** `docs/superpowers/specs/2026-03-19-guide-watcher-v2-design.md`

---

## Task 0: Install Prerequisites

**Files:** None (system setup)

This task installs the Rust toolchain. MSVC Build Tools are required by Tauri on Windows for native linking. The user already has Node.js 22.

- [ ] **Step 1: Install Rust via rustup**

Run:
```bash
winget install Rustlang.Rustup
```

If `winget` is unavailable, download and run `rustup-init.exe` from https://rustup.rs/. Accept defaults (stable toolchain, MSVC target).

- [ ] **Step 2: Verify Rust installation**

Close and reopen terminal, then run:
```bash
rustc --version && cargo --version
```
Expected: version numbers for both (e.g. `rustc 1.82.x`, `cargo 1.82.x`).

- [ ] **Step 3: Verify MSVC Build Tools**

Tauri requires the MSVC C++ build tools. Check if they're already installed:
```bash
where cl.exe
```
If not found, install Visual Studio Build Tools 2022 with the "Desktop development with C++" workload via https://visualstudio.microsoft.com/visual-cpp-build-tools/. This is a ~3GB download.

- [ ] **Step 4: Commit — no code yet, just document the setup**

No commit needed. Proceed to scaffolding.

---

## Task 1: Scaffold Tauri + Svelte Project

**Files:**
- Create: `package.json`, `vite.config.js`, `tailwind.config.js`, `src/main.js`, `src/App.svelte`, `src-tauri/` (entire directory)

- [ ] **Step 1: Scaffold with create-tauri-app**

```bash
cd "C:/Users/you/projects/guide-watcher"
npm create tauri-app@latest . -- --template svelte
```

When prompted:
- Project name: `guide-watcher`
- Frontend language: `JavaScript`
- Package manager: `npm`
- UI template: `Svelte`

This creates `src/`, `src-tauri/`, `package.json`, `vite.config.js`.

- [ ] **Step 2: Install npm dependencies**

```bash
npm install
```

- [ ] **Step 3: Install Tailwind CSS 4**

```bash
npm install -D tailwindcss @tailwindcss/vite
```

- [ ] **Step 4: Configure Vite for Tailwind**

Replace `vite.config.js`:

```javascript
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [svelte(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
});
```

- [ ] **Step 5: Create `src/app.css` with Tailwind import and design tokens**

```css
@import "tailwindcss";

:root {
  /* Backgrounds */
  --bg-base: #0c0c14;
  --bg-surface: rgba(255, 255, 255, 0.04);
  --bg-surface-hover: rgba(255, 255, 255, 0.06);
  --bg-panel: rgba(0, 0, 0, 0.25);
  --bg-active: linear-gradient(135deg, rgba(59,130,246,0.1), rgba(139,92,246,0.06));

  /* Borders */
  --border-subtle: rgba(255, 255, 255, 0.06);
  --border-panel: rgba(255, 255, 255, 0.04);
  --border-active: rgba(99, 102, 241, 0.15);

  /* Text */
  --text-primary: #f0f0f0;
  --text-secondary: rgba(255, 255, 255, 0.45);
  --text-tertiary: rgba(255, 255, 255, 0.25);
  --text-dim: rgba(255, 255, 255, 0.12);

  /* Accents */
  --accent-blue: #3b82f6;
  --accent-purple: #8b5cf6;
  --accent-green: #34d399;
  --accent-red: #f87171;
  --accent-yellow: #fbbf24;
  --accent-gradient: linear-gradient(135deg, #3b82f6, #8b5cf6);

  /* Log colors */
  --log-read: rgba(147, 197, 253, 0.7);
  --log-bash: rgba(110, 231, 183, 0.7);
  --log-write: rgba(196, 181, 253, 0.7);
  --log-text: rgba(255, 255, 255, 0.3);
  --log-dim: rgba(255, 255, 255, 0.15);
  --log-line-num: rgba(255, 255, 255, 0.12);

  /* Fonts */
  --font-ui: 'Inter', -apple-system, 'Segoe UI', system-ui, sans-serif;
  --font-mono: 'JetBrains Mono', 'Cascadia Code', 'Fira Code', monospace;
}

* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

html, body {
  height: 100%;
  overflow: hidden;
  background: var(--bg-base);
  color: var(--text-primary);
  font-family: var(--font-ui);
  font-size: 13px;
  -webkit-font-smoothing: antialiased;
}

#app {
  height: 100%;
}

/* Scrollbar styling */
::-webkit-scrollbar { width: 6px; }
::-webkit-scrollbar-track { background: transparent; }
::-webkit-scrollbar-thumb { background: rgba(255,255,255,0.08); border-radius: 3px; }
::-webkit-scrollbar-thumb:hover { background: rgba(255,255,255,0.15); }

/* Animations */
@keyframes pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}

@keyframes slideIn {
  from { transform: translateX(-100%); }
  to { transform: translateX(0); }
}
```

- [ ] **Step 6: Add Tauri plugins to Cargo.toml**

Edit `src-tauri/Cargo.toml` — add to `[dependencies]`:

```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tauri-plugin-notification = "2"
tauri-plugin-dialog = "2"
notify = "7"
tokio = { version = "1", features = ["full"] }
uuid = { version = "1", features = ["v4"] }
```

- [ ] **Step 7: Configure tauri.conf.json**

Edit `src-tauri/tauri.conf.json`. Set these key fields:

```json
{
  "productName": "Guide Watcher",
  "version": "2.0.0",
  "identifier": "com.mushfique.guide-watcher",
  "build": {
    "frontendDist": "../dist",
    "devUrl": "http://localhost:1420",
    "beforeDevCommand": "npm run dev",
    "beforeBuildCommand": "npm run build"
  },
  "app": {
    "withGlobalTauri": true,
    "windows": [
      {
        "title": "Guide Watcher",
        "width": 800,
        "height": 600,
        "minWidth": 600,
        "minHeight": 400,
        "resizable": true,
        "visible": false,
        "decorations": true,
        "backgroundColor": "#0c0c14"
      }
    ]
  },
  "plugins": {
    "notification": {},
    "dialog": {}
  }
}
```

Also ensure the capabilities file at `src-tauri/capabilities/default.json` includes:
```json
{
  "identifier": "default",
  "description": "Default capabilities",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "notification:default",
    "dialog:default"
  ]
}
```

- [ ] **Step 8: Verify the scaffold builds**

```bash
cd "C:/Users/you/projects/guide-watcher"
npm run tauri dev
```

Expected: A window opens (will show default Svelte content). Close it. First build takes 2-5 minutes (compiling Rust deps). Subsequent builds are fast.

- [ ] **Step 9: Commit scaffold**

```bash
git init
echo "node_modules\ntarget\ndist\n.superpowers" > .gitignore
git add -A
git commit -m "feat: scaffold Tauri v2 + Svelte + Tailwind project"
```

---

## Task 2: Rust Backend — config.rs

**Files:**
- Create: `src-tauri/src/config.rs`
- Modify: `src-tauri/src/main.rs` (add `mod config;`)

- [ ] **Step 1: Create config.rs**

```rust
// src-tauri/src/config.rs
use serde::Serialize;

pub const WATCH_DIR: &str = "C:/Users/you/Documents/Study/4th Semester";
pub const DGIST_ROOT: &str = "C:/Users/you/Documents/Study";
pub const TEMPLATE_FILE: &str =
    "C:/Users/you/Documents/Study/4th Semester/_automation/guide_prompt.txt";
pub const GIT_BASH: &str = "C:/Program Files/Git/bin/bash.exe";

pub const WATCHED_EXTENSIONS: &[&str] = &[".pdf", ".html"];
pub const SKIP_FOLDERS: &[&str] = &[
    "_automation",
    "__MACOSX",
    ".ipynb_checkpoints",
    ".claude",
    ".vscode",
    "Archive-ugrp",
    "kisub kim",
    "output",
    "datalab",
];
pub const SKIP_PATTERNS: &[&str] = &["libgen.li", "syllabus", "guideline"];

pub const BATCH_WINDOW_SECS: u64 = 3;
pub const COOLDOWN_SECS: u64 = 60;
pub const FILE_STABILITY_TIMEOUT_SECS: u64 = 20;
pub const FILE_STABILITY_POLL_MS: u64 = 1000;
pub const DIR_SCAN_DELAY_MS: u64 = 1500;

pub const DEFAULT_MODEL: &str = "opus";
pub const DEFAULT_EFFORT: &str = "max";
pub const MODELS: &[&str] = &["opus", "sonnet", "haiku"];
pub const EFFORTS: &[&str] = &["max", "high", "medium", "low"];

#[derive(Serialize, Clone)]
pub struct ConfigResponse {
    pub models: Vec<String>,
    pub efforts: Vec<String>,
    pub default_model: String,
    pub default_effort: String,
}

impl ConfigResponse {
    pub fn new() -> Self {
        Self {
            models: MODELS.iter().map(|s| s.to_string()).collect(),
            efforts: EFFORTS.iter().map(|s| s.to_string()).collect(),
            default_model: DEFAULT_MODEL.to_string(),
            default_effort: DEFAULT_EFFORT.to_string(),
        }
    }
}

/// Check if a file path should be skipped based on folder names and filename patterns.
pub fn should_skip(filepath: &str) -> bool {
    let normalized = filepath.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').collect();

    // Check skip folders (any path component)
    for part in &parts {
        if SKIP_FOLDERS.iter().any(|f| f.eq_ignore_ascii_case(part)) {
            return true;
        }
    }

    // Check skip patterns (filename substring, case-insensitive)
    if let Some(filename) = parts.last() {
        let lower = filename.to_lowercase();
        if SKIP_PATTERNS.iter().any(|p| lower.contains(&p.to_lowercase())) {
            return true;
        }
    }

    false // not skipped
}

/// Check if file extension is watched.
pub fn is_watched_extension(filepath: &str) -> bool {
    let lower = filepath.to_lowercase();
    WATCHED_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}
```

- [ ] **Step 2: Register module in main.rs**

Add to the top of `src-tauri/src/main.rs`:
```rust
mod config;
```

- [ ] **Step 3: Verify it compiles**

```bash
cd "C:/Users/you/projects/guide-watcher/src-tauri"
cargo check
```
Expected: compiles with no errors.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/config.rs src-tauri/src/main.rs
git commit -m "feat: add config module with paths, filters, and constants"
```

---

## Task 3: Rust Backend — watcher.rs

**Files:**
- Create: `src-tauri/src/watcher.rs`
- Modify: `src-tauri/src/main.rs` (add `mod watcher;`)

- [ ] **Step 1: Create watcher.rs**

```rust
// src-tauri/src/watcher.rs
use crate::config;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Emitter};
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
                        let _ = window.show();
                        let _ = window.set_focus();
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
```

- [ ] **Step 2: Add walkdir to Cargo.toml**

Add to `src-tauri/Cargo.toml` `[dependencies]`:
```toml
walkdir = "2"
```

- [ ] **Step 3: Register module in main.rs**

Add to `src-tauri/src/main.rs`:
```rust
mod watcher;
```

- [ ] **Step 4: Verify it compiles**

```bash
cd "C:/Users/you/projects/guide-watcher/src-tauri"
cargo check
```
Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/watcher.rs src-tauri/Cargo.toml src-tauri/src/main.rs
git commit -m "feat: add file watcher with batching, cooldown, and stability checks"
```

---

## Task 4: Rust Backend — claude.rs

**Files:**
- Create: `src-tauri/src/claude.rs`
- Modify: `src-tauri/src/main.rs` (add `mod claude;`)

- [ ] **Step 1: Create claude.rs**

```rust
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
    let child = Command::new(config::GIT_BASH)
        .args(["--login", &sh_path.to_string_lossy()])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .spawn();

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
```

- [ ] **Step 2: Register module in main.rs**

Add to `src-tauri/src/main.rs`:
```rust
mod claude;
```

- [ ] **Step 3: Verify it compiles**

```bash
cd "C:/Users/you/projects/guide-watcher/src-tauri"
cargo check
```
Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/claude.rs src-tauri/src/main.rs
git commit -m "feat: add Claude process manager with streaming stdout and error handling"
```

---

## Task 5: Rust Backend — main.rs (Tray, IPC, Startup)

**Files:**
- Modify: `src-tauri/src/main.rs` (full rewrite)

- [ ] **Step 1: Rewrite main.rs**

```rust
// src-tauri/src/main.rs
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

fn main() {
    // Startup validation
    if !std::path::Path::new(config::TEMPLATE_FILE).exists() {
        eprintln!("Template file not found: {}", config::TEMPLATE_FILE);
        // Dialog will be shown after app setup if we can't find template
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
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
                .menu_on_left_click(false)
                .on_menu_event(move |app, event| match event.id.as_ref() {
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
                .on_tray_icon_event(|tray, event| {
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
```

- [ ] **Step 2: Verify it compiles**

```bash
cd "C:/Users/you/projects/guide-watcher/src-tauri"
cargo check
```
Expected: no errors.

- [ ] **Step 3: Test the app launches and shows tray icon**

```bash
cd "C:/Users/you/projects/guide-watcher"
npm run tauri dev
```

Expected: no visible window, but a tray icon appears. Right-click shows "Show Window" and "Quit". Left-click shows the window (with placeholder Svelte content). Close button hides to tray.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/main.rs
git commit -m "feat: add main with system tray, IPC commands, and startup validation"
```

---

## Task 6: Frontend — Stores and Parser

**Files:**
- Create: `src/stores/jobs.js`
- Create: `src/lib/parser.js`

- [ ] **Step 1: Create the job store**

```javascript
// src/stores/jobs.js
import { writable, derived } from 'svelte/store';

export const jobs = writable([]);
export const selectedJobId = writable(null);
export const pendingFiles = writable([]);
export const showConfirmPanel = writable(false);

const LOG_CAP = 2000;

export const selectedJob = derived(
  [jobs, selectedJobId],
  ([$jobs, $selectedJobId]) => $jobs.find(j => j.id === $selectedJobId) ?? null
);

export const activeJobs = derived(jobs, $jobs =>
  $jobs.filter(j => j.status === 'starting' || j.status === 'working')
);

export const completedJobs = derived(jobs, $jobs =>
  $jobs.filter(j => j.status === 'done' || j.status === 'done-warnings' || j.status === 'failed')
);

export function addJob(job) {
  jobs.update(list => [job, ...list]);
}

export function updateJobStatus(jobId, status, finishedAt = null) {
  jobs.update(list =>
    list.map(j => j.id === jobId ? { ...j, status, finishedAt } : j)
  );
}

export function updateJobActivity(jobId, activity) {
  jobs.update(list =>
    list.map(j => j.id === jobId ? { ...j, activity } : j)
  );
}

export function appendLogLine(jobId, text, tag) {
  jobs.update(list =>
    list.map(j => {
      if (j.id !== jobId) return j;
      const lineNum = j.logLines.length + 1;
      let logLines = [...j.logLines, { text, tag, lineNum }];
      // Cap at LOG_CAP, evict oldest
      if (logLines.length > LOG_CAP) {
        logLines = logLines.slice(logLines.length - LOG_CAP);
      }
      return { ...j, logLines };
    })
  );
}
```

- [ ] **Step 2: Create the parser**

```javascript
// src/lib/parser.js

/**
 * Parse a line of Claude stream-json output.
 * @param {string} raw - raw JSON string from stdout
 * @returns {{ text: string, tag: string|null, activity: string|null }}
 */
export function parseLine(raw) {
  let data;
  try {
    data = JSON.parse(raw);
  } catch {
    // Non-JSON line
    const trimmed = raw.length > 150 ? raw.slice(0, 147) + '...' : raw;
    const isError = /error/i.test(raw);
    return {
      text: trimmed,
      tag: isError ? 'error' : 'dim',
      activity: null,
    };
  }

  return parseJsonEvent(data);
}

function parseJsonEvent(data) {
  const type = data.type || '';
  const subtype = data.subtype || '';

  // Tool use events
  if (subtype === 'tool_use' || type === 'tool_use') {
    const tool = data.tool_name || data.name || 'unknown';
    const input = data.input || {};

    const toolLabels = {
      Read: 'Read', Bash: 'Bash', Write: 'Write', Edit: 'Edit',
      Glob: 'Search', Grep: 'Grep', Agent: 'Sub-agent', TodoWrite: 'Planning',
    };
    const label = toolLabels[tool] || tool;

    // Extract detail from input
    let detail = '';
    if (input.file_path) {
      detail = input.file_path.replace(/\\/g, '/').split('/').pop();
    } else if (input.command) {
      detail = input.command.length > 55
        ? input.command.slice(0, 52) + '...'
        : input.command;
    } else if (input.pattern) {
      detail = input.pattern;
    }

    const text = detail ? `▸ ${label}: ${detail}` : `▸ ${label}...`;
    const tag = getToolTag(tool);

    return { text, tag, activity: detail ? `${label}: ${detail}` : `${label}...` };
  }

  // Tool results
  if (subtype === 'tool_result' || type === 'tool_result') {
    const tool = data.tool_name || data.name || '';
    return { text: tool ? `✓ ${tool} done` : '✓ done', tag: 'dim', activity: null };
  }

  // Text generation
  if (subtype === 'text' || type === 'text') {
    const text = (data.text || '').trim();
    if (text.length > 5) {
      const snippet = text.slice(0, 80);
      return { text: `▸ Writing: ${snippet}`, tag: 'text', activity: null };
    }
    return { text: '▸ Generating...', tag: 'text', activity: null };
  }

  // Result / completion
  if (type === 'result') {
    return { text: 'Generation complete', tag: 'header', activity: null };
  }

  // System / init
  if (type === 'system' || type === 'init') {
    return { text: 'Initializing...', tag: 'dim', activity: null };
  }

  // Fallback
  return { text: JSON.stringify(data).slice(0, 150), tag: 'dim', activity: null };
}

function getToolTag(tool) {
  if (['Read', 'Glob', 'Grep'].includes(tool)) return 'read';
  if (tool === 'Bash') return 'bash';
  if (['Write', 'Edit'].includes(tool)) return 'write';
  return 'read';
}
```

- [ ] **Step 3: Create stores directory**

```bash
mkdir -p "C:/Users/you/projects/guide-watcher/src/stores"
mkdir -p "C:/Users/you/projects/guide-watcher/src/lib"
```

- [ ] **Step 4: Commit**

```bash
git add src/stores/jobs.js src/lib/parser.js
git commit -m "feat: add job store and Claude output parser"
```

---

## Task 7: Frontend — Sidebar Component

**Files:**
- Create: `src/lib/Sidebar.svelte`

- [ ] **Step 1: Create Sidebar.svelte**

```svelte
<!-- src/lib/Sidebar.svelte -->
<script>
  import { selectedJobId, activeJobs, completedJobs } from '../stores/jobs.js';

  function formatElapsed(startedAt, finishedAt) {
    const end = finishedAt || Date.now();
    const secs = Math.floor((end - startedAt) / 1000);
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    const s = secs % 60;
    if (h) return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
    return `${m}:${String(s).padStart(2, '0')}`;
  }

  function selectJob(id) {
    $selectedJobId = id;
  }

  // Tick every second for elapsed time
  let now = $state(Date.now());
  $effect(() => {
    const interval = setInterval(() => { now = Date.now(); }, 1000);
    return () => clearInterval(interval);
  });
</script>

<aside class="sidebar">
  <!-- Watching status -->
  <div class="watch-status">
    <span class="dot watching"></span>
    <span class="watch-label">Watching</span>
  </div>

  <!-- Active section -->
  {#if $activeJobs.length > 0}
    <div class="section-label">ACTIVE</div>
    {#each $activeJobs as job (job.id)}
      <button
        class="job-item"
        class:selected={$selectedJobId === job.id}
        onclick={() => selectJob(job.id)}
      >
        <div class="job-header">
          <span class="dot active"></span>
          <span class="job-name">{job.filename}</span>
        </div>
        <div class="job-meta">
          <span class="job-folder">{job.folder}</span>
          <span class="job-time">{formatElapsed(job.startedAt, null)}</span>
        </div>
        {#if job.activity}
          <div class="job-activity">{job.activity}</div>
        {/if}
      </button>
    {/each}
  {/if}

  <!-- Completed section -->
  {#if $completedJobs.length > 0}
    <div class="section-label">COMPLETED</div>
    {#each $completedJobs as job (job.id)}
      <button
        class="job-item"
        class:selected={$selectedJobId === job.id}
        onclick={() => selectJob(job.id)}
      >
        <div class="job-header">
          <span class="dot"
            class:done={job.status === 'done'}
            class:warnings={job.status === 'done-warnings'}
            class:failed={job.status === 'failed'}
          ></span>
          <span class="job-name dim">{job.filename}</span>
        </div>
        <div class="job-meta">
          <span class="job-folder">{job.folder}</span>
          <span class="job-time">{formatElapsed(job.startedAt, job.finishedAt)}</span>
        </div>
      </button>
    {/each}
  {/if}

  {#if $activeJobs.length === 0 && $completedJobs.length === 0}
    <div class="empty-state">No jobs yet</div>
  {/if}
</aside>

<style>
  .sidebar {
    width: 240px;
    height: 100%;
    background: var(--glass-bg, linear-gradient(180deg, rgba(255,255,255,0.04), rgba(255,255,255,0.015)));
    border-right: 1px solid var(--border-subtle);
    border-radius: 12px;
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    overflow-y: auto;
  }

  .watch-status {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 10px;
    background: var(--bg-surface);
    border-radius: 8px;
    margin-bottom: 12px;
  }
  .watch-label { color: var(--text-secondary); font-size: 11px; font-weight: 500; }

  .section-label {
    color: var(--text-tertiary);
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 1.8px;
    font-weight: 600;
    padding: 10px 4px 6px;
  }

  .job-item {
    all: unset;
    cursor: pointer;
    display: block;
    width: 100%;
    padding: 10px 12px;
    border-radius: 10px;
    transition: background 0.15s;
  }
  .job-item:hover { background: var(--bg-surface-hover); }
  .job-item.selected {
    background: var(--bg-active);
    border: 1px solid var(--border-active);
  }

  .job-header { display: flex; align-items: center; gap: 8px; }
  .job-name { color: var(--text-primary); font-size: 11px; font-weight: 600; }
  .job-name.dim { color: var(--text-secondary); font-weight: 400; }

  .job-meta {
    display: flex; justify-content: space-between;
    margin-top: 4px; padding-left: 18px;
  }
  .job-folder { color: var(--text-tertiary); font-size: 9px; }
  .job-time { color: rgba(147,197,253,0.6); font-size: 9px; font-variant-numeric: tabular-nums; }

  .job-activity {
    color: rgba(147,197,253,0.5);
    font-size: 8px;
    margin-top: 4px;
    padding-left: 18px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* Dots */
  .dot { width: 6px; height: 6px; border-radius: 50%; flex-shrink: 0; }
  .dot.watching { background: var(--accent-green); box-shadow: 0 0 8px rgba(52,211,153,0.4); }
  .dot.active { background: var(--accent-blue); box-shadow: 0 0 8px rgba(59,130,246,0.4); animation: pulse 1.5s ease-in-out infinite; }
  .dot.done { background: var(--accent-green); }
  .dot.warnings { background: var(--accent-yellow); }
  .dot.failed { background: var(--accent-red); }

  .empty-state {
    color: var(--text-tertiary);
    font-size: 11px;
    text-align: center;
    padding: 24px 0;
  }
</style>
```

- [ ] **Step 2: Commit**

```bash
git add src/lib/Sidebar.svelte
git commit -m "feat: add Sidebar component with job list and status dots"
```

---

## Task 8: Frontend — LogPanel Component

**Files:**
- Create: `src/lib/LogPanel.svelte`

- [ ] **Step 1: Create LogPanel.svelte**

```svelte
<!-- src/lib/LogPanel.svelte -->
<script>
  import { selectedJob } from '../stores/jobs.js';

  let logContainer = $state(null);
  let autoScroll = $state(true);
  let prevLineCount = $state(0);

  function formatElapsed(startedAt, finishedAt) {
    const end = finishedAt || Date.now();
    const secs = Math.floor((end - startedAt) / 1000);
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    const s = secs % 60;
    if (h) return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
    return `${m}:${String(s).padStart(2, '0')}`;
  }

  // Tick every second
  let now = $state(Date.now());
  $effect(() => {
    const interval = setInterval(() => { now = Date.now(); }, 1000);
    return () => clearInterval(interval);
  });

  // Auto-scroll when new lines arrive
  $effect(() => {
    const job = $selectedJob;
    if (!job || !logContainer) return;
    const count = job.logLines.length;
    if (count > prevLineCount && autoScroll) {
      logContainer.scrollTop = logContainer.scrollHeight;
    }
    prevLineCount = count;
  });

  function onScroll() {
    if (!logContainer) return;
    const { scrollTop, scrollHeight, clientHeight } = logContainer;
    autoScroll = scrollHeight - scrollTop - clientHeight < 50;
  }

  function getColorClass(tag) {
    const map = {
      read: 'log-read', bash: 'log-bash', write: 'log-write',
      text: 'log-text', dim: 'log-dim', error: 'log-error', header: 'log-tag-header',
    };
    return map[tag] || 'log-dim';
  }


</script>

{#if $selectedJob}
  {@const job = $selectedJob}
  {@const visibleLines = job.logLines.length > 500 ? job.logLines.slice(-500) : job.logLines}

  <div class="log-panel">
    <!-- Header -->
    <div class="log-header">
      <div class="log-title-area">
        <div class="log-filename">{job.filename}</div>
        <div class="log-meta">{job.folder} · {job.model} · {job.effort} effort</div>
      </div>
      <div class="log-timer">
        <span>{formatElapsed(job.startedAt, job.finishedAt)}</span>
      </div>
    </div>

    <!-- Log lines -->
    <div class="log-body" bind:this={logContainer} onscroll={onScroll}>
      {#each visibleLines as line (line.lineNum)}
        <div class="log-line">
          <span class="line-num">{String(line.lineNum).padStart(3, ' ')}</span>
          <span class={getColorClass(line.tag)}>{line.text}</span>
        </div>
      {/each}
      {#if job.status === 'working' || job.status === 'starting'}
        <div class="log-line">
          <span class="line-num">&nbsp;</span>
          <span class="cursor">█</span>
        </div>
      {/if}
    </div>

    <!-- Status bar -->
    <div class="log-statusbar">
      <div class="status-left">
        {#if job.status === 'working' && job.activity}
          <span class="dot-mini active"></span>
          <span>{job.activity}</span>
        {:else if job.status === 'done'}
          <span class="dot-mini done"></span>
          <span>Complete</span>
        {:else if job.status === 'failed'}
          <span class="dot-mini failed"></span>
          <span>Failed</span>
        {:else}
          <span class="dot-mini active"></span>
          <span>Starting...</span>
        {/if}
      </div>
      <span class="event-count">{job.logLines.length} events</span>
    </div>
  </div>
{:else}
  <div class="log-panel empty">
    <span>Click a job to view its output</span>
  </div>
{/if}

<style>
  .log-panel {
    flex: 1;
    display: flex;
    flex-direction: column;
    background: var(--bg-panel);
    border-radius: 12px;
    border: 1px solid var(--border-panel);
    overflow: hidden;
  }
  .log-panel.empty {
    justify-content: center;
    align-items: center;
    color: var(--text-tertiary);
    font-size: 12px;
  }

  .log-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 14px 16px;
    border-bottom: 1px solid rgba(255,255,255,0.05);
  }
  .log-filename { font-size: 12px; font-weight: 600; }
  .log-meta { color: var(--text-tertiary); font-size: 9px; margin-top: 3px; }
  .log-timer {
    background: rgba(59,130,246,0.08);
    padding: 4px 12px;
    border-radius: 8px;
    border: 1px solid rgba(59,130,246,0.12);
    color: rgba(147,197,253,0.9);
    font-size: 11px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .log-body {
    flex: 1;
    overflow-y: auto;
    padding: 12px 16px;
    font-family: var(--font-mono);
    font-size: 11px;
    line-height: 2;
  }

  .log-line { display: flex; white-space: nowrap; }
  .line-num {
    color: var(--log-line-num);
    width: 32px;
    text-align: right;
    margin-right: 12px;
    user-select: none;
    font-size: 10px;
    flex-shrink: 0;
  }

  .log-read { color: var(--log-read); }
  .log-bash { color: var(--log-bash); }
  .log-write { color: var(--log-write); }
  .log-text { color: var(--log-text); }
  .log-dim { color: var(--log-dim); }
  .log-error { color: var(--accent-red); }
  .log-tag-header { color: var(--accent-purple); font-weight: bold; }

  .cursor { color: rgba(255,255,255,0.1); animation: pulse 1s infinite; }

  .log-statusbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 8px 16px;
    border-top: 1px solid rgba(255,255,255,0.04);
    font-size: 9px;
    color: var(--text-tertiary);
  }
  .status-left { display: flex; align-items: center; gap: 6px; }
  .event-count { color: var(--text-dim); }

  .dot-mini { width: 5px; height: 5px; border-radius: 50%; }
  .dot-mini.active { background: var(--accent-blue); box-shadow: 0 0 6px rgba(59,130,246,0.4); }
  .dot-mini.done { background: var(--accent-green); }
  .dot-mini.failed { background: var(--accent-red); }
</style>
```

- [ ] **Step 2: Commit**

```bash
git add src/lib/LogPanel.svelte
git commit -m "feat: add LogPanel with line numbers, colors, and auto-scroll"
```

---

## Task 9: Frontend — ConfirmPanel Component

**Files:**
- Create: `src/lib/ConfirmPanel.svelte`

- [ ] **Step 1: Create ConfirmPanel.svelte**

```svelte
<!-- src/lib/ConfirmPanel.svelte -->
<script>
  import { invoke } from '@tauri-apps/api/core';
  import { pendingFiles, showConfirmPanel, addJob, selectedJobId } from '../stores/jobs.js';

  let config = $state({ models: [], efforts: [], default_model: 'opus', default_effort: 'max' });
  let model = $state('opus');
  let effort = $state('max');
  let checked = $state([]);

  $effect(() => {
    invoke('get_config').then(c => {
      config = c;
      model = c.default_model;
      effort = c.default_effort;
    });
  });

  $effect(() => {
    // Initialize all checked when pendingFiles changes
    checked = $pendingFiles.map(() => true);
  });

  function getFilename(fp) { return fp.replace(/\\/g, '/').split('/').pop(); }
  function getFolder(fp) {
    const parts = fp.replace(/\\/g, '/').split('/');
    return parts.length > 1 ? parts[parts.length - 2] : '';
  }
  function getExt(fp) { return fp.split('.').pop().toUpperCase(); }

  async function onGenerate() {
    const selected = $pendingFiles.filter((_, i) => checked[i]);
    if (selected.length === 0) { onSkip(); return; }

    const jobIds = await invoke('approve_files', {
      filePaths: selected, model, effort,
    });

    // Create job entries in store
    for (let i = 0; i < selected.length; i++) {
      addJob({
        id: jobIds[i],
        filename: getFilename(selected[i]),
        filepath: selected[i],
        folder: getFolder(selected[i]),
        outputName: getFilename(selected[i]).replace(/\.[^.]+$/, '').replace(/ /g, '_') + '_Guide.md',
        status: 'starting',
        activity: 'Launching Claude...',
        model, effort,
        startedAt: Date.now(),
        finishedAt: null,
        logLines: [],
      });
    }

    // Auto-select first new job
    $selectedJobId = jobIds[0];
    onSkip(); // close panel
  }

  function onSkip() {
    $showConfirmPanel = false;
    $pendingFiles = [];
  }
</script>

{#if $showConfirmPanel && $pendingFiles.length > 0}
  <div class="confirm-panel">
    <h2>{$pendingFiles.length} new file{$pendingFiles.length > 1 ? 's' : ''} detected</h2>

    <div class="file-list">
      {#each $pendingFiles as fp, i (fp)}
        <label class="file-row">
          <input type="checkbox" bind:checked={checked[i]} />
          <span class="badge" class:pdf={getExt(fp) === 'PDF'} class:html={getExt(fp) === 'HTML'}>
            {getExt(fp)}
          </span>
          <span class="fname">{getFilename(fp)}</span>
          <span class="ffolder">{getFolder(fp)}</span>
        </label>
      {/each}
    </div>

    <div class="settings">
      <label>
        <span>Model</span>
        <select bind:value={model}>
          {#each config.models as m}<option value={m}>{m}</option>{/each}
        </select>
      </label>
      <label>
        <span>Effort</span>
        <select bind:value={effort}>
          {#each config.efforts as e}<option value={e}>{e}</option>{/each}
        </select>
      </label>
    </div>

    <div class="buttons">
      <button class="btn-skip" onclick={onSkip}>Skip</button>
      <button class="btn-generate" onclick={onGenerate}>Generate</button>
    </div>
  </div>
{/if}

<style>
  .confirm-panel {
    position: absolute;
    top: 0; left: 0; bottom: 0;
    width: 280px;
    background: var(--bg-base);
    border-right: 1px solid var(--border-subtle);
    border-radius: 12px 0 0 12px;
    padding: 20px 16px;
    z-index: 10;
    display: flex;
    flex-direction: column;
    animation: slideIn 0.3s ease;
  }

  h2 { font-size: 14px; font-weight: 600; margin-bottom: 16px; }

  .file-list {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .file-row {
    display: flex; align-items: center; gap: 8px;
    padding: 6px 8px; border-radius: 6px; cursor: pointer;
    font-size: 11px;
  }
  .file-row:hover { background: var(--bg-surface-hover); }
  .file-row input[type="checkbox"] { accent-color: var(--accent-blue); }

  .badge {
    font-size: 8px; font-weight: 700; padding: 2px 6px; border-radius: 4px;
    text-transform: uppercase;
  }
  .badge.pdf { background: rgba(59,130,246,0.15); color: rgba(147,197,253,0.9); }
  .badge.html { background: rgba(52,211,153,0.15); color: rgba(110,231,183,0.9); }

  .fname { color: var(--text-primary); flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ffolder { color: var(--text-tertiary); font-size: 9px; }

  .settings {
    display: flex; gap: 12px; margin: 16px 0;
  }
  .settings label { display: flex; flex-direction: column; gap: 4px; flex: 1; }
  .settings label span { color: var(--text-secondary); font-size: 9px; text-transform: uppercase; letter-spacing: 1px; }
  .settings select {
    background: var(--bg-surface); color: var(--text-primary);
    border: 1px solid var(--border-subtle); border-radius: 6px;
    padding: 6px 8px; font-size: 11px;
  }

  .buttons { display: flex; gap: 8px; }
  .btn-skip {
    flex: 1; padding: 8px; border-radius: 8px;
    background: var(--bg-surface); color: var(--text-secondary);
    border: 1px solid var(--border-subtle);
    cursor: pointer; font-size: 11px; font-weight: 500;
  }
  .btn-generate {
    flex: 2; padding: 8px; border-radius: 8px;
    background: linear-gradient(135deg, var(--accent-blue), var(--accent-purple));
    color: white; border: none;
    cursor: pointer; font-size: 11px; font-weight: 600;
  }
  .btn-generate:hover { opacity: 0.9; }
</style>
```

- [ ] **Step 2: Commit**

```bash
git add src/lib/ConfirmPanel.svelte
git commit -m "feat: add ConfirmPanel with file selection, model/effort dropdowns"
```

---

## Task 10: Frontend — App.svelte (Root Wiring)

**Files:**
- Modify: `src/App.svelte` (full rewrite)
- Modify: `src/main.js` (ensure CSS import)

- [ ] **Step 1: Rewrite App.svelte**

```svelte
<!-- src/App.svelte -->
<script>
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { parseLine } from './lib/parser.js';
  import {
    pendingFiles, showConfirmPanel,
    updateJobStatus, updateJobActivity, appendLogLine, selectedJobId,
  } from './stores/jobs.js';
  import Sidebar from './lib/Sidebar.svelte';
  import LogPanel from './lib/LogPanel.svelte';
  import ConfirmPanel from './lib/ConfirmPanel.svelte';

  onMount(() => {
    // New files detected by watcher
    listen('new-files', (event) => {
      $pendingFiles = event.payload;
      $showConfirmPanel = true;
    });

    // Job started (process spawned)
    listen('job-started', (event) => {
      updateJobStatus(event.payload.job_id, 'working');
    });

    // Job output line
    listen('job-output', (event) => {
      const { job_id, line } = event.payload;
      const parsed = parseLine(line);
      appendLogLine(job_id, parsed.text, parsed.tag);
      if (parsed.activity) {
        updateJobActivity(job_id, parsed.activity);
      }
    });

    // Job completed
    listen('job-done', (event) => {
      const { job_id, exit_code, output_file_exists } = event.payload;
      let status;
      if (exit_code === 0 && output_file_exists) status = 'done';
      else if (output_file_exists) status = 'done-warnings';
      else status = 'failed';
      updateJobStatus(job_id, status, Date.now());
    });
  });
</script>

<div class="app-layout">
  <div class="sidebar-container">
    <Sidebar />
    <ConfirmPanel />
  </div>
  <LogPanel />
</div>

<style>
  .app-layout {
    display: flex;
    height: 100vh;
    gap: 10px;
    padding: 12px;
    background: var(--bg-base);
  }

  .sidebar-container {
    position: relative;
    flex-shrink: 0;
  }
</style>
```

- [ ] **Step 2: Update main.js**

```javascript
// src/main.js
import './app.css';
import App from './App.svelte';
import { mount } from 'svelte';

const app = mount(App, { target: document.getElementById('app') });

export default app;
```

- [ ] **Step 3: Verify dev build works**

```bash
cd "C:/Users/you/projects/guide-watcher"
npm run tauri dev
```

Expected: Window shows sidebar (empty) + log panel ("Click a job to view its output"). Deep dark theme. Tray icon works.

- [ ] **Step 4: Commit**

```bash
git add src/App.svelte src/main.js
git commit -m "feat: wire up App with event listeners, sidebar, log panel, and confirm panel"
```

---

## Task 11: Integration Test — End-to-End

**Files:** None (manual test)

- [ ] **Step 1: Run the app in dev mode**

```bash
cd "C:/Users/you/projects/guide-watcher"
npm run tauri dev
```

- [ ] **Step 2: Test file detection**

Copy a PDF into `C:/Users/you/Documents/Study\4th Semester\Data Structure\`. The confirm panel should slide in. Verify:
- File appears with checkbox checked
- Model defaults to "opus", effort to "max"
- "Generate" and "Skip" buttons visible

- [ ] **Step 3: Test job execution**

Click "Generate". Verify:
- Job appears in sidebar with pulsing blue dot
- Elapsed time ticks
- Log panel shows live output lines with correct colors
- Status bar shows current activity

- [ ] **Step 4: Test completion**

Wait for job to finish (or test with a quick prompt). Verify:
- Dot turns green (done) or red (failed)
- Elapsed time freezes
- Windows notification appears
- "Generation complete" line in log

- [ ] **Step 5: Test tray behavior**

- Close window → should hide to tray (not quit)
- Click tray icon → window reappears
- Right-click tray → "Quit" exits the app

- [ ] **Step 6: Commit any fixes**

```bash
git add -A
git commit -m "fix: integration test fixes"
```

---

## Task 12: Production Build & Startup

**Files:**
- Modify: `C:/Users/you/AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup\AutoGuideWatcher.bat`

- [ ] **Step 1: Build release binary**

```bash
cd "C:/Users/you/projects/guide-watcher"
npm run tauri build
```

Expected: produces `src-tauri/target/release/guide-watcher.exe` (~5-10MB). First release build takes 3-5 minutes.

- [ ] **Step 2: Test the release binary**

```bash
"./src-tauri/target/release/guide-watcher.exe"
```

Verify: starts hidden in tray, same behavior as dev mode.

- [ ] **Step 3: Update Windows startup batch file**

```bat
@echo off
start "" "C:/Users/you/projects\guide-watcher\src-tauri\target\release\guide-watcher.exe"
```

- [ ] **Step 4: Final commit**

```bash
git add -A
git commit -m "feat: production build and startup integration"
```
