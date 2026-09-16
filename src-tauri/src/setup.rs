//! First-run setup: what this machine still needs, and how to give it.
//!
//! A downloaded build knows nothing about the computer it landed on. Rather than failing later
//! with a path error, the app asks these questions up front, checks each answer against the
//! machine itself, and refuses to call itself ready until every check passes.
//!
//! Two routes lead to the same settings file. The manual route walks the person through each
//! requirement, checking as it goes. The assisted route hands the work to an AI assistant that is
//! already signed in on this machine, and waits.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::provider_executable::{resolve_provider_executable, ProviderExecutable};
use crate::settings::{self, Settings};

/// One thing the app needs, and whether this machine has it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Requirement {
    /// Stable name the interface keys on.
    pub id: String,
    /// What the user is being asked for, in their words.
    pub label: String,
    pub satisfied: bool,
    /// What was found, or what is wrong.
    pub detail: String,
    /// What the user should do about it, when they must act.
    pub fix: String,
    /// False when the app still works without it.
    pub required: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupState {
    /// True when every required check passes, so the wizard can step aside.
    pub ready: bool,
    pub requirements: Vec<Requirement>,
    pub settings: Settings,
    pub settings_path: String,
    /// Whether an AI assistant that could do this setup is installed and signed in.
    pub assistant_available: bool,
    pub assistant_detail: String,
}

fn requirement(
    id: &str,
    label: &str,
    satisfied: bool,
    detail: String,
    fix: &str,
    required: bool,
) -> Requirement {
    Requirement {
        id: id.to_string(),
        label: label.to_string(),
        satisfied,
        detail,
        fix: fix.to_string(),
        required,
    }
}

/// Whether a command exists on this machine, using each platform's own lookup.
fn command_exists(program: &str) -> bool {
    let finder = if cfg!(target_os = "windows") { "where" } else { "which" };
    std::process::Command::new(finder)
        .arg(program)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn python_program() -> &'static str {
    if cfg!(target_os = "windows") {
        "python"
    } else {
        "python3"
    }
}

/// Which Python packages the checker needs, and whether they import.
fn python_packages_present() -> Option<Vec<String>> {
    let script = "import importlib.util as u; \
        print(','.join(n for n in ['fitz','markdown_it','requests'] if u.find_spec(n) is None))";
    let output = std::process::Command::new(python_program())
        .args(["-c", script])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let missing = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Some(if missing.is_empty() {
        Vec::new()
    } else {
        missing.split(',').map(|name| name.to_string()).collect()
    })
}

/// The package name a user installs, for each import name.
fn package_name(import: &str) -> &'static str {
    match import {
        "fitz" => "pymupdf",
        "markdown_it" => "markdown-it-py",
        _ => "requests",
    }
}

/// Everything this machine still needs, checked against the machine rather than assumed.
pub fn state() -> SetupState {
    let current = settings::current();
    let mut requirements = Vec::new();

    requirements.push(match resolve_provider_executable(ProviderExecutable::Claude) {
        Ok(path) => requirement(
            "claude",
            "Claude Code, which writes the guides",
            true,
            format!("Found at {}", path.display()),
            "",
            true,
        ),
        Err(error) => requirement(
            "claude",
            "Claude Code, which writes the guides",
            false,
            error,
            "Install Claude Code, then run `claude` once in a terminal and sign in.",
            true,
        ),
    });
    requirements.push(match resolve_provider_executable(ProviderExecutable::Codex) {
        Ok(path) => requirement(
            "codex",
            "OpenAI Codex, which collects the material",
            true,
            format!("Found at {}", path.display()),
            "",
            true,
        ),
        Err(error) => requirement(
            "codex",
            "OpenAI Codex, which collects the material",
            false,
            error,
            "Install the Codex CLI, then run `codex login` and sign in.",
            true,
        ),
    });

    let python = command_exists(python_program());
    requirements.push(requirement(
        "python",
        "Python, which runs the checker",
        python,
        if python {
            format!("`{}` is on your PATH", python_program())
        } else {
            format!("`{}` was not found on your PATH", python_program())
        },
        "Install Python 3.13 and make sure it is on your PATH.",
        true,
    ));

    if python {
        match python_packages_present() {
            Some(missing) if missing.is_empty() => requirements.push(requirement(
                "python-packages",
                "The checker's Python packages",
                true,
                "pymupdf, markdown-it-py and requests are installed".to_string(),
                "",
                true,
            )),
            Some(missing) => {
                let names: Vec<&str> = missing.iter().map(|name| package_name(name)).collect();
                requirements.push(requirement(
                    "python-packages",
                    "The checker's Python packages",
                    false,
                    format!("Missing: {}", names.join(", ")),
                    &format!("Run: pip install {}", names.join(" ")),
                    true,
                ));
            }
            None => requirements.push(requirement(
                "python-packages",
                "The checker's Python packages",
                false,
                "Could not ask Python which packages are installed".to_string(),
                "Check that Python runs from a terminal, then try again.",
                true,
            )),
        }
    }

    let study = Path::new(&current.watch_dir);
    requirements.push(requirement(
        "study-folder",
        "Your study folder",
        !current.watch_dir.trim().is_empty() && study.is_dir(),
        if current.watch_dir.trim().is_empty() {
            "Not chosen yet".to_string()
        } else if study.is_dir() {
            current.watch_dir.clone()
        } else {
            format!("This folder does not exist: {}", current.watch_dir)
        },
        "Choose the folder that holds your subject folders.",
        true,
    ));

    let automation_missing: Vec<&str> = [
        "guide_prompt.txt",
        "guide_depth_contract.md",
        "guide_lint.py",
        "requirements-verifier.txt",
    ]
    .into_iter()
    .filter(|file| !current.automation_path(file).is_file())
    .collect();
    requirements.push(requirement(
        "automation-folder",
        "The writing rules and checker",
        !current.automation_dir.trim().is_empty() && automation_missing.is_empty(),
        if current.automation_dir.trim().is_empty() {
            "Not chosen yet".to_string()
        } else if automation_missing.is_empty() {
            current.automation_dir.clone()
        } else {
            format!("Missing from that folder: {}", automation_missing.join(", "))
        },
        "Choose the _automation folder that came with Guide Watcher.",
        true,
    ));

    let subjects_ready = !current.courses.is_empty()
        && current
            .courses
            .iter()
            .all(|course| current.course_root(course).is_dir());
    requirements.push(requirement(
        "subjects",
        "At least one subject",
        subjects_ready,
        if current.courses.is_empty() {
            "None added yet".to_string()
        } else {
            current
                .courses
                .iter()
                .map(|course| {
                    let mark = if current.course_root(course).is_dir() { "" } else { " (folder missing)" };
                    format!("{}{mark}", course.label)
                })
                .collect::<Vec<_>>()
                .join(", ")
        },
        "Add each subject you want guides for, and point it at its folder.",
        true,
    ));

    let ready = requirements
        .iter()
        .all(|item| item.satisfied || !item.required);
    let (assistant_available, assistant_detail) =
        match resolve_provider_executable(ProviderExecutable::Claude) {
            Ok(path) => (
                true,
                format!("Claude Code is installed at {}", path.display()),
            ),
            Err(error) => (false, error),
        };

    SetupState {
        ready,
        requirements,
        settings: current,
        settings_path: settings::settings_path().to_string_lossy().into_owned(),
        assistant_available,
        assistant_detail,
    }
}

/// Save what the wizard collected, refusing anything that would not actually work.
pub fn save(settings: Settings) -> Result<SetupState, String> {
    let problems = settings.problems();
    if !problems.is_empty() {
        return Err(problems.join(" "));
    }
    settings::save(&settings)?;
    Ok(state())
}

/// A folder the app can suggest, so the first question is not a blank box: the place most people
/// keep documents.
pub fn suggested_study_folder() -> String {
    let home = std::env::var(if cfg!(target_os = "windows") { "USERPROFILE" } else { "HOME" })
        .unwrap_or_default();
    if home.is_empty() {
        return String::new();
    }
    let documents = PathBuf::from(&home).join("Documents");
    if documents.is_dir() {
        documents.to_string_lossy().into_owned()
    } else {
        home
    }
}

/// The automation folder that shipped beside the application, when it can be found. Saves asking.
pub fn bundled_automation_folder() -> String {
    let Ok(executable) = std::env::current_exe() else {
        return String::new();
    };
    let mut folder = executable.parent().map(Path::to_path_buf);
    // Walk up a few levels: installed next to the binary, or beside a development checkout.
    for _ in 0..4 {
        let Some(current) = folder.clone() else { break };
        let candidate = current.join("_automation");
        if candidate.join("guide_prompt.txt").is_file() {
            return candidate.to_string_lossy().into_owned();
        }
        folder = current.parent().map(Path::to_path_buf);
    }
    String::new()
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubjectDraft {
    pub label: String,
    pub folder: String,
    #[serde(default)]
    pub lecture_files: String,
}

/// Turn what the wizard collected into settings, deriving the parts the user should not have to
/// think about, such as a stable id per subject.
pub fn draft_settings(
    watch_dir: String,
    automation_dir: String,
    subjects: Vec<SubjectDraft>,
) -> Settings {
    let courses = subjects
        .into_iter()
        .filter(|subject| !subject.label.trim().is_empty())
        .map(|subject| crate::settings::CourseSetting {
            id: slug(&subject.label),
            label: subject.label.trim().to_string(),
            folder: subject.folder.trim().to_string(),
            lecture_files: subject.lecture_files.trim().to_string(),
            description: String::new(),
            mode: String::new(),
            kind: String::new(),
            // A new subject pins nothing; existing guides are pinned later, deliberately.
            pinned_guides: Vec::new(),
        })
        .collect();
    Settings {
        watch_dir: watch_dir.trim().to_string(),
        automation_dir: automation_dir.trim().to_string(),
        archive_dir: String::new(),
        courses,
        tools: settings::current().tools,
    }
}

/// A stable, readable id from a subject's name.
fn slug(label: &str) -> String {
    let mut out = String::new();
    for character in label.trim().to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            out.push(character);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "subject".to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subject_name_becomes_a_readable_stable_id() {
        assert_eq!(slug("Computer Networks"), "computer-networks");
        assert_eq!(slug("  Circuit Theory & Measurement Lab "), "circuit-theory-measurement-lab");
        assert_eq!(slug("Anatomy"), "anatomy");
        // A name with nothing usable still produces a valid id rather than an empty one.
        assert_eq!(slug("***"), "subject");
        assert_eq!(slug(""), "subject");
    }

    #[test]
    fn the_wizard_only_keeps_subjects_that_were_actually_named() {
        let settings = draft_settings(
            " C:/Study ".to_string(),
            "C:/Study/_automation".to_string(),
            vec![
                SubjectDraft { label: "Photography".into(), folder: "Photography".into(), lecture_files: "^lesson".into() },
                SubjectDraft { label: "   ".into(), folder: "Ignored".into(), lecture_files: String::new() },
            ],
        );
        assert_eq!(settings.watch_dir, "C:/Study");
        assert_eq!(settings.courses.len(), 1);
        assert_eq!(settings.courses[0].id, "photography");
        assert_eq!(settings.courses[0].lecture_files, "^lesson");
    }

    #[test]
    fn saving_refuses_a_setup_that_would_fail_later() {
        let broken = Settings {
            watch_dir: "C:/definitely/not/here".to_string(),
            ..Settings::default()
        };
        let error = save(broken).unwrap_err();
        assert!(error.contains("does not exist") || error.contains("study folder"), "{error}");
    }

    #[test]
    fn every_requirement_tells_the_user_what_to_do_when_it_fails() {
        for item in state().requirements {
            assert!(!item.label.is_empty());
            assert!(!item.detail.is_empty(), "{} has no detail", item.id);
            if !item.satisfied && item.required {
                assert!(!item.fix.is_empty(), "{} fails without telling the user how to fix it", item.id);
            }
        }
    }
}
