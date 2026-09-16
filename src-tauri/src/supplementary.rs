//! One-time supplementary source collection for a course folder.
//!
//! The expensive judgement — which free lectures are worth transcribing for this course —
//! is made once by the cheap prep model with live web search (`plan`). Fetching is then
//! deterministic and fully reported by `supplementary_collect.py` (`collect`), and the
//! result lives in `<course>/_supplementary/` where every later guide run reads only the
//! paragraphs that match its lecture. Nothing here runs during guide generation.

use crate::codex::{self, CodexFileRequest};
use crate::job_events::{ConsoleProgress, SharedProgress};
use crate::{config, process_registry, source_context};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

pub(crate) const SUPPLEMENTARY_DIR: &str = "_supplementary";
pub(crate) const CANDIDATES_NAME: &str = "candidates.json";
const MAX_CANDIDATES: usize = 40;
const MAX_EVIDENCE_PDFS: usize = 40;
const EVIDENCE_CHARS_PER_PDF: usize = 700;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupplementaryAction {
    Plan,
    Collect,
    Status,
}

impl SupplementaryAction {
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "plan" => Some(Self::Plan),
            "collect" => Some(Self::Collect),
            "status" => Some(Self::Status),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct CandidatePlan {
    schema_version: u32,
    #[serde(default)]
    course: String,
    candidates: Vec<Candidate>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Candidate {
    id: String,
    kind: String,
    url: String,
    title: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    topics: Vec<String>,
    #[serde(default)]
    license_note: String,
    #[serde(default)]
    rationale: String,
}

pub async fn run(course_dir: &Path, action: SupplementaryAction) -> Result<serde_json::Value, String> {
    let course_dir = course_dir.canonicalize().map_err(|error| {
        format!("course folder is not readable: {} ({error})", course_dir.display())
    })?;
    if !course_dir.is_dir() {
        return Err(format!("course folder is not a directory: {}", course_dir.display()));
    }
    let cancellation = process_registry::cancellation_token();
    let result = match action {
        SupplementaryAction::Plan => plan(&course_dir, cancellation).await,
        SupplementaryAction::Collect => {
            let candidates = course_dir.join(SUPPLEMENTARY_DIR).join(CANDIDATES_NAME);
            if !candidates.is_file() {
                Err(format!(
                    "no candidate plan at {}; run `supplementary plan` first",
                    candidates.display()
                ))
            } else {
                run_collector(
                    &[
                        OsString::from("collect"),
                        course_dir.as_os_str().to_os_string(),
                        OsString::from("--candidates"),
                        candidates.as_os_str().to_os_string(),
                    ],
                    cancellation,
                )
                .await
            }
        }
        SupplementaryAction::Status => {
            run_collector(
                &[OsString::from("status"), course_dir.as_os_str().to_os_string()],
                cancellation,
            )
            .await
        }
    };
    process_registry::finish(cancellation);
    result
}

/// Ask the prep model, with live web search, for free lecture sources matching this course,
/// and store the validated plan at `_supplementary/candidates.json`.
async fn plan(
    course_dir: &Path,
    cancellation: process_registry::CancellationToken,
) -> Result<serde_json::Value, String> {
    let evidence = course_evidence(course_dir)?;
    if evidence.is_empty() {
        return Err(format!(
            "no PDF lecture or book files found directly in {}; nothing to plan from",
            course_dir.display()
        ));
    }
    let course_name = course_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "course".to_string());
    let workspace = std::env::temp_dir().join(format!("guide-watcher-supplementary-{}", Uuid::new_v4()));
    std::fs::create_dir(&workspace)
        .map_err(|error| format!("could not create planning workspace: {error}"))?;
    let raw_output = workspace.join("candidates-raw.json");
    let app: SharedProgress = Arc::new(ConsoleProgress);
    let job_id = format!("supplementary-plan-{}", Uuid::new_v4());
    let outcome = codex::run_codex_to_file(
        &app,
        &job_id,
        &workspace,
        CodexFileRequest {
            model: config::CODEX_PREP_PHASE_MODEL,
            effort: config::CODEX_PREP_PHASE_EFFORT,
            live_web_search: true,
            image_paths: &[],
            prompt: plan_prompt(&course_name, &evidence),
            output_path: &raw_output,
        },
        cancellation,
    )
    .await;
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&workspace);
            return Err(format!("the planning model could not run: {error}"));
        }
    };
    if outcome.exit_code != 0 {
        let _ = std::fs::remove_dir_all(&workspace);
        return Err(format!(
            "the planning model exited with status {}",
            outcome.exit_code
        ));
    }
    let raw = std::fs::read_to_string(&raw_output)
        .map_err(|error| format!("the planning model produced no readable output: {error}"))?;
    let _ = std::fs::remove_dir_all(&workspace);
    let plan = parse_plan(&raw, &course_name)?;
    let target_dir = course_dir.join(SUPPLEMENTARY_DIR);
    std::fs::create_dir_all(&target_dir)
        .map_err(|error| format!("could not create {}: {error}", target_dir.display()))?;
    let target = target_dir.join(CANDIDATES_NAME);
    let serialized = serde_json::to_string_pretty(&plan)
        .map_err(|error| format!("could not serialize the candidate plan: {error}"))?;
    let temp = target_dir.join(format!(".{CANDIDATES_NAME}.{}.tmp", Uuid::new_v4()));
    std::fs::write(&temp, format!("{serialized}\n"))
        .map_err(|error| format!("could not write the candidate plan: {error}"))?;
    std::fs::rename(&temp, &target)
        .map_err(|error| format!("could not store the candidate plan: {error}"))?;
    Ok(serde_json::json!({
        "candidates_path": target.display().to_string(),
        "candidate_count": plan.candidates.len(),
        "candidates": plan.candidates,
        "next_step": "review the plan, then run `guide-watcher-cli supplementary collect <course_dir>`",
    }))
}

/// Filename plus the first page's text of every PDF directly inside the course folder.
fn course_evidence(course_dir: &Path) -> Result<Vec<(String, String)>, String> {
    let mut entries = std::fs::read_dir(course_dir)
        .map_err(|error| format!("could not list {}: {error}", course_dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
                && !path
                    .file_name()
                    .map(|name| name.to_string_lossy().starts_with('.'))
                    .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    entries.sort();
    let mut evidence = Vec::new();
    for path in entries.into_iter().take(MAX_EVIDENCE_PDFS) {
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        let excerpt = match source_context::safe_extract_pdf_pages(&bytes, &path.to_string_lossy()) {
            Ok(pages) => pages
                .iter()
                .take(2)
                .map(|page| page.split_whitespace().collect::<Vec<_>>().join(" "))
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(EVIDENCE_CHARS_PER_PDF)
                .collect::<String>(),
            Err(error) => format!("(text not extractable: {error})"),
        };
        evidence.push((
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            excerpt,
        ));
    }
    Ok(evidence)
}

fn plan_prompt(course_name: &str, evidence: &[(String, String)]) -> String {
    let listing = evidence
        .iter()
        .map(|(name, excerpt)| format!("- `{name}`: {excerpt}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "You are planning supplementary study material for one university course. The course folder \
         is named {course_name:?} and contains these PDF files (lecture decks and textbooks), each \
         shown with the text of its first pages:\n{listing}\n\n\
         Propose free, high-quality, publicly available lecture recordings or transcript pages that \
         teach the SAME topics these lectures cover, so a study-guide writer can borrow their \
         explanations. Prefer complete university courses with captions or transcripts: MIT \
         OpenCourseWare, UW-Madison CS537 (Remzi Arpaci-Dusseau), Berkeley CS162, Stanford (CS144, \
         CS140), CMU 15-213, Kurose & Ross's own videos, and comparable sources for other subjects. \
         Use web search to confirm each URL exists; never invent a YouTube video id. When you know a \
         course but not the exact video for a topic, give the playlist or course page as `kind: url`.\n\n\
         Return STRICT JSON only, no prose and no code fence, in this exact shape:\n\
         {{\"schema_version\": 1, \"course\": \"<course name>\", \"candidates\": [{{\"id\": \"<lowercase slug, [a-z0-9._-], unique>\", \"kind\": \"youtube\" | \"url\", \"url\": \"<https URL>\", \"title\": \"<what this lecture covers>\", \"source\": \"<institution and instructor>\", \"topics\": [\"<topic words matching the course lectures>\"], \"license_note\": \"<license or 'personal study use'>\", \"rationale\": \"<one sentence: which course lecture(s) it supports>\"}}]}}\n\n\
         Rules: at most {MAX_CANDIDATES} candidates; one candidate per lecture video or transcript \
         page; cover every lecture topic in the folder that a free source can support; skip topics \
         with no good free source rather than padding; `topics` must use words that appear in the \
         lecture files above so the app can match transcripts to lectures."
    )
}

fn parse_plan(raw: &str, course_name: &str) -> Result<CandidatePlan, String> {
    let trimmed = raw.trim();
    let body = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|rest| rest.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(trimmed);
    let start = body.find('{').ok_or("the planning model returned no JSON object")?;
    let end = body.rfind('}').ok_or("the planning model returned no JSON object")?;
    let mut plan: CandidatePlan = serde_json::from_str(&body[start..=end])
        .map_err(|error| format!("the planning model's JSON is invalid: {error}"))?;
    if plan.schema_version != 1 {
        return Err("candidate plan schema_version must be 1".to_string());
    }
    if plan.course.trim().is_empty() {
        plan.course = course_name.to_string();
    }
    if plan.candidates.is_empty() {
        return Err("the planning model proposed no candidates".to_string());
    }
    if plan.candidates.len() > MAX_CANDIDATES {
        return Err(format!(
            "the planning model proposed {} candidates; at most {MAX_CANDIDATES} are accepted",
            plan.candidates.len()
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for candidate in &plan.candidates {
        if !is_valid_id(&candidate.id) {
            return Err(format!("candidate id is not a valid slug: {:?}", candidate.id));
        }
        if !seen.insert(candidate.id.clone()) {
            return Err(format!("duplicate candidate id: {}", candidate.id));
        }
        if candidate.kind != "youtube" && candidate.kind != "url" {
            return Err(format!(
                "candidate {} has unknown kind {:?}",
                candidate.id, candidate.kind
            ));
        }
        if !(candidate.url.starts_with("https://") || candidate.url.starts_with("http://")) {
            return Err(format!("candidate {} has a non-http URL", candidate.id));
        }
        if candidate.title.trim().is_empty() {
            return Err(format!("candidate {} has no title", candidate.id));
        }
    }
    Ok(plan)
}

fn is_valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() || first.is_ascii_digit() => {}
        _ => return false,
    }
    id.len() <= 81
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

async fn run_collector(
    args: &[OsString],
    cancellation: process_registry::CancellationToken,
) -> Result<serde_json::Value, String> {
    let script = PathBuf::from(config::SUPPLEMENTARY_COLLECT_SCRIPT);
    if !script.is_file() {
        return Err(format!(
            "the collector script is missing: {}",
            script.display()
        ));
    }
    let mut full = vec![script.as_os_str().to_os_string()];
    full.extend(args.iter().cloned());
    let outcome = codex::run_native_command("supplementary-collect", "python", &full, cancellation).await?;
    if outcome.exit_code != 0 {
        return Err(format!(
            "the collector exited with status {}:\n{}",
            outcome.exit_code, outcome.output
        ));
    }
    Ok(serde_json::json!({
        "exit_code": outcome.exit_code,
        "output": outcome.output,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_are_parsed_strictly_and_validated() {
        let raw = "```json\n{\"schema_version\": 1, \"course\": \"\", \"candidates\": [{\"id\": \"remzi-day1\", \"kind\": \"youtube\", \"url\": \"https://youtu.be/LVxN7ZkGh3w\", \"title\": \"Day 1\"}]}\n```";
        let plan = parse_plan(raw, "Operating Systems").unwrap();
        assert_eq!(plan.course, "Operating Systems");
        assert_eq!(plan.candidates.len(), 1);
        assert_eq!(plan.candidates[0].topics, Vec::<String>::new());

        let bad_id = raw.replace("remzi-day1", "../escape");
        assert!(parse_plan(&bad_id, "x").unwrap_err().contains("valid slug"));
        let bad_kind = raw.replace("\"youtube\"", "\"podcast\"");
        assert!(parse_plan(&bad_kind, "x").unwrap_err().contains("unknown kind"));
        let bad_url = raw.replace("https://youtu.be/LVxN7ZkGh3w", "file:///x");
        assert!(parse_plan(&bad_url, "x").unwrap_err().contains("non-http"));
        let duplicate = raw.replace(
            "]}",
            ", {\"id\": \"remzi-day1\", \"kind\": \"url\", \"url\": \"https://ocw.mit.edu\", \"title\": \"dup\"}]}",
        );
        assert!(parse_plan(&duplicate, "x").unwrap_err().contains("duplicate"));
        assert!(parse_plan("no json here", "x").is_err());
        assert!(parse_plan("{\"schema_version\": 1, \"candidates\": []}", "x")
            .unwrap_err()
            .contains("no candidates"));
    }

    #[test]
    fn actions_parse_from_the_cli_words_only() {
        assert_eq!(SupplementaryAction::parse("plan"), Some(SupplementaryAction::Plan));
        assert_eq!(SupplementaryAction::parse("collect"), Some(SupplementaryAction::Collect));
        assert_eq!(SupplementaryAction::parse("status"), Some(SupplementaryAction::Status));
        assert_eq!(SupplementaryAction::parse("fetch"), None);
    }

    #[test]
    fn the_prompt_names_every_pdf_and_demands_strict_json() {
        let prompt = plan_prompt(
            "Operating Systems",
            &[("2-What_is_OS.pdf".to_string(), "What is an OS".to_string())],
        );
        assert!(prompt.contains("`2-What_is_OS.pdf`: What is an OS"));
        assert!(prompt.contains("STRICT JSON"));
        assert!(prompt.contains("never invent a YouTube video id"));
    }
}
