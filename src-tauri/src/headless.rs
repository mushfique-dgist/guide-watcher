use crate::codex::HybridRunConfig;
use crate::job_events::ConsoleProgress;
use crate::process_registry;
use crate::{batch, BatchJobResult};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum HeadlessCommand {
    Generate(PathBuf),
    ResumePrep(PathBuf),
}

#[derive(Debug, Eq, PartialEq)]
enum ValidatedHeadlessCommand {
    Generate(PathBuf),
    Resume(PathBuf),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct HeadlessOutcome {
    pub job_id: String,
    pub guide_path: PathBuf,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum HeadlessError {
    InvalidInput(String),
    Pipeline(String),
}

impl HeadlessError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::InvalidInput(_) => 2,
            Self::Pipeline(_) => 1,
        }
    }
}

impl fmt::Display for HeadlessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(message) | Self::Pipeline(message) => formatter.write_str(message),
        }
    }
}

pub async fn run_headless(command: HeadlessCommand) -> Result<HeadlessOutcome, HeadlessError> {
    let input = validate_command(command)?;
    let progress = Arc::new(ConsoleProgress);
    let (job_id, guide_path, completed) = match input {
        ValidatedHeadlessCommand::Generate(source) => {
            let cancellation = process_registry::cancellation_token();
            let prepared = match tokio::task::spawn_blocking(move || {
                batch::preflight_paths(&[source], "auto", cancellation)
            })
            .await
            {
                Ok(Ok(prepared)) => prepared,
                Ok(Err(error)) => {
                    process_registry::finish(cancellation);
                    return Err(HeadlessError::InvalidInput(error));
                }
                Err(error) => {
                    process_registry::finish(cancellation);
                    return Err(HeadlessError::InvalidInput(format!(
                        "generation preflight worker failed: {error}"
                    )));
                }
            };
            let job = match prepared.planned_jobs() {
                Ok(jobs) => jobs,
                Err(error) => {
                    process_registry::finish(cancellation);
                    return Err(HeadlessError::InvalidInput(error));
                }
            }
            .into_iter()
            .next()
            .ok_or_else(|| HeadlessError::InvalidInput("no guide was planned".to_string()));
            let job = match job {
                Ok(job) => job,
                Err(error) => {
                    process_registry::finish(cancellation);
                    return Err(error);
                }
            };
            let completed = batch::execute_batch(
                progress,
                prepared,
                HybridRunConfig::canonical(),
                cancellation,
            )
            .await
            .first()
            .is_some_and(|result| result.status == crate::BatchJobStatus::Succeeded);
            process_registry::finish(cancellation);
            (job.job_id, PathBuf::from(job.output_path), completed)
        }
        ValidatedHeadlessCommand::Resume(prep) => {
            let cancellation = process_registry::cancellation_token();
            let prepared = match tokio::task::spawn_blocking(move || {
                batch::preflight_resume_path(&prep, cancellation)
            })
            .await
            {
                Ok(Ok(prepared)) => prepared,
                Ok(Err(error)) => {
                    process_registry::finish(cancellation);
                    return Err(HeadlessError::InvalidInput(error));
                }
                Err(error) => {
                    process_registry::finish(cancellation);
                    return Err(HeadlessError::InvalidInput(format!(
                        "resume preflight worker failed: {error}"
                    )));
                }
            };
            let guide_path = prepared.output_path().to_path_buf();
            let job_id = Uuid::new_v4().to_string();
            let completed = batch::execute_resume(
                progress,
                job_id.clone(),
                prepared,
                HybridRunConfig::canonical(),
                cancellation,
            )
            .await;
            process_registry::finish(cancellation);
            (job_id, guide_path, completed)
        }
    };

    if !completed || !guide_path.is_file() {
        return Err(HeadlessError::Pipeline(
            "guide generation failed; the detailed failure and recovery paths are shown above"
                .to_string(),
        ));
    }

    Ok(HeadlessOutcome { job_id, guide_path })
}

/// Run the real preflight for one lecture source or one `.prep.md` packet and report the
/// outcome as JSON, without starting any model. Reserved workspaces are discarded.
pub async fn run_headless_check(path: PathBuf) -> Result<serde_json::Value, HeadlessError> {
    let current_dir = std::env::current_dir().map_err(|error| {
        HeadlessError::InvalidInput(format!("could not resolve the current directory: {error}"))
    })?;
    let path = canonical_existing_file(&path, &current_dir, "check target")?;
    let cancellation = process_registry::cancellation_token();
    let target = path.display().to_string();
    let result = if crate::job_events::is_prep_file(&path) {
        let prep = path.clone();
        match tokio::task::spawn_blocking(move || batch::preflight_resume_path(&prep, cancellation))
            .await
        {
            Ok(Ok(prepared)) => {
                let output = prepared.output_path().display().to_string();
                let predecessor_drift = prepared.predecessor_drift().to_vec();
                // Preflight passed; now the checks the writer start performs.
                match prepared.dry_run() {
                    Ok(notes) => Ok(serde_json::json!({
                        "kind": "resume",
                        "target": target,
                        "ok": true,
                        "output": output,
                        "predecessor_drift": predecessor_drift,
                        "notes": notes,
                    })),
                    Err(error) => Ok(serde_json::json!({
                        "kind": "resume", "target": target, "ok": false, "error": error
                    })),
                }
            }
            Ok(Err(error)) => Ok(serde_json::json!({
                "kind": "resume", "target": target, "ok": false, "error": error
            })),
            Err(error) => Err(HeadlessError::Pipeline(format!("check worker failed: {error}"))),
        }
    } else {
        let source = path.clone();
        match tokio::task::spawn_blocking(move || {
            batch::preflight_paths(&[source], "auto", cancellation)
        })
        .await
        {
            Ok(Ok(prepared)) => {
                let planned = prepared.planned_jobs().map_err(HeadlessError::InvalidInput)?;
                let value = serde_json::json!({
                    "kind": "generate", "target": target, "ok": true, "planned": planned
                });
                prepared.discard().map_err(HeadlessError::Pipeline)?;
                Ok(value)
            }
            Ok(Err(error)) => Ok(serde_json::json!({
                "kind": "generate", "target": target, "ok": false, "error": error
            })),
            Err(error) => Err(HeadlessError::Pipeline(format!("check worker failed: {error}"))),
        }
    };
    process_registry::finish(cancellation);
    result
}

pub async fn run_headless_batch(paths: Vec<PathBuf>) -> Result<Vec<BatchJobResult>, HeadlessError> {
    let current_dir = std::env::current_dir().map_err(|error| {
        HeadlessError::InvalidInput(format!("could not resolve the current directory: {error}"))
    })?;
    let mut canonical = Vec::with_capacity(paths.len());
    for path in paths {
        canonical.push(canonical_existing_file(
            &path,
            &current_dir,
            "guide source",
        )?);
    }
    let cancellation = process_registry::cancellation_token();
    let prepared = match tokio::task::spawn_blocking(move || {
        batch::preflight_paths(&canonical, "auto", cancellation)
    })
    .await
    {
        Ok(Ok(prepared)) => prepared,
        Ok(Err(error)) => {
            process_registry::finish(cancellation);
            return Err(HeadlessError::InvalidInput(error));
        }
        Err(error) => {
            process_registry::finish(cancellation);
            return Err(HeadlessError::InvalidInput(format!(
                "batch preflight worker failed: {error}"
            )));
        }
    };
    let progress = Arc::new(ConsoleProgress);
    let results = batch::execute_batch(
        progress,
        prepared,
        HybridRunConfig::canonical(),
        cancellation,
    )
    .await;
    process_registry::finish(cancellation);
    Ok(results)
}

pub fn cancel_headless_jobs() {
    process_registry::cancel_all_nonblocking();
}

fn validate_command(command: HeadlessCommand) -> Result<ValidatedHeadlessCommand, HeadlessError> {
    let current_dir = std::env::current_dir().map_err(|error| {
        HeadlessError::InvalidInput(format!("could not resolve the current directory: {error}"))
    })?;
    validate_command_from(command, &current_dir)
}

fn validate_command_from(
    command: HeadlessCommand,
    current_dir: &Path,
) -> Result<ValidatedHeadlessCommand, HeadlessError> {
    match command {
        HeadlessCommand::Generate(source) => {
            let source = canonical_existing_file(&source, current_dir, "guide source")?;
            let source_text = source.to_string_lossy();
            if !crate::config::is_watched_extension(&source_text)
                || crate::config::should_skip(&source_text)
            {
                return Err(HeadlessError::InvalidInput(format!(
                    "source is unsupported or reserved as supporting material: {}",
                    source.display()
                )));
            }
            Ok(ValidatedHeadlessCommand::Generate(source))
        }
        HeadlessCommand::ResumePrep(prep) => {
            let prep = canonical_existing_file(&prep, current_dir, "prep packet")?;
            prep.to_str().ok_or_else(|| {
                HeadlessError::InvalidInput(format!(
                    "prep path is not valid Unicode: {}",
                    prep.display()
                ))
            })?;
            if !crate::job_events::is_prep_file(&prep) {
                return Err(HeadlessError::InvalidInput(
                    "the canonical workflow can resume only an exact .prep.md evidence packet"
                        .to_string(),
                ));
            }
            Ok(ValidatedHeadlessCommand::Resume(prep))
        }
    }
}

fn canonical_existing_file(
    path: &Path,
    current_dir: &Path,
    label: &str,
) -> Result<PathBuf, HeadlessError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        current_dir.join(path)
    };
    if !absolute.is_file() {
        return Err(HeadlessError::InvalidInput(format!(
            "{label} is not a readable file: {}",
            absolute.display()
        )));
    }
    absolute.canonicalize().map_err(|error| {
        HeadlessError::InvalidInput(format!(
            "could not resolve {label} {}: {error}",
            absolute.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("guide-watcher-headless-{}", Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        dir
    }

    #[test]
    fn generate_preserves_a_path_with_spaces_before_course_planning() {
        let dir = scratch_dir();
        let source = dir.join("Lecture 01.PDF");
        std::fs::write(&source, b"pdf").unwrap();

        let resolved = canonical_existing_file(&source, &dir, "guide source").unwrap();
        assert_eq!(resolved, source.canonicalize().unwrap());
        assert_eq!(
            validate_command(HeadlessCommand::Generate(source.clone())).unwrap(),
            ValidatedHeadlessCommand::Generate(source.canonicalize().unwrap())
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn bare_relative_paths_are_canonicalized_before_validation() {
        let dir = scratch_dir();
        let source = dir.join("Lecture.pdf");
        std::fs::write(&source, b"pdf").unwrap();
        let resolved = canonical_existing_file(Path::new("Lecture.pdf"), &dir, "guide source")
            .expect("resolve bare relative path");
        assert_eq!(resolved, source.canonicalize().unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn resume_accepts_only_exact_prep_suffix_before_provider_execution() {
        let dir = scratch_dir();
        let prep = dir.join("Lecture_01_Guide.prep.md");
        let guide = dir.join("Lecture_01_Guide.md");
        std::fs::write(&prep, "packet").unwrap();
        std::fs::write(&guide, "guide").unwrap();

        assert_eq!(
            validate_command(HeadlessCommand::ResumePrep(prep.clone())).unwrap(),
            ValidatedHeadlessCommand::Resume(prep.canonicalize().unwrap())
        );
        let error = validate_command(HeadlessCommand::ResumePrep(guide.clone())).unwrap_err();
        assert!(matches!(error, HeadlessError::InvalidInput(_)));
        assert!(error.to_string().contains("exact .prep.md"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_and_skipped_sources_fail_before_provider_execution() {
        let missing = PathBuf::from("definitely-missing-guide-watcher-source.pdf");
        assert!(matches!(
            validate_command(HeadlessCommand::Generate(missing)),
            Err(HeadlessError::InvalidInput(_))
        ));

        let dir = scratch_dir();
        let skipped = dir.join("syllabus.pdf");
        std::fs::write(&skipped, b"pdf").unwrap();
        let error = validate_command(HeadlessCommand::Generate(skipped)).unwrap_err();
        assert!(error
            .to_string()
            .contains("reserved as supporting material"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
