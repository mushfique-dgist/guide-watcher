use crate::codex::{self, HybridRunConfig, ValidatedPrepPacket};
use crate::course_plan::{PlannedGuide, PlannedJobDto, ResolvedCourse};
#[cfg(test)]
use crate::job_events::reserve_output_path;
use crate::job_events::{emit_started, reserve_course, CourseReservation, SharedProgress};
use crate::process_registry;
use crate::publication::PublicationSession;
use crate::source_context::{self, SourceMaterial};
use futures::{future::join_all, stream, StreamExt};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};

const MAX_CONCURRENT_VISUAL_GUIDES: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BatchJobStatus {
    Succeeded,
    Failed,
    BlockedByPredecessor,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchJobResult {
    pub job_id: String,
    pub course_profile: String,
    pub guide_path: PathBuf,
    pub status: BatchJobStatus,
    pub message: Option<String>,
}

pub struct PreparedGuide {
    pub plan: PlannedGuide,
    pub source_material: SourceMaterial,
    publication: PublicationSession,
    precollected_vision: Option<Result<codex::VisionBatchReport, String>>,
}

pub struct PreparedBatch {
    guides: Vec<PreparedGuide>,
    _course_reservations: Vec<CourseReservation>,
}

pub struct PreparedResume {
    packet: ValidatedPrepPacket,
    source_material: SourceMaterial,
    publication: PublicationSession,
    _course_reservation: CourseReservation,
}

impl PreparedBatch {
    pub fn planned_jobs(&self) -> Result<Vec<PlannedJobDto>, String> {
        self.guides
            .iter()
            .map(|guide| PlannedJobDto::try_from(&guide.plan))
            .collect()
    }
}

impl PreparedResume {
    pub fn output_path(&self) -> &Path {
        &self.packet.plan.output_path
    }

    /// How the predecessor guides differ from when the context was saved.
    pub fn predecessor_drift(&self) -> &[String] {
        &self.packet.predecessor_drift
    }

    /// Run every check the resume performs before a model starts (packet re-read, source
    /// pack binding, contract and digest agreement, visual-plan re-binding), then release the
    /// workspace. Returns the notes the resume itself would report.
    pub fn dry_run(self) -> Result<Vec<String>, String> {
        let PreparedResume {
            packet,
            source_material,
            publication,
            _course_reservation,
        } = self;
        let outcome = codex::dry_run_prepared_resume(&packet, source_material);
        publication.discard_unprepared()?;
        outcome
    }
}

impl PreparedBatch {
    /// Release every reserved publication workspace without running any provider.
    pub fn discard(self) -> Result<(), String> {
        for guide in self.guides {
            guide.publication.discard_unprepared()?;
        }
        Ok(())
    }
}

pub fn preflight_resume_path(
    prep_path: &Path,
    cancellation: process_registry::CancellationToken,
) -> Result<PreparedResume, String> {
    let loaded = codex::load_resume_prep(prep_path)?;
    let course = crate::course_plan::configured_course(&loaded.contract.course_profile)?;
    let course_reservation = reserve_course(&course.id, &course.root)?;
    let packet = codex::validate_loaded_resume_prep(loaded)?;
    preflight_locked_resume(
        packet,
        course_reservation,
        &source_context::NativeSourceRenderer,
        cancellation,
    )
}

fn preflight_locked_resume(
    packet: ValidatedPrepPacket,
    course_reservation: CourseReservation,
    renderer: &dyn source_context::LocalSourceRenderer,
    cancellation: process_registry::CancellationToken,
) -> Result<PreparedResume, String> {
    preflight_locked_resume_with_runtime_probe(
        packet,
        course_reservation,
        renderer,
        &source_context::NativeVerifierRuntimeProbe,
        cancellation,
    )
}

fn preflight_locked_resume_with_runtime_probe(
    packet: ValidatedPrepPacket,
    course_reservation: CourseReservation,
    renderer: &dyn source_context::LocalSourceRenderer,
    runtime_probe: &dyn source_context::VerifierRuntimeProbe,
    cancellation: process_registry::CancellationToken,
) -> Result<PreparedResume, String> {
    ensure_preflight_current(cancellation)?;
    validate_runtime_files()?;
    codex::recheck_validated_prep(&packet)?;
    let plan = &packet.plan;
    let source_material = source_context::preflight_source_material_with_runtime_probe(
        plan,
        renderer,
        runtime_probe,
        cancellation,
    )?;
    ensure_preflight_current(cancellation)?;
    source_context::preflight_predecessors(plan, &HashSet::new())?;
    let publication = PublicationSession::open(&plan.output_path)?;
    Ok(PreparedResume {
        packet,
        source_material,
        publication,
        _course_reservation: course_reservation,
    })
}

trait BatchSequenceItem {
    fn plan(&self) -> &PlannedGuide;
}

impl BatchSequenceItem for PreparedGuide {
    fn plan(&self) -> &PlannedGuide {
        &self.plan
    }
}

#[cfg(test)]
impl BatchSequenceItem for PlannedGuide {
    fn plan(&self) -> &PlannedGuide {
        self
    }
}

pub fn preflight_paths(
    paths: &[PathBuf],
    requested_profile: &str,
    cancellation: process_registry::CancellationToken,
) -> Result<PreparedBatch, String> {
    ensure_preflight_current(cancellation)?;
    let (paths, courses) = crate::course_plan::resolve_course_targets(paths, requested_profile)?;
    let course_reservations = acquire_course_reservations(&courses)?;
    let plans = crate::course_plan::plan_guides(&paths, requested_profile)?;
    preflight_locked_plans(
        plans,
        course_reservations,
        &source_context::NativeSourceRenderer,
        cancellation,
    )
}

#[cfg(test)]
fn preflight_paths_with_courses_and_renderer(
    paths: &[PathBuf],
    requested_profile: &str,
    courses: &[ResolvedCourse],
    renderer: &dyn source_context::LocalSourceRenderer,
    cancellation: process_registry::CancellationToken,
    before_lock: impl FnOnce(),
) -> Result<PreparedBatch, String> {
    let (paths, targets) =
        crate::course_plan::resolve_course_targets_with_courses(paths, requested_profile, courses)?;
    before_lock();
    let course_reservations = acquire_course_reservations(&targets)?;
    let plans = crate::course_plan::plan_guides_with_courses(&paths, requested_profile, courses)?;
    for plan in &plans {
        crate::visual_assets::write_no_visual_packet_for_test(plan)?;
    }
    preflight_locked_plans(plans, course_reservations, renderer, cancellation)
}

fn acquire_course_reservations(
    courses: &[ResolvedCourse],
) -> Result<Vec<CourseReservation>, String> {
    let mut reservations = Vec::with_capacity(courses.len());
    for course in courses {
        reservations.push(reserve_course(&course.id, &course.root)?);
    }
    Ok(reservations)
}

fn preflight_locked_plans(
    plans: Vec<PlannedGuide>,
    course_reservations: Vec<CourseReservation>,
    renderer: &dyn source_context::LocalSourceRenderer,
    cancellation: process_registry::CancellationToken,
) -> Result<PreparedBatch, String> {
    preflight_locked_plans_with_runtime_probe(
        plans,
        course_reservations,
        renderer,
        &source_context::NativeVerifierRuntimeProbe,
        cancellation,
    )
}

fn preflight_locked_plans_with_runtime_probe(
    plans: Vec<PlannedGuide>,
    course_reservations: Vec<CourseReservation>,
    renderer: &dyn source_context::LocalSourceRenderer,
    runtime_probe: &dyn source_context::VerifierRuntimeProbe,
    cancellation: process_registry::CancellationToken,
) -> Result<PreparedBatch, String> {
    let mut resource_budget = source_context::PreflightResourceBudget::new();
    preflight_locked_plans_with_runtime_probe_and_budget(
        plans,
        course_reservations,
        renderer,
        runtime_probe,
        &mut resource_budget,
        cancellation,
    )
}

fn preflight_locked_plans_with_runtime_probe_and_budget(
    plans: Vec<PlannedGuide>,
    course_reservations: Vec<CourseReservation>,
    renderer: &dyn source_context::LocalSourceRenderer,
    runtime_probe: &dyn source_context::VerifierRuntimeProbe,
    resource_budget: &mut source_context::PreflightResourceBudget,
    cancellation: process_registry::CancellationToken,
) -> Result<PreparedBatch, String> {
    ensure_preflight_current(cancellation)?;
    if plans.is_empty() {
        return Err("batch contains no planned guides".to_string());
    }
    let plans = plans
        .into_iter()
        .map(codex::canonicalize_plan)
        .collect::<Result<Vec<_>, _>>()?;
    validate_runtime_files()?;
    validate_in_batch_predecessors(&plans)?;
    validate_artifact_collisions(&plans)?;

    let pending_outputs = plans
        .iter()
        .map(|plan| plan.output_path.clone())
        .collect::<HashSet<_>>();
    let mut material = Vec::with_capacity(plans.len());
    let mut unique_sources = HashMap::new();
    let mut total_source_bytes = 0_u64;
    let mut total_rendered_bytes = 0_u64;
    for (plan_index, plan) in plans.iter().enumerate() {
        ensure_preflight_current(cancellation)?;
        let remaining_guides = (plans.len() - plan_index) as u64;
        let automatic_visual_limit = resource_budget.remaining_visual_decoded() / remaining_guides;
        let captured = source_context::preflight_source_material_with_runtime_probe_and_budget_and_visual_limit(
                plan,
                renderer,
                runtime_probe,
                resource_budget,
                automatic_visual_limit,
                cancellation,
            )?;
        ensure_preflight_current(cancellation)?;
        source_context::preflight_predecessors(plan, &pending_outputs)?;
        for source in &captured.captured_sources {
            match unique_sources.insert(source.path.clone(), source.sha256.clone()) {
                None => {
                    total_source_bytes = total_source_bytes
                        .checked_add(source.size_bytes)
                        .ok_or_else(|| {
                            "batch source sizes overflowed preflight accounting".to_string()
                        })?;
                }
                Some(previous) if previous != source.sha256 => {
                    return Err(format!(
                        "source changed while the whole batch was being preflighted: {}",
                        source.path.display()
                    ));
                }
                Some(_) => {}
            }
        }
        for dependency in &captured.context_dependencies {
            match unique_sources.insert(dependency.path.clone(), dependency.sha256.clone()) {
                None => {
                    total_source_bytes = total_source_bytes
                        .checked_add(dependency.size_bytes)
                        .ok_or_else(|| {
                            "batch context sizes overflowed preflight accounting".to_string()
                        })?;
                }
                Some(previous) if previous != dependency.sha256 => {
                    return Err(format!(
                        "context dependency changed while the whole batch was being preflighted: {}",
                        dependency.path.display()
                    ));
                }
                Some(_) => {}
            }
        }
        for dependency in &captured.runtime_dependencies {
            match unique_sources.insert(dependency.path.clone(), dependency.sha256.clone()) {
                None => {
                    total_source_bytes = total_source_bytes
                        .checked_add(dependency.size_bytes)
                        .ok_or_else(|| {
                            "batch runtime-dependency sizes overflowed preflight accounting"
                                .to_string()
                        })?;
                }
                Some(previous) if previous != dependency.sha256 => {
                    return Err(format!(
                        "runtime dependency changed while the whole batch was being preflighted: {}",
                        dependency.path.display()
                    ));
                }
                Some(_) => {}
            }
        }
        for dependency in &captured.visual_material.dependencies {
            match unique_sources.insert(dependency.path.clone(), dependency.sha256.clone()) {
                None => {
                    total_source_bytes = total_source_bytes
                        .checked_add(dependency.size_bytes)
                        .ok_or_else(|| {
                            "batch visual-input sizes overflowed preflight accounting".to_string()
                        })?;
                }
                Some(previous) if previous != dependency.sha256 => {
                    return Err(format!(
                        "visual dependency changed while the whole batch was being preflighted: {}",
                        dependency.path.display()
                    ));
                }
                Some(_) => {}
            }
        }
        for rendered in &captured.rendered_sources {
            for image in &rendered.images {
                total_rendered_bytes = total_rendered_bytes
                    .checked_add(image.bytes.len() as u64)
                    .ok_or_else(|| {
                    "batch rendered-image sizes overflowed preflight accounting".to_string()
                })?;
            }
        }
        for asset in &captured.visual_material.compiled_assets {
            total_rendered_bytes = total_rendered_bytes
                .checked_add(asset.bytes.len() as u64)
                .ok_or_else(|| {
                    "batch compiled-visual sizes overflowed preflight accounting".to_string()
                })?;
        }
        material.push(captured);
    }
    if total_source_bytes > source_context::MAX_BATCH_SOURCE_BYTES {
        return Err(format!(
            "batch sources exceed the {} MiB preflight limit",
            source_context::MAX_BATCH_SOURCE_BYTES / 1024 / 1024
        ));
    }
    if total_rendered_bytes > source_context::MAX_BATCH_RENDERED_BYTES {
        return Err(format!(
            "batch rendered images exceed the {} MiB preflight limit",
            source_context::MAX_BATCH_RENDERED_BYTES / 1024 / 1024
        ));
    }
    let mut publications = Vec::with_capacity(plans.len());
    for plan in &plans {
        ensure_preflight_current(cancellation)?;
        match PublicationSession::open(&plan.output_path) {
            Ok(publication) => publications.push(publication),
            Err(error) => {
                for publication in publications {
                    let _ = publication.discard_unprepared();
                }
                return Err(error);
            }
        }
    }
    let guides = plans
        .into_iter()
        .zip(material)
        .zip(publications)
        .map(|((plan, source_material), publication)| PreparedGuide {
            plan,
            source_material,
            publication,
            precollected_vision: None,
        })
        .collect();
    Ok(PreparedBatch {
        guides,
        _course_reservations: course_reservations,
    })
}

fn ensure_preflight_current(
    cancellation: process_registry::CancellationToken,
) -> Result<(), String> {
    if process_registry::is_cancelled(cancellation) {
        Err("guide preflight was cancelled".to_string())
    } else {
        Ok(())
    }
}

fn validate_runtime_files() -> Result<(), String> {
    for (path, label) in [
        (crate::config::TEMPLATE_FILE, "template file"),
        (crate::config::DEPTH_CONTRACT_FILE, "depth contract"),
        (crate::config::RENDER_SLIDES_SCRIPT, "source renderer"),
        (crate::config::GUIDE_LINT_SCRIPT, "guide verifier"),
        (
            crate::config::GUIDE_LINT_REQUIREMENTS,
            "guide verifier dependency lock",
        ),
    ] {
        let path = Path::new(path);
        if !path.is_file() {
            return Err(format!(
                "required {label} is not a readable file: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn validate_in_batch_predecessors(plans: &[PlannedGuide]) -> Result<(), String> {
    let outputs = plans
        .iter()
        .enumerate()
        .map(|(index, plan)| (normalized_path_identity(&plan.output_path), (index, plan)))
        .collect::<HashMap<_, _>>();
    for (index, plan) in plans.iter().enumerate() {
        for predecessor in &plan.predecessors {
            let Some((predecessor_index, predecessor_plan)) =
                outputs.get(&normalized_path_identity(&predecessor.path))
            else {
                continue;
            };
            if *predecessor_index >= index {
                return Err(format!(
                    "planned predecessor is not earlier in the batch: {}",
                    predecessor.path.display()
                ));
            }
            if predecessor.course_profile != plan.course.id
                || predecessor.course_profile != predecessor_plan.course.id
                || predecessor.generation_identity != predecessor_plan.generation_identity
                || predecessor.sequence_key != predecessor_plan.sequence_key
            {
                return Err(format!(
                    "planned predecessor identity does not match its in-batch guide: {}",
                    predecessor.path.display()
                ));
            }
        }
    }
    Ok(())
}

fn validate_artifact_collisions(plans: &[PlannedGuide]) -> Result<(), String> {
    let mut reserved = HashSet::new();
    for plan in plans {
        for (label, path) in planned_artifacts(plan)? {
            let identity = normalized_path_identity(&path);
            if !reserved.insert(identity) {
                return Err(format!(
                    "two planned jobs collide on the same {label}: {}",
                    path.display()
                ));
            }
            if label != "prep packet" {
                continue;
            }
            match path.try_exists() {
                Ok(false) => {}
                Ok(true) => {
                    return Err(format!(
                        "refusing to overwrite existing {label}: {}",
                        path.display()
                    ));
                }
                Err(error) => {
                    return Err(format!(
                        "could not safely inspect planned {label} {}: {error}",
                        path.display()
                    ));
                }
            }
        }
    }
    Ok(())
}

fn planned_artifacts(plan: &PlannedGuide) -> Result<Vec<(&'static str, PathBuf)>, String> {
    let output = &plan.output_path;
    let parent = output
        .parent()
        .ok_or_else(|| "planned output has no parent directory".to_string())?;
    let stem = output
        .file_stem()
        .ok_or_else(|| "planned output has no file stem".to_string())?;
    let stem_text = stem
        .to_str()
        .ok_or_else(|| "planned output filename is not valid Unicode".to_string())?;
    Ok(vec![
        ("guide output", output.clone()),
        ("prep packet", parent.join(format!("{stem_text}.prep.md"))),
        (
            "learner-assets directory",
            parent.join(format!("{stem_text}_assets")),
        ),
        (
            "verification directory",
            PathBuf::from(format!("{}.gwverify", output.to_string_lossy())),
        ),
    ])
}

fn normalized_path_identity(path: &Path) -> String {
    let mut identity = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        identity.make_ascii_lowercase();
    }
    identity
}

pub async fn execute_batch(
    progress: SharedProgress,
    prepared: PreparedBatch,
    config: HybridRunConfig,
    cancellation: process_registry::CancellationToken,
) -> Vec<BatchJobResult> {
    let PreparedBatch {
        mut guides,
        _course_reservations,
    } = prepared;
    let _course_reservations = _course_reservations;
    guides = precollect_visual_contexts(progress.clone(), guides, &config, cancellation).await;
    let progress_for_execute = progress.clone();
    let progress_for_blocked = progress.clone();
    execute_course_chains(
        guides,
        |prepared| {
            let progress = progress_for_execute.clone();
            let config = config.clone();
            async move {
                let PreparedGuide {
                    plan,
                    source_material,
                    publication,
                    precollected_vision,
                } = prepared;
                codex::execute_prepared_generate(
                    progress,
                    plan.job_id.clone(),
                    codex::PreparedGeneration {
                        plan,
                        source_material,
                        publication,
                        precollected_vision,
                    },
                    config,
                    cancellation,
                )
                .await
            }
        },
        move |result| emit_blocked_job(&progress_for_blocked, result),
    )
    .await
}

pub async fn execute_resume(
    progress: SharedProgress,
    job_id: String,
    prepared: PreparedResume,
    config: HybridRunConfig,
    cancellation: process_registry::CancellationToken,
) -> bool {
    let PreparedResume {
        packet,
        source_material,
        publication,
        _course_reservation,
    } = prepared;
    let _course_reservation = _course_reservation;
    codex::execute_prepared_resume(
        progress,
        job_id,
        packet,
        source_material,
        publication,
        config,
        cancellation,
    )
    .await
}

async fn precollect_visual_contexts(
    progress: SharedProgress,
    guides: Vec<PreparedGuide>,
    config: &HybridRunConfig,
    cancellation: process_registry::CancellationToken,
) -> Vec<PreparedGuide> {
    let config = config.clone();
    map_bounded_ordered(guides, MAX_CONCURRENT_VISUAL_GUIDES, |mut guide| {
        let progress = progress.clone();
        let config = config.clone();
        async move {
            emit_started(&progress, &guide.plan.job_id);
            guide.precollected_vision = Some(
                codex::precollect_visual_context(
                    &progress,
                    &guide.plan.job_id,
                    &guide.source_material,
                    &config,
                    cancellation,
                )
                .await,
            );
            guide
        }
    })
    .await
}

async fn map_bounded_ordered<T, U, F, Fut>(items: Vec<T>, limit: usize, map: F) -> Vec<U>
where
    F: Fn(T) -> Fut,
    Fut: Future<Output = U>,
{
    let mut completed = stream::iter(items.into_iter().enumerate().map(|(index, item)| {
        let future = map(item);
        async move { (index, future.await) }
    }))
    .buffer_unordered(limit.max(1))
    .collect::<Vec<_>>()
    .await;
    completed.sort_by_key(|(index, _)| *index);
    completed.into_iter().map(|(_, output)| output).collect()
}

async fn execute_course_chains<T, F, Fut, B>(
    plans: Vec<T>,
    execute: F,
    blocked: B,
) -> Vec<BatchJobResult>
where
    T: BatchSequenceItem,
    F: Fn(T) -> Fut,
    Fut: Future<Output = bool>,
    B: Fn(&BatchJobResult),
{
    let mut course_indexes = HashMap::<String, usize>::new();
    let mut chains = Vec::<Vec<(usize, T)>>::new();
    for (index, prepared) in plans.into_iter().enumerate() {
        let course_profile = prepared.plan().course.id.clone();
        let chain_index = match course_indexes.get(&course_profile) {
            Some(index) => *index,
            None => {
                let index = chains.len();
                course_indexes.insert(course_profile, index);
                chains.push(Vec::new());
                index
            }
        };
        chains[chain_index].push((index, prepared));
    }

    let grouped = join_all(
        chains
            .into_iter()
            .map(|chain| execute_course_chain(chain, &execute, &blocked)),
    )
    .await;
    let mut indexed = grouped.into_iter().flatten().collect::<Vec<_>>();
    indexed.sort_by_key(|(index, _)| *index);
    indexed.into_iter().map(|(_, result)| result).collect()
}

async fn execute_course_chain<T, F, Fut, B>(
    plans: Vec<(usize, T)>,
    execute: &F,
    blocked: &B,
) -> Vec<(usize, BatchJobResult)>
where
    T: BatchSequenceItem,
    F: Fn(T) -> Fut,
    Fut: Future<Output = bool>,
    B: Fn(&BatchJobResult),
{
    let mut failed_courses = HashSet::new();
    let mut results = Vec::with_capacity(plans.len());
    for (index, prepared) in plans {
        let job_id = prepared.plan().job_id.clone();
        let course_profile = prepared.plan().course.id.clone();
        let guide_path = prepared.plan().output_path.clone();
        if failed_courses.contains(&course_profile) {
            let result = BatchJobResult {
                job_id,
                course_profile,
                guide_path,
                status: BatchJobStatus::BlockedByPredecessor,
                message: Some(
                    "an earlier guide in this course failed, so cumulative generation was not started"
                        .to_string(),
                ),
            };
            blocked(&result);
            results.push((index, result));
            continue;
        }
        let succeeded = execute(prepared).await;
        let status = if succeeded {
            BatchJobStatus::Succeeded
        } else {
            failed_courses.insert(course_profile.clone());
            BatchJobStatus::Failed
        };
        results.push((
            index,
            BatchJobResult {
                job_id,
                course_profile,
                guide_path,
                status,
                message: (!succeeded).then(|| "guide generation failed".to_string()),
            },
        ));
    }
    results
}

fn emit_blocked_job(progress: &SharedProgress, result: &BatchJobResult) {
    emit_started(progress, &result.job_id);
    progress.blocked(
        &result.job_id,
        result.message.as_deref().unwrap_or("guide was blocked"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::course_plan::{
        GuideKind, GuideMode, LecturePrimaryRule, PlannedPredecessor, ResolvedCourse,
    };
    use sha2::Digest;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    const COMPLETE_PREDECESSOR_PROBE: &str = "GUIDE_WATCHER_COMPLETE_PREDECESSOR_PROBE";
    const COMPLETE_PREDECESSOR_PATH: &str = "GUIDE_WATCHER_COMPLETE_PREDECESSOR_PATH";

    struct EmptyRenderer;

    impl source_context::LocalSourceRenderer for EmptyRenderer {
        fn render(
            &self,
            _source: &source_context::CapturedSource,
            _cancellation: process_registry::CancellationToken,
            _limits: source_context::SourceRenderLimits,
            _renderer_script_bytes: &[u8],
        ) -> Result<Vec<source_context::RenderedSourceImage>, String> {
            Ok(Vec::new())
        }
    }

    struct FailingRenderer {
        fail_at: usize,
        calls: AtomicUsize,
    }

    struct BlockingRenderer {
        entered: std::sync::mpsc::Sender<()>,
    }

    struct FailingRuntimeProbe(&'static str);

    struct PassingRuntimeProbe;

    struct StaticPngRenderer {
        bytes: Vec<u8>,
        calls: AtomicUsize,
    }

    impl source_context::VerifierRuntimeProbe for FailingRuntimeProbe {
        fn verify(
            &self,
            _dependency_lock: &str,
            _cancellation: process_registry::CancellationToken,
        ) -> Result<(), String> {
            Err(self.0.to_string())
        }
    }

    impl source_context::VerifierRuntimeProbe for PassingRuntimeProbe {
        fn verify(
            &self,
            _dependency_lock: &str,
            _cancellation: process_registry::CancellationToken,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    impl source_context::LocalSourceRenderer for StaticPngRenderer {
        fn render(
            &self,
            _source: &source_context::CapturedSource,
            _cancellation: process_registry::CancellationToken,
            _limits: source_context::SourceRenderLimits,
            _renderer_script_bytes: &[u8],
        ) -> Result<Vec<source_context::RenderedSourceImage>, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![source_context::RenderedSourceImage {
                filename: "slide_1.png".to_string(),
                number: 1,
                sha256: format!("{:x}", sha2::Sha256::digest(&self.bytes)),
                bytes: self.bytes.clone(),
                width_px: 1,
                height_px: 1,
            }])
        }
    }

    fn tiny_png() -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[1, 2, 3, 255]).unwrap();
        }
        bytes
    }

    fn unique_runtime_dependency_bytes() -> u64 {
        let mut paths = std::collections::HashSet::new();
        [
            crate::config::TEMPLATE_FILE,
            crate::config::DEPTH_CONTRACT_FILE,
            crate::config::RENDER_SLIDES_SCRIPT,
            crate::config::GUIDE_LINT_SCRIPT,
            crate::config::GUIDE_LINT_REQUIREMENTS,
        ]
        .into_iter()
        .map(PathBuf::from)
        .filter(|path| paths.insert(path.clone()))
        .map(|path| std::fs::metadata(path).unwrap().len())
        .sum()
    }

    impl source_context::LocalSourceRenderer for BlockingRenderer {
        fn render(
            &self,
            _source: &source_context::CapturedSource,
            cancellation: process_registry::CancellationToken,
            _limits: source_context::SourceRenderLimits,
            _renderer_script_bytes: &[u8],
        ) -> Result<Vec<source_context::RenderedSourceImage>, String> {
            let _ = self.entered.send(());
            while !process_registry::is_cancelled(cancellation) {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err("injected slow preflight observed cancellation".to_string())
        }
    }

    impl source_context::LocalSourceRenderer for FailingRenderer {
        fn render(
            &self,
            source: &source_context::CapturedSource,
            _cancellation: process_registry::CancellationToken,
            _limits: source_context::SourceRenderLimits,
            _renderer_script_bytes: &[u8],
        ) -> Result<Vec<source_context::RenderedSourceImage>, String> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if call == self.fail_at {
                Err(format!(
                    "injected local renderer dependency failure for {}",
                    source.path.display()
                ))
            } else {
                Ok(Vec::new())
            }
        }
    }

    fn plan(course: &str, sequence: &str) -> PlannedGuide {
        PlannedGuide {
            job_id: format!("{course}-{sequence}"),
            source_paths: vec![PathBuf::from(format!("{course}-{sequence}.pdf"))],
            primary_source: PathBuf::from(format!("{course}-{sequence}.pdf")),
            output_path: PathBuf::from(format!("{course}-{sequence}.md")),
            course: ResolvedCourse {
                id: course.to_string(),
                label: course.to_string(),
                root: PathBuf::from(course),
                guide_mode: GuideMode::LectureDeck,
                lecture_primary_rule: LecturePrimaryRule::Any,
                expected_guide_kind: GuideKind::Lecture,
                profile_order: 0,
                pinned_baseline: None,
            },
            sequence_key: sequence.to_string(),
            generation_identity: format!("{course}:{sequence}"),
            predecessors: Vec::new(),
        }
    }

    fn test_course(root: &Path) -> ResolvedCourse {
        ResolvedCourse {
            id: "preflight-course".to_string(),
            label: "Preflight course".to_string(),
            root: root.canonicalize().unwrap(),
            guide_mode: GuideMode::LectureDeck,
            lecture_primary_rule: LecturePrimaryRule::Any,
            expected_guide_kind: GuideKind::Lecture,
            profile_order: 0,
            pinned_baseline: None,
        }
    }

    #[tokio::test]
    async fn failure_blocks_only_later_jobs_in_the_same_course() {
        let attempted = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = attempted.clone();
        let results = execute_course_chains(
            vec![
                plan("networks", "1"),
                plan("networks", "2"),
                plan("os", "1"),
            ],
            move |plan| {
                let seen = seen.clone();
                async move {
                    seen.lock().unwrap().push(plan.job_id.clone());
                    plan.job_id != "networks-1"
                }
            },
            |_| {},
        )
        .await;

        assert_eq!(
            results
                .iter()
                .map(|result| result.status)
                .collect::<Vec<_>>(),
            vec![
                BatchJobStatus::Failed,
                BatchJobStatus::BlockedByPredecessor,
                BatchJobStatus::Succeeded
            ]
        );
        assert_eq!(
            *attempted.lock().unwrap(),
            vec!["networks-1".to_string(), "os-1".to_string()]
        );
    }

    #[tokio::test]
    async fn different_courses_overlap_but_each_course_remains_ordered() {
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let active_courses = Arc::new(std::sync::Mutex::new(HashSet::new()));
        let same_course_overlap = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let active_for_run = active.clone();
        let maximum_for_run = maximum.clone();
        let active_courses_for_run = active_courses.clone();
        let same_course_overlap_for_run = same_course_overlap.clone();
        let results = execute_course_chains(
            vec![plan("networks", "1"), plan("os", "1"), plan("os", "2")],
            move |plan| {
                let active = active_for_run.clone();
                let maximum = maximum_for_run.clone();
                let active_courses = active_courses_for_run.clone();
                let same_course_overlap = same_course_overlap_for_run.clone();
                async move {
                    let course = plan.course.id.clone();
                    if !active_courses.lock().unwrap().insert(course.clone()) {
                        same_course_overlap.store(true, Ordering::SeqCst);
                    }
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    active_courses.lock().unwrap().remove(&course);
                    true
                }
            },
            |_| {},
        )
        .await;
        assert!(results
            .iter()
            .all(|result| result.status == BatchJobStatus::Succeeded));
        assert_eq!(maximum.load(Ordering::SeqCst), 2);
        assert!(!same_course_overlap.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn independent_batches_are_not_globally_serialized() {
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let run = |active: Arc<AtomicUsize>, maximum: Arc<AtomicUsize>| async move {
            let now = active.fetch_add(1, Ordering::SeqCst) + 1;
            maximum.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            active.fetch_sub(1, Ordering::SeqCst);
        };
        let ((), ()) = tokio::join!(
            run(active.clone(), maximum.clone()),
            run(active.clone(), maximum.clone())
        );
        assert_eq!(maximum.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn visual_preparation_is_bounded_concurrent_and_preserves_input_order() {
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let active_for_run = active.clone();
        let maximum_for_run = maximum.clone();
        let outputs = map_bounded_ordered((0..9).collect(), 3, move |value| {
            let active = active_for_run.clone();
            let maximum = maximum_for_run.clone();
            async move {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                maximum.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(10)).await;
                active.fetch_sub(1, Ordering::SeqCst);
                value
            }
        })
        .await;

        assert_eq!(outputs, (0..9).collect::<Vec<_>>());
        assert_eq!(maximum.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn late_renderer_failure_prevents_provider_start_and_releases_all_reservations() {
        let root = std::env::temp_dir().join(format!(
            "guide-watcher-batch-preflight-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("Lecture 1.html"), "<h1>Valid first source</h1>").unwrap();
        std::fs::write(root.join("Lecture 2.html"), "<h1>Valid second source</h1>").unwrap();
        let sources = vec![root.join("Lecture 1.html"), root.join("Lecture 2.html")];
        let course = test_course(&root);
        let renderer = FailingRenderer {
            fail_at: 2,
            calls: AtomicUsize::new(0),
        };
        let provider_calls = AtomicUsize::new(0);
        let cancellation = process_registry::cancellation_token();
        let result = preflight_paths_with_courses_and_renderer(
            &sources,
            "auto",
            std::slice::from_ref(&course),
            &renderer,
            cancellation,
            || {},
        );
        process_registry::finish(cancellation);
        if result.is_ok() {
            provider_calls.fetch_add(1, Ordering::SeqCst);
        }
        let error = match result {
            Ok(_) => panic!("late local-render failure must reject the whole batch"),
            Err(error) => error,
        };
        assert!(error.contains("renderer dependency failure"), "{error}");
        assert_eq!(renderer.calls.load(Ordering::SeqCst), 2);
        assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
        let first_output = crate::job_events::derive_output_paths(&sources[0])
            .unwrap()
            .2;
        assert!(!first_output.exists());
        assert!(!root.join("Lecture_1_Guide.prep.md").exists());
        assert!(!root.join("Lecture_1_Guide_assets").exists());
        assert!(!PathBuf::from(format!("{}.gwverify", first_output.display())).exists());

        let _output = reserve_output_path(&first_output).expect("output reservation was released");
        let _course = reserve_course(&course.id, &course.root)
            .expect("course reservation was released after failed preflight");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancellation_during_slow_preflight_returns_promptly_and_starts_no_provider() {
        let root = std::env::temp_dir().join(format!(
            "guide-watcher-cancel-preflight-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("Lecture 1.html");
        std::fs::write(&source, "<h1>Slow preflight</h1>").unwrap();
        let course = test_course(&root);
        let cancellation = process_registry::cancellation_token();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let worker_source = source.clone();
        let worker = std::thread::spawn(move || {
            let renderer = BlockingRenderer {
                entered: entered_tx,
            };
            preflight_paths_with_courses_and_renderer(
                &[worker_source],
                "auto",
                std::slice::from_ref(&course),
                &renderer,
                cancellation,
                || {},
            )
        });
        entered_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("slow renderer entered before cancellation");
        let started = std::time::Instant::now();
        process_registry::cancel(cancellation);
        let result = worker.join().expect("preflight worker did not panic");
        assert!(started.elapsed() < Duration::from_secs(2));
        let error = match result {
            Ok(_) => panic!("cancelled preflight must not produce a prepared batch"),
            Err(error) => error,
        };
        assert!(
            error.contains("injected slow preflight observed cancellation"),
            "{error}"
        );
        process_registry::finish(cancellation);
        let output = crate::job_events::derive_output_paths(&source).unwrap().2;
        assert!(!output.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cumulative_source_and_render_budgets_fail_before_any_provider_or_output() {
        for rendered_case in [false, true] {
            let root = std::env::temp_dir().join(format!(
                "guide-watcher-incremental-budget-test-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir(&root).unwrap();
            let first = root.join("Lecture 1.html");
            let second = root.join("Lecture 2.html");
            std::fs::write(&first, "<h1>First bounded source</h1>").unwrap();
            std::fs::write(&second, "<h1>Second bounded source</h1>").unwrap();
            let course = test_course(&root);
            let plans = crate::course_plan::plan_guides_with_courses(
                &[first.clone(), second.clone()],
                "auto",
                std::slice::from_ref(&course),
            )
            .unwrap();
            for plan in &plans {
                crate::visual_assets::write_no_visual_packet_for_test(plan).unwrap();
            }
            let first_packet = crate::visual_assets::packet_path(&plans[0].primary_source).unwrap();
            let first_cost = unique_runtime_dependency_bytes()
                + std::fs::metadata(&first).unwrap().len()
                + std::fs::metadata(&first_packet).unwrap().len();
            let second_source = std::fs::metadata(&second).unwrap().len();
            let png = tiny_png();
            let renderer = StaticPngRenderer {
                bytes: png.clone(),
                calls: AtomicUsize::new(0),
            };
            let mut budget = if rendered_case {
                source_context::PreflightResourceBudget::with_limits(
                    source_context::MAX_BATCH_SOURCE_BYTES,
                    png.len() as u64 * 2 - 1,
                    source_context::MAX_BATCH_RENDER_PIXEL_WORK,
                )
            } else {
                source_context::PreflightResourceBudget::with_limits(
                    first_cost + second_source - 1,
                    source_context::MAX_BATCH_RENDERED_BYTES,
                    source_context::MAX_BATCH_RENDER_PIXEL_WORK,
                )
            };
            let reservations = acquire_course_reservations(std::slice::from_ref(&course)).unwrap();
            let cancellation = process_registry::cancellation_token();
            let result = preflight_locked_plans_with_runtime_probe_and_budget(
                plans,
                reservations,
                &renderer,
                &PassingRuntimeProbe,
                &mut budget,
                cancellation,
            );
            process_registry::finish(cancellation);
            let error = match result {
                Ok(_) => panic!("cumulative budget exhaustion must reject the whole batch"),
                Err(error) => error,
            };
            if rendered_case {
                assert!(error.contains("rendered-byte budget"), "{error}");
                assert_eq!(renderer.calls.load(Ordering::SeqCst), 2);
            } else {
                assert!(error.contains("source/context byte budget"), "{error}");
                assert_eq!(renderer.calls.load(Ordering::SeqCst), 1);
            }
            for source in [&first, &second] {
                let output = crate::job_events::derive_output_paths(source).unwrap().2;
                assert!(!output.exists());
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn runtime_and_textbook_bytes_are_charged_before_read_or_provider() {
        for textbook_case in [false, true] {
            let root = std::env::temp_dir().join(format!(
                "guide-watcher-runtime-textbook-budget-test-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir(&root).unwrap();
            let source = root.join("Lecture 1.html");
            std::fs::write(&source, "<h1>Budget source</h1>").unwrap();
            let course = test_course(&root);
            let plans = crate::course_plan::plan_guides_with_courses(
                std::slice::from_ref(&source),
                "auto",
                std::slice::from_ref(&course),
            )
            .unwrap();
            crate::visual_assets::write_no_visual_packet_for_test(&plans[0]).unwrap();
            let source_limit = if textbook_case {
                let textbook = root.join("Course Textbook.pdf");
                std::fs::write(&textbook, vec![b'x'; 4096]).unwrap();
                unique_runtime_dependency_bytes()
                    + std::fs::metadata(&source).unwrap().len()
                    + std::fs::metadata(&textbook).unwrap().len()
                    - 1
            } else {
                std::fs::metadata(crate::config::TEMPLATE_FILE)
                    .unwrap()
                    .len()
                    - 1
            };
            let mut budget = source_context::PreflightResourceBudget::with_limits(
                source_limit,
                source_context::MAX_BATCH_RENDERED_BYTES,
                source_context::MAX_BATCH_RENDER_PIXEL_WORK,
            );
            let reservations = acquire_course_reservations(std::slice::from_ref(&course)).unwrap();
            let cancellation = process_registry::cancellation_token();
            let result = preflight_locked_plans_with_runtime_probe_and_budget(
                plans,
                reservations,
                &EmptyRenderer,
                &PassingRuntimeProbe,
                &mut budget,
                cancellation,
            );
            process_registry::finish(cancellation);
            let error = match result {
                Ok(_) => panic!("oversized dependency must fail before providers"),
                Err(error) => error,
            };
            assert!(error.contains("source/context byte budget"), "{error}");
            let output = crate::job_events::derive_output_paths(&source).unwrap().2;
            assert!(!output.exists());
            assert!(!root.join("Lecture_1_Guide.prep.md").exists());
            assert!(!root.join("Lecture_1_Guide_assets").exists());
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn decoded_visual_budget_is_shared_across_the_whole_batch() {
        let root = std::env::temp_dir().join(format!(
            "guide-watcher-batch-decoded-budget-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&root).unwrap();
        let first = root.join("Lecture 1.html");
        let second = root.join("Lecture 2.html");
        std::fs::write(&first, "<h1>First visual source</h1>").unwrap();
        std::fs::write(&second, "<h1>Second visual source</h1>").unwrap();
        let course = test_course(&root);
        let plans = crate::course_plan::plan_guides_with_courses(
            &[first.clone(), second.clone()],
            "auto",
            std::slice::from_ref(&course),
        )
        .unwrap();
        for plan in &plans {
            crate::visual_assets::write_source_crop_packet_for_test(plan).unwrap();
        }
        let renderer = StaticPngRenderer {
            bytes: tiny_png(),
            calls: AtomicUsize::new(0),
        };
        let mut budget = source_context::PreflightResourceBudget::new();
        budget.set_visual_decoded_limit(7);
        let reservations = acquire_course_reservations(std::slice::from_ref(&course)).unwrap();
        let cancellation = process_registry::cancellation_token();
        let result = preflight_locked_plans_with_runtime_probe_and_budget(
            plans,
            reservations,
            &renderer,
            &PassingRuntimeProbe,
            &mut budget,
            cancellation,
        );
        process_registry::finish(cancellation);
        let error = match result {
            Ok(_) => panic!("second guide must not receive a fresh decoded-input budget"),
            Err(error) => error,
        };
        assert!(error.contains("whole-batch decoded-RGBA budget"), "{error}");
        assert_eq!(renderer.calls.load(Ordering::SeqCst), 2);
        for source in [&first, &second] {
            let output = crate::job_events::derive_output_paths(source).unwrap().2;
            assert!(!output.exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn verifier_dependency_failure_blocks_single_batch_and_resume_before_provider() {
        for failure in [
            "markdown-it-py is not installed",
            "markdown-it-py version mismatch: expected 4.2.0, found 4.1.0",
            "markdown_it import failed: no module named markdown_it",
            "markdown-it-py CommonMark smoke parse did not produce the required image token",
        ] {
            let root = std::env::temp_dir().join(format!(
                "guide-watcher-verifier-runtime-preflight-test-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir(&root).unwrap();
            let first = root.join("Lecture 1.html");
            let second = root.join("Lecture 2.html");
            std::fs::write(&first, "<h1>First source</h1>").unwrap();
            std::fs::write(&second, "<h1>Second source</h1>").unwrap();
            let course = test_course(&root);
            let provider_calls = AtomicUsize::new(0);
            let probe = FailingRuntimeProbe(failure);

            for selected in [vec![first.clone()], vec![first.clone(), second.clone()]] {
                let plans = crate::course_plan::plan_guides_with_courses(
                    &selected,
                    "auto",
                    std::slice::from_ref(&course),
                )
                .unwrap();
                let reservations =
                    acquire_course_reservations(std::slice::from_ref(&course)).unwrap();
                let cancellation = process_registry::cancellation_token();
                let result = preflight_locked_plans_with_runtime_probe(
                    plans,
                    reservations,
                    &EmptyRenderer,
                    &probe,
                    cancellation,
                );
                process_registry::finish(cancellation);
                if result.is_ok() {
                    provider_calls.fetch_add(1, Ordering::SeqCst);
                }
                let error = match result {
                    Ok(_) => panic!("verifier dependency failure must reject generation"),
                    Err(error) => error,
                };
                assert!(error.contains(failure), "{error}");
            }

            let resume_plan = crate::course_plan::plan_guides_with_courses(
                std::slice::from_ref(&first),
                "auto",
                std::slice::from_ref(&course),
            )
            .unwrap()
            .remove(0);
            let prep_path = root.join("Lecture_1_Guide.prep.md");
            let prep_bytes = b"validated prep packet";
            std::fs::write(&prep_path, prep_bytes).unwrap();
            let packet = ValidatedPrepPacket {
                path: prep_path,
                text: String::from_utf8(prep_bytes.to_vec()).unwrap(),
                plan: resume_plan,
                sha256: format!("{:x}", sha2::Sha256::digest(prep_bytes)),
                size_bytes: prep_bytes.len() as u64,
                predecessor_drift: Vec::new(),
            };
            let course_reservation = reserve_course(&course.id, &course.root).unwrap();
            let cancellation = process_registry::cancellation_token();
            let result = preflight_locked_resume_with_runtime_probe(
                packet,
                course_reservation,
                &EmptyRenderer,
                &probe,
                cancellation,
            );
            process_registry::finish(cancellation);
            if result.is_ok() {
                provider_calls.fetch_add(1, Ordering::SeqCst);
            }
            let error = match result {
                Ok(_) => panic!("verifier dependency failure must reject resume"),
                Err(error) => error,
            };
            assert!(error.contains(failure), "{error}");
            assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn later_visual_packet_failure_starts_no_provider_and_preserves_week_one() {
        let root = std::env::temp_dir().join(format!(
            "guide-watcher-batch-visual-preflight-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&root).unwrap();
        let week_one = root.join("Week1_Guide.md");
        let week_one_bytes = b"# Existing verified Week 1\n";
        std::fs::write(&week_one, week_one_bytes).unwrap();
        let first = root.join("Lecture 2.html");
        let second = root.join("Lecture 3.html");
        std::fs::write(&first, "<h1>Valid first source</h1>").unwrap();
        std::fs::write(&second, "<h1>Later source with bad visuals</h1>").unwrap();
        let course = test_course(&root);
        let plans = crate::course_plan::plan_guides_with_courses(
            &[first.clone(), second.clone()],
            "auto",
            std::slice::from_ref(&course),
        )
        .unwrap();
        for plan in &plans {
            crate::visual_assets::write_no_visual_packet_for_test(plan).unwrap();
        }
        let second_packet = crate::visual_assets::packet_path(&plans[1].primary_source).unwrap();
        let mut malformed: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&second_packet).unwrap()).unwrap();
        malformed["unexpected_live_fetch"] = serde_json::json!("https://example.invalid/image.png");
        std::fs::write(&second_packet, serde_json::to_vec(&malformed).unwrap()).unwrap();
        let provider_calls = AtomicUsize::new(0);
        let reservations = acquire_course_reservations(std::slice::from_ref(&course)).unwrap();
        let cancellation = process_registry::cancellation_token();
        let result = preflight_locked_plans(plans, reservations, &EmptyRenderer, cancellation);
        process_registry::finish(cancellation);
        if result.is_ok() {
            provider_calls.fetch_add(1, Ordering::SeqCst);
        }
        let error = match result {
            Ok(_) => panic!("later malformed visual packet must reject the entire batch"),
            Err(error) => error,
        };
        assert!(error.contains("unknown field"), "{error}");
        assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
        assert_eq!(std::fs::read(&week_one).unwrap(), week_one_bytes);
        for source in [&first, &second] {
            let output = crate::job_events::derive_output_paths(source).unwrap().2;
            assert!(!output.exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cross_process_complete_predecessor_probe() {
        if std::env::var_os(COMPLETE_PREDECESSOR_PROBE).is_none() {
            return;
        }
        let path = PathBuf::from(std::env::var_os(COMPLETE_PREDECESSOR_PATH).unwrap());
        std::fs::write(&path, "# Completed by the other process\n").unwrap();
        crate::completion::append_receipt(&path).unwrap();
    }

    #[test]
    fn planning_is_repeated_under_course_lock_after_an_earlier_guide_completes() {
        let root = std::env::temp_dir().join(format!(
            "guide-watcher-plan-lock-race-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&root).unwrap();
        let earlier = root.join("Lecture 1.html");
        let selected = root.join("Lecture 2.html");
        std::fs::write(&earlier, "<h1>Earlier</h1>").unwrap();
        std::fs::write(&selected, "<h1>Selected</h1>").unwrap();
        let course = test_course(&root);
        let initial = crate::course_plan::plan_guides_with_courses(
            std::slice::from_ref(&selected),
            "auto",
            std::slice::from_ref(&course),
        )
        .unwrap();
        assert!(initial[0].predecessors.is_empty());
        let earlier_guide = crate::job_events::derive_output_paths(&earlier).unwrap().2;

        let cancellation = process_registry::cancellation_token();
        let prepared = preflight_paths_with_courses_and_renderer(
            std::slice::from_ref(&selected),
            "auto",
            std::slice::from_ref(&course),
            &EmptyRenderer,
            cancellation,
            || {
                let status = std::process::Command::new(std::env::current_exe().unwrap())
                    .arg("--exact")
                    .arg("batch::tests::cross_process_complete_predecessor_probe")
                    .arg("--nocapture")
                    .env(COMPLETE_PREDECESSOR_PROBE, "1")
                    .env(COMPLETE_PREDECESSOR_PATH, &earlier_guide)
                    .status()
                    .unwrap();
                assert!(status.success());
            },
        )
        .expect("planning under the acquired lock must see the newly completed guide");
        process_registry::finish(cancellation);
        assert_eq!(prepared.guides.len(), 1);
        assert_eq!(prepared.guides[0].plan.predecessors.len(), 1);
        assert_eq!(
            prepared.guides[0].plan.predecessors[0]
                .path
                .canonicalize()
                .unwrap(),
            earlier_guide.canonicalize().unwrap()
        );
        drop(prepared);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn in_batch_predecessors_must_be_earlier_and_match_exact_identity() {
        let mut first = plan("networks", "1");
        let second = plan("networks", "2");
        first.predecessors.push(PlannedPredecessor {
            course_profile: second.course.id.clone(),
            generation_identity: second.generation_identity.clone(),
            sequence_key: second.sequence_key.clone(),
            path: second.output_path.clone(),
            pinned: false,
        });
        let future_error = validate_in_batch_predecessors(&[first, second.clone()]).unwrap_err();
        assert!(future_error.contains("not earlier"), "{future_error}");

        let first = plan("networks", "1");
        let mut second = second;
        second.predecessors.push(PlannedPredecessor {
            course_profile: "operating-systems".to_string(),
            generation_identity: first.generation_identity.clone(),
            sequence_key: first.sequence_key.clone(),
            path: first.output_path.clone(),
            pinned: false,
        });
        let identity_error = validate_in_batch_predecessors(&[first, second]).unwrap_err();
        assert!(
            identity_error.contains("identity does not match"),
            "{identity_error}"
        );
    }
}
