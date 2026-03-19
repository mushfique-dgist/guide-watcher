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
    ".git",
    "node_modules",
    ".DS_Store",
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
