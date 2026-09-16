// Claude Code subscription runner.
//
// Guide Watcher owns all writes. Claude receives an inert prompt on stdin and
// can use only read-only discovery tools.
use crate::artifact_bundle::{ARTIFACTS_END, ARTIFACTS_START};
use crate::job_events::{emit_output, emit_phase, ProgressSink};
use crate::process_registry;
use crate::provider_executable::{resolve_provider_executable, ProviderExecutable};
use serde_json::Value;
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaudeModelSpec {
    pub model: String,
    pub effort: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClaudeFailureKind {
    Cancelled,
    AvailabilityEntitlementOrQuota,
    /// The CLI is logged out or the account lacks permission; another provider can still write.
    Authentication,
    /// The network or the API connection failed; another provider can still write.
    Transport,
    ContentRejected,
    /// The CLI failed in a way the app does not recognize, having produced no guide at all.
    /// A non-zero exit is never a writing mistake - a bad guide exits zero and is caught by the
    /// verifier - so another writer should still be tried.
    Unrecognized,
    Other,
}

impl ClaudeFailureKind {
    /// Claude cannot serve this request right now, through no fault of the prompt or the
    /// draft: try the next Claude candidate, then the Codex fallback.
    pub(crate) fn is_provider_unavailable(self) -> bool {
        matches!(
            self,
            ClaudeFailureKind::AvailabilityEntitlementOrQuota
                | ClaudeFailureKind::Authentication
                | ClaudeFailureKind::Transport
                | ClaudeFailureKind::Unrecognized
        )
    }
}

#[derive(Debug)]
pub(crate) struct ClaudePhaseError {
    pub kind: ClaudeFailureKind,
    pub message: String,
}

#[derive(Debug)]
pub(crate) struct ClaudeRunSuccess {
    pub model: String,
    pub effort: String,
}

/// Relative layout of the writer harness inside a provider workspace.
pub(crate) const DRAFT_DIR: &str = "draft";
pub(crate) const DRAFT_GUIDE: &str = "draft/guide.md";
pub(crate) const DRAFT_ARTIFACTS: &str = "draft/artifacts.json";
/// The primary writer's record of how this guide is written, handed to any fallback writer.
pub(crate) const DRAFT_STYLE_PLAN: &str = "draft/style_plan.md";
pub(crate) const VERIFY_SCRIPT: &str = "verify_draft.sh";

/// How a Claude run delivers its result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClaudeHarness {
    /// Read-only tools; the guide is the final response text.
    ResponseText,
    /// Tool-using writer: it may create and edit files only under `draft/` and run only
    /// `./verify_draft.sh`; the result is read from `draft/guide.md` (+ `draft/artifacts.json`).
    DraftWorkspace,
}

pub(crate) struct ClaudeFileRequest<'a> {
    pub working_dir: &'a Path,
    pub output_path: &'a Path,
    pub prompt: String,
    pub candidates: &'a [ClaudeModelSpec],
    pub enable_web_search: bool,
    pub harness: ClaudeHarness,
}

#[derive(Debug)]
struct ClaudeInvocation {
    working_dir: PathBuf,
    args: Vec<OsString>,
    stdin: String,
}

#[derive(Debug)]
struct ProcessOutput {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

pub(crate) async fn run_claude_to_file<S: ProgressSink + ?Sized>(
    app: &S,
    job_id: &str,
    request: ClaudeFileRequest<'_>,
    cancellation: process_registry::CancellationToken,
) -> Result<ClaudeRunSuccess, ClaudePhaseError> {
    if request.candidates.is_empty() {
        return Err(ClaudePhaseError {
            kind: ClaudeFailureKind::Other,
            message: "Claude candidate list must not be empty".to_string(),
        });
    }
    if request.output_path.exists() {
        return Err(ClaudePhaseError {
            kind: ClaudeFailureKind::Other,
            message: format!(
                "refusing to reuse Claude staging path: {}",
                request.output_path.display()
            ),
        });
    }

    let mut last_error = None;
    for (index, candidate) in request.candidates.iter().enumerate() {
        if index > 0 {
            emit_phase(
                app,
                job_id,
                &format!(
                    "Claude fallback {}/{} ({} · {})",
                    index + 1,
                    request.candidates.len(),
                    candidate.model,
                    candidate.effort
                ),
            );
        }
        let invocation = build_claude_invocation(
            request.working_dir,
            &candidate.model,
            &candidate.effort,
            request.prompt.clone(),
            request.enable_web_search,
            request.harness,
        )?;
        match run_claude_process(job_id, invocation, cancellation).await {
            Ok(output) => match claude_result(&output) {
                Ok(response_text) => {
                    let markdown = match request.harness {
                        ClaudeHarness::ResponseText => response_text,
                        ClaudeHarness::DraftWorkspace => {
                            collect_draft_output(request.working_dir, &response_text).map_err(
                                |message| ClaudePhaseError {
                                    kind: ClaudeFailureKind::Other,
                                    message,
                                },
                            )?
                        }
                    };
                    write_new_file(request.output_path, markdown.as_bytes()).map_err(|error| {
                        ClaudePhaseError {
                            kind: ClaudeFailureKind::Other,
                            message: error,
                        }
                    })?;
                    return Ok(ClaudeRunSuccess {
                        model: candidate.model.clone(),
                        effort: candidate.effort.clone(),
                    });
                }
                Err(error) => {
                    emit_command_output(app, job_id, &output.stderr);
                    if !error.kind.is_provider_unavailable() {
                        return Err(error);
                    }
                    last_error = Some(error);
                }
            },
            Err(error) => return Err(error),
        }
    }
    Err(last_error.unwrap_or_else(|| ClaudePhaseError {
        kind: ClaudeFailureKind::Other,
        message: "Claude did not produce a guide".to_string(),
    }))
}

async fn run_claude_process(
    job_id: &str,
    invocation: ClaudeInvocation,
    cancellation: process_registry::CancellationToken,
) -> Result<ProcessOutput, ClaudePhaseError> {
    if process_registry::is_cancelled(cancellation) {
        return Err(ClaudePhaseError {
            kind: ClaudeFailureKind::Cancelled,
            message: "job cancelled before starting Claude".to_string(),
        });
    }
    let claude_executable =
        resolve_provider_executable(ProviderExecutable::Claude).map_err(|message| {
            ClaudePhaseError {
                kind: ClaudeFailureKind::AvailabilityEntitlementOrQuota,
                message,
            }
        })?;
    let mut command = Command::new(&claude_executable);
    command
        .current_dir(&invocation.working_dir)
        .args(&invocation.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let mut child = command.spawn().map_err(|error| ClaudePhaseError {
        kind: classify_claude_spawn_error(&error),
        message: format!(
            "failed to spawn native Claude process at {}: {error}",
            claude_executable.display()
        ),
    })?;
    match child.id() {
        Some(pid) if process_registry::register(job_id, pid, cancellation) => {}
        _ => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(ClaudePhaseError {
                kind: ClaudeFailureKind::Cancelled,
                message: "job cancelled before Claude could start".to_string(),
            });
        }
    }

    let mut stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            let _ = child.kill().await;
            process_registry::unregister(job_id);
            return Err(ClaudePhaseError {
                kind: ClaudeFailureKind::Other,
                message: "native Claude stdin was not available".to_string(),
            });
        }
    };
    let stdin_bytes = invocation.stdin.into_bytes();
    let stdin_task = tokio::spawn(async move {
        stdin.write_all(&stdin_bytes).await?;
        stdin.shutdown().await
    });
    let stdout_task = read_stream(child.stdout.take());
    let stderr_task = read_stream(child.stderr.take());
    let status = child.wait().await;
    let stdin_result = stdin_task.await;
    let (stdout, stderr) = tokio::join!(stdout_task, stderr_task);
    process_registry::unregister(job_id);

    let status = status.map_err(|error| ClaudePhaseError {
        kind: ClaudeFailureKind::Other,
        message: format!("failed while waiting for native Claude: {error}"),
    })?;
    if status.success() {
        match stdin_result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                return Err(ClaudePhaseError {
                    kind: ClaudeFailureKind::Other,
                    message: format!("failed to send prompt to Claude: {error}"),
                });
            }
            Err(error) => {
                return Err(ClaudePhaseError {
                    kind: ClaudeFailureKind::Other,
                    message: format!("Claude stdin task failed: {error}"),
                });
            }
        }
    }
    if process_registry::is_cancelled(cancellation) {
        return Err(ClaudePhaseError {
            kind: ClaudeFailureKind::Cancelled,
            message: "job cancelled while Claude was running".to_string(),
        });
    }
    let stdout = stdout
        .map_err(|error| ClaudePhaseError {
            kind: ClaudeFailureKind::Other,
            message: format!("Claude stdout task failed: {error}"),
        })?
        .map_err(|error| ClaudePhaseError {
            kind: ClaudeFailureKind::Other,
            message: format!("failed to read Claude stdout: {error}"),
        })?;
    let stderr = stderr
        .map_err(|error| ClaudePhaseError {
            kind: ClaudeFailureKind::Other,
            message: format!("Claude stderr task failed: {error}"),
        })?
        .map_err(|error| ClaudePhaseError {
            kind: ClaudeFailureKind::Other,
            message: format!("failed to read Claude stderr: {error}"),
        })?;
    Ok(ProcessOutput {
        exit_code: status.code().unwrap_or(-1),
        stdout,
        stderr,
    })
}

fn read_stream<R>(stream: Option<R>) -> tokio::task::JoinHandle<std::io::Result<String>>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut text = String::new();
        if let Some(mut stream) = stream {
            stream.read_to_string(&mut text).await?;
        }
        Ok(text)
    })
}

/// Assemble the harness result: `draft/guide.md` plus, when present, `draft/artifacts.json`
/// wrapped in the app's artifact markers. A writer that ignored the workspace and pasted the
/// guide into its reply is tolerated once the reply plainly is a guide.
pub(crate) fn collect_draft_output(working_dir: &Path, response_text: &str) -> Result<String, String> {
    let guide_path = working_dir.join(DRAFT_GUIDE);
    let guide = match std::fs::read_to_string(&guide_path) {
        Ok(text) if !text.trim().is_empty() => text,
        _ => {
            let reply = response_text.trim_start();
            if reply.starts_with("# ") && reply.split_whitespace().count() > 200 {
                return Ok(response_text.to_string());
            }
            return Err(format!(
                "the writer finished without writing {} (its reply was not a guide either)",
                guide_path.display()
            ));
        }
    };
    let artifacts_path = working_dir.join(DRAFT_ARTIFACTS);
    let artifacts = std::fs::read_to_string(&artifacts_path).unwrap_or_default();
    if guide.contains(ARTIFACTS_START) || artifacts.trim().is_empty() {
        return Ok(guide);
    }
    Ok(format!(
        "{}\n\n{ARTIFACTS_START}\n{}\n{ARTIFACTS_END}\n",
        guide.trim_end(),
        artifacts.trim()
    ))
}

fn build_claude_invocation(
    working_dir: &Path,
    model: &str,
    effort: &str,
    prompt: String,
    enable_web_search: bool,
    harness: ClaudeHarness,
) -> Result<ClaudeInvocation, ClaudePhaseError> {
    validate_model(model)?;
    validate_effort(effort)?;
    if prompt.trim().is_empty() {
        return Err(ClaudePhaseError {
            kind: ClaudeFailureKind::Other,
            message: "Claude prompt must not be empty".to_string(),
        });
    }
    if !working_dir.is_dir() {
        return Err(ClaudePhaseError {
            kind: ClaudeFailureKind::Other,
            message: format!(
                "Claude working directory does not exist: {}",
                working_dir.display()
            ),
        });
    }
    let tools = match (harness, enable_web_search) {
        (ClaudeHarness::DraftWorkspace, _) => "Read,Glob,Grep,Write,Edit,Bash",
        (ClaudeHarness::ResponseText, true) => "Read,Glob,Grep,WebSearch",
        (ClaudeHarness::ResponseText, false) => "Read,Glob,Grep",
    };
    let mut harness_args = Vec::new();
    if harness == ClaudeHarness::DraftWorkspace {
        // `Edit(...)` rules govern every file-editing tool (Write included); a `Write(...)`
        // rule is ignored by the CLI. The single Bash rule is an exact command match.
        harness_args.extend([
            OsString::from("--allowedTools"),
            OsString::from(format!("Edit({DRAFT_DIR}/**)")),
            OsString::from(format!("Bash(./{VERIFY_SCRIPT})")),
        ]);
    }
    Ok(ClaudeInvocation {
        working_dir: working_dir.to_path_buf(),
        args: [vec![
            OsString::from("--print"),
            OsString::from("--verbose"),
            OsString::from("--output-format"),
            OsString::from("stream-json"),
            OsString::from("--input-format"),
            OsString::from("text"),
            OsString::from("--model"),
            OsString::from(model),
            OsString::from("--effort"),
            OsString::from(effort),
            OsString::from("--permission-mode"),
            OsString::from("dontAsk"),
            OsString::from("--restricted"),
            OsString::from("--safe-mode"),
            OsString::from("--tools"),
            OsString::from(tools),
            OsString::from("--strict-mcp-config"),
            OsString::from("--mcp-config"),
            OsString::from(r#"{"mcpServers":{}}"#),
            OsString::from("--disable-slash-commands"),
            OsString::from("--no-session-persistence"),
            OsString::from("--no-chrome"),
        ], harness_args]
        .concat(),
        stdin: prompt,
    })
}

fn claude_result(output: &ProcessOutput) -> Result<String, ClaudePhaseError> {
    // Classify the *failure*, not the transcript. A long stream-json run carries every
    // assistant message and tool result, and the classifier's permission-denied block
    // returns `Other` before availability patterns are considered - so one incidental
    // phrase anywhere in the transcript used to strip the Codex fallback off a genuine
    // quota or capacity failure.
    let failure_text = claude_failure_text(output);
    if output.exit_code != 0 {
        return Err(ClaudePhaseError {
            kind: classify_failure_without_output(&failure_text),
            message: format!(
                "Claude exited with status {}: {}",
                output.exit_code, failure_text
            ),
        });
    }
    let mut result = None;
    let mut reported_error = false;
    for line in output.stdout.lines() {
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            if value.get("type").and_then(Value::as_str) == Some("result") {
                reported_error |= value
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if let Some(text) = value.get("result").and_then(Value::as_str) {
                    result = Some(text.to_string());
                }
            }
        }
    }
    if reported_error {
        return Err(ClaudePhaseError {
            kind: classify_failure_without_output(&failure_text),
            message: failure_text,
        });
    }
    result
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| ClaudePhaseError {
            kind: classify_failure_without_output(&output.combined()),
            message: format!(
                "Claude exited successfully without a non-empty result: {}",
                concise_error(&output.combined())
            ),
        })
}

/// Classify a failure in which the CLI produced no guide. The wording decides the reason
/// reported to the user; it never decides whether the run may continue with another writer,
/// because an unrecognized CLI failure is still a failure of the CLI, not of the writing.
pub(crate) fn classify_failure_without_output(text: &str) -> ClaudeFailureKind {
    match classify_claude_failure(text) {
        ClaudeFailureKind::Other => ClaudeFailureKind::Unrecognized,
        kind => kind,
    }
}

pub(crate) fn classify_claude_failure(text: &str) -> ClaudeFailureKind {
    let lower = text.to_ascii_lowercase();
    if lower.contains("content filtering policy")
        || lower.contains("content filter")
        || lower.contains("output blocked")
    {
        return ClaudeFailureKind::ContentRejected;
    }
    if [
        "job cancelled",
        "job canceled",
        "request cancelled",
        "request canceled",
        "operation cancelled",
        "operation canceled",
        "interrupted by user",
        "terminated by user",
    ]
    .iter()
    .any(|pattern| lower.contains(pattern))
    {
        return ClaudeFailureKind::Cancelled;
    }
    if [
        "authentication failed",
        "authentication error",
        "unauthorized",
        "invalid api key",
        "invalid token",
        "token expired",
        "credentials are missing",
        "credentials expired",
        "login required",
        "not logged in",
        "please log in",
        "please sign in",
        "permission denied",
        "insufficient permission",
        "access denied",
        "forbidden",
        "http 401",
        "http 403",
        "run /login",
    ]
    .iter()
    .any(|pattern| lower.contains(pattern))
    {
        return ClaudeFailureKind::Authentication;
    }
    if [
        "connection lost",
        "connection reset",
        "connection refused",
        "econnreset",
        "econnrefused",
        "enotfound",
        "etimedout",
        "socket hang up",
        "fetch failed",
        "network error",
        "can't reach the api",
        "cannot reach the api",
        "timed out",
        "request timeout",
        "http 502",
        "http 503",
        "http 504",
    ]
    .iter()
    .any(|pattern| lower.contains(pattern))
    {
        return ClaudeFailureKind::Transport;
    }
    let availability_patterns = [
        "rate_limit",
        "rate limit",
        "usage limit",
        "session limit",
        "weekly limit",
        "hit your limit",
        "hit your session",
        "limit will reset",
        "resets at",
        "\u{b7} resets",
        "limit reached",
        "quota",
        "credit balance",
        "billing limit",
        "overloaded",
        "over capacity",
        "capacity error",
        "model_not_found",
        "model not found",
        "model is not available",
        "model unavailable",
        "unsupported model",
        "not entitled",
        "not eligible",
        "not available on your plan",
        "requires a max plan",
        "upgrade your plan to use",
        "does not have access to model",
        "do not have access to model",
        "does not exist or you do not have access",
        // "You've reached your Fable limit. Switch to another model to continue."
        "switch to another model",
        "switch models",
    ];
    // Per-model allowances are phrased with the model's name in the middle
    // ("reached your <model> limit"), so match the shape rather than each name.
    let exhausted_allowance = lower.contains("limit")
        && ["reached your", "reached the", "used up your", "run out of"]
            .iter()
            .any(|pattern| lower.contains(pattern));
    if exhausted_allowance
        || availability_patterns
            .iter()
            .any(|pattern| lower.contains(pattern))
    {
        ClaudeFailureKind::AvailabilityEntitlementOrQuota
    } else {
        ClaudeFailureKind::Other
    }
}

fn classify_claude_spawn_error(error: &std::io::Error) -> ClaudeFailureKind {
    if error.kind() == std::io::ErrorKind::NotFound {
        ClaudeFailureKind::AvailabilityEntitlementOrQuota
    } else {
        ClaudeFailureKind::Other
    }
}

fn validate_model(model: &str) -> Result<(), ClaudePhaseError> {
    let model = model.trim();
    if model.is_empty()
        || model.len() > 128
        || !model.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | ':' | '/')
        })
    {
        return Err(ClaudePhaseError {
            kind: ClaudeFailureKind::Other,
            message: format!("invalid Claude model name: {model}"),
        });
    }
    Ok(())
}

fn validate_effort(effort: &str) -> Result<(), ClaudePhaseError> {
    match effort.trim() {
        "low" | "medium" | "high" | "xhigh" | "max" => Ok(()),
        _ => Err(ClaudePhaseError {
            kind: ClaudeFailureKind::Other,
            message: format!("invalid Claude effort: {effort}"),
        }),
    }
}

pub(crate) fn validate_model_spec(candidate: &ClaudeModelSpec) -> Result<(), ClaudePhaseError> {
    validate_model(&candidate.model)?;
    validate_effort(&candidate.effort)
}

impl ProcessOutput {
    fn combined(&self) -> String {
        match (self.stdout.is_empty(), self.stderr.is_empty()) {
            (false, false) => format!("{}\n{}", self.stdout.trim_end(), self.stderr),
            (false, true) => self.stdout.clone(),
            (true, false) => self.stderr.clone(),
            (true, true) => String::new(),
        }
    }
}

/// Reduce failure text to one bounded excerpt, keeping the END of the stream.
///
/// This previously kept the first 800 characters. Under `--output-format stream-json`
/// the first stdout line is always a large `{"type":"system","subtype":"init",...}`
/// banner, so head-truncation reported that banner and discarded the actual failure,
/// which is emitted last. Failures live at the end of a stream, so keep the tail.
fn concise_error(text: &str) -> String {
    let text = text.trim();
    let total = text.chars().count();
    if total <= 800 {
        return text.to_string();
    }
    let mut concise = String::from("...");
    concise.extend(text.chars().skip(total - 800));
    concise
}

/// Pick the most informative failure text a Claude CLI run produced.
///
/// Prefers, in order: anything on stderr, then the terminal `result` event that carries
/// the model-visible error, and only then the raw tail of the combined stream. Without
/// this the caller reported the stream-json init banner as the error.
fn claude_failure_text(output: &ProcessOutput) -> String {
    let mut last_result: Option<String> = None;
    for line in output.stdout.lines() {
        let value = match serde_json::from_str::<Value>(line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if value.get("type").and_then(Value::as_str) != Some("result") {
            continue;
        }
        if let Some(text) = value
            .get("result")
            .or_else(|| value.get("error"))
            .and_then(Value::as_str)
        {
            let text = text.trim();
            if !text.is_empty() {
                last_result = Some(text.to_string());
            }
        }
    }
    let stderr = output.stderr.trim();
    let mut parts = Vec::new();
    if !stderr.is_empty() {
        parts.push(stderr.to_string());
    }
    if let Some(result) = last_result {
        parts.push(result);
    }
    if parts.is_empty() {
        return concise_error(&output.combined());
    }
    concise_error(&parts.join("\n"))
}

fn emit_command_output<S: ProgressSink + ?Sized>(app: &S, job_id: &str, output: &str) {
    for line in output.lines() {
        emit_output(app, job_id, line.to_string());
    }
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("could not create {}: {error}", path.display()))?;
    let result = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    if let Err(error) = result {
        let _ = std::fs::remove_file(path);
        return Err(format!("could not write {}: {error}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn scratch_dir() -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("guide-watcher-claude-test-{}", Uuid::new_v4()));
        std::fs::create_dir(&path).expect("create scratch directory");
        path
    }

    #[test]
    fn hostile_prompt_exists_only_on_stdin_and_no_bypass_or_shell_is_used() {
        let dir = scratch_dir();
        let original_course_path = r"C:\DGIST\Networks\CH2.pdf";
        let prompt = format!(
            "GUIDEPROMPT\n'quoted' $() `touch owned`\n한글\nprovenance_path_do_not_read: {original_course_path}"
        );
        let invocation =
            build_claude_invocation(
                &dir,
                "claude-opus-4-8",
                "high",
                prompt.clone(),
                false,
                ClaudeHarness::ResponseText,
            )
            .expect("valid invocation");
        assert_eq!(invocation.working_dir, dir);
        assert_eq!(invocation.stdin, prompt);
        let args = invocation
            .args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>();
        let joined = args.join("\0");
        assert!(!joined.contains("GUIDEPROMPT"));
        assert!(!joined.contains("touch owned"));
        assert!(!joined.contains(original_course_path));
        assert!(!joined.to_ascii_lowercase().contains("dangerously"));
        assert!(!joined.contains("bypassPermissions"));
        assert!(!joined.contains("bash"));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--permission-mode", "dontAsk"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--model", "claude-opus-4-8"]));
        assert!(args.windows(2).any(|pair| pair == ["--effort", "high"]));
        assert!(args.iter().any(|arg| arg == "--restricted"));
        let mcp_config = args
            .windows(2)
            .find(|pair| pair[0] == "--mcp-config")
            .expect("explicit MCP configuration");
        let parsed: serde_json::Value =
            serde_json::from_str(&mcp_config[1]).expect("valid MCP JSON");
        assert_eq!(
            parsed["mcpServers"]
                .as_object()
                .map(|servers| servers.len()),
            Some(0)
        );
        assert!(args.iter().any(|arg| arg == "--strict-mcp-config"));
        assert!(!args.iter().any(|arg| arg == "--add-dir"));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--tools", "Read,Glob,Grep"]));
        for forbidden in ["Bash", "PowerShell", "Edit", "Write", "NotebookEdit"] {
            assert!(!joined.contains(forbidden));
        }
        std::fs::remove_dir_all(dir).expect("remove scratch directory");
    }

    #[test]
    fn draft_workspace_harness_scopes_edits_to_draft_and_allows_one_verifier_command() {
        let dir = scratch_dir();
        let invocation = build_claude_invocation(
            &dir,
            "claude-opus-4-8",
            "high",
            "write".to_string(),
            false,
            ClaudeHarness::DraftWorkspace,
        )
        .unwrap();
        let args = invocation
            .args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--tools", "Read,Glob,Grep,Write,Edit,Bash"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--permission-mode", "dontAsk"]));
        assert!(args.iter().any(|arg| arg == "--restricted"));
        let allowed = args
            .iter()
            .position(|arg| arg == "--allowedTools")
            .expect("allow rules present");
        assert_eq!(args[allowed + 1], "Edit(draft/**)");
        assert_eq!(args[allowed + 2], "Bash(./verify_draft.sh)");
        assert!(!args.iter().any(|arg| arg.starts_with("Write(")));
        assert!(!args.iter().any(|arg| arg.contains("bypassPermissions") || arg.contains("acceptEdits")));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn draft_output_is_assembled_from_the_workspace_files() {
        let dir = scratch_dir();
        std::fs::create_dir(dir.join(DRAFT_DIR)).unwrap();
        // No draft and a chatty reply: an error, not a guide.
        assert!(collect_draft_output(&dir, "DONE. I could not write the file.").is_err());
        // No draft but the reply plainly is a guide: tolerated.
        let pasted = format!("# Guide\n\n{}", "word ".repeat(300));
        assert!(collect_draft_output(&dir, &pasted).unwrap().starts_with("# Guide"));
        // Draft and artifacts present: assembled with the app's markers, exactly once.
        std::fs::write(dir.join(DRAFT_GUIDE), "# Guide\n\nbody\n").unwrap();
        std::fs::write(dir.join(DRAFT_ARTIFACTS), "{\"coverage_manifest\": {}}\n").unwrap();
        let assembled = collect_draft_output(&dir, "DONE").unwrap();
        assert_eq!(
            assembled,
            format!("# Guide\n\nbody\n\n{ARTIFACTS_START}\n{{\"coverage_manifest\": {{}}}}\n{ARTIFACTS_END}\n")
        );
        // A draft that already carries a block is passed through untouched.
        let with_block = format!("# Guide\n\nbody\n\n{ARTIFACTS_START}\n{{}}\n{ARTIFACTS_END}\n");
        std::fs::write(dir.join(DRAFT_GUIDE), &with_block).unwrap();
        assert_eq!(collect_draft_output(&dir, "DONE").unwrap(), with_block);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn provider_unavailability_is_recognized_in_every_form_the_cli_uses() {
        for (text, kind) in [
            ("You've hit your session limit \u{b7} resets 10:40pm (Asia/Seoul)", ClaudeFailureKind::AvailabilityEntitlementOrQuota),
            ("You've hit your weekly limit. Limit will reset at 9am", ClaudeFailureKind::AvailabilityEntitlementOrQuota),
            ("API Error: 429 rate_limit_error", ClaudeFailureKind::AvailabilityEntitlementOrQuota),
            ("Not logged in \u{b7} Please run /login", ClaudeFailureKind::Authentication),
            ("authentication failed: token expired", ClaudeFailureKind::Authentication),
            ("API Error: Connection lost mid-response. The response above may be incomplete.", ClaudeFailureKind::Transport),
            ("API Error: Can't reach the API server \u{2014} check your internet or DNS (ENOTFOUND).", ClaudeFailureKind::Transport),
            ("output blocked by content filtering policy", ClaudeFailureKind::ContentRejected),
            ("model artifact block is not valid JSON", ClaudeFailureKind::Other),
            // The real message that ended a run with no fallback on 2026-09-13.
            ("You've reached your Fable limit. Switch to another model to continue.", ClaudeFailureKind::AvailabilityEntitlementOrQuota),
            ("You've reached your Opus limit.", ClaudeFailureKind::AvailabilityEntitlementOrQuota),
        ] {
            assert_eq!(classify_claude_failure(text), kind, "{text}");
        }
        assert!(ClaudeFailureKind::AvailabilityEntitlementOrQuota.is_provider_unavailable());
        assert!(ClaudeFailureKind::Authentication.is_provider_unavailable());
        assert!(ClaudeFailureKind::Transport.is_provider_unavailable());
        assert!(ClaudeFailureKind::Unrecognized.is_provider_unavailable());
        assert!(!ClaudeFailureKind::ContentRejected.is_provider_unavailable());
        assert!(!ClaudeFailureKind::Other.is_provider_unavailable());
        assert!(!ClaudeFailureKind::Cancelled.is_provider_unavailable());
    }

    #[test]
    fn a_cli_failure_with_no_guide_stays_eligible_for_another_writer() {
        // Wording the app has never seen must not end the run: the CLI produced nothing, so
        // the next candidate and then the fallback writer still get their turn.
        let novel = "Error: your plan changed; this model is temporarily paused for your account";
        assert_eq!(classify_claude_failure(novel), ClaudeFailureKind::Other);
        assert_eq!(
            classify_failure_without_output(novel),
            ClaudeFailureKind::Unrecognized
        );
        assert!(classify_failure_without_output(novel).is_provider_unavailable());

        // A recognized reason keeps its own label, so the user still reads what happened.
        for (text, kind) in [
            ("You've reached your Fable limit. Switch to another model to continue.", ClaudeFailureKind::AvailabilityEntitlementOrQuota),
            ("Not logged in \u{b7} Please run /login", ClaudeFailureKind::Authentication),
            ("output blocked by content filtering policy", ClaudeFailureKind::ContentRejected),
            ("job cancelled", ClaudeFailureKind::Cancelled),
        ] {
            assert_eq!(classify_failure_without_output(text), kind, "{text}");
        }
        // Cancellation and content rejection stay terminal.
        assert!(!classify_failure_without_output("job cancelled").is_provider_unavailable());
        assert!(
            !classify_failure_without_output("output blocked by content filtering policy")
                .is_provider_unavailable()
        );
    }

    #[test]
    fn a_nonzero_exit_is_never_treated_as_a_writing_mistake() {
        let output = ProcessOutput {
            exit_code: 1,
            stdout: String::new(),
            stderr: "Error: something entirely new went wrong".to_string(),
        };
        let error = claude_result(&output).unwrap_err();
        assert!(
            error.kind.is_provider_unavailable(),
            "{:?}: {}",
            error.kind,
            error.message
        );
        assert!(error.message.contains("something entirely new"), "{}", error.message);
    }

    #[test]
    fn model_and_effort_reject_flag_injection() {
        let dir = scratch_dir();
        assert!(build_claude_invocation(
            &dir,
            "fable --dangerously-skip-permissions",
            "max",
            "prompt".to_string(),
            false,
            ClaudeHarness::ResponseText,
        )
        .is_err());
        assert!(build_claude_invocation(
            &dir,
            "fable",
            "max --permission-mode bypassPermissions",
            "prompt".to_string(),
            false,
            ClaudeHarness::ResponseText,
        )
        .is_err());
        std::fs::remove_dir_all(dir).expect("remove scratch directory");
    }

    #[test]
    fn prep_fallback_can_enable_web_search_without_enabling_write_or_shell_tools() {
        let dir = scratch_dir();
        let invocation = build_claude_invocation(
            &dir,
            "claude-opus-4-8",
            "high",
            "collect context".to_string(),
            true,
            ClaudeHarness::ResponseText,
        )
        .unwrap();
        let args = invocation
            .args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>();
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--tools", "Read,Glob,Grep,WebSearch"]));
        let joined = args.join("\0");
        for forbidden in ["Bash", "PowerShell", "Edit", "Write", "NotebookEdit"] {
            assert!(!joined.contains(forbidden));
        }
        std::fs::remove_dir_all(dir).expect("remove scratch directory");
    }

    #[test]
    fn classification_allows_fallback_only_for_availability_entitlement_or_quota() {
        for error in [
            "429 rate_limit exceeded",
            "Your usage limit has been reached",
            "You've hit your limit · resets later",
            "Model is not available for this account",
            "You do not have access to model claude-fable-5",
            "Service overloaded",
        ] {
            assert_eq!(
                classify_claude_failure(error),
                ClaudeFailureKind::AvailabilityEntitlementOrQuota,
                "{error}"
            );
        }
        for error in [
            "Output blocked by content filtering policy",
            "invalid request: malformed prompt",
            "authentication failed",
            "permission denied writing a file",
            "verification failed",
        ] {
            assert_ne!(
                classify_claude_failure(error),
                ClaudeFailureKind::AvailabilityEntitlementOrQuota,
                "{error}"
            );
        }
        assert_eq!(
            classify_claude_failure("Output blocked by content filtering policy"),
            ClaudeFailureKind::ContentRejected
        );
        // Authentication wins over quota wording, and it is a provider outage: the Codex
        // fallback may take over instead of the job dying while the writer is logged out.
        assert_eq!(
            classify_claude_failure("quota exhausted; authentication failed: please log in"),
            ClaudeFailureKind::Authentication
        );
        assert_eq!(
            classify_claude_failure("model unavailable; permission denied reading credentials"),
            ClaudeFailureKind::Authentication
        );
        assert_eq!(
            classify_claude_failure("quota exhausted; output blocked by content filter"),
            ClaudeFailureKind::ContentRejected
        );
        assert_eq!(
            classify_claude_failure("model not found after request cancelled by user"),
            ClaudeFailureKind::Cancelled
        );
    }

    #[test]
    fn transcript_prose_cannot_strip_the_codex_fallback_off_a_quota_failure() {
        // A long repair transcript carries every assistant message and tool result. The
        // classifier's permission-denied block returns `Other` before the availability
        // patterns are reached, and only `AvailabilityEntitlementOrQuota` keeps the Codex
        // fallback eligible - so incidental prose used to fail the whole job instead.
        let transcript = concat!(
            "{\"type\":\"system\",\"subtype\":\"init\",\"tools\":[\"Read\"]}\n",
            "{\"type\":\"assistant\",\"message\":\"I tried to read that path and got permission denied, so I will skip it.\"}\n",
            "{\"type\":\"result\",\"is_error\":true,\"result\":\"Claude usage limit reached. Try again later.\"}\n",
        );
        let output = ProcessOutput {
            exit_code: 1,
            stdout: transcript.to_string(),
            stderr: String::new(),
        };

        let error = claude_result(&output).expect_err("non-zero exit must fail");
        assert_eq!(
            error.kind,
            ClaudeFailureKind::AvailabilityEntitlementOrQuota,
            "quota failure must stay eligible for the Codex fallback: {}",
            error.message
        );
        assert!(error.message.contains("usage limit reached"), "{}", error.message);

        // A permission failure is Claude's problem, not the guide's: it is classified as an
        // authentication outage so the Codex fallback can write instead.
        let denied = ProcessOutput {
            exit_code: 1,
            stdout: "{\"type\":\"system\",\"subtype\":\"init\"}\n".to_string(),
            stderr: "Error: permission denied reading credentials".to_string(),
        };
        let error = claude_result(&denied).expect_err("non-zero exit must fail");
        assert_eq!(error.kind, ClaudeFailureKind::Authentication, "{}", error.message);
        assert!(error.kind.is_provider_unavailable());
    }

    #[test]
    fn stream_json_failure_reports_the_real_error_not_the_init_banner() {
        // Reproduces an observed field failure: a repair run exited 1 and the operator was
        // shown a truncated `{"type":"system","subtype":"init",...}` banner, because the
        // error text was head-truncated and that banner is always the first stdout line.
        let banner = format!(
            "{{\"type\":\"system\",\"subtype\":\"init\",\"cwd\":\"C:/work\",\"session_id\":\"abc\",\
             \"tools\":[\"Glob\",\"Grep\",\"Read\"],\"model\":\"claude-opus-5\",\"padding\":\"{}\"}}",
            "p".repeat(2_000)
        );
        let result_line = "{\"type\":\"result\",\"is_error\":true,\"result\":\"context limit exceeded while repairing the guide\"}";
        let output = ProcessOutput {
            exit_code: 1,
            stdout: format!("{banner}\n{result_line}\n"),
            stderr: String::new(),
        };

        let error = claude_result(&output).expect_err("non-zero exit must fail");
        assert!(
            error.message.contains("context limit exceeded while repairing the guide"),
            "the real error must survive: {}",
            error.message
        );
        assert!(
            !error.message.contains("subtype"),
            "the init banner must not be reported as the error: {}",
            error.message
        );

        // stderr, when present, is the most direct signal and must be preserved too.
        let with_stderr = ProcessOutput {
            exit_code: 1,
            stdout: format!("{banner}\n"),
            stderr: "Error: credit balance is too low".to_string(),
        };
        let error = claude_result(&with_stderr).expect_err("non-zero exit must fail");
        assert!(
            error.message.contains("credit balance is too low"),
            "stderr must survive: {}",
            error.message
        );

        // With no structured signal at all, keep the tail rather than the banner head.
        let unstructured = ProcessOutput {
            exit_code: 1,
            stdout: format!("{banner}\nfatal: the writer stopped unexpectedly\n"),
            stderr: String::new(),
        };
        let error = claude_result(&unstructured).expect_err("non-zero exit must fail");
        assert!(
            error.message.contains("fatal: the writer stopped unexpectedly"),
            "the tail must survive: {}",
            error.message
        );
    }

    #[test]
    fn missing_claude_executable_is_availability_but_permission_denial_is_not() {
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(
            classify_claude_spawn_error(&missing),
            ClaudeFailureKind::AvailabilityEntitlementOrQuota
        );

        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(
            classify_claude_spawn_error(&denied),
            ClaudeFailureKind::Other
        );
    }

    #[test]
    fn stream_json_result_is_extracted_without_emitting_protocol_text() {
        let output = ProcessOutput {
            exit_code: 0,
            stdout: "{\"type\":\"system\",\"subtype\":\"init\"}\n{\"type\":\"result\",\"is_error\":false,\"result\":\"# Guide\\n\\nBody\"}\n".to_string(),
            stderr: String::new(),
        };
        assert_eq!(claude_result(&output).unwrap(), "# Guide\n\nBody");
    }
}
