use crate::{
    completion,
    process_registry::{self, CancellationToken},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn failure(error: impl std::fmt::Display) -> String {
    format!("Could not save or read guide history: {error}")
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEvent {
    pub timestamp: u64,
    pub level: String,
    /// Plain-language line shown in the Overview timeline.
    pub message: String,
    /// The raw internal text this line was derived from, for the collapsible
    /// "Technical detail" under it. Absent when the message already is the raw text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// What the user should do about it, when there is something to do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_step: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRecord {
    pub job_id: String,
    pub source_path: String,
    pub source_paths: Vec<String>,
    pub filename: String,
    pub folder: String,
    pub output_path: Option<String>,
    pub prep_path: Option<String>,
    pub status: String,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    #[serde(default)]
    pub archived_at: Option<u64>,
    pub summary: String,
    pub events: Vec<HistoryEvent>,
    pub can_resume: bool,
    pub can_retry: bool,
    pub can_open_output: bool,
    pub recovery_note: Option<String>,
    pub options: serde_json::Value,
    pub parent_job_id: Option<String>,
    pub session_id: String,
    pub can_cancel: bool,
    pub cancellation_scope: String,
    /// Action guidance for the current terminal state, shown under the summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_step: Option<String>,
}
impl HistoryRecord {
    pub fn request(
        paths: Vec<String>,
        options: serde_json::Value,
        parent: Option<String>,
        session: String,
    ) -> Self {
        let source = paths.first().cloned().unwrap_or_default();
        let path = Path::new(&source);
        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let folder = path
            .parent()
            .and_then(Path::file_name)
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut result = Self {
            job_id: uuid::Uuid::new_v4().to_string(),
            source_path: source,
            source_paths: paths,
            filename,
            folder,
            output_path: None,
            prep_path: None,
            status: "starting".into(),
            started_at: now(),
            finished_at: None,
            archived_at: None,
            summary: String::new(),
            events: vec![],
            can_resume: false,
            can_retry: false,
            can_open_output: false,
            recovery_note: None,
            options,
            parent_job_id: parent,
            session_id: session,
            can_cancel: true,
            cancellation_scope: "batch".into(),
            next_step: None,
        };
        result.event("info", "Checking your selected files and settings.");
        result
    }
    pub fn event(&mut self, level: &str, message: &str) {
        self.event_rich(
            level,
            Humanized {
                message: message.to_string(),
                detail: None,
                next_step: None,
            },
        );
    }
    /// Record one timeline line. `message` is what the user reads; `detail` keeps the
    /// raw text behind a disclosure; `next_step` tells them what to do.
    pub fn event_rich(&mut self, level: &str, humanized: Humanized) {
        self.summary = humanized.message.chars().take(2000).collect();
        if self
            .events
            .last()
            .is_some_and(|e| e.message == self.summary)
        {
            return;
        }
        self.events.push(HistoryEvent {
            timestamp: now(),
            level: level.into(),
            message: self.summary.clone(),
            detail: humanized.detail,
            next_step: humanized.next_step,
        });
        if self.events.len() > 500 {
            self.events.remove(1);
        }
    }
    pub fn terminal(&mut self, status: &str, message: &str) {
        self.terminal_rich(
            status,
            Humanized {
                message: message.to_string(),
                detail: None,
                next_step: None,
            },
        );
    }
    pub fn terminal_rich(&mut self, status: &str, humanized: Humanized) {
        self.status = status.into();
        self.finished_at = Some(now());
        self.can_cancel = false;
        self.next_step = humanized.next_step.clone();
        self.event_rich(if status == "done" { "info" } else { "warning" }, humanized);
    }
}

/// One user-facing line derived from an internal message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Humanized {
    pub message: String,
    pub detail: Option<String>,
    pub next_step: Option<String>,
}

impl Humanized {
    fn plain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            detail: None,
            next_step: None,
        }
    }
    fn with_detail(message: impl Into<String>, raw: &str) -> Self {
        Self {
            message: message.into(),
            detail: Some(tidy_raw(raw)),
            next_step: None,
        }
    }
    fn action(message: impl Into<String>, next_step: impl Into<String>, raw: &str) -> Self {
        Self {
            message: message.into(),
            detail: Some(tidy_raw(raw)),
            next_step: Some(next_step.into()),
        }
    }
}

/// Strip Windows extended-length prefixes and the "Error:" label from raw text.
fn tidy_raw(raw: &str) -> String {
    raw.trim_start_matches("__stderr__")
        .trim()
        .trim_start_matches("Error:")
        .trim()
        .replace("\\\\?\\", "")
        .replace("\\?\\", "")
        .chars()
        .take(1400)
        .collect()
}

/// Pull the trailing "(model · effort)" or "(N words, ...)" annotation off a phase line.
fn phase_head(raw: &str) -> &str {
    raw.split(" (").next().unwrap_or(raw).trim()
}

/// First integer in `text`, if any.
fn first_number(text: &str) -> Option<usize> {
    let mut digits = String::new();
    for character in text.chars() {
        if character.is_ascii_digit() {
            digits.push(character);
        } else if !digits.is_empty() {
            break;
        }
    }
    digits.parse().ok()
}

/// Translate an internal phase line into what the Overview shows.
///
/// Unknown phrasing is passed through unchanged so nothing is ever hidden; the raw text
/// is attached as detail whenever the human line rewrites it.
/// The source-unit count the writing phase reports, when it reports one. Read from the whole
/// line, because `phase_head` cuts everything from the opening parenthesis.
fn source_units(raw: &str) -> Option<usize> {
    let (_, tail) = raw.trim_end().rsplit_once(", ")?;
    tail.strip_suffix(" source units)")?.parse().ok()
}

/// How long the draft pass takes, scaled by the lecture. Measured at roughly a third of a
/// minute per slide (a 115-slide lecture drafted in 37 minutes), and the deepening passes
/// that follow each cost about the same, so say so rather than quoting one figure for
/// every lecture.
fn writing_estimate(units: Option<usize>) -> String {
    let Some(units) = units.filter(|units| *units > 0) else {
        return "Step 2 of 2 · Writing the guide draft. This usually takes 15–25 minutes."
            .to_string();
    };
    let minutes = (units * 3 / 10).max(10);
    format!(
        "Step 2 of 2 · Writing the guide draft ({units} slides). This pass usually takes about {minutes} minutes, and up to three deepening passes of about the same length follow."
    )
}

pub fn humanize_phase(raw: &str) -> Humanized {
    let head = phase_head(raw);
    let lower = raw.to_lowercase();

    if head == "Checking the selected sources and generation requirements." {
        return Humanized::plain("Checking your selected files and settings.");
    }
    if head.starts_with("Source checks passed") {
        return Humanized::plain(
            "Your files look good. Waiting for this course's turn to start collecting context.",
        );
    }
    if head.starts_with("Phase 1/2 - inspecting source visuals") {
        let progress = head.rsplit(' ').next().unwrap_or("");
        return Humanized::with_detail(
            format!("Step 1 of 2 · Reading the slides and textbook pages (batch {progress})."),
            raw,
        );
    }
    if head.starts_with("Retrying this collection unit") {
        return Humanized::with_detail(
            "One batch of pages needed a second look, so a backup model is retrying it. This is routine and the run continues.",
            raw,
        );
    }
    if head.starts_with("Phase 1/2 - ranking visuals") {
        return Humanized::with_detail(
            "Step 1 of 2 · Choosing which figures are worth showing in the guide.",
            raw,
        );
    }
    if head == "Preparing source text locally" {
        return Humanized::plain("Step 1 of 2 · Extracting the lecture text.");
    }
    if head.starts_with("Phase 1/2 - context synthesis") {
        return Humanized::with_detail(
            "Step 1 of 2 · Researching the topic and assembling the source material. This is the longest part of step 1.",
            raw,
        );
    }
    if head == "Materializing immutable preflight source renders" {
        return Humanized::plain(
            "Freezing copies of your source files, so the guide is written from exactly these versions.",
        );
    }
    if head.starts_with("Phase 2/2 - Claude writing") || head.starts_with("Phase 2/2 - writing") {
        return Humanized::with_detail(writing_estimate(source_units(raw)), raw);
    }
    if head.starts_with("Claude writer completed") {
        return Humanized::with_detail(
            "The writer finished this pass. Checking the result before it is kept.",
            raw,
        );
    }
    if head.starts_with("Phase 2/2 - deepening pass") {
        let counter = head.trim_start_matches("Phase 2/2 - deepening pass ").trim();
        return Humanized::with_detail(
            format!("Step 2 of 2 · Deepening pass {counter} — adding worked examples and step-by-step traces."),
            raw,
        );
    }
    if head.starts_with("Deepening pass") && lower.contains("accepted") {
        return Humanized::with_detail("A deepening pass made the guide richer.", raw);
    }
    if head.starts_with("Deepening pass") {
        return Humanized::with_detail(
            "A deepening pass was set aside because it did not improve the guide safely; the previous version is kept.",
            raw,
        );
    }
    if head.starts_with("Handing the primary writer's style plan to the Codex fallback") {
        return Humanized::with_detail(
            "The backup writer receives the original writer's style plan, so the guide keeps one voice.",
            raw,
        );
    }
    if head.starts_with("Predecessor guide changed since the context was saved") {
        return Humanized::with_detail(
            "An earlier guide in this course changed after this context was saved; the run continues with the guides as they are now.",
            raw,
        );
    }
    if head.starts_with("Continuing the draft left by an earlier attempt") {
        return Humanized::with_detail(
            format!(
                "Picking up the {}-word draft the earlier attempt left, in the same style, instead of starting over.",
                first_number(head).unwrap_or(0)
            ),
            raw,
        );
    }
    if head.starts_with("Phase 2/2 - pedagogy pass") {
        let counter = head.trim_start_matches("Phase 2/2 - pedagogy pass ").trim();
        return Humanized::with_detail(
            format!("Step 2 of 2 · Pedagogy pass {counter} — defining terms before they are used, adding missing examples, and tying figures into the text."),
            raw,
        );
    }
    if head.starts_with("Pedagogy check found nothing to fix") {
        return Humanized::with_detail(
            "The teaching-quality check found nothing to fix, so the pedagogy pass was skipped.",
            raw,
        );
    }
    if head.starts_with("Pedagogy check found") {
        return Humanized::with_detail(
            format!(
                "The teaching-quality check found {} gap(s) to fix: missing examples, terms used before definition, or figures not tied into the text.",
                first_number(head).unwrap_or(0)
            ),
            raw,
        );
    }
    if head.starts_with("Pedagogy pass") && lower.contains("accepted") {
        return Humanized::with_detail("The pedagogy pass improved the explanations.", raw);
    }
    if head.starts_with("Pedagogy pass") {
        return Humanized::with_detail(
            "The pedagogy pass was set aside because it did not improve the guide safely; the previous version is kept.",
            raw,
        );
    }
    if head.starts_with("Depth target already met") {
        return Humanized::with_detail(
            "The guide already meets the depth target, so no further deepening is needed.",
            raw,
        );
    }
    if head.starts_with("Native verification pass") {
        return match first_number(head) {
            Some(1) | None => Humanized::with_detail("Checking the guide against the sources.", raw),
            Some(n) => Humanized::with_detail(
                format!("Checking the guide again after repairs (attempt {n})."),
                raw,
            ),
        };
    }
    if head.starts_with("Removed") && lower.contains("preamble") {
        return Humanized::with_detail("Cleaned up stray text that appeared before the guide title.", raw);
    }
    if head.starts_with("Restoring the saved source-bound visual teaching plan") {
        let count = first_number(raw)
            .map(|n| format!(" ({n} figures)"))
            .unwrap_or_default();
        return Humanized::with_detail(format!("Reusing the figure plan saved earlier{count}."), raw);
    }
    if head.starts_with("Restoring the retained legacy visual teaching checkpoint") {
        return Humanized::plain("Reusing the figure plan saved earlier.");
    }
    if head.starts_with("Ignoring") && lower.contains("source visual") {
        return Humanized::action(
            "Files were added to the course folder after this context was collected; they are left out of this guide.",
            "If you want the new files included, start a fresh guide instead of resuming.",
            raw,
        );
    }
    if head.starts_with("Excluding") && lower.contains("supplementary document") {
        return Humanized::with_detail(
            "Leaving out documents that were added to the folder after context collection.",
            raw,
        );
    }
    if head.starts_with("Leaving out") && lower.contains("saved figure") {
        return Humanized::with_detail(
            "Some figures in the saved plan came from a file this course no longer draws on, so they are left out. The rest of the saved work is used as it is.",
            raw,
        );
    }
    if head.starts_with("Re-keyed") {
        return Humanized::with_detail(
            "Adjusted saved figure references after files in the folder were reordered.",
            raw,
        );
    }
    if head.starts_with("Dropped") && lower.contains("textbook page") {
        return Humanized::with_detail("Trimmed textbook pages the saved plan did not cover.", raw);
    }
    if head.starts_with("Resuming the hybrid pipeline") {
        return Humanized::plain("Continuing from the saved context — skipping straight to writing.");
    }
    if head.starts_with("Saved context and bound sources passed validation") {
        return Humanized::plain("The saved context checked out. Starting the writer.");
    }
    if head.starts_with("Legacy prep has no saved visual plan") {
        return Humanized::with_detail(
            "This older saved context has no figure plan, so the figures are being inspected first.",
            raw,
        );
    }
    if head.starts_with("Codex collection unavailable") {
        return Humanized::with_detail(
            "The primary model was not available for context collection; a backup model has taken over.",
            raw,
        );
    }
    if head.starts_with("Claude unavailable after ordered fallbacks") {
        return Humanized::with_detail(
            "The writing model was not available; the backup writer is being used.",
            raw,
        );
    }
    if head.starts_with("Claude became unavailable during repair") {
        return Humanized::with_detail(
            "The writing model became unavailable during fixes; the backup is finishing them.",
            raw,
        );
    }
    Humanized::plain(raw)
}

/// Translate an internal error into a message plus a concrete next step.
pub fn humanize_error(raw: &str) -> Humanized {
    let text = tidy_raw(raw);
    let lower = text.to_lowercase();

    if lower.contains("refusing to overwrite existing prep file") {
        return Humanized::action(
            "A saved context file for this lecture already exists, so a fresh run was not started.",
            "Open this lecture in History and click Resume writing to continue from it. To start over instead, delete the .prep.md file next to the lecture first.",
            &text,
        );
    }
    if lower.contains("no longer present in the course folder")
        || lower.contains("collect fresh context")
        || lower.contains("contain exactly") && lower.contains("observations")
    {
        return Humanized::action(
            "A file the saved context depended on has been removed or changed.",
            "Click Retry from sources to collect context again from the files that are there now.",
            &text,
        );
    }
    if lower.contains("native verification still failed") {
        let attempts = first_number(&text)
            .map(|n| format!(" after {n} repair attempts"))
            .unwrap_or_default();
        return Humanized::action(
            format!("The written guide did not pass the app's checks{attempts}."),
            "Click Resume writing to run the writing step again from the saved context — your context collection is kept. The exact check that failed is in the technical detail.",
            &text,
        );
    }
    if lower.contains("without an eligible provider fallback") {
        return Humanized::action(
            "The writing model stopped with an error the app could not recover from on its own.",
            "Check the model provider's sign-in and usage limits, then click Resume writing.",
            &text,
        );
    }
    if lower.contains("quota")
        || lower.contains("rate limit")
        || lower.contains("usage limit")
        || lower.contains("credit balance")
        || lower.contains("overloaded")
    {
        return Humanized::action(
            "The model provider reached a usage limit.",
            "Wait for the limit to reset, then click Resume writing if this run has saved context, or Retry from sources if not.",
            &text,
        );
    }
    if lower.contains("not logged in")
        || lower.contains("login")
        || lower.contains("sign in")
        || lower.contains("authentication")
        || lower.contains("http 401")
    {
        return Humanized::action(
            "The model provider needs you to sign in.",
            "Sign in with the provider's command-line tool, then click Resume writing or Retry from sources.",
            &text,
        );
    }
    if lower.contains("permission denied") || lower.contains("access denied") {
        return Humanized::action(
            "The app could not read or write a required file.",
            "Check that the course folder is readable and writable, then retry.",
            &text,
        );
    }
    if lower.contains("already generating course") || lower.contains("already using") {
        return Humanized::action(
            "Another guide in this course is already running.",
            "Wait for it to finish; this one can run afterwards.",
            &text,
        );
    }
    if lower.contains("predecessor") {
        return Humanized::action(
            "An earlier guide in this course is missing, incomplete, or has changed.",
            "Finish or restore the earlier guide, then retry this one.",
            &text,
        );
    }
    if lower.contains("source integrity digest") || lower.contains("generation contract no longer matches") {
        return Humanized::action(
            "The saved context no longer matches the source files.",
            "Click Retry from sources to collect fresh context.",
            &text,
        );
    }
    if lower.contains("phase 1 failed") {
        return Humanized::action(
            "Context collection could not finish.",
            "Click Retry from sources. A diagnostic file was saved next to the lecture for reference.",
            &text,
        );
    }
    if lower.contains("cancel") {
        return Humanized::action(
            "This run was stopped.",
            "Click Resume writing to continue from saved context, or Retry from sources to start over.",
            &text,
        );
    }
    if lower.contains("not found") || lower.contains("no such file") {
        return Humanized::action(
            "A required file or program could not be found.",
            "Check that the source file still exists and that the model provider's tool is installed, then retry.",
            &text,
        );
    }
    Humanized::action(
        "This step could not finish.",
        "Read the technical detail below. If it mentions the model provider, fix that and click Resume writing; otherwise click Retry from sources.",
        &text,
    )
}
pub fn active(status: &str) -> bool {
    matches!(status, "starting" | "queued" | "working")
}
pub struct HistoryStore {
    connection: Mutex<Connection>,
    root: PathBuf,
    pub session_id: String,
    _session_lock: File,
    tokens: Mutex<HashMap<String, CancellationToken>>,
    actions: Mutex<std::collections::HashSet<String>>,
}
impl HistoryStore {
    pub fn open(root: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(root).map_err(failure)?;
        let session_id = uuid::Uuid::new_v4().to_string();
        let session_lock = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(root.join(format!("{session_id}.session")))
            .map_err(failure)?;
        session_lock.lock().map_err(failure)?;
        let connection = Connection::open(root.join("history.sqlite3")).map_err(failure)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(failure)?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(failure)?;
        if version > 2 {
            return Err(
                "This history database was created by a newer app version. It was left unchanged."
                    .into(),
            );
        }
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS attempts(id TEXT PRIMARY KEY NOT NULL, record TEXT NOT NULL); CREATE TABLE IF NOT EXISTS deleted_history(key_hash TEXT PRIMARY KEY NOT NULL); PRAGMA user_version=2; COMMIT;").map_err(failure)?;
        Ok(Self {
            connection: Mutex::new(connection),
            root: root.to_owned(),
            session_id,
            _session_lock: session_lock,
            tokens: Mutex::new(HashMap::new()),
            actions: Mutex::new(std::collections::HashSet::new()),
        })
    }
    pub fn insert(
        &self,
        record: &HistoryRecord,
        token: Option<CancellationToken>,
    ) -> Result<(), String> {
        let text = serde_json::to_string(record).map_err(failure)?;
        self.connection
            .lock()
            .map_err(failure)?
            .execute(
                "INSERT INTO attempts(id,record) VALUES(?1,?2)",
                params![record.job_id, text],
            )
            .map_err(failure)?;
        if let Some(token) = token {
            self.tokens
                .lock()
                .map_err(failure)?
                .insert(record.job_id.clone(), token);
        }
        Ok(())
    }
    pub fn get(&self, id: &str) -> Result<HistoryRecord, String> {
        let text: Option<String> = self
            .connection
            .lock()
            .map_err(failure)?
            .query_row("SELECT record FROM attempts WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .optional()
            .map_err(failure)?;
        serde_json::from_str(&text.ok_or("This run is not present in history.")?).map_err(failure)
    }
    pub fn update(&self, id: &str, change: impl FnOnce(&mut HistoryRecord)) -> Result<(), String> {
        let mut conn = self.connection.lock().map_err(failure)?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(failure)?;
        let text: String = tx
            .query_row("SELECT record FROM attempts WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .map_err(failure)?;
        let mut record: HistoryRecord = serde_json::from_str(&text).map_err(failure)?;
        change(&mut record);
        tx.execute(
            "UPDATE attempts SET record=?2 WHERE id=?1",
            params![id, serde_json::to_string(&record).map_err(failure)?],
        )
        .map_err(failure)?;
        tx.commit().map_err(failure)
    }
    pub fn replace_request(
        &self,
        id: &str,
        records: &[HistoryRecord],
        token: CancellationToken,
    ) -> Result<(), String> {
        let mut conn = self.connection.lock().map_err(failure)?;
        let tx = conn.transaction().map_err(failure)?;
        for record in records {
            tx.execute(
                "INSERT INTO attempts(id,record) VALUES(?1,?2)",
                params![
                    record.job_id,
                    serde_json::to_string(record).map_err(failure)?
                ],
            )
            .map_err(failure)?;
        }
        tx.execute("DELETE FROM attempts WHERE id=?1", [id])
            .map_err(failure)?;
        tx.commit().map_err(failure)?;
        let mut tokens = self.tokens.lock().map_err(failure)?;
        tokens.remove(id);
        for record in records {
            tokens.insert(record.job_id.clone(), token);
        }
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<HistoryRecord>, String> {
        let mut records = {
            let conn = self.connection.lock().map_err(failure)?;
            let mut stmt = conn
                .prepare("SELECT record FROM attempts")
                .map_err(failure)?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(failure)?;
            let mut records = Vec::new();
            for row in rows {
                records.push(
                    serde_json::from_str::<HistoryRecord>(&row.map_err(failure)?)
                        .map_err(failure)?,
                );
            }
            records
        };
        for record in &mut records {
            if active(&record.status)
                && record.session_id != self.session_id
                && !self.session_alive(&record.session_id)?
            {
                self.update(&record.job_id,|r|{if active(&r.status){r.terminal("interrupted","The app stopped before this attempt finished. Its original detailed output is unavailable.");}})?;
                *record = self.get(&record.job_id)?;
            }
            self.capabilities(record);
        }
        records.sort_by_key(|r| std::cmp::Reverse(r.started_at));
        Ok(records)
    }
    fn session_alive(&self, id: &str) -> Result<bool, String> {
        if uuid::Uuid::parse_str(id).is_err() {
            return Ok(false);
        }
        let file = match OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.root.join(format!("{id}.session")))
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(failure(e)),
        };
        match file.try_lock() {
            Ok(()) => Ok(false),
            Err(std::fs::TryLockError::WouldBlock) => Ok(true),
            Err(std::fs::TryLockError::Error(e)) => Err(failure(e)),
        }
    }
    fn capabilities(&self, record: &mut HistoryRecord) {
        record.can_open_output = record
            .output_path
            .as_ref()
            .is_some_and(|p| completion::is_usable_guide(Path::new(p)));
        record.can_cancel = active(&record.status) && record.session_id == self.session_id;
        record.can_resume = false;
        record.can_retry = false;
        if active(&record.status) {
            return;
        }
        if let Some(prep) = &record.prep_path {
            if Path::new(prep).is_file() {
                match crate::codex::load_resume_prep(Path::new(prep)).and_then(crate::codex::validate_loaded_resume_prep){Ok(_)=>{record.can_resume= !record.can_open_output;record.recovery_note=Some("Saved context from the earlier steps is available. Resume writing continues from it and skips context collection, saving about 20 minutes.".into());},Err(_)=>record.recovery_note=Some("Saved context exists but no longer matches the source files, so it cannot be resumed. Use Retry from sources to start again.".into())}
            }
        }
        record.can_retry = !record.can_open_output
            && !record.can_resume
            && record
                .prep_path
                .as_ref()
                .is_none_or(|p| !Path::new(p).exists())
            && !record.source_paths.is_empty()
            && record
                .source_paths
                .iter()
                .all(|p| Path::new(p).is_file() && !crate::job_events::is_prep_file(Path::new(p)))
            && record.options.is_object()
            && record
                .output_path
                .as_ref()
                .is_none_or(|p| !Path::new(p).exists());
        if record.status == "done" && !record.can_open_output {
            record.recovery_note=Some("This run succeeded at the time, but its published files are now missing or changed (moved, edited, or deleted), so the app can no longer open them as a verified guide. If you edited the guide on purpose, that is expected.".into());
        }
    }
    pub fn archive(&self, id: &str, archived: bool) -> Result<(), String> {
        self.manage_record(
            id,
            if archived {
                HistoryChange::Archive
            } else {
                HistoryChange::Restore
            },
        )
    }
    pub fn delete(&self, id: &str) -> Result<(), String> {
        self.manage_record(id, HistoryChange::Delete)
    }
    fn manage_record(&self, id: &str, change: HistoryChange) -> Result<(), String> {
        if id.trim().is_empty() {
            return Err("Choose a history record first.".into());
        }
        let mut conn = self.connection.lock().map_err(failure)?;
        let actions = self.actions.lock().map_err(failure)?;
        let tokens = self.tokens.lock().map_err(failure)?;
        if actions.contains(id) || tokens.contains_key(id) {
            return Err(
                "This attempt is still in use. Wait for its current work to finish.".into(),
            );
        }
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(failure)?;
        let text: Option<String> = tx
            .query_row("SELECT record FROM attempts WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(failure)?;
        let Some(text) = text else {
            if matches!(change, HistoryChange::Delete)
                && tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM deleted_history WHERE key_hash=?1)",
                        [history_key("job", id)],
                        |r| r.get::<_, bool>(0),
                    )
                    .map_err(failure)?
            {
                return Ok(());
            }
            return Err("This run is not present in history.".into());
        };
        let mut record: HistoryRecord = serde_json::from_str(&text).map_err(failure)?;
        if active(&record.status) {
            return Err("Active guides stay in Recents. Wait until this attempt finishes.".into());
        }
        match change {
            HistoryChange::Archive => {
                record.archived_at.get_or_insert_with(now);
            }
            HistoryChange::Restore => {
                record.archived_at = None;
            }
            HistoryChange::Delete => {
                if record.archived_at.is_none() {
                    return Err(
                        "Remove this attempt from Recents before permanently deleting its history."
                            .into(),
                    );
                }
                let mut keys = vec![history_key("job", id)];
                if record.job_id.starts_with("import-") {
                    if let Some(origin) = import_origin(&record) {
                        keys.push(history_key("origin", &artifact_identity(origin)));
                    }
                } else {
                    for path in [&record.output_path, &record.prep_path]
                        .into_iter()
                        .flatten()
                    {
                        keys.push(history_key("artifact", &artifact_identity(path)));
                    }
                }
                for key in keys {
                    tx.execute(
                        "INSERT OR IGNORE INTO deleted_history(key_hash) VALUES(?1)",
                        [key],
                    )
                    .map_err(failure)?;
                }
                tx.execute("DELETE FROM attempts WHERE id=?1", [id])
                    .map_err(failure)?;
                return tx.commit().map_err(failure);
            }
        }
        tx.execute(
            "UPDATE attempts SET record=?2 WHERE id=?1",
            params![id, serde_json::to_string(&record).map_err(failure)?],
        )
        .map_err(failure)?;
        tx.commit().map_err(failure)
    }
    pub fn clear_recents(&self) -> Result<usize, String> {
        let mut conn = self.connection.lock().map_err(failure)?;
        let actions = self.actions.lock().map_err(failure)?;
        let tokens = self.tokens.lock().map_err(failure)?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(failure)?;
        let records = {
            let mut statement = tx.prepare("SELECT record FROM attempts").map_err(failure)?;
            let rows = statement
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(failure)?;
            let mut records = Vec::new();
            for row in rows {
                records.push(
                    serde_json::from_str::<HistoryRecord>(&row.map_err(failure)?)
                        .map_err(failure)?,
                );
            }
            records
        };
        let mut count = 0;
        let timestamp = now();
        for mut record in records {
            if active(&record.status)
                || record.archived_at.is_some()
                || actions.contains(&record.job_id)
                || tokens.contains_key(&record.job_id)
            {
                continue;
            }
            record.archived_at = Some(timestamp);
            count += tx
                .execute(
                    "UPDATE attempts SET record=?2 WHERE id=?1",
                    params![
                        record.job_id,
                        serde_json::to_string(&record).map_err(failure)?
                    ],
                )
                .map_err(failure)?;
        }
        tx.commit().map_err(failure)?;
        Ok(count)
    }
    pub fn cancel(&self, id: &str) -> Result<(), String> {
        let token = *self
            .tokens
            .lock()
            .map_err(failure)?
            .get(id)
            .ok_or("This run is no longer active in this app instance.")?;
        // A cancel the user clicked is not something to notify them about afterwards.
        crate::attention::user_stopped_jobs();
        process_registry::cancel(token);
        Ok(())
    }
    pub fn release(&self, ids: &[String]) {
        if let Ok(mut tokens) = self.tokens.lock() {
            for id in ids {
                tokens.remove(id);
            }
        }
    }
    pub fn claim(&self, id: &str) -> Result<(), String> {
        if !self.actions.lock().map_err(failure)?.insert(id.into()) {
            return Err("A recovery action for this run is already being prepared.".into());
        }
        Ok(())
    }
    pub fn unclaim(&self, id: &str) {
        if let Ok(mut actions) = self.actions.lock() {
            actions.remove(id);
        }
    }
    pub fn progress(&self, id: &str, line: &str) -> Result<(), String> {
        if let Some(phase) = line.strip_prefix("__phase__") {
            let humanized = humanize_phase(phase);
            return self.update(id, |r| {
                if active(&r.status) {
                    r.status = "working".into();
                    r.event_rich("info", humanized.clone());
                }
            });
        }
        let lower = line.to_lowercase();
        if (line.starts_with("Error:") || line.starts_with("__stderr__"))
            && (lower.contains("error") || lower.contains("fail") || lower.contains("cancel"))
        {
            let humanized = humanize_error(line);
            self.update(id, |r| {
                if active(&r.status) {
                    r.event_rich("error", humanized.clone());
                }
            })?;
        }
        Ok(())
    }
    pub fn blocked(&self, id: &str, token: CancellationToken) -> Result<(), String> {
        self.update(id, |r| { if active(&r.status) {
            if process_registry::is_cancelled(token) {
                r.terminal_rich("cancelled", Humanized{message:"This batch was stopped before this guide could start.".into(),detail:None,next_step:Some("Start it again from its source file when you are ready.".into())});
            } else {
                r.terminal_rich("blocked", Humanized{message:"An earlier guide in this course did not finish, so this one could not start.".into(),detail:None,next_step:Some("Fix or retry the earlier guide first; guides in a course are written in order.".into())});
            }
        }})
    }
    pub fn finish(&self, id: &str, exit: i32, token: CancellationToken) -> Result<(), String> {
        let record = self.get(id)?;
        let verified = exit == 0
            && record
                .output_path
                .as_ref()
                .is_some_and(|p| completion::has_valid_bundle_receipt(Path::new(p)));
        self.update(id,|r|{if !active(&r.status){return;}
if verified{r.terminal_rich("done",Humanized{message:"Your guide is ready. It passed every check and was published with its figures.".into(),detail:None,next_step:Some("Click Open guide to read it.".into())});}else if process_registry::is_cancelled(token){r.terminal_rich("cancelled",Humanized{message:"This run was stopped.".into(),detail:None,next_step:Some("Click Resume writing to continue from saved context, or Retry from sources to start over.".into())});}else{let last=r.events.iter().rev().find(|e|e.level=="error").cloned();let humanized=match last{Some(event)=>Humanized{message:event.message,detail:event.detail,next_step:event.next_step},None=>Humanized{message:"This attempt ended without a published guide.".into(),detail:None,next_step:Some("Check the timeline for the last step that completed, then click Resume writing or Retry from sources.".into())}};r.terminal_rich("failed",humanized);}})
    }
}
/// Single-string form of `humanize_error`, for callers that record one summary line.
/// The raw text stays reachable through the timeline's detail; this string leads with
/// the plain-language reason and what to do about it.
/// Message-only rendering for places that cannot show a detail pane. Records should use
/// `terminal_rich(status, humanize_error(raw))` so the raw text survives as the detail.
#[cfg(test)]
pub fn plain_error(text: &str) -> String {
    let humanized = humanize_error(text);
    match humanized.next_step {
        Some(step) => format!("{} {step}", humanized.message),
        None => humanized.message,
    }
}

pub struct DesktopProgress {
    pub app: tauri::AppHandle,
    pub store: std::sync::Arc<HistoryStore>,
    pub token: CancellationToken,
}
impl DesktopProgress {
    fn saved(&self, result: Result<(), String>) {
        if let Err(error) = result {
            use tauri::Emitter;
            process_registry::cancel(self.token);
            let _ = self.app.emit("history-error", error);
        }
    }
}
impl crate::job_events::ProgressSink for DesktopProgress {
    fn blocked(&self, id: &str, message: &str) {
        self.saved(self.store.blocked(id, self.token));
        crate::job_events::ProgressSink::output(&self.app, id, message);
        self.done(id, -1, false);
    }
    fn started(&self, id: &str) {
        self.saved(self.store.update(id, |r| {
            if active(&r.status) {
                r.status = "working".into();
            }
        }));
        crate::job_events::ProgressSink::started(&self.app, id);
    }
    fn output(&self, id: &str, line: &str) {
        self.saved(self.store.progress(id, line));
        crate::job_events::ProgressSink::output(&self.app, id, line);
    }
    fn done(&self, id: &str, exit: i32, exists: bool) {
        self.saved(self.store.finish(id, exit, self.token));
        use tauri::Emitter;
        let status = self.store.get(id).ok().map(|r| r.status);
        let _=self.app.emit("job-done",serde_json::json!({"job_id":id,"exit_code":exit,"output_file_exists":exists,"status":status}));
    }
}

impl HistoryStore {
    fn insert_import_if_untracked(&self, record: &HistoryRecord) -> Result<usize, String> {
        let mut conn = self.connection.lock().map_err(failure)?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(failure)?;
        if import_is_deleted(&tx, record)? {
            return Ok(0);
        }
        let tracked = {
            let mut stmt = tx
                .prepare("SELECT record FROM attempts WHERE id NOT LIKE 'import-%'")
                .map_err(failure)?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(failure)?;
            let mut found = false;
            for row in rows {
                let existing: HistoryRecord =
                    serde_json::from_str(&row.map_err(failure)?).map_err(failure)?;
                for (left, right) in [
                    (&existing.output_path, &record.output_path),
                    (&existing.prep_path, &record.prep_path),
                ] {
                    if let (Some(left), Some(right)) = (left, right) {
                        if artifact_identity(left) == artifact_identity(right) {
                            found = true;
                        }
                    }
                }
            }
            found
        };
        let changed = if tracked {
            0
        } else {
            tx.execute(
                "INSERT OR IGNORE INTO attempts(id,record) VALUES(?1,?2)",
                params![
                    record.job_id,
                    serde_json::to_string(record).map_err(failure)?
                ],
            )
            .map_err(failure)?
        };
        tx.commit().map_err(failure)?;
        Ok(changed)
    }
    pub fn import_existing(&self, root: &Path) -> Result<usize, String> {
        use sha2::{Digest, Sha256};
        let mut imported = 0;
        for entry in walkdir::WalkDir::new(root)
            .max_depth(3)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                !e.file_name().to_string_lossy().ends_with(".gwfailed") || e.depth() <= 2
            })
        {
            let entry = entry.map_err(failure)?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy();
            let mut record = None;
            if entry.file_type().is_file() && name.ends_with(".prep.md") {
                if let Ok(packet) = crate::codex::load_resume_prep(path)
                    .and_then(crate::codex::validate_loaded_resume_prep)
                {
                    let paths = packet
                        .plan
                        .source_paths
                        .iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect();
                    let mut r =
                        HistoryRecord::request(paths, serde_json::Value::Null, None, String::new());
                    r.output_path = Some(packet.plan.output_path.to_string_lossy().into_owned());
                    r.prep_path = Some(path.to_string_lossy().into_owned());
                    r.terminal("interrupted","Imported saved context from an earlier run. The original run log was not recorded; writing can resume after validation.");
                    record = Some(r);
                }
            } else if entry.file_type().is_file()
                && name.ends_with("_Guide.md")
                && completion::is_usable_guide(path)
            {
                let mut r =
                    HistoryRecord::request(vec![], serde_json::Value::Null, None, String::new());
                r.source_path = path.to_string_lossy().into_owned();
                r.filename = name.into_owned();
                r.output_path = Some(path.to_string_lossy().into_owned());
                r.terminal("done",if completion::has_valid_bundle_receipt(path){"Imported a verified published guide. Its original run timeline was not recorded."}else{"Imported a legacy guide with a valid guide-only receipt. This does not establish modern bundle verification."});
                record = Some(r);
            } else if entry.file_type().is_dir() && name.ends_with(".gwfailed") {
                if let Some(output) = legacy_workspace_output(path) {
                    let mut r = HistoryRecord::request(
                        vec![],
                        serde_json::Value::Null,
                        None,
                        String::new(),
                    );
                    r.filename = output.clone();
                    r.source_path = path.to_string_lossy().into_owned();
                    r.output_path = path
                        .parent()
                        .map(|p| p.join(&output).to_string_lossy().into_owned());
                    r.prep_path = r.output_path.as_ref().map(|p| {
                        Path::new(p)
                            .with_extension("prep.md")
                            .to_string_lossy()
                            .into_owned()
                    });
                    r.terminal("interrupted","Imported an unfinished workspace retained by an earlier version. The original outcome and failure reason were not recorded; no success is inferred.");
                    record = Some(r);
                }
            }
            if let Some(mut r) = record {
                r.job_id = format!(
                    "import-{:x}",
                    Sha256::digest(
                        path.to_string_lossy()
                            .replace('\\', "/")
                            .to_lowercase()
                            .as_bytes()
                    )
                );
                r.events.clear();
                r.started_at = entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|t| t.as_millis() as u64)
                    .unwrap_or(0);
                r.finished_at = None;
                r.can_cancel = false;
                r.folder = path
                    .parent()
                    .and_then(Path::file_name)
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                r.recovery_note=Some("Recovered from files on disk. The date is the artifact modification time, not a recorded start time.".into());
                let changed = self.insert_import_if_untracked(&r)?;
                imported += changed;
            }
        }
        Ok(imported)
    }
}

enum HistoryChange {
    Archive,
    Restore,
    Delete,
}
fn history_key(kind: &str, value: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{kind}:{:x}", Sha256::digest(value.as_bytes()))
}
fn import_origin(record: &HistoryRecord) -> Option<&str> {
    if record.source_path.ends_with(".gwfailed") {
        Some(&record.source_path)
    } else if record.status == "done" {
        record.output_path.as_deref()
    } else {
        record.prep_path.as_deref()
    }
}
fn import_is_deleted(
    tx: &rusqlite::Transaction<'_>,
    record: &HistoryRecord,
) -> Result<bool, String> {
    let mut keys = vec![history_key("job", &record.job_id)];
    if let Some(origin) = import_origin(record) {
        keys.push(history_key("origin", &artifact_identity(origin)));
    }
    for path in [&record.output_path, &record.prep_path]
        .into_iter()
        .flatten()
    {
        keys.push(history_key("artifact", &artifact_identity(path)));
    }
    for key in keys {
        if tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM deleted_history WHERE key_hash=?1)",
                [key],
                |r| r.get::<_, bool>(0),
            )
            .map_err(failure)?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn artifact_identity(path: &str) -> String {
    let path = path.replace('\\', "/");
    #[cfg(windows)]
    {
        path.strip_prefix("//?/").unwrap_or(&path).to_lowercase()
    }
    #[cfg(not(windows))]
    {
        path
    }
}

fn legacy_workspace_output(path: &Path) -> Option<String> {
    let marker = path.join("workspace.json");
    if marker.metadata().ok()?.len() >= 8192 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(marker).ok()?).ok()?;
    if value["owner"] != "guide-watcher" || value["schema_version"] != 1 {
        return None;
    }
    let transaction = value["transaction_id"].as_str()?;
    uuid::Uuid::parse_str(transaction).ok()?;
    let suffix = format!(".{transaction}.gwfailed");
    let name = path.file_name()?.to_str()?;
    let output = name.strip_prefix('.')?.strip_suffix(&suffix)?;
    output.ends_with(".md").then(|| output.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                "guide-watcher-history-test-{}",
                uuid::Uuid::new_v4()
            )))
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn record(store: &HistoryStore) -> HistoryRecord {
        HistoryRecord::request(
            vec!["missing.pdf".into()],
            serde_json::json!({}),
            None,
            store.session_id.clone(),
        )
    }

    #[test]
    fn overview_lines_are_plain_language_with_raw_text_kept_as_detail() {
        // Every phase string the app emits must become something a student can read,
        // while the raw text stays reachable. Unknown phrasing must pass through untouched.
        let cases: &[(&str, &str)] = &[
            ("Phase 1/2 - inspecting source visuals 3/5 (gpt-5.6-luna · medium)", "Reading the slides and textbook pages (batch 3/5)"),
            ("Retrying this collection unit with gpt-5.6-terra · medium after gpt-5.6-luna · medium returned output rejected by the collection contract: source-vision observation 3 does not match its app-owned image binding", "backup model is retrying"),
            ("Phase 1/2 - context synthesis (gpt-5.6-luna · medium)", "Researching the topic"),
            ("Phase 2/2 - Claude writing (claude-opus-5 · high)", "Writing the guide draft"),
            ("Phase 2/2 - deepening pass 1/2 (9800 words, target 18000)", "Deepening pass 1/2"),
            ("Deepening pass 1 accepted: 9800 -> 14200 words", "made the guide richer"),
            ("Deepening pass 2 rejected: the enriched guide changed its major headings", "set aside"),
            ("Native verification pass 1", "Checking the guide against the sources"),
            ("Native verification pass 3", "attempt 3"),
            ("Materializing immutable preflight source renders", "Freezing copies"),
            ("Restoring the saved source-bound visual teaching plan (52 visuals)", "52 figures"),
            ("Ignoring 3 source visual(s) discovered after this context was collected; they are not part of the saved plan: textbook-005-page-0010", "left out of this guide"),
            ("Removed 71 bytes of provider preamble before the guide title", "stray text"),
        ];
        for (raw, expected) in cases {
            let humanized = humanize_phase(raw);
            assert!(humanized.message.contains(expected), "{raw:?} -> {:?}", humanized.message);
            assert!(!humanized.message.contains("app-owned"), "{:?}", humanized.message);
            assert!(!humanized.message.contains("gpt-5.6"), "{:?}", humanized.message);
            if humanized.message != *raw && raw.chars().any(|c| c.is_ascii_digit()) {
                assert!(humanized.detail.is_some(), "rewritten lines with data keep the raw text: {raw:?}");
            }
        }
        // The added-files line is the one phase that also tells the user what to do.
        let ignored = humanize_phase("Ignoring 3 source visual(s) discovered after this context was collected; they are not part of the saved plan: x");
        assert!(ignored.next_step.as_deref().unwrap_or("").contains("fresh guide"));
        // Unknown phrasing is never hidden.
        assert_eq!(humanize_phase("Some brand new phase").message, "Some brand new phase");
        assert!(humanize_phase("Some brand new phase").detail.is_none());
    }

    #[test]
    fn the_writing_estimate_scales_with_the_lecture_and_never_hides_the_passes() {
        // The lecture that prompted this: 115 slides drafted in 37 minutes.
        let big = humanize_phase("Phase 2/2 - Claude writing (claude-fable-5-1 \u{b7} xhigh, 115 source units)");
        assert!(big.message.contains("115 slides"), "{}", big.message);
        assert!(big.message.contains("about 34 minutes"), "{}", big.message);
        assert!(big.message.contains("deepening passes"), "{}", big.message);
        assert!(!big.message.contains("fable"), "{}", big.message);
        assert!(big.detail.is_some());

        let small = humanize_phase("Phase 2/2 - Claude writing (claude-fable-5-1 \u{b7} xhigh, 38 source units)");
        assert!(small.message.contains("38 slides"), "{}", small.message);
        assert!(small.message.contains("about 11 minutes"), "{}", small.message);

        // A short lecture still gets a floor, and an older line without the count still reads.
        assert!(writing_estimate(Some(4)).contains("about 10 minutes"));
        assert!(writing_estimate(Some(0)).contains("15\u{2013}25 minutes"));
        let legacy = humanize_phase("Phase 2/2 - Claude writing (claude-fable-5-1 \u{b7} xhigh)");
        assert!(legacy.message.contains("15\u{2013}25 minutes"), "{}", legacy.message);
    }

    #[test]
    fn errors_get_a_concrete_next_step_and_lose_raw_path_noise() {
        let verification = humanize_error("Error: native verification still failed after 3 repair attempt(s): [FAIL] MODEL ARTIFACT VALIDATION: model output must start with the guide's level-1 Markdown title. Raw provider response retained at \\\\?\\C:\\Users\\x\\raw-responses\\response-004.md");
        assert!(verification.message.contains("did not pass the app's checks after 3 repair attempts"), "{verification:?}");
        assert!(verification.next_step.as_deref().unwrap().contains("Resume writing"));
        let detail = verification.detail.as_deref().unwrap();
        assert!(detail.contains("MODEL ARTIFACT VALIDATION"), "raw category kept: {detail}");
        assert!(!detail.contains("\\\\?\\"), "extended-length prefix stripped: {detail}");
        assert!(!detail.starts_with("Error:"));

        let stale = humanize_error("Error: source-vision report must use schema version 1 and contain exactly 55 observations");
        assert!(stale.message.contains("removed or changed"), "{stale:?}");
        assert!(stale.next_step.as_deref().unwrap().contains("Retry from sources"));

        let prep = humanize_error("Error: refusing to overwrite existing prep file: C:/course/L_Guide.prep.md");
        assert!(prep.next_step.as_deref().unwrap().contains("Resume writing"));

        let quota = humanize_error("Error: Claude exited with status 1: Claude usage limit reached");
        assert!(quota.message.contains("usage limit"));

        let unknown = humanize_error("Error: something nobody anticipated");
        assert_eq!(unknown.message, "This step could not finish.");
        assert!(unknown.next_step.is_some());
        assert_eq!(unknown.detail.as_deref(), Some("something nobody anticipated"));

        // The one-string form leads with the reason and the action, never a raw dump.
        let single = plain_error("Error: native verification still failed after 3 repair attempt(s): [FAIL] X");
        assert!(single.starts_with("The written guide did not pass"));
        assert!(single.contains("Resume writing"));
        assert!(!single.contains("Details:"));
    }

    #[test]
    fn history_records_without_the_new_fields_still_load() {
        // Records persisted before detail/next_step existed must deserialize unchanged.
        let legacy = serde_json::json!({
            "timestamp": 1, "level": "info", "message": "Checking the selected sources and generation requirements."
        });
        let event: HistoryEvent = serde_json::from_value(legacy).unwrap();
        assert_eq!(event.detail, None);
        assert_eq!(event.next_step, None);
        // And the new optional fields are omitted when empty, so old readers see the old shape.
        let text = serde_json::to_string(&event).unwrap();
        assert!(!text.contains("detail"));
        assert!(!text.contains("nextStep"));
        let rich = HistoryEvent { detail: Some("raw".into()), next_step: Some("do x".into()), ..event };
        let text = serde_json::to_string(&rich).unwrap();
        assert!(text.contains("\"detail\":\"raw\"") && text.contains("\"nextStep\":\"do x\""));
    }

    #[test]
    fn preflight_failures_keep_the_raw_reason_as_detail() {
        let raw = "transcript changed since it was collected (manifest sha256 differs): \\\\?\\C:\\course\\_supplementary\\x.txt";
        let humanized = humanize_error(raw);
        let detail = humanized.detail.expect("raw preflight text is kept as detail");
        assert!(detail.contains("transcript changed since it was collected"));
        assert!(!detail.contains("\\\\?\\"), "extended-length prefix is tidied: {detail}");
        assert!(humanized.next_step.is_some());
        let mut record = HistoryRecord::request(vec!["C:/course/L4.pdf".into()], serde_json::json!({}), None, "s".into());
        record.terminal_rich("failed", humanize_error(raw));
        let last = record.events.last().unwrap();
        assert_eq!(last.level, "warning");
        assert!(last.detail.as_deref().unwrap_or("").contains("manifest sha256 differs"));
    }

    #[test]
    fn terminal_states_carry_guidance_for_the_failure_card() {
        let mut record = HistoryRecord::request(vec!["C:/course/a.pdf".into()], serde_json::json!({}), None, "s".into());
        assert_eq!(record.events[0].message, "Checking your selected files and settings.");
        record.event_rich("error", humanize_error("Error: native verification still failed after 3 repair attempt(s): [FAIL] X"));
        record.terminal_rich("failed", Humanized { message: "m".into(), detail: None, next_step: Some("do this".into()) });
        assert_eq!(record.status, "failed");
        assert_eq!(record.next_step.as_deref(), Some("do this"));
        assert!(record.events.iter().any(|e| e.level == "error" && e.next_step.is_some()));
    }

    #[test]
    fn archive_restore_and_clear_survive_restart_without_changing_outcomes() {
        let temp = Temp::new();
        let id;
        {
            let store = HistoryStore::open(&temp.0).unwrap();
            let mut r = record(&store);
            r.terminal("failed", "Provider unavailable");
            id = r.job_id.clone();
            store.insert(&r, None).unwrap();
            assert_eq!(store.clear_recents().unwrap(), 1);
            let saved = store.get(&id).unwrap();
            assert!(saved.archived_at.is_some());
            assert_eq!(saved.summary, r.summary);
            assert_eq!(saved.events.len(), r.events.len());
            assert_eq!(saved.finished_at, r.finished_at);
            assert_eq!(store.clear_recents().unwrap(), 0);
            store.archive(&id, true).unwrap();
            assert_eq!(store.get(&id).unwrap().archived_at, saved.archived_at);
        }
        let store = HistoryStore::open(&temp.0).unwrap();
        assert!(store.get(&id).unwrap().archived_at.is_some());
        store.archive(&id, false).unwrap();
        assert!(store.get(&id).unwrap().archived_at.is_none());
        store.archive(&id, false).unwrap();
        assert_eq!(store.get(&id).unwrap().status, "failed");
    }
    #[test]
    fn management_rejects_live_and_claimed_work_and_invalid_identities() {
        let temp = Temp::new();
        let store = HistoryStore::open(&temp.0).unwrap();
        let mut r = record(&store);
        let token = process_registry::cancellation_token();
        store.insert(&r, Some(token)).unwrap();
        assert!(store.archive(&r.job_id, true).is_err());
        assert!(store.delete(&r.job_id).is_err());
        assert_eq!(store.clear_recents().unwrap(), 0);
        store
            .update(&r.job_id, |r| r.terminal("failed", "finished"))
            .unwrap();
        assert_eq!(store.clear_recents().unwrap(), 0);
        store.release(&[r.job_id.clone()]);
        process_registry::finish(token);
        store.claim(&r.job_id).unwrap();
        assert!(store.archive(&r.job_id, true).is_err());
        assert_eq!(store.clear_recents().unwrap(), 0);
        store.unclaim(&r.job_id);
        assert!(store.delete(&r.job_id).is_err());
        store.archive(&r.job_id, true).unwrap();
        store.claim(&r.job_id).unwrap();
        assert!(store.delete(&r.job_id).is_err());
        store.unclaim(&r.job_id);
        for id in ["", " ", "missing", "' OR 1=1 --"] {
            assert!(store.archive(id, true).is_err());
            assert!(store.delete(id).is_err());
        }
        r.job_id = "other".into();
        store.insert(&r, None).unwrap();
        let other = HistoryStore::open(&temp.0).unwrap();
        assert_eq!(other.clear_recents().unwrap(), 0);
        assert!(other.get("other").unwrap().archived_at.is_none());
    }
    #[test]
    fn deleting_imported_history_keeps_files_and_never_reimports_after_restart() {
        let temp = Temp::new();
        let root = temp.0.join("course");
        std::fs::create_dir_all(&root).unwrap();
        let uuid = uuid::Uuid::new_v4();
        let artifact = root.join(format!(".Week_Guide.md.{uuid}.gwfailed"));
        std::fs::create_dir(&artifact).unwrap();
        let marker=serde_json::json!({"owner":"guide-watcher","schema_version":1,"transaction_id":uuid.to_string()}).to_string();
        std::fs::write(artifact.join("workspace.json"), &marker).unwrap();
        std::fs::write(root.join("source.pdf"), "original").unwrap();
        let id;
        {
            let store = HistoryStore::open(&temp.0.join("db")).unwrap();
            assert_eq!(store.import_existing(&root).unwrap(), 1);
            id = store.list().unwrap()[0].job_id.clone();
            store.archive(&id, true).unwrap();
            store.delete(&id).unwrap();
            store.delete(&id).unwrap();
            assert!(store.list().unwrap().is_empty());
            assert_eq!(store.import_existing(&root).unwrap(), 0);
        }
        let store = HistoryStore::open(&temp.0.join("db")).unwrap();
        assert_eq!(store.import_existing(&root).unwrap(), 0);
        assert!(store.list().unwrap().is_empty());
        assert_eq!(
            std::fs::read_to_string(artifact.join("workspace.json")).unwrap(),
            marker
        );
        assert_eq!(
            std::fs::read_to_string(root.join("source.pdf")).unwrap(),
            "original"
        );
        // A separate old attempt at the same output remains independently importable.
        let uuid = uuid::Uuid::new_v4();
        let other = root.join(format!(".Week_Guide.md.{uuid}.gwfailed"));
        std::fs::create_dir(&other).unwrap();
        std::fs::write(other.join("workspace.json"),serde_json::json!({"owner":"guide-watcher","schema_version":1,"transaction_id":uuid.to_string()}).to_string()).unwrap();
        assert_eq!(store.import_existing(&root).unwrap(), 1);
    }
    #[test]
    fn deleting_recorded_attempt_suppresses_artifact_duplicates_but_allows_new_attempts() {
        let temp = Temp::new();
        let store = HistoryStore::open(&temp.0).unwrap();
        let mut r = record(&store);
        r.output_path = Some(temp.0.join("Week_Guide.md").to_string_lossy().into_owned());
        r.prep_path = Some(
            temp.0
                .join("Week_Guide.prep.md")
                .to_string_lossy()
                .into_owned(),
        );
        r.terminal("failed", "Original outcome");
        store.insert(&r, None).unwrap();
        store.archive(&r.job_id, true).unwrap();
        store.delete(&r.job_id).unwrap();
        let mut imported = r.clone();
        imported.job_id = "import-duplicate".into();
        assert_eq!(store.insert_import_if_untracked(&imported).unwrap(), 0);
        let mut fresh = r.clone();
        fresh.job_id = uuid::Uuid::new_v4().to_string();
        fresh.archived_at = None;
        store.insert(&fresh, None).unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
        let conn = store.connection.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT key_hash FROM deleted_history")
            .unwrap();
        for key in stmt.query_map([], |row| row.get::<_, String>(0)).unwrap() {
            let key = key.unwrap();
            assert!(!key.contains("Week"));
            assert!(!key.contains(&r.job_id));
        }
    }
    #[test]
    fn schema_one_migrates_legacy_json_and_malformed_clear_rolls_back() {
        let temp = Temp::new();
        std::fs::create_dir_all(&temp.0).unwrap();
        let path = temp.0.join("history.sqlite3");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE attempts(id TEXT PRIMARY KEY NOT NULL,record TEXT NOT NULL);PRAGMA user_version=1;").unwrap();
        }
        let store = HistoryStore::open(&temp.0).unwrap();
        let mut r = record(&store);
        r.terminal("interrupted", "old attempt");
        let mut value = serde_json::to_value(&r).unwrap();
        value.as_object_mut().unwrap().remove("archivedAt");
        {
            let conn = store.connection.lock().unwrap();
            conn.execute(
                "INSERT INTO attempts VALUES(?1,?2)",
                params![r.job_id, value.to_string()],
            )
            .unwrap();
            conn.execute("INSERT INTO attempts VALUES('bad','not json')", [])
                .unwrap();
            assert_eq!(
                conn.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                    .unwrap(),
                2
            );
        }
        assert!(store.get(&r.job_id).unwrap().archived_at.is_none());
        assert!(store.clear_recents().is_err());
        assert!(store.get(&r.job_id).unwrap().archived_at.is_none());
    }
    #[test]
    fn independent_connections_cannot_resurrect_a_deleted_record() {
        let temp = Temp::new();
        let one = HistoryStore::open(&temp.0).unwrap();
        let mut r = record(&one);
        r.terminal("failed", "finished");
        one.insert(&r, None).unwrap();
        one.archive(&r.job_id, true).unwrap();
        let two = HistoryStore::open(&temp.0).unwrap();
        let id = r.job_id.clone();
        std::thread::scope(|scope| {
            let a = scope.spawn(|| one.delete(&id));
            let b = scope.spawn(|| two.archive(&id, true));
            assert!(a.join().unwrap().is_ok());
            let _ = b.join().unwrap();
        });
        assert!(one.get(&id).is_err());
        assert!(two.get(&id).is_err());
        assert!(two.archive(&id, false).is_err());
    }

    #[test]
    fn clear_recents_archives_every_terminal_outcome_but_keeps_active_jobs_visible() {
        let temp = Temp::new();
        let store = HistoryStore::open(&temp.0).unwrap();
        for status in [
            "starting",
            "working",
            "queued",
            "failed",
            "done",
            "interrupted",
            "blocked",
            "cancelled",
        ] {
            let mut r = record(&store);
            r.status = status.into();
            store.insert(&r, None).unwrap();
        }
        assert_eq!(store.clear_recents().unwrap(), 5);
        let rows = store.list().unwrap();
        assert_eq!(rows.len(), 8);
        for r in rows {
            assert_eq!(r.archived_at.is_none(), active(&r.status));
        }
        assert_eq!(store.clear_recents().unwrap(), 0);
    }
    #[test]
    fn failed_delete_rolls_back_suppression_and_keeps_the_record_for_retry() {
        let temp = Temp::new();
        let store = HistoryStore::open(&temp.0).unwrap();
        let mut r = record(&store);
        r.terminal("failed", "original outcome");
        store.insert(&r, None).unwrap();
        store.archive(&r.job_id, true).unwrap();
        store.connection.lock().unwrap().execute_batch("CREATE TEMP TRIGGER refuse_delete BEFORE DELETE ON attempts BEGIN SELECT RAISE(ABORT, 'simulated storage failure'); END;").unwrap();
        assert!(store.delete(&r.job_id).is_err());
        assert_eq!(store.get(&r.job_id).unwrap().summary, "original outcome");
        assert_eq!(
            store
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM deleted_history", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        store
            .connection
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER refuse_delete;")
            .unwrap();
        store.delete(&r.job_id).unwrap();
        assert!(store.get(&r.job_id).is_err());
    }

    #[test]
    fn records_survive_restart_and_unfinished_work_becomes_interrupted() {
        let temp = Temp::new();
        let id;
        {
            let store = HistoryStore::open(&temp.0).unwrap();
            let r = record(&store);
            id = r.job_id.clone();
            store.insert(&r, None).unwrap();
        }
        let store = HistoryStore::open(&temp.0).unwrap();
        let rows = store.list().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].job_id, id);
        assert_eq!(rows[0].status, "interrupted");
        assert!(!rows[0].can_resume);
    }
    #[test]
    fn another_live_instance_is_not_mistaken_for_a_crash() {
        let temp = Temp::new();
        let one = HistoryStore::open(&temp.0).unwrap();
        let r = record(&one);
        one.insert(&r, None).unwrap();
        let two = HistoryStore::open(&temp.0).unwrap();
        let rows = two.list().unwrap();
        assert_eq!(rows[0].status, "starting");
        assert!(!rows[0].can_cancel);
    }
    #[test]
    fn duplicate_identity_fails_without_replacing_data() {
        let temp = Temp::new();
        let store = HistoryStore::open(&temp.0).unwrap();
        let r = record(&store);
        store.insert(&r, None).unwrap();
        let mut changed = r.clone();
        changed.summary = "wrong".into();
        assert!(store.insert(&changed, None).is_err());
        assert_eq!(store.get(&r.job_id).unwrap().summary, r.summary);
    }
    #[test]
    fn request_expansion_is_atomic_when_a_child_conflicts() {
        let temp = Temp::new();
        let store = HistoryStore::open(&temp.0).unwrap();
        let r = record(&store);
        store.insert(&r, None).unwrap();
        let child = record(&store);
        store.insert(&child, None).unwrap();
        let new = record(&store);
        let token = process_registry::cancellation_token();
        assert!(store
            .replace_request(&r.job_id, &[new.clone(), child], token)
            .is_err());
        assert!(store.get(&r.job_id).is_ok());
        assert!(store.get(&new.job_id).is_err());
        process_registry::finish(token);
    }
    #[test]
    fn success_requires_a_verified_bundle_not_just_an_output_file() {
        let temp = Temp::new();
        let store = HistoryStore::open(&temp.0).unwrap();
        let output = temp.0.join("fake.md");
        std::fs::write(&output, "not verified").unwrap();
        let mut r = record(&store);
        r.output_path = Some(output.to_string_lossy().into_owned());
        store.insert(&r, None).unwrap();
        let token = process_registry::cancellation_token();
        store.finish(&r.job_id, 0, token).unwrap();
        assert_eq!(store.get(&r.job_id).unwrap().status, "failed");
        process_registry::finish(token);
    }
    #[test]
    fn cancellation_and_repeated_completion_preserve_specific_outcome() {
        let temp = Temp::new();
        let store = HistoryStore::open(&temp.0).unwrap();
        let r = record(&store);
        let token = process_registry::cancellation_token();
        store.insert(&r, Some(token)).unwrap();
        store.cancel(&r.job_id).unwrap();
        store.finish(&r.job_id, -1, token).unwrap();
        store.finish(&r.job_id, -1, token).unwrap();
        let record = store.get(&r.job_id).unwrap();
        assert_eq!(record.status, "cancelled");
        assert_eq!(record.events.len(), 2);
        process_registry::finish(token);
    }
    #[test]
    fn malformed_database_is_reported_and_not_replaced() {
        let temp = Temp::new();
        std::fs::create_dir_all(&temp.0).unwrap();
        let path = temp.0.join("history.sqlite3");
        std::fs::write(&path, "student data").unwrap();
        assert!(HistoryStore::open(&temp.0).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "student data");
    }
    #[test]
    fn action_claims_prevent_concurrent_retries_and_release_cleanly() {
        let temp = Temp::new();
        let store = HistoryStore::open(&temp.0).unwrap();
        store.claim("one").unwrap();
        assert!(store.claim("one").is_err());
        store.claim("two").unwrap();
        store.unclaim("one");
        store.claim("one").unwrap();
    }
    #[test]
    fn legacy_foreign_workspaces_are_ignored_and_import_is_idempotent() {
        let temp = Temp::new();
        let store = HistoryStore::open(&temp.0.join("db")).unwrap();
        let root = temp.0.join("course");
        std::fs::create_dir_all(&root).unwrap();
        let id = uuid::Uuid::new_v4();
        let path = root.join(format!(".Week_Guide.md.{id}.gwfailed"));
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("workspace.json"),serde_json::json!({"owner":"foreign","schema_version":1,"transaction_id":id.to_string()}).to_string()).unwrap();
        assert_eq!(store.import_existing(&root).unwrap(), 0);
        std::fs::write(path.join("workspace.json"),serde_json::json!({"owner":"guide-watcher","schema_version":1,"transaction_id":id.to_string()}).to_string()).unwrap();
        assert_eq!(store.import_existing(&root).unwrap(), 1);
        assert_eq!(store.import_existing(&root).unwrap(), 0);
        assert_eq!(store.list().unwrap()[0].status, "interrupted");
        assert!(store.list().unwrap()[0].events.is_empty());
    }
    #[test]
    fn concurrent_updates_do_not_lose_timeline_events() {
        let temp = Temp::new();
        let store = std::sync::Arc::new(HistoryStore::open(&temp.0).unwrap());
        let r = record(&store);
        store.insert(&r, None).unwrap();
        let handles = (0..12)
            .map(|i| {
                let store = store.clone();
                let id = r.job_id.clone();
                std::thread::spawn(move || {
                    store
                        .update(&id, |r| r.event("info", &format!("step {i}")))
                        .unwrap()
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(store.get(&r.job_id).unwrap().events.len(), 13);
    }
    #[test]
    fn verified_success_survives_restart_but_changed_output_is_not_openable() {
        let temp = Temp::new();
        let store = HistoryStore::open(&temp.0).unwrap();
        let guide = temp.0.join("Verified_Guide.md");
        let assets = temp.0.join("Verified_Guide_assets");
        let verify = temp.0.join(".Verified_Guide.md.gwverify");
        std::fs::create_dir(&assets).unwrap();
        std::fs::create_dir(&verify).unwrap();
        std::fs::write(&guide, "# Guide\n").unwrap();
        std::fs::write(assets.join("asset_manifest.json"), "{}").unwrap();
        for name in [
            "coverage_manifest.json",
            "visual_packet.json",
            "visual_contract.json",
        ] {
            std::fs::write(verify.join(name), "{}").unwrap();
        }
        completion::seal_v3_bundle(
            &guide,
            &assets,
            &verify,
            &guide,
            &"a".repeat(64),
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
        let mut r = record(&store);
        r.output_path = Some(guide.to_string_lossy().into_owned());
        store.insert(&r, None).unwrap();
        let token = process_registry::cancellation_token();
        store.finish(&r.job_id, 0, token).unwrap();
        process_registry::finish(token);
        assert_eq!(store.list().unwrap()[0].status, "done");
        assert!(store.list().unwrap()[0].can_open_output);
        drop(store);
        let store = HistoryStore::open(&temp.0).unwrap();
        assert_eq!(store.list().unwrap()[0].status, "done");
        std::fs::write(&guide, "changed").unwrap();
        let records = store.list().unwrap();
        assert_eq!(records[0].status, "done");
        assert!(!records[0].can_open_output);
        assert!(records[0]
            .recovery_note
            .as_ref()
            .unwrap()
            .contains("missing or changed"));
    }
    #[test]
    fn future_database_schema_is_never_downgraded() {
        let temp = Temp::new();
        std::fs::create_dir_all(&temp.0).unwrap();
        let path = temp.0.join("history.sqlite3");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA user_version=9;").unwrap();
        drop(db);
        assert!(HistoryStore::open(&temp.0).is_err());
        let db = Connection::open(path).unwrap();
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 9);
    }

    #[test]
    fn imported_artifacts_do_not_duplicate_recorded_attempts() {
        let temp = Temp::new();
        let store = HistoryStore::open(&temp.0).unwrap();
        let mut existing = record(&store);
        existing.output_path = Some("C:/Courses/Week_Guide.md".into());
        existing.prep_path = Some("C:/Courses/Week_Guide.prep.md".into());
        existing.terminal("failed", "Actual recorded provider failure");
        store.insert(&existing, None).unwrap();
        let mut imported = existing.clone();
        imported.job_id = "import-output".into();
        imported.terminal("done", "Imported output");
        assert_eq!(store.insert_import_if_untracked(&imported).unwrap(), 0);
        imported.job_id = "import-prep".into();
        imported.output_path = None;
        imported.terminal("interrupted", "Imported prep");
        assert_eq!(store.insert_import_if_untracked(&imported).unwrap(), 0);
        assert_eq!(
            store.get(&existing.job_id).unwrap().summary,
            "Actual recorded provider failure"
        );
        imported.job_id = "import-different".into();
        imported.prep_path = Some("C:/Courses/Other_Guide.prep.md".into());
        assert_eq!(store.insert_import_if_untracked(&imported).unwrap(), 1);
        assert_eq!(store.insert_import_if_untracked(&imported).unwrap(), 0);
    }
    #[test]
    fn blocked_is_persisted_before_batch_finishes_and_not_overwritten_by_done() {
        let temp = Temp::new();
        let store = HistoryStore::open(&temp.0).unwrap();
        let token = process_registry::cancellation_token();
        let r = record(&store);
        store.insert(&r, Some(token)).unwrap();
        store.blocked(&r.job_id, token).unwrap();
        assert_eq!(store.get(&r.job_id).unwrap().status, "blocked");
        store.finish(&r.job_id, -1, token).unwrap();
        assert_eq!(store.get(&r.job_id).unwrap().status, "blocked");
        let cancelled = record(&store);
        store.insert(&cancelled, Some(token)).unwrap();
        process_registry::cancel(token);
        store.blocked(&cancelled.job_id, token).unwrap();
        assert_eq!(store.get(&cancelled.job_id).unwrap().status, "cancelled");
        process_registry::finish(token);
    }
}
