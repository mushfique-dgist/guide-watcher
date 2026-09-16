use crate::{completion, config};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;


#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GuideKind {
    Lecture,
    CircuitLab,
    ScientificWritingWeek,
    CourseWeek,
}

impl GuideKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lecture => "lecture",
            Self::CircuitLab => "circuit-lab",
            Self::ScientificWritingWeek => "scientific-writing-week",
            Self::CourseWeek => "course-week",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuideMode {
    LectureDeck,
    WeeklyLab,
    WeeklyMaterial,
    LegacyAuto,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LecturePrimaryRule {
    /// Every supported file in the folder can be the primary source of a guide.
    Any,
    /// Only files whose name matches this case-insensitive regular expression, which the user
    /// writes in the settings file. This is how a course says "my lectures are CH01.pdf,
    /// CH02.pdf" without the app having to know anything about the subject.
    Named(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PinnedBaseline {
    pub path: PathBuf,
    pub sha256: String,
    pub sequence_key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedCourse {
    pub id: String,
    pub label: String,
    pub root: PathBuf,
    pub guide_mode: GuideMode,
    pub lecture_primary_rule: LecturePrimaryRule,
    pub expected_guide_kind: GuideKind,
    pub profile_order: u8,
    /// Guides this subject keeps rather than regenerates, in sequence order.
    pub pinned_guides: Vec<PinnedBaseline>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedGuide {
    pub job_id: String,
    pub source_paths: Vec<PathBuf>,
    pub primary_source: PathBuf,
    pub output_path: PathBuf,
    pub course: ResolvedCourse,
    pub sequence_key: String,
    pub generation_identity: String,
    pub predecessors: Vec<PlannedPredecessor>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedPredecessor {
    pub course_profile: String,
    pub generation_identity: String,
    pub sequence_key: String,
    pub path: PathBuf,
    pub pinned: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedJobDto {
    pub job_id: String,
    pub source_paths: Vec<String>,
    pub primary_source: String,
    pub output_path: String,
    pub course_profile: String,
    pub course_label: String,
    pub expected_guide_kind: GuideKind,
    pub generation_identity: String,
}

impl TryFrom<&PlannedGuide> for PlannedJobDto {
    type Error = String;

    fn try_from(plan: &PlannedGuide) -> Result<Self, Self::Error> {
        Ok(Self {
            job_id: plan.job_id.clone(),
            source_paths: plan
                .source_paths
                .iter()
                .map(|path| path_text(path))
                .collect::<Result<Vec<_>, _>>()?,
            primary_source: path_text(&plan.primary_source)?,
            output_path: path_text(&plan.output_path)?,
            course_profile: plan.course.id.clone(),
            course_label: plan.course.label.clone(),
            expected_guide_kind: plan.course.expected_guide_kind,
            generation_identity: plan.generation_identity.clone(),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BoundFile {
    pub path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BoundPredecessor {
    pub course_profile: String,
    pub generation_identity: String,
    pub sequence_key: String,
    pub path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BoundGenerationContract {
    pub schema_version: u8,
    pub course_profile: String,
    pub expected_guide_kind: GuideKind,
    pub generation_identity: String,
    pub primary_source: String,
    pub output_path: String,
    pub sources: Vec<BoundFile>,
    pub predecessors: Vec<BoundPredecessor>,
}

pub fn configured_courses() -> Result<Vec<ResolvedCourse>, String> {
    let configured = config::settings();
    let specifications = configured.courses.iter().map(|course| {
        (
            course.id.clone(),
            course.label.clone(),
            configured.course_root(course),
            match course.mode.as_str() {
                "weekly-lab" => GuideMode::WeeklyLab,
                "weekly-material" => GuideMode::WeeklyMaterial,
                "legacy-auto" => GuideMode::LegacyAuto,
                _ => GuideMode::LectureDeck,
            },
            match course.kind.as_str() {
                "circuit-lab" => GuideKind::CircuitLab,
                "scientific-writing-week" => GuideKind::ScientificWritingWeek,
                "course-week" => GuideKind::CourseWeek,
                _ => GuideKind::Lecture,
            },
            if course.lecture_files.trim().is_empty() {
                LecturePrimaryRule::Any
            } else {
                LecturePrimaryRule::Named(course.lecture_files.clone())
            },
            course
                .pinned_guides
                .iter()
                .map(|pinned| PinnedBaseline {
                    path: configured.pinned_guide_path(course, pinned),
                    sha256: pinned.sha256.clone(),
                    sequence_key: pinned.sequence_key.clone(),
                })
                .collect::<Vec<_>>(),
        )
    });

    specifications
        .enumerate()
        .map(
            |(order, (id, label, root, mode, kind, lecture_primary_rule, pinned))| {
                let order = order as u8;
                let root = canonical_directory(Path::new(&root), "course root")?;
                let pinned_guides = pinned;
                Ok(ResolvedCourse {
                    id: id.clone(),
                    label: label.clone(),
                    root,
                    guide_mode: mode,
                    lecture_primary_rule,
                    expected_guide_kind: kind,
                    profile_order: order,
                    pinned_guides,
                })
            },
        )
        .collect()
}

/// Every guide this subject keeps rather than regenerates, as configured for this installation.
pub(crate) fn preserved_baselines(course: &ResolvedCourse) -> Vec<PinnedBaseline> {
    course.pinned_guides.clone()
}

fn is_preserved_week(course: &ResolvedCourse, week: u32) -> bool {
    let sequence_key = format!("week-{week:02}");
    preserved_baselines(course)
        .iter()
        .any(|baseline| baseline.sequence_key == sequence_key)
}

pub fn plan_guides(
    paths: &[PathBuf],
    requested_profile: &str,
) -> Result<Vec<PlannedGuide>, String> {
    plan_guides_with_courses(paths, requested_profile, &configured_courses()?)
}

pub(crate) fn configured_course(course_profile: &str) -> Result<ResolvedCourse, String> {
    configured_courses()?
        .into_iter()
        .find(|course| course.id == course_profile)
        .ok_or_else(|| format!("unknown course profile: {course_profile}"))
}

pub(crate) fn resolve_course_targets(
    paths: &[PathBuf],
    requested_profile: &str,
) -> Result<(Vec<PathBuf>, Vec<ResolvedCourse>), String> {
    resolve_course_targets_with_courses(paths, requested_profile, &configured_courses()?)
}

pub(crate) fn resolve_course_targets_with_courses(
    paths: &[PathBuf],
    requested_profile: &str,
    courses: &[ResolvedCourse],
) -> Result<(Vec<PathBuf>, Vec<ResolvedCourse>), String> {
    if paths.is_empty() {
        return Err("at least one source file is required".to_string());
    }
    let explicit = if requested_profile == "auto" {
        None
    } else {
        Some(
            courses
                .iter()
                .find(|course| course.id == requested_profile)
                .ok_or_else(|| format!("unknown course profile: {requested_profile}"))?,
        )
    };
    let mut canonical_paths = Vec::with_capacity(paths.len());
    let mut seen_paths = HashSet::new();
    let mut targets = HashMap::new();
    for path in paths {
        let canonical = canonical_file(path, "course source")?;
        let text = path_text(&canonical)?;
        if !config::is_watched_extension(&text) || config::should_skip(&text) {
            return Err(format!(
                "source is unsupported or reserved as supporting material: {}",
                canonical.display()
            ));
        }
        if !seen_paths.insert(path_identity(&canonical)?) {
            return Err(format!(
                "the same source was selected more than once: {}",
                canonical.display()
            ));
        }
        let course = match explicit {
            Some(course) => {
                if !path_is_within(&canonical, &course.root) {
                    return Err(format!(
                        "course profile '{}' does not contain selected source: {}",
                        course.id,
                        canonical.display()
                    ));
                }
                course.clone()
            }
            None => resolve_auto(&canonical, courses)?,
        };
        targets.entry(course.id.clone()).or_insert(course);
        canonical_paths.push(canonical);
    }
    let mut targets = targets.into_values().collect::<Vec<_>>();
    targets.sort_by(|left, right| {
        path_identity(&left.root)
            .unwrap_or_default()
            .cmp(&path_identity(&right.root).unwrap_or_default())
            .then(left.id.cmp(&right.id))
    });
    Ok((canonical_paths, targets))
}

fn is_scannable_source_with_courses(path: &Path, courses: &[ResolvedCourse]) -> bool {
    let Ok(path) = canonical_file(path, "course source") else {
        return false;
    };
    let Ok(course) = resolve_auto(&path, courses) else {
        return false;
    };
    match course.guide_mode {
        GuideMode::LectureDeck => is_allowed_lecture_primary(&path, &course.lecture_primary_rule),
        GuideMode::LegacyAuto => true,
        GuideMode::WeeklyLab | GuideMode::WeeklyMaterial => source_week(&path, course.guide_mode)
            .ok()
            .flatten()
            .is_some_and(|week| !is_preserved_week(&course, week)),
    }
}

/// Build one chronological dependency wave for the normal workspace scan.
///
/// A wave contains at most the earliest unfinished guide from each course. This
/// keeps cumulative predecessors ordered, prevents a valid completed guide from
/// being offered again, and bounds the normal scan independently of the total
/// number of semester files. Supporting files that `plan_week` discovers on its
/// own are intentionally not returned; they will be rediscovered when the user
/// approves the primary/week inputs in this wave.
pub(crate) fn next_pending_scan_wave_with_courses(
    paths: &[PathBuf],
    courses: &[ResolvedCourse],
) -> Result<Vec<PathBuf>, String> {
    let eligible = paths
        .iter()
        .filter(|path| is_scannable_source_with_courses(path, courses))
        .cloned()
        .collect::<Vec<_>>();
    if eligible.is_empty() {
        return Ok(Vec::new());
    }
    let eligible_identities = eligible
        .iter()
        .map(|path| canonical_file(path, "course source").and_then(|path| path_identity(&path)))
        .collect::<Result<HashSet<_>, _>>()?;
    let plans = plan_guides_with_courses(&eligible, "auto", courses)?;
    let mut selected_courses = HashSet::new();
    let mut selected = Vec::new();
    for plan in plans {
        if completion::is_usable_guide(&plan.output_path)
            || !selected_courses.insert(plan.course.id.clone())
        {
            continue;
        }
        for source in plan.source_paths {
            let identity = path_identity(&source)?;
            if eligible_identities.contains(&identity) {
                selected.push(source);
            }
        }
    }
    Ok(selected)
}

pub(crate) fn plan_guides_with_courses(
    paths: &[PathBuf],
    requested_profile: &str,
    courses: &[ResolvedCourse],
) -> Result<Vec<PlannedGuide>, String> {
    if paths.is_empty() {
        return Err("at least one source file is required".to_string());
    }
    let mut canonical_paths = Vec::with_capacity(paths.len());
    let mut seen = HashSet::new();
    for path in paths {
        let canonical = canonical_file(path, "course source")?;
        let text = path_text(&canonical)?;
        if !config::is_watched_extension(&text) || config::should_skip(&text) {
            return Err(format!(
                "source is unsupported or reserved as supporting material: {}",
                canonical.display()
            ));
        }
        let identity = path_identity(&canonical)?;
        if !seen.insert(identity) {
            return Err(format!(
                "the same source was selected more than once: {}",
                canonical.display()
            ));
        }
        canonical_paths.push(canonical);
    }

    let explicit = if requested_profile == "auto" {
        None
    } else {
        Some(
            courses
                .iter()
                .find(|course| course.id == requested_profile)
                .ok_or_else(|| format!("unknown course profile: {requested_profile}"))?,
        )
    };
    if let Some(course) = explicit {
        for path in &canonical_paths {
            if !path_is_within(path, &course.root) {
                return Err(format!(
                    "course profile '{}' does not contain selected source: {}",
                    course.id,
                    path.display()
                ));
            }
        }
    }

    let mut by_course: HashMap<String, (ResolvedCourse, Vec<PathBuf>)> = HashMap::new();
    for path in canonical_paths {
        let course = match explicit {
            Some(course) => course.clone(),
            None => resolve_auto(&path, courses)?,
        };
        by_course
            .entry(course.id.clone())
            .or_insert_with(|| (course, Vec::new()))
            .1
            .push(path);
    }

    let mut planned = Vec::new();
    for (_, (course, sources)) in by_course {
        planned.extend(plan_course_sources(course, sources)?);
    }
    planned.sort_by(|left, right| {
        left.course
            .profile_order
            .cmp(&right.course.profile_order)
            .then_with(|| natural_cmp(&left.sequence_key, &right.sequence_key))
    });
    let mut output_paths = HashSet::new();
    let mut generation_identities = HashSet::new();
    for plan in &planned {
        if !output_paths.insert(path_identity(&plan.output_path)?) {
            return Err(format!(
                "more than one selected source resolves to the same guide output: {}",
                plan.output_path.display()
            ));
        }
        if !generation_identities.insert(plan.generation_identity.clone()) {
            return Err(format!(
                "more than one selected source resolves to generation identity '{}'",
                plan.generation_identity
            ));
        }
    }

    let mut prior_in_batch: HashMap<String, Vec<PlannedPredecessor>> = HashMap::new();
    for plan in &mut planned {
        let mut predecessors = historical_predecessors(plan)?;
        if let Some(earlier) = prior_in_batch.get(&plan.course.id) {
            predecessors.extend(earlier.iter().cloned());
        }
        sort_and_deduplicate_predecessors(&mut predecessors);
        plan.predecessors = predecessors;
        prior_in_batch
            .entry(plan.course.id.clone())
            .or_default()
            .push(predecessor_from_plan(plan, false));
    }
    Ok(planned)
}

fn resolve_auto(path: &Path, courses: &[ResolvedCourse]) -> Result<ResolvedCourse, String> {
    let mut matches = courses
        .iter()
        .filter(|course| path_is_within(path, &course.root))
        .collect::<Vec<_>>();
    matches.sort_by_key(|course| std::cmp::Reverse(course.root.components().count()));
    let Some(best) = matches.first() else {
        return Err(format!(
            "selected source is outside every configured course root: {}",
            path.display()
        ));
    };
    if matches.get(1).is_some_and(|candidate| {
        candidate.root.components().count() == best.root.components().count()
    }) {
        return Err(format!(
            "selected source matches more than one equally specific course root: {}",
            path.display()
        ));
    }
    Ok((*best).clone())
}

fn plan_course_sources(
    course: ResolvedCourse,
    mut sources: Vec<PathBuf>,
) -> Result<Vec<PlannedGuide>, String> {
    sources.sort_by(|left, right| natural_path_cmp(left, right, &course.root));
    match course.guide_mode {
        GuideMode::LectureDeck => sources
            .into_iter()
            .map(|source| {
                if !is_allowed_lecture_primary(&source, &course.lecture_primary_rule) {
                    return Err(format!(
                        "selected source does not match the conservative lecture-primary rule for '{}': {}",
                        course.id,
                        source.display()
                    ));
                }
                plan_single(course.clone(), source)
            })
            .collect(),
        GuideMode::LegacyAuto => sources
            .into_iter()
            .map(|source| plan_single(course.clone(), source))
            .collect(),
        GuideMode::WeeklyLab | GuideMode::WeeklyMaterial => {
            let mut weeks: BTreeMap<u32, Vec<PathBuf>> = BTreeMap::new();
            for source in sources {
                let week = source_week(&source, course.guide_mode)?.ok_or_else(|| {
                    format!(
                        "could not determine a teaching week for selected source: {}",
                        source.display()
                    )
                })?;
                if is_preserved_week(&course, week) {
                    return Err(format!(
                        "week {week} for '{}' is checksum-pinned as a preserved baseline; select only newer weeks",
                        course.id,
                    ));
                }
                weeks.entry(week).or_default().push(source);
            }
            weeks
                .into_iter()
                .map(|(week, sources)| plan_week(course.clone(), week, sources))
                .collect()
        }
    }
}

fn plan_single(course: ResolvedCourse, source: PathBuf) -> Result<PlannedGuide, String> {
    let relative = source
        .strip_prefix(&course.root)
        .map_err(|_| "planned source escaped its resolved course root".to_string())?;
    let sequence_key = relative
        .with_extension("")
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase();
    let output_path = crate::job_events::derive_output_paths(&source)
        .map(|(_, _, output)| output)
        .ok_or_else(|| format!("could not derive guide output for {}", source.display()))?;
    Ok(new_plan(
        course,
        vec![source.clone()],
        source,
        output_path,
        sequence_key,
    ))
}

fn plan_week(
    course: ResolvedCourse,
    week: u32,
    mut sources: Vec<PathBuf>,
) -> Result<PlannedGuide, String> {
    sources.sort_by(|left, right| natural_path_cmp(left, right, &course.root));
    let primaries = sources
        .iter()
        .filter(|source| is_week_primary(source, course.guide_mode))
        .cloned()
        .collect::<Vec<_>>();
    let primary = match primaries.as_slice() {
        [primary] => primary.clone(),
        [] => {
            return Err(format!(
                "week {week} for '{}' has no unambiguous primary class source",
                course.id
            ))
        }
        _ => {
            return Err(format!(
                "week {week} for '{}' has multiple primary class sources: {}",
                course.id,
                primaries
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        }
    };
    if course.guide_mode == GuideMode::WeeklyMaterial {
        let selected = sources
            .iter()
            .map(|path| path_identity(path))
            .collect::<Result<HashSet<_>, _>>()?;
        for entry in walkdir::WalkDir::new(&course.root)
            .min_depth(1)
            .max_depth(3)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
        {
            let candidate = entry.into_path();
            let Some(text) = candidate.to_str() else {
                continue;
            };
            if !config::is_watched_extension(text) || config::should_skip(text) {
                continue;
            }
            if source_week(&candidate, course.guide_mode)?.is_some_and(|value| value != week) {
                continue;
            }
            let canonical = canonical_file(&candidate, "shared weekly support source")?;
            if !selected.contains(&path_identity(&canonical)?) {
                sources.push(canonical);
            }
        }
    }
    sources.retain(|source| source != &primary);
    sources.sort_by(|left, right| natural_path_cmp(left, right, &course.root));
    sources.insert(0, primary.clone());
    let output_path = crate::job_events::derive_output_paths(&primary)
        .map(|(_, _, output)| output)
        .ok_or_else(|| format!("could not derive guide output for {}", primary.display()))?;
    let sequence_key = format!("week-{week:02}");
    Ok(new_plan(
        course,
        sources,
        primary,
        output_path,
        sequence_key,
    ))
}

fn new_plan(
    course: ResolvedCourse,
    source_paths: Vec<PathBuf>,
    primary_source: PathBuf,
    output_path: PathBuf,
    sequence_key: String,
) -> PlannedGuide {
    let generation_identity = format!(
        "{}:{}:{}",
        course.id,
        course.expected_guide_kind.as_str(),
        sequence_key
    );
    PlannedGuide {
        job_id: Uuid::new_v4().to_string(),
        source_paths,
        primary_source,
        output_path,
        course,
        sequence_key,
        generation_identity,
        predecessors: Vec::new(),
    }
}

fn historical_predecessors(plan: &PlannedGuide) -> Result<Vec<PlannedPredecessor>, String> {
    let mut predecessors = Vec::new();
    if plan.course.guide_mode == GuideMode::LectureDeck {
        let mut sources = walkdir::WalkDir::new(&plan.course.root)
            .min_depth(1)
            .max_depth(3)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
            .map(|entry| entry.into_path())
            .filter(|path| {
                path.to_str().is_some_and(|text| {
                    config::is_watched_extension(text) && !config::should_skip(text)
                }) && is_allowed_lecture_primary(path, &plan.course.lecture_primary_rule)
            })
            .collect::<Vec<_>>();
        sources.sort_by(|left, right| natural_path_cmp(left, right, &plan.course.root));
        for source in sources {
            let relative = source
                .strip_prefix(&plan.course.root)
                .unwrap_or(&source)
                .with_extension("")
                .to_string_lossy()
                .replace('\\', "/")
                .to_lowercase();
            if natural_cmp(&relative, &plan.sequence_key) != Ordering::Less {
                continue;
            }
            if let Some((_, _, output)) = crate::job_events::derive_output_paths(&source) {
                if completion::is_usable_guide(&output) {
                    let predecessor_plan = plan_single(plan.course.clone(), source)?;
                    predecessors.push(predecessor_from_plan(&predecessor_plan, false));
                }
            }
        }
    }
    if matches!(
        plan.course.guide_mode,
        GuideMode::WeeklyLab | GuideMode::WeeklyMaterial
    ) {
        let current_week = source_week(&plan.primary_source, plan.course.guide_mode)?
            .ok_or_else(|| "planned weekly guide has no teaching week".to_string())?;
        let mut by_week: BTreeMap<u32, Vec<PathBuf>> = BTreeMap::new();
        for entry in walkdir::WalkDir::new(&plan.course.root)
            .min_depth(1)
            .max_depth(3)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
        {
            let source = entry.into_path();
            if !source.to_str().is_some_and(|text| {
                config::is_watched_extension(text) && !config::should_skip(text)
            }) {
                continue;
            }
            if let Some(week) = source_week(&source, plan.course.guide_mode)? {
                if week < current_week {
                    by_week.entry(week).or_default().push(source);
                }
            }
        }
        for (week, sources) in by_week {
            let predecessor_plan = plan_week(plan.course.clone(), week, sources)?;
            if completion::is_usable_guide(&predecessor_plan.output_path) {
                predecessors.push(predecessor_from_plan(&predecessor_plan, false));
            }
        }
    }
    if matches!(
        plan.course.guide_mode,
        GuideMode::WeeklyLab | GuideMode::WeeklyMaterial
    ) {
        for baseline in preserved_baselines(&plan.course)
            .into_iter()
            .filter(|baseline| natural_cmp(&baseline.sequence_key, &plan.sequence_key).is_lt())
        {
            let baseline_path = canonical_file(&baseline.path, "pinned predecessor baseline")?;
            let actual = sha256_file(&baseline_path)?;
            if actual != baseline.sha256 {
                return Err(format!(
                    "pinned predecessor baseline checksum mismatch: {}",
                    baseline_path.display()
                ));
            }
            predecessors.push(PlannedPredecessor {
                course_profile: plan.course.id.clone(),
                generation_identity: format!(
                    "{}:{}:{}",
                    plan.course.id,
                    plan.course.expected_guide_kind.as_str(),
                    baseline.sequence_key
                ),
                sequence_key: baseline.sequence_key.clone(),
                path: baseline_path,
                pinned: true,
            });
        }
    }
    sort_and_deduplicate_predecessors(&mut predecessors);
    Ok(predecessors)
}

/// A bound contract re-validated against the live course folder. `predecessor_drift` lists
/// every way the predecessor guides differ from when the context was saved; the plan carries
/// the current predecessors.
#[derive(Debug, Clone)]
pub struct ContractValidation {
    pub plan: PlannedGuide,
    pub predecessor_drift: Vec<String>,
}

pub fn validate_bound_contract(
    contract: &BoundGenerationContract,
) -> Result<ContractValidation, String> {
    validate_bound_contract_with_courses(contract, &configured_courses()?)
}

fn validate_bound_contract_with_courses(
    contract: &BoundGenerationContract,
    courses: &[ResolvedCourse],
) -> Result<ContractValidation, String> {
    if contract.schema_version != 1 {
        return Err(format!(
            "unsupported generation contract schema {}; expected 1",
            contract.schema_version
        ));
    }
    if contract.sources.is_empty() {
        return Err("generation contract has no bound sources".to_string());
    }
    let sources = contract
        .sources
        .iter()
        .map(|source| {
            let path = canonical_file(Path::new(&source.path), "bound source")?;
            let actual = sha256_file(&path)?;
            if actual != source.sha256 {
                return Err(format!(
                    "bound source checksum mismatch: {}",
                    path.display()
                ));
            }
            Ok(path)
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut plans = plan_guides_with_courses(&sources, &contract.course_profile, courses)?;
    if plans.len() != 1 {
        return Err("generation contract sources no longer resolve to one guide".to_string());
    }
    let mut plan = plans.remove(0);
    let primary = canonical_file(Path::new(&contract.primary_source), "bound primary source")?;
    let output = absolute_output_path(Path::new(&contract.output_path))?;
    if plan.source_paths != sources
        || plan.primary_source != primary
        || plan.output_path != output
        || plan.generation_identity != contract.generation_identity
        || plan.course.expected_guide_kind != contract.expected_guide_kind
    {
        return Err("generation contract course, guide kind, identity, primary source, or output path was tampered with".to_string());
    }

    // Predecessor guides are allowed to drift between saving the context and resuming:
    // an owner may revise one by hand, and an earlier guide may have become usable since.
    // The plan already carries the current predecessors; report every difference so the
    // run log says exactly what changed, and never refuse on it.
    let bound_predecessors = contract
        .predecessors
        .iter()
        .map(|value| (value, canonical_file(Path::new(&value.path), "bound predecessor").ok()))
        .collect::<Vec<_>>();
    let display_name = |path: &Path| {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string())
    };
    let mut predecessor_drift = Vec::new();
    for planned in plan.predecessors.iter_mut() {
        if let Ok(canonical) = canonical_file(&planned.path, "planned predecessor") {
            planned.path = canonical;
        }
        let bound = bound_predecessors
            .iter()
            .find(|(_, path)| path.as_ref().is_some_and(|path| same_path(&planned.path, path)));
        match bound {
            None => predecessor_drift.push(format!(
                "{} is now a usable predecessor (it was not when the context was saved)",
                display_name(&planned.path)
            )),
            Some((bound, _)) => {
                let actual = sha256_file(&planned.path)?;
                if actual != bound.sha256 {
                    predecessor_drift.push(format!(
                        "{} was revised since the context was saved; its current version is used",
                        display_name(&planned.path)
                    ));
                }
            }
        }
    }
    for (bound, path) in &bound_predecessors {
        let still_planned = path
            .as_ref()
            .is_some_and(|path| plan.predecessors.iter().any(|planned| same_path(&planned.path, path)));
        if !still_planned {
            predecessor_drift.push(format!(
                "{} is no longer a usable predecessor and is left out",
                display_name(Path::new(&bound.path))
            ));
        }
    }
    Ok(ContractValidation {
        plan,
        predecessor_drift,
    })
}

fn predecessor_from_plan(plan: &PlannedGuide, pinned: bool) -> PlannedPredecessor {
    PlannedPredecessor {
        course_profile: plan.course.id.clone(),
        generation_identity: plan.generation_identity.clone(),
        sequence_key: plan.sequence_key.clone(),
        path: plan.output_path.clone(),
        pinned,
    }
}

fn sort_and_deduplicate_predecessors(predecessors: &mut Vec<PlannedPredecessor>) {
    predecessors.sort_by(|left, right| {
        natural_cmp(&left.sequence_key, &right.sequence_key)
            .then_with(|| left.generation_identity.cmp(&right.generation_identity))
    });
    let mut seen = HashSet::new();
    predecessors.retain(|predecessor| {
        path_identity(&predecessor.path).is_ok_and(|identity| seen.insert(identity))
    });
}

fn source_week(path: &Path, mode: GuideMode) -> Result<Option<u32>, String> {
    if mode == GuideMode::WeeklyLab {
        let source_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "weekly source filename is not valid Unicode".to_string())?;
        let sidecar = path.with_file_name(format!("{source_name}.guide-context.json"));
        if sidecar.is_file() {
            let bytes = std::fs::read(&sidecar)
                .map_err(|error| format!("could not read weekly sidecar: {error}"))?;
            let value: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|error| format!("invalid weekly sidecar JSON: {error}"))?;
            return value
                .pointer("/unit/week")
                .and_then(serde_json::Value::as_u64)
                .and_then(|week| u32::try_from(week).ok())
                .filter(|week| (1..=60).contains(week))
                .map(Some)
                .ok_or_else(|| "weekly sidecar unit.week must be between 1 and 60".to_string());
        }
    }
    week_from_path(path)
}

fn week_from_path(path: &Path) -> Result<Option<u32>, String> {
    let pattern = Regex::new(r"(?i)(?:^|[^a-z])week[ _-]*0*([1-9][0-9]?)(?:[^0-9]|$)")
        .map_err(|error| format!("could not compile week matcher: {error}"))?;
    let text = path.to_string_lossy();
    Ok(pattern
        .captures_iter(&text)
        .last()
        .and_then(|captures| captures[1].parse::<u32>().ok())
        .filter(|week| *week <= 60))
}

fn is_week_primary(path: &Path, mode: GuideMode) -> bool {
    match mode {
        GuideMode::WeeklyLab => {
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            path.with_file_name(format!("{name}.guide-context.json"))
                .is_file()
        }
        GuideMode::WeeklyMaterial => {
            path.extension()
                .and_then(|value| value.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("pptx"))
                && path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| {
                        let name = name.to_lowercase();
                        name.contains("lecture") || name.contains("class")
                    })
        }
        _ => false,
    }
}

/// Whether `path` is one of this course's own lecture decks by the course's naming rule.
/// `Any` recognizes nothing here: it admits every file as a primary source, so it cannot
/// single out sibling decks.
pub(crate) fn is_sibling_lecture_deck(path: &Path, rule: &LecturePrimaryRule) -> bool {
    *rule != LecturePrimaryRule::Any && is_allowed_lecture_primary(path, rule)
}

fn is_allowed_lecture_primary(path: &Path, rule: &LecturePrimaryRule) -> bool {
    let LecturePrimaryRule::Named(pattern) = rule else {
        return true;
    };
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    // A bad pattern in the settings file must not silently hide every lecture, so an
    // uncompilable expression accepts everything and the user sees their files.
    Regex::new(&format!("(?i){pattern}")).is_ok_and(|matcher| matcher.is_match(name))
        || Regex::new(pattern).is_err()
}

fn natural_path_cmp(left: &Path, right: &Path, root: &Path) -> Ordering {
    let left = left.strip_prefix(root).unwrap_or(left).to_string_lossy();
    let right = right.strip_prefix(root).unwrap_or(right).to_string_lossy();
    natural_cmp(&left, &right)
}

pub fn natural_cmp(left: &str, right: &str) -> Ordering {
    let left = left.to_lowercase();
    let right = right.to_lowercase();
    let left_bytes = left.as_bytes();
    let right_bytes = right.as_bytes();
    let (mut left_index, mut right_index) = (0, 0);
    while left_index < left_bytes.len() && right_index < right_bytes.len() {
        if left_bytes[left_index].is_ascii_digit() && right_bytes[right_index].is_ascii_digit() {
            let left_end = digit_end(left_bytes, left_index);
            let right_end = digit_end(right_bytes, right_index);
            let left_number = left[left_index..left_end].trim_start_matches('0');
            let right_number = right[right_index..right_end].trim_start_matches('0');
            let left_number = if left_number.is_empty() {
                "0"
            } else {
                left_number
            };
            let right_number = if right_number.is_empty() {
                "0"
            } else {
                right_number
            };
            let order = left_number
                .len()
                .cmp(&right_number.len())
                .then_with(|| left_number.cmp(right_number));
            if order != Ordering::Equal {
                return order;
            }
            left_index = left_end;
            right_index = right_end;
        } else {
            let order = left_bytes[left_index].cmp(&right_bytes[right_index]);
            if order != Ordering::Equal {
                return order;
            }
            left_index += 1;
            right_index += 1;
        }
    }
    left_bytes
        .len()
        .cmp(&right_bytes.len())
        .then_with(|| left.cmp(&right))
}

fn digit_end(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    index
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    let path = path.components().map(component_key).collect::<Vec<_>>();
    let root = root.components().map(component_key).collect::<Vec<_>>();
    path.len() >= root.len()
        && path
            .iter()
            .zip(root.iter())
            .all(|(left, right)| left == right)
}

fn component_key(component: Component<'_>) -> String {
    component.as_os_str().to_string_lossy().to_lowercase()
}

fn canonical_file(path: &Path, label: &str) -> Result<PathBuf, String> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("could not resolve current directory: {error}"))?
            .join(path)
    };
    if !path.is_file() {
        return Err(format!(
            "{label} is not a readable file: {}",
            path.display()
        ));
    }
    path.canonicalize()
        .map_err(|error| format!("could not resolve {label} {}: {error}", path.display()))
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    if !path.is_dir() {
        return Err(format!(
            "{label} is not a readable directory: {}",
            path.display()
        ));
    }
    path.canonicalize()
        .map_err(|error| format!("could not resolve {label} {}: {error}", path.display()))
}

fn absolute_output_path(path: &Path) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| "bound output path has no parent directory".to_string())?;
    let parent = canonical_directory(parent, "bound output directory")?;
    let name = path
        .file_name()
        .ok_or_else(|| "bound output path has no filename".to_string())?;
    Ok(parent.join(name))
}

fn path_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| format!("path is not valid Unicode: {}", path.display()))
}

fn path_identity(path: &Path) -> Result<String, String> {
    Ok(path_text(path)?.replace('\\', "/").to_lowercase())
}

fn same_path(left: &Path, right: &Path) -> bool {
    path_identity(left).ok() == path_identity(right).ok()
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("could not read {} for hashing: {error}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("guide-course-plan-{}", Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn course(
        root: &Path,
        id: &str,
        mode: GuideMode,
        kind: GuideKind,
        order: u8,
    ) -> ResolvedCourse {
        ResolvedCourse {
            id: id.to_string(),
            label: id.to_string(),
            root: root.canonicalize().unwrap(),
            guide_mode: mode,
            lecture_primary_rule: LecturePrimaryRule::Any,
            expected_guide_kind: kind,
            profile_order: order,
            pinned_guides: Vec::new(),
        }
    }

    fn source(root: &Path, name: &str) -> PathBuf {
        let path = root.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, b"source").unwrap();
        path
    }

    #[test]
    fn auto_uses_longest_component_root_and_rejects_near_prefix_and_explicit_mismatch() {
        let root = TestDir::new();
        let broad_root = root.0.join("Course");
        let narrow_root = broad_root.join("Advanced");
        let near_root = root.0.join("Course Extra");
        let other_root = root.0.join("Other");
        std::fs::create_dir_all(&narrow_root).unwrap();
        std::fs::create_dir_all(&near_root).unwrap();
        std::fs::create_dir_all(&other_root).unwrap();
        let selected = source(&narrow_root, "Lecture 1.pdf");
        let near = source(&near_root, "Lecture 2.pdf");
        let courses = vec![
            course(
                &broad_root,
                "broad",
                GuideMode::LectureDeck,
                GuideKind::Lecture,
                0,
            ),
            course(
                &narrow_root,
                "narrow",
                GuideMode::LectureDeck,
                GuideKind::Lecture,
                1,
            ),
            course(
                &other_root,
                "other",
                GuideMode::LectureDeck,
                GuideKind::Lecture,
                2,
            ),
        ];

        let planned =
            plan_guides_with_courses(std::slice::from_ref(&selected), "auto", &courses).unwrap();
        assert_eq!(planned[0].course.id, "narrow");
        assert!(plan_guides_with_courses(&[near], "auto", &courses).is_err());
        let error = plan_guides_with_courses(&[selected], "other", &courses).unwrap_err();
        assert!(error.contains("does not contain") || error.contains("unknown"));
    }

    #[test]
    fn plans_all_inputs_before_work_and_uses_course_then_natural_order() {
        let root = TestDir::new();
        let networks = root.0.join("Networks");
        let algorithms = root.0.join("Algorithms");
        std::fs::create_dir_all(&networks).unwrap();
        std::fs::create_dir_all(&algorithms).unwrap();
        let n10 = source(&networks, "Lecture 10.pdf");
        let n2 = source(&networks, "Lecture 2.pdf");
        let n2_duplicate_output = source(&networks, "Lecture 2.pptx");
        let a1 = source(&algorithms, "Lecture 1.pdf");
        let invalid = root.0.join("missing.pdf");
        let courses = vec![
            course(
                &networks,
                "networks",
                GuideMode::LectureDeck,
                GuideKind::Lecture,
                0,
            ),
            course(
                &algorithms,
                "algorithms",
                GuideMode::LectureDeck,
                GuideKind::Lecture,
                1,
            ),
        ];

        let planned = plan_guides_with_courses(&[a1, n10, n2], "auto", &courses).unwrap();
        assert_eq!(planned[0].sequence_key, "lecture 2");
        assert_eq!(planned[1].sequence_key, "lecture 10");
        assert_eq!(planned[2].course.id, "algorithms");
        assert!(plan_guides_with_courses(
            &[planned[0].primary_source.clone(), invalid],
            "auto",
            &courses
        )
        .is_err());
        let error = plan_guides_with_courses(
            &[planned[0].primary_source.clone(), n2_duplicate_output],
            "auto",
            &courses,
        )
        .unwrap_err();
        assert!(error.contains("same guide output") || error.contains("generation identity"));
    }

    #[test]
    fn weekly_sources_group_once_in_week_order_and_duplicate_primaries_fail() {
        let root = TestDir::new();
        let sw = root.0.join("Scientific Writing");
        std::fs::create_dir_all(&sw).unwrap();
        let week10 = source(&sw, "Week 10 Lecture.pptx");
        let week2 = source(&sw, "Week 2 Lecture.pptx");
        let worksheet = source(&sw, "Week 2 Worksheet.docx");
        let courses = vec![course(
            &sw,
            "scientific-writing",
            GuideMode::WeeklyMaterial,
            GuideKind::ScientificWritingWeek,
            4,
        )];

        let planned = plan_guides_with_courses(
            &[week10, worksheet.clone(), week2.clone()],
            "auto",
            &courses,
        )
        .unwrap();
        assert_eq!(planned.len(), 2);
        assert_eq!(planned[0].sequence_key, "week-02");
        assert_eq!(planned[0].source_paths.len(), 2);
        assert_eq!(planned[1].sequence_key, "week-10");
        let duplicate = source(&sw, "Week 2 Class.pptx");
        let error =
            plan_guides_with_courses(&[week2, worksheet, duplicate], "auto", &courses).unwrap_err();
        assert!(error.contains("multiple primary"));
    }

    #[test]
    fn invalid_future_and_cross_course_guides_are_not_discovered_as_predecessors() {
        let root = TestDir::new();
        let networks = root.0.join("Networks");
        let algorithms = root.0.join("Algorithms");
        std::fs::create_dir_all(&networks).unwrap();
        std::fs::create_dir_all(&algorithms).unwrap();
        let lecture2 = source(&networks, "Lecture 2.pdf");
        source(&networks, "Lecture 10.pdf");
        std::fs::write(networks.join("Lecture_1_Guide.md"), "invalid receipt").unwrap();
        std::fs::write(networks.join("Lecture_10_Guide.md"), "future").unwrap();
        std::fs::write(algorithms.join("Lecture_1_Guide.md"), "cross course").unwrap();
        let courses = vec![
            course(
                &networks,
                "networks",
                GuideMode::LectureDeck,
                GuideKind::Lecture,
                0,
            ),
            course(
                &algorithms,
                "algorithms",
                GuideMode::LectureDeck,
                GuideKind::Lecture,
                1,
            ),
        ];

        let planned = plan_guides_with_courses(&[lecture2], "auto", &courses).unwrap();
        assert!(planned[0].predecessors.is_empty());
    }

    #[test]
    fn checksum_pinned_baseline_is_a_narrow_receipt_exception() {
        let root = TestDir::new();
        let sw = root.0.join("Scientific Writing");
        std::fs::create_dir_all(&sw).unwrap();
        let week2 = source(&sw, "Week 2 Lecture.pptx");
        let baseline = sw.join("Week 1 preserved.md");
        std::fs::write(&baseline, b"preserved baseline").unwrap();
        let mut sw_course = course(
            &sw,
            "scientific-writing",
            GuideMode::WeeklyMaterial,
            GuideKind::ScientificWritingWeek,
            4,
        );
        sw_course.pinned_guides = vec![PinnedBaseline {
            path: baseline.clone(),
            sha256: sha256_file(&baseline).unwrap(),
            sequence_key: "week-01".to_string(),
        }];

        let planned = plan_guides_with_courses(&[week2], "auto", &[sw_course]).unwrap();
        assert_eq!(
            planned[0]
                .predecessors
                .iter()
                .map(|predecessor| predecessor.path.clone())
                .collect::<Vec<_>>(),
            vec![baseline.canonicalize().unwrap()]
        );
    }

    #[test]
    fn a_pinned_guide_is_preserved_and_never_offered_as_a_new_source() {
        let root = TestDir::new();
        let lab = root.0.join("Circuit Lab");
        std::fs::create_dir_all(&lab).unwrap();

        // Two guides already written, which must be kept rather than regenerated.
        let week_one = lab.join("Week 01 Guide.md");
        let week_two = lab.join("Week 02 Guide.md");
        std::fs::write(&week_one, b"week one, already written").unwrap();
        std::fs::write(&week_two, b"week two, already written").unwrap();

        let mut lab_course = course(
            &lab,
            "circuit-lab",
            GuideMode::WeeklyLab,
            GuideKind::CircuitLab,
            3,
        );
        lab_course.pinned_guides = vec![
            PinnedBaseline {
                path: week_one.clone(),
                sha256: sha256_file(&week_one).unwrap(),
                sequence_key: "week-01".to_string(),
            },
            PinnedBaseline {
                path: week_two.clone(),
                sha256: sha256_file(&week_two).unwrap(),
                sequence_key: "week-02".to_string(),
            },
        ];

        let baselines = preserved_baselines(&lab_course);
        assert_eq!(
            baselines
                .iter()
                .map(|baseline| baseline.sequence_key.as_str())
                .collect::<Vec<_>>(),
            ["week-01", "week-02"]
        );
        // The checksum is what proves a pinned guide is still the file that was pinned.
        for baseline in &baselines {
            assert_eq!(sha256_file(&baseline.path).unwrap(), baseline.sha256);
        }

        // A pinned guide is not material to make another guide out of.
        let courses = [lab_course];
        assert!(!is_scannable_source_with_courses(&week_one, &courses));
        assert!(!is_scannable_source_with_courses(&week_two, &courses));

        // New material in the same folder still is.
        let fresh = source(&lab, "Week 03 Lab.pdf");
        assert!(is_scannable_source_with_courses(&fresh, &courses));
    }

    #[test]
    fn configured_scan_ignores_unassigned_support_and_the_pinned_week_one_sources() {
        let root = TestDir::new();
        let sw = root.0.join("Scientific Writing");
        std::fs::create_dir_all(&sw).unwrap();
        let week1 = source(&sw, "Week 1 Lecture.pptx");
        let week2 = source(&sw, "Week 2 Lecture.pptx");
        let week2_worksheet = source(&sw, "Week 2 Worksheet.pdf");
        let reference = source(&sw, "APA Reference.pdf");
        let baseline = sw.join("Week 1 preserved.md");
        std::fs::write(&baseline, b"preserved baseline").unwrap();
        let mut sw_course = course(
            &sw,
            "scientific-writing",
            GuideMode::WeeklyMaterial,
            GuideKind::ScientificWritingWeek,
            4,
        );
        sw_course.pinned_guides = vec![PinnedBaseline {
            sha256: sha256_file(&baseline).unwrap(),
            path: baseline,
            sequence_key: "week-01".to_string(),
        }];
        let courses = [sw_course];

        assert!(!is_scannable_source_with_courses(&week1, &courses));
        assert!(is_scannable_source_with_courses(&week2, &courses));
        assert!(!is_scannable_source_with_courses(&reference, &courses));
        assert!(plan_guides_with_courses(&[week1], "auto", &courses).is_err());
        let planned = plan_guides_with_courses(&[week2], "auto", &courses).unwrap();
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].source_paths.len(), 3);
        assert!(planned[0]
            .source_paths
            .contains(&week2_worksheet.canonicalize().unwrap()));
        assert!(planned[0]
            .source_paths
            .contains(&reference.canonicalize().unwrap()));
    }

    #[test]
    fn normal_scan_returns_only_the_next_unfinished_guide_per_course() {
        let root = TestDir::new();
        let networks = root.0.join("Networks");
        let algorithms = root.0.join("Algorithms");
        std::fs::create_dir_all(&networks).unwrap();
        std::fs::create_dir_all(&algorithms).unwrap();
        let network_1 = source(&networks, "Lecture 1.pdf");
        let network_2 = source(&networks, "Lecture 2.pdf");
        let network_3 = source(&networks, "Lecture 3.pdf");
        let algorithm_1 = source(&algorithms, "Lecture 1.pdf");
        let courses = vec![
            course(
                &networks,
                "computer-networks",
                GuideMode::LectureDeck,
                GuideKind::Lecture,
                0,
            ),
            course(
                &algorithms,
                "computer-algorithms",
                GuideMode::LectureDeck,
                GuideKind::Lecture,
                1,
            ),
        ];
        let network_1_output = crate::job_events::derive_output_paths(&network_1)
            .unwrap()
            .2;
        std::fs::write(&network_1_output, "# Completed lecture one").unwrap();
        completion::append_receipt(&network_1_output).unwrap();

        let wave = next_pending_scan_wave_with_courses(
            &[
                network_3.clone(),
                algorithm_1.clone(),
                network_1.clone(),
                network_2.clone(),
            ],
            &courses,
        )
        .unwrap();
        assert_eq!(wave.len(), 2);
        assert!(wave.contains(&network_2.canonicalize().unwrap()));
        assert!(wave.contains(&algorithm_1.canonicalize().unwrap()));
        assert!(!wave.contains(&network_1.canonicalize().unwrap()));
        assert!(!wave.contains(&network_3.canonicalize().unwrap()));

        let network_2_output = crate::job_events::derive_output_paths(&network_2)
            .unwrap()
            .2;
        std::fs::write(&network_2_output, "# Completed lecture two").unwrap();
        completion::append_receipt(&network_2_output).unwrap();
        let next = next_pending_scan_wave_with_courses(
            &[network_1, network_2, network_3.clone()],
            &courses,
        )
        .unwrap();
        assert_eq!(next, vec![network_3.canonicalize().unwrap()]);
    }

    #[test]
    fn bound_contract_rejects_profile_and_kind_tampering() {
        let root = TestDir::new();
        let networks = root.0.join("Networks");
        std::fs::create_dir_all(&networks).unwrap();
        let lecture = source(&networks, "Lecture 2.pdf").canonicalize().unwrap();
        let courses = vec![course(
            &networks,
            "computer-networks",
            GuideMode::LectureDeck,
            GuideKind::Lecture,
            0,
        )];
        let plan = plan_guides_with_courses(std::slice::from_ref(&lecture), "auto", &courses)
            .unwrap()
            .remove(0);
        let mut contract = BoundGenerationContract {
            schema_version: 1,
            course_profile: "computer-networks".to_string(),
            expected_guide_kind: GuideKind::Lecture,
            generation_identity: plan.generation_identity,
            primary_source: path_text(&lecture).unwrap(),
            output_path: path_text(&plan.output_path).unwrap(),
            sources: vec![BoundFile {
                path: path_text(&lecture).unwrap(),
                sha256: sha256_file(&lecture).unwrap(),
            }],
            predecessors: Vec::new(),
        };
        assert!(validate_bound_contract_with_courses(&contract, &courses).is_ok());
        contract.expected_guide_kind = GuideKind::CircuitLab;
        assert!(validate_bound_contract_with_courses(&contract, &courses).is_err());
        contract.expected_guide_kind = GuideKind::Lecture;
        contract.course_profile = "networks-near-prefix".to_string();
        assert!(validate_bound_contract_with_courses(&contract, &courses).is_err());

        let lecture1 = source(&networks, "Lecture 1.pdf");
        let predecessor = crate::job_events::derive_output_paths(&lecture1).unwrap().2;
        std::fs::write(&predecessor, "# Verified predecessor").unwrap();
        completion::append_receipt(&predecessor).unwrap();
        let planned = plan_guides_with_courses(std::slice::from_ref(&lecture), "auto", &courses)
            .unwrap()
            .remove(0);
        let mut with_predecessor = BoundGenerationContract {
            schema_version: 1,
            course_profile: planned.course.id.clone(),
            expected_guide_kind: planned.course.expected_guide_kind,
            generation_identity: planned.generation_identity.clone(),
            primary_source: path_text(&planned.primary_source).unwrap(),
            output_path: path_text(&planned.output_path).unwrap(),
            sources: vec![BoundFile {
                path: path_text(&planned.primary_source).unwrap(),
                sha256: sha256_file(&planned.primary_source).unwrap(),
            }],
            predecessors: vec![BoundPredecessor {
                course_profile: planned.course.id.clone(),
                generation_identity: planned.predecessors[0].generation_identity.clone(),
                sequence_key: planned.predecessors[0].sequence_key.clone(),
                path: path_text(&predecessor.canonicalize().unwrap()).unwrap(),
                sha256: sha256_file(&predecessor).unwrap(),
            }],
        };
        let exact = validate_bound_contract_with_courses(&with_predecessor, &courses).unwrap();
        assert!(exact.predecessor_drift.is_empty(), "{:?}", exact.predecessor_drift);

        // A predecessor that became usable after the context was saved is re-bound and
        // reported, never refused.
        with_predecessor.predecessors.clear();
        let gained = validate_bound_contract_with_courses(&with_predecessor, &courses).unwrap();
        assert_eq!(gained.plan.predecessors.len(), 1);
        assert!(gained.predecessor_drift.iter().any(|note| note.contains("now a usable predecessor")), "{:?}", gained.predecessor_drift);

        // A predecessor revised after the context was saved is reported with its new bytes used.
        let mut revised = BoundGenerationContract {
            predecessors: vec![BoundPredecessor {
                course_profile: planned.course.id.clone(),
                generation_identity: planned.predecessors[0].generation_identity.clone(),
                sequence_key: planned.predecessors[0].sequence_key.clone(),
                path: path_text(&predecessor.canonicalize().unwrap()).unwrap(),
                sha256: "0".repeat(64),
            }],
            ..with_predecessor.clone()
        };
        let drifted = validate_bound_contract_with_courses(&revised, &courses).unwrap();
        assert!(drifted.predecessor_drift.iter().any(|note| note.contains("was revised since")), "{:?}", drifted.predecessor_drift);

        // A bound predecessor that no longer exists is reported as left out.
        revised.predecessors[0].path = path_text(&networks.join("Gone_Guide.md")).unwrap();
        let gone = validate_bound_contract_with_courses(&revised, &courses).unwrap();
        assert!(gone.predecessor_drift.iter().any(|note| note.contains("no longer a usable predecessor")), "{:?}", gone.predecessor_drift);
    }

    #[test]
    fn a_configured_pattern_picks_the_lectures_out_of_a_folder() {
        let chapters = LecturePrimaryRule::Named(r"^ch[0-9]+([_ -].*)?\.pdf$".to_string());
        assert!(is_allowed_lecture_primary(
            Path::new("CH02_Physical Layer (1).pdf"),
            &chapters
        ));
        // The textbook in the same folder is not a lecture.
        assert!(!is_allowed_lecture_primary(Path::new("Computer Networking textbook.pdf"), &chapters));

        // Patterns are matched case-insensitively, so nobody has to think about it.
        let lessons = LecturePrimaryRule::Named(r"^lesson[0-9]+.*\.pdf$".to_string());
        assert!(is_allowed_lecture_primary(Path::new("Lesson04 Composition.pdf"), &lessons));
        assert!(!is_allowed_lecture_primary(Path::new("Reading list.pdf"), &lessons));

        // Any subject can be watched without writing a pattern at all.
        for name in ["Anything At All.pdf", "notes.docx", "slides.pptx"] {
            assert!(is_allowed_lecture_primary(Path::new(name), &LecturePrimaryRule::Any));
        }

        // A pattern with a typo must not hide every lecture the user owns: it fails open, so
        // they see their files and can fix the settings, rather than facing an empty folder.
        let broken = LecturePrimaryRule::Named(r"^ch[0-9+([_ -.pdf$".to_string());
        assert!(is_allowed_lecture_primary(Path::new("CH02 Physical Layer.pdf"), &broken));
        assert!(is_allowed_lecture_primary(Path::new("anything.pdf"), &broken));
    }

    #[test]
    fn historical_week_with_zero_or_multiple_primaries_is_never_silently_skipped() {
        let zero_root = TestDir::new();
        let zero_course_root = zero_root.0.join("Scientific Writing");
        std::fs::create_dir_all(&zero_course_root).unwrap();
        source(&zero_course_root, "Week 2 Worksheet.docx");
        let selected = source(&zero_course_root, "Week 3 Lecture.pptx");
        let zero_course = course(
            &zero_course_root,
            "scientific-writing",
            GuideMode::WeeklyMaterial,
            GuideKind::ScientificWritingWeek,
            4,
        );
        let error = plan_guides_with_courses(&[selected], "auto", &[zero_course]).unwrap_err();
        assert!(error.contains("no unambiguous primary"), "{error}");

        let multiple_root = TestDir::new();
        let multiple_course_root = multiple_root.0.join("Scientific Writing");
        std::fs::create_dir_all(&multiple_course_root).unwrap();
        source(&multiple_course_root, "Week 2 Lecture.pptx");
        source(&multiple_course_root, "Week 2 Class.pptx");
        let selected = source(&multiple_course_root, "Week 3 Lecture.pptx");
        let multiple_course = course(
            &multiple_course_root,
            "scientific-writing",
            GuideMode::WeeklyMaterial,
            GuideKind::ScientificWritingWeek,
            4,
        );
        let error = plan_guides_with_courses(&[selected], "auto", &[multiple_course]).unwrap_err();
        assert!(error.contains("multiple primary"), "{error}");
    }

    #[test]
    fn weekly_predecessors_keep_semantic_week_order_for_writing_and_circuit_titles() {
        let writing_root = TestDir::new();
        let writing = writing_root.0.join("Scientific Writing");
        std::fs::create_dir_all(&writing).unwrap();
        let baseline = writing.join("Week 1 preserved.md");
        std::fs::write(&baseline, "# Week one baseline").unwrap();
        for week in [2, 3] {
            let lecture = source(&writing, &format!("#Week{week} Lecture.pptx"));
            let output = crate::job_events::derive_output_paths(&lecture).unwrap().2;
            std::fs::write(&output, format!("# Week {week} guide")).unwrap();
            completion::append_receipt(&output).unwrap();
        }
        let selected = source(&writing, "Week 4 Lecture.pptx");
        let mut writing_course = course(
            &writing,
            "scientific-writing",
            GuideMode::WeeklyMaterial,
            GuideKind::ScientificWritingWeek,
            4,
        );
        writing_course.pinned_guides = vec![PinnedBaseline {
            path: baseline.clone(),
            sha256: sha256_file(&baseline).unwrap(),
            sequence_key: "week-01".to_string(),
        }];
        let plan = plan_guides_with_courses(&[selected], "auto", &[writing_course])
            .unwrap()
            .remove(0);
        assert_eq!(
            plan.predecessors
                .iter()
                .map(|predecessor| predecessor.sequence_key.as_str())
                .collect::<Vec<_>>(),
            ["week-01", "week-02", "week-03"]
        );

        let circuit_root = TestDir::new();
        let circuit = circuit_root.0.join("Circuit Lab");
        std::fs::create_dir_all(&circuit).unwrap();
        for (week, title) in [(2, "Ohms Law"), (3, "Kirchhoff Nodes")] {
            let lecture = source(&circuit, &format!("Week {week} - {title}.pdf"));
            std::fs::write(
                lecture.with_file_name(format!(
                    "{}.guide-context.json",
                    lecture.file_name().unwrap().to_string_lossy()
                )),
                serde_json::json!({"unit": {"week": week}}).to_string(),
            )
            .unwrap();
            let output = crate::job_events::derive_output_paths(&lecture).unwrap().2;
            std::fs::write(&output, format!("# {title}")).unwrap();
            completion::append_receipt(&output).unwrap();
        }
        let selected = source(&circuit, "Week 4 - Thevenin Equivalent.pdf");
        std::fs::write(
            selected.with_file_name(format!(
                "{}.guide-context.json",
                selected.file_name().unwrap().to_string_lossy()
            )),
            serde_json::json!({"unit": {"week": 4}}).to_string(),
        )
        .unwrap();
        let circuit_course = course(
            &circuit,
            "circuit-lab",
            GuideMode::WeeklyLab,
            GuideKind::CircuitLab,
            3,
        );
        let plan = plan_guides_with_courses(&[selected], "auto", &[circuit_course])
            .unwrap()
            .remove(0);
        assert_eq!(
            plan.predecessors
                .iter()
                .map(|predecessor| predecessor.sequence_key.as_str())
                .collect::<Vec<_>>(),
            ["week-02", "week-03"]
        );
    }
}
