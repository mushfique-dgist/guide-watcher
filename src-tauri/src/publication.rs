use crate::completion::{self, CompletionStatus};
use crate::job_events::{acquire_output_lock, OutputReservation};
use crate::process_registry;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const OWNER_FILE: &str = ".guide-watcher-owner.json";
const WORKSPACE_FILE: &str = "workspace.json";
const PREPARED_FILE: &str = "prepared.json";
const ABORT_FILE: &str = "abort-requested.json";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FailPoint {
    AfterWorkspaceCreated,
    AfterPreparedSynced,
    AfterTransactionRenamed,
    AfterAssetsInstalled,
    AfterVerificationInstalled,
    BeforeGuideCommit,
    AfterGuideCommit,
    AfterFinalValidation,
    BeforeCleanup,
    AfterPrepCleanup,
}

pub(crate) trait FailureInjector: Send + Sync {
    fn hit(&self, point: FailPoint) -> Result<(), String>;
}

struct NoFailures;

impl FailureInjector for NoFailures {
    fn hit(&self, _point: FailPoint) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PrepCleanup<'a> {
    pub path: &'a Path,
    pub sha256: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PublishOutcome {
    pub prep_cleanup_warning: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct PublicationTargets {
    guide: String,
    assets: String,
    verification: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct StagingPaths {
    guide: String,
    assets: String,
    verification: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct PreparedExpected {
    source_sha256: String,
    sealed_guide_sha256: String,
    assets_tree_sha256: String,
    verification_tree_sha256: String,
    work_tree_sha256: String,
    manifest_sha256: String,
    bundle_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct PrepRecord {
    name: String,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct PreparedJournal {
    schema_version: u32,
    owner: String,
    transaction_id: String,
    cleanup_id: String,
    bundle_id: String,
    output_identity_sha256: String,
    targets: PublicationTargets,
    staging: StagingPaths,
    expected: PreparedExpected,
    prep: Option<PrepRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceMarker {
    schema_version: u32,
    owner: String,
    transaction_id: String,
    output_identity_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct OwnerMarker {
    schema_version: u32,
    owner: String,
    transaction_id: String,
    bundle_id: String,
    output_identity_sha256: String,
    guide_name: String,
    role: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct AbortMarker {
    schema_version: u32,
    owner: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CleanupMarker {
    schema_version: u32,
    owner: String,
    transaction_id: String,
    cleanup_id: String,
    output_identity_sha256: String,
    source_name: String,
    quarantine_name: String,
    role: String,
}

pub(crate) struct PublicationSession {
    output: PathBuf,
    assets: PathBuf,
    verification: PathBuf,
    work_dir: PathBuf,
    txn_dir: PathBuf,
    transaction_id: String,
    output_identity_sha256: String,
    producer_version: String,
    _lock: OutputReservation,
    injector: Box<dyn FailureInjector>,
}

impl std::fmt::Debug for PublicationSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PublicationSession")
            .field("output", &self.output)
            .field("work_dir", &self.work_dir)
            .field("transaction_id", &self.transaction_id)
            .finish_non_exhaustive()
    }
}

impl PublicationSession {
    pub(crate) fn open(output: &Path) -> Result<Self, String> {
        Self::open_with_injector(output, Box::new(NoFailures))
    }

    pub(crate) fn open_with_injector(
        output: &Path,
        injector: Box<dyn FailureInjector>,
    ) -> Result<Self, String> {
        Self::open_internal(output, injector, env!("CARGO_PKG_VERSION"))
    }

    #[cfg(test)]
    pub(crate) fn open_with_injector_and_version(
        output: &Path,
        injector: Box<dyn FailureInjector>,
        producer_version: &str,
    ) -> Result<Self, String> {
        Self::open_internal(output, injector, producer_version)
    }

    fn open_internal(
        output: &Path,
        injector: Box<dyn FailureInjector>,
        producer_version: &str,
    ) -> Result<Self, String> {
        completion::validate_v3_producer_version(producer_version)?;
        let output = canonical_output(output)?;
        let lock = acquire_output_lock(&output)?;
        recover_output_locked(&output)?;
        let (assets, verification) = final_targets(&output)?;
        reject_existing_target(&output, "guide")?;
        reject_existing_target(&assets, "learner-assets directory")?;
        reject_existing_target(&verification, "verification directory")?;
        let transaction_id = Uuid::new_v4().to_string();
        let output_identity_sha256 = output_identity_sha256(&output)?;
        let work_dir = transaction_path(&output, &transaction_id, "gwwork")?;
        let txn_dir = transaction_path(&output, &transaction_id, "gwtxn")?;
        create_new_directory(&work_dir)?;
        create_new_directory(&work_dir.join("work"))?;
        let marker = WorkspaceMarker {
            schema_version: 1,
            owner: "guide-watcher".to_string(),
            transaction_id: transaction_id.clone(),
            output_identity_sha256: output_identity_sha256.clone(),
        };
        write_json_new_synced(&work_dir.join(WORKSPACE_FILE), &marker)?;
        sync_directory(&work_dir)?;
        injector.hit(FailPoint::AfterWorkspaceCreated)?;
        Ok(Self {
            output,
            assets,
            verification,
            work_dir,
            txn_dir,
            transaction_id,
            output_identity_sha256,
            producer_version: producer_version.to_string(),
            _lock: lock,
            injector,
        })
    }

    pub(crate) fn work_path(&self) -> PathBuf {
        self.work_dir.join("work")
    }

    pub(crate) fn discard_unprepared(self) -> Result<(), String> {
        let mut hook = |_workspace: &Path| Ok(());
        self.discard_unprepared_with_hook(&mut hook)
    }

    fn discard_unprepared_with_hook(
        self,
        hook: &mut dyn FnMut(&Path) -> Result<(), String>,
    ) -> Result<(), String> {
        let label = "unprepared publication workspace";
        let delete_plan = DeletePlan::open(&self.work_dir, label)?;
        self.validate_unprepared_workspace()?;
        delete_plan.require_current_paths(label)?;
        hook(&self.work_dir)?;
        self.validate_unprepared_workspace().map_err(|error| {
            format!("{label} changed after final validation; it was preserved: {error}")
        })?;
        delete_plan.require_current_paths(label)?;
        delete_plan.delete_bottom_up(label)?;
        sync_parent(&self.work_dir)
    }

    fn validate_unprepared_workspace(&self) -> Result<(), String> {
        if self
            .work_dir
            .join(PREPARED_FILE)
            .try_exists()
            .unwrap_or(true)
            || self.txn_dir.try_exists().unwrap_or(true)
        {
            return Err("refusing to discard a prepared publication transaction".to_string());
        }
        let marker: WorkspaceMarker = read_json_strict(&self.work_dir.join(WORKSPACE_FILE))?;
        if marker.owner != "guide-watcher"
            || marker.schema_version != 1
            || marker.transaction_id != self.transaction_id
            || marker.output_identity_sha256 != self.output_identity_sha256
        {
            return Err("refusing to discard a mismatched publication workspace".to_string());
        }
        let work = self.work_dir.join("work");
        ensure_safe_tree(&self.work_dir)?;
        let mut names = directory_names(&self.work_dir)?;
        let mut expected = vec![
            std::ffi::OsString::from(WORKSPACE_FILE),
            std::ffi::OsString::from("work"),
        ];
        names.sort();
        expected.sort();
        if names != expected || !directory_names(&work)?.is_empty() {
            return Err(
                "unprepared publication workspace contains extra data and was preserved"
                    .to_string(),
            );
        }
        Ok(())
    }

    pub(crate) fn publish_verified(
        &mut self,
        verified_guide: &Path,
        staged_assets: &Path,
        staged_verification: &Path,
        source_sha256: &str,
        prep: Option<PrepCleanup<'_>>,
        cancellation: process_registry::CancellationToken,
    ) -> Result<PublishOutcome, String> {
        cancellation_barrier(cancellation, "before transaction preparation")?;
        let publish = self.work_dir.join("publish");
        create_new_directory(&publish)?;
        let publish_guide = publish.join("guide.v3.md");
        let publish_assets = publish.join("assets");
        let publish_verification = publish.join("verify");
        move_within_workspace(verified_guide, &publish_guide)?;
        move_within_workspace(staged_assets, &publish_assets)?;
        move_within_workspace(staged_verification, &publish_verification)?;

        let bundle_id = Uuid::new_v4().to_string();
        let cleanup_id = Uuid::new_v4().to_string();
        self.write_owner_marker(&publish_assets, &bundle_id, "assets")?;
        self.write_owner_marker(&publish_verification, &bundle_id, "verification")?;
        let seal = completion::seal_v3_bundle_with_version(
            &publish_guide,
            &publish_assets,
            &publish_verification,
            &self.output,
            source_sha256,
            &bundle_id,
            &self.producer_version,
        )?;
        let expected = PreparedExpected {
            source_sha256: source_sha256.to_string(),
            sealed_guide_sha256: seal.sealed_guide_sha256.clone(),
            assets_tree_sha256: tree_sha256(&publish_assets, None)?,
            verification_tree_sha256: tree_sha256(&publish_verification, None)?,
            work_tree_sha256: tree_sha256(&self.work_dir.join("work"), None)?,
            manifest_sha256: seal.manifest_sha256.clone(),
            bundle_sha256: seal.bundle_sha256.clone(),
        };
        let journal = PreparedJournal {
            schema_version: 1,
            owner: "guide-watcher".to_string(),
            transaction_id: self.transaction_id.clone(),
            cleanup_id,
            bundle_id,
            output_identity_sha256: self.output_identity_sha256.clone(),
            targets: target_names(&self.output)?,
            staging: StagingPaths {
                guide: "publish/guide.v3.md".to_string(),
                assets: "publish/assets".to_string(),
                verification: "publish/verify".to_string(),
            },
            expected,
            prep: prep
                .as_ref()
                .map(|record| prep_record(&self.output, record))
                .transpose()?,
        };
        write_json_new_synced(&self.work_dir.join(PREPARED_FILE), &journal)?;
        sync_tree(&self.work_dir)?;
        self.injector.hit(FailPoint::AfterPreparedSynced)?;
        move_no_replace(&self.work_dir, &self.txn_dir)?;
        sync_parent(&self.txn_dir)?;
        self.injector.hit(FailPoint::AfterTransactionRenamed)?;

        let txn_publish = self.txn_dir.join("publish");
        run_publication_transition(cancellation, &self.txn_dir, || {
            move_no_replace(&txn_publish.join("assets"), &self.assets)
        })?;
        sync_parent(&self.assets)?;
        self.injector.hit(FailPoint::AfterAssetsInstalled)?;

        run_publication_transition(cancellation, &self.txn_dir, || {
            move_no_replace(&txn_publish.join("verify"), &self.verification)
        })?;
        sync_parent(&self.verification)?;
        self.injector.hit(FailPoint::AfterVerificationInstalled)?;
        self.injector.hit(FailPoint::BeforeGuideCommit)?;

        run_publication_transition(cancellation, &self.txn_dir, || {
            move_no_replace(&txn_publish.join("guide.v3.md"), &self.output)
        })?;
        sync_parent(&self.output)?;
        self.injector.hit(FailPoint::AfterGuideCommit)?;
        // Once the guide exists, commit wins over later cancellation.
        require_committed_bundle(&self.output, &journal)?;
        self.injector.hit(FailPoint::AfterFinalValidation)?;
        self.injector.hit(FailPoint::BeforeCleanup)?;
        let prep_cleanup = cleanup_prep_record(&self.output, &journal)?;
        self.injector.hit(FailPoint::AfterPrepCleanup)?;
        if prep_cleanup.complete {
            remove_owned_transaction(&self.txn_dir, &journal)?;
        }
        sync_parent(&self.output)?;
        Ok(PublishOutcome {
            prep_cleanup_warning: prep_cleanup.warning,
        })
    }

    fn write_owner_marker(
        &self,
        directory: &Path,
        bundle_id: &str,
        role: &str,
    ) -> Result<(), String> {
        let marker = OwnerMarker {
            schema_version: 1,
            owner: "guide-watcher".to_string(),
            transaction_id: self.transaction_id.clone(),
            bundle_id: bundle_id.to_string(),
            output_identity_sha256: self.output_identity_sha256.clone(),
            guide_name: self
                .output
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| "guide filename is not valid Unicode".to_string())?
                .to_string(),
            role: role.to_string(),
        };
        write_json_new_synced(&directory.join(OWNER_FILE), &marker)
    }
}

pub(crate) fn recover_parent_directory(parent: &Path) -> Result<(), String> {
    let parent = parent
        .canonicalize()
        .map_err(|error| format!("could not resolve recovery directory: {error}"))?;
    let entries = std::fs::read_dir(&parent)
        .map_err(|error| format!("could not scan recovery directory: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("could not inspect recovery entry: {error}"))?;
    for entry in &entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".gwtxn") && !name.ends_with(".gwdelete") {
            continue;
        }
        let journal_path = entry.path().join(PREPARED_FILE);
        let Ok(journal) = read_json_strict::<PreparedJournal>(&journal_path) else {
            continue;
        };
        if let Ok(output) = output_from_journal(&parent, &journal) {
            let Ok(_lock) = acquire_output_lock(&output) else {
                continue;
            };
            recover_output_locked(&output)?;
        }
    }
    for entry in &entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".gwwork") {
            continue;
        }
        let marker: WorkspaceMarker = match read_json_strict(&entry.path().join(WORKSPACE_FILE)) {
            Ok(marker) => marker,
            Err(_) => continue,
        };
        let suffix = format!(".{}.gwwork", marker.transaction_id);
        let Some(guide_name) = name
            .strip_prefix('.')
            .and_then(|value| value.strip_suffix(&suffix))
        else {
            continue;
        };
        let output = parent.join(guide_name);
        let Ok(_lock) = acquire_output_lock(&output) else {
            continue;
        };
        recover_output_locked(&output)?;
    }
    Ok(())
}

/// Startup sweep. Only directories that visibly contain Guide Watcher journal
/// names are considered; each output is then independently OS-locked before
/// the strict journal validator can move or remove anything.
pub(crate) fn recover_tree(root: &Path) -> Result<(), String> {
    let mut parents = Vec::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry =
            entry.map_err(|error| format!("could not scan publication recovery tree: {error}"))?;
        if !entry.file_type().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if name.ends_with(".gwtxn") || name.ends_with(".gwwork") || name.ends_with(".gwdelete") {
            if let Some(parent) = entry.path().parent() {
                parents.push(parent.to_path_buf());
            }
        }
    }
    parents.sort();
    parents.dedup();
    let mut errors = Vec::new();
    for parent in parents {
        if let Err(error) = recover_parent_directory(&parent) {
            errors.push(format!("{}: {error}", parent.display()));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "startup recovery preserved unresolved data:\n{}",
            errors.join("\n")
        ))
    }
}

fn recover_output_locked(output: &Path) -> Result<(), String> {
    recover_transaction_quarantine_locked(output)?;
    recover_workspace_locked(output)?;
    let candidates = transaction_candidates(output)?;
    if candidates.len() > 1 {
        return Err(format!(
            "multiple publication journals claim '{}'; preserving all for manual inspection",
            output.display()
        ));
    }
    let Some(txn_dir) = candidates.first() else {
        return Ok(());
    };
    let journal: PreparedJournal = read_json_strict(&txn_dir.join(PREPARED_FILE))?;
    validate_journal(output, txn_dir, &journal)?;
    validate_transaction_for_deletion(txn_dir, &journal)?;
    let (assets, verification) = final_targets(output)?;
    match std::fs::symlink_metadata(output) {
        Ok(_) => {
            require_committed_bundle(output, &journal)?;
            complete_recovery_prep_cleanup(output, &journal)?;
            remove_owned_transaction(txn_dir, &journal)?;
            sync_parent(output)?;
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "could not inspect final guide during recovery: {error}"
            ))
        }
    }
    if std::fs::symlink_metadata(txn_dir.join(ABORT_FILE)).is_ok() {
        let abort: AbortMarker = read_json_strict(&txn_dir.join(ABORT_FILE))?;
        if abort
            != (AbortMarker {
                schema_version: 1,
                owner: "guide-watcher".to_string(),
            })
        {
            return Err("invalid cancellation marker was preserved".to_string());
        }
        rollback_owned_target(
            &assets,
            "assets",
            &journal.expected.assets_tree_sha256,
            &journal,
        )?;
        rollback_owned_target(
            &verification,
            "verification",
            &journal.expected.verification_tree_sha256,
            &journal,
        )?;
        remove_owned_transaction(txn_dir, &journal)?;
        return Ok(());
    }
    let publish = txn_dir.join("publish");
    install_or_validate_directory(
        &publish.join("assets"),
        &assets,
        "assets",
        &journal.expected.assets_tree_sha256,
        &journal,
    )?;
    install_or_validate_directory(
        &publish.join("verify"),
        &verification,
        "verification",
        &journal.expected.verification_tree_sha256,
        &journal,
    )?;
    reject_existing_target(output, "guide")?;
    let staged_guide = publish.join("guide.v3.md");
    if sha256_file(&staged_guide)? != journal.expected.sealed_guide_sha256 {
        return Err("staged recovery guide does not match its journal".to_string());
    }
    move_no_replace(&staged_guide, output)?;
    sync_parent(output)?;
    require_committed_bundle(output, &journal)?;
    complete_recovery_prep_cleanup(output, &journal)?;
    remove_owned_transaction(txn_dir, &journal)?;
    Ok(())
}

fn recover_transaction_quarantine_locked(output: &Path) -> Result<(), String> {
    let parent = output
        .parent()
        .ok_or_else(|| "output has no parent".to_string())?;
    let prefix = transaction_prefix(output)?;
    let mut candidates = std::fs::read_dir(parent)
        .map_err(|error| format!("could not scan cleanup quarantines: {error}"))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            name.starts_with(&prefix) && name.ends_with(".gwdelete")
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    candidates.sort();
    if candidates.len() > 1 {
        return Err("multiple transaction cleanup quarantines were preserved".to_string());
    }
    let Some(quarantine) = candidates.first() else {
        return Ok(());
    };
    let journal: PreparedJournal = read_json_strict(&quarantine.join(PREPARED_FILE))?;
    let expected_quarantine = transaction_quarantine_path(output, &journal)?;
    if &expected_quarantine != quarantine {
        return Err("transaction cleanup quarantine path is mismatched; preserving it".to_string());
    }
    validate_journal_contents(output, quarantine, &journal)?;
    validate_transaction_for_deletion(quarantine, &journal)?;

    match std::fs::symlink_metadata(output) {
        Ok(_) => {
            require_committed_bundle(output, &journal)?;
            complete_recovery_prep_cleanup(output, &journal)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let abort: AbortMarker = read_json_strict(&quarantine.join(ABORT_FILE))?;
            if abort
                != (AbortMarker {
                    schema_version: 1,
                    owner: "guide-watcher".to_string(),
                })
            {
                return Err("transaction quarantine lacks a valid abort marker".to_string());
            }
            let (assets, verification) = final_targets(output)?;
            rollback_owned_target(
                &assets,
                "assets",
                &journal.expected.assets_tree_sha256,
                &journal,
            )?;
            rollback_owned_target(
                &verification,
                "verification",
                &journal.expected.verification_tree_sha256,
                &journal,
            )?;
        }
        Err(error) => {
            return Err(format!(
                "could not inspect guide while recovering cleanup quarantine: {error}"
            ));
        }
    }
    let txn = transaction_path(output, &journal.transaction_id, "gwtxn")?;
    remove_owned_transaction(&txn, &journal)
}

fn recover_workspace_locked(output: &Path) -> Result<(), String> {
    let parent = output
        .parent()
        .ok_or_else(|| "output has no parent".to_string())?;
    let prefix = transaction_prefix(output)?;
    let mut owned = Vec::new();
    for entry in std::fs::read_dir(parent)
        .map_err(|error| format!("could not scan publication workspaces: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("could not read publication workspace: {error}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(&prefix) || !name.ends_with(".gwwork") {
            continue;
        }
        let marker: WorkspaceMarker = read_json_strict(&entry.path().join(WORKSPACE_FILE))?;
        if marker.schema_version != 1
            || marker.owner != "guide-watcher"
            || marker.output_identity_sha256 != output_identity_sha256(output)?
            || transaction_path(output, &marker.transaction_id, "gwwork")? != entry.path()
        {
            return Err(format!(
                "foreign or mismatched workspace preserved: {}",
                entry.path().display()
            ));
        }
        ensure_safe_tree(&entry.path())?;
        owned.push((entry.path(), marker.transaction_id));
    }
    if owned.len() > 1 {
        return Err("multiple unfinished publication workspaces were preserved".to_string());
    }
    if let Some((path, transaction_id)) = owned.pop() {
        let prepared_path = path.join(PREPARED_FILE);
        if prepared_path.is_file() {
            let journal: PreparedJournal = read_json_strict(&prepared_path)?;
            if journal.owner != "guide-watcher"
                || journal.schema_version != 1
                || journal.transaction_id != transaction_id
                || journal.output_identity_sha256 != output_identity_sha256(output)?
                || journal.targets != target_names(output)?
                || transaction_path(output, &transaction_id, "gwwork")? != path
            {
                return Err("prepared workspace identity mismatch; preserving it".to_string());
            }
            let txn = transaction_path(output, &transaction_id, "gwtxn")?;
            move_no_replace(&path, &txn)?;
            sync_parent(&txn)?;
        } else {
            let failed = transaction_path(output, &transaction_id, "gwfailed")?;
            move_no_replace(&path, &failed)?;
            sync_parent(&failed)?;
        }
    }
    Ok(())
}

fn install_or_validate_directory(
    staged: &Path,
    target: &Path,
    role: &str,
    expected_tree: &str,
    journal: &PreparedJournal,
) -> Result<(), String> {
    match std::fs::symlink_metadata(target) {
        Ok(_) => validate_owned_target(target, role, expected_tree, journal),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if tree_sha256(staged, None)? != expected_tree {
                return Err(format!("staged {role} tree does not match the journal"));
            }
            move_no_replace(staged, target)?;
            sync_parent(target)
        }
        Err(error) => Err(format!("could not inspect recovery {role} target: {error}")),
    }
}

fn rollback_owned_target(
    target: &Path,
    role: &str,
    expected_tree: &str,
    journal: &PreparedJournal,
) -> Result<(), String> {
    let mut hook = |_source: &Path, _quarantine: &Path| Ok(());
    match rollback_owned_target_with_hook(target, role, expected_tree, journal, &mut hook)? {
        QuarantineOutcome::Absent | QuarantineOutcome::Deleted => Ok(()),
        QuarantineOutcome::RejectedRestored(error) => Err(error),
    }
}

fn rollback_owned_target_with_hook(
    target: &Path,
    role: &str,
    expected_tree: &str,
    journal: &PreparedJournal,
    hook: &mut dyn FnMut(&Path, &Path) -> Result<(), String>,
) -> Result<QuarantineOutcome, String> {
    let quarantine = owned_target_quarantine(target, &journal.cleanup_id, role)?;
    let marker = cleanup_marker(journal, target, &quarantine, role)?;
    quarantine_then_delete(
        target,
        &quarantine,
        &format!("owned {role} tree"),
        &marker,
        |path| validate_owned_target(path, role, expected_tree, journal),
        hook,
    )
}

fn validate_owned_target(
    target: &Path,
    role: &str,
    expected_tree: &str,
    journal: &PreparedJournal,
) -> Result<(), String> {
    ensure_safe_tree(target)?;
    let marker: OwnerMarker = read_json_strict(&target.join(OWNER_FILE))?;
    if marker.schema_version != 1
        || marker.owner != "guide-watcher"
        || marker.transaction_id != journal.transaction_id
        || marker.bundle_id != journal.bundle_id
        || marker.output_identity_sha256 != journal.output_identity_sha256
        || marker.guide_name != journal.targets.guide
        || marker.role != role
    {
        return Err(format!("foreign or mismatched {role} target was preserved"));
    }
    if tree_sha256(target, None)? != expected_tree {
        return Err(format!("changed or extra {role} data was preserved"));
    }
    Ok(())
}

fn validate_journal(
    output: &Path,
    txn_dir: &Path,
    journal: &PreparedJournal,
) -> Result<(), String> {
    if transaction_path(output, &journal.transaction_id, "gwtxn")? != txn_dir {
        return Err("publication journal path does not match its identity".to_string());
    }
    validate_journal_contents(output, txn_dir, journal)
}

fn validate_journal_contents(
    output: &Path,
    storage_dir: &Path,
    journal: &PreparedJournal,
) -> Result<(), String> {
    if journal.schema_version != 1
        || journal.owner != "guide-watcher"
        || journal.output_identity_sha256 != output_identity_sha256(output)?
        || journal.targets != target_names(output)?
        || journal.staging
            != (StagingPaths {
                guide: "publish/guide.v3.md".to_string(),
                assets: "publish/assets".to_string(),
                verification: "publish/verify".to_string(),
            })
    {
        return Err("publication journal identity or derived paths do not match".to_string());
    }
    Uuid::parse_str(&journal.transaction_id)
        .map_err(|_| "publication journal transaction id is invalid".to_string())?;
    Uuid::parse_str(&journal.cleanup_id)
        .map_err(|_| "publication journal cleanup id is invalid".to_string())?;
    Uuid::parse_str(&journal.bundle_id)
        .map_err(|_| "publication journal bundle id is invalid".to_string())?;
    for digest in [
        &journal.expected.source_sha256,
        &journal.expected.sealed_guide_sha256,
        &journal.expected.assets_tree_sha256,
        &journal.expected.verification_tree_sha256,
        &journal.expected.work_tree_sha256,
        &journal.expected.manifest_sha256,
        &journal.expected.bundle_sha256,
    ] {
        validate_sha(digest)?;
    }
    validate_prep_record(output, journal.prep.as_ref())?;
    ensure_safe_tree(storage_dir)
}

fn require_committed_bundle(output: &Path, journal: &PreparedJournal) -> Result<(), String> {
    if sha256_file(output)? != journal.expected.sealed_guide_sha256 {
        return Err("published guide differs from its transaction journal".to_string());
    }
    match completion::inspect_completion(output) {
        CompletionStatus::ValidV3 => Ok(()),
        status => Err(format!(
            "published bundle did not validate as v3: {status:?}"
        )),
    }
}

fn remove_owned_transaction(txn_dir: &Path, journal: &PreparedJournal) -> Result<(), String> {
    let mut hook = |_source: &Path, _quarantine: &Path| Ok(());
    match remove_owned_transaction_with_hook(txn_dir, journal, &mut hook)? {
        QuarantineOutcome::Deleted => Ok(()),
        QuarantineOutcome::Absent => {
            Err("owned publication transaction disappeared during cleanup".to_string())
        }
        QuarantineOutcome::RejectedRestored(error) => Err(error),
    }
}

fn remove_owned_transaction_with_hook(
    txn_dir: &Path,
    journal: &PreparedJournal,
    hook: &mut dyn FnMut(&Path, &Path) -> Result<(), String>,
) -> Result<QuarantineOutcome, String> {
    let parent = txn_dir
        .parent()
        .ok_or_else(|| "transaction has no parent".to_string())?;
    let output = output_from_journal(parent, journal)?;
    if transaction_path(&output, &journal.transaction_id, "gwtxn")? != txn_dir {
        return Err("refusing to remove a transaction from an underived path".to_string());
    }
    let quarantine = transaction_quarantine_path(&output, journal)?;
    let marker = cleanup_marker(journal, txn_dir, &quarantine, "transaction")?;
    quarantine_then_delete(
        txn_dir,
        &quarantine,
        "publication transaction",
        &marker,
        |path| {
            validate_journal_contents(&output, path, journal)?;
            validate_transaction_for_deletion(path, journal)
        },
        hook,
    )
}

fn validate_transaction_for_deletion(
    txn_dir: &Path,
    journal: &PreparedJournal,
) -> Result<(), String> {
    ensure_safe_tree(txn_dir)?;
    let mut root_names = std::fs::read_dir(txn_dir)
        .map_err(|error| format!("could not enumerate finalized transaction: {error}"))?
        .map(|entry| {
            entry
                .map(|entry| entry.file_name())
                .map_err(|error| format!("could not read finalized transaction entry: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    root_names.sort();
    let mut expected_names = vec![
        std::ffi::OsString::from(WORKSPACE_FILE),
        std::ffi::OsString::from(PREPARED_FILE),
        std::ffi::OsString::from("work"),
        std::ffi::OsString::from("publish"),
    ];
    if std::fs::symlink_metadata(txn_dir.join(ABORT_FILE)).is_ok() {
        let abort: AbortMarker = read_json_strict(&txn_dir.join(ABORT_FILE))?;
        if abort
            != (AbortMarker {
                schema_version: 1,
                owner: "guide-watcher".to_string(),
            })
        {
            return Err("invalid cancellation marker was preserved".to_string());
        }
        expected_names.push(std::ffi::OsString::from(ABORT_FILE));
    }
    expected_names.sort();
    if root_names != expected_names {
        return Err(
            "finalized transaction contains extra or missing root entries; preserving it"
                .to_string(),
        );
    }

    let marker: WorkspaceMarker = read_json_strict(&txn_dir.join(WORKSPACE_FILE))?;
    if marker.schema_version != 1
        || marker.owner != "guide-watcher"
        || marker.transaction_id != journal.transaction_id
        || marker.output_identity_sha256 != journal.output_identity_sha256
    {
        return Err(
            "finalized transaction workspace marker does not match; preserving it".to_string(),
        );
    }
    let stored_journal: PreparedJournal = read_json_strict(&txn_dir.join(PREPARED_FILE))?;
    if &stored_journal != journal {
        return Err("finalized transaction journal changed; preserving it".to_string());
    }
    if tree_sha256(&txn_dir.join("work"), None)? != journal.expected.work_tree_sha256 {
        return Err(
            "finalized transaction work tree changed or gained data; preserving it".to_string(),
        );
    }

    let publish = txn_dir.join("publish");
    let mut publish_names = std::fs::read_dir(&publish)
        .map_err(|error| format!("could not enumerate finalized publish staging: {error}"))?
        .map(|entry| {
            entry
                .map(|entry| entry.file_name())
                .map_err(|error| format!("could not read finalized publish entry: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    publish_names.sort();
    for name in &publish_names {
        let Some(name) = name.to_str() else {
            return Err(
                "finalized publish staging has a non-Unicode entry; preserving it".to_string(),
            );
        };
        match name {
            "guide.v3.md" => {
                if sha256_file(&publish.join(name))? != journal.expected.sealed_guide_sha256 {
                    return Err("staged guide changed during recovery; preserving it".to_string());
                }
            }
            "assets" => {
                if tree_sha256(&publish.join(name), None)? != journal.expected.assets_tree_sha256 {
                    return Err(
                        "staged assets changed during recovery; preserving them".to_string()
                    );
                }
            }
            "verify" => {
                if tree_sha256(&publish.join(name), None)?
                    != journal.expected.verification_tree_sha256
                {
                    return Err(
                        "staged verification changed during recovery; preserving it".to_string()
                    );
                }
            }
            _ => {
                return Err(
                    "finalized publish staging contains extra data; preserving it".to_string(),
                )
            }
        }
    }
    Ok(())
}

fn transaction_candidates(output: &Path) -> Result<Vec<PathBuf>, String> {
    let parent = output
        .parent()
        .ok_or_else(|| "output has no parent".to_string())?;
    let prefix = transaction_prefix(output)?;
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(parent)
        .map_err(|error| format!("could not scan transaction journals: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("could not read transaction journal entry: {error}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(&prefix) && name.ends_with(".gwtxn") {
            candidates.push(entry.path());
        }
    }
    candidates.sort();
    Ok(candidates)
}

fn transaction_prefix(output: &Path) -> Result<String, String> {
    Ok(format!(
        ".{}.",
        output
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| "output filename is not valid Unicode".to_string())?
    ))
}

fn transaction_path(output: &Path, transaction_id: &str, suffix: &str) -> Result<PathBuf, String> {
    let parent = output
        .parent()
        .ok_or_else(|| "output has no parent".to_string())?;
    Ok(parent.join(format!(
        ".{}.{}.{}",
        output
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| "output filename is not valid Unicode".to_string())?,
        transaction_id,
        suffix
    )))
}

fn target_names(output: &Path) -> Result<PublicationTargets, String> {
    let (assets, verification) = final_targets(output)?;
    Ok(PublicationTargets {
        guide: filename(output)?,
        assets: filename(&assets)?,
        verification: filename(&verification)?,
    })
}

fn final_targets(output: &Path) -> Result<(PathBuf, PathBuf), String> {
    let parent = output
        .parent()
        .ok_or_else(|| "output has no parent".to_string())?;
    let stem = output
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "output stem is not valid Unicode".to_string())?;
    let name = filename(output)?;
    Ok((
        parent.join(format!("{stem}_assets")),
        parent.join(format!(".{name}.gwverify")),
    ))
}

fn output_from_journal(parent: &Path, journal: &PreparedJournal) -> Result<PathBuf, String> {
    let mut components = Path::new(&journal.targets.guide).components();
    if !matches!(components.next(), Some(std::path::Component::Normal(_)))
        || components.next().is_some()
    {
        return Err("journal guide target is not a filename".to_string());
    }
    Ok(parent.join(&journal.targets.guide))
}

fn canonical_output(output: &Path) -> Result<PathBuf, String> {
    let parent = output
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .canonicalize()
        .map_err(|error| format!("could not resolve output directory: {error}"))?;
    Ok(parent.join(
        output
            .file_name()
            .ok_or_else(|| "output has no filename".to_string())?,
    ))
}

fn output_identity_sha256(output: &Path) -> Result<String, String> {
    let mut identity = output
        .to_str()
        .ok_or_else(|| "output path is not valid Unicode".to_string())?
        .replace('\\', "/");
    if cfg!(windows) {
        identity.make_ascii_lowercase();
    }
    Ok(format!("{:x}", Sha256::digest(identity.as_bytes())))
}

fn prep_record(output: &Path, prep: &PrepCleanup<'_>) -> Result<PrepRecord, String> {
    let prep_parent = prep
        .path
        .parent()
        .ok_or_else(|| "prep packet has no parent".to_string())?
        .canonicalize()
        .map_err(|error| format!("could not resolve prep parent: {error}"))?;
    if prep_parent != output.parent().unwrap_or_else(|| Path::new(".")) {
        return Err("prep cleanup target is not beside the guide".to_string());
    }
    if filename(prep.path)? != expected_prep_name(output)? {
        return Err("prep cleanup target is not the exact derived prep sibling".to_string());
    }
    validate_sha(prep.sha256)?;
    Ok(PrepRecord {
        name: filename(prep.path)?,
        sha256: prep.sha256.to_string(),
    })
}

#[derive(Debug, Eq, PartialEq)]
struct PrepCleanupResult {
    complete: bool,
    warning: Option<String>,
}

fn expected_prep_name(output: &Path) -> Result<String, String> {
    let stem = output
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "output stem is not valid Unicode".to_string())?;
    Ok(format!("{stem}.prep.md"))
}

fn validate_prep_record(output: &Path, prep: Option<&PrepRecord>) -> Result<(), String> {
    let Some(prep) = prep else {
        return Ok(());
    };
    if prep.name != expected_prep_name(output)? {
        return Err("publication journal prep target is not the exact derived sibling".to_string());
    }
    validate_sha(&prep.sha256)
}

fn cleanup_prep_record(
    output: &Path,
    journal: &PreparedJournal,
) -> Result<PrepCleanupResult, String> {
    let prep = journal.prep.as_ref();
    validate_prep_record(output, prep)?;
    let Some(prep) = prep else {
        return Ok(PrepCleanupResult {
            complete: true,
            warning: None,
        });
    };
    let parent = output
        .parent()
        .ok_or_else(|| "output has no parent".to_string())?;
    let path = parent.join(expected_prep_name(output)?);
    let mut hook = |_source: &Path, _quarantine: &Path| Ok(());
    cleanup_prep_record_with_hook(&path, prep, journal, &mut hook)
}

fn complete_recovery_prep_cleanup(output: &Path, journal: &PreparedJournal) -> Result<(), String> {
    let result = cleanup_prep_record(output, journal)?;
    if result.complete {
        Ok(())
    } else {
        Err(result
            .warning
            .unwrap_or_else(|| "completed prep packet cleanup remains pending".to_string()))
    }
}

fn cleanup_prep_record_with_hook(
    path: &Path,
    prep: &PrepRecord,
    journal: &PreparedJournal,
    hook: &mut dyn FnMut(&Path, &Path) -> Result<(), String>,
) -> Result<PrepCleanupResult, String> {
    let quarantine = owned_target_quarantine(path, &journal.cleanup_id, "prep")?;
    let marker = cleanup_marker(journal, path, &quarantine, "prep")?;
    match quarantine_then_delete(
        path,
        &quarantine,
        "prep packet",
        &marker,
        |quarantined| {
            let metadata = std::fs::symlink_metadata(quarantined)
                .map_err(|error| format!("could not inspect quarantined prep packet: {error}"))?;
            reject_link_or_reparse(quarantined, &metadata)?;
            if !metadata.is_file() {
                return Err("quarantined prep packet is not a plain file".to_string());
            }
            if sha256_file(quarantined)? != prep.sha256 {
                return Err("completed prep packet changed and was retained".to_string());
            }
            Ok(())
        },
        hook,
    ) {
        Ok(QuarantineOutcome::Absent | QuarantineOutcome::Deleted) => Ok(PrepCleanupResult {
            complete: true,
            warning: None,
        }),
        Ok(QuarantineOutcome::RejectedRestored(warning)) => Ok(PrepCleanupResult {
            complete: true,
            warning: Some(warning),
        }),
        Err(error) => Ok(PrepCleanupResult {
            complete: false,
            warning: Some(error),
        }),
    }
}

#[derive(Debug, Eq, PartialEq)]
enum QuarantineOutcome {
    Absent,
    Deleted,
    RejectedRestored(String),
}

fn owned_target_quarantine(source: &Path, cleanup_id: &str, role: &str) -> Result<PathBuf, String> {
    Uuid::parse_str(cleanup_id).map_err(|_| "cleanup id is invalid".to_string())?;
    if role.is_empty()
        || !role
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err("cleanup role is invalid".to_string());
    }
    let parent = source
        .parent()
        .ok_or_else(|| "cleanup source has no parent".to_string())?;
    Ok(parent.join(format!(
        ".{}.{}.{}.gwdelete",
        filename(source)?,
        cleanup_id,
        role
    )))
}

fn transaction_quarantine_path(
    output: &Path,
    journal: &PreparedJournal,
) -> Result<PathBuf, String> {
    transaction_path(
        output,
        &journal.transaction_id,
        &format!("{}.gwdelete", journal.cleanup_id),
    )
}

fn cleanup_marker(
    journal: &PreparedJournal,
    source: &Path,
    quarantine: &Path,
    role: &str,
) -> Result<CleanupMarker, String> {
    Ok(CleanupMarker {
        schema_version: 1,
        owner: "guide-watcher".to_string(),
        transaction_id: journal.transaction_id.clone(),
        cleanup_id: journal.cleanup_id.clone(),
        output_identity_sha256: journal.output_identity_sha256.clone(),
        source_name: filename(source)?,
        quarantine_name: filename(quarantine)?,
        role: role.to_string(),
    })
}

fn cleanup_marker_path(quarantine: &Path) -> Result<PathBuf, String> {
    Ok(quarantine
        .parent()
        .ok_or_else(|| "cleanup quarantine has no parent".to_string())?
        .join(format!("{}.owner.json", filename(quarantine)?)))
}

fn quarantine_then_delete(
    source: &Path,
    quarantine: &Path,
    label: &str,
    expected_marker: &CleanupMarker,
    validate: impl Fn(&Path) -> Result<(), String>,
    hook: &mut dyn FnMut(&Path, &Path) -> Result<(), String>,
) -> Result<QuarantineOutcome, String> {
    if source.parent() != quarantine.parent() {
        return Err(format!("refusing cross-directory {label} quarantine"));
    }
    if expected_marker.source_name != filename(source)?
        || expected_marker.quarantine_name != filename(quarantine)?
    {
        return Err(format!("refusing mismatched {label} cleanup marker"));
    }
    let marker_path = cleanup_marker_path(quarantine)?;
    let quarantine_exists = path_exists(quarantine)?;
    let marker_exists = path_exists(&marker_path)?;
    if quarantine_exists && !marker_exists {
        return Err(format!(
            "pre-existing {label} quarantine has no journal-bound owner marker; it was preserved"
        ));
    }
    if marker_exists {
        validate_cleanup_marker(&marker_path, expected_marker)?;
    }
    if !quarantine_exists {
        if !path_exists(source)? {
            if marker_exists {
                delete_cleanup_marker(&marker_path, expected_marker)?;
            }
            return Ok(QuarantineOutcome::Absent);
        }
        if !marker_exists {
            write_json_new_synced(&marker_path, expected_marker)?;
            sync_parent(&marker_path)?;
        }
        move_no_replace(source, quarantine)?;
        sync_parent(quarantine)?;
    }

    if let Err(validation_error) = validate(quarantine) {
        if path_exists(source)? {
            return Err(format!(
                "quarantined {label} failed validation and its live path is occupied; both were preserved: {validation_error}"
            ));
        }
        move_no_replace(quarantine, source)?;
        sync_parent(source)?;
        delete_cleanup_marker(&marker_path, expected_marker)?;
        return Ok(QuarantineOutcome::RejectedRestored(format!(
            "{label} failed cleanup validation and was restored: {validation_error}"
        )));
    }
    let delete_plan = DeletePlan::open(quarantine, label)?;
    validate(quarantine).map_err(|error| {
        format!("quarantined {label} changed while delete handles were opened: {error}")
    })?;
    delete_plan.require_current_paths(label)?;
    hook(source, quarantine)?;
    validate(quarantine).map_err(|error| {
        format!("quarantined {label} changed after final validation; it was preserved: {error}")
    })?;
    delete_plan.require_current_paths(label)?;
    delete_plan.delete_bottom_up(label)?;
    sync_parent(quarantine)?;
    delete_cleanup_marker(&marker_path, expected_marker)?;
    Ok(QuarantineOutcome::Deleted)
}

fn validate_cleanup_marker(path: &Path, expected: &CleanupMarker) -> Result<(), String> {
    let actual: CleanupMarker = read_json_strict(path)?;
    if &actual == expected {
        Ok(())
    } else {
        Err("cleanup owner marker does not match the immutable journal".to_string())
    }
}

fn delete_cleanup_marker(path: &Path, expected: &CleanupMarker) -> Result<(), String> {
    validate_cleanup_marker(path, expected)?;
    let plan = DeletePlan::open(path, "cleanup owner marker")?;
    validate_cleanup_marker(path, expected)?;
    plan.require_current_paths("cleanup owner marker")?;
    plan.delete_bottom_up("cleanup owner marker")?;
    sync_parent(path)
}

#[cfg(windows)]
struct DeleteEntry {
    path: PathBuf,
    depth: usize,
    handle: same_file::Handle,
}

#[cfg(windows)]
struct DeletePlan {
    entries: Vec<DeleteEntry>,
}

#[cfg(windows)]
impl DeletePlan {
    fn open(root: &Path, label: &str) -> Result<Self, String> {
        ensure_safe_tree(root)?;
        let mut entries = Vec::new();
        for entry in walkdir::WalkDir::new(root).follow_links(false) {
            let entry = entry.map_err(|error| {
                format!("could not enumerate quarantined {label} for handle cleanup: {error}")
            })?;
            let metadata = std::fs::symlink_metadata(entry.path()).map_err(|error| {
                format!("could not inspect quarantined {label} member: {error}")
            })?;
            reject_link_or_reparse(entry.path(), &metadata)?;
            let file = open_windows_delete_handle(entry.path(), metadata.is_dir())?;
            let handle = same_file::Handle::from_file(file).map_err(|error| {
                format!("could not identify quarantined {label} member: {error}")
            })?;
            entries.push(DeleteEntry {
                path: entry.path().to_path_buf(),
                depth: entry.depth(),
                handle,
            });
        }
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.depth));
        Ok(Self { entries })
    }

    fn require_current_paths(&self, label: &str) -> Result<(), String> {
        for entry in &self.entries {
            let current = same_file::Handle::from_path(&entry.path).map_err(|error| {
                format!("quarantined {label} path changed or disappeared: {error}")
            })?;
            if current != entry.handle {
                return Err(format!(
                    "quarantined {label} path identity changed; all replacements were preserved"
                ));
            }
        }
        Ok(())
    }

    fn delete_bottom_up(self, label: &str) -> Result<(), String> {
        for entry in self.entries {
            set_delete_by_handle(entry.handle.as_file()).map_err(|error| {
                format!(
                    "could not delete quarantined {label} member by validated handle {}: {error}",
                    entry.path.display()
                )
            })?;
            drop(entry.handle);
        }
        Ok(())
    }
}

/// The same plan on macOS and Linux. Windows can hold a delete-on-close handle open, which this
/// cannot; what it keeps is the guarantee that matters, that every member is proven to be the
/// same object immediately before it is removed, so a path replaced underneath the app is
/// refused rather than followed.
#[cfg(not(windows))]
struct DeleteEntry {
    path: PathBuf,
    depth: usize,
    is_dir: bool,
    handle: same_file::Handle,
}

#[cfg(not(windows))]
struct DeletePlan {
    entries: Vec<DeleteEntry>,
}

#[cfg(not(windows))]
impl DeletePlan {
    fn open(root: &Path, label: &str) -> Result<Self, String> {
        ensure_safe_tree(root)?;
        let mut entries = Vec::new();
        for entry in walkdir::WalkDir::new(root).follow_links(false) {
            let entry = entry.map_err(|error| {
                format!("could not enumerate quarantined {label} for cleanup: {error}")
            })?;
            let metadata = std::fs::symlink_metadata(entry.path()).map_err(|error| {
                format!("could not inspect quarantined {label} member: {error}")
            })?;
            reject_link_or_reparse(entry.path(), &metadata)?;
            let handle = same_file::Handle::from_path(entry.path()).map_err(|error| {
                format!("could not identify quarantined {label} member: {error}")
            })?;
            entries.push(DeleteEntry {
                path: entry.path().to_path_buf(),
                depth: entry.depth(),
                is_dir: metadata.is_dir(),
                handle,
            });
        }
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.depth));
        Ok(Self { entries })
    }

    fn require_current_paths(&self, label: &str) -> Result<(), String> {
        for entry in &self.entries {
            let current = same_file::Handle::from_path(&entry.path).map_err(|error| {
                format!("quarantined {label} path changed or disappeared: {error}")
            })?;
            if current != entry.handle {
                return Err(format!(
                    "quarantined {label} path identity changed; all replacements were preserved"
                ));
            }
        }
        Ok(())
    }

    fn delete_bottom_up(self, label: &str) -> Result<(), String> {
        for entry in self.entries {
            // Re-check identity immediately before removing this member, not only for the tree
            // as a whole, so the gap between proof and removal stays as small as it can be.
            let metadata = std::fs::symlink_metadata(&entry.path).map_err(|error| {
                format!(
                    "quarantined {label} member disappeared before deletion {}: {error}",
                    entry.path.display()
                )
            })?;
            reject_link_or_reparse(&entry.path, &metadata)?;
            let current = same_file::Handle::from_path(&entry.path).map_err(|error| {
                format!(
                    "could not re-identify quarantined {label} member {}: {error}",
                    entry.path.display()
                )
            })?;
            if current != entry.handle {
                return Err(format!(
                    "quarantined {label} member {} was replaced before deletion; it was preserved",
                    entry.path.display()
                ));
            }
            let removed = if entry.is_dir {
                std::fs::remove_dir(&entry.path)
            } else {
                std::fs::remove_file(&entry.path)
            };
            removed.map_err(|error| {
                format!(
                    "could not delete quarantined {label} member {}: {error}",
                    entry.path.display()
                )
            })?;
        }
        Ok(())
    }
}

#[cfg(windows)]
fn open_windows_delete_handle(path: &Path, is_directory: bool) -> Result<File, String> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use std::ptr::null_mut;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateFileW(
            file_name: *const u16,
            desired_access: u32,
            share_mode: u32,
            security_attributes: *mut std::ffi::c_void,
            creation_disposition: u32,
            flags_and_attributes: u32,
            template_file: *mut std::ffi::c_void,
        ) -> *mut std::ffi::c_void;
    }

    const DELETE: u32 = 0x0001_0000;
    const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_WRITE: u32 = 0x2;
    const FILE_SHARE_DELETE: u32 = 0x4;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let flags = FILE_FLAG_OPEN_REPARSE_POINT
        | if is_directory {
            FILE_FLAG_BACKUP_SEMANTICS
        } else {
            0
        };
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(),
            DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null_mut(),
            OPEN_EXISTING,
            flags,
            null_mut(),
        )
    };
    if raw as isize == -1 {
        return Err(format!(
            "could not open cleanup handle for {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(unsafe { File::from_raw_handle(raw) })
}

#[cfg(windows)]
fn set_delete_by_handle(file: &File) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;

    #[repr(C)]
    struct FileDispositionInfo {
        delete_file: u8,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn SetFileInformationByHandle(
            file: *mut std::ffi::c_void,
            information_class: i32,
            information: *const std::ffi::c_void,
            buffer_size: u32,
        ) -> i32;
    }

    const FILE_DISPOSITION_INFO_CLASS: i32 = 4;
    let info = FileDispositionInfo { delete_file: 1 };
    let result = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FILE_DISPOSITION_INFO_CLASS,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<FileDispositionInfo>() as u32,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

fn path_exists(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("could not inspect {}: {error}", path.display())),
    }
}

fn create_new_directory(path: &Path) -> Result<(), String> {
    std::fs::create_dir(path).map_err(|error| {
        format!(
            "could not create publication directory {}: {error}",
            path.display()
        )
    })
}

fn directory_names(path: &Path) -> Result<Vec<std::ffi::OsString>, String> {
    std::fs::read_dir(path)
        .map_err(|error| format!("could not enumerate {}: {error}", path.display()))?
        .map(|entry| {
            entry
                .map(|entry| entry.file_name())
                .map_err(|error| format!("could not read entry in {}: {error}", path.display()))
        })
        .collect()
}

fn reject_existing_target(path: &Path, label: &str) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Err(format!(
            "refusing to overwrite existing {label}: {}",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "could not safely inspect {label} {}: {error}",
            path.display()
        )),
    }
}

fn write_json_new_synced(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("could not serialize publication metadata: {error}"))?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            format!(
                "could not create publication metadata {}: {error}",
                path.display()
            )
        })?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("could not durably write publication metadata: {error}"))
}

fn read_json_strict<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        format!(
            "could not inspect publication metadata {}: {error}",
            path.display()
        )
    })?;
    reject_link_or_reparse(path, &metadata)?;
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return Err(format!(
            "publication metadata is not a bounded plain file: {}",
            path.display()
        ));
    }
    let bytes = std::fs::read(path)
        .map_err(|error| format!("could not read publication metadata: {error}"))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("publication metadata is malformed: {error}"))
}

fn move_within_workspace(source: &Path, target: &Path) -> Result<(), String> {
    move_no_replace(source, target)
}

#[cfg(windows)]
fn move_no_replace(source: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    let source_wide = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let target_wide = target
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            target_wide.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(format!(
            "could not atomically move {} to {} without replacement: {}",
            source.display(),
            target.display(),
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn move_no_replace(source: &Path, target: &Path) -> Result<(), String> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    extern "C" {
        fn renameat2(
            olddirfd: i32,
            oldpath: *const i8,
            newdirfd: i32,
            newpath: *const i8,
            flags: u32,
        ) -> i32;
    }
    const AT_FDCWD: i32 = -100;
    const RENAME_NOREPLACE: u32 = 1;
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| "source path contains NUL".to_string())?;
    let target = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| "target path contains NUL".to_string())?;
    let result = unsafe {
        renameat2(
            AT_FDCWD,
            source.as_ptr(),
            AT_FDCWD,
            target.as_ptr(),
            RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(format!(
            "no-replace rename failed: {}",
            std::io::Error::last_os_error()
        ))
    }
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
fn move_no_replace(_source: &Path, _target: &Path) -> Result<(), String> {
    Err("no safe directory no-replace primitive is implemented for this Unix target".to_string())
}

fn sync_tree(root: &Path) -> Result<(), String> {
    ensure_safe_tree(root)?;
    let mut directories = Vec::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry =
            entry.map_err(|error| format!("could not traverse publication tree: {error}"))?;
        let metadata = std::fs::symlink_metadata(entry.path())
            .map_err(|error| format!("could not inspect publication tree: {error}"))?;
        reject_link_or_reparse(entry.path(), &metadata)?;
        if metadata.is_file() {
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(entry.path())
                .and_then(|file| file.sync_all())
                .map_err(|error| format!("could not sync publication file: {error}"))?;
        } else if metadata.is_dir() {
            directories.push(entry.path().to_path_buf());
        }
    }
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        sync_directory(&directory)?;
    }
    Ok(())
}

fn ensure_safe_tree(root: &Path) -> Result<(), String> {
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry =
            entry.map_err(|error| format!("could not traverse owned publication tree: {error}"))?;
        let metadata = std::fs::symlink_metadata(entry.path())
            .map_err(|error| format!("could not inspect owned publication tree: {error}"))?;
        reject_link_or_reparse(entry.path(), &metadata)?;
        if !metadata.is_file() && !metadata.is_dir() {
            return Err(format!(
                "owned publication tree contains a special entry: {}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

fn tree_sha256(root: &Path, omitted: Option<&str>) -> Result<String, String> {
    ensure_safe_tree(root)?;
    let mut records = Vec::new();
    for entry in walkdir::WalkDir::new(root).min_depth(1).follow_links(false) {
        let entry =
            entry.map_err(|error| format!("could not traverse publication tree: {error}"))?;
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| "publication member escaped its tree".to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        if omitted == Some(relative.as_str()) {
            continue;
        }
        let metadata = entry.metadata().map_err(|error| error.to_string())?;
        if metadata.is_dir() {
            records.push((
                relative,
                "directory",
                0,
                format!("{:x}", Sha256::digest(b"guide-watcher-directory-v1")),
            ));
        } else if metadata.is_file() {
            records.push((relative, "file", metadata.len(), sha256_file(entry.path())?));
        } else {
            return Err("publication tree contains a special entry".to_string());
        }
    }
    records.sort();
    let mut digest = Sha256::new();
    digest.update(b"guide-watcher-tree-v1\0");
    for (path, kind, size, sha) in records {
        digest.update((path.len() as u32).to_be_bytes());
        digest.update(path.as_bytes());
        digest.update((kind.len() as u32).to_be_bytes());
        digest.update(kind.as_bytes());
        digest.update(size.to_be_bytes());
        digest.update(sha.as_bytes());
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("could not open {} for hashing: {error}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("could not hash {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn validate_sha(value: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err("publication metadata contains an invalid SHA-256 digest".to_string())
    }
}

fn filename(path: &Path) -> Result<String, String> {
    path.file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "publication filename is not valid Unicode".to_string())
}

fn cancellation_barrier(
    cancellation: process_registry::CancellationToken,
    checkpoint: &str,
) -> Result<(), String> {
    if process_registry::is_cancelled(cancellation) {
        Err(format!("job was cancelled {checkpoint}"))
    } else {
        Ok(())
    }
}

fn run_publication_transition(
    cancellation: process_registry::CancellationToken,
    txn_dir: &Path,
    action: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    match process_registry::run_if_current(cancellation, action) {
        Ok(result) => result,
        Err(error) => {
            mark_abort_requested(txn_dir)?;
            Err(format!(
                "{error} before the guide commit; recovery will roll back owned artifacts"
            ))
        }
    }
}

fn mark_abort_requested(txn_dir: &Path) -> Result<(), String> {
    let path = txn_dir.join(ABORT_FILE);
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            let mut bytes = serde_json::to_vec(&AbortMarker {
                schema_version: 1,
                owner: "guide-watcher".to_string(),
            })
            .map_err(|error| format!("could not serialize cancellation marker: {error}"))?;
            bytes.push(b'\n');
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|error| format!("could not durably record cancellation: {error}"))
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(format!("could not record cancellation request: {error}")),
    }
}

fn sync_parent(path: &Path) -> Result<(), String> {
    sync_directory(
        path.parent()
            .ok_or_else(|| "path has no parent".to_string())?,
    )
}

fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("could not sync directory {}: {error}", path.display()))?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn reject_link_or_reparse(path: &Path, metadata: &std::fs::Metadata) -> Result<(), String> {
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "symbolic link preserved in publication data: {}",
            path.display()
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(format!(
                "reparse point preserved in publication data: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {

    #[test]
    fn file_hashing_is_bounded_on_a_small_stack() {
        crate::artifact_bundle::assert_file_hash_on_small_stack(
            "publication::tests::file_hashing_is_bounded_on_a_small_stack",
            sha256_file,
        );
    }

    use super::*;
    use std::sync::{Arc, Barrier, Mutex};

    struct TestRoot(PathBuf);
    impl TestRoot {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("guide-watcher-publication-{}", Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct FailOnce(Mutex<Option<FailPoint>>);
    impl FailureInjector for FailOnce {
        fn hit(&self, point: FailPoint) -> Result<(), String> {
            let mut target = self.0.lock().unwrap();
            if *target == Some(point) {
                *target = None;
                Err(format!("injected failure at {point:?}"))
            } else {
                Ok(())
            }
        }
    }

    struct CancelAt {
        point: Mutex<Option<FailPoint>>,
        token: process_registry::CancellationToken,
    }
    impl FailureInjector for CancelAt {
        fn hit(&self, point: FailPoint) -> Result<(), String> {
            let mut target = self.point.lock().unwrap();
            if *target == Some(point) {
                *target = None;
                process_registry::cancel(self.token);
            }
            Ok(())
        }
    }

    struct SynchronizeAt {
        point: FailPoint,
        barrier: Arc<Barrier>,
        cancel: Option<process_registry::CancellationToken>,
    }
    impl FailureInjector for SynchronizeAt {
        fn hit(&self, point: FailPoint) -> Result<(), String> {
            if point == self.point {
                self.barrier.wait();
                if let Some(token) = self.cancel {
                    process_registry::cancel(token);
                }
            }
            Ok(())
        }
    }

    fn staged(session: &PublicationSession) -> (PathBuf, PathBuf, PathBuf) {
        let work = session.work_path();
        let guide = work.join("candidate.md");
        let assets = work.join("learner-assets");
        let verify = work.join("authoritative-inputs");
        std::fs::create_dir(&assets).unwrap();
        std::fs::create_dir(&verify).unwrap();
        std::fs::write(&guide, b"# verified\n").unwrap();
        std::fs::write(assets.join("image.png"), b"png").unwrap();
        std::fs::write(verify.join("coverage.json"), b"{}\n").unwrap();
        (guide, assets, verify)
    }

    #[test]
    fn publish_commits_guide_last_and_validates_v3() {
        let root = TestRoot::new();
        let output = root.0.join("Unicode 한글 Guide.md");
        let mut session = PublicationSession::open(&output).unwrap();
        let (guide, assets, verify) = staged(&session);
        session
            .publish_verified(
                &guide,
                &assets,
                &verify,
                &"a".repeat(64),
                None,
                process_registry::cancellation_token(),
            )
            .unwrap();
        assert_eq!(
            completion::inspect_completion(&output),
            CompletionStatus::ValidV3
        );
        assert!(!transaction_candidates(&output)
            .unwrap()
            .iter()
            .any(|path| path.exists()));
    }

    #[test]
    fn crash_after_assets_install_recovers_idempotently() {
        let root = TestRoot::new();
        let output = root.0.join("Guide.md");
        let injector = Box::new(FailOnce(Mutex::new(Some(FailPoint::AfterAssetsInstalled))));
        let mut session = PublicationSession::open_with_injector(&output, injector).unwrap();
        let (guide, assets, verify) = staged(&session);
        assert!(session
            .publish_verified(
                &guide,
                &assets,
                &verify,
                &"b".repeat(64),
                None,
                process_registry::cancellation_token()
            )
            .is_err());
        drop(session);
        let recovered = PublicationSession::open(&output).unwrap_err();
        assert!(
            recovered.contains("guide already exists")
                || recovered.contains("overwrite existing guide")
        );
        assert_eq!(
            completion::inspect_completion(&output),
            CompletionStatus::ValidV3
        );
        recover_output_locked(&output).unwrap();
    }

    #[test]
    fn foreign_final_target_is_never_deleted() {
        let root = TestRoot::new();
        let output = root.0.join("Guide.md");
        std::fs::create_dir(root.0.join("Guide_assets")).unwrap();
        std::fs::write(root.0.join("Guide_assets/foreign"), b"keep").unwrap();
        assert!(PublicationSession::open(&output).is_err());
        assert_eq!(
            std::fs::read(root.0.join("Guide_assets/foreign")).unwrap(),
            b"keep"
        );
    }

    #[test]
    fn every_post_prepare_crash_point_recovers_to_one_valid_commit() {
        for point in [
            FailPoint::AfterPreparedSynced,
            FailPoint::AfterTransactionRenamed,
            FailPoint::AfterAssetsInstalled,
            FailPoint::AfterVerificationInstalled,
            FailPoint::BeforeGuideCommit,
            FailPoint::AfterGuideCommit,
            FailPoint::AfterFinalValidation,
            FailPoint::BeforeCleanup,
            FailPoint::AfterPrepCleanup,
        ] {
            let root = TestRoot::new();
            let output = root.0.join(format!("Guide-{point:?}.md"));
            let mut session = PublicationSession::open_with_injector(
                &output,
                Box::new(FailOnce(Mutex::new(Some(point)))),
            )
            .unwrap();
            let (guide, assets, verify) = staged(&session);
            assert!(session
                .publish_verified(
                    &guide,
                    &assets,
                    &verify,
                    &"c".repeat(64),
                    None,
                    process_registry::cancellation_token(),
                )
                .is_err());
            drop(session);
            let error = PublicationSession::open(&output).unwrap_err();
            assert!(
                error.contains("overwrite existing guide"),
                "{point:?}: {error}"
            );
            assert_eq!(
                completion::inspect_completion(&output),
                CompletionStatus::ValidV3
            );
            assert!(transaction_candidates(&output).unwrap().is_empty());
        }
    }

    #[test]
    fn unprepared_crash_is_preserved_as_failed_and_never_published() {
        let root = TestRoot::new();
        let output = root.0.join("Guide.md");
        let error = PublicationSession::open_with_injector(
            &output,
            Box::new(FailOnce(Mutex::new(Some(FailPoint::AfterWorkspaceCreated)))),
        )
        .unwrap_err();
        assert!(error.contains("injected failure"));
        let session = PublicationSession::open(&output).unwrap();
        assert!(!output.exists());
        assert_eq!(
            std::fs::read_dir(&root.0)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().ends_with(".gwfailed"))
                .count(),
            1
        );
        session.discard_unprepared().unwrap();
    }

    #[test]
    fn cancellation_before_commit_rolls_back_but_after_commit_is_success() {
        let root = TestRoot::new();
        let cancelled_output = root.0.join("Cancelled.md");
        let token = process_registry::cancellation_token();
        let mut cancelled = PublicationSession::open_with_injector(
            &cancelled_output,
            Box::new(CancelAt {
                point: Mutex::new(Some(FailPoint::AfterVerificationInstalled)),
                token,
            }),
        )
        .unwrap();
        let (guide, assets, verify) = staged(&cancelled);
        assert!(cancelled
            .publish_verified(&guide, &assets, &verify, &"d".repeat(64), None, token)
            .is_err());
        drop(cancelled);
        let recovered = PublicationSession::open(&cancelled_output).unwrap();
        assert!(!cancelled_output.exists());
        assert!(!root.0.join("Cancelled_assets").exists());
        assert!(!root.0.join(".Cancelled.md.gwverify").exists());
        recovered.discard_unprepared().unwrap();

        let committed_output = root.0.join("Committed.md");
        let token = process_registry::cancellation_token();
        let mut committed = PublicationSession::open_with_injector(
            &committed_output,
            Box::new(CancelAt {
                point: Mutex::new(Some(FailPoint::AfterGuideCommit)),
                token,
            }),
        )
        .unwrap();
        let (guide, assets, verify) = staged(&committed);
        committed
            .publish_verified(&guide, &assets, &verify, &"e".repeat(64), None, token)
            .unwrap();
        assert_eq!(
            completion::inspect_completion(&committed_output),
            CompletionStatus::ValidV3
        );
    }

    #[test]
    fn scoped_cancellation_cannot_cross_parallel_course_publications() {
        let root = TestRoot::new();
        let course_a = root.0.join("Course-A");
        let course_b = root.0.join("Course-B");
        std::fs::create_dir(&course_a).unwrap();
        std::fs::create_dir(&course_b).unwrap();
        let output_a = course_a.join("Guide-A.md");
        let output_b = course_b.join("Guide-B.md");
        let token_a = process_registry::cancellation_token();
        let token_b = process_registry::cancellation_token();
        let barrier = Arc::new(Barrier::new(2));

        let first_output = output_a.clone();
        let first_barrier = barrier.clone();
        let first = std::thread::spawn(move || {
            let mut session = PublicationSession::open_with_injector(
                &first_output,
                Box::new(SynchronizeAt {
                    point: FailPoint::AfterVerificationInstalled,
                    barrier: first_barrier,
                    cancel: Some(token_a),
                }),
            )
            .unwrap();
            let (guide, assets, verify) = staged(&session);
            session.publish_verified(&guide, &assets, &verify, &"1".repeat(64), None, token_a)
        });

        let second_output = output_b.clone();
        let second = std::thread::spawn(move || {
            let mut session = PublicationSession::open_with_injector(
                &second_output,
                Box::new(SynchronizeAt {
                    point: FailPoint::AfterVerificationInstalled,
                    barrier,
                    cancel: None,
                }),
            )
            .unwrap();
            let (guide, assets, verify) = staged(&session);
            session.publish_verified(&guide, &assets, &verify, &"2".repeat(64), None, token_b)
        });

        assert!(first.join().unwrap().is_err());
        assert!(second.join().unwrap().is_ok());
        assert!(process_registry::is_cancelled(token_a));
        assert!(!process_registry::is_cancelled(token_b));
        assert!(!output_a.exists());
        assert_eq!(
            completion::inspect_completion(&output_b),
            CompletionStatus::ValidV3
        );
        process_registry::finish(token_a);
        process_registry::finish(token_b);
    }

    #[test]
    fn changed_owned_target_and_multiple_journals_are_preserved() {
        let root = TestRoot::new();
        let output = root.0.join("Guide.md");
        let mut session = PublicationSession::open_with_injector(
            &output,
            Box::new(FailOnce(Mutex::new(Some(FailPoint::AfterAssetsInstalled)))),
        )
        .unwrap();
        let (guide, assets, verify) = staged(&session);
        assert!(session
            .publish_verified(
                &guide,
                &assets,
                &verify,
                &"f".repeat(64),
                None,
                process_registry::cancellation_token()
            )
            .is_err());
        std::fs::write(root.0.join("Guide_assets/foreign-extra"), b"keep").unwrap();
        drop(session);
        assert!(PublicationSession::open(&output).is_err());
        assert_eq!(
            std::fs::read(root.0.join("Guide_assets/foreign-extra")).unwrap(),
            b"keep"
        );

        let other = root.0.join("Other.md");
        for _ in 0..2 {
            std::fs::create_dir(root.0.join(format!(".Other.md.{}.gwtxn", Uuid::new_v4())))
                .unwrap();
        }
        assert!(PublicationSession::open(&other)
            .unwrap_err()
            .contains("multiple publication journals"));
    }

    #[test]
    fn prep_cleanup_is_crash_idempotent_before_and_after_the_removal() {
        for point in [FailPoint::BeforeCleanup, FailPoint::AfterPrepCleanup] {
            let root = TestRoot::new();
            let output = root.0.join(format!("Cleanup-{point:?}.md"));
            let prep = root.0.join(format!("Cleanup-{point:?}.prep.md"));
            std::fs::write(&prep, b"bound prep packet").unwrap();
            let prep_sha = sha256_file(&prep).unwrap();
            let mut session = PublicationSession::open_with_injector(
                &output,
                Box::new(FailOnce(Mutex::new(Some(point)))),
            )
            .unwrap();
            let txn = session.txn_dir.clone();
            let (guide, assets, verify) = staged(&session);
            assert!(session
                .publish_verified(
                    &guide,
                    &assets,
                    &verify,
                    &"1".repeat(64),
                    Some(PrepCleanup {
                        path: &prep,
                        sha256: &prep_sha,
                    }),
                    process_registry::cancellation_token(),
                )
                .is_err());
            assert!(txn.exists());
            assert_eq!(
                prep.exists(),
                point == FailPoint::BeforeCleanup,
                "{point:?}"
            );
            drop(session);

            let error = PublicationSession::open(&output).unwrap_err();
            assert!(
                error.contains("overwrite existing guide"),
                "{point:?}: {error}"
            );
            assert!(!prep.exists());
            assert!(!txn.exists());
            assert_eq!(
                completion::inspect_completion(&output),
                CompletionStatus::ValidV3
            );
        }
    }

    #[test]
    fn replaced_prep_is_retained_but_does_not_pin_a_committed_transaction() {
        let root = TestRoot::new();
        let output = root.0.join("Guide.md");
        let prep = root.0.join("Guide.prep.md");
        std::fs::write(&prep, b"original prep packet").unwrap();
        let prep_sha = sha256_file(&prep).unwrap();
        let mut session = PublicationSession::open_with_injector(
            &output,
            Box::new(FailOnce(Mutex::new(Some(FailPoint::BeforeCleanup)))),
        )
        .unwrap();
        let txn = session.txn_dir.clone();
        let (guide, assets, verify) = staged(&session);
        assert!(session
            .publish_verified(
                &guide,
                &assets,
                &verify,
                &"2".repeat(64),
                Some(PrepCleanup {
                    path: &prep,
                    sha256: &prep_sha,
                }),
                process_registry::cancellation_token(),
            )
            .is_err());
        std::fs::write(&prep, b"external replacement").unwrap();
        drop(session);

        assert!(PublicationSession::open(&output).is_err());
        assert_eq!(std::fs::read(&prep).unwrap(), b"external replacement");
        assert!(!txn.exists());
        assert_eq!(
            completion::inspect_completion(&output),
            CompletionStatus::ValidV3
        );
    }

    #[test]
    fn tampered_prep_target_never_selects_an_arbitrary_sibling() {
        let root = TestRoot::new();
        let output = root.0.join("Guide.md");
        let prep = root.0.join("Guide.prep.md");
        let decoy = root.0.join("decoy.txt");
        std::fs::write(&prep, b"prep packet").unwrap();
        std::fs::write(&decoy, b"do not delete").unwrap();
        let prep_sha = sha256_file(&prep).unwrap();
        let mut session = PublicationSession::open_with_injector(
            &output,
            Box::new(FailOnce(Mutex::new(Some(FailPoint::AfterPreparedSynced)))),
        )
        .unwrap();
        let journal_path = session.work_dir.join(PREPARED_FILE);
        let (guide, assets, verify) = staged(&session);
        assert!(session
            .publish_verified(
                &guide,
                &assets,
                &verify,
                &"3".repeat(64),
                Some(PrepCleanup {
                    path: &prep,
                    sha256: &prep_sha,
                }),
                process_registry::cancellation_token(),
            )
            .is_err());
        let mut journal: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&journal_path).unwrap()).unwrap();
        journal["prep"]["name"] = serde_json::Value::String("decoy.txt".to_string());
        journal["prep"]["sha256"] = serde_json::Value::String(sha256_file(&decoy).unwrap());
        std::fs::write(&journal_path, serde_json::to_vec_pretty(&journal).unwrap()).unwrap();
        drop(session);

        let error = PublicationSession::open(&output).unwrap_err();
        assert!(error.contains("prep target"), "{error}");
        assert_eq!(std::fs::read(&decoy).unwrap(), b"do not delete");
        assert_eq!(std::fs::read(&prep).unwrap(), b"prep packet");
        assert!(!output.exists());
        assert_eq!(transaction_candidates(&output).unwrap().len(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn discard_unprepared_preserves_a_post_validation_member_replacement() {
        let root = TestRoot::new();
        let output = root.0.join("Guide.md");
        let session = PublicationSession::open(&output).unwrap();
        let workspace = session.work_dir.clone();
        let original_marker = root.0.join("validated-original-workspace.json");
        let marker_bytes = std::fs::read(workspace.join(WORKSPACE_FILE)).unwrap();

        let mut replace = |source: &Path| {
            move_no_replace(&source.join(WORKSPACE_FILE), &original_marker)?;
            std::fs::write(source.join(WORKSPACE_FILE), &marker_bytes)
                .map_err(|error| error.to_string())
        };
        let error = session
            .discard_unprepared_with_hook(&mut replace)
            .unwrap_err();

        assert!(error.contains("path identity changed"), "{error}");
        assert!(workspace.is_dir());
        assert!(workspace.join("work").is_dir());
        assert_eq!(
            std::fs::read(workspace.join(WORKSPACE_FILE)).unwrap(),
            marker_bytes
        );
        assert_eq!(std::fs::read(original_marker).unwrap(), marker_bytes);
    }

    #[test]
    fn foreign_files_in_workspaces_and_transactions_are_never_deleted() {
        let root = TestRoot::new();
        let unprepared_output = root.0.join("Unprepared.md");
        let session = PublicationSession::open(&unprepared_output).unwrap();
        let workspace = session.work_dir.clone();
        let foreign_work = session.work_path().join("foreign.txt");
        let foreign_work_dir = session.work_path().join("foreign-empty");
        let foreign_workspace_dir = workspace.join("foreign-root-empty");
        std::fs::write(&foreign_work, b"keep work").unwrap();
        std::fs::create_dir(&foreign_work_dir).unwrap();
        std::fs::create_dir(&foreign_workspace_dir).unwrap();
        assert!(session.discard_unprepared().is_err());
        assert_eq!(std::fs::read(&foreign_work).unwrap(), b"keep work");
        assert!(foreign_work_dir.is_dir());
        assert!(foreign_workspace_dir.is_dir());
        assert!(workspace.exists());

        let output = root.0.join("Prepared.md");
        let mut session = PublicationSession::open_with_injector(
            &output,
            Box::new(FailOnce(Mutex::new(Some(
                FailPoint::AfterTransactionRenamed,
            )))),
        )
        .unwrap();
        let txn = session.txn_dir.clone();
        let (guide, assets, verify) = staged(&session);
        assert!(session
            .publish_verified(
                &guide,
                &assets,
                &verify,
                &"4".repeat(64),
                None,
                process_registry::cancellation_token(),
            )
            .is_err());
        let foreign_txn = txn.join("foreign.txt");
        let foreign_txn_dir = txn.join("foreign-empty");
        std::fs::write(&foreign_txn, b"keep txn").unwrap();
        std::fs::create_dir(&foreign_txn_dir).unwrap();
        drop(session);
        assert!(PublicationSession::open(&output).is_err());
        assert_eq!(std::fs::read(&foreign_txn).unwrap(), b"keep txn");
        assert!(foreign_txn_dir.is_dir());
        assert!(!output.exists());
    }

    #[test]
    fn empty_directories_in_hashed_work_or_final_trees_are_preserved_and_block_recovery() {
        let root = TestRoot::new();
        let work_output = root.0.join("Work.md");
        let mut work_session = PublicationSession::open_with_injector(
            &work_output,
            Box::new(FailOnce(Mutex::new(Some(
                FailPoint::AfterTransactionRenamed,
            )))),
        )
        .unwrap();
        let work_txn = work_session.txn_dir.clone();
        let (guide, assets, verify) = staged(&work_session);
        assert!(work_session
            .publish_verified(
                &guide,
                &assets,
                &verify,
                &"6".repeat(64),
                None,
                process_registry::cancellation_token(),
            )
            .is_err());
        let injected_work_dir = work_txn.join("work/foreign-empty");
        std::fs::create_dir(&injected_work_dir).unwrap();
        drop(work_session);
        assert!(PublicationSession::open(&work_output).is_err());
        assert!(injected_work_dir.is_dir());
        assert!(!work_output.exists());

        let final_output = root.0.join("Final.md");
        let mut final_session = PublicationSession::open_with_injector(
            &final_output,
            Box::new(FailOnce(Mutex::new(Some(
                FailPoint::AfterVerificationInstalled,
            )))),
        )
        .unwrap();
        let (guide, assets, verify) = staged(&final_session);
        assert!(final_session
            .publish_verified(
                &guide,
                &assets,
                &verify,
                &"7".repeat(64),
                None,
                process_registry::cancellation_token(),
            )
            .is_err());
        let (final_assets, final_verify) = final_targets(&final_output).unwrap();
        let asset_empty = final_assets.join("foreign-empty");
        let verify_empty = final_verify.join("foreign-empty");
        std::fs::create_dir(&asset_empty).unwrap();
        std::fs::create_dir(&verify_empty).unwrap();
        drop(final_session);
        assert!(PublicationSession::open(&final_output).is_err());
        assert!(asset_empty.is_dir());
        assert!(verify_empty.is_dir());
        assert!(!final_output.exists());
    }

    #[test]
    fn cleanup_quarantines_never_delete_recreated_live_paths() {
        let root = TestRoot::new();

        let prep_output = root.0.join("Prep.md");
        let prep = root.0.join("Prep.prep.md");
        std::fs::write(&prep, b"original prep").unwrap();
        let prep_sha = sha256_file(&prep).unwrap();
        let mut prep_session = PublicationSession::open_with_injector(
            &prep_output,
            Box::new(FailOnce(Mutex::new(Some(FailPoint::BeforeCleanup)))),
        )
        .unwrap();
        let (guide, assets, verify) = staged(&prep_session);
        assert!(prep_session
            .publish_verified(
                &guide,
                &assets,
                &verify,
                &"8".repeat(64),
                Some(PrepCleanup {
                    path: &prep,
                    sha256: &prep_sha,
                }),
                process_registry::cancellation_token(),
            )
            .is_err());
        let prep_journal: PreparedJournal =
            read_json_strict(&prep_session.txn_dir.join(PREPARED_FILE)).unwrap();
        let prep_record = prep_journal.prep.as_ref().unwrap();
        let mut replace_prep = |source: &Path, _quarantine: &Path| {
            std::fs::write(source, b"replacement prep").map_err(|error| error.to_string())
        };
        let result =
            cleanup_prep_record_with_hook(&prep, prep_record, &prep_journal, &mut replace_prep)
                .unwrap();
        assert!(result.complete);
        assert_eq!(std::fs::read(&prep).unwrap(), b"replacement prep");

        let target_output = root.0.join("Target.md");
        let mut target_session = PublicationSession::open_with_injector(
            &target_output,
            Box::new(FailOnce(Mutex::new(Some(FailPoint::AfterAssetsInstalled)))),
        )
        .unwrap();
        let (guide, assets, verify) = staged(&target_session);
        assert!(target_session
            .publish_verified(
                &guide,
                &assets,
                &verify,
                &"9".repeat(64),
                None,
                process_registry::cancellation_token(),
            )
            .is_err());
        let target_journal: PreparedJournal =
            read_json_strict(&target_session.txn_dir.join(PREPARED_FILE)).unwrap();
        let assets_target = final_targets(&target_output).unwrap().0;
        let mut replace_target = |source: &Path, _quarantine: &Path| {
            std::fs::create_dir(source).map_err(|error| error.to_string())?;
            std::fs::write(source.join("foreign"), b"replacement")
                .map_err(|error| error.to_string())
        };
        assert_eq!(
            rollback_owned_target_with_hook(
                &assets_target,
                "assets",
                &target_journal.expected.assets_tree_sha256,
                &target_journal,
                &mut replace_target,
            )
            .unwrap(),
            QuarantineOutcome::Deleted
        );
        assert_eq!(
            std::fs::read(assets_target.join("foreign")).unwrap(),
            b"replacement"
        );

        let txn_output = root.0.join("Txn.md");
        let mut txn_session = PublicationSession::open_with_injector(
            &txn_output,
            Box::new(FailOnce(Mutex::new(Some(
                FailPoint::AfterTransactionRenamed,
            )))),
        )
        .unwrap();
        let (guide, assets, verify) = staged(&txn_session);
        assert!(txn_session
            .publish_verified(
                &guide,
                &assets,
                &verify,
                &"a".repeat(64),
                None,
                process_registry::cancellation_token(),
            )
            .is_err());
        let txn_journal: PreparedJournal =
            read_json_strict(&txn_session.txn_dir.join(PREPARED_FILE)).unwrap();
        let mut replace_txn = |source: &Path, _quarantine: &Path| {
            std::fs::create_dir(source).map_err(|error| error.to_string())?;
            std::fs::write(source.join("foreign"), b"replacement")
                .map_err(|error| error.to_string())
        };
        assert_eq!(
            remove_owned_transaction_with_hook(
                &txn_session.txn_dir,
                &txn_journal,
                &mut replace_txn,
            )
            .unwrap(),
            QuarantineOutcome::Deleted
        );
        assert_eq!(
            std::fs::read(txn_session.txn_dir.join("foreign")).unwrap(),
            b"replacement"
        );
    }

    #[test]
    fn crash_committed_bundle_from_an_older_app_version_recovers() {
        let root = TestRoot::new();
        let output = root.0.join("OldVersion.md");
        let mut session = PublicationSession::open_with_injector_and_version(
            &output,
            Box::new(FailOnce(Mutex::new(Some(FailPoint::AfterGuideCommit)))),
            "1.5.0-previous",
        )
        .unwrap();
        let (guide, assets, verify) = staged(&session);
        assert!(session
            .publish_verified(
                &guide,
                &assets,
                &verify,
                &"b".repeat(64),
                None,
                process_registry::cancellation_token(),
            )
            .is_err());
        drop(session);
        assert!(PublicationSession::open(&output).is_err());
        assert_eq!(
            completion::inspect_completion(&output),
            CompletionStatus::ValidV3
        );
        assert!(transaction_candidates(&output).unwrap().is_empty());
    }

    #[test]
    fn crash_after_transaction_quarantine_is_restart_recoverable() {
        let root = TestRoot::new();
        let output = root.0.join("QuarantinedTxn.md");
        let mut session = PublicationSession::open_with_injector(
            &output,
            Box::new(FailOnce(Mutex::new(Some(FailPoint::BeforeCleanup)))),
        )
        .unwrap();
        let (guide, assets, verify) = staged(&session);
        assert!(session
            .publish_verified(
                &guide,
                &assets,
                &verify,
                &"c".repeat(64),
                None,
                process_registry::cancellation_token(),
            )
            .is_err());
        let journal: PreparedJournal =
            read_json_strict(&session.txn_dir.join(PREPARED_FILE)).unwrap();
        let quarantine = transaction_quarantine_path(&output, &journal).unwrap();
        let mut crash = |_source: &Path, _quarantine: &Path| {
            Err("injected crash after quarantine validation".to_string())
        };
        assert!(
            remove_owned_transaction_with_hook(&session.txn_dir, &journal, &mut crash).is_err()
        );
        assert!(!session.txn_dir.exists());
        assert!(quarantine.exists());
        drop(session);

        assert!(PublicationSession::open(&output).is_err());
        assert!(!quarantine.exists());
        assert_eq!(
            completion::inspect_completion(&output),
            CompletionStatus::ValidV3
        );
    }

    #[test]
    fn data_injected_into_a_validated_quarantine_is_preserved() {
        let root = TestRoot::new();
        let output = root.0.join("QuarantineMutation.md");
        let mut session = PublicationSession::open_with_injector(
            &output,
            Box::new(FailOnce(Mutex::new(Some(
                FailPoint::AfterTransactionRenamed,
            )))),
        )
        .unwrap();
        let (guide, assets, verify) = staged(&session);
        assert!(session
            .publish_verified(
                &guide,
                &assets,
                &verify,
                &"d".repeat(64),
                None,
                process_registry::cancellation_token(),
            )
            .is_err());
        let journal: PreparedJournal =
            read_json_strict(&session.txn_dir.join(PREPARED_FILE)).unwrap();
        let quarantine = transaction_quarantine_path(&output, &journal).unwrap();
        let mut inject = |_source: &Path, quarantined: &Path| {
            std::fs::create_dir(quarantined.join("foreign-empty"))
                .map_err(|error| error.to_string())
        };
        assert!(
            remove_owned_transaction_with_hook(&session.txn_dir, &journal, &mut inject).is_err()
        );
        assert!(quarantine.join("foreign-empty").is_dir());
        assert!(!session.txn_dir.exists());
    }

    #[test]
    fn post_validation_quarantine_swap_and_unmarked_collision_are_preserved() {
        let root = TestRoot::new();
        for case in ["swap", "collision"] {
            let output = root.0.join(format!("Quarantine-{case}.md"));
            let prep = root.0.join(format!("Quarantine-{case}.prep.md"));
            std::fs::write(&prep, b"exact prep bytes").unwrap();
            let prep_sha = sha256_file(&prep).unwrap();
            let mut session = PublicationSession::open_with_injector(
                &output,
                Box::new(FailOnce(Mutex::new(Some(FailPoint::BeforeCleanup)))),
            )
            .unwrap();
            let (guide, assets, verify) = staged(&session);
            assert!(session
                .publish_verified(
                    &guide,
                    &assets,
                    &verify,
                    &"1".repeat(64),
                    Some(PrepCleanup {
                        path: &prep,
                        sha256: &prep_sha,
                    }),
                    process_registry::cancellation_token(),
                )
                .is_err());
            let journal: PreparedJournal =
                read_json_strict(&session.txn_dir.join(PREPARED_FILE)).unwrap();
            let quarantine = owned_target_quarantine(&prep, &journal.cleanup_id, "prep").unwrap();

            if case == "collision" {
                std::fs::copy(&prep, &quarantine).unwrap();
                let mut no_hook = |_source: &Path, _quarantine: &Path| Ok(());
                let result = cleanup_prep_record_with_hook(
                    &prep,
                    journal.prep.as_ref().unwrap(),
                    &journal,
                    &mut no_hook,
                )
                .unwrap();
                assert!(!result.complete);
                assert_eq!(std::fs::read(&prep).unwrap(), b"exact prep bytes");
                assert_eq!(std::fs::read(&quarantine).unwrap(), b"exact prep bytes");
            } else {
                let saved = root.0.join("original-quarantine-saved");
                let mut swap = |_source: &Path, quarantined: &Path| {
                    move_no_replace(quarantined, &saved)?;
                    std::fs::copy(&saved, quarantined)
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                };
                let result = cleanup_prep_record_with_hook(
                    &prep,
                    journal.prep.as_ref().unwrap(),
                    &journal,
                    &mut swap,
                )
                .unwrap();
                assert!(!result.complete);
                assert_eq!(std::fs::read(&saved).unwrap(), b"exact prep bytes");
                assert_eq!(std::fs::read(&quarantine).unwrap(), b"exact prep bytes");
            }
        }
    }

    #[test]
    fn prep_and_final_tree_quarantines_are_restart_recoverable() {
        let root = TestRoot::new();
        let prep_output = root.0.join("PrepCrash.md");
        let prep = root.0.join("PrepCrash.prep.md");
        std::fs::write(&prep, b"prep").unwrap();
        let prep_sha = sha256_file(&prep).unwrap();
        let mut prep_session = PublicationSession::open_with_injector(
            &prep_output,
            Box::new(FailOnce(Mutex::new(Some(FailPoint::BeforeCleanup)))),
        )
        .unwrap();
        let (guide, assets, verify) = staged(&prep_session);
        assert!(prep_session
            .publish_verified(
                &guide,
                &assets,
                &verify,
                &"e".repeat(64),
                Some(PrepCleanup {
                    path: &prep,
                    sha256: &prep_sha,
                }),
                process_registry::cancellation_token(),
            )
            .is_err());
        let prep_journal: PreparedJournal =
            read_json_strict(&prep_session.txn_dir.join(PREPARED_FILE)).unwrap();
        let prep_quarantine =
            owned_target_quarantine(&prep, &prep_journal.cleanup_id, "prep").unwrap();
        let mut crash = |_source: &Path, _quarantine: &Path| {
            Err("injected crash in prep quarantine".to_string())
        };
        let cleanup = cleanup_prep_record_with_hook(
            &prep,
            prep_journal.prep.as_ref().unwrap(),
            &prep_journal,
            &mut crash,
        )
        .unwrap();
        assert!(!cleanup.complete);
        assert!(prep_quarantine.exists());
        drop(prep_session);
        assert!(PublicationSession::open(&prep_output).is_err());
        assert!(!prep_quarantine.exists());

        let tree_output = root.0.join("TreeCrash.md");
        let mut tree_session = PublicationSession::open_with_injector(
            &tree_output,
            Box::new(FailOnce(Mutex::new(Some(
                FailPoint::AfterVerificationInstalled,
            )))),
        )
        .unwrap();
        let (guide, assets, verify) = staged(&tree_session);
        assert!(tree_session
            .publish_verified(
                &guide,
                &assets,
                &verify,
                &"f".repeat(64),
                None,
                process_registry::cancellation_token(),
            )
            .is_err());
        mark_abort_requested(&tree_session.txn_dir).unwrap();
        let tree_journal: PreparedJournal =
            read_json_strict(&tree_session.txn_dir.join(PREPARED_FILE)).unwrap();
        let assets_target = final_targets(&tree_output).unwrap().0;
        let assets_quarantine =
            owned_target_quarantine(&assets_target, &tree_journal.cleanup_id, "assets").unwrap();
        let mut crash = |_source: &Path, _quarantine: &Path| {
            Err("injected crash in tree quarantine".to_string())
        };
        assert!(rollback_owned_target_with_hook(
            &assets_target,
            "assets",
            &tree_journal.expected.assets_tree_sha256,
            &tree_journal,
            &mut crash,
        )
        .is_err());
        assert!(assets_quarantine.exists());
        drop(tree_session);
        let recovered = PublicationSession::open(&tree_output).unwrap();
        assert!(!assets_quarantine.exists());
        assert!(!tree_output.exists());
        recovered.discard_unprepared().unwrap();
    }

    #[test]
    fn malformed_workspace_and_owner_markers_are_preserved() {
        let root = TestRoot::new();
        let unprepared_output = root.0.join("BadWorkspace.md");
        let session = PublicationSession::open(&unprepared_output).unwrap();
        let workspace = session.work_dir.clone();
        let marker_path = workspace.join(WORKSPACE_FILE);
        let mut marker: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&marker_path).unwrap()).unwrap();
        marker["schema_version"] = serde_json::Value::from(2);
        std::fs::write(&marker_path, serde_json::to_vec_pretty(&marker).unwrap()).unwrap();
        assert!(session.discard_unprepared().is_err());
        assert!(workspace.exists());

        for (case, value) in [
            ("schema", serde_json::Value::from(2)),
            (
                "guide-name",
                serde_json::Value::String("Different.md".to_string()),
            ),
        ] {
            let output = root.0.join(format!("BadOwner-{case}.md"));
            let mut session = PublicationSession::open_with_injector(
                &output,
                Box::new(FailOnce(Mutex::new(Some(FailPoint::AfterAssetsInstalled)))),
            )
            .unwrap();
            let (guide, assets, verify) = staged(&session);
            assert!(session
                .publish_verified(
                    &guide,
                    &assets,
                    &verify,
                    &"5".repeat(64),
                    None,
                    process_registry::cancellation_token(),
                )
                .is_err());
            let owner_path = final_targets(&output).unwrap().0.join(OWNER_FILE);
            let mut owner: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&owner_path).unwrap()).unwrap();
            if case == "schema" {
                owner["schema_version"] = value;
            } else {
                owner["guide_name"] = value;
            }
            std::fs::write(&owner_path, serde_json::to_vec_pretty(&owner).unwrap()).unwrap();
            drop(session);

            assert!(PublicationSession::open(&output).is_err());
            assert!(owner_path.exists());
            assert!(!output.exists());
        }
    }

    #[test]
    fn no_replace_move_preserves_an_existing_target_byte_for_byte() {
        let root = TestRoot::new();
        let source = root.0.join("source");
        let target = root.0.join("target");
        std::fs::write(&source, b"new").unwrap();
        std::fs::write(&target, b"old").unwrap();
        assert!(move_no_replace(&source, &target).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert_eq!(std::fs::read(&source).unwrap(), b"new");
    }
}
