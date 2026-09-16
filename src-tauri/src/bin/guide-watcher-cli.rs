use guide_watcher_lib::{HeadlessCommand, HeadlessError};
use std::ffi::OsString;
use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

const USAGE: &str = "Usage:\n  guide-watcher-cli generate <lecture.pdf|pptx|docx|html>\n  guide-watcher-cli resume <guide.prep.md>\n  guide-watcher-cli batch <source> [source ...]\n  guide-watcher-cli circuit prepare --week <3-60> [--source <course-file>]\n  guide-watcher-cli circuit generate --week <3-60> [--source <course-file>]\n  guide-watcher-cli visuals inspect <source> [--write-skeleton]\n  guide-watcher-cli visuals validate <source>\n  guide-watcher-cli supplementary plan <course_dir>      (prep model proposes free lecture sources)\n  guide-watcher-cli supplementary collect <course_dir>   (fetch transcripts; full report, nothing skipped silently)\n  guide-watcher-cli supplementary status <course_dir>\n  guide-watcher-cli check <lecture.pdf|guide.prep.md>       (run preflight only; no model is started)";
const INTERRUPT_CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Eq, PartialEq)]
enum CliAction {
    RunSingle(HeadlessCommand),
    RunBatch(Vec<PathBuf>),
    InspectVisuals {
        source: PathBuf,
        write_skeleton: bool,
    },
    ValidateVisuals(PathBuf),
    Supplementary {
        action: String,
        course_dir: PathBuf,
    },
    Check(PathBuf),
    Circuit {
        week: u32,
        source: Option<PathBuf>,
        generate: bool,
    },
    Help,
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<CliAction, String> {
    let mut args = args.into_iter();
    let command = args.next().ok_or_else(|| USAGE.to_string())?;
    if matches!(command.to_str(), Some("--help" | "-h")) {
        return if args.next().is_none() {
            Ok(CliAction::Help)
        } else {
            Err(USAGE.to_string())
        };
    }
    match command.to_str() {
        Some("generate" | "resume") => {
            let path = args.next().ok_or_else(|| USAGE.to_string())?;
            if args.next().is_some() {
                return Err(USAGE.to_string());
            }
            if matches!(command.to_str(), Some("generate")) {
                Ok(CliAction::RunSingle(HeadlessCommand::Generate(
                    PathBuf::from(path),
                )))
            } else {
                Ok(CliAction::RunSingle(HeadlessCommand::ResumePrep(
                    PathBuf::from(path),
                )))
            }
        }
        Some("batch") => {
            let paths = args.map(PathBuf::from).collect::<Vec<_>>();
            if paths.is_empty() {
                Err(USAGE.to_string())
            } else {
                Ok(CliAction::RunBatch(paths))
            }
        }
        Some("circuit") => {
            let subcommand = args.next().ok_or_else(|| USAGE.to_string())?;
            let generate = match subcommand.to_str() {
                Some("prepare") => false,
                Some("generate") => true,
                _ => return Err(USAGE.to_string()),
            };
            let mut week = None;
            let mut source = None;
            while let Some(flag) = args.next() {
                match flag.to_str() {
                    Some("--week") if week.is_none() => {
                        let value = args.next().ok_or_else(|| USAGE.to_string())?;
                        week = Some(
                            value
                                .to_str()
                                .and_then(|value| value.parse::<u32>().ok())
                                .ok_or_else(|| USAGE.to_string())?,
                        );
                    }
                    Some("--source") if source.is_none() => {
                        source = Some(PathBuf::from(args.next().ok_or_else(|| USAGE.to_string())?));
                    }
                    _ => return Err(USAGE.to_string()),
                }
            }
            let week = week.ok_or_else(|| USAGE.to_string())?;
            if !(3..=60).contains(&week) {
                return Err(USAGE.to_string());
            }
            Ok(CliAction::Circuit {
                week,
                source,
                generate,
            })
        }
        Some("visuals") => {
            let subcommand = args.next().ok_or_else(|| USAGE.to_string())?;
            let source = PathBuf::from(args.next().ok_or_else(|| USAGE.to_string())?);
            match subcommand.to_str() {
                Some("inspect") => {
                    let write_skeleton = match args.next() {
                        None => false,
                        Some(flag) if flag == "--write-skeleton" => true,
                        Some(_) => return Err(USAGE.to_string()),
                    };
                    if args.next().is_some() {
                        return Err(USAGE.to_string());
                    }
                    Ok(CliAction::InspectVisuals {
                        source,
                        write_skeleton,
                    })
                }
                Some("validate") if args.next().is_none() => Ok(CliAction::ValidateVisuals(source)),
                _ => Err(USAGE.to_string()),
            }
        }
        Some("check") => {
            let path = PathBuf::from(args.next().ok_or_else(|| USAGE.to_string())?);
            if args.next().is_some() {
                return Err(USAGE.to_string());
            }
            Ok(CliAction::Check(path))
        }
        Some("supplementary") => {
            let action = args.next().ok_or_else(|| USAGE.to_string())?;
            let course_dir = PathBuf::from(args.next().ok_or_else(|| USAGE.to_string())?);
            if args.next().is_some() {
                return Err(USAGE.to_string());
            }
            match action.to_str() {
                Some(word @ ("plan" | "collect" | "status")) => Ok(CliAction::Supplementary {
                    action: word.to_string(),
                    course_dir,
                }),
                _ => Err(USAGE.to_string()),
            }
        }
        _ => Err(USAGE.to_string()),
    }
}

enum CliOutcome {
    Single(guide_watcher_lib::HeadlessOutcome),
    Batch(Vec<guide_watcher_lib::BatchJobResult>),
    Json(serde_json::Value),
}

async fn execute_action(action: CliAction) -> Result<CliOutcome, HeadlessError> {
    match action {
        CliAction::RunSingle(command) => guide_watcher_lib::run_headless(command)
            .await
            .map(CliOutcome::Single),
        CliAction::RunBatch(paths) => guide_watcher_lib::run_headless_batch(paths)
            .await
            .map(CliOutcome::Batch),
        CliAction::InspectVisuals {
            source,
            write_skeleton,
        } => guide_watcher_lib::inspect_visuals(&source, write_skeleton)
            .await
            .map(CliOutcome::Json)
            .map_err(HeadlessError::InvalidInput),
        CliAction::ValidateVisuals(source) => guide_watcher_lib::validate_visuals(&source)
            .await
            .map(CliOutcome::Json)
            .map_err(HeadlessError::InvalidInput),
        CliAction::Supplementary { action, course_dir } => {
            guide_watcher_lib::run_supplementary(&course_dir, &action)
                .await
                .map(CliOutcome::Json)
                .map_err(HeadlessError::Pipeline)
        }
        CliAction::Check(path) => {
            let value = guide_watcher_lib::run_headless_check(path).await?;
            if value.get("ok") == Some(&serde_json::Value::Bool(false)) {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).expect("check result is serializable")
                );
                std::process::exit(1);
            }
            Ok(CliOutcome::Json(value))
        }
        CliAction::Circuit {
            week,
            source,
            generate,
        } => {
            let prepared = guide_watcher_lib::prepare_circuit_lab_week(week, source)
                .await
                .map_err(HeadlessError::InvalidInput)?;
            if generate {
                guide_watcher_lib::run_headless(HeadlessCommand::Generate(prepared.source_path))
                    .await
                    .map(CliOutcome::Single)
            } else {
                serde_json::to_value(prepared)
                    .map(CliOutcome::Json)
                    .map_err(|error| HeadlessError::InvalidInput(error.to_string()))
            }
        }
        CliAction::Help => unreachable!("help exits before execution"),
    }
}

#[tokio::main]
async fn main() {
    let action = match parse_args(std::env::args_os().skip(1)) {
        Ok(
            action @ (CliAction::RunSingle(_)
            | CliAction::RunBatch(_)
            | CliAction::InspectVisuals { .. }
            | CliAction::ValidateVisuals(_)
            | CliAction::Supplementary { .. }
            | CliAction::Check(_)
            | CliAction::Circuit { .. }),
        ) => action,
        Ok(CliAction::Help) => {
            println!("{USAGE}");
            return;
        }
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };

    let execution = execute_action(action);
    tokio::pin!(execution);
    let result = tokio::select! {
        result = &mut execution => result,
        interrupt = tokio::signal::ctrl_c() => {
            guide_watcher_lib::cancel_headless_jobs();
            if !await_cleanup_bounded(&mut execution, INTERRUPT_CLEANUP_TIMEOUT).await {
                eprintln!("cleanup did not finish within 30 seconds; exiting after requesting process-tree termination");
            }
            if let Err(error) = interrupt {
                eprintln!("could not listen for Ctrl+C: {error}");
                std::process::exit(1);
            }
            std::process::exit(130);
        }
    };

    match result {
        Ok(CliOutcome::Single(outcome)) => {
            println!("{}", outcome.guide_path.display());
        }
        Ok(CliOutcome::Batch(results)) => {
            for result in &results {
                println!(
                    "{}",
                    serde_json::to_string(result).expect("batch result is serializable")
                );
            }
            if results
                .iter()
                .any(|result| result.status != guide_watcher_lib::BatchJobStatus::Succeeded)
            {
                std::process::exit(1);
            }
        }
        Ok(CliOutcome::Json(value)) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&value).expect("visual result is serializable")
            );
        }
        Err(error) => exit_with_error(error),
    }
}

async fn await_cleanup_bounded<F>(future: F, timeout: Duration) -> bool
where
    F: Future,
{
    tokio::time::timeout(timeout, future).await.is_ok()
}

fn exit_with_error(error: HeadlessError) -> ! {
    eprintln!("{error}");
    std::process::exit(error.exit_code());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_routes_check_with_exactly_one_path() {
        let target = OsString::from(r"C:\Course Files\L3_Guide.prep.md");
        assert_eq!(
            parse_args([OsString::from("check"), target.clone()]).unwrap(),
            CliAction::Check(PathBuf::from(&target))
        );
        assert!(parse_args([OsString::from("check")]).is_err());
        assert!(parse_args([OsString::from("check"), target.clone(), OsString::from("extra")]).is_err());
    }

    #[test]
    fn parser_routes_supplementary_actions_with_exactly_one_course_dir() {
        let course = OsString::from(r"C:\Course Files\Operating Systems");
        for word in ["plan", "collect", "status"] {
            assert_eq!(
                parse_args([
                    OsString::from("supplementary"),
                    OsString::from(word),
                    course.clone(),
                ])
                .unwrap(),
                CliAction::Supplementary {
                    action: word.to_string(),
                    course_dir: PathBuf::from(&course),
                }
            );
        }
        assert!(parse_args([OsString::from("supplementary"), OsString::from("fetch"), course.clone()]).is_err());
        assert!(parse_args([OsString::from("supplementary"), OsString::from("plan")]).is_err());
        assert!(parse_args([
            OsString::from("supplementary"),
            OsString::from("plan"),
            course.clone(),
            OsString::from("extra"),
        ])
        .is_err());
    }

    #[test]
    fn parser_keeps_the_path_as_an_os_string() {
        let path = OsString::from(r"C:\Course Files\Lecture 01.PDF");
        assert_eq!(
            parse_args([OsString::from("generate"), path.clone()]).unwrap(),
            CliAction::RunSingle(HeadlessCommand::Generate(PathBuf::from(path)))
        );
    }

    #[test]
    fn parser_routes_only_generate_and_resume_with_exactly_one_path() {
        assert!(matches!(
            parse_args([OsString::from("resume"), OsString::from("Guide.prep.md")]),
            Ok(CliAction::RunSingle(HeadlessCommand::ResumePrep(_)))
        ));
        for invalid in [
            vec![],
            vec![OsString::from("generate")],
            vec![OsString::from("unknown"), OsString::from("lecture.pdf")],
            vec![
                OsString::from("generate"),
                OsString::from("one.pdf"),
                OsString::from("two.pdf"),
            ],
        ] {
            assert!(parse_args(invalid).is_err());
        }
    }

    #[test]
    fn batch_accepts_one_or_more_os_paths_without_flattening_them() {
        let first = OsString::from(r"C:\Course Files\Lecture 2.pdf");
        let second = OsString::from(r"C:\Course Files\Lecture 10.pdf");
        assert_eq!(
            parse_args([OsString::from("batch"), first.clone(), second.clone()]).unwrap(),
            CliAction::RunBatch(vec![PathBuf::from(first), PathBuf::from(second)])
        );
        assert!(parse_args([OsString::from("batch")]).is_err());
    }

    #[test]
    fn help_is_a_zero_work_action_and_rejects_extra_arguments() {
        assert_eq!(parse_args([OsString::from("--help")]), Ok(CliAction::Help));
        assert_eq!(parse_args([OsString::from("-h")]), Ok(CliAction::Help));
        assert!(parse_args([OsString::from("--help"), OsString::from("extra")]).is_err());
    }

    #[test]
    fn visuals_commands_are_explicit_and_write_skeleton_is_opt_in() {
        let source = OsString::from(r"C:\Course Files\Lecture 01.PDF");
        assert_eq!(
            parse_args([
                OsString::from("visuals"),
                OsString::from("inspect"),
                source.clone(),
            ])
            .unwrap(),
            CliAction::InspectVisuals {
                source: PathBuf::from(&source),
                write_skeleton: false,
            }
        );
        assert_eq!(
            parse_args([
                OsString::from("visuals"),
                OsString::from("inspect"),
                source.clone(),
                OsString::from("--write-skeleton"),
            ])
            .unwrap(),
            CliAction::InspectVisuals {
                source: PathBuf::from(&source),
                write_skeleton: true,
            }
        );
        assert_eq!(
            parse_args([
                OsString::from("visuals"),
                OsString::from("validate"),
                source.clone(),
            ])
            .unwrap(),
            CliAction::ValidateVisuals(PathBuf::from(source))
        );
        assert!(parse_args([
            OsString::from("visuals"),
            OsString::from("validate"),
            OsString::from("source.pdf"),
            OsString::from("--write-skeleton"),
        ])
        .is_err());
    }

    #[test]
    fn circuit_commands_require_an_explicit_newer_week_and_preserve_source_paths() {
        let source = OsString::from(r"C:\Course Files\Week 03.pdf");
        assert_eq!(
            parse_args([
                OsString::from("circuit"),
                OsString::from("generate"),
                OsString::from("--source"),
                source.clone(),
                OsString::from("--week"),
                OsString::from("3"),
            ])
            .unwrap(),
            CliAction::Circuit {
                week: 3,
                source: Some(PathBuf::from(source)),
                generate: true,
            }
        );
        assert!(parse_args([
            OsString::from("circuit"),
            OsString::from("prepare"),
            OsString::from("--week"),
            OsString::from("not-a-week"),
        ])
        .is_err());
        assert!(parse_args([
            OsString::from("circuit"),
            OsString::from("prepare"),
            OsString::from("--source"),
            OsString::from("week.pdf"),
        ])
        .is_err());
    }

    #[tokio::test]
    async fn interrupt_cleanup_wait_is_bounded() {
        assert!(await_cleanup_bounded(async {}, Duration::from_secs(1)).await);
        assert!(
            !await_cleanup_bounded(std::future::pending::<()>(), Duration::from_millis(1)).await
        );
    }
}
