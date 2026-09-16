use std::collections::HashSet;
use std::env;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderExecutable {
    Codex,
    Claude,
}

impl ProviderExecutable {
    fn display_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex CLI",
            Self::Claude => "Claude Code",
        }
    }

    fn executable_name(self) -> &'static str {
        #[cfg(windows)]
        match self {
            Self::Codex => "codex.exe",
            Self::Claude => "claude.exe",
        }
        #[cfg(not(windows))]
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    fn override_name(self) -> &'static str {
        match self {
            Self::Codex => "CODEX_CLI_PATH",
            Self::Claude => "CLAUDE_CODE_CLI_PATH",
        }
    }
}

#[derive(Debug, Default)]
struct SearchEnvironment {
    explicit_override: Option<OsString>,
    process_path: Option<OsString>,
    user_profile: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
    roaming_app_data: Option<PathBuf>,
    /// The Unix home directory. A desktop launch on macOS or Linux inherits a minimal PATH, so
    /// the usual install locations have to be searched explicitly.
    home: Option<PathBuf>,
}

impl SearchEnvironment {
    fn capture(provider: ProviderExecutable) -> Self {
        Self {
            explicit_override: nonempty_environment(provider.override_name()),
            process_path: nonempty_environment("PATH"),
            user_profile: nonempty_environment("USERPROFILE").map(PathBuf::from),
            local_app_data: nonempty_environment("LOCALAPPDATA").map(PathBuf::from),
            roaming_app_data: nonempty_environment("APPDATA").map(PathBuf::from),
            home: nonempty_environment("HOME").map(PathBuf::from),
        }
    }
}

fn nonempty_environment(name: &str) -> Option<OsString> {
    env::var_os(name).filter(|value| !value.is_empty())
}

pub(crate) fn resolve_provider_executable(provider: ProviderExecutable) -> Result<PathBuf, String> {
    resolve_with_environment(provider, &SearchEnvironment::capture(provider))
}

fn resolve_with_environment(
    provider: ProviderExecutable,
    environment: &SearchEnvironment,
) -> Result<PathBuf, String> {
    if let Some(explicit) = &environment.explicit_override {
        return validate_explicit_override(provider, explicit);
    }

    let mut candidates = Vec::new();
    if let Some(path) = &environment.process_path {
        candidates.extend(
            env::split_paths(path)
                .filter(|directory| directory.is_absolute())
                .map(|directory| directory.join(provider.executable_name())),
        );
    }
    candidates.extend(standard_candidates(provider, environment));

    let mut seen = HashSet::new();
    for candidate in candidates {
        if !seen.insert(normalized_candidate_identity(&candidate)) {
            continue;
        }
        if let Some(executable) = canonical_regular_file(&candidate) {
            return Ok(executable);
        }
    }

    Err(format!(
        "{} was not found from this desktop launch. Install its native CLI or set {} to an absolute executable file, then restart Guide Watcher.",
        provider.display_name(),
        provider.override_name()
    ))
}

fn validate_explicit_override(
    provider: ProviderExecutable,
    explicit: &OsStr,
) -> Result<PathBuf, String> {
    let path = PathBuf::from(explicit);
    if !path.is_absolute() {
        return Err(format!(
            "{} must be an absolute path to a native executable",
            provider.override_name()
        ));
    }
    canonical_regular_file(&path).ok_or_else(|| {
        format!(
            "{} does not resolve to a regular executable file: {}",
            provider.override_name(),
            path.display()
        )
    })
}

fn canonical_regular_file(path: &Path) -> Option<PathBuf> {
    let canonical = path.canonicalize().ok()?;
    canonical.is_file().then_some(canonical)
}

fn normalized_candidate_identity(path: &Path) -> String {
    let mut identity = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        identity.make_ascii_lowercase();
    }
    identity
}

/// Where these CLIs install on macOS and Linux. Empty on Windows.
fn unix_candidates(
    provider: ProviderExecutable,
    environment: &SearchEnvironment,
) -> Vec<PathBuf> {
    if cfg!(windows) {
        return Vec::new();
    }
    let name = provider.executable_name();
    let mut candidates = Vec::new();
    if let Some(home) = &environment.home {
        for relative in [".local/bin", ".bun/bin", ".npm-global/bin", ".volta/bin"] {
            candidates.push(home.join(relative).join(name));
        }
        if provider.override_name().contains("CODEX") {
            candidates.push(
                home.join(".codex")
                    .join("packages")
                    .join("standalone")
                    .join("current")
                    .join("bin")
                    .join(name),
            );
        }
    }
    for absolute in [
        "/opt/homebrew/bin", // Homebrew on Apple silicon, absent from a Finder launch's PATH
        "/usr/local/bin",
        "/usr/bin",
        "/snap/bin",
        "/var/lib/flatpak/exports/bin",
    ] {
        candidates.push(PathBuf::from(absolute).join(name));
    }
    candidates
}

fn standard_candidates(
    provider: ProviderExecutable,
    environment: &SearchEnvironment,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    candidates.extend(unix_candidates(provider, environment));
    match provider {
        ProviderExecutable::Codex => {
            if let Some(user) = &environment.user_profile {
                candidates.push(
                    user.join(".codex")
                        .join("packages")
                        .join("standalone")
                        .join("current")
                        .join("bin")
                        .join(provider.executable_name()),
                );
            }
            if let Some(local) = &environment.local_app_data {
                candidates.push(
                    local
                        .join("Programs")
                        .join("OpenAI")
                        .join("Codex")
                        .join("bin")
                        .join(provider.executable_name()),
                );
                candidates.push(
                    local
                        .join("Microsoft")
                        .join("WinGet")
                        .join("Links")
                        .join(provider.executable_name()),
                );
                candidates.extend(codex_desktop_candidates(local));
            }
            if let Some(roaming) = &environment.roaming_app_data {
                for (package, architecture) in [
                    ("codex-win32-x64", "x86_64-pc-windows-msvc"),
                    ("codex-win32-arm64", "aarch64-pc-windows-msvc"),
                ] {
                    candidates.push(
                        roaming
                            .join("npm")
                            .join("node_modules")
                            .join("@openai")
                            .join("codex")
                            .join("node_modules")
                            .join("@openai")
                            .join(package)
                            .join("vendor")
                            .join(architecture)
                            .join("bin")
                            .join(provider.executable_name()),
                    );
                }
            }
        }
        ProviderExecutable::Claude => {
            if let Some(user) = &environment.user_profile {
                candidates.push(
                    user.join(".local")
                        .join("bin")
                        .join(provider.executable_name()),
                );
                candidates.push(
                    user.join(".claude")
                        .join("local")
                        .join(provider.executable_name()),
                );
            }
            if let Some(local) = &environment.local_app_data {
                candidates.push(
                    local
                        .join("Microsoft")
                        .join("WinGet")
                        .join("Links")
                        .join(provider.executable_name()),
                );
            }
        }
    }
    candidates
}

fn codex_desktop_candidates(local_app_data: &Path) -> Vec<PathBuf> {
    let bin_root = local_app_data.join("OpenAI").join("Codex").join("bin");
    let Ok(entries) = std::fs::read_dir(bin_root) else {
        return Vec::new();
    };
    let mut candidates = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("codex.exe"))
        .filter_map(|path| {
            let modified = path.metadata().ok()?.modified().unwrap_or(UNIX_EPOCH);
            path.is_file().then_some((modified, path))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|(left_time, left_path), (right_time, right_path)| {
        right_time
            .cmp(left_time)
            .then_with(|| left_path.cmp(right_path))
    });
    candidates.into_iter().map(|(_, path)| path).collect()
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use uuid::Uuid;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "guide-watcher-provider-executable-test-{}",
                Uuid::new_v4()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn environment(root: &Path) -> SearchEnvironment {
        SearchEnvironment {
            process_path: Some(OsString::from(r"C:\Windows\System32")),
            user_profile: Some(root.join("User")),
            local_app_data: Some(root.join("Local")),
            roaming_app_data: Some(root.join("Roaming")),
            ..SearchEnvironment::default()
        }
    }

    fn create_file(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"test executable").unwrap();
    }

    #[test]
    fn explorer_launch_finds_codex_desktop_binary_without_terminal_path() {
        let root = TestDir::new();
        let expected = root.0.join("Local/OpenAI/Codex/bin/release-id/codex.exe");
        create_file(&expected);

        let resolved =
            resolve_with_environment(ProviderExecutable::Codex, &environment(&root.0)).unwrap();

        assert_eq!(resolved, expected.canonicalize().unwrap());
    }

    #[test]
    fn explorer_launch_finds_claude_native_binary_without_terminal_path() {
        let root = TestDir::new();
        let expected = root.0.join("User/.local/bin/claude.exe");
        create_file(&expected);

        let resolved =
            resolve_with_environment(ProviderExecutable::Claude, &environment(&root.0)).unwrap();

        assert_eq!(resolved, expected.canonicalize().unwrap());
    }

    #[test]
    fn explicit_override_is_authoritative() {
        let root = TestDir::new();
        let expected = root.0.join("chosen/codex.exe");
        let ignored = root.0.join("Local/OpenAI/Codex/bin/release/codex.exe");
        create_file(&expected);
        create_file(&ignored);
        let mut search = environment(&root.0);
        search.explicit_override = Some(expected.as_os_str().to_os_string());

        let resolved = resolve_with_environment(ProviderExecutable::Codex, &search).unwrap();

        assert_eq!(resolved, expected.canonicalize().unwrap());
    }

    #[test]
    fn malformed_override_fails_instead_of_running_an_ambiguous_fallback() {
        let root = TestDir::new();
        let fallback = root.0.join("Local/OpenAI/Codex/bin/release/codex.exe");
        create_file(&fallback);
        let mut search = environment(&root.0);
        search.explicit_override = Some(OsString::from("relative/codex.exe"));

        let error = resolve_with_environment(ProviderExecutable::Codex, &search).unwrap_err();

        assert!(error.contains("CODEX_CLI_PATH must be an absolute path"));
    }

    #[test]
    fn path_search_ignores_relative_entries_directories_and_cmd_shims() {
        let root = TestDir::new();
        let shadow = root.0.join("shadow");
        std::fs::create_dir_all(shadow.join("codex.exe")).unwrap();
        std::fs::write(shadow.join("codex.cmd"), b"untrusted shim").unwrap();
        let search = SearchEnvironment {
            process_path: Some(env::join_paths([PathBuf::from("relative"), shadow]).unwrap()),
            ..SearchEnvironment::default()
        };

        let error = resolve_with_environment(ProviderExecutable::Codex, &search).unwrap_err();

        assert!(error.contains("Codex CLI was not found"));
    }

    #[test]
    fn missing_provider_error_names_the_recovery_variable() {
        let error =
            resolve_with_environment(ProviderExecutable::Claude, &SearchEnvironment::default())
                .unwrap_err();

        assert!(error.contains("Claude Code was not found"));
        assert!(error.contains("CLAUDE_CODE_CLI_PATH"));
    }

    #[test]
    fn desktop_candidate_order_prefers_the_newest_existing_binary() {
        let root = TestDir::new();
        let local = root.0.join("Local");
        let older = local.join("OpenAI/Codex/bin/older/codex.exe");
        let newer = local.join("OpenAI/Codex/bin/newer/codex.exe");
        create_file(&older);
        std::thread::sleep(std::time::Duration::from_millis(20));
        create_file(&newer);

        let candidates = codex_desktop_candidates(&local);

        assert_eq!(candidates.first(), Some(&newer));
        assert_eq!(candidates.get(1), Some(&older));
    }
}
