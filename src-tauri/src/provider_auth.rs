use crate::process_registry;
use crate::provider_executable::{resolve_provider_executable, ProviderExecutable};
use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;
use std::ffi::OsString;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use uuid::Uuid;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;
const STATUS_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_AUTH_OUTPUT_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Provider {
    Codex,
    Claude,
}

impl Provider {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "codex-chatgpt" => Ok(Self::Codex),
            "claude-code" => Ok(Self::Claude),
            _ => Err(format!("unsupported authentication provider: {value}")),
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex-chatgpt",
            Self::Claude => "claude-code",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Codex => "OpenAI Codex",
            Self::Claude => "Claude Code",
        }
    }

    fn executable(self) -> ProviderExecutable {
        match self {
            Self::Codex => ProviderExecutable::Codex,
            Self::Claude => ProviderExecutable::Claude,
        }
    }

    fn status_args(self) -> Vec<OsString> {
        match self {
            Self::Codex => ["login", "status"]
                .into_iter()
                .map(OsString::from)
                .collect(),
            Self::Claude => ["auth", "status", "--json"]
                .into_iter()
                .map(OsString::from)
                .collect(),
        }
    }

    fn login_args(self) -> Vec<OsString> {
        match self {
            Self::Codex => ["login", "--device-auth"]
                .into_iter()
                .map(OsString::from)
                .collect(),
            Self::Claude => ["auth", "login", "--claudeai"]
                .into_iter()
                .map(OsString::from)
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAuthStatus {
    pub provider: String,
    pub label: String,
    pub installed: bool,
    pub authenticated: bool,
    pub account_label: Option<String>,
    pub auth_method: Option<String>,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAuthSession {
    pub provider: String,
    pub session_id: String,
    pub state: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderAuthEvent {
    provider: String,
    session_id: String,
    state: String,
    message: String,
    verification_uri: Option<String>,
    user_code: Option<String>,
}

#[derive(Clone, Copy)]
struct ActiveSession {
    token: process_registry::CancellationToken,
}

#[derive(Clone, Default)]
pub struct ProviderAuthState {
    active: Arc<Mutex<HashMap<String, (String, ActiveSession)>>>,
}

impl ProviderAuthState {
    fn reserve(
        &self,
        provider: Provider,
        session_id: &str,
        token: process_registry::CancellationToken,
    ) -> Result<(), String> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| "authentication session state is unavailable".to_string())?;
        if active.contains_key(provider.id()) {
            return Err(format!(
                "{} already has a sign-in session in progress",
                provider.label()
            ));
        }
        active.insert(
            provider.id().to_string(),
            (session_id.to_string(), ActiveSession { token }),
        );
        Ok(())
    }

    fn release(&self, provider: Provider, session_id: &str) {
        if let Ok(mut active) = self.active.lock() {
            let matches = active
                .get(provider.id())
                .is_some_and(|(active_id, _)| active_id == session_id);
            if matches {
                active.remove(provider.id());
            }
        }
    }

    pub fn cancel(&self, provider_id: &str, session_id: &str) -> Result<(), String> {
        let provider = Provider::parse(provider_id)?;
        let token = self
            .active
            .lock()
            .map_err(|_| "authentication session state is unavailable".to_string())?
            .get(provider.id())
            .filter(|(active_id, _)| active_id == session_id)
            .map(|(_, active)| active.token)
            .ok_or_else(|| "that sign-in session is no longer active".to_string())?;
        process_registry::cancel(token);
        Ok(())
    }
}

pub async fn statuses() -> Vec<ProviderAuthStatus> {
    let (codex, claude) = tokio::join!(status(Provider::Codex), status(Provider::Claude));
    vec![codex, claude]
}

pub fn start_login(
    app: AppHandle,
    state: ProviderAuthState,
    provider_id: String,
) -> Result<ProviderAuthSession, String> {
    let provider = Provider::parse(&provider_id)?;
    let executable = resolve_provider_executable(provider.executable())?;
    let session_id = Uuid::new_v4().to_string();
    let token = process_registry::cancellation_token();
    if let Err(error) = state.reserve(provider, &session_id, token) {
        process_registry::finish(token);
        return Err(error);
    }

    let returned = ProviderAuthSession {
        provider: provider.id().to_string(),
        session_id: session_id.clone(),
        state: "starting".to_string(),
    };
    tauri::async_runtime::spawn(async move {
        let result = run_login_process(&app, provider, &session_id, executable, token).await;
        if let Err(error) = result {
            emit_auth_event(&app, provider, &session_id, "failed", &error, None, None);
        }
        state.release(provider, &session_id);
        process_registry::finish(token);
    });
    Ok(returned)
}

async fn status(provider: Provider) -> ProviderAuthStatus {
    let executable = match resolve_provider_executable(provider.executable()) {
        Ok(path) => path,
        Err(_) => {
            return ProviderAuthStatus {
                provider: provider.id().to_string(),
                label: provider.label().to_string(),
                installed: false,
                authenticated: false,
                account_label: None,
                auth_method: None,
                detail: "Native CLI not found. Install it, then restart Guide Watcher.".to_string(),
            }
        }
    };
    let mut command = Command::new(executable);
    command
        .args(provider.status_args())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let output = match tokio::time::timeout(STATUS_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(_)) => {
            return unavailable_status(provider, "Could not check the CLI sign-in state.")
        }
        Err(_) => return unavailable_status(provider, "CLI sign-in check timed out."),
    };
    let stdout = bounded_text(&output.stdout);
    let stderr = bounded_text(&output.stderr);
    match provider {
        Provider::Codex => parse_codex_status(output.status.success(), &stdout, &stderr),
        Provider::Claude => parse_claude_status(output.status.success(), &stdout, &stderr),
    }
}

fn unavailable_status(provider: Provider, detail: &str) -> ProviderAuthStatus {
    ProviderAuthStatus {
        provider: provider.id().to_string(),
        label: provider.label().to_string(),
        installed: true,
        authenticated: false,
        account_label: None,
        auth_method: None,
        detail: detail.to_string(),
    }
}

fn parse_codex_status(success: bool, stdout: &str, stderr: &str) -> ProviderAuthStatus {
    let combined = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    let authenticated = success
        && !combined.contains("not logged in")
        && !combined.contains("login required")
        && combined.contains("logged in");
    ProviderAuthStatus {
        provider: Provider::Codex.id().to_string(),
        label: Provider::Codex.label().to_string(),
        installed: true,
        authenticated,
        account_label: authenticated.then(|| "ChatGPT account".to_string()),
        auth_method: authenticated.then(|| "ChatGPT".to_string()),
        detail: if authenticated {
            "Ready for Guide Watcher generation.".to_string()
        } else {
            "Sign in to use Codex for collection or fallback writing.".to_string()
        },
    }
}

fn parse_claude_status(success: bool, stdout: &str, _stderr: &str) -> ProviderAuthStatus {
    let value = serde_json::from_str::<serde_json::Value>(stdout).ok();
    let authenticated = success
        && value
            .as_ref()
            .and_then(|item| item.get("loggedIn"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
    let account_label = authenticated
        .then(|| {
            value
                .as_ref()
                .and_then(|item| item.get("email"))
                .and_then(serde_json::Value::as_str)
                .and_then(safe_account_label)
        })
        .flatten();
    let subscription = value
        .as_ref()
        .and_then(|item| item.get("subscriptionType"))
        .and_then(serde_json::Value::as_str)
        .filter(|item| matches!(*item, "pro" | "max" | "team" | "enterprise"));
    ProviderAuthStatus {
        provider: Provider::Claude.id().to_string(),
        label: Provider::Claude.label().to_string(),
        installed: true,
        authenticated,
        account_label,
        auth_method: authenticated.then(|| match subscription {
            Some(kind) => format!("Claude {}", uppercase_first(kind)),
            None => "Claude account".to_string(),
        }),
        detail: if authenticated {
            "Ready for Guide Watcher generation.".to_string()
        } else {
            "Sign in to use Claude for writing or Codex collection failover.".to_string()
        },
    }
}

fn uppercase_first(value: &str) -> String {
    let mut characters = value.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    }
}

fn safe_account_label(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()
        && trimmed.len() <= 254
        && trimmed.chars().all(|character| !character.is_control()))
    .then(|| trimmed.to_string())
}

async fn run_login_process(
    app: &AppHandle,
    provider: Provider,
    session_id: &str,
    executable: std::path::PathBuf,
    token: process_registry::CancellationToken,
) -> Result<(), String> {
    emit_auth_event(
        app,
        provider,
        session_id,
        "starting",
        match provider {
            Provider::Codex => "Preparing a secure device sign-in.",
            Provider::Claude => "Opening Claude sign-in in your browser.",
        },
        None,
        None,
    );
    let mut command = Command::new(executable);
    command
        .args(provider.login_args())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let mut child = command
        .spawn()
        .map_err(|_| format!("{} sign-in could not start.", provider.label()))?;
    let process_key = format!("provider-auth:{}:{session_id}", provider.id());
    match child.id() {
        Some(pid) if process_registry::register(&process_key, pid, token) => {}
        _ => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err("Sign-in was cancelled before the CLI could start.".to_string());
        }
    }

    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let stdout_task = spawn_line_reader(child.stdout.take(), sender.clone());
    let stderr_task = spawn_line_reader(child.stderr.take(), sender);
    let mut wait = Box::pin(child.wait());
    let mut captured = String::new();
    let mut last_uri = None;
    let mut last_code = None;
    let status_result = loop {
        tokio::select! {
            result = &mut wait => break result,
            line = receiver.recv() => {
                if let Some(line) = line {
                    append_bounded(&mut captured, &line);
                    let (uri, code) = parse_auth_hints(&captured);
                    if uri != last_uri || code != last_code {
                        last_uri = uri.clone();
                        last_code = code.clone();
                        emit_auth_event(
                            app,
                            provider,
                            session_id,
                            "waiting",
                            if code.is_some() {
                                "Copy the one-time code, then open the sign-in page."
                            } else if provider == Provider::Codex {
                                "Preparing the one-time device code…"
                            } else {
                                "Finish sign-in in your browser."
                            },
                            uri,
                            code,
                        );
                    }
                }
            }
        }
    };
    let _ = tokio::join!(stdout_task, stderr_task);
    while let Ok(line) = receiver.try_recv() {
        append_bounded(&mut captured, &line);
    }
    process_registry::unregister(&process_key);

    if process_registry::is_cancelled(token) {
        emit_auth_event(
            app,
            provider,
            session_id,
            "cancelled",
            "Sign-in was cancelled.",
            None,
            None,
        );
        return Ok(());
    }
    let exit_status =
        status_result.map_err(|_| format!("{} sign-in stopped unexpectedly.", provider.label()))?;
    if !exit_status.success() {
        return Err(format!(
            "{} sign-in was not completed. You can try again without entering credentials in Guide Watcher.",
            provider.label()
        ));
    }
    let current = status(provider).await;
    if !current.authenticated {
        return Err(format!(
            "{} finished its sign-in flow, but the account is not available yet.",
            provider.label()
        ));
    }
    emit_auth_event(
        app,
        provider,
        session_id,
        "succeeded",
        "Account connected. New guide runs will use this CLI account.",
        None,
        None,
    );
    Ok(())
}

fn spawn_line_reader<R>(
    stream: Option<R>,
    sender: tokio::sync::mpsc::UnboundedSender<String>,
) -> tokio::task::JoinHandle<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        if let Some(stream) = stream {
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if sender.send(line).is_err() {
                    break;
                }
            }
        }
    })
}

fn append_bounded(captured: &mut String, line: &str) {
    captured.push_str(line);
    captured.push('\n');
    if captured.len() > MAX_AUTH_OUTPUT_BYTES {
        let mut start = captured.len() - MAX_AUTH_OUTPUT_BYTES;
        while start < captured.len() && !captured.is_char_boundary(start) {
            start += 1;
        }
        captured.drain(..start);
    }
}

fn parse_auth_hints(text: &str) -> (Option<String>, Option<String>) {
    static URL: OnceLock<Regex> = OnceLock::new();
    static CODE: OnceLock<Regex> = OnceLock::new();
    static ANSI: OnceLock<Regex> = OnceLock::new();
    // Codex colorizes the values. The trailing `m` in `\x1b[94m`
    // otherwise touches the first code character and defeats `\b` below.
    let ansi_pattern = ANSI.get_or_init(|| Regex::new(r"\x1B\[[0-?]*[ -/]*[@-~]").unwrap());
    let plain = ansi_pattern.replace_all(text, "");
    let url_pattern =
        URL.get_or_init(|| Regex::new(r"https://[A-Za-z0-9._~:/?#\[\]@!$&*+,;=%-]+").unwrap());
    let code_pattern =
        CODE.get_or_init(|| Regex::new(r"\b[A-Z0-9]{4,8}(?:-[A-Z0-9]{4,8})+\b").unwrap());
    let uri = url_pattern
        .find_iter(&plain)
        .map(|value| value.as_str().trim_end_matches(['.', ',', ')', ']']))
        .find(|value| {
            [
                "https://auth.openai.com/",
                "https://platform.openai.com/",
                "https://chatgpt.com/",
                "https://claude.ai/",
                "https://console.anthropic.com/",
            ]
            .iter()
            .any(|allowed| value.starts_with(allowed))
        })
        .map(str::to_string);
    let code = code_pattern
        .find(&plain)
        .map(|value| value.as_str().to_string());
    (uri, code)
}

fn emit_auth_event(
    app: &AppHandle,
    provider: Provider,
    session_id: &str,
    state: &str,
    message: &str,
    verification_uri: Option<String>,
    user_code: Option<String>,
) {
    let _ = app.emit(
        "provider-auth",
        ProviderAuthEvent {
            provider: provider.id().to_string(),
            session_id: session_id.to_string(),
            state: state.to_string(),
            message: message.to_string(),
            verification_uri,
            user_code,
        },
    );
}

fn bounded_text(bytes: &[u8]) -> String {
    let start = bytes.len().saturating_sub(MAX_AUTH_OUTPUT_BYTES);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_contract_uses_only_native_subscription_login_flows() {
        assert_eq!(
            Provider::Codex.login_args(),
            ["login", "--device-auth"].map(OsString::from)
        );
        assert_eq!(
            Provider::Claude.login_args(),
            ["auth", "login", "--claudeai"].map(OsString::from)
        );
        for provider in [Provider::Codex, Provider::Claude] {
            let joined = provider
                .login_args()
                .iter()
                .map(|item| item.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ");
            assert!(!joined.contains("token"));
            assert!(!joined.contains("api-key"));
            assert!(!joined.contains("password"));
        }
    }

    #[test]
    fn status_parsers_do_not_treat_negative_or_malformed_output_as_authenticated() {
        assert!(!parse_codex_status(true, "Not logged in", "").authenticated);
        assert!(!parse_codex_status(false, "Logged in using ChatGPT", "").authenticated);
        assert!(parse_codex_status(true, "Logged in using ChatGPT", "").authenticated);
        assert!(!parse_claude_status(true, "not-json", "").authenticated);
        assert!(!parse_claude_status(false, r#"{"loggedIn":true}"#, "").authenticated);
    }

    #[test]
    fn claude_status_exposes_bounded_account_identity_without_org_or_token_fields() {
        let parsed = parse_claude_status(
            true,
            r#"{"loggedIn":true,"email":"student@example.com","subscriptionType":"max","orgId":"secret-internal-id"}"#,
            "",
        );
        assert_eq!(parsed.account_label.as_deref(), Some("student@example.com"));
        assert_eq!(parsed.auth_method.as_deref(), Some("Claude Max"));
        let serialized = serde_json::to_string(&parsed).unwrap();
        assert!(!serialized.contains("orgId"));
        assert!(!serialized.contains("secret-internal-id"));
    }

    #[test]
    fn auth_hint_parser_accepts_only_known_https_hosts_and_device_code_shape() {
        let (uri, code) =
            parse_auth_hints("Open https://auth.openai.com/codex/device. Enter code ABCD-EFGH.");
        assert_eq!(uri.as_deref(), Some("https://auth.openai.com/codex/device"));
        assert_eq!(code.as_deref(), Some("ABCD-EFGH"));

        let (uri, code) = parse_auth_hints(
            "Ignore this https://evil.example/login and id 9fb859a6-e24c-4a81-acaa-3535005a7e56",
        );
        assert_eq!(uri, None);
        assert_eq!(code, None);
    }

    #[test]
    fn auth_hint_parser_handles_the_real_codex_colored_device_prompt() {
        let prompt = "1. Open this link\n \x1b[94mhttps://auth.openai.com/codex/device\x1b[0m\n\n2. Enter this one-time code\n \x1b[94mABCD-EFGH\x1b[0m\n";
        let (uri, code) = parse_auth_hints(prompt);
        assert_eq!(uri.as_deref(), Some("https://auth.openai.com/codex/device"));
        assert_eq!(code.as_deref(), Some("ABCD-EFGH"));
    }

    #[test]
    fn duplicate_sessions_and_stale_cancellation_are_rejected() {
        let state = ProviderAuthState::default();
        let first = process_registry::cancellation_token();
        let second = process_registry::cancellation_token();
        state.reserve(Provider::Codex, "one", first).unwrap();
        assert!(state.reserve(Provider::Codex, "two", second).is_err());
        assert!(state.cancel(Provider::Codex.id(), "stale").is_err());
        state.cancel(Provider::Codex.id(), "one").unwrap();
        assert!(process_registry::is_cancelled(first));
        state.release(Provider::Codex, "one");
        process_registry::finish(first);
        process_registry::finish(second);
    }
}
