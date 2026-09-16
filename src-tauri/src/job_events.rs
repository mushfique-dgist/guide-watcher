use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

#[derive(Debug)]
pub struct OutputReservation {
    _lock_file: File,
    _lock_path: PathBuf,
    _owner: String,
}

#[derive(Debug)]
pub struct CourseReservation {
    _lock_file: File,
    _lock_path: PathBuf,
    _owner: String,
}

/// Acquire the cross-process lock for a final output identity without making
/// any conclusion about recoverable publication artifacts beside it.
pub fn acquire_output_lock(output_path: &Path) -> Result<OutputReservation, String> {
    let identity = normalized_output_identity(output_path)?;
    let lock_path = output_lock_path(&identity)?;
    acquire_output_lock_for_identity(output_path, identity, lock_path)
}

/// Exclusively reserves a final guide path for the lifetime of the returned guard.
/// Existing guides are never reserved: the hybrid pipeline publishes a new guide
/// atomically and does not support in-place Markdown resume.
#[cfg(test)]
pub fn reserve_output_path(output_path: &Path) -> Result<OutputReservation, String> {
    let reservation = acquire_output_lock(output_path)?;
    match output_path.try_exists() {
        Ok(true) => {
            return Err(format!(
                "A guide already exists at '{}'. Remove or rename it before generating a replacement.",
                output_path.display()
            ));
        }
        Ok(false) => {}
        Err(error) => {
            return Err(format!(
                "Could not safely inspect output path '{}': {}. No guide was started.",
                output_path.display(),
                error
            ));
        }
    }
    Ok(reservation)
}

fn acquire_output_lock_for_identity(
    output_path: &Path,
    identity: String,
    lock_path: PathBuf,
) -> Result<OutputReservation, String> {
    let mut lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| {
            format!(
                "Could not open the output lock {}: {error}",
                lock_path.display()
            )
        })?;
    match lock_file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            return Err(format!(
                "Another Guide Watcher process is already using '{}'. Wait for it to finish or choose another filename.",
                output_path.display()
            ));
        }
        Err(TryLockError::Error(error)) => {
            return Err(format!(
                "Could not lock output path '{}': {error}. No guide was started.",
                output_path.display()
            ));
        }
    }

    let owner = Uuid::new_v4().to_string();
    let started_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let ownership = format!(
        "owner={owner}\nprocess={}\nstarted_unix_ms={started_ms}\noutput={identity}\n",
        std::process::id()
    );
    lock_file
        .set_len(0)
        .and_then(|_| lock_file.seek(SeekFrom::Start(0)).map(|_| ()))
        .and_then(|_| lock_file.write_all(ownership.as_bytes()))
        .and_then(|_| lock_file.sync_data())
        .map_err(|error| {
            format!(
                "Could not record output-lock ownership in {}: {error}",
                lock_path.display()
            )
        })?;

    Ok(OutputReservation {
        _lock_file: lock_file,
        _lock_path: lock_path,
        _owner: owner,
    })
}

/// Exclusively reserves one course root across all Guide Watcher processes.
/// Lock artifacts are persistent metadata files; ownership is the live OS file
/// lock, so an unlocked stale file is safely reused without PID-based deletion.
pub fn reserve_course(course_id: &str, course_root: &Path) -> Result<CourseReservation, String> {
    let canonical_root = course_root.canonicalize().map_err(|error| {
        format!(
            "Could not resolve course root '{}': {error}",
            course_root.display()
        )
    })?;
    let root = canonical_root
        .to_str()
        .ok_or_else(|| "Course root is not valid Unicode".to_string())?
        .replace('\\', "/");
    #[cfg(windows)]
    let root = root.to_lowercase();
    let identity = format!("{course_id}\n{root}");
    let lock_dir = std::env::temp_dir().join("guide-watcher-course-locks");
    std::fs::create_dir_all(&lock_dir).map_err(|error| {
        format!(
            "Could not create course-lock directory {}: {error}",
            lock_dir.display()
        )
    })?;
    let digest = format!("{:x}", Sha256::digest(identity.as_bytes()));
    let lock_path = lock_dir.join(format!("{digest}.lock"));
    let mut lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| {
            format!(
                "Could not open course lock {}: {error}",
                lock_path.display()
            )
        })?;
    match lock_file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            return Err(format!(
                "Another Guide Watcher process is already generating course '{course_id}'. Wait for that batch to finish."
            ));
        }
        Err(TryLockError::Error(error)) => {
            return Err(format!(
                "Could not lock course '{course_id}': {error}. No provider was started."
            ));
        }
    }
    let owner = Uuid::new_v4().to_string();
    let started_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let ownership = format!(
        "owner={owner}\nprocess={}\nstarted_unix_ms={started_ms}\ncourse={course_id}\nroot={root}\n",
        std::process::id()
    );
    lock_file
        .set_len(0)
        .and_then(|_| lock_file.seek(SeekFrom::Start(0)).map(|_| ()))
        .and_then(|_| lock_file.write_all(ownership.as_bytes()))
        .and_then(|_| lock_file.sync_data())
        .map_err(|error| format!("Could not record course-lock ownership: {error}"))?;
    Ok(CourseReservation {
        _lock_file: lock_file,
        _lock_path: lock_path,
        _owner: owner,
    })
}

fn normalized_output_identity(path: &Path) -> Result<String, String> {
    let parent = path
        .parent()
        .filter(|candidate| !candidate.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let canonical_parent = parent
        .canonicalize()
        .map_err(|error| format!("Could not resolve output directory: {error}"))?;
    let filename = path
        .file_name()
        .ok_or_else(|| "Could not derive output filename".to_string())?;
    let absolute = canonical_parent.join(filename);
    let identity = absolute
        .to_str()
        .ok_or_else(|| "Output path is not valid Unicode".to_string())?
        .replace('\\', "/");
    #[cfg(windows)]
    let identity = identity.to_lowercase();
    Ok(identity)
}

fn output_lock_path(identity: &str) -> Result<PathBuf, String> {
    let lock_dir = std::env::temp_dir().join("guide-watcher-output-locks");
    std::fs::create_dir_all(&lock_dir).map_err(|error| {
        format!(
            "Could not create output-lock directory {}: {error}",
            lock_dir.display()
        )
    })?;
    let digest = format!("{:x}", Sha256::digest(identity.as_bytes()));
    Ok(lock_dir.join(format!("{digest}.lock")))
}

#[derive(Clone, Serialize)]
pub struct JobStarted {
    pub job_id: String,
}

#[derive(Clone, Serialize)]
pub struct JobOutput {
    pub job_id: String,
    pub line: String,
}

#[derive(Clone, Serialize)]
pub struct JobDone {
    pub job_id: String,
    pub exit_code: i32,
    pub output_file_exists: bool,
}

pub trait ProgressSink: Send + Sync {
    fn started(&self, job_id: &str);
    fn output(&self, job_id: &str, line: &str);
    fn done(&self, job_id: &str, exit_code: i32, output_file_exists: bool);
    fn blocked(&self, job_id: &str, message: &str) {
        self.output(job_id, message);
        self.done(job_id, -1, false);
    }
}

pub type SharedProgress = Arc<dyn ProgressSink>;

impl<T> ProgressSink for Arc<T>
where
    T: ProgressSink + ?Sized,
{
    fn blocked(&self, job_id: &str, message: &str) {
        (**self).blocked(job_id, message);
    }

    fn started(&self, job_id: &str) {
        (**self).started(job_id);
    }

    fn output(&self, job_id: &str, line: &str) {
        (**self).output(job_id, line);
    }

    fn done(&self, job_id: &str, exit_code: i32, output_file_exists: bool) {
        (**self).done(job_id, exit_code, output_file_exists);
    }
}

impl ProgressSink for AppHandle {
    fn started(&self, job_id: &str) {
        crate::attention::job_started(job_id);
        let _ = self.emit(
            "job-started",
            JobStarted {
                job_id: job_id.to_string(),
            },
        );
    }

    fn output(&self, job_id: &str, line: &str) {
        let _ = self.emit(
            "job-output",
            JobOutput {
                job_id: job_id.to_string(),
                line: line.to_string(),
            },
        );
    }

    fn done(&self, job_id: &str, exit_code: i32, output_file_exists: bool) {
        // A guide takes hours, so its ending reaches the user even when the window does not.
        crate::attention::job_finished(self, job_id, exit_code, output_file_exists);
        let _ = self.emit(
            "job-done",
            JobDone {
                job_id: job_id.to_string(),
                exit_code,
                output_file_exists,
            },
        );
    }
}

#[derive(Default)]
pub struct ConsoleProgress;

impl ProgressSink for ConsoleProgress {
    fn started(&self, job_id: &str) {
        eprintln!("Guide Watcher job: {job_id}");
    }

    fn output(&self, _job_id: &str, line: &str) {
        if let Some(phase) = line.strip_prefix("__phase__") {
            eprintln!("\n== {phase} ==");
        } else if let Some(stderr) = line.strip_prefix("__stderr__") {
            eprintln!("{stderr}");
        } else {
            eprintln!("{line}");
        }
    }

    fn done(&self, _job_id: &str, _exit_code: i32, _output_file_exists: bool) {}
}

pub fn emit_started<S: ProgressSink + ?Sized>(sink: &S, job_id: &str) {
    sink.started(job_id);
}

pub fn emit_output<S: ProgressSink + ?Sized>(sink: &S, job_id: &str, line: impl Into<String>) {
    let line = line.into();
    sink.output(job_id, &line);
}

pub fn emit_phase<S: ProgressSink + ?Sized>(sink: &S, job_id: &str, message: &str) {
    emit_output(sink, job_id, format!("__phase__{}", message));
}

pub fn finish_job<S: ProgressSink + ?Sized>(
    sink: &S,
    job_id: &str,
    exit_code: i32,
    output_path: &Path,
) {
    let output_file_exists = output_path.exists();
    sink.done(job_id, exit_code, output_file_exists);
}

pub fn emit_error<S: ProgressSink + ?Sized>(sink: &S, job_id: &str, message: &str) {
    emit_output(sink, job_id, message);
    sink.done(job_id, -1, false);
}

pub fn derive_output_paths(filepath: &Path) -> Option<(PathBuf, String, PathBuf)> {
    let output_dir = filepath.parent()?.to_path_buf();
    let output_name = filepath
        .file_stem()
        .and_then(|stem| stem.to_str())?
        .replace(' ', "_")
        + "_Guide.md";
    let output_path = output_dir.join(&output_name);
    Some((output_dir, output_name, output_path))
}

pub fn is_prep_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_lowercase().ends_with(".prep.md"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{mpsc, Mutex};
    use std::time::Duration;

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);
    const LOCK_PROBE_ENV: &str = "GUIDE_WATCHER_TEST_LOCK_PROBE";
    const COURSE_LOCK_PROBE_ENV: &str = "GUIDE_WATCHER_TEST_COURSE_LOCK_PROBE";

    #[derive(Default)]
    struct RecordingProgress {
        events: Mutex<Vec<String>>,
    }

    impl ProgressSink for RecordingProgress {
        fn started(&self, job_id: &str) {
            self.events.lock().unwrap().push(format!("start:{job_id}"));
        }

        fn output(&self, job_id: &str, line: &str) {
            self.events
                .lock()
                .unwrap()
                .push(format!("output:{job_id}:{line}"));
        }

        fn done(&self, job_id: &str, exit_code: i32, output_file_exists: bool) {
            self.events
                .lock()
                .unwrap()
                .push(format!("done:{job_id}:{exit_code}:{output_file_exists}"));
        }
    }

    struct TestDir(std::path::PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            let sequence = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "guide_watcher_reservation_{}_{}_{}",
                std::process::id(),
                sequence,
                label
            ));
            std::fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn progress_sink_receives_the_same_lifecycle_without_a_tauri_handle() {
        let progress = RecordingProgress::default();
        emit_started(&progress, "job-1");
        emit_phase(&progress, "job-1", "Preparing");
        emit_error(&progress, "job-1", "Error: stopped");

        assert_eq!(
            *progress.events.lock().unwrap(),
            [
                "start:job-1",
                "output:job-1:__phase__Preparing",
                "output:job-1:Error: stopped",
                "done:job-1:-1:false",
            ]
        );
    }

    #[test]
    fn independent_file_handles_cannot_reserve_the_same_target_concurrently() {
        let dir = TestDir::new("same_stem");
        let pdf = dir.path().join("lecture.pdf");
        let slides = dir.path().join("lecture.pptx");
        let (_, _, first_target) = derive_output_paths(&pdf).unwrap();
        let (_, _, second_target) = derive_output_paths(&slides).unwrap();
        assert_eq!(first_target, second_target);

        let (acquired_tx, acquired_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker_target = first_target.clone();
        let worker = std::thread::spawn(move || {
            let reservation = reserve_output_path(&worker_target).unwrap();
            acquired_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            drop(reservation);
        });

        acquired_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("first reservation should be acquired");
        let error = reserve_output_path(&second_target).unwrap_err();
        assert!(error.contains("already using"), "unexpected error: {error}");

        release_tx.send(()).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn cross_process_lock_probe() {
        let Some(target) = std::env::var_os(LOCK_PROBE_ENV) else {
            return;
        };
        let error = reserve_output_path(Path::new(&target)).unwrap_err();
        assert!(
            error.contains("Another Guide Watcher process"),
            "unexpected probe error: {error}"
        );
    }

    #[test]
    fn cross_process_course_lock_probe() {
        let Some(root) = std::env::var_os(COURSE_LOCK_PROBE_ENV) else {
            return;
        };
        let error = reserve_course("test-course", Path::new(&root)).unwrap_err();
        assert!(
            error.contains("already generating course"),
            "unexpected probe error: {error}"
        );
    }

    #[test]
    fn separate_process_cannot_acquire_the_same_output_lock() {
        let dir = TestDir::new("cross_process");
        let target = dir.path().join("Guide.md");
        let _reservation = reserve_output_path(&target).expect("parent reservation");

        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "job_events::tests::cross_process_lock_probe",
                "--nocapture",
            ])
            .env(LOCK_PROBE_ENV, &target)
            .output()
            .expect("run independent lock probe process");

        assert!(
            output.status.success(),
            "lock probe failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn separate_process_cannot_acquire_the_same_course_lock() {
        let dir = TestDir::new("cross_process_course");
        let _reservation = reserve_course("test-course", dir.path()).expect("parent reservation");

        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "job_events::tests::cross_process_course_lock_probe",
                "--nocapture",
            ])
            .env(COURSE_LOCK_PROBE_ENV, dir.path())
            .output()
            .expect("run independent course-lock probe process");

        assert!(
            output.status.success(),
            "course-lock probe failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(windows)]
    #[test]
    fn case_and_slash_aliases_share_one_reservation() {
        let dir = TestDir::new("aliases");
        let target = dir.path().join("Week_One_Guide.md");
        let alias = PathBuf::from(target.to_string_lossy().replace('\\', "/").to_uppercase());

        let _reservation = reserve_output_path(&target).expect("reserve target");
        let error = reserve_output_path(&alias).unwrap_err();
        assert!(error.contains("already using"), "unexpected error: {error}");
    }

    #[test]
    fn reservation_is_released_on_drop() {
        let dir = TestDir::new("drop");
        let target = dir.path().join("Guide.md");
        let reservation = reserve_output_path(&target).expect("first reservation");
        drop(reservation);
        reserve_output_path(&target).expect("reservation should be reusable after drop");
    }

    #[test]
    fn unlocked_stale_metadata_is_reused_without_pid_based_deletion() {
        let dir = TestDir::new("stale_lock");
        let target = dir.path().join("Guide.md");
        let first = reserve_output_path(&target).expect("first reservation");
        let lock_path = first._lock_path.clone();
        let first_owner = first._owner.clone();
        drop(first);
        std::fs::write(&lock_path, "owner=stale\nprocess=1\n").unwrap();

        let second = reserve_output_path(&target).expect("stale unlocked file is reusable");
        assert_ne!(second._owner, first_owner);
        let second_owner = second._owner.clone();
        drop(second);
        let metadata = std::fs::read_to_string(lock_path).unwrap();
        assert!(metadata.contains(&format!("owner={second_owner}")));
    }

    #[test]
    fn preexisting_output_is_never_reserved_for_overwrite() {
        let dir = TestDir::new("preexisting");
        let target = dir.path().join("Existing_Guide.md");
        std::fs::write(&target, "student data that must not be overwritten").unwrap();
        let error = reserve_output_path(&target).unwrap_err();
        assert!(
            error.contains("already exists"),
            "unexpected error: {error}"
        );
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "student data that must not be overwritten"
        );
    }
}
