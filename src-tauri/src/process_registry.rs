use std::collections::HashMap;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Clone)]
struct ActiveProcess {
    pid: u32,
    token_id: u64,
}

#[derive(Default)]
struct RegistryState {
    next_token_id: u64,
    tokens: HashMap<u64, bool>,
    processes: HashMap<String, ActiveProcess>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CancellationToken {
    id: u64,
}

static ACTIVE: OnceLock<Mutex<RegistryState>> = OnceLock::new();

fn active() -> &'static Mutex<RegistryState> {
    ACTIVE.get_or_init(|| Mutex::new(RegistryState::default()))
}

/// Create an independently cancellable scope before a batch or resume job is
/// queued. The token is registered immediately, so global shutdown can still
/// invalidate it before the first child process is spawned.
pub fn cancellation_token() -> CancellationToken {
    let mut state = active()
        .lock()
        .expect("cancellation registry must be available when creating a token");
    let id = loop {
        state.next_token_id = state.next_token_id.wrapping_add(1).max(1);
        if !state.tokens.contains_key(&state.next_token_id) {
            break state.next_token_id;
        }
    };
    state.tokens.insert(id, false);
    CancellationToken { id }
}

pub fn is_cancelled(token: CancellationToken) -> bool {
    active()
        .lock()
        .map(|state| state.tokens.get(&token.id).copied() != Some(false))
        .unwrap_or(true)
}

/// Run a short state transition only while the token is active. The token
/// check and action share the cancellation lock, so cancellation and final
/// publication have a deterministic order.
pub fn run_if_current<T>(
    token: CancellationToken,
    action: impl FnOnce() -> T,
) -> Result<T, &'static str> {
    let state = active()
        .lock()
        .map_err(|_| "cancellation registry is unavailable")?;
    if state.tokens.get(&token.id).copied() != Some(false) {
        return Err("job was cancelled");
    }
    Ok(action())
}

/// Register a child only while its owning job token is still active. The
/// token check and insertion share one lock, closing the race between a
/// window-close cancellation and process registration.
pub fn register(job_id: &str, pid: u32, token: CancellationToken) -> bool {
    if let Ok(mut state) = active().lock() {
        if state.tokens.get(&token.id).copied() != Some(false) {
            return false;
        }
        if state.processes.contains_key(job_id) {
            return false;
        }
        state.processes.insert(
            job_id.to_string(),
            ActiveProcess {
                pid,
                token_id: token.id,
            },
        );
        true
    } else {
        false
    }
}

pub fn unregister(job_id: &str) {
    if let Ok(mut state) = active().lock() {
        state.processes.remove(job_id);
    }
}

/// Cancel one job/batch scope without invalidating unrelated tokens.
pub fn cancel(token: CancellationToken) {
    for process in take_token_processes(token) {
        kill_process_tree(process.pid);
    }
}

/// Release a completed scope and defensively terminate any child registration
/// that failed to unregister. Late registrations fail after this returns.
pub fn finish(token: CancellationToken) {
    cancel(token);
    if let Ok(mut state) = active().lock() {
        state.tokens.remove(&token.id);
    }
}

pub fn cancel_all() {
    for process in take_active_processes() {
        kill_process_tree(process.pid);
    }
}

/// Invalidate every active token synchronously, then terminate child trees on
/// detached threads. This keeps Ctrl+C bounded even if an operating-system
/// process-control command stalls.
pub fn cancel_all_nonblocking() {
    for process in take_active_processes() {
        let _ = std::thread::Builder::new()
            .name(format!("guide-watcher-kill-{}", process.pid))
            .spawn(move || kill_process_tree(process.pid));
    }
}

fn take_active_processes() -> Vec<ActiveProcess> {
    active()
        .lock()
        .map(|mut state| {
            for cancelled in state.tokens.values_mut() {
                *cancelled = true;
            }
            state
                .processes
                .drain()
                .map(|(_, process)| process)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn take_token_processes(token: CancellationToken) -> Vec<ActiveProcess> {
    active()
        .lock()
        .map(|mut state| {
            if let Some(cancelled) = state.tokens.get_mut(&token.id) {
                *cancelled = true;
            }
            let job_ids = state
                .processes
                .iter()
                .filter_map(|(job_id, process)| {
                    (process.token_id == token.id).then_some(job_id.clone())
                })
                .collect::<Vec<_>>();
            job_ids
                .into_iter()
                .filter_map(|job_id| state.processes.remove(&job_id))
                .collect()
        })
        .unwrap_or_default()
}

fn kill_process_tree(pid: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }

    #[cfg(not(windows))]
    {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .output();
    }
}

pub(crate) fn terminate_unregistered_process_tree(pid: u32) {
    kill_process_tree(pid);
}

#[cfg(test)]
pub(crate) fn has_registered_process(token: CancellationToken) -> bool {
    active()
        .lock()
        .map(|state| {
            state
                .processes
                .values()
                .any(|process| process.token_id == token.id)
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_cancellation_blocks_only_its_late_process_registration() {
        let old = cancellation_token();
        let independent = cancellation_token();
        cancel(old);

        assert!(is_cancelled(old));
        assert!(!register("late-test-job", u32::MAX, old));
        assert!(!is_cancelled(independent));
        unregister("late-test-job");
        finish(old);
        finish(independent);
    }

    #[test]
    fn cancelled_token_cannot_enter_an_atomic_publication_transition() {
        let token = cancellation_token();
        cancel(token);
        let entered = std::sync::atomic::AtomicBool::new(false);

        let result = run_if_current(token, || {
            entered.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        assert_eq!(result, Err("job was cancelled"));
        assert!(!entered.load(std::sync::atomic::Ordering::SeqCst));
        finish(token);
    }

    #[test]
    fn cancellation_after_entry_waits_for_atomic_publication_transition() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Barrier,
        };

        let token = cancellation_token();
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let published = Arc::new(AtomicBool::new(false));
        let worker = {
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            let published = Arc::clone(&published);
            std::thread::spawn(move || {
                run_if_current(token, || {
                    entered.wait();
                    release.wait();
                    published.store(true, Ordering::SeqCst);
                })
            })
        };

        entered.wait();
        let cancellation_attempted = Arc::new(AtomicBool::new(false));
        let canceller = {
            let cancellation_attempted = Arc::clone(&cancellation_attempted);
            std::thread::spawn(move || {
                cancellation_attempted.store(true, Ordering::SeqCst);
                cancel(token);
            })
        };
        while !cancellation_attempted.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        release.wait();

        assert_eq!(worker.join().unwrap(), Ok(()));
        canceller.join().unwrap();
        assert!(published.load(Ordering::SeqCst));
        assert!(is_cancelled(token));
        finish(token);
    }

    #[test]
    fn finishing_one_token_does_not_invalidate_an_independent_token() {
        let completed = cancellation_token();
        let still_running = cancellation_token();
        finish(completed);
        assert!(is_cancelled(completed));
        assert!(!is_cancelled(still_running));
        finish(still_running);
    }

    #[test]
    fn duplicate_job_registration_cannot_replace_another_tokens_process() {
        let first = cancellation_token();
        let second = cancellation_token();
        assert!(register("shared-job-id", u32::MAX, first));
        assert!(!register("shared-job-id", u32::MAX - 1, second));
        unregister("shared-job-id");
        finish(first);
        finish(second);
    }
}
