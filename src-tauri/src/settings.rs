//! What this installation is pointed at, read at run time.
//!
//! These values used to be compiled in, which meant a downloaded build was hard-wired to the
//! machine that built it and could never be configured by the person who installed it. They now
//! live in one JSON file in the user's own configuration directory, written by the setup wizard
//! or by hand, and read once per process.
//!
//! Nothing here panics when the file is missing. A fresh installation has no settings yet, which
//! is a normal state that the app answers with its wizard, so every accessor returns something
//! harmless and `is_configured()` reports the truth.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

/// A guide that must be preserved rather than regenerated: an existing week's work that the app
/// keeps and builds on. The checksum is what proves the file is still the one that was pinned.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PinnedGuide {
    /// Absolute, or relative to the subject's folder.
    pub file: String,
    pub sha256: String,
    /// Where this guide sits in the subject's order, such as "week-01".
    pub sequence_key: String,
}

/// One subject to watch.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CourseSetting {
    pub id: String,
    pub label: String,
    /// Absolute, or relative to `watch_dir`.
    pub folder: String,
    /// Case-insensitive regular expression a file name must match to start a guide. Empty means
    /// every supported file in the folder counts, which is what most subjects want.
    #[serde(default)]
    pub lecture_files: String,
    #[serde(default)]
    pub description: String,
    /// "lecture-deck" (the default), "weekly-lab", "weekly-material", "legacy-auto".
    #[serde(default)]
    pub mode: String,
    /// "lecture" (the default), "circuit-lab", "scientific-writing-week", "course-week".
    #[serde(default)]
    pub kind: String,
    /// Guides already written that must be kept rather than regenerated.
    #[serde(default)]
    pub pinned_guides: Vec<PinnedGuide>,
}

/// Tools that only some features need. An empty path simply means that feature is unavailable,
/// never a broken installation.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct OptionalTools {
    #[serde(default)]
    pub lms_agent_root: String,
    #[serde(default)]
    pub lms_node_executable: String,
    #[serde(default)]
    pub lms_tsx_cli: String,
    #[serde(default)]
    pub ffmpeg_executable: String,
    #[serde(default)]
    pub ffprobe_executable: String,
    #[serde(default)]
    pub transcription_uv_executable: String,
    #[serde(default)]
    pub transcription_script: String,
    #[serde(default)]
    pub circuit_lab_course_id: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Settings {
    /// The folder holding the subject folders.
    pub watch_dir: String,
    /// The folder holding the writing rules and the checker, shipped as `_automation`.
    pub automation_dir: String,
    /// An older folder kept for its finished guides. Optional.
    pub archive_dir: String,
    pub courses: Vec<CourseSetting>,
    pub tools: OptionalTools,
}

impl Settings {
    /// Whether this installation has been told enough to write a guide.
    #[allow(dead_code)]
    pub fn is_configured(&self) -> bool {
        self.problems().is_empty()
    }

    /// Everything standing between this installation and its first guide, in the words the
    /// wizard shows the user. An empty list means it is ready.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.watch_dir.trim().is_empty() {
            problems.push("No study folder chosen yet.".to_string());
        } else if !Path::new(&self.watch_dir).is_dir() {
            problems.push(format!("The study folder does not exist: {}", self.watch_dir));
        }
        if self.automation_dir.trim().is_empty() {
            problems.push("No automation folder chosen yet.".to_string());
        } else {
            for (file, what) in [
                ("guide_prompt.txt", "the writing rules"),
                ("guide_depth_contract.md", "the depth contract"),
                ("guide_lint.py", "the checker"),
                ("requirements-verifier.txt", "the checker's pinned packages"),
            ] {
                if !self.automation_path(file).is_file() {
                    problems.push(format!(
                        "The automation folder is missing {what} ({file}): {}",
                        self.automation_dir
                    ));
                }
            }
        }
        if self.courses.is_empty() {
            problems.push("No subjects added yet.".to_string());
        }
        for course in &self.courses {
            if course.id.trim().is_empty() || course.label.trim().is_empty() {
                problems.push("A subject is missing its name.".to_string());
            } else if !self.course_root(course).is_dir() {
                problems.push(format!(
                    "The folder for {} does not exist: {}",
                    course.label,
                    self.course_root(course).display()
                ));
            }
        }
        problems
    }

    /// Where a subject's material lives. A folder may be absolute, or relative to the study
    /// folder, which is what most people will type.
    pub fn course_root(&self, course: &CourseSetting) -> PathBuf {
        let folder = course.folder.trim();
        if folder.is_empty() {
            return PathBuf::from(&self.watch_dir);
        }
        if is_absolute(folder) {
            PathBuf::from(folder)
        } else {
            Path::new(&self.watch_dir).join(folder)
        }
    }

    /// Where a pinned guide lives: absolute as given, or inside the subject's folder.
    pub fn pinned_guide_path(&self, course: &CourseSetting, pinned: &PinnedGuide) -> PathBuf {
        if is_absolute(&pinned.file) {
            PathBuf::from(&pinned.file)
        } else {
            self.course_root(course).join(&pinned.file)
        }
    }

    pub fn automation_path(&self, file: &str) -> PathBuf {
        Path::new(&self.automation_dir).join(file)
    }

    /// The archive folder, or the study folder when none was given, so callers never have to
    /// handle an empty path.
    pub fn archive_dir(&self) -> String {
        if self.archive_dir.trim().is_empty() {
            self.watch_dir.clone()
        } else {
            self.archive_dir.clone()
        }
    }
}

/// Windows drive letters, UNC paths, and POSIX roots.
fn is_absolute(path: &str) -> bool {
    let path = path.trim();
    path.starts_with('/')
        || path.starts_with('\\')
        || (path.len() > 1 && path.as_bytes()[1] == b':')
}

/// Where this user's settings file lives, per platform convention.
pub fn settings_path() -> PathBuf {
    if let Ok(explicit) = std::env::var("GUIDE_WATCHER_SETTINGS") {
        if !explicit.trim().is_empty() {
            return PathBuf::from(explicit);
        }
    }
    let base = if cfg!(target_os = "windows") {
        std::env::var("APPDATA").ok().map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var("HOME")
            .ok()
            .map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|home| PathBuf::from(home).join(".config"))
            })
    };
    base.unwrap_or_else(std::env::temp_dir)
        .join("com.mushfique.guide-watcher")
        .join("settings.json")
}

fn store() -> &'static RwLock<Settings> {
    static STORE: OnceLock<RwLock<Settings>> = OnceLock::new();
    STORE.get_or_init(|| RwLock::new(read_from_disk().unwrap_or_default()))
}

fn read_from_disk() -> Option<Settings> {
    let text = std::fs::read_to_string(settings_path()).ok()?;
    match serde_json::from_str(&text) {
        Ok(settings) => Some(settings),
        Err(error) => {
            // A corrupt file must not look like a fresh installation, or the wizard would
            // silently overwrite settings the user spent time on.
            eprintln!(
                "Guide Watcher could not read {}: {error}",
                settings_path().display()
            );
            None
        }
    }
}

/// This installation's settings. Cheap after the first call.
pub fn current() -> Settings {
    store().read().map(|value| value.clone()).unwrap_or_default()
}

/// Write new settings and use them immediately, so the wizard's last step takes effect without
/// a restart.
pub fn save(settings: &Settings) -> Result<(), String> {
    save_to(&settings_path(), settings)?;
    if let Ok(mut guard) = store().write() {
        *guard = settings.clone();
    }
    Ok(())
}

/// Read settings from one file. Separate from the process-wide cache, so exercising the file
/// format never changes what the rest of the program sees.
#[allow(dead_code)]
pub fn load_from(path: &Path) -> Result<Settings, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("{} is not valid settings: {error}", path.display()))
}

/// Write settings to one file, creating its folder.
pub fn save_to(path: &Path, settings: &Settings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(settings)
        .map_err(|error| format!("could not serialise the settings: {error}"))?;
    std::fs::write(path, text + "\n")
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let path = std::env::temp_dir().join(format!("gw-settings-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn a_fresh_installation_is_unconfigured_and_says_exactly_what_it_needs() {
        let empty = Settings::default();
        assert!(!empty.is_configured());
        let problems = empty.problems().join(" | ");
        assert!(problems.contains("study folder"), "{problems}");
        assert!(problems.contains("automation folder"), "{problems}");
        assert!(problems.contains("subjects"), "{problems}");
    }

    #[test]
    fn a_complete_setup_reports_ready_and_a_broken_one_names_the_missing_piece() {
        let root = scratch();
        let automation = root.join("_automation");
        std::fs::create_dir_all(automation.join("sub")).unwrap();
        for file in [
            "guide_prompt.txt",
            "guide_depth_contract.md",
            "guide_lint.py",
            "requirements-verifier.txt",
        ] {
            std::fs::write(automation.join(file), "x").unwrap();
        }
        let study = root.join("Study");
        std::fs::create_dir_all(study.join("Photography")).unwrap();

        let mut settings = Settings {
            watch_dir: study.to_string_lossy().into_owned(),
            automation_dir: automation.to_string_lossy().into_owned(),
            archive_dir: String::new(),
            courses: vec![CourseSetting {
                id: "photography".into(),
                label: "Photography".into(),
                folder: "Photography".into(),
                ..CourseSetting::default()
            }],
            tools: OptionalTools::default(),
        };
        assert!(settings.is_configured(), "{:?}", settings.problems());
        // A relative folder resolves under the study folder; the archive falls back to it.
        assert_eq!(settings.course_root(&settings.courses[0]), study.join("Photography"));
        assert_eq!(settings.archive_dir(), settings.watch_dir);

        // Each broken piece is named in the words the wizard shows.
        settings.courses[0].folder = "Missing Subject".into();
        assert!(settings.problems()[0].contains("Photography"), "{:?}", settings.problems());

        settings.courses[0].folder = "Photography".into();
        std::fs::remove_file(automation.join("guide_lint.py")).unwrap();
        assert!(settings.problems()[0].contains("the checker"), "{:?}", settings.problems());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn settings_round_trip_through_the_file_the_wizard_writes() {
        let home = scratch();
        let file = home.join("settings.json");
        let settings = Settings {
            watch_dir: "C:/Study".into(),
            automation_dir: "C:/Study/_automation".into(),
            archive_dir: String::new(),
            courses: vec![CourseSetting {
                id: "anatomy".into(),
                label: "Anatomy".into(),
                folder: "Anatomy".into(),
                lecture_files: "^week[0-9]+".into(),
                ..CourseSetting::default()
            }],
            tools: OptionalTools::default(),
        };
        save_to(&file, &settings).unwrap();
        assert_eq!(load_from(&file).unwrap(), settings);

        // A file written by an older version, before a field existed, still loads: an update
        // must never silently wipe someone's setup.
        std::fs::write(&file, r#"{"watch_dir":"C:/Only"}"#).unwrap();
        let sparse = load_from(&file).unwrap();
        assert_eq!(sparse.watch_dir, "C:/Only");
        assert!(sparse.courses.is_empty());

        // A corrupt file is an error, never silently an empty setup that the wizard would
        // then overwrite.
        std::fs::write(&file, "{ not json").unwrap();
        assert!(load_from(&file).is_err());
        std::fs::remove_dir_all(home).unwrap();
    }
}
