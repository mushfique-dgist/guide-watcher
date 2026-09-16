//! App-owned visual planning, validation, and deterministic rasterization.
//!
//! Visual packets are local data contracts. They cannot ask the app to fetch a
//! URL, execute code, or interpret SVG/HTML. Every byte used by a renderer is
//! captured during whole-batch preflight and checked again before a provider is
//! allowed to run.

use crate::course_plan::{GuideMode, PlannedGuide};
#[cfg(test)]
use crate::source_context::CourseContextAnnotation;
use crate::source_context::{
    CapturedDependency, CapturedSource, CapturedTextbook, CourseContextAsset,
    CourseContextProcedureStep, CourseContextProcedureStepKind, PreflightResourceBudget,
    RenderedSourceSnapshot, SourceSnapshot,
};
use fontdue::layout::{CoordinateSystem, Layout, LayoutSettings, TextStyle};
use fontdue::{Font, FontSettings};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

const MAX_PACKET_BYTES: u64 = 2 * 1024 * 1024;
const MAX_INPUT_BYTES: u64 = crate::png_validation::MAX_PNG_INPUT_BYTES;
const MAX_INPUTS: usize = 128;
const MAX_NEEDS: usize = 256;
const MAX_ASSETS: usize = 256;
const MAX_ANNOTATIONS: usize = 64;
const MAX_DIAGRAM_NODES: usize = 64;
const MAX_DIAGRAM_EDGES: usize = 128;
const DIAGRAM_TITLE_X: u32 = 28;
const DIAGRAM_TITLE_Y: u32 = 24;
const DIAGRAM_TITLE_SCALE: u32 = 3;
const MAX_DIAGRAM_TITLE_LINES: usize = 3;
const MAX_DIAGRAM_EDGE_LABEL_LINES: usize = 3;
const MAX_AUTOMATIC_VISUALS: usize = 128;
const MAX_VISUAL_DECODED_INPUT_BYTES: u64 = 256 * 1024 * 1024;
const WORST_CASE_GLYPH_PIXEL_WORK: u64 = 4_096;
const NORM_MAX: u16 = 10_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualMaterial {
    pub packet_path: PathBuf,
    pub packet_bytes: Vec<u8>,
    pub packet_sha256: String,
    pub dependencies: Vec<CapturedDependency>,
    pub contract: VisualContract,
    pub compiled_assets: Vec<CompiledVisualAsset>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledVisualAsset {
    pub catalog: VisualCatalogEntry,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisualContract {
    pub schema_version: u8,
    pub packet_sha256: String,
    pub decision: VisualDecision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_visuals_rationale: Option<String>,
    pub needs: Vec<VisualNeed>,
    pub procedure_steps: Vec<ProcedureStepDefinition>,
    pub assets: Vec<VisualCatalogEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisualCatalogEntry {
    pub id: String,
    pub filename: String,
    pub sha256: String,
    pub spec_sha256: String,
    pub width_px: u32,
    pub height_px: u32,
    pub kind: VisualAssetKind,
    pub evidence_class: EvidenceClass,
    pub need_ids: Vec<String>,
    pub source_unit_ids: Vec<String>,
    pub procedure_step_ids: Vec<String>,
    pub learning_purpose: String,
    pub alt: String,
    pub caption: String,
    pub explanation: String,
    pub provenance: VisualProvenance,
    pub rights: RightsMetadata,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VisualDecision {
    PurposefulVisuals,
    NoPurposefulVisual,
    AuthoringTodo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VisualAssetKind {
    AnnotatedSource,
    SourceCrop,
    DeterministicDiagram,
    GeneratedEducational,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceClass {
    Source,
    Explanatory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NeedKind {
    Concept,
    ProcedureStep,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceRequirement {
    Source,
    Precision,
    Explanatory,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProcedureStepKind {
    Physical,
    Conceptual,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProcedureEvidenceRole {
    Before,
    Action,
    Expected,
    WrongState,
    Recovery,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcedureStepDefinition {
    pub id: String,
    pub action: String,
    pub kind: ProcedureStepKind,
    pub guide_anchor: String,
    pub need_ids: Vec<String>,
    #[serde(default)]
    pub source_unit_ids: Vec<String>,
    pub required_evidence: Vec<ProcedureEvidenceRole>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisualNeed {
    pub id: String,
    pub kind: NeedKind,
    #[serde(default)]
    pub source_unit_ids: Vec<String>,
    #[serde(default)]
    pub procedure_step_ids: Vec<String>,
    pub learner_question: String,
    pub misconception_prevented: String,
    pub evidence_requirement: EvidenceRequirement,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisualProvenance {
    #[serde(rename = "type")]
    pub kind: String,
    pub source: String,
    pub locator: String,
    pub transformation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RightsMetadata {
    pub basis: RightsBasis,
    pub reuse_scope: ReuseScope,
    pub attribution: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RightsBasis {
    CourseProvided,
    Licensed,
    PublicDomain,
    Permission,
    Original,
    Generated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReuseScope {
    PrivateStudy,
    Redistributable,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct VisualPacket {
    schema_version: u8,
    sources: Vec<PacketSource>,
    decision: VisualDecision,
    no_visuals_rationale: Option<String>,
    #[serde(default)]
    inputs: Vec<VisualInput>,
    #[serde(default)]
    needs: Vec<VisualNeed>,
    #[serde(default)]
    procedure_steps: Vec<ProcedureStepDefinition>,
    #[serde(default)]
    assets: Vec<VisualAssetSpec>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PacketSource {
    id: String,
    filename: String,
    sha256: String,
    role: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum VisualInput {
    SourceUnit {
        id: String,
        source_id: String,
        unit_id: String,
        provenance: VisualProvenance,
        rights: RightsMetadata,
    },
    TextbookPage {
        id: String,
        textbook_sha256: String,
        page_number: usize,
        provenance: VisualProvenance,
        rights: RightsMetadata,
    },
    ContextFrame {
        id: String,
        context_asset_id: String,
        provenance: VisualProvenance,
        rights: RightsMetadata,
    },
    LocalPng {
        id: String,
        path: String,
        sha256: String,
        width_px: u32,
        height_px: u32,
        evidence_class: EvidenceClass,
        provenance: VisualProvenance,
        rights: RightsMetadata,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct VisualAssetSpec {
    id: String,
    kind: VisualAssetKind,
    input_id: Option<String>,
    need_ids: Vec<String>,
    learning_purpose: String,
    alt: String,
    caption: String,
    explanation: String,
    #[serde(default)]
    procedure_step_ids: Vec<String>,
    #[serde(default)]
    crop: Option<NormRect>,
    #[serde(default)]
    annotations: Vec<Annotation>,
    #[serde(default)]
    diagram: Option<DiagramSpec>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct NormPoint {
    x: u16,
    y: u16,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NormRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum Annotation {
    Focus {
        rect: NormRect,
        tone: SemanticTone,
        label: String,
    },
    Arrow {
        from: NormPoint,
        to: NormPoint,
        tone: SemanticTone,
        label: String,
    },
    Callout {
        target: NormPoint,
        tone: SemanticTone,
        number: u8,
        text: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SemanticTone {
    Focus,
    Warning,
    Action,
    Expected,
    Wrong,
    Recovery,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum TeacherCalloutTone {
    Focus,
    Warning,
    Action,
    Expected,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TeacherCallout {
    pub x: u16,
    pub y: u16,
    pub number: u8,
    pub text: String,
    pub tone: TeacherCalloutTone,
}

#[derive(Clone, Debug)]
pub(crate) struct TeacherVisualPlan {
    pub source_unit_ids: Vec<String>,
    pub purposeful: bool,
    pub priority: u8,
    pub treatment: TeacherVisualTreatment,
    pub recommendation: String,
    pub callouts: Vec<TeacherCallout>,
    pub diagram: Option<DiagramSpec>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum TeacherVisualTreatment {
    None,
    SourceCrop,
    AnnotatedSource,
    DeterministicDiagram,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiagramSpec {
    pub width_px: u32,
    pub height_px: u32,
    pub title: String,
    pub nodes: Vec<DiagramNode>,
    pub edges: Vec<DiagramEdge>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiagramNode {
    pub id: String,
    pub rect: NormRect,
    pub label: String,
    pub tone: SemanticTone,
    pub shape: DiagramShape,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum DiagramShape {
    Box,
    RoundedBox,
    Circle,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiagramEdge {
    pub from: String,
    pub to: String,
    pub label: String,
    pub tone: SemanticTone,
}

#[derive(Clone)]
struct BoundInput {
    id: String,
    rgba: RgbaImage,
    evidence_class: EvidenceClass,
    source_unit_ids: Vec<String>,
    procedure_step_ids: Vec<String>,
    origin: InputOrigin,
    provenance: VisualProvenance,
    rights: RightsMetadata,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum InputOrigin {
    SourceUnit,
    TextbookPage,
    ContextFrame,
    LocalPng,
}

#[derive(Clone, Debug)]
struct RgbaImage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

struct VisualDecodeBudget {
    remaining: u64,
}

impl VisualDecodeBudget {
    fn new() -> Self {
        Self {
            remaining: MAX_VISUAL_DECODED_INPUT_BYTES,
        }
    }

    fn charge(&mut self, bytes: usize) -> Result<(), String> {
        self.remaining = self.remaining.checked_sub(bytes as u64).ok_or_else(|| {
            "visual inputs exceed the 256 MiB cumulative decoded-RGBA budget".to_string()
        })?;
        Ok(())
    }
}

pub fn packet_path(primary_source: &Path) -> Result<PathBuf, String> {
    let name = primary_source
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "primary source filename is not valid Unicode".to_string())?;
    Ok(primary_source.with_file_name(format!("{name}.guide-visuals.json")))
}

fn automatic_visual_packet(
    plan: &PlannedGuide,
    snapshot: &SourceSnapshot,
    inputs: VisualPreflightInputs<'_>,
    automatic_visual_limit: u64,
) -> Result<Vec<u8>, String> {
    let VisualPreflightInputs {
        sources,
        rendered,
        textbooks,
        context,
        context_procedure_steps,
    } = inputs;
    let sources_json = snapshot
        .sources
        .iter()
        .map(|source| {
            serde_json::json!({
                "id": source.id,
                "filename": source.name,
                "sha256": source.sha256,
                "role": source.role,
            })
        })
        .collect::<Vec<_>>();

    let packet = if plan.course.guide_mode == GuideMode::WeeklyLab {
        automatic_lab_visual_packet(sources_json, snapshot, context, context_procedure_steps)?
    } else {
        automatic_source_visual_packet(
            sources_json,
            sources,
            rendered,
            textbooks,
            automatic_visual_limit,
        )?
    };
    let mut bytes = serde_json::to_vec_pretty(&packet)
        .map_err(|error| format!("could not serialize automatic visual packet: {error}"))?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_PACKET_BYTES {
        return Err("automatic visual packet exceeds the 2 MiB packet limit".to_string());
    }
    Ok(bytes)
}

fn automatic_source_visual_packet(
    sources_json: Vec<serde_json::Value>,
    sources: &[CapturedSource],
    rendered: &[RenderedSourceSnapshot],
    textbooks: &[CapturedTextbook],
    automatic_visual_limit: u64,
) -> Result<serde_json::Value, String> {
    enum CandidateKind<'a> {
        Source(&'a CapturedSource),
        Textbook(&'a CapturedTextbook),
    }
    struct Candidate<'a> {
        kind: CandidateKind<'a>,
        unit_id: String,
        unit_number: usize,
        decoded_bytes: u64,
    }

    let mut candidates = Vec::new();
    let mut seen_images = HashSet::new();
    for source in sources {
        let images = rendered
            .iter()
            .find(|rendered_source| rendered_source.source_id == source.source_id)
            .ok_or_else(|| {
                format!(
                    "automatic visuals have no rendered source for {}",
                    source.source_id
                )
            })?;
        for (unit_id, image) in source.unit_ids.iter().zip(&images.images) {
            if seen_images.insert(image.sha256.as_str()) {
                candidates.push(Candidate {
                    kind: CandidateKind::Source(source),
                    unit_id: unit_id.clone(),
                    unit_number: image.number,
                    decoded_bytes: u64::from(image.width_px)
                        .checked_mul(u64::from(image.height_px))
                        .and_then(|pixels| pixels.checked_mul(4))
                        .ok_or_else(|| "automatic visual dimensions overflowed".to_string())?,
                });
            }
        }
    }
    for textbook in textbooks {
        for image in &textbook.rendered_pages {
            if seen_images.insert(image.sha256.as_str()) {
                candidates.push(Candidate {
                    kind: CandidateKind::Textbook(textbook),
                    unit_id: crate::source_context::textbook_visual_unit_id(textbook, image.number),
                    unit_number: image.number,
                    decoded_bytes: u64::from(image.width_px)
                        .checked_mul(u64::from(image.height_px))
                        .and_then(|pixels| pixels.checked_mul(4))
                        .ok_or_else(|| {
                            "automatic textbook visual dimensions overflowed".to_string()
                        })?,
                });
            }
        }
    }
    if candidates.is_empty() {
        let text_only_html = sources.iter().all(|source| {
            source
                .path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| {
                    value.eq_ignore_ascii_case("html") || value.eq_ignore_ascii_case("htm")
                })
        });
        if text_only_html {
            return Ok(serde_json::json!({
                "schema_version": 2,
                "sources": sources_json,
                "decision": "no-purposeful-visual",
                "no_visuals_rationale": "The selected HTML material is captured as visible text and has no trustworthy local render. Inventing a picture would add unsupported visual evidence; use a reviewed packet if the page contains an instructional figure.",
                "inputs": [],
                "needs": [],
                "procedure_steps": [],
                "assets": [],
            }));
        }
        return Err("automatic visuals require at least one rendered source unit".to_string());
    }

    let largest_candidate = candidates
        .iter()
        .map(|candidate| candidate.decoded_bytes)
        .max()
        .unwrap_or(0);
    let selected_count = automatic_visual_count(
        candidates.len(),
        largest_candidate,
        automatic_visual_limit.min(MAX_VISUAL_DECODED_INPUT_BYTES),
    )?;
    let selected = evenly_spaced_indices(candidates.len(), selected_count);
    let mut inputs = Vec::with_capacity(selected.len());
    let mut needs = Vec::with_capacity(selected.len());
    let mut assets = Vec::with_capacity(selected.len());
    for (position, candidate_index) in selected.into_iter().enumerate() {
        let candidate = &candidates[candidate_index];
        let ordinal = position + 1;
        let input_id = format!("auto-source-input-{ordinal:03}");
        let need_id = format!("auto-source-need-{ordinal:03}");
        let asset_id = format!("auto-source-visual-{ordinal:03}");
        let (input, source_unit_ids, learner_source) = match &candidate.kind {
            CandidateKind::Source(source) => {
                let learner_source = match source.unit_kind.as_str() {
                    "pdf-page" => format!("lecture PDF page {}", candidate.unit_number),
                    "pptx-slide" => format!("lecture slide {}", candidate.unit_number),
                    _ => format!("lecture section {}", candidate.unit_number),
                };
                (
                    serde_json::json!({
                        "kind": "source-unit",
                        "id": input_id,
                        "source_id": source.source_id,
                        "unit_id": candidate.unit_id,
                        "provenance": {
                            "type": "course-provided",
                            "source": source.path.to_string_lossy(),
                            "locator": rendered_source_locator(
                                &source.unit_kind,
                                candidate.unit_number,
                                &candidate.unit_id,
                            ),
                            "transformation": "Rendered locally by Guide Watcher from the frozen course source."
                        },
                        "rights": {
                            "basis": "course-provided",
                            "reuse_scope": "private-study",
                            "attribution": "Course-provided source material"
                        }
                    }),
                    vec![candidate.unit_id.clone()],
                    learner_source,
                )
            }
            CandidateKind::Textbook(textbook) => {
                let learner_source = format!(
                    "{} (PDF page {})",
                    canonical_learner_text(&textbook.display_title, 160, "course textbook"),
                    candidate.unit_number
                );
                (
                    serde_json::json!({
                        "kind": "textbook-page",
                        "id": input_id,
                        "textbook_sha256": textbook.dependency.sha256,
                        "page_number": candidate.unit_number,
                        "provenance": {
                            "type": "course-provided",
                            "source": textbook.dependency.path.to_string_lossy(),
                            "locator": format!("PDF page {}", candidate.unit_number),
                            "transformation": "Rendered locally by Guide Watcher from the frozen textbook."
                        },
                        "rights": {
                            "basis": "course-provided",
                            "reuse_scope": "private-study",
                            "attribution": textbook.display_title
                        }
                    }),
                    Vec::new(),
                    learner_source,
                )
            }
        };
        inputs.push(input);
        needs.push(serde_json::json!({
            "id": need_id,
            "kind": "concept",
            "source_unit_ids": source_unit_ids,
            "procedure_step_ids": [],
            "learner_question": format!("What visual information in {learner_source} helps explain the concept?"),
            "misconception_prevented": format!("Ignoring labels, spatial relationships, or emphasis shown in {learner_source}."),
            "evidence_requirement": "source"
        }));
        assets.push(serde_json::json!({
            "id": asset_id,
            "kind": "source-crop",
            "input_id": input_id,
            "need_ids": [need_id],
            "learning_purpose": format!("Use the visible labels and layout in {learner_source} to understand how the depicted parts relate."),
            "alt": format!("{learner_source}, showing the concept's visible labels and layout."),
            "caption": learner_source,
            "explanation": "The labels and spatial arrangement show how the visible parts of the concept relate; compare that relationship with the guide explanation.",
            "procedure_step_ids": [],
            "annotations": []
        }));
    }

    Ok(serde_json::json!({
        "schema_version": 2,
        "sources": sources_json,
        "decision": "purposeful-visuals",
        "inputs": inputs,
        "needs": needs,
        "procedure_steps": [],
        "assets": assets,
    }))
}

fn rendered_source_locator(unit_kind: &str, number: usize, unit_id: &str) -> String {
    let location = match unit_kind {
        "pdf-page" => "page",
        "pptx-slide" => "slide",
        _ => "section",
    };
    format!("app-rendered {location} {number} (source unit {unit_id})")
}

fn automatic_visual_count(
    candidate_count: usize,
    largest_decoded_bytes: u64,
    decoded_budget: u64,
) -> Result<usize, String> {
    if candidate_count == 0 || largest_decoded_bytes == 0 {
        return Err("automatic visuals require nonempty rendered image dimensions".to_string());
    }
    let budget_count = decoded_budget / largest_decoded_bytes;
    let count = candidate_count
        .min(MAX_AUTOMATIC_VISUALS)
        .min(usize::try_from(budget_count).unwrap_or(usize::MAX));
    if count == 0 {
        return Err(
            "automatic visual budget cannot admit one frozen source-unit render".to_string(),
        );
    }
    Ok(count)
}

fn automatic_lab_visual_packet(
    sources_json: Vec<serde_json::Value>,
    snapshot: &SourceSnapshot,
    context: &[CourseContextAsset],
    context_procedure_steps: &[CourseContextProcedureStep],
) -> Result<serde_json::Value, String> {
    let source_unit = snapshot
        .unit_ids
        .first()
        .ok_or_else(|| "Circuit Lab automatic visuals require a source unit".to_string())?;
    if context.is_empty() {
        return Err("Circuit Lab requires captured LMS video frames or a reviewed visual packet; the source-bound context sidecar contains no usable frames".to_string());
    }

    if context_procedure_steps.is_empty() {
        return Err("Circuit Lab context must supply an ordered procedure-step inventory before automatic annotated visuals can be prepared".to_string());
    }
    if context_procedure_steps.len() > MAX_NEEDS {
        return Err("Circuit Lab sidecar has too many distinct procedure steps".to_string());
    }
    let mut seen_steps = HashSet::new();
    for step in context_procedure_steps {
        validate_id(&step.id, "Circuit Lab sidecar procedure step")?;
        require_learner_text(&step.action, "Circuit Lab procedure action", 600)?;
        if !seen_steps.insert(step.id.as_str()) {
            return Err("Circuit Lab sidecar has duplicate procedure-step IDs".to_string());
        }
    }
    for frame in context {
        for step in &frame.procedure_step_ids {
            if !seen_steps.contains(step.as_str()) {
                return Err(
                    "Circuit Lab frame references a procedure step outside the ordered inventory"
                        .to_string(),
                );
            }
        }
    }

    let mut inputs = Vec::with_capacity(context.len());
    let mut input_ids = HashMap::new();
    for (index, frame) in context.iter().enumerate() {
        let input_id = format!("auto-frame-input-{:03}", index + 1);
        input_ids.insert(frame.id.as_str(), input_id.clone());
        inputs.push(serde_json::json!({
            "kind": "context-frame",
            "id": input_id,
            "context_asset_id": frame.id,
            "provenance": {
                "type": "course-provided-video-frame",
                "source": frame.video_title,
                "locator": format!("frame {} at {:02}:{:02}", frame.id, frame.timestamp_seconds / 60, frame.timestamp_seconds % 60),
                "transformation": "Captured from the course video and annotated deterministically by Guide Watcher."
            },
            "rights": {
                "basis": "course-provided",
                "reuse_scope": "private-study",
                "attribution": "DGIST Circuit Lab course video"
            }
        }));
    }

    let needs = context_procedure_steps
        .iter()
        .map(|step| {
            let need_id = format!("auto-need-{}", step.id);
            serde_json::json!({
                "id": need_id,
                "kind": "procedure-step",
                "source_unit_ids": [source_unit],
                "procedure_step_ids": [step.id],
                "learner_question": format!("What exact visual state should I match while I {}?", step.action),
                "misconception_prevented": format!("Performing Circuit Lab step {} from text alone without checking the course video state.", step.id),
                "evidence_requirement": "source"
            })
        })
        .collect::<Vec<_>>();
    let procedure_steps = context_procedure_steps
        .iter()
        .map(|step| {
            let (kind, required_evidence) = match step.kind {
                CourseContextProcedureStepKind::Physical => ("physical", vec!["action"]),
                CourseContextProcedureStepKind::Conceptual => ("conceptual", Vec::new()),
            };
            serde_json::json!({
                "id": step.id,
                "action": step.action,
                "kind": kind,
                "guide_anchor": format!("procedure-{}", step.id),
                "need_ids": [format!("auto-need-{}", step.id)],
                "source_unit_ids": [source_unit],
                "required_evidence": required_evidence
            })
        })
        .collect::<Vec<_>>();

    let mut placements = Vec::new();
    let mut used_frames = HashSet::new();
    for step in context_procedure_steps {
        let frame = context
            .iter()
            .find(|frame| frame.procedure_step_ids.contains(&step.id) && !frame.annotations.is_empty())
            .ok_or_else(|| {
                format!(
                    "Circuit Lab step {} has no content-aware annotation target in its captured frames",
                    step.id
                )
            })?;
        used_frames.insert(frame.id.as_str());
        placements.push((frame, step.id.as_str()));
    }
    for frame in context {
        if placements.len() >= MAX_ASSETS || used_frames.contains(frame.id.as_str()) {
            continue;
        }
        if !frame.annotations.is_empty() {
            if let Some(step) = frame.procedure_step_ids.first() {
                placements.push((frame, step.as_str()));
            }
        }
    }

    let mut assets = Vec::with_capacity(placements.len());
    for (index, (frame, step)) in placements.into_iter().enumerate() {
        let ordinal = index + 1;
        let input_id = input_ids
            .get(frame.id.as_str())
            .expect("every context frame received an automatic input");
        let alt = canonical_learner_text(
            &frame.alt,
            220,
            &format!("Circuit Lab video frame for step {step}"),
        );
        let caption = canonical_learner_text(
            &frame.caption,
            300,
            &format!("Course video evidence for Circuit Lab step {step}."),
        );
        let explanation = canonical_learner_text(
            &frame.what_to_notice,
            620,
            &format!("Match the shown state before performing Circuit Lab step {step}."),
        );
        let annotations = frame
            .annotations
            .iter()
            .enumerate()
            .map(|(annotation_index, annotation)| {
                serde_json::json!({
                    "kind": "callout",
                    "target": { "x": annotation.target_x, "y": annotation.target_y },
                    "tone": "action",
                    "number": (annotation_index % 99) + 1,
                    "text": printable_ascii(
                        &annotation.label,
                        260,
                        &format!("Match this state for step {step}"),
                    )
                })
            })
            .collect::<Vec<_>>();
        assets.push(serde_json::json!({
            "id": format!("auto-lab-visual-{ordinal:03}"),
            "kind": "annotated-source",
            "input_id": input_id,
            "need_ids": [format!("auto-need-{step}")],
            "learning_purpose": format!("Show course-video evidence from frame {} for Circuit Lab step {step}.", frame.id),
            "alt": alt,
            "caption": caption,
            "explanation": explanation,
            "procedure_step_ids": [step],
            "annotations": annotations
        }));
    }

    Ok(serde_json::json!({
        "schema_version": 2,
        "sources": sources_json,
        "decision": "purposeful-visuals",
        "inputs": inputs,
        "needs": needs,
        "procedure_steps": procedure_steps,
        "assets": assets,
    }))
}

fn evenly_spaced_indices(length: usize, limit: usize) -> Vec<usize> {
    if length <= limit {
        return (0..length).collect();
    }
    if limit <= 1 {
        return vec![0];
    }
    (0..limit)
        .map(|position| position * (length - 1) / (limit - 1))
        .collect()
}

fn canonical_learner_text(value: &str, max_bytes: usize, fallback: &str) -> String {
    let cleaned = value
        .chars()
        .map(|character| {
            if character.is_control() || "\\[]<>&`*_".contains(character) {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let selected = if collapsed.is_empty() {
        fallback.split_whitespace().collect::<Vec<_>>().join(" ")
    } else {
        collapsed
    };
    truncate_utf8(&selected, max_bytes)
}

fn printable_ascii(value: &str, max_bytes: usize, fallback: &str) -> String {
    let cleaned = value
        .chars()
        .map(|character| {
            if character.is_ascii() && !character.is_control() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let selected = if collapsed.is_empty() {
        fallback.split_whitespace().collect::<Vec<_>>().join(" ")
    } else {
        collapsed
    };
    truncate_utf8(&selected, max_bytes)
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].trim_end().to_string()
}

pub struct VisualPreflightInputs<'a> {
    pub sources: &'a [CapturedSource],
    pub rendered: &'a [RenderedSourceSnapshot],
    pub textbooks: &'a [CapturedTextbook],
    pub context: &'a [CourseContextAsset],
    pub context_procedure_steps: &'a [CourseContextProcedureStep],
}

struct VisualCompilationRequest<'a> {
    plan: &'a PlannedGuide,
    snapshot: &'a SourceSnapshot,
    inputs: VisualPreflightInputs<'a>,
    packet_path: PathBuf,
    packet_bytes: Vec<u8>,
    packet_dependency: Option<CapturedDependency>,
}

pub(crate) struct AutomaticTeacherPlanInput<'a> {
    pub plan: &'a PlannedGuide,
    pub snapshot: &'a SourceSnapshot,
    pub inputs: VisualPreflightInputs<'a>,
    pub current: VisualMaterial,
    pub teacher_plans: &'a [TeacherVisualPlan],
}

#[cfg(test)]
pub fn preflight_visual_material(
    plan: &PlannedGuide,
    snapshot: &SourceSnapshot,
    inputs: VisualPreflightInputs<'_>,
    resource_budget: &mut PreflightResourceBudget,
    cancellation: crate::process_registry::CancellationToken,
) -> Result<VisualMaterial, String> {
    let automatic_visual_limit = resource_budget.remaining_visual_decoded();
    preflight_visual_material_with_automatic_budget(
        plan,
        snapshot,
        inputs,
        resource_budget,
        automatic_visual_limit,
        cancellation,
    )
}

pub(crate) fn preflight_visual_material_with_automatic_budget(
    plan: &PlannedGuide,
    snapshot: &SourceSnapshot,
    inputs: VisualPreflightInputs<'_>,
    resource_budget: &mut PreflightResourceBudget,
    automatic_visual_limit: u64,
    cancellation: crate::process_registry::CancellationToken,
) -> Result<VisualMaterial, String> {
    let VisualPreflightInputs {
        sources,
        rendered,
        textbooks,
        context,
        context_procedure_steps,
    } = inputs;
    ensure_visual_preflight_current(cancellation)?;
    let packet_path = packet_path(&plan.primary_source)?;
    let (packet_bytes, packet_dependency) = match std::fs::symlink_metadata(&packet_path) {
        Ok(_) => {
            let (bytes, dependency) = capture_visual_file(
                &packet_path,
                MAX_PACKET_BYTES,
                "visual packet",
                resource_budget,
            )
            .map_err(|error| {
                format!(
                    "could not load reviewed visual packet {}: {error}",
                    packet_path.display()
                )
            })?;
            (bytes, Some(dependency))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (
            automatic_visual_packet(
                plan,
                snapshot,
                VisualPreflightInputs {
                    sources,
                    rendered,
                    textbooks,
                    context,
                    context_procedure_steps,
                },
                automatic_visual_limit,
            )?,
            None,
        ),
        Err(error) => {
            return Err(format!(
                "could not inspect visual packet path {}: {error}",
                packet_path.display()
            ));
        }
    };
    compile_visual_material_from_packet(
        VisualCompilationRequest {
            plan,
            snapshot,
            inputs: VisualPreflightInputs {
                sources,
                rendered,
                textbooks,
                context,
                context_procedure_steps,
            },
            packet_path,
            packet_bytes,
            packet_dependency,
        },
        resource_budget,
        cancellation,
    )
}

fn compile_visual_material_from_packet(
    request: VisualCompilationRequest<'_>,
    resource_budget: &mut PreflightResourceBudget,
    cancellation: crate::process_registry::CancellationToken,
) -> Result<VisualMaterial, String> {
    let VisualCompilationRequest {
        plan,
        snapshot,
        inputs,
        packet_path,
        packet_bytes,
        packet_dependency,
    } = request;
    let VisualPreflightInputs {
        sources,
        rendered,
        textbooks,
        context,
        ..
    } = inputs;
    ensure_visual_preflight_current(cancellation)?;
    let packet_value: serde_json::Value = serde_json::from_slice(&packet_bytes)
        .map_err(|error| format!("visual packet is not JSON: {error}"))?;
    validate_packet_keys(&packet_value)?;
    let packet: VisualPacket = serde_json::from_value(packet_value)
        .map_err(|error| format!("visual packet is not strict schema-2 JSON: {error}"))?;
    validate_packet_shape(&packet, plan, snapshot)?;

    let mut dependencies = packet_dependency.into_iter().collect::<Vec<_>>();
    let local_root = plan.primary_source.with_file_name(format!(
        "{}.guide-visuals",
        plan.primary_source
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| "primary source filename is not valid Unicode".to_string())?
    ));
    let mut inputs = HashMap::new();
    let mut decoded_input_budget = VisualDecodeBudget::new();
    {
        let mut binding = VisualInputBindingContext {
            local_root: &local_root,
            sources,
            rendered,
            textbooks,
            context,
            dependencies: &mut dependencies,
            resource_budget,
            decoded_input_budget: &mut decoded_input_budget,
        };
        for input in &packet.inputs {
            ensure_visual_preflight_current(cancellation)?;
            let bound = bind_input(input, &mut binding)?;
            if inputs.insert(bound.id.clone(), bound).is_some() {
                return Err("visual input IDs must be unique".to_string());
            }
        }
    }

    let needs = packet
        .needs
        .iter()
        .map(|need| (need.id.as_str(), need))
        .collect::<HashMap<_, _>>();
    let mut compiled_assets = Vec::with_capacity(packet.assets.len());
    let mut seen_ids = HashSet::new();
    let mut seen_bytes = HashSet::new();
    let mut seen_specs = HashSet::new();
    let mut seen_purposes: HashMap<String, String> = HashMap::new();
    for spec in &packet.assets {
        ensure_visual_preflight_current(cancellation)?;
        validate_id(&spec.id, "visual asset")?;
        if !seen_ids.insert(spec.id.clone()) {
            return Err(format!("visual asset ID is duplicated: {}", spec.id));
        }
        validate_asset_text(spec)?;
        let referenced_needs =
            spec.need_ids
                .iter()
                .map(|id| {
                    needs.get(id.as_str()).copied().ok_or_else(|| {
                        format!("visual asset {} references unknown need {id}", spec.id)
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
        if referenced_needs.is_empty() {
            return Err(format!(
                "visual asset {} must serve at least one need",
                spec.id
            ));
        }
        let input = match spec.kind {
            VisualAssetKind::DeterministicDiagram => {
                if spec.input_id.is_some() {
                    return Err(format!("diagram {} cannot have input_id", spec.id));
                }
                None
            }
            _ => Some(
                inputs
                    .get(
                        spec.input_id
                            .as_deref()
                            .ok_or_else(|| format!("visual asset {} requires input_id", spec.id))?,
                    )
                    .ok_or_else(|| format!("visual asset {} references unknown input", spec.id))?,
            ),
        };
        validate_asset_semantics(
            spec,
            input,
            &referenced_needs,
            &packet.procedure_steps,
            plan,
        )?;
        let output = render_asset(spec, input, resource_budget, cancellation)?;
        ensure_visual_preflight_current(cancellation)?;
        let bytes = encode_png(&output)?;
        resource_budget.charge_rendered(bytes.len() as u64, "compiled learner visuals")?;
        let decoded = decode_png_rgba(&bytes)?;
        if decoded.width != output.width || decoded.height != output.height {
            return Err("deterministic visual failed its decode/dimension check".to_string());
        }
        let sha256 = digest(&bytes);
        let spec_sha256 = digest(
            &serde_json::to_vec(spec)
                .map_err(|error| format!("could not hash visual specification: {error}"))?,
        );
        if !seen_bytes.insert(sha256.clone()) {
            return Err("two visual assets compile to duplicate bytes".to_string());
        }
        if !seen_specs.insert(spec_sha256.clone()) {
            return Err("two visual assets use a duplicate rendering specification".to_string());
        }
        if let Some(earlier) =
            seen_purposes.insert(spec.learning_purpose.trim().to_lowercase(), spec.id.clone())
        {
            return Err(format!(
                "two visual assets claim the same learning purpose ({earlier} and {}): {}",
                spec.id,
                spec.learning_purpose.trim()
            ));
        }
        let mut source_unit_ids = referenced_needs
            .iter()
            .flat_map(|need| need.source_unit_ids.iter().cloned())
            .collect::<Vec<_>>();
        source_unit_ids.sort();
        source_unit_ids.dedup();
        let procedure_step_ids = spec.procedure_step_ids.clone();
        let (provenance, rights, evidence_class) = match input {
            Some(input) => (
                output_provenance(spec.kind, &input.provenance),
                input.rights.clone(),
                input.evidence_class,
            ),
            None => (
                VisualProvenance {
                    kind: "deterministic-diagram".to_string(),
                    source: "Guide Watcher bounded diagram DSL".to_string(),
                    locator: "not applicable - original deterministic diagram".to_string(),
                    transformation: "Rasterized deterministically by the app from a bounded declarative specification.".to_string(),
                },
                RightsMetadata {
                    basis: RightsBasis::Original,
                    reuse_scope: ReuseScope::Redistributable,
                    attribution: "Original Guide Watcher diagram".to_string(),
                },
                EvidenceClass::Explanatory,
            ),
        };
        compiled_assets.push(CompiledVisualAsset {
            catalog: VisualCatalogEntry {
                id: spec.id.clone(),
                filename: format!("{}.png", spec.id),
                sha256,
                spec_sha256,
                width_px: output.width,
                height_px: output.height,
                kind: spec.kind,
                evidence_class,
                need_ids: spec.need_ids.clone(),
                source_unit_ids,
                procedure_step_ids,
                learning_purpose: spec.learning_purpose.clone(),
                alt: spec.alt.clone(),
                caption: spec.caption.clone(),
                explanation: spec.explanation.clone(),
                provenance,
                rights,
            },
            bytes,
        });
    }
    validate_need_coverage(&packet, &compiled_assets, plan)?;
    let packet_sha256 = digest(&packet_bytes);
    Ok(VisualMaterial {
        packet_path,
        packet_bytes,
        packet_sha256: packet_sha256.clone(),
        dependencies,
        contract: VisualContract {
            schema_version: 2,
            packet_sha256,
            decision: packet.decision,
            no_visuals_rationale: packet.no_visuals_rationale,
            needs: packet.needs,
            procedure_steps: packet.procedure_steps,
            assets: compiled_assets
                .iter()
                .map(|asset| asset.catalog.clone())
                .collect(),
        },
        compiled_assets,
    })
}

pub(crate) fn apply_automatic_teacher_plan(
    input: AutomaticTeacherPlanInput<'_>,
    cancellation: crate::process_registry::CancellationToken,
) -> Result<VisualMaterial, String> {
    let AutomaticTeacherPlanInput {
        plan,
        snapshot,
        inputs,
        current,
        teacher_plans,
    } = input;
    let VisualPreflightInputs {
        sources,
        rendered,
        textbooks,
        context,
        context_procedure_steps,
    } = inputs;
    if plan.course.guide_mode == GuideMode::WeeklyLab
        || current.dependencies.iter().any(|dependency| {
            dependency.requested_path.as_deref() == Some(current.packet_path.as_path())
        })
    {
        return Ok(current);
    }
    if std::fs::symlink_metadata(&current.packet_path).is_ok() {
        return Err(format!(
            "a reviewed visual packet appeared after automatic preflight: {}",
            current.packet_path.display()
        ));
    }
    if current.contract.decision == VisualDecision::NoPurposefulVisual {
        return Ok(current);
    }
    let visual_limit = current.compiled_assets.len();
    if visual_limit == 0 {
        return Err("automatic visual preflight produced no usable visual capacity".to_string());
    }
    let selected =
        select_teacher_visuals(sources, rendered, textbooks, teacher_plans, visual_limit)?;
    let packet_value: serde_json::Value = serde_json::from_slice(&current.packet_bytes)
        .map_err(|error| format!("automatic visual packet is no longer valid JSON: {error}"))?;
    validate_packet_keys(&packet_value)?;
    let mut packet: VisualPacket = serde_json::from_value(packet_value)
        .map_err(|error| format!("automatic visual packet is no longer strict JSON: {error}"))?;
    packet.inputs.clear();
    packet.needs.clear();
    packet.procedure_steps.clear();
    packet.assets.clear();
    if selected.is_empty() {
        packet.decision = VisualDecision::NoPurposefulVisual;
        packet.no_visuals_rationale = Some(
            "Sol inspected every frozen source render and found no image whose labels, geometry, sequence, or visible state would teach more clearly than the guide text."
                .to_string(),
        );
    } else {
        packet.decision = VisualDecision::PurposefulVisuals;
        packet.no_visuals_rationale = None;
    }
    for (position, selection) in selected.iter().enumerate() {
        let ordinal = position + 1;
        let input_id = format!("teacher-source-input-{ordinal:03}");
        let need_id = format!("teacher-source-need-{ordinal:03}");
        let asset_id = format!("teacher-source-visual-{ordinal:03}");
        let learner_source = &selection.learner_source;
        let recommendation = canonical_learner_text(
            &selection.teacher_plan.recommendation,
            240,
            "Keep the source image and point out its most important visible relationship.",
        );
        let provenance = VisualProvenance {
            kind: "course-provided".to_string(),
            source: selection.source_path.clone(),
            locator: selection.source_locator.clone(),
            transformation: "Rendered locally by Guide Watcher from frozen study material."
                .to_string(),
        };
        let visual_input = match &selection.input {
            RankedTeacherVisualInput::Source { source_id } => VisualInput::SourceUnit {
                id: input_id.clone(),
                source_id: source_id.clone(),
                unit_id: selection.unit_id.clone(),
                provenance,
                rights: RightsMetadata {
                    basis: RightsBasis::CourseProvided,
                    reuse_scope: ReuseScope::PrivateStudy,
                    attribution: "Course-provided lecture material".to_string(),
                },
            },
            RankedTeacherVisualInput::Textbook {
                textbook_sha256,
                page_number,
                title,
            } => VisualInput::TextbookPage {
                id: input_id.clone(),
                textbook_sha256: textbook_sha256.clone(),
                page_number: *page_number,
                provenance,
                rights: RightsMetadata {
                    basis: RightsBasis::CourseProvided,
                    reuse_scope: ReuseScope::PrivateStudy,
                    attribution: title.clone(),
                },
            },
        };
        packet.inputs.push(visual_input);
        let source_unit_ids = match &selection.input {
            RankedTeacherVisualInput::Source { .. } => vec![selection.unit_id.clone()],
            RankedTeacherVisualInput::Textbook { .. } => Vec::new(),
        };
        packet.needs.push(VisualNeed {
            id: need_id.clone(),
            kind: NeedKind::Concept,
            source_unit_ids,
            procedure_step_ids: Vec::new(),
            learner_question: format!(
                "Which visible relationship in {learner_source} is essential to understanding the concept?"
            ),
            misconception_prevented: format!(
                "Reading {learner_source} as decoration instead of evidence with meaningful labels and spatial relationships."
            ),
            evidence_requirement: EvidenceRequirement::Source,
        });
        packet.assets.push(VisualAssetSpec {
            id: asset_id,
            kind: VisualAssetKind::SourceCrop,
            input_id: Some(input_id),
            need_ids: vec![need_id],
            learning_purpose: format!(
                "Use {learner_source} to follow the visible relationship explained below."
            ),
            alt: format!("{learner_source}, showing the relationship described in the guide."),
            caption: learner_source.clone(),
            explanation: recommendation,
            procedure_step_ids: Vec::new(),
            crop: None,
            annotations: Vec::new(),
            diagram: None,
        });
    }
    let mut packet_bytes = serde_json::to_vec_pretty(&packet)
        .map_err(|error| format!("could not serialize teacher-annotated visual packet: {error}"))?;
    packet_bytes.push(b'\n');
    if packet_bytes.len() as u64 > MAX_PACKET_BYTES {
        return Err("teacher-annotated visual packet exceeds the 2 MiB limit".to_string());
    }
    let packet_path = current.packet_path.clone();
    drop(current);
    let mut resource_budget = PreflightResourceBudget::new();
    compile_visual_material_from_packet(
        VisualCompilationRequest {
            plan,
            snapshot,
            inputs: VisualPreflightInputs {
                sources,
                rendered,
                textbooks,
                context,
                context_procedure_steps,
            },
            packet_path,
            packet_bytes,
            packet_dependency: None,
        },
        &mut resource_budget,
        cancellation,
    )
}

#[derive(Clone, Debug)]
enum RankedTeacherVisualInput {
    Source {
        source_id: String,
    },
    Textbook {
        textbook_sha256: String,
        page_number: usize,
        title: String,
    },
}

#[derive(Clone, Debug)]
struct RankedTeacherVisual {
    input: RankedTeacherVisualInput,
    source_path: String,
    source_locator: String,
    learner_source: String,
    unit_id: String,
    source_order: usize,
    teacher_plan: TeacherVisualPlan,
}

fn select_teacher_visuals(
    sources: &[CapturedSource],
    rendered: &[RenderedSourceSnapshot],
    textbooks: &[CapturedTextbook],
    teacher_plans: &[TeacherVisualPlan],
    limit: usize,
) -> Result<Vec<RankedTeacherVisual>, String> {
    let mut plans_by_unit = HashMap::new();
    for teacher_plan in teacher_plans {
        if teacher_plan.source_unit_ids.len() != 1 {
            continue;
        }
        let unit_id = teacher_plan.source_unit_ids[0].as_str();
        if plans_by_unit.insert(unit_id, teacher_plan).is_some() {
            return Err(format!(
                "teacher visual plan contains duplicate source-unit binding: {unit_id}"
            ));
        }
    }

    let mut candidates = Vec::new();
    let mut seen_images = HashSet::new();
    let mut source_order = 0usize;
    for source in sources {
        let rendered_source = rendered
            .iter()
            .find(|candidate| candidate.source_id == source.source_id)
            .ok_or_else(|| {
                format!(
                    "teacher visual selection has no rendered source for {}",
                    source.source_id
                )
            })?;
        if rendered_source.images.len() != source.unit_ids.len() {
            return Err(format!(
                "teacher visual selection render count does not match source units for {}",
                source.source_id
            ));
        }
        for (unit_id, image) in source.unit_ids.iter().zip(&rendered_source.images) {
            if !seen_images.insert(image.sha256.as_str()) {
                continue;
            }
            let teacher_plan = plans_by_unit.get(unit_id.as_str()).ok_or_else(|| {
                format!(
                    "Sol visual inspection did not produce a plan for frozen source unit {unit_id}"
                )
            })?;
            validate_teacher_visual_plan(teacher_plan, unit_id)?;
            if teacher_plan.purposeful {
                candidates.push(RankedTeacherVisual {
                    input: RankedTeacherVisualInput::Source {
                        source_id: source.source_id.clone(),
                    },
                    source_path: source.path.to_string_lossy().to_string(),
                    source_locator: rendered_source_locator(
                        &source.unit_kind,
                        image.number,
                        unit_id,
                    ),
                    learner_source: match source.unit_kind.as_str() {
                        "pdf-page" => format!("lecture PDF page {}", image.number),
                        "pptx-slide" => format!("lecture slide {}", image.number),
                        _ => format!("lecture section {}", image.number),
                    },
                    unit_id: unit_id.clone(),
                    source_order,
                    teacher_plan: (*teacher_plan).clone(),
                });
            }
            source_order = source_order.saturating_add(1);
        }
    }
    for textbook in textbooks {
        for image in &textbook.rendered_pages {
            if !seen_images.insert(image.sha256.as_str()) {
                continue;
            }
            let unit_id = crate::source_context::textbook_visual_unit_id(textbook, image.number);
            let teacher_plan = plans_by_unit.get(unit_id.as_str()).ok_or_else(|| {
                format!(
                    "Sol visual inspection did not produce a plan for frozen textbook page {unit_id}"
                )
            })?;
            validate_teacher_visual_plan(teacher_plan, &unit_id)?;
            if teacher_plan.purposeful {
                let title = canonical_learner_text(&textbook.display_title, 160, "course textbook");
                candidates.push(RankedTeacherVisual {
                    input: RankedTeacherVisualInput::Textbook {
                        textbook_sha256: textbook.dependency.sha256.clone(),
                        page_number: image.number,
                        title: title.clone(),
                    },
                    source_path: textbook.dependency.path.to_string_lossy().to_string(),
                    source_locator: format!("PDF page {}", image.number),
                    learner_source: format!("{title} (PDF page {})", image.number),
                    unit_id,
                    source_order,
                    teacher_plan: (*teacher_plan).clone(),
                });
            }
            source_order = source_order.saturating_add(1);
        }
    }
    candidates.sort_by(|left, right| {
        right
            .teacher_plan
            .priority
            .cmp(&left.teacher_plan.priority)
            .then_with(|| left.source_order.cmp(&right.source_order))
    });
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut selected_indices = (0..candidates.len().min(limit)).collect::<Vec<_>>();
    if let Some(textbook_index) = candidates
        .iter()
        .position(|candidate| matches!(&candidate.input, RankedTeacherVisualInput::Textbook { .. }))
    {
        if !selected_indices.contains(&textbook_index) {
            selected_indices.pop();
            selected_indices.push(textbook_index);
            selected_indices.sort_unstable();
        }
    }
    Ok(selected_indices
        .into_iter()
        .map(|index| candidates[index].clone())
        .collect())
}

pub(crate) fn validate_teacher_visual_plan(
    plan: &TeacherVisualPlan,
    unit_id: &str,
) -> Result<(), String> {
    if plan.priority > 100 {
        return Err(format!(
            "teacher visual priority for {unit_id} must be between 0 and 100"
        ));
    }
    require_text(&plan.recommendation, "teacher visual recommendation")?;
    match (plan.purposeful, plan.treatment) {
        (false, TeacherVisualTreatment::None) => {
            if !plan.callouts.is_empty() || plan.diagram.is_some() {
                return Err(format!(
                    "non-purposeful teacher visual plan for {unit_id} cannot contain rendering instructions"
                ));
            }
        }
        (true, TeacherVisualTreatment::AnnotatedSource) => {
            validate_teacher_callouts(&plan.callouts, unit_id)?;
            if plan.diagram.is_some() {
                return Err(format!(
                    "annotated-source teacher plan for {unit_id} cannot contain a diagram"
                ));
            }
        }
        (true, TeacherVisualTreatment::SourceCrop) => {
            if !plan.callouts.is_empty() || plan.diagram.is_some() {
                return Err(format!(
                    "source-crop teacher plan for {unit_id} cannot contain overlays or a diagram"
                ));
            }
        }
        (true, TeacherVisualTreatment::DeterministicDiagram) => {
            if !plan.callouts.is_empty() {
                return Err(format!(
                    "deterministic diagram teacher plan for {unit_id} cannot contain source callouts"
                ));
            }
            let diagram = plan.diagram.as_ref().ok_or_else(|| {
                format!("deterministic diagram teacher plan for {unit_id} requires a diagram")
            })?;
            validate_diagram(diagram)?;
            if diagram.nodes.len() > 8 {
                return Err(format!(
                    "automatic teacher diagram for {unit_id} cannot exceed eight nodes"
                ));
            }
            for (index, left) in diagram.nodes.iter().enumerate() {
                for right in diagram.nodes.iter().skip(index + 1) {
                    if normalized_rectangles_overlap(left.rect, right.rect) {
                        return Err(format!(
                            "automatic teacher diagram for {unit_id} has overlapping nodes {} and {}",
                            left.id, right.id
                        ));
                    }
                }
            }
        }
        _ => {
            return Err(format!(
                "teacher visual purpose and treatment disagree for {unit_id}"
            ));
        }
    }
    Ok(())
}

fn normalized_rectangles_overlap(left: NormRect, right: NormRect) -> bool {
    let left_right = u32::from(left.x) + u32::from(left.width);
    let right_right = u32::from(right.x) + u32::from(right.width);
    let left_bottom = u32::from(left.y) + u32::from(left.height);
    let right_bottom = u32::from(right.y) + u32::from(right.height);
    u32::from(left.x) < right_right
        && u32::from(right.x) < left_right
        && u32::from(left.y) < right_bottom
        && u32::from(right.y) < left_bottom
}

fn validate_teacher_callouts(callouts: &[TeacherCallout], unit_id: &str) -> Result<(), String> {
    if callouts.is_empty() || callouts.len() > 2 {
        return Err(format!(
            "teacher visual plan for {unit_id} must contain one or two callouts"
        ));
    }
    for (index, callout) in callouts.iter().enumerate() {
        if callout.number as usize != index + 1
            || !(500..=9_500).contains(&callout.x)
            || !(500..=9_500).contains(&callout.y)
        {
            return Err(format!(
                "teacher callout {} for {unit_id} has an invalid number or normalized target",
                index + 1
            ));
        }
        require_ascii_text(&callout.text, "teacher callout text", 60)?;
    }
    Ok(())
}

fn ensure_visual_preflight_current(
    cancellation: crate::process_registry::CancellationToken,
) -> Result<(), String> {
    if crate::process_registry::is_cancelled(cancellation) {
        Err("guide preflight was cancelled during visual compilation".to_string())
    } else {
        Ok(())
    }
}

pub fn recheck_visual_material(material: &VisualMaterial) -> Result<(), String> {
    for dependency in &material.dependencies {
        crate::source_context::recheck_dependency(dependency).map_err(|error| {
            format!(
                "visual dependency changed after whole-batch preflight ({}): {error}",
                dependency.path.display()
            )
        })?;
    }
    for asset in &material.compiled_assets {
        let decoded = decode_png_rgba(&asset.bytes)?;
        if digest(&asset.bytes) != asset.catalog.sha256
            || decoded.width != asset.catalog.width_px
            || decoded.height != asset.catalog.height_px
        {
            return Err(format!(
                "compiled visual changed after preflight: {}",
                asset.catalog.id
            ));
        }
    }
    Ok(())
}

fn validate_packet_shape(
    packet: &VisualPacket,
    plan: &PlannedGuide,
    snapshot: &SourceSnapshot,
) -> Result<(), String> {
    if packet.schema_version != 2 {
        return Err("visual packet schema_version must be 2".to_string());
    }
    if packet.inputs.len() > MAX_INPUTS
        || packet.needs.len() > MAX_NEEDS
        || packet.assets.len() > MAX_ASSETS
    {
        return Err("visual packet exceeds its bounded input/need/asset counts".to_string());
    }
    let expected = snapshot
        .sources
        .iter()
        .map(|source| (&source.id, &source.name, &source.sha256, &source.role))
        .collect::<Vec<_>>();
    if packet.sources.len() != expected.len()
        || packet
            .sources
            .iter()
            .zip(expected)
            .any(|(actual, expected)| {
                (&actual.id, &actual.filename, &actual.sha256, &actual.role) != expected
            })
    {
        return Err(
            "visual packet sources must exactly match the app-owned source snapshot in order"
                .to_string(),
        );
    }
    let mut need_ids = HashSet::new();
    let known_units = snapshot
        .unit_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    for need in &packet.needs {
        validate_id(&need.id, "visual need")?;
        if !need_ids.insert(need.id.as_str()) {
            return Err(format!("visual need ID is duplicated: {}", need.id));
        }
        require_text(&need.learner_question, "visual need learner_question")?;
        require_text(
            &need.misconception_prevented,
            "visual need misconception_prevented",
        )?;
        let serving_assets = packet
            .assets
            .iter()
            .filter(|asset| asset.need_ids.contains(&need.id))
            .collect::<Vec<_>>();
        let textbook_concept = need.kind == NeedKind::Concept
            && need.procedure_step_ids.is_empty()
            && !serving_assets.is_empty()
            && serving_assets.iter().all(|asset| {
                asset.input_id.as_deref().is_some_and(|input_id| {
                    packet.inputs.iter().any(|input| {
                            matches!(input, VisualInput::TextbookPage { id, .. } if id == input_id)
                        })
                })
            });
        if (need.source_unit_ids.is_empty() && !textbook_concept)
            || need
                .source_unit_ids
                .iter()
                .any(|id| !known_units.contains(id.as_str()))
        {
            return Err(format!(
                "visual need {} must reference known source units",
                need.id
            ));
        }
        if need.kind == NeedKind::ProcedureStep && need.procedure_step_ids.is_empty() {
            return Err(format!(
                "procedure visual need {} requires procedure_step_ids",
                need.id
            ));
        }
    }
    validate_procedure_steps(packet, plan, &known_units)?;
    match packet.decision {
        VisualDecision::AuthoringTodo => {
            return Err("visual packet is an authoring skeleton: resolve decision, needs, and assets before validation".to_string());
        }
        VisualDecision::NoPurposefulVisual => {
            if plan.course.guide_mode == GuideMode::WeeklyLab {
                return Err("Circuit Lab cannot opt out of purposeful visuals".to_string());
            }
            require_text(
                packet.no_visuals_rationale.as_deref().unwrap_or(""),
                "no-purposeful-visual rationale",
            )?;
            if !packet.inputs.is_empty()
                || !packet.needs.is_empty()
                || !packet.procedure_steps.is_empty()
                || !packet.assets.is_empty()
            {
                return Err(
                    "no-purposeful-visual packets cannot contain inputs, needs, or assets"
                        .to_string(),
                );
            }
        }
        VisualDecision::PurposefulVisuals => {
            if packet.no_visuals_rationale.is_some() {
                return Err(
                    "purposeful-visuals packet cannot have no_visuals_rationale".to_string()
                );
            }
            if packet.needs.is_empty() || packet.assets.is_empty() {
                return Err("purposeful-visuals packet requires needs and assets".to_string());
            }
        }
    }
    Ok(())
}

fn validate_procedure_steps(
    packet: &VisualPacket,
    plan: &PlannedGuide,
    known_units: &HashSet<&str>,
) -> Result<(), String> {
    if plan.course.guide_mode != GuideMode::WeeklyLab {
        if !packet.procedure_steps.is_empty() {
            return Err("procedure_steps are allowed only for Circuit Lab guides".to_string());
        }
        if packet
            .needs
            .iter()
            .any(|need| need.kind == NeedKind::ProcedureStep || !need.procedure_step_ids.is_empty())
        {
            return Err("non-lab visual needs cannot bind procedure steps".to_string());
        }
        return Ok(());
    }
    if packet.decision == VisualDecision::AuthoringTodo && packet.procedure_steps.is_empty() {
        return Ok(());
    }
    if packet.procedure_steps.is_empty() {
        return Err("Circuit Lab visual packets require app-owned procedure_steps".to_string());
    }

    let needs = packet
        .needs
        .iter()
        .map(|need| (need.id.as_str(), need))
        .collect::<HashMap<_, _>>();
    let mut step_ids = HashSet::new();
    for step in &packet.procedure_steps {
        validate_id(&step.id, "procedure step")?;
        require_learner_text(&step.action, "procedure step action", 600)?;
        if !step_ids.insert(step.id.as_str()) {
            return Err(format!("procedure step ID is duplicated: {}", step.id));
        }
        require_anchor(&step.guide_anchor, "procedure step guide_anchor")?;
        if step.need_ids.is_empty() || has_duplicates(&step.need_ids) {
            return Err(format!(
                "procedure step {} requires unique need_ids",
                step.id
            ));
        }
        if has_duplicates(&step.source_unit_ids)
            || step
                .source_unit_ids
                .iter()
                .any(|unit| !known_units.contains(unit.as_str()))
        {
            return Err(format!(
                "procedure step {} has unknown or duplicate source_unit_ids",
                step.id
            ));
        }
        if has_duplicates(&step.required_evidence) {
            return Err(format!(
                "procedure step {} has duplicate required_evidence roles",
                step.id
            ));
        }
        match step.kind {
            ProcedureStepKind::Physical if step.required_evidence.is_empty() => {
                return Err(format!(
                    "physical procedure step {} requires at least one evidence role",
                    step.id
                ));
            }
            ProcedureStepKind::Conceptual if !step.required_evidence.is_empty() => {
                return Err(format!(
                    "conceptual procedure step {} cannot require physical visual evidence",
                    step.id
                ));
            }
            _ => {}
        }
        for need_id in &step.need_ids {
            let need = needs.get(need_id.as_str()).ok_or_else(|| {
                format!(
                    "procedure step {} references unknown visual need {need_id}",
                    step.id
                )
            })?;
            if need.kind != NeedKind::ProcedureStep || !need.procedure_step_ids.contains(&step.id) {
                return Err(format!(
                    "procedure step {} and visual need {need_id} must link to each other",
                    step.id
                ));
            }
        }
    }
    for need in &packet.needs {
        if need.kind != NeedKind::ProcedureStep {
            if !need.procedure_step_ids.is_empty() {
                return Err(format!(
                    "concept visual need {} cannot bind procedure steps",
                    need.id
                ));
            }
            continue;
        }
        let linked = packet
            .procedure_steps
            .iter()
            .filter(|step| step.need_ids.contains(&need.id))
            .map(|step| step.id.as_str())
            .collect::<HashSet<_>>();
        let declared = need
            .procedure_step_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        if linked != declared || declared.len() != need.procedure_step_ids.len() {
            return Err(format!(
                "procedure visual need {} must exactly match app-owned step links",
                need.id
            ));
        }
    }
    Ok(())
}

fn validate_packet_keys(value: &serde_json::Value) -> Result<(), String> {
    check_keys(
        value,
        &[
            "schema_version",
            "sources",
            "decision",
            "no_visuals_rationale",
            "inputs",
            "needs",
            "procedure_steps",
            "assets",
        ],
        "visual packet",
    )?;
    for source in value
        .get("sources")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        check_keys(
            source,
            &["id", "filename", "sha256", "role"],
            "visual packet source",
        )?;
    }
    for input in value
        .get("inputs")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let kind = input
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let allowed: &[&str] = match kind {
            "source-unit" => &["kind", "id", "source_id", "unit_id", "provenance", "rights"],
            "textbook-page" => &[
                "kind",
                "id",
                "textbook_sha256",
                "page_number",
                "provenance",
                "rights",
            ],
            "context-frame" => &["kind", "id", "context_asset_id", "provenance", "rights"],
            "local-png" => &[
                "kind",
                "id",
                "path",
                "sha256",
                "width_px",
                "height_px",
                "evidence_class",
                "provenance",
                "rights",
            ],
            _ => return Err(format!("unsupported visual input kind: {kind:?}")),
        };
        check_keys(input, allowed, "visual input")?;
        check_keys(
            input.get("provenance").unwrap_or(&serde_json::Value::Null),
            &["type", "source", "locator", "transformation"],
            "visual provenance",
        )?;
        check_keys(
            input.get("rights").unwrap_or(&serde_json::Value::Null),
            &["basis", "reuse_scope", "attribution"],
            "visual rights",
        )?;
    }
    for need in value
        .get("needs")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        check_keys(
            need,
            &[
                "id",
                "action",
                "kind",
                "source_unit_ids",
                "procedure_step_ids",
                "learner_question",
                "misconception_prevented",
                "evidence_requirement",
            ],
            "visual need",
        )?;
    }
    for step in value
        .get("procedure_steps")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        check_keys(
            step,
            &[
                "id",
                "action",
                "kind",
                "guide_anchor",
                "need_ids",
                "source_unit_ids",
                "required_evidence",
            ],
            "procedure step",
        )?;
    }
    for asset in value
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        check_keys(
            asset,
            &[
                "id",
                "kind",
                "input_id",
                "need_ids",
                "learning_purpose",
                "alt",
                "caption",
                "explanation",
                "procedure_step_ids",
                "crop",
                "annotations",
                "diagram",
            ],
            "visual asset",
        )?;
        if let Some(crop) = asset.get("crop").filter(|value| !value.is_null()) {
            check_keys(crop, &["x", "y", "width", "height"], "visual crop")?;
        }
        for annotation in asset
            .get("annotations")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            let kind = annotation
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let allowed: &[&str] = match kind {
                "focus" => &["kind", "rect", "tone", "label"],
                "arrow" => &["kind", "from", "to", "tone", "label"],
                "callout" => &["kind", "target", "tone", "number", "text"],
                _ => return Err(format!("unsupported annotation kind: {kind:?}")),
            };
            check_keys(annotation, allowed, "visual annotation")?;
            for field in ["rect", "from", "to", "target"] {
                if let Some(inner) = annotation.get(field) {
                    let keys: &[&str] = if field == "rect" {
                        &["x", "y", "width", "height"]
                    } else {
                        &["x", "y"]
                    };
                    check_keys(inner, keys, "annotation coordinates")?;
                }
            }
        }
        if let Some(diagram) = asset.get("diagram").filter(|value| !value.is_null()) {
            check_keys(
                diagram,
                &["width_px", "height_px", "title", "nodes", "edges"],
                "diagram",
            )?;
            for node in diagram
                .get("nodes")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
            {
                check_keys(
                    node,
                    &["id", "rect", "label", "tone", "shape"],
                    "diagram node",
                )?;
                check_keys(
                    node.get("rect").unwrap_or(&serde_json::Value::Null),
                    &["x", "y", "width", "height"],
                    "diagram node rect",
                )?;
            }
            for edge in diagram
                .get("edges")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
            {
                check_keys(edge, &["from", "to", "label", "tone"], "diagram edge")?;
            }
        }
    }
    Ok(())
}

fn check_keys(value: &serde_json::Value, allowed: &[&str], label: &str) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{label} must be an object"))?;
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("{label} contains unknown field {key:?}"));
    }
    Ok(())
}

struct VisualInputBindingContext<'a> {
    local_root: &'a Path,
    sources: &'a [CapturedSource],
    rendered: &'a [RenderedSourceSnapshot],
    textbooks: &'a [CapturedTextbook],
    context: &'a [CourseContextAsset],
    dependencies: &'a mut Vec<CapturedDependency>,
    resource_budget: &'a mut PreflightResourceBudget,
    decoded_input_budget: &'a mut VisualDecodeBudget,
}

fn bind_input(
    input: &VisualInput,
    binding: &mut VisualInputBindingContext<'_>,
) -> Result<BoundInput, String> {
    let (id, rgba, evidence_class, source_units, procedure_steps, origin, provenance, rights) =
        match input {
            VisualInput::SourceUnit {
                id,
                source_id,
                unit_id,
                provenance,
                rights,
            } => {
                let source = binding
                    .sources
                    .iter()
                    .find(|source| &source.source_id == source_id)
                    .ok_or_else(|| {
                        format!("visual input {id} references unknown source {source_id}")
                    })?;
                let index = source
                    .unit_ids
                    .iter()
                    .position(|candidate| candidate == unit_id)
                    .ok_or_else(|| {
                        format!("visual input {id} references unknown unit {unit_id}")
                    })?;
                let image = binding
                    .rendered
                    .iter()
                    .find(|snapshot| &snapshot.source_id == source_id)
                    .and_then(|snapshot| snapshot.images.get(index))
                    .ok_or_else(|| {
                        format!("visual input {id} has no app-rendered source image for {unit_id}")
                    })?;
                if provenance.source != source.path.to_string_lossy()
                    || !provenance.locator.contains(unit_id)
                {
                    return Err(format!(
                    "visual input {id} provenance must name the bound source path and exact unit ID"
                ));
                }
                (
                    id,
                    decode_png_rgba_budgeted(
                        &image.bytes,
                        binding.decoded_input_budget,
                        binding.resource_budget,
                    )?,
                    EvidenceClass::Source,
                    vec![unit_id.clone()],
                    Vec::new(),
                    InputOrigin::SourceUnit,
                    provenance,
                    rights,
                )
            }
            VisualInput::TextbookPage {
                id,
                textbook_sha256,
                page_number,
                provenance,
                rights,
            } => {
                let textbook = binding
                    .textbooks
                    .iter()
                    .find(|candidate| candidate.dependency.sha256 == *textbook_sha256)
                    .ok_or_else(|| {
                        format!("visual input {id} references unknown textbook {textbook_sha256}")
                    })?;
                let image = textbook
                    .rendered_pages
                    .iter()
                    .find(|candidate| candidate.number == *page_number)
                    .ok_or_else(|| {
                        format!(
                            "visual input {id} has no frozen textbook render for PDF page {page_number}"
                        )
                    })?;
                if textbook.bytes.len() as u64 != textbook.dependency.size_bytes
                    || digest(&textbook.bytes) != textbook.dependency.sha256
                {
                    return Err(format!(
                        "visual input {id} textbook bytes do not match the captured PDF SHA"
                    ));
                }
                if provenance.source != textbook.dependency.path.to_string_lossy()
                    || provenance.locator != format!("PDF page {page_number}")
                {
                    return Err(format!(
                        "visual input {id} provenance must name the bound textbook path and PDF page"
                    ));
                }
                if digest(&image.bytes) != image.sha256 {
                    return Err(format!(
                        "visual input {id} frozen textbook page digest does not match metadata"
                    ));
                }
                let rgba = decode_png_rgba_budgeted(
                    &image.bytes,
                    binding.decoded_input_budget,
                    binding.resource_budget,
                )?;
                if rgba.width != image.width_px || rgba.height != image.height_px {
                    return Err(format!(
                        "visual input {id} frozen textbook page dimensions do not match metadata"
                    ));
                }
                (
                    id,
                    rgba,
                    EvidenceClass::Source,
                    Vec::new(),
                    Vec::new(),
                    InputOrigin::TextbookPage,
                    provenance,
                    rights,
                )
            }
            VisualInput::ContextFrame {
                id,
                context_asset_id,
                provenance,
                rights,
            } => {
                let asset = binding
                    .context
                    .iter()
                    .find(|asset| &asset.id == context_asset_id)
                    .ok_or_else(|| {
                        format!(
                            "visual input {id} references unknown context frame {context_asset_id}"
                        )
                    })?;
                let timestamp = format!(
                    "{:02}:{:02}",
                    asset.timestamp_seconds / 60,
                    asset.timestamp_seconds % 60
                );
                if provenance.source != asset.video_title
                    || (!provenance.locator.contains(&asset.id)
                        && !provenance.locator.contains(&timestamp))
                {
                    return Err(format!(
                    "visual input {id} provenance must name the bound video and stable frame ID or timestamp"
                ));
                }
                (
                    id,
                    decode_png_rgba_budgeted(
                        &asset.bytes,
                        binding.decoded_input_budget,
                        binding.resource_budget,
                    )?,
                    EvidenceClass::Source,
                    Vec::new(),
                    asset.procedure_step_ids.clone(),
                    InputOrigin::ContextFrame,
                    provenance,
                    rights,
                )
            }
            VisualInput::LocalPng {
                id,
                path,
                sha256,
                width_px,
                height_px,
                evidence_class,
                provenance,
                rights,
            } => {
                let relative = safe_relative_path(path)?;
                ensure_plain_directory(binding.local_root, "visual local-input root")?;
                let root = binding.local_root.canonicalize().map_err(|error| {
                    format!("could not resolve visual local-input root: {error}")
                })?;
                let mut parent = binding.local_root.to_path_buf();
                let components = relative.components().collect::<Vec<_>>();
                for component in components.iter().take(components.len().saturating_sub(1)) {
                    if let Component::Normal(name) = component {
                        parent.push(name);
                        ensure_plain_directory(&parent, "visual local-input subdirectory")?;
                    }
                }
                let candidate = binding.local_root.join(&relative);
                ensure_plain_file(&candidate, "visual local input")?;
                let canonical = candidate
                    .canonicalize()
                    .map_err(|error| format!("could not resolve visual local input: {error}"))?;
                if !canonical.starts_with(&root) {
                    return Err(format!(
                        "visual local input escapes its sibling root: {path}"
                    ));
                }
                let (bytes, dependency) = capture_visual_file(
                    &candidate,
                    MAX_INPUT_BYTES,
                    "visual local input",
                    binding.resource_budget,
                )?;
                if dependency.path != canonical || dependency.sha256 != *sha256 {
                    return Err(format!("visual local input SHA/size mismatch: {path}"));
                }
                ensure_plain_directory(binding.local_root, "visual local-input root")?;
                if binding.local_root.canonicalize().ok().as_ref() != Some(&root) {
                    return Err(
                        "visual local-input root identity changed while an input was captured"
                            .to_string(),
                    );
                }
                let rgba = decode_png_rgba_budgeted(
                    &bytes,
                    binding.decoded_input_budget,
                    binding.resource_budget,
                )?;
                if rgba.width != *width_px || rgba.height != *height_px {
                    return Err(format!("visual local input dimensions mismatch: {path}"));
                }
                binding.dependencies.push(dependency);
                (
                    id,
                    rgba,
                    *evidence_class,
                    Vec::new(),
                    Vec::new(),
                    InputOrigin::LocalPng,
                    provenance,
                    rights,
                )
            }
        };
    validate_id(id, "visual input")?;
    validate_provenance(provenance, rights)?;
    if evidence_class == EvidenceClass::Source && rights.basis == RightsBasis::Generated {
        return Err(format!(
            "visual input {id} cannot label generated material as source evidence"
        ));
    }
    Ok(BoundInput {
        id: id.clone(),
        rgba,
        evidence_class,
        source_unit_ids: source_units,
        procedure_step_ids: procedure_steps,
        origin,
        provenance: provenance.clone(),
        rights: rights.clone(),
    })
}

fn validate_asset_semantics(
    spec: &VisualAssetSpec,
    input: Option<&BoundInput>,
    needs: &[&VisualNeed],
    procedure_steps: &[ProcedureStepDefinition],
    plan: &PlannedGuide,
) -> Result<(), String> {
    if spec.annotations.len() > MAX_ANNOTATIONS {
        return Err(format!("visual asset {} has too many annotations", spec.id));
    }
    match spec.kind {
        VisualAssetKind::AnnotatedSource => {
            if input.is_none_or(|input| input.evidence_class != EvidenceClass::Source) {
                return Err(format!(
                    "annotated-source {} requires source evidence",
                    spec.id
                ));
            }
            if spec.annotations.is_empty() || spec.diagram.is_some() {
                return Err(format!(
                    "annotated-source {} needs annotations and cannot have a diagram",
                    spec.id
                ));
            }
        }
        VisualAssetKind::SourceCrop => {
            if input.is_none_or(|input| input.evidence_class != EvidenceClass::Source)
                || !spec.annotations.is_empty()
                || spec.diagram.is_some()
            {
                return Err(format!(
                    "source-crop {} must be an unannotated source input",
                    spec.id
                ));
            }
        }
        VisualAssetKind::GeneratedEducational => {
            if input.is_none_or(|input| input.evidence_class != EvidenceClass::Explanatory)
                || !spec.annotations.is_empty()
                || spec.diagram.is_some()
            {
                return Err(format!(
                    "generated-educational {} requires an explanatory local PNG only",
                    spec.id
                ));
            }
        }
        VisualAssetKind::DeterministicDiagram => {
            if spec.diagram.is_none() || !spec.annotations.is_empty() || spec.crop.is_some() {
                return Err(format!(
                    "deterministic-diagram {} requires only the bounded diagram DSL",
                    spec.id
                ));
            }
            validate_diagram(spec.diagram.as_ref().expect("checked"))?;
        }
    }
    let need_steps = needs
        .iter()
        .flat_map(|need| need.procedure_step_ids.iter())
        .collect::<HashSet<_>>();
    if spec
        .procedure_step_ids
        .iter()
        .any(|step| !need_steps.contains(step))
    {
        return Err(format!(
            "visual asset {} binds a procedure step outside its app-owned needs",
            spec.id
        ));
    }
    let steps_by_id = procedure_steps
        .iter()
        .map(|step| (step.id.as_str(), step))
        .collect::<HashMap<_, _>>();
    if spec
        .procedure_step_ids
        .iter()
        .any(|step| !steps_by_id.contains_key(step.as_str()))
    {
        return Err(format!(
            "visual asset {} references an unknown app-owned procedure step",
            spec.id
        ));
    }
    if let Some(input) = input {
        let need_units = needs
            .iter()
            .flat_map(|need| need.source_unit_ids.iter())
            .collect::<HashSet<_>>();
        if input
            .source_unit_ids
            .iter()
            .any(|unit| !need_units.contains(unit))
        {
            return Err(format!(
                "visual asset {} does not bind its source input to the matching app-owned need",
                spec.id
            ));
        }
        for step_id in &spec.procedure_step_ids {
            let step = steps_by_id
                .get(step_id.as_str())
                .expect("procedure step IDs were checked");
            match input.origin {
                InputOrigin::ContextFrame if !input.procedure_step_ids.contains(step_id) => {
                    return Err(format!(
                        "visual asset {} binds a procedure step not present on its source context frame",
                        spec.id
                    ));
                }
                InputOrigin::SourceUnit
                    if input
                        .source_unit_ids
                        .iter()
                        .any(|unit| !step.source_unit_ids.contains(unit)) =>
                {
                    return Err(format!(
                        "visual asset {} source render is not explicitly mapped to procedure step {step_id}",
                        spec.id
                    ));
                }
                InputOrigin::LocalPng if input.evidence_class == EvidenceClass::Source => {
                    return Err(format!(
                        "visual asset {} cannot use a local PNG as procedure-step source evidence",
                        spec.id
                    ));
                }
                _ => {}
            }
        }
        if spec.kind == VisualAssetKind::GeneratedEducational
            && input.rights.basis != RightsBasis::Generated
        {
            return Err(format!(
                "generated-educational asset {} requires generated rights provenance",
                spec.id
            ));
        }
    }
    if plan.course.guide_mode == GuideMode::WeeklyLab
        && needs
            .iter()
            .any(|need| need.kind == NeedKind::ProcedureStep)
        && spec.procedure_step_ids.is_empty()
    {
        return Err(format!(
            "Circuit Lab procedure visual {} must name procedure_step_ids",
            spec.id
        ));
    }
    for annotation in &spec.annotations {
        validate_annotation(annotation)?;
    }
    if let Some(crop) = spec.crop {
        validate_rect(crop, "crop")?;
    }
    Ok(())
}

fn validate_need_coverage(
    packet: &VisualPacket,
    assets: &[CompiledVisualAsset],
    plan: &PlannedGuide,
) -> Result<(), String> {
    for need in &packet.needs {
        let serving = assets
            .iter()
            .filter(|asset| asset.catalog.need_ids.contains(&need.id))
            .collect::<Vec<_>>();
        if serving.is_empty() {
            return Err(format!("visual need has no compiled asset: {}", need.id));
        }
        if matches!(need.evidence_requirement, EvidenceRequirement::Source)
            && !serving
                .iter()
                .any(|asset| asset.catalog.evidence_class == EvidenceClass::Source)
        {
            return Err(format!("visual need requires source evidence: {}", need.id));
        }
        if plan.course.guide_mode == GuideMode::WeeklyLab && need.kind == NeedKind::ProcedureStep {
            for step_id in &need.procedure_step_ids {
                let step = packet
                    .procedure_steps
                    .iter()
                    .find(|step| step.id == *step_id)
                    .expect("procedure need links were checked");
                if step.kind != ProcedureStepKind::Physical {
                    continue;
                }
                if !serving.iter().any(|asset| {
                    asset.catalog.kind == VisualAssetKind::AnnotatedSource
                        && asset.catalog.evidence_class == EvidenceClass::Source
                        && asset.catalog.procedure_step_ids.len() == 1
                        && asset.catalog.procedure_step_ids[0].as_str() == step_id.as_str()
                }) {
                    return Err(format!(
                        "Circuit Lab physical step {step_id} requires primary annotated-source evidence"
                    ));
                }
            }
        }
    }
    Ok(())
}

fn render_asset(
    spec: &VisualAssetSpec,
    input: Option<&BoundInput>,
    resource_budget: &mut PreflightResourceBudget,
    cancellation: crate::process_registry::CancellationToken,
) -> Result<RgbaImage, String> {
    resource_budget.charge_render_work(estimate_render_pixel_work(spec, input)?)?;
    ensure_visual_preflight_current(cancellation)?;
    match spec.kind {
        VisualAssetKind::DeterministicDiagram => render_diagram(
            spec.diagram
                .as_ref()
                .ok_or_else(|| "missing diagram".to_string())?,
            cancellation,
        ),
        VisualAssetKind::AnnotatedSource => render_annotated(
            &crop_image_checked(
                &input.ok_or_else(|| "missing input".to_string())?.rgba,
                spec.crop,
                cancellation,
            )?,
            &spec.annotations,
            cancellation,
        ),
        VisualAssetKind::SourceCrop | VisualAssetKind::GeneratedEducational => crop_image_checked(
            &input.ok_or_else(|| "missing input".to_string())?.rgba,
            spec.crop,
            cancellation,
        ),
    }
}

fn estimate_render_pixel_work(
    spec: &VisualAssetSpec,
    input: Option<&BoundInput>,
) -> Result<u64, String> {
    let checked_area = |width: u32, height: u32| {
        u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or_else(|| "visual pixel-work estimate overflowed".to_string())
    };
    match spec.kind {
        VisualAssetKind::DeterministicDiagram => {
            let diagram = spec
                .diagram
                .as_ref()
                .ok_or_else(|| "missing diagram".to_string())?;
            let mut work = checked_area(diagram.width_px, diagram.height_px)?;
            work = work
                .checked_add(estimate_text_pixel_work(&diagram.title)?)
                .ok_or_else(|| "visual pixel-work estimate overflowed".to_string())?;
            for node in &diagram.nodes {
                let (_, _, width, height) =
                    rect_pixels(node.rect, diagram.width_px, diagram.height_px);
                let label_work = estimate_text_pixel_work(&node.label)?;
                work = work
                    .checked_add(checked_area(width, height)?)
                    .and_then(|value| value.checked_add(label_work))
                    .ok_or_else(|| "visual pixel-work estimate overflowed".to_string())?;
            }
            for edge in &diagram.edges {
                let from = diagram
                    .nodes
                    .iter()
                    .find(|node| node.id == edge.from)
                    .ok_or_else(|| "diagram edge references an unknown source node".to_string())?;
                let to = diagram
                    .nodes
                    .iter()
                    .find(|node| node.id == edge.to)
                    .ok_or_else(|| "diagram edge references an unknown target node".to_string())?;
                let a = rect_center(from.rect, diagram.width_px, diagram.height_px);
                let b = rect_center(to.rect, diagram.width_px, diagram.height_px);
                let length = u64::from(a.0.abs_diff(b.0).max(a.1.abs_diff(b.1)));
                let label_work = estimate_text_pixel_work(&edge.label)?;
                work = work
                    .checked_add(length.saturating_mul(100))
                    .and_then(|value| value.checked_add(label_work))
                    .ok_or_else(|| "visual pixel-work estimate overflowed".to_string())?;
            }
            Ok(work)
        }
        VisualAssetKind::AnnotatedSource => {
            let source = &input.ok_or_else(|| "missing input".to_string())?.rgba;
            let (_, _, crop_width, crop_height) = spec
                .crop
                .map(|crop| rect_pixels(crop, source.width, source.height))
                .unwrap_or((0, 0, source.width, source.height));
            let scaled_height =
                ((u64::from(crop_height) * 960) / u64::from(crop_width)).clamp(480, 1800) as u32;
            let output = checked_area(1400, scaled_height)?;
            let annotation_passes = u64::try_from(spec.annotations.len())
                .map_err(|_| "visual annotation count overflowed".to_string())?;
            let mut work = checked_area(crop_width, crop_height)?
                .checked_add(output.saturating_mul(3 + annotation_passes))
                .ok_or_else(|| "visual pixel-work estimate overflowed".to_string())?;
            for annotation in &spec.annotations {
                let text = match annotation {
                    Annotation::Focus { label, .. } | Annotation::Arrow { label, .. } => label,
                    Annotation::Callout { text, .. } => text,
                };
                work = work
                    .checked_add(estimate_text_pixel_work(text)?)
                    .ok_or_else(|| "visual pixel-work estimate overflowed".to_string())?;
            }
            Ok(work)
        }
        VisualAssetKind::SourceCrop | VisualAssetKind::GeneratedEducational => {
            let source = &input.ok_or_else(|| "missing input".to_string())?.rgba;
            let (_, _, width, height) = spec
                .crop
                .map(|crop| rect_pixels(crop, source.width, source.height))
                .unwrap_or((0, 0, source.width, source.height));
            checked_area(width, height)?
                .checked_mul(2)
                .ok_or_else(|| "visual pixel-work estimate overflowed".to_string())
        }
    }
}

fn estimate_text_pixel_work(text: &str) -> Result<u64, String> {
    u64::try_from(text.len())
        .ok()
        .and_then(|length| length.checked_mul(WORST_CASE_GLYPH_PIXEL_WORK))
        .ok_or_else(|| "visual text pixel-work estimate overflowed".to_string())
}

fn render_annotated(
    source: &RgbaImage,
    annotations: &[Annotation],
    cancellation: crate::process_registry::CancellationToken,
) -> Result<RgbaImage, String> {
    let width = 1400_u32;
    let panel_width = 440_u32;
    let source_width = width - panel_width;
    let scaled_height = ((source.height as u64 * source_width as u64) / source.width as u64)
        .clamp(480, 1800) as u32;
    let mut canvas =
        RgbaImage::solid_checked(width, scaled_height, [248, 249, 251, 255], cancellation)?;
    let fitted = resize_nearest_checked(source, source_width, scaled_height, cancellation)?;
    canvas.blit_checked(&fitted, 0, 0, cancellation)?;
    canvas.fill_rect_checked(
        source_width,
        0,
        panel_width,
        scaled_height,
        [246, 247, 250, 255],
        cancellation,
    )?;
    canvas.fill_rect_checked(
        source_width,
        0,
        4,
        scaled_height,
        [35, 43, 58, 255],
        cancellation,
    )?;
    draw_text_checked(
        &mut canvas,
        source_width + 28,
        26,
        "TEACHER CALLOUTS",
        2,
        [35, 43, 58, 255],
        cancellation,
    )?;
    let mut callout_y = 76_u32;
    for annotation in annotations {
        ensure_visual_preflight_current(cancellation)?;
        match annotation {
            Annotation::Focus { rect, tone, label } => {
                let (x, y, w, h) = rect_pixels(*rect, source_width, scaled_height);
                let color = tone_color(*tone);
                draw_rect_outline_checked(&mut canvas, (x, y, w, h), color, 5, cancellation)?;
                let label_height = line_stride(2) + 8;
                let label_y = if y >= label_height + 6 {
                    y - label_height - 6
                } else {
                    y + 10
                };
                let text_width = measure_text_width(label, 2);
                let available_width = source_width.saturating_sub(x + 10);
                if text_width.saturating_add(20) > available_width
                    || label_y.saturating_add(label_height) > scaled_height
                {
                    return Err(
                        "focus label does not fit its source canvas; shorten or reposition it"
                            .to_string(),
                    );
                }
                let label_width = text_width.saturating_add(20);
                canvas.fill_rect_checked(
                    x + 10,
                    label_y,
                    label_width,
                    label_height,
                    [255, 255, 255, 255],
                    cancellation,
                )?;
                draw_text_checked(
                    &mut canvas,
                    x.saturating_add(20),
                    label_y.saturating_add(4),
                    label,
                    2,
                    color,
                    cancellation,
                )?;
            }
            Annotation::Arrow {
                from,
                to,
                tone,
                label,
            } => {
                let a = point_pixels(*from, source_width, scaled_height);
                let b = point_pixels(*to, source_width, scaled_height);
                if a.0
                    .saturating_add(8)
                    .saturating_add(measure_text_width(label, 2))
                    > source_width
                    || a.1.saturating_add(8).saturating_add(line_stride(2)) > scaled_height
                {
                    return Err(
                        "arrow label does not fit its source canvas; shorten or reposition it"
                            .to_string(),
                    );
                }
                draw_arrow_checked(&mut canvas, a, b, tone_color(*tone), 4, cancellation)?;
                draw_text_checked(
                    &mut canvas,
                    a.0.saturating_add(8),
                    a.1.saturating_add(8),
                    label,
                    2,
                    tone_color(*tone),
                    cancellation,
                )?;
            }
            Annotation::Callout {
                target,
                tone,
                number,
                text,
            } => {
                let target = point_pixels(*target, source_width, scaled_height);
                let color = tone_color(*tone);
                let marker = format!("{number}");
                let label = format!("{number}. {text}");
                let lines = wrap_ascii_text(&label, 20);
                if lines
                    .iter()
                    .any(|line| measure_text_width(line, 2) > panel_width - 70)
                {
                    return Err(
                        "teacher callout text does not fit the side panel; shorten it".to_string(),
                    );
                }
                let callout_height = (lines.len() as u32)
                    .saturating_mul(line_stride(2))
                    .saturating_add(24)
                    .max(68);
                if callout_y.saturating_add(callout_height).saturating_add(20) > scaled_height {
                    return Err(
                        "teacher callouts do not fit the side panel; shorten or split the visual"
                            .to_string(),
                    );
                }
                canvas.fill_rect_checked(
                    source_width + 18,
                    callout_y,
                    panel_width - 36,
                    callout_height,
                    [255, 255, 255, 255],
                    cancellation,
                )?;
                canvas.fill_rect_checked(
                    source_width + 18,
                    callout_y,
                    7,
                    callout_height,
                    color,
                    cancellation,
                )?;
                let panel_target = (source_width + 18, callout_y + callout_height / 2);
                let connector_end = shorten_endpoint(panel_target, target, 26);
                draw_arrow_checked(
                    &mut canvas,
                    panel_target,
                    connector_end,
                    color,
                    3,
                    cancellation,
                )?;
                fill_circle_checked(&mut canvas, target.0, target.1, 20, color, cancellation)?;
                let marker_width = measure_text_width(&marker, 2);
                draw_text_checked(
                    &mut canvas,
                    target.0.saturating_sub(marker_width / 2),
                    target.1.saturating_sub(15),
                    &marker,
                    2,
                    [255, 255, 255, 255],
                    cancellation,
                )?;
                draw_text_lines_checked(
                    &mut canvas,
                    source_width + 34,
                    callout_y + 12,
                    &lines,
                    2,
                    color,
                    cancellation,
                )?;
                callout_y = callout_y.saturating_add(callout_height + 14);
            }
        }
    }
    Ok(canvas)
}

fn render_diagram(
    spec: &DiagramSpec,
    cancellation: crate::process_registry::CancellationToken,
) -> Result<RgbaImage, String> {
    let mut canvas = RgbaImage::solid_checked(
        spec.width_px,
        spec.height_px,
        [250, 251, 253, 255],
        cancellation,
    )?;
    let title_lines = diagram_title_lines(spec)?;
    draw_text_lines_checked(
        &mut canvas,
        DIAGRAM_TITLE_X,
        DIAGRAM_TITLE_Y,
        &title_lines,
        DIAGRAM_TITLE_SCALE,
        [35, 43, 58, 255],
        cancellation,
    )?;
    let nodes = spec
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    for edge in &spec.edges {
        ensure_visual_preflight_current(cancellation)?;
        let from = nodes
            .get(edge.from.as_str())
            .expect("validated edge source");
        let to = nodes.get(edge.to.as_str()).expect("validated edge target");
        let a = rect_center(from.rect, spec.width_px, spec.height_px);
        let b = rect_center(to.rect, spec.width_px, spec.height_px);
        draw_arrow_checked(&mut canvas, a, b, tone_color(edge.tone), 4, cancellation)?;
        let label_x = (a.0 + b.0) / 2;
        let label_y = (a.1 + b.1) / 2;
        let (label_lines, label_scale) =
            diagram_edge_label_layout(&edge.label, label_x, label_y, spec.width_px, spec.height_px)
                .ok_or_else(|| "diagram edge label does not fit inside the canvas".to_string())?;
        draw_text_lines_checked(
            &mut canvas,
            label_x,
            label_y,
            &label_lines,
            label_scale,
            tone_color(edge.tone),
            cancellation,
        )?;
    }
    for node in &spec.nodes {
        ensure_visual_preflight_current(cancellation)?;
        let (x, y, w, h) = rect_pixels(node.rect, spec.width_px, spec.height_px);
        let color = tone_color(node.tone);
        match node.shape {
            DiagramShape::Box | DiagramShape::RoundedBox => {
                canvas.fill_rect_checked(x, y, w, h, [255, 255, 255, 255], cancellation)?;
                draw_rect_outline_checked(&mut canvas, (x, y, w, h), color, 4, cancellation)?;
            }
            DiagramShape::Circle => {
                fill_circle_checked(
                    &mut canvas,
                    x + w / 2,
                    y + h / 2,
                    w.min(h) / 2,
                    [255, 255, 255, 255],
                    cancellation,
                )?;
                draw_circle_outline_checked(
                    &mut canvas,
                    x + w / 2,
                    y + h / 2,
                    w.min(h) / 2,
                    color,
                    4,
                    cancellation,
                )?;
            }
        }
        let (lines, label_scale) = diagram_node_label_layout(&node.label, w, h)
            .ok_or_else(|| format!("diagram node label does not fit inside node {}", node.id))?;
        let text_height = lines.len() as u32 * line_stride(label_scale);
        draw_text_lines_checked(
            &mut canvas,
            x + 12,
            y + h.saturating_sub(text_height) / 2,
            &lines,
            label_scale,
            color,
            cancellation,
        )?;
    }
    Ok(canvas)
}

fn validate_diagram(diagram: &DiagramSpec) -> Result<(), String> {
    if !(320..=4096).contains(&diagram.width_px)
        || !(240..=4096).contains(&diagram.height_px)
        || diagram.nodes.is_empty()
        || diagram.nodes.len() > MAX_DIAGRAM_NODES
        || diagram.edges.len() > MAX_DIAGRAM_EDGES
    {
        return Err("diagram exceeds safe canvas/node/edge bounds".to_string());
    }
    require_ascii_text(&diagram.title, "diagram title", 160)?;
    diagram_title_lines(diagram)?;
    let mut ids = HashSet::new();
    for node in &diagram.nodes {
        validate_id(&node.id, "diagram node")?;
        validate_rect(node.rect, "diagram node")?;
        require_ascii_text(&node.label, "diagram node label", 160)?;
        let (_, _, width, height) = rect_pixels(node.rect, diagram.width_px, diagram.height_px);
        if width <= 24
            || height <= 24
            || diagram_node_label_layout(&node.label, width, height).is_none()
        {
            return Err(format!(
                "diagram node label does not fit inside node {}",
                node.id
            ));
        }
        if !ids.insert(node.id.as_str()) {
            return Err(format!("diagram node ID is duplicated: {}", node.id));
        }
    }
    let mut directed_edges = HashSet::new();
    for edge in &diagram.edges {
        if !ids.contains(edge.from.as_str()) || !ids.contains(edge.to.as_str()) {
            return Err("diagram edge references an unknown node".to_string());
        }
        if edge.from == edge.to {
            return Err("diagram self-edges are not allowed".to_string());
        }
        if !directed_edges.insert((edge.from.as_str(), edge.to.as_str())) {
            return Err("diagram duplicate directed edges are not allowed".to_string());
        }
        require_ascii_text(&edge.label, "diagram edge label", 120)?;
        let from = diagram
            .nodes
            .iter()
            .find(|node| node.id == edge.from)
            .expect("edge source was checked");
        let to = diagram
            .nodes
            .iter()
            .find(|node| node.id == edge.to)
            .expect("edge target was checked");
        let a = rect_center(from.rect, diagram.width_px, diagram.height_px);
        let b = rect_center(to.rect, diagram.width_px, diagram.height_px);
        if a == b {
            return Err("diagram edges must not have zero visual length".to_string());
        }
        let label_x = (a.0 + b.0) / 2;
        let label_y = (a.1 + b.1) / 2;
        if diagram_edge_label_layout(
            &edge.label,
            label_x,
            label_y,
            diagram.width_px,
            diagram.height_px,
        )
        .is_none()
        {
            return Err("diagram edge label does not fit inside the canvas".to_string());
        }
    }
    Ok(())
}

fn validate_annotation(annotation: &Annotation) -> Result<(), String> {
    match annotation {
        Annotation::Focus { rect, label, .. } => {
            validate_rect(*rect, "focus annotation")?;
            require_ascii_text(label, "focus label", 120)
        }
        Annotation::Arrow {
            from, to, label, ..
        } => {
            validate_point(*from, "arrow start")?;
            validate_point(*to, "arrow end")?;
            if from == to {
                return Err("annotation arrow cannot have identical endpoints".to_string());
            }
            require_ascii_text(label, "arrow label", 120)
        }
        Annotation::Callout {
            target,
            number,
            text,
            ..
        } => {
            validate_point(*target, "callout target")?;
            if *number == 0 || *number > 99 {
                return Err("callout number must be between 1 and 99".to_string());
            }
            require_ascii_text(text, "callout text", 360)
        }
    }
}

fn validate_asset_text(spec: &VisualAssetSpec) -> Result<(), String> {
    require_text(&spec.learning_purpose, "visual asset learning_purpose")?;
    require_learner_text(&spec.alt, "visual asset alt", 240)?;
    require_learner_text(&spec.caption, "visual asset caption", 320)?;
    require_learner_text(&spec.explanation, "visual asset explanation", 640)?;
    if spec.need_ids.is_empty()
        || has_duplicates(&spec.need_ids)
        || has_duplicates(&spec.procedure_step_ids)
    {
        return Err(format!(
            "visual asset {} has missing or duplicate bindings",
            spec.id
        ));
    }
    Ok(())
}

fn require_learner_text(value: &str, label: &str, max: usize) -> Result<(), String> {
    if value.is_empty()
        || value.len() > max
        || value.trim() != value
        || value.split_whitespace().collect::<Vec<_>>().join(" ") != value
        || value
            .chars()
            .any(|character| character.is_control() || "\\[]<>&`*_".contains(character))
    {
        return Err(format!(
            "{label} must be canonical single-line Markdown-safe text of at most {max} bytes"
        ));
    }
    Ok(())
}

fn validate_provenance(
    provenance: &VisualProvenance,
    rights: &RightsMetadata,
) -> Result<(), String> {
    for (value, label) in [
        (&provenance.kind, "provenance type"),
        (&provenance.source, "provenance source"),
        (&provenance.locator, "provenance locator"),
        (&provenance.transformation, "provenance transformation"),
        (&rights.attribution, "rights attribution"),
    ] {
        require_text(value, label)?;
    }
    Ok(())
}

fn output_provenance(kind: VisualAssetKind, input: &VisualProvenance) -> VisualProvenance {
    VisualProvenance {
        kind: match kind {
            VisualAssetKind::AnnotatedSource => "annotated-source",
            VisualAssetKind::SourceCrop => "source-crop",
            VisualAssetKind::GeneratedEducational => "generated-educational",
            VisualAssetKind::DeterministicDiagram => "deterministic-diagram",
        }
        .to_string(),
        source: input.source.clone(),
        locator: input.locator.clone(),
        transformation: match kind {
            VisualAssetKind::AnnotatedSource => "Cropped if requested, normalized to RGBA8, and annotated deterministically with teacher callouts; source facts unchanged.",
            VisualAssetKind::SourceCrop => "Cropped if requested and normalized deterministically to RGBA8; source facts unchanged.",
            VisualAssetKind::GeneratedEducational => "Validated local generated input normalized deterministically to RGBA8; explanatory only.",
            VisualAssetKind::DeterministicDiagram => unreachable!(),
        }
        .to_string(),
    }
}

fn decode_png_rgba(bytes: &[u8]) -> Result<RgbaImage, String> {
    let decoded = crate::png_validation::decode_rgba(bytes, 8192)?;
    Ok(RgbaImage {
        width: decoded.width,
        height: decoded.height,
        pixels: decoded.rgba,
    })
}

fn decode_png_rgba_budgeted(
    bytes: &[u8],
    budget: &mut VisualDecodeBudget,
    resource_budget: &mut PreflightResourceBudget,
) -> Result<RgbaImage, String> {
    let (_, _, decoded_bytes) = crate::png_validation::inspect_rgba_size(bytes, 8192)?;
    budget.charge(decoded_bytes)?;
    resource_budget.charge_visual_decoded(decoded_bytes as u64)?;
    let decoded = decode_png_rgba(bytes)?;
    if decoded.pixels.len() != decoded_bytes {
        return Err("decoded visual allocation did not match its reserved byte count".to_string());
    }
    Ok(decoded)
}

fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Best);
        encoder.set_filter(png::FilterType::Sub);
        let mut writer = encoder
            .write_header()
            .map_err(|error| format!("could not encode visual PNG header: {error}"))?;
        writer
            .write_image_data(&image.pixels)
            .map_err(|error| format!("could not encode visual PNG data: {error}"))?;
    }
    Ok(bytes)
}

impl RgbaImage {
    #[cfg(test)]
    fn solid(width: u32, height: u32, color: [u8; 4]) -> Result<Self, String> {
        let len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|value| value.checked_mul(4))
            .ok_or_else(|| "visual canvas size overflow".to_string())?;
        if len > crate::png_validation::MAX_PNG_DECODED_BYTES {
            return Err("visual canvas exceeds the 64 MiB limit".to_string());
        }
        let mut pixels = vec![0; len];
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&color);
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    fn solid_checked(
        width: u32,
        height: u32,
        color: [u8; 4],
        cancellation: crate::process_registry::CancellationToken,
    ) -> Result<Self, String> {
        ensure_visual_preflight_current(cancellation)?;
        let len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|value| value.checked_mul(4))
            .ok_or_else(|| "visual canvas size overflow".to_string())?;
        if len > crate::png_validation::MAX_PNG_DECODED_BYTES {
            return Err("visual canvas exceeds the 64 MiB limit".to_string());
        }
        let mut pixels = vec![0; len];
        let row_bytes = (width as usize)
            .checked_mul(4)
            .ok_or_else(|| "visual canvas row size overflow".to_string())?;
        for row in pixels.chunks_exact_mut(row_bytes) {
            ensure_visual_preflight_current(cancellation)?;
            for pixel in row.chunks_exact_mut(4) {
                pixel.copy_from_slice(&color);
            }
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    fn set(&mut self, x: i32, y: i32, color: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let index = ((y as u32 * self.width + x as u32) * 4) as usize;
        self.pixels[index..index + 4].copy_from_slice(&color);
    }

    fn blend_opaque(&mut self, x: i32, y: i32, color: [u8; 4], coverage: u8) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let index = ((y as u32 * self.width + x as u32) * 4) as usize;
        let alpha = u16::from(coverage);
        for (offset, foreground) in color[..3].iter().enumerate() {
            let background = u16::from(self.pixels[index + offset]);
            self.pixels[index + offset] =
                ((u16::from(*foreground) * alpha + background * (255 - alpha) + 127) / 255) as u8;
        }
        self.pixels[index + 3] = 255;
    }

    #[cfg(test)]
    fn fill_rect(&mut self, x: u32, y: u32, width: u32, height: u32, color: [u8; 4]) {
        for py in y..y.saturating_add(height).min(self.height) {
            for px in x..x.saturating_add(width).min(self.width) {
                self.set(px as i32, py as i32, color);
            }
        }
    }

    fn fill_rect_checked(
        &mut self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        color: [u8; 4],
        cancellation: crate::process_registry::CancellationToken,
    ) -> Result<(), String> {
        for py in y..y.saturating_add(height).min(self.height) {
            ensure_visual_preflight_current(cancellation)?;
            for px in x..x.saturating_add(width).min(self.width) {
                self.set(px as i32, py as i32, color);
            }
        }
        Ok(())
    }

    fn blit_checked(
        &mut self,
        other: &RgbaImage,
        x: u32,
        y: u32,
        cancellation: crate::process_registry::CancellationToken,
    ) -> Result<(), String> {
        if x + other.width > self.width || y + other.height > self.height {
            return Err("visual blit exceeds destination canvas".to_string());
        }
        for row in 0..other.height {
            ensure_visual_preflight_current(cancellation)?;
            let source = (row * other.width * 4) as usize;
            let destination = (((y + row) * self.width + x) * 4) as usize;
            let count = (other.width * 4) as usize;
            self.pixels[destination..destination + count]
                .copy_from_slice(&other.pixels[source..source + count]);
        }
        Ok(())
    }
}

fn crop_image_checked(
    image: &RgbaImage,
    crop: Option<NormRect>,
    cancellation: crate::process_registry::CancellationToken,
) -> Result<RgbaImage, String> {
    let Some(crop) = crop else {
        ensure_visual_preflight_current(cancellation)?;
        return Ok(image.clone());
    };
    validate_rect(crop, "crop")?;
    let (x, y, width, height) = rect_pixels(crop, image.width, image.height);
    let mut output = RgbaImage::solid_checked(
        width.max(1),
        height.max(1),
        [255, 255, 255, 255],
        cancellation,
    )?;
    for row in 0..output.height {
        ensure_visual_preflight_current(cancellation)?;
        let source = (((y + row).min(image.height - 1) * image.width + x) * 4) as usize;
        let destination = (row * output.width * 4) as usize;
        let count = (output.width.min(image.width - x) * 4) as usize;
        output.pixels[destination..destination + count]
            .copy_from_slice(&image.pixels[source..source + count]);
    }
    Ok(output)
}

fn resize_nearest_checked(
    image: &RgbaImage,
    width: u32,
    height: u32,
    cancellation: crate::process_registry::CancellationToken,
) -> Result<RgbaImage, String> {
    let mut output = RgbaImage::solid_checked(width, height, [255, 255, 255, 255], cancellation)?;
    for y in 0..height {
        ensure_visual_preflight_current(cancellation)?;
        let source_y = (y as u64 * image.height as u64 / height as u64) as u32;
        for x in 0..width {
            let source_x = (x as u64 * image.width as u64 / width as u64) as u32;
            let source = ((source_y * image.width + source_x) * 4) as usize;
            let destination = ((y * width + x) * 4) as usize;
            output.pixels[destination..destination + 4]
                .copy_from_slice(&image.pixels[source..source + 4]);
        }
    }
    Ok(output)
}

fn teacher_font() -> &'static Font {
    static FONT: OnceLock<Font> = OnceLock::new();
    FONT.get_or_init(|| {
        Font::from_bytes(notosans::REGULAR_TTF, FontSettings::default())
            .expect("the checksum-pinned bundled Noto Sans font must parse")
    })
}

fn font_px(scale: u32) -> f32 {
    scale as f32 * 12.0
}

fn line_stride(scale: u32) -> u32 {
    scale * 15
}

#[cfg(test)]
fn draw_text(image: &mut RgbaImage, x: u32, y: u32, text: &str, scale: u32, color: [u8; 4]) {
    let font = teacher_font();
    let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
    layout.reset(&LayoutSettings {
        x: x as f32,
        y: y as f32,
        max_width: Some(image.width.saturating_sub(x) as f32),
        max_height: Some(image.height.saturating_sub(y) as f32),
        ..LayoutSettings::default()
    });
    layout.append(&[font], &TextStyle::new(text, font_px(scale), 0));
    for glyph in layout.glyphs() {
        let (_, bitmap) = font.rasterize_config(glyph.key);
        for row in 0..glyph.height {
            for column in 0..glyph.width {
                let alpha = bitmap[row * glyph.width + column];
                if alpha == 0 {
                    continue;
                }
                image.blend_opaque(
                    glyph.x.floor() as i32 + column as i32,
                    glyph.y.floor() as i32 + row as i32,
                    color,
                    alpha,
                );
            }
        }
    }
}

fn draw_text_checked(
    image: &mut RgbaImage,
    x: u32,
    y: u32,
    text: &str,
    scale: u32,
    color: [u8; 4],
    cancellation: crate::process_registry::CancellationToken,
) -> Result<(), String> {
    let font = teacher_font();
    let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
    layout.reset(&LayoutSettings {
        x: x as f32,
        y: y as f32,
        max_width: Some(image.width.saturating_sub(x) as f32),
        max_height: Some(image.height.saturating_sub(y) as f32),
        ..LayoutSettings::default()
    });
    layout.append(&[font], &TextStyle::new(text, font_px(scale), 0));
    for glyph in layout.glyphs() {
        ensure_visual_preflight_current(cancellation)?;
        let (_, bitmap) = font.rasterize_config(glyph.key);
        for row in 0..glyph.height {
            ensure_visual_preflight_current(cancellation)?;
            for column in 0..glyph.width {
                let alpha = bitmap[row * glyph.width + column];
                if alpha == 0 {
                    continue;
                }
                image.blend_opaque(
                    glyph.x.floor() as i32 + column as i32,
                    glyph.y.floor() as i32 + row as i32,
                    color,
                    alpha,
                );
            }
        }
    }
    Ok(())
}

fn measure_text_width(text: &str, scale: u32) -> u32 {
    let font = teacher_font();
    let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
    layout.reset(&LayoutSettings::default());
    layout.append(&[font], &TextStyle::new(text, font_px(scale), 0));
    layout
        .glyphs()
        .iter()
        .map(|glyph| glyph.x.max(0.0) + glyph.width as f32)
        .fold(0.0_f32, f32::max)
        .ceil() as u32
}

fn wrap_ascii_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_ascii_whitespace() {
        let mut remainder = word;
        while remainder.len() > max_chars {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            let (chunk, rest) = remainder.split_at(max_chars);
            lines.push(chunk.to_string());
            remainder = rest;
        }
        if remainder.is_empty() {
            continue;
        }
        let separator = usize::from(!current.is_empty());
        if current.len() + separator + remainder.len() > max_chars {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(remainder);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn wrap_ascii_text_to_width(text: &str, max_width: u32, scale: u32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_ascii_whitespace() {
        if !current.is_empty() {
            let candidate = format!("{current} {word}");
            if measure_text_width(&candidate, scale) <= max_width {
                current = candidate;
                continue;
            }
            lines.push(std::mem::take(&mut current));
        }

        for character in word.chars() {
            let mut candidate = current.clone();
            candidate.push(character);
            if !current.is_empty() && measure_text_width(&candidate, scale) > max_width {
                lines.push(std::mem::take(&mut current));
            }
            current.push(character);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn diagram_title_lines(diagram: &DiagramSpec) -> Result<Vec<String>, String> {
    let max_width = diagram.width_px.saturating_sub(DIAGRAM_TITLE_X * 2);
    let lines = wrap_ascii_text_to_width(&diagram.title, max_width, DIAGRAM_TITLE_SCALE);
    let title_height = (lines.len() as u32).saturating_mul(line_stride(DIAGRAM_TITLE_SCALE));
    if max_width == 0
        || lines.len() > MAX_DIAGRAM_TITLE_LINES
        || lines
            .iter()
            .any(|line| measure_text_width(line, DIAGRAM_TITLE_SCALE) > max_width)
        || DIAGRAM_TITLE_Y.saturating_add(title_height) > diagram.height_px
    {
        return Err("diagram title does not fit inside the canvas".to_string());
    }
    Ok(lines)
}

fn diagram_node_label_layout(label: &str, width: u32, height: u32) -> Option<(Vec<String>, u32)> {
    let max_width = width.checked_sub(24)?;
    let max_height = height.checked_sub(24)?;
    for scale in [2, 1] {
        let lines = wrap_ascii_text_to_width(label, max_width, scale);
        let text_height = (lines.len() as u32).saturating_mul(line_stride(scale));
        if text_height <= max_height
            && lines
                .iter()
                .all(|line| measure_text_width(line, scale) <= max_width)
        {
            return Some((lines, scale));
        }
    }
    None
}

fn diagram_edge_label_layout(
    label: &str,
    x: u32,
    y: u32,
    canvas_width: u32,
    canvas_height: u32,
) -> Option<(Vec<String>, u32)> {
    let max_width = canvas_width.checked_sub(x.saturating_add(4))?;
    let max_height = canvas_height.checked_sub(y.saturating_add(4))?;
    for scale in [2, 1] {
        let lines = wrap_ascii_text_to_width(label, max_width, scale);
        let text_height = (lines.len() as u32).saturating_mul(line_stride(scale));
        if lines.len() <= MAX_DIAGRAM_EDGE_LABEL_LINES
            && text_height <= max_height
            && lines
                .iter()
                .all(|line| measure_text_width(line, scale) <= max_width)
        {
            return Some((lines, scale));
        }
    }
    None
}

fn draw_text_lines_checked(
    image: &mut RgbaImage,
    x: u32,
    y: u32,
    lines: &[String],
    scale: u32,
    color: [u8; 4],
    cancellation: crate::process_registry::CancellationToken,
) -> Result<(), String> {
    for (index, line) in lines.iter().enumerate() {
        ensure_visual_preflight_current(cancellation)?;
        draw_text_checked(
            image,
            x,
            y.saturating_add(index as u32 * line_stride(scale)),
            line,
            scale,
            color,
            cancellation,
        )?;
    }
    Ok(())
}

#[cfg(test)]
fn draw_line(image: &mut RgbaImage, from: (u32, u32), to: (u32, u32), color: [u8; 4], width: i32) {
    let (mut x0, mut y0) = (from.0 as i32, from.1 as i32);
    let (x1, y1) = (to.0 as i32, to.1 as i32);
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        for oy in -width..=width {
            for ox in -width..=width {
                image.set(x0 + ox, y0 + oy, color);
            }
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let doubled = 2 * error;
        if doubled >= dy {
            error += dy;
            x0 += sx;
        }
        if doubled <= dx {
            error += dx;
            y0 += sy;
        }
    }
}

fn draw_line_checked(
    image: &mut RgbaImage,
    from: (u32, u32),
    to: (u32, u32),
    color: [u8; 4],
    width: i32,
    cancellation: crate::process_registry::CancellationToken,
) -> Result<(), String> {
    let (mut x0, mut y0) = (from.0 as i32, from.1 as i32);
    let (x1, y1) = (to.0 as i32, to.1 as i32);
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        ensure_visual_preflight_current(cancellation)?;
        for oy in -width..=width {
            for ox in -width..=width {
                image.set(x0 + ox, y0 + oy, color);
            }
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let doubled = 2 * error;
        if doubled >= dy {
            error += dy;
            x0 += sx;
        }
        if doubled <= dx {
            error += dx;
            y0 += sy;
        }
    }
    Ok(())
}

#[cfg(test)]
fn draw_arrow(image: &mut RgbaImage, from: (u32, u32), to: (u32, u32), color: [u8; 4], width: i32) {
    draw_line(image, from, to, color, width);
    let dx = from.0 as f32 - to.0 as f32;
    let dy = from.1 as f32 - to.1 as f32;
    let length = (dx * dx + dy * dy).sqrt().max(1.0);
    let ux = dx / length;
    let uy = dy / length;
    let left = (
        (to.0 as f32 + ux * 24.0 - uy * 12.0).max(0.0) as u32,
        (to.1 as f32 + uy * 24.0 + ux * 12.0).max(0.0) as u32,
    );
    let right = (
        (to.0 as f32 + ux * 24.0 + uy * 12.0).max(0.0) as u32,
        (to.1 as f32 + uy * 24.0 - ux * 12.0).max(0.0) as u32,
    );
    draw_line(image, to, left, color, width);
    draw_line(image, to, right, color, width);
}

fn draw_arrow_checked(
    image: &mut RgbaImage,
    from: (u32, u32),
    to: (u32, u32),
    color: [u8; 4],
    width: i32,
    cancellation: crate::process_registry::CancellationToken,
) -> Result<(), String> {
    draw_line_checked(image, from, to, color, width, cancellation)?;
    let dx = from.0 as f32 - to.0 as f32;
    let dy = from.1 as f32 - to.1 as f32;
    let length = (dx * dx + dy * dy).sqrt().max(1.0);
    let ux = dx / length;
    let uy = dy / length;
    let left = (
        (to.0 as f32 + ux * 24.0 - uy * 12.0).max(0.0) as u32,
        (to.1 as f32 + uy * 24.0 + ux * 12.0).max(0.0) as u32,
    );
    let right = (
        (to.0 as f32 + ux * 24.0 + uy * 12.0).max(0.0) as u32,
        (to.1 as f32 + uy * 24.0 - ux * 12.0).max(0.0) as u32,
    );
    draw_line_checked(image, to, left, color, width, cancellation)?;
    draw_line_checked(image, to, right, color, width, cancellation)
}

fn shorten_endpoint(from: (u32, u32), to: (u32, u32), padding: u32) -> (u32, u32) {
    let dx = to.0 as f32 - from.0 as f32;
    let dy = to.1 as f32 - from.1 as f32;
    let length = (dx * dx + dy * dy).sqrt();
    if length <= padding as f32 || length == 0.0 {
        return to;
    }
    let remaining = (length - padding as f32) / length;
    (
        (from.0 as f32 + dx * remaining).round().max(0.0) as u32,
        (from.1 as f32 + dy * remaining).round().max(0.0) as u32,
    )
}

#[cfg(test)]
fn draw_rect_outline(
    image: &mut RgbaImage,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    color: [u8; 4],
    width: u32,
) {
    image.fill_rect(x, y, w, width, color);
    image.fill_rect(x, y + h.saturating_sub(width), w, width, color);
    image.fill_rect(x, y, width, h, color);
    image.fill_rect(x + w.saturating_sub(width), y, width, h, color);
}

fn draw_rect_outline_checked(
    image: &mut RgbaImage,
    rect: (u32, u32, u32, u32),
    color: [u8; 4],
    width: u32,
    cancellation: crate::process_registry::CancellationToken,
) -> Result<(), String> {
    let (x, y, w, h) = rect;
    image.fill_rect_checked(x, y, w, width, color, cancellation)?;
    image.fill_rect_checked(
        x,
        y + h.saturating_sub(width),
        w,
        width,
        color,
        cancellation,
    )?;
    image.fill_rect_checked(x, y, width, h, color, cancellation)?;
    image.fill_rect_checked(
        x + w.saturating_sub(width),
        y,
        width,
        h,
        color,
        cancellation,
    )
}

fn fill_circle_checked(
    image: &mut RgbaImage,
    cx: u32,
    cy: u32,
    radius: u32,
    color: [u8; 4],
    cancellation: crate::process_registry::CancellationToken,
) -> Result<(), String> {
    let r2 = (radius * radius) as i64;
    for y in -(radius as i32)..=radius as i32 {
        ensure_visual_preflight_current(cancellation)?;
        for x in -(radius as i32)..=radius as i32 {
            if i64::from(x * x + y * y) <= r2 {
                image.set(cx as i32 + x, cy as i32 + y, color);
            }
        }
    }
    Ok(())
}

fn draw_circle_outline_checked(
    image: &mut RgbaImage,
    cx: u32,
    cy: u32,
    radius: u32,
    color: [u8; 4],
    width: u32,
    cancellation: crate::process_registry::CancellationToken,
) -> Result<(), String> {
    let outer = (radius * radius) as i64;
    let inner_radius = radius.saturating_sub(width);
    let inner = (inner_radius * inner_radius) as i64;
    for y in -(radius as i32)..=radius as i32 {
        ensure_visual_preflight_current(cancellation)?;
        for x in -(radius as i32)..=radius as i32 {
            let d = i64::from(x * x + y * y);
            if d <= outer && d >= inner {
                image.set(cx as i32 + x, cy as i32 + y, color);
            }
        }
    }
    Ok(())
}

fn tone_color(tone: SemanticTone) -> [u8; 4] {
    match tone {
        SemanticTone::Focus => [28, 100, 242, 255],
        SemanticTone::Warning => [180, 83, 9, 255],
        SemanticTone::Action => [99, 50, 180, 255],
        SemanticTone::Expected => [20, 122, 72, 255],
        SemanticTone::Wrong => [194, 45, 45, 255],
        SemanticTone::Recovery => [0, 113, 132, 255],
    }
}

fn rect_pixels(rect: NormRect, width: u32, height: u32) -> (u32, u32, u32, u32) {
    let x = u32::from(rect.x) * width / u32::from(NORM_MAX);
    let y = u32::from(rect.y) * height / u32::from(NORM_MAX);
    let w = (u32::from(rect.width) * width / u32::from(NORM_MAX)).max(1);
    let h = (u32::from(rect.height) * height / u32::from(NORM_MAX)).max(1);
    (
        x.min(width - 1),
        y.min(height - 1),
        w.min(width - x),
        h.min(height - y),
    )
}

fn point_pixels(point: NormPoint, width: u32, height: u32) -> (u32, u32) {
    (
        (u32::from(point.x) * width / u32::from(NORM_MAX)).min(width - 1),
        (u32::from(point.y) * height / u32::from(NORM_MAX)).min(height - 1),
    )
}

fn rect_center(rect: NormRect, width: u32, height: u32) -> (u32, u32) {
    let (x, y, w, h) = rect_pixels(rect, width, height);
    (x + w / 2, y + h / 2)
}

fn validate_rect(rect: NormRect, label: &str) -> Result<(), String> {
    if rect.width == 0
        || rect.height == 0
        || rect.x > NORM_MAX
        || rect.y > NORM_MAX
        || u32::from(rect.x) + u32::from(rect.width) > u32::from(NORM_MAX)
        || u32::from(rect.y) + u32::from(rect.height) > u32::from(NORM_MAX)
    {
        return Err(format!(
            "{label} rectangle must stay within normalized 0..10000 bounds"
        ));
    }
    Ok(())
}

fn validate_point(point: NormPoint, label: &str) -> Result<(), String> {
    if point.x > NORM_MAX || point.y > NORM_MAX {
        return Err(format!(
            "{label} must stay within normalized 0..10000 bounds"
        ));
    }
    Ok(())
}

fn validate_id(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 96
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(format!(
            "{label} ID must use 1..96 lowercase ASCII letters, digits, or hyphens"
        ));
    }
    Ok(())
}

fn require_text(value: &str, label: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.contains('\0') || value.len() > 4000 {
        return Err(format!(
            "{label} must be nonempty, NUL-free, and at most 4000 bytes"
        ));
    }
    Ok(())
}

fn require_ascii_text(value: &str, label: &str, max: usize) -> Result<(), String> {
    require_text(value, label)?;
    if value.len() > max || !value.is_ascii() || value.chars().any(char::is_control) {
        return Err(format!(
            "{label} must be printable ASCII and at most {max} bytes"
        ));
    }
    Ok(())
}

fn has_duplicates<T: Eq + std::hash::Hash>(values: &[T]) -> bool {
    let mut seen = HashSet::new();
    values.iter().any(|value| !seen.insert(value))
}

fn require_anchor(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 160
        || value.starts_with('-')
        || value.ends_with('-')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(format!(
            "{label} must be a 1..160 byte lowercase Markdown heading anchor"
        ));
    }
    Ok(())
}

fn safe_relative_path(value: &str) -> Result<PathBuf, String> {
    if value.is_empty()
        || value.contains('\0')
        || value.contains(':')
        || value.contains("//")
        || value.contains('\\')
    {
        return Err("visual local input path is unsafe".to_string());
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || path.extension().and_then(|ext| ext.to_str()) != Some("png")
    {
        return Err(
            "visual local input must be a relative .png path without traversal".to_string(),
        );
    }
    Ok(path.to_path_buf())
}

fn ensure_plain_file(path: &Path, label: &str) -> Result<(), String> {
    ensure_plain_path_chain(path, false, label)
}

fn ensure_plain_directory(path: &Path, label: &str) -> Result<(), String> {
    ensure_plain_path_chain(path, true, label)
}

fn ensure_plain_path_chain(
    path: &Path,
    leaf_is_directory: bool,
    label: &str,
) -> Result<(), String> {
    let ancestors = path
        .ancestors()
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
        .collect::<Vec<_>>();
    let roots = crate::source_context::trusted_roots();
    for (index, component_path) in ancestors.iter().rev().enumerate() {
        let is_leaf = index + 1 == ancestors.len();
        if !is_leaf && crate::source_context::is_above_trusted_root(component_path, &roots) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(component_path).map_err(|error| {
            format!(
                "{label} path component is missing or unreadable ({}): {error}",
                component_path.display()
            )
        })?;
        if metadata.file_type().is_symlink() || is_reparse(&metadata) {
            return Err(format!(
                "{label} path component must not be a symlink, junction, or reparse point: {}",
                component_path.display()
            ));
        }
        let correct_kind = if is_leaf && !leaf_is_directory {
            metadata.file_type().is_file()
        } else {
            metadata.file_type().is_dir()
        };
        if !correct_kind {
            return Err(format!(
                "{label} path component has the wrong file type: {}",
                component_path.display()
            ));
        }
    }
    Ok(())
}

fn capture_visual_file(
    requested: &Path,
    max_bytes: u64,
    label: &str,
    resource_budget: &mut PreflightResourceBudget,
) -> Result<(Vec<u8>, CapturedDependency), String> {
    ensure_plain_file(requested, label)?;
    let canonical = requested
        .canonicalize()
        .map_err(|error| format!("could not resolve {label}: {error}"))?;
    ensure_plain_file(&canonical, &format!("resolved {label}"))?;
    let metadata = std::fs::symlink_metadata(&canonical)
        .map_err(|error| format!("could not inspect {label}: {error}"))?;
    if metadata.len() == 0 || metadata.len() > max_bytes {
        return Err(format!("{label} has invalid size {} bytes", metadata.len()));
    }
    resource_budget.charge_source_file(&canonical, metadata.len(), label)?;
    let bytes =
        std::fs::read(&canonical).map_err(|error| format!("could not read {label}: {error}"))?;
    if bytes.len() as u64 != metadata.len() {
        return Err(format!("{label} changed size while it was captured"));
    }
    ensure_plain_file(requested, label)?;
    let resolved_after = requested
        .canonicalize()
        .map_err(|error| format!("could not re-resolve {label}: {error}"))?;
    if resolved_after != canonical {
        return Err(format!(
            "{label} path identity changed while it was captured"
        ));
    }
    ensure_plain_file(&resolved_after, &format!("resolved {label}"))?;
    let sha256 = digest(&bytes);
    Ok((
        bytes,
        CapturedDependency {
            path: canonical,
            requested_path: Some(requested.to_path_buf()),
            sha256,
            size_bytes: metadata.len(),
        },
    ))
}

#[cfg(windows)]
fn is_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_reparse(_: &std::fs::Metadata) -> bool {
    false
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
pub fn write_no_visual_packet_for_test(plan: &PlannedGuide) -> Result<(), String> {
    let sources = plan
        .source_paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let bytes = std::fs::read(path)
                .map_err(|error| format!("could not read test source {}: {error}", path.display()))?;
            Ok(serde_json::json!({
                "id": crate::source_context::source_id_for_path(path)?,
                "filename": path.file_name().and_then(|value| value.to_str()).ok_or_else(|| "test source filename is not Unicode".to_string())?,
                "sha256": digest(&bytes),
                "role": if index == 0 { "primary" } else { "support" },
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let packet = serde_json::json!({
        "schema_version": 2,
        "sources": sources,
        "decision": "no-purposeful-visual",
        "no_visuals_rationale": "Test fixture is intentionally text-only; visual behavior has dedicated adversarial tests.",
        "inputs": [],
        "needs": [],
        "procedure_steps": [],
        "assets": [],
    });
    let bytes = serde_json::to_vec_pretty(&packet)
        .map_err(|error| format!("could not serialize test visual packet: {error}"))?;
    std::fs::write(packet_path(&plan.primary_source)?, bytes)
        .map_err(|error| format!("could not write test visual packet: {error}"))
}

#[cfg(test)]
pub(crate) fn write_source_crop_packet_for_test(plan: &PlannedGuide) -> Result<(), String> {
    let sources = plan
        .source_paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let bytes = std::fs::read(path)
                .map_err(|error| format!("could not read test source {}: {error}", path.display()))?;
            Ok(serde_json::json!({
                "id": crate::source_context::source_id_for_path(path)?,
                "filename": path.file_name().and_then(|value| value.to_str()).ok_or_else(|| "test source filename is not Unicode".to_string())?,
                "sha256": digest(&bytes),
                "role": if index == 0 { "primary" } else { "support" },
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let source_id = crate::source_context::source_id_for_path(&plan.primary_source)?;
    let unit_id = format!("{source_id}-unit-001");
    let source_label = plan.primary_source.to_string_lossy().to_string();
    let packet = serde_json::json!({
        "schema_version": 2,
        "sources": sources,
        "decision": "purposeful-visuals",
        "inputs": [{
            "kind": "source-unit",
            "id": "source-render",
            "source_id": source_id,
            "unit_id": unit_id,
            "provenance": {
                "type": "course-provided",
                "source": source_label,
                "locator": format!("app-rendered source unit {unit_id}"),
                "transformation": "Rendered locally by the app."
            },
            "rights": {
                "basis": "course-provided",
                "reuse_scope": "private-study",
                "attribution": "Course source"
            }
        }],
        "needs": [{
            "id": "need-source-render",
            "kind": "concept",
            "source_unit_ids": [unit_id],
            "procedure_step_ids": [],
            "learner_question": "What should I notice in this source unit?",
            "misconception_prevented": "Missing the source unit visual structure.",
            "evidence_requirement": "source"
        }],
        "procedure_steps": [],
        "assets": [{
            "id": "source-render-crop",
            "kind": "source-crop",
            "input_id": "source-render",
            "need_ids": ["need-source-render"],
            "learning_purpose": "Preserve the source unit visual structure.",
            "alt": "Source unit visual structure.",
            "caption": "The source unit is reproduced without decoration.",
            "explanation": "Notice how the original source organizes the concept.",
            "procedure_step_ids": [],
            "annotations": []
        }]
    });
    let path = packet_path(&plan.primary_source)?;
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&packet)
            .map_err(|error| format!("could not serialize test visual packet: {error}"))?,
    )
    .map_err(|error| {
        format!(
            "could not write test visual packet {}: {error}",
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_visual_count_stays_within_real_deck_decode_budgets() {
        let budget = 256 * 1024 * 1024;
        let four_by_three = 1_568_u64 * 1_176 * 4;
        let sixteen_by_nine = 1_568_u64 * 882 * 4;

        let algorithm_count = automatic_visual_count(117, four_by_three, budget).unwrap();
        let network_count = automatic_visual_count(69, sixteen_by_nine, budget).unwrap();
        assert_eq!(algorithm_count, 36);
        assert_eq!(network_count, 48);
        assert!(algorithm_count * usize::try_from(four_by_three).unwrap() <= budget as usize);
        assert!(network_count * usize::try_from(sixteen_by_nine).unwrap() <= budget as usize);
    }

    fn scratch_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "guide-watcher-visual-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[cfg(windows)]
    fn make_file_symlink(target: &Path, link: &Path) -> bool {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }

    #[cfg(not(windows))]
    fn make_file_symlink(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    #[cfg(windows)]
    fn make_dir_symlink(target: &Path, link: &Path) -> bool {
        std::os::windows::fs::symlink_dir(target, link).is_ok()
    }

    #[cfg(not(windows))]
    fn make_dir_symlink(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    fn overlapping_diagram(node_count: usize) -> DiagramSpec {
        DiagramSpec {
            width_px: 4096,
            height_px: 4096,
            title: "Bounded cancellation test".to_string(),
            nodes: (0..node_count)
                .map(|index| DiagramNode {
                    id: format!("node-{index}"),
                    rect: NormRect {
                        x: 0,
                        y: 0,
                        width: 10_000,
                        height: 10_000,
                    },
                    label: format!("Node {index}"),
                    tone: SemanticTone::Focus,
                    shape: DiagramShape::Box,
                })
                .collect(),
            edges: Vec::new(),
        }
    }

    fn diagram_asset(diagram: DiagramSpec) -> VisualAssetSpec {
        VisualAssetSpec {
            id: "bounded-work".to_string(),
            kind: VisualAssetKind::DeterministicDiagram,
            input_id: None,
            need_ids: vec!["need-one".to_string()],
            learning_purpose: "Explain bounded deterministic work.".to_string(),
            alt: "Diagram used to test bounded deterministic work.".to_string(),
            caption: "The renderer rejects excessive overlapping work.".to_string(),
            explanation: "Cancellation remains observable inside every long drawing loop."
                .to_string(),
            procedure_step_ids: Vec::new(),
            crop: None,
            annotations: Vec::new(),
            diagram: Some(diagram),
        }
    }

    fn render_diagram_for_test(spec: &DiagramSpec) -> Result<RgbaImage, String> {
        let cancellation = crate::process_registry::cancellation_token();
        let result = render_diagram(spec, cancellation);
        crate::process_registry::finish(cancellation);
        result
    }

    fn render_annotated_for_test(
        source: &RgbaImage,
        annotations: &[Annotation],
    ) -> Result<RgbaImage, String> {
        let cancellation = crate::process_registry::cancellation_token();
        let result = render_annotated(source, annotations, cancellation);
        crate::process_registry::finish(cancellation);
        result
    }

    fn weekly_lab_plan() -> PlannedGuide {
        PlannedGuide {
            job_id: "lab-test".to_string(),
            source_paths: vec![PathBuf::from("week-01.html")],
            primary_source: PathBuf::from("week-01.html"),
            output_path: PathBuf::from("week-01-guide.md"),
            course: crate::course_plan::ResolvedCourse {
                id: "circuit-lab".to_string(),
                label: "Circuit Lab".to_string(),
                root: PathBuf::from("."),
                guide_mode: GuideMode::WeeklyLab,
                lecture_primary_rule: crate::course_plan::LecturePrimaryRule::Any,
                expected_guide_kind: crate::course_plan::GuideKind::CircuitLab,
                profile_order: 0,
                pinned_guides: Vec::new(),
            },
            sequence_key: "week 1".to_string(),
            generation_identity: "circuit-lab:week:1".to_string(),
            predecessors: Vec::new(),
        }
    }

    fn automatic_visual_fixture(
        guide_mode: GuideMode,
    ) -> (
        PathBuf,
        PlannedGuide,
        SourceSnapshot,
        Vec<CapturedSource>,
        Vec<RenderedSourceSnapshot>,
    ) {
        let root = scratch_dir();
        let source_path = root.join(if guide_mode == GuideMode::WeeklyLab {
            "Week 3.html"
        } else {
            "CH03.pdf"
        });
        std::fs::write(&source_path, b"frozen course source").unwrap();
        let source_path = source_path.canonicalize().unwrap();
        let source_id = "source-automatic".to_string();
        let unit_id = "source-automatic-unit-001".to_string();
        let source_sha = digest(b"frozen course source");
        let mut image = RgbaImage::solid(64, 48, [240, 244, 248, 255]).unwrap();
        image.fill_rect(31, 23, 2, 2, [20, 30, 45, 255]);
        let image_bytes = encode_png(&image).unwrap();
        let plan = PlannedGuide {
            job_id: "automatic-visual-test".to_string(),
            source_paths: vec![source_path.clone()],
            primary_source: source_path.clone(),
            output_path: root.join("automatic-guide.md"),
            course: crate::course_plan::ResolvedCourse {
                id: if guide_mode == GuideMode::WeeklyLab {
                    "circuit-lab"
                } else {
                    "computer-networks"
                }
                .to_string(),
                label: "Automatic visual test".to_string(),
                root: root.clone(),
                guide_mode,
                lecture_primary_rule: crate::course_plan::LecturePrimaryRule::Any,
                expected_guide_kind: if guide_mode == GuideMode::WeeklyLab {
                    crate::course_plan::GuideKind::CircuitLab
                } else {
                    crate::course_plan::GuideKind::Lecture
                },
                profile_order: 0,
                pinned_guides: Vec::new(),
            },
            sequence_key: "3".to_string(),
            generation_identity: "automatic-visual:3".to_string(),
            predecessors: Vec::new(),
        };
        let snapshot = SourceSnapshot {
            schema_version: 2,
            primary_source_id: source_id.clone(),
            sources: vec![crate::source_context::SourceSnapshotEntry {
                id: source_id.clone(),
                path: source_path.to_string_lossy().to_string(),
                name: source_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string(),
                sha256: source_sha.clone(),
                size_bytes: b"frozen course source".len() as u64,
                role: "primary".to_string(),
                unit_kind: if guide_mode == GuideMode::WeeklyLab {
                    "html-document"
                } else {
                    "pdf-page"
                }
                .to_string(),
                unit_count: 1,
                unit_ids: vec![unit_id.clone()],
            }],
            unit_ids: vec![unit_id.clone()],
        };
        let sources = vec![CapturedSource {
            source_id: source_id.clone(),
            path: source_path,
            bytes: b"frozen course source".to_vec(),
            sha256: source_sha,
            size_bytes: b"frozen course source".len() as u64,
            unit_kind: snapshot.sources[0].unit_kind.clone(),
            unit_ids: vec![unit_id],
        }];
        let rendered = vec![RenderedSourceSnapshot {
            source_id,
            images: vec![crate::source_context::RenderedSourceImage {
                filename: "unit-001.png".to_string(),
                number: 1,
                sha256: digest(&image_bytes),
                bytes: image_bytes,
                width_px: 64,
                height_px: 48,
            }],
        }];
        (root, plan, snapshot, sources, rendered)
    }

    #[test]
    fn automatic_provenance_uses_render_position_not_filename_or_opaque_unit_id() {
        let (root, _plan, _snapshot, mut sources, mut rendered) =
            automatic_visual_fixture(GuideMode::LectureDeck);
        let unit_id =
            "source-9cc572eb6f5f7e4c5da0c62ed575b418643953defbba676f6e94ad0cc35ee05f-unit-003";
        sources[0].unit_ids = vec![unit_id.to_string()];
        rendered[0].images[0].number = 3;
        for (filename, kind, expected) in [
            ("2-What_is_OS.pdf", "pdf-page", "page 3"),
            ("Operating Systems.pdf", "pdf-page", "page 3"),
            ("Lecture 99.pptx", "pptx-slide", "slide 3"),
            ("Introduction.pptx", "pptx-slide", "slide 3"),
        ] {
            sources[0].path = root.join(filename);
            sources[0].unit_kind = kind.to_string();
            let packet = automatic_source_visual_packet(
                Vec::new(),
                &sources,
                &rendered,
                &[],
                MAX_VISUAL_DECODED_INPUT_BYTES,
            )
            .unwrap();
            let locator = packet["inputs"][0]["provenance"]["locator"]
                .as_str()
                .unwrap();
            assert_eq!(
                locator,
                format!("app-rendered {expected} (source unit {unit_id})")
            );
            assert!(!locator.contains(filename));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_packet_automatically_compiles_source_faithful_lecture_visuals() {
        let (root, plan, snapshot, sources, rendered) =
            automatic_visual_fixture(GuideMode::LectureDeck);
        let expected_packet = packet_path(&plan.primary_source).unwrap();
        assert!(!expected_packet.exists());
        let cancellation = crate::process_registry::cancellation_token();
        let mut budget = PreflightResourceBudget::new();

        let material = preflight_visual_material(
            &plan,
            &snapshot,
            VisualPreflightInputs {
                sources: &sources,
                rendered: &rendered,
                textbooks: &[],
                context: &[],
                context_procedure_steps: &[],
            },
            &mut budget,
            cancellation,
        )
        .unwrap();
        crate::process_registry::finish(cancellation);

        assert_eq!(
            material.contract.decision,
            VisualDecision::PurposefulVisuals
        );
        assert_eq!(material.contract.needs.len(), 1);
        assert_eq!(material.compiled_assets.len(), 1);
        assert_eq!(
            material.compiled_assets[0].catalog.provenance.locator,
            format!("app-rendered page 1 (source unit {})", snapshot.unit_ids[0])
        );
        assert_eq!(
            material.compiled_assets[0].catalog.kind,
            VisualAssetKind::SourceCrop
        );
        assert_eq!(
            material.compiled_assets[0].catalog.source_unit_ids,
            snapshot.unit_ids
        );
        assert!(
            !expected_packet.exists(),
            "automatic packets stay app-owned"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mixed_lecture_and_textbook_visuals_keep_primary_coverage_and_bind_exact_book_page() {
        let (root, plan, snapshot, sources, rendered) =
            automatic_visual_fixture(GuideMode::LectureDeck);
        let original_snapshot = snapshot.clone();
        let textbook_path = root.join("Meaningful textbook.pdf");
        let textbook_bytes = b"frozen meaningful textbook".to_vec();
        std::fs::write(&textbook_path, &textbook_bytes).unwrap();
        let textbook_sha256 = digest(&textbook_bytes);
        let mut image = RgbaImage::solid(72, 54, [28, 92, 156, 255]).unwrap();
        image.fill_rect(8, 9, 30, 18, [238, 242, 246, 255]);
        let book_png = encode_png(&image).unwrap();
        let textbook = CapturedTextbook {
            dependency: CapturedDependency {
                path: textbook_path.clone(),
                requested_path: Some(textbook_path.clone()),
                sha256: textbook_sha256.clone(),
                size_bytes: textbook_bytes.len() as u64,
            },
            bytes: textbook_bytes,
            page_count: 3,
            display_title: "Meaningful Systems Textbook".to_string(),
            visual_page_numbers: vec![3],
            rendered_pages: vec![crate::source_context::RenderedSourceImage {
                filename: "page-0003.png".to_string(),
                number: 3,
                sha256: digest(&book_png),
                bytes: book_png.clone(),
                width_px: 72,
                height_px: 54,
            }],
        };
        let book_unit = crate::source_context::textbook_visual_unit_id(&textbook, 3);
        let cancellation = crate::process_registry::cancellation_token();
        let mut budget = PreflightResourceBudget::new();
        let initial = preflight_visual_material(
            &plan,
            &snapshot,
            VisualPreflightInputs {
                sources: &sources,
                rendered: &rendered,
                textbooks: std::slice::from_ref(&textbook),
                context: &[],
                context_procedure_steps: &[],
            },
            &mut budget,
            cancellation,
        )
        .unwrap();
        let teacher_plans = vec![
            TeacherVisualPlan {
                source_unit_ids: snapshot.unit_ids.clone(),
                purposeful: false,
                priority: 20,
                treatment: TeacherVisualTreatment::None,
                recommendation: "The lecture render adds no useful figure.".to_string(),
                callouts: Vec::new(),
                diagram: None,
            },
            TeacherVisualPlan {
                source_unit_ids: vec![book_unit.clone()],
                purposeful: true,
                priority: 95,
                treatment: TeacherVisualTreatment::AnnotatedSource,
                recommendation: "Use the labeled relationship on this textbook page.".to_string(),
                callouts: vec![TeacherCallout {
                    x: 8_000,
                    y: 8_000,
                    number: 1,
                    text: "Legacy callout is not rendered".to_string(),
                    tone: TeacherCalloutTone::Focus,
                }],
                diagram: None,
            },
        ];
        let refined = apply_automatic_teacher_plan(
            AutomaticTeacherPlanInput {
                plan: &plan,
                snapshot: &snapshot,
                inputs: VisualPreflightInputs {
                    sources: &sources,
                    rendered: &rendered,
                    textbooks: std::slice::from_ref(&textbook),
                    context: &[],
                    context_procedure_steps: &[],
                },
                current: initial,
                teacher_plans: &teacher_plans,
            },
            cancellation,
        )
        .unwrap();
        crate::process_registry::finish(cancellation);

        assert_eq!(snapshot, original_snapshot);
        assert_eq!(refined.compiled_assets.len(), 1);
        let catalog = &refined.compiled_assets[0].catalog;
        assert_eq!(catalog.kind, VisualAssetKind::SourceCrop);
        assert!(catalog.source_unit_ids.is_empty());
        assert_eq!(catalog.provenance.source, textbook_path.to_string_lossy());
        assert_eq!(catalog.provenance.locator, "PDF page 3");
        assert_eq!(refined.compiled_assets[0].bytes, book_png);
        let valid_packet: serde_json::Value =
            serde_json::from_slice(&refined.packet_bytes).unwrap();
        assert_eq!(
            valid_packet["inputs"][0]["textbook_sha256"],
            textbook_sha256
        );
        assert_eq!(
            valid_packet["needs"][0]["source_unit_ids"],
            serde_json::json!([])
        );

        let compile = |value: serde_json::Value| {
            let mut packet_bytes = serde_json::to_vec_pretty(&value).unwrap();
            packet_bytes.push(b'\n');
            let token = crate::process_registry::cancellation_token();
            let mut budget = PreflightResourceBudget::new();
            let result = compile_visual_material_from_packet(
                VisualCompilationRequest {
                    plan: &plan,
                    snapshot: &snapshot,
                    inputs: VisualPreflightInputs {
                        sources: &sources,
                        rendered: &rendered,
                        textbooks: std::slice::from_ref(&textbook),
                        context: &[],
                        context_procedure_steps: &[],
                    },
                    packet_path: root.join("candidate-visual-packet.json"),
                    packet_bytes,
                    packet_dependency: None,
                },
                &mut budget,
                token,
            );
            crate::process_registry::finish(token);
            result.unwrap_err()
        };

        let mut near_match = valid_packet.clone();
        near_match["inputs"][0]["provenance"]["locator"] = serde_json::json!("PDF page 13");
        assert!(compile(near_match).contains("PDF page"));
        let mut unknown_book = valid_packet.clone();
        unknown_book["inputs"][0]["textbook_sha256"] = serde_json::json!("0".repeat(64));
        assert!(compile(unknown_book).contains("unknown textbook"));
        let mut invented_need = valid_packet.clone();
        invented_need["needs"][0]["source_unit_ids"] = serde_json::json!([book_unit]);
        assert!(compile(invented_need).contains("known source units"));
        let mut unrendered = valid_packet.clone();
        unrendered["inputs"][0]["page_number"] = serde_json::json!(2);
        unrendered["inputs"][0]["provenance"]["locator"] = serde_json::json!("PDF page 2");
        assert!(compile(unrendered).contains("no frozen textbook render"));
        let mut unknown_key = valid_packet;
        unknown_key["inputs"][0]["source_unit_ids"] = serde_json::json!([]);
        assert!(compile(unknown_key).contains("unknown"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sol_plan_reselects_but_preserves_automatic_source_pixels_in_production_path() {
        let (root, plan, snapshot, sources, rendered) =
            automatic_visual_fixture(GuideMode::LectureDeck);
        let cancellation = crate::process_registry::cancellation_token();
        let mut budget = PreflightResourceBudget::new();
        let material = preflight_visual_material(
            &plan,
            &snapshot,
            VisualPreflightInputs {
                sources: &sources,
                rendered: &rendered,
                textbooks: &[],
                context: &[],
                context_procedure_steps: &[],
            },
            &mut budget,
            cancellation,
        )
        .unwrap();
        let original_packet = material.packet_bytes.clone();
        let teacher_plans = vec![TeacherVisualPlan {
            source_unit_ids: snapshot.unit_ids.clone(),
            purposeful: true,
            priority: 94,
            treatment: TeacherVisualTreatment::AnnotatedSource,
            recommendation:
                "Keep the full diagram and mark the causal arrow before the endpoint label."
                    .to_string(),
            callouts: vec![TeacherCallout {
                x: 5_000,
                y: 5_000,
                number: 1,
                text: "Trace the causal arrow first".to_string(),
                tone: TeacherCalloutTone::Focus,
            }],
            diagram: None,
        }];

        let refined = apply_automatic_teacher_plan(
            AutomaticTeacherPlanInput {
                plan: &plan,
                snapshot: &snapshot,
                inputs: VisualPreflightInputs {
                    sources: &sources,
                    rendered: &rendered,
                    textbooks: &[],
                    context: &[],
                    context_procedure_steps: &[],
                },
                current: material,
                teacher_plans: &teacher_plans,
            },
            cancellation,
        )
        .unwrap();
        crate::process_registry::finish(cancellation);

        assert_ne!(refined.packet_bytes, original_packet);
        assert_eq!(refined.compiled_assets.len(), 1);
        assert_eq!(
            refined.compiled_assets[0].catalog.provenance.locator,
            format!("app-rendered page 1 (source unit {})", snapshot.unit_ids[0])
        );
        assert_eq!(
            refined.compiled_assets[0].catalog.kind,
            VisualAssetKind::SourceCrop
        );
        assert_eq!(
            refined.compiled_assets[0].catalog.source_unit_ids,
            snapshot.unit_ids
        );
        assert_eq!(
            refined.compiled_assets[0].bytes,
            rendered[0].images[0].bytes
        );
        let packet = String::from_utf8(refined.packet_bytes.clone()).unwrap();
        assert!(packet.contains("source-crop"));
        assert!(!packet.contains("annotated-source"));
        assert!(!packet.contains("deterministic-diagram"));
        let learner_text = format!(
            "{} {} {} {}",
            refined.compiled_assets[0].catalog.learning_purpose,
            refined.compiled_assets[0].catalog.alt,
            refined.compiled_assets[0].catalog.caption,
            refined.compiled_assets[0].catalog.explanation
        );
        assert!(!learner_text.contains(&snapshot.unit_ids[0]));
        assert!(!learner_text.contains("provider"));
        recheck_visual_material(&refined).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sol_diagram_plan_cannot_replace_frozen_source_pixels_in_automatic_path() {
        let (root, plan, snapshot, sources, rendered) =
            automatic_visual_fixture(GuideMode::LectureDeck);
        let cancellation = crate::process_registry::cancellation_token();
        let mut budget = PreflightResourceBudget::new();
        let material = preflight_visual_material(
            &plan,
            &snapshot,
            VisualPreflightInputs {
                sources: &sources,
                rendered: &rendered,
                textbooks: &[],
                context: &[],
                context_procedure_steps: &[],
            },
            &mut budget,
            cancellation,
        )
        .unwrap();
        let teacher_plans = vec![TeacherVisualPlan {
            source_unit_ids: snapshot.unit_ids.clone(),
            purposeful: true,
            priority: 97,
            treatment: TeacherVisualTreatment::DeterministicDiagram,
            recommendation:
                "Redraw the verified one-way relationship as a clean two-node teaching diagram."
                    .to_string(),
            callouts: Vec::new(),
            diagram: Some(DiagramSpec {
                width_px: 960,
                height_px: 640,
                title: "Verified one-way relationship".to_string(),
                nodes: vec![
                    DiagramNode {
                        id: "sender".to_string(),
                        rect: NormRect {
                            x: 800,
                            y: 3_500,
                            width: 2_800,
                            height: 2_500,
                        },
                        label: "Sender".to_string(),
                        tone: SemanticTone::Focus,
                        shape: DiagramShape::RoundedBox,
                    },
                    DiagramNode {
                        id: "receiver".to_string(),
                        rect: NormRect {
                            x: 6_400,
                            y: 3_500,
                            width: 2_800,
                            height: 2_500,
                        },
                        label: "Receiver".to_string(),
                        tone: SemanticTone::Expected,
                        shape: DiagramShape::RoundedBox,
                    },
                ],
                edges: vec![DiagramEdge {
                    from: "sender".to_string(),
                    to: "receiver".to_string(),
                    label: "data".to_string(),
                    tone: SemanticTone::Action,
                }],
            }),
        }];

        let refined = apply_automatic_teacher_plan(
            AutomaticTeacherPlanInput {
                plan: &plan,
                snapshot: &snapshot,
                inputs: VisualPreflightInputs {
                    sources: &sources,
                    rendered: &rendered,
                    textbooks: &[],
                    context: &[],
                    context_procedure_steps: &[],
                },
                current: material,
                teacher_plans: &teacher_plans,
            },
            cancellation,
        )
        .unwrap();
        crate::process_registry::finish(cancellation);

        assert_eq!(refined.compiled_assets.len(), 1);
        assert_eq!(
            refined.compiled_assets[0].catalog.kind,
            VisualAssetKind::SourceCrop
        );
        assert_eq!(
            refined.compiled_assets[0].catalog.provenance.locator,
            format!("app-rendered page 1 (source unit {})", snapshot.unit_ids[0])
        );
        assert_eq!(
            refined.compiled_assets[0].catalog.evidence_class,
            EvidenceClass::Source
        );
        assert_eq!(
            refined.compiled_assets[0].catalog.source_unit_ids,
            snapshot.unit_ids
        );
        assert_eq!(
            refined.compiled_assets[0].bytes,
            rendered[0].images[0].bytes
        );
        let packet = String::from_utf8(refined.packet_bytes.clone()).unwrap();
        assert!(packet.contains("source-crop"));
        assert!(!packet.contains("deterministic-diagram"));
        assert!(!packet.contains("annotated-source"));
        recheck_visual_material(&refined).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn automatic_teacher_diagram_rejects_overlapping_nodes() {
        let plan = TeacherVisualPlan {
            source_unit_ids: vec!["source-unit-001".to_string()],
            purposeful: true,
            priority: 90,
            treatment: TeacherVisualTreatment::DeterministicDiagram,
            recommendation: "Separate the states so their transition remains visually unambiguous."
                .to_string(),
            callouts: Vec::new(),
            diagram: Some(DiagramSpec {
                width_px: 960,
                height_px: 640,
                title: "Overlapping state test".to_string(),
                nodes: vec![
                    DiagramNode {
                        id: "state-a".to_string(),
                        rect: NormRect {
                            x: 1_000,
                            y: 2_000,
                            width: 4_000,
                            height: 3_000,
                        },
                        label: "State A".to_string(),
                        tone: SemanticTone::Focus,
                        shape: DiagramShape::Box,
                    },
                    DiagramNode {
                        id: "state-b".to_string(),
                        rect: NormRect {
                            x: 4_000,
                            y: 3_000,
                            width: 4_000,
                            height: 3_000,
                        },
                        label: "State B".to_string(),
                        tone: SemanticTone::Expected,
                        shape: DiagramShape::Box,
                    },
                ],
                edges: Vec::new(),
            }),
        };

        let error = validate_teacher_visual_plan(&plan, "source-unit-001").unwrap_err();

        assert!(error.contains("overlapping nodes"), "{error}");
    }

    fn teacher_selection_textbook(root: &Path, page_number: usize) -> CapturedTextbook {
        let textbook_path = root.join("Main textbook.pdf");
        let textbook_bytes = b"frozen main textbook".to_vec();
        std::fs::write(&textbook_path, &textbook_bytes).unwrap();
        let mut page = RgbaImage::solid(72, 54, [58, 41, 122, 255]).unwrap();
        page.fill_rect(7, 8, 31, 17, [240, 240, 245, 255]);
        let page_bytes = encode_png(&page).unwrap();
        CapturedTextbook {
            dependency: CapturedDependency {
                path: textbook_path.clone(),
                requested_path: Some(textbook_path),
                sha256: digest(&textbook_bytes),
                size_bytes: textbook_bytes.len() as u64,
            },
            bytes: textbook_bytes,
            page_count: page_number,
            display_title: "Operating Systems: Three Easy Pieces".to_string(),
            visual_page_numbers: vec![page_number],
            rendered_pages: vec![crate::source_context::RenderedSourceImage {
                filename: format!("page-{page_number:04}.png"),
                number: page_number,
                sha256: digest(&page_bytes),
                bytes: page_bytes,
                width_px: 72,
                height_px: 54,
            }],
        }
    }

    #[test]
    fn teacher_visual_selection_reserves_a_purposeful_textbook_page_from_lecture_crowding() {
        let (root, _plan, _snapshot, mut sources, mut rendered) =
            automatic_visual_fixture(GuideMode::LectureDeck);
        let mut second_slide = RgbaImage::solid(64, 48, [20, 130, 80, 255]).unwrap();
        second_slide.fill_rect(22, 12, 20, 20, [245, 245, 245, 255]);
        let second_slide = encode_png(&second_slide).unwrap();
        sources[0].unit_ids = vec!["lecture-best".to_string(), "lecture-second".to_string()];
        rendered[0]
            .images
            .push(crate::source_context::RenderedSourceImage {
                filename: "unit-002.png".to_string(),
                number: 2,
                sha256: digest(&second_slide),
                bytes: second_slide,
                width_px: 64,
                height_px: 48,
            });
        let textbook = teacher_selection_textbook(&root, 7);
        let textbook_unit = crate::source_context::textbook_visual_unit_id(&textbook, 7);
        let plan = |unit_id: &str, priority| TeacherVisualPlan {
            source_unit_ids: vec![unit_id.to_string()],
            purposeful: true,
            priority,
            treatment: TeacherVisualTreatment::SourceCrop,
            recommendation: format!(
                "The visible labels in {unit_id} show how the related parts connect."
            ),
            callouts: Vec::new(),
            diagram: None,
        };
        let plans = vec![
            plan("lecture-best", 100),
            plan("lecture-second", 90),
            plan(&textbook_unit, 40),
        ];

        let selected = select_teacher_visuals(
            &sources,
            &rendered,
            std::slice::from_ref(&textbook),
            &plans,
            2,
        )
        .unwrap();

        assert_eq!(
            selected
                .iter()
                .map(|candidate| candidate.unit_id.as_str())
                .collect::<Vec<_>>(),
            vec!["lecture-best", textbook_unit.as_str()]
        );
        assert!(matches!(
            &selected[1].input,
            RankedTeacherVisualInput::Textbook { page_number: 7, .. }
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn teacher_visual_selection_does_not_force_a_book_with_zero_capacity_or_no_purpose() {
        let (root, _plan, _snapshot, sources, rendered) =
            automatic_visual_fixture(GuideMode::LectureDeck);
        let textbook = teacher_selection_textbook(&root, 5);
        let textbook_unit = crate::source_context::textbook_visual_unit_id(&textbook, 5);
        let lecture_plan = TeacherVisualPlan {
            source_unit_ids: sources[0].unit_ids.clone(),
            purposeful: true,
            priority: 80,
            treatment: TeacherVisualTreatment::SourceCrop,
            recommendation:
                "The lecture diagram visibly connects the two operating-system concepts."
                    .to_string(),
            callouts: Vec::new(),
            diagram: None,
        };
        let purposeful_book = TeacherVisualPlan {
            source_unit_ids: vec![textbook_unit.clone()],
            purposeful: true,
            priority: 100,
            treatment: TeacherVisualTreatment::SourceCrop,
            recommendation:
                "The textbook figure visibly connects the two operating-system concepts."
                    .to_string(),
            callouts: Vec::new(),
            diagram: None,
        };
        assert!(select_teacher_visuals(
            &sources,
            &rendered,
            std::slice::from_ref(&textbook),
            &[lecture_plan.clone(), purposeful_book],
            0,
        )
        .unwrap()
        .is_empty());

        let nonpurposeful_book = TeacherVisualPlan {
            source_unit_ids: vec![textbook_unit],
            purposeful: false,
            priority: 100,
            treatment: TeacherVisualTreatment::None,
            recommendation:
                "This textbook page is text-only and adds no purposeful visual relationship."
                    .to_string(),
            callouts: Vec::new(),
            diagram: None,
        };
        let selected = select_teacher_visuals(
            &sources,
            &rendered,
            std::slice::from_ref(&textbook),
            &[lecture_plan, nonpurposeful_book],
            1,
        )
        .unwrap();
        assert_eq!(selected.len(), 1);
        assert!(matches!(
            &selected[0].input,
            RankedTeacherVisualInput::Source { .. }
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn teacher_visual_selection_ignores_decorative_slides_and_obeys_priority() {
        let (root, _plan, _snapshot, mut sources, mut rendered) =
            automatic_visual_fixture(GuideMode::LectureDeck);
        let mut diagram_image = RgbaImage::solid(64, 48, [20, 80, 140, 255]).unwrap();
        diagram_image.fill_rect(31, 23, 2, 2, [245, 245, 245, 255]);
        let diagram_bytes = encode_png(&diagram_image).unwrap();
        let mut example_image = RgbaImage::solid(64, 48, [30, 120, 60, 255]).unwrap();
        example_image.fill_rect(31, 23, 2, 2, [245, 245, 245, 255]);
        let example_bytes = encode_png(&example_image).unwrap();
        sources[0].unit_ids = vec![
            "title-slide".to_string(),
            "central-diagram".to_string(),
            "worked-example".to_string(),
        ];
        rendered[0].images = vec![
            rendered[0].images[0].clone(),
            crate::source_context::RenderedSourceImage {
                filename: "unit-002.png".to_string(),
                number: 2,
                sha256: digest(&diagram_bytes),
                bytes: diagram_bytes,
                width_px: 64,
                height_px: 48,
            },
            crate::source_context::RenderedSourceImage {
                filename: "unit-003.png".to_string(),
                number: 3,
                sha256: digest(&example_bytes),
                bytes: example_bytes,
                width_px: 64,
                height_px: 48,
            },
        ];
        let callout = |text: &str| TeacherCallout {
            x: 5_000,
            y: 5_000,
            number: 1,
            text: text.to_string(),
            tone: TeacherCalloutTone::Focus,
        };
        let plans = vec![
            TeacherVisualPlan {
                source_unit_ids: vec!["title-slide".to_string()],
                purposeful: false,
                priority: 100,
                treatment: TeacherVisualTreatment::None,
                recommendation: "Use the title only as text context; it adds no teaching visual."
                    .to_string(),
                callouts: Vec::new(),
                diagram: None,
            },
            TeacherVisualPlan {
                source_unit_ids: vec!["central-diagram".to_string()],
                purposeful: true,
                priority: 95,
                treatment: TeacherVisualTreatment::AnnotatedSource,
                recommendation: "Annotate the main relationship in the central diagram."
                    .to_string(),
                callouts: vec![callout("Start with the central relation")],
                diagram: None,
            },
            TeacherVisualPlan {
                source_unit_ids: vec!["worked-example".to_string()],
                purposeful: true,
                priority: 60,
                treatment: TeacherVisualTreatment::AnnotatedSource,
                recommendation: "Keep the worked example as secondary visual evidence.".to_string(),
                callouts: vec![callout("Check the substituted value")],
                diagram: None,
            },
        ];

        let selected = select_teacher_visuals(&sources, &rendered, &[], &plans, 1).unwrap();

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].unit_id, "central-diagram");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_callout_target_does_not_remove_a_useful_clean_source_image() {
        let (root, _plan, _snapshot, mut sources, mut rendered) =
            automatic_visual_fixture(GuideMode::LectureDeck);
        let mut useful_image = RgbaImage::solid(64, 48, [40, 90, 150, 255]).unwrap();
        useful_image.fill_rect(4, 4, 18, 12, [245, 245, 245, 255]);
        let useful_bytes = encode_png(&useful_image).unwrap();
        let mut detailed_image = RgbaImage::solid(64, 48, [30, 120, 60, 255]).unwrap();
        detailed_image.fill_rect(31, 23, 2, 2, [245, 245, 245, 255]);
        let detailed_bytes = encode_png(&detailed_image).unwrap();
        sources[0].unit_ids = vec!["useful-clean-image".to_string(), "second-image".to_string()];
        rendered[0].images = vec![
            crate::source_context::RenderedSourceImage {
                filename: "unit-001.png".to_string(),
                number: 1,
                sha256: digest(&useful_bytes),
                bytes: useful_bytes,
                width_px: 64,
                height_px: 48,
            },
            crate::source_context::RenderedSourceImage {
                filename: "unit-002.png".to_string(),
                number: 2,
                sha256: digest(&detailed_bytes),
                bytes: detailed_bytes,
                width_px: 64,
                height_px: 48,
            },
        ];
        let callout = TeacherCallout {
            x: 5_000,
            y: 5_000,
            number: 1,
            text: "Inspect the visible center detail".to_string(),
            tone: TeacherCalloutTone::Focus,
        };
        let plans = vec![
            TeacherVisualPlan {
                source_unit_ids: vec!["useful-clean-image".to_string()],
                purposeful: true,
                priority: 99,
                treatment: TeacherVisualTreatment::AnnotatedSource,
                recommendation: "Annotate the claimed detail only when the local pixels verify it."
                    .to_string(),
                callouts: vec![callout.clone()],
                diagram: None,
            },
            TeacherVisualPlan {
                source_unit_ids: vec!["second-image".to_string()],
                purposeful: true,
                priority: 70,
                treatment: TeacherVisualTreatment::AnnotatedSource,
                recommendation:
                    "Use the next ranked visual because its callout is locally grounded."
                        .to_string(),
                callouts: vec![callout],
                diagram: None,
            },
        ];

        let selected = select_teacher_visuals(&sources, &rendered, &[], &plans, 1).unwrap();

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].unit_id, "useful-clean-image");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn html_only_source_without_a_safe_render_uses_an_explicit_no_visual_contract() {
        let (root, plan, snapshot, mut sources, mut rendered) =
            automatic_visual_fixture(GuideMode::WeeklyMaterial);
        sources[0].path = root.join("Week 3 Lecture.html");
        rendered[0].images.clear();
        let cancellation = crate::process_registry::cancellation_token();
        let mut budget = PreflightResourceBudget::new();

        let material = preflight_visual_material(
            &plan,
            &snapshot,
            VisualPreflightInputs {
                sources: &sources,
                rendered: &rendered,
                textbooks: &[],
                context: &[],
                context_procedure_steps: &[],
            },
            &mut budget,
            cancellation,
        )
        .unwrap();
        crate::process_registry::finish(cancellation);

        assert_eq!(
            material.contract.decision,
            VisualDecision::NoPurposefulVisual
        );
        assert!(material.contract.needs.is_empty());
        assert!(material.compiled_assets.is_empty());
        assert!(material
            .contract
            .no_visuals_rationale
            .as_deref()
            .is_some_and(|value| value.contains("no trustworthy local render")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_lab_packet_turns_video_frames_into_step_bound_annotations() {
        let (root, plan, snapshot, sources, rendered) =
            automatic_visual_fixture(GuideMode::WeeklyLab);
        let frame_bytes =
            encode_png(&RgbaImage::solid(80, 60, [230, 238, 246, 255]).unwrap()).unwrap();
        let frame = CourseContextAsset {
            id: "meter-frame-001".to_string(),
            path: root.join("meter-frame-001.png"),
            video_title: "Week 3 meter setup".to_string(),
            timestamp_seconds: 42,
            alt: "Meter leads connected for the resistance measurement.".to_string(),
            caption: "Course video state before measuring resistance.".to_string(),
            what_to_notice: "Confirm the black lead is in COM and power is disconnected."
                .to_string(),
            procedure_step_ids: vec!["configure-meter".to_string()],
            annotations: vec![CourseContextAnnotation {
                target_x: 1800,
                target_y: 7600,
                label: "Black lead must be in the COM jack.".to_string(),
            }],
            sha256: digest(&frame_bytes),
            bytes: frame_bytes,
            width_px: 80,
            height_px: 60,
        };
        let cancellation = crate::process_registry::cancellation_token();
        let mut budget = PreflightResourceBudget::new();

        let material = preflight_visual_material(
            &plan,
            &snapshot,
            VisualPreflightInputs {
                sources: &sources,
                rendered: &rendered,
                textbooks: &[],
                context: &[frame],
                context_procedure_steps: &[CourseContextProcedureStep {
                    id: "configure-meter".to_string(),
                    action: "Configure the meter leads before measuring resistance.".to_string(),
                    kind: CourseContextProcedureStepKind::Physical,
                    transcript_segment_ids: vec!["v01-seg-0001".to_string()],
                }],
            },
            &mut budget,
            cancellation,
        )
        .unwrap();
        crate::process_registry::finish(cancellation);

        assert_eq!(material.contract.procedure_steps.len(), 1);
        assert_eq!(material.contract.procedure_steps[0].id, "configure-meter");
        assert_eq!(material.compiled_assets.len(), 1);
        let asset = &material.compiled_assets[0].catalog;
        assert_eq!(asset.kind, VisualAssetKind::AnnotatedSource);
        assert_eq!(asset.procedure_step_ids, ["configure-meter"]);
        assert!(asset.explanation.contains("black lead"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_lab_packet_fails_closed_without_captured_video_frames() {
        let (root, plan, snapshot, sources, rendered) =
            automatic_visual_fixture(GuideMode::WeeklyLab);
        let cancellation = crate::process_registry::cancellation_token();
        let mut budget = PreflightResourceBudget::new();
        let error = preflight_visual_material(
            &plan,
            &snapshot,
            VisualPreflightInputs {
                sources: &sources,
                rendered: &rendered,
                textbooks: &[],
                context: &[],
                context_procedure_steps: &[],
            },
            &mut budget,
            cancellation,
        )
        .unwrap_err();
        crate::process_registry::finish(cancellation);

        assert!(error.contains("captured LMS video frames"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }

    fn physical_step(id: &str, unit: &str, need: &str) -> ProcedureStepDefinition {
        ProcedureStepDefinition {
            id: id.to_string(),
            action: format!("Perform {id} while matching the captured course-video state."),
            kind: ProcedureStepKind::Physical,
            guide_anchor: "1-safe-setup".to_string(),
            need_ids: vec![need.to_string()],
            source_unit_ids: vec![unit.to_string()],
            required_evidence: vec![ProcedureEvidenceRole::Action],
        }
    }

    fn procedure_need(id: &str, steps: &[&str], unit: &str) -> VisualNeed {
        VisualNeed {
            id: id.to_string(),
            kind: NeedKind::ProcedureStep,
            source_unit_ids: vec![unit.to_string()],
            procedure_step_ids: steps.iter().map(|step| (*step).to_string()).collect(),
            learner_question: "Where is the safe connection point?".to_string(),
            misconception_prevented: "Connecting the probe to the wrong node.".to_string(),
            evidence_requirement: EvidenceRequirement::Source,
        }
    }

    fn annotated_spec(step: &str, need: &str) -> VisualAssetSpec {
        VisualAssetSpec {
            id: format!("asset-{step}"),
            kind: VisualAssetKind::AnnotatedSource,
            input_id: Some("source-input".to_string()),
            need_ids: vec![need.to_string()],
            learning_purpose: "Show the exact connection point.".to_string(),
            alt: "Annotated source showing the safe connection point.".to_string(),
            caption: "The highlighted node is used for this physical step.".to_string(),
            explanation: "Match the marker to the source before touching the circuit.".to_string(),
            procedure_step_ids: vec![step.to_string()],
            crop: None,
            annotations: vec![Annotation::Callout {
                target: NormPoint { x: 5000, y: 5000 },
                tone: SemanticTone::Action,
                number: 1,
                text: "Connect here".to_string(),
            }],
            diagram: None,
        }
    }

    #[test]
    fn diagram_renderer_has_a_stable_golden_hash() {
        assert_eq!(
            digest(notosans::REGULAR_TTF),
            "f1ce78a25f519ef928b5b8755ca230314e47f8821e7aa89ecf000f033f389a9f"
        );
        let diagram = DiagramSpec {
            width_px: 1200,
            height_px: 700,
            title: "Packet flow: what crosses the link".to_string(),
            nodes: vec![
                DiagramNode {
                    id: "sender".to_string(),
                    rect: NormRect {
                        x: 800,
                        y: 3000,
                        width: 2600,
                        height: 2500,
                    },
                    label: "Sender encapsulates data".to_string(),
                    tone: SemanticTone::Action,
                    shape: DiagramShape::RoundedBox,
                },
                DiagramNode {
                    id: "receiver".to_string(),
                    rect: NormRect {
                        x: 6500,
                        y: 3000,
                        width: 2600,
                        height: 2500,
                    },
                    label: "Receiver checks the frame".to_string(),
                    tone: SemanticTone::Expected,
                    shape: DiagramShape::RoundedBox,
                },
            ],
            edges: vec![DiagramEdge {
                from: "sender".to_string(),
                to: "receiver".to_string(),
                label: "frame".to_string(),
                tone: SemanticTone::Focus,
            }],
        };
        validate_diagram(&diagram).unwrap();
        let first = encode_png(&render_diagram_for_test(&diagram).unwrap()).unwrap();
        let second = encode_png(&render_diagram_for_test(&diagram).unwrap()).unwrap();
        assert_eq!(first, second);
        let hash = digest(&first);
        assert_eq!(hash, digest(&second));
        assert_eq!(
            hash,
            "725ef5b008aa1c88a84c5e8a8f128b65c5def12a630957bae30d040c02f966b3"
        );
        if let Some(path) = std::env::var_os("GUIDE_WATCHER_GOLDEN_OUTPUT") {
            std::fs::write(path, &first).unwrap();
        }
        assert_eq!(decode_png_rgba(&first).unwrap().width, 1200);
    }

    #[test]
    fn excessive_overlapping_diagram_work_is_rejected_before_rendering() {
        let diagram = overlapping_diagram(MAX_DIAGRAM_NODES);
        validate_diagram(&diagram).unwrap();
        let spec = diagram_asset(diagram);
        let mut budget = PreflightResourceBudget::new();
        let cancellation = crate::process_registry::cancellation_token();
        let started = std::time::Instant::now();
        let error = render_asset(&spec, None, &mut budget, cancellation).unwrap_err();
        crate::process_registry::finish(cancellation);
        assert!(error.contains("pixel-work budget"), "{error}");
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn high_work_production_renderer_observes_cancellation_promptly() {
        let diagram = overlapping_diagram(20);
        validate_diagram(&diagram).unwrap();
        let spec = diagram_asset(diagram);
        let cancellation = crate::process_registry::cancellation_token();
        let started = std::time::Instant::now();
        let worker = std::thread::spawn(move || {
            let mut budget = PreflightResourceBudget::new();
            render_asset(&spec, None, &mut budget, cancellation)
        });
        std::thread::sleep(std::time::Duration::from_millis(25));
        crate::process_registry::cancel(cancellation);
        let error = worker.join().unwrap().unwrap_err();
        crate::process_registry::finish(cancellation);
        assert!(error.contains("cancelled"), "{error}");
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
    }

    #[test]
    fn annotated_source_renderer_has_a_stable_golden_hash() {
        let mut source = RgbaImage::solid(900, 600, [239, 243, 248, 255]).unwrap();
        source.fill_rect(80, 170, 250, 180, [218, 225, 235, 255]);
        source.fill_rect(570, 170, 250, 180, [218, 239, 228, 255]);
        draw_rect_outline(&mut source, 80, 170, 250, 180, [65, 76, 94, 255], 4);
        draw_rect_outline(&mut source, 570, 170, 250, 180, [65, 76, 94, 255], 4);
        draw_arrow(&mut source, (330, 260), (570, 260), [65, 76, 94, 255], 3);
        draw_text(&mut source, 120, 230, "Input", 2, [35, 43, 58, 255]);
        draw_text(&mut source, 620, 230, "Output", 2, [35, 43, 58, 255]);
        let annotations = vec![
            Annotation::Focus {
                rect: NormRect {
                    x: 600,
                    y: 2500,
                    width: 3200,
                    height: 4100,
                },
                tone: SemanticTone::Action,
                label: "Measure here".to_string(),
            },
            Annotation::Arrow {
                from: NormPoint { x: 4100, y: 6600 },
                to: NormPoint { x: 6300, y: 5000 },
                tone: SemanticTone::Expected,
                label: "signal path".to_string(),
            },
            Annotation::Callout {
                target: NormPoint { x: 2150, y: 4350 },
                tone: SemanticTone::Action,
                number: 1,
                text: "Place the probe on the highlighted input node.".to_string(),
            },
            Annotation::Callout {
                target: NormPoint { x: 7700, y: 4350 },
                tone: SemanticTone::Expected,
                number: 2,
                text: "Compare the output with the expected waveform.".to_string(),
            },
        ];
        for annotation in &annotations {
            validate_annotation(annotation).unwrap();
        }
        let first = encode_png(&render_annotated_for_test(&source, &annotations).unwrap()).unwrap();
        let second =
            encode_png(&render_annotated_for_test(&source, &annotations).unwrap()).unwrap();
        assert_eq!(first, second);
        let hash = digest(&first);
        assert_eq!(
            hash,
            "8c943f22861575942da26e62e1fb75d78e4be31d9638d30823601aeadd9d85d6"
        );
        if let Some(path) = std::env::var_os("GUIDE_WATCHER_ANNOTATED_GOLDEN_OUTPUT") {
            std::fs::write(path, &first).unwrap();
        }
        let decoded = decode_png_rgba(&first).unwrap();
        assert_eq!((decoded.width, decoded.height), (1400, 640));
    }

    #[test]
    fn dense_teacher_callouts_wrap_without_clipping() {
        let mut source = RgbaImage::solid(900, 1200, [235, 240, 246, 255]).unwrap();
        for (index, color) in [
            [221, 228, 240, 255],
            [221, 239, 229, 255],
            [247, 232, 215, 255],
            [240, 221, 224, 255],
        ]
        .into_iter()
        .enumerate()
        {
            let y = 100 + index as u32 * 260;
            source.fill_rect(120, y, 620, 170, color);
            draw_rect_outline(&mut source, 120, y, 620, 170, [65, 76, 94, 255], 3);
        }
        let annotations = vec![
            Annotation::Focus {
                rect: NormRect {
                    x: 1200,
                    y: 700,
                    width: 7100,
                    height: 1800,
                },
                tone: SemanticTone::Focus,
                label: "Read this state first".to_string(),
            },
            Annotation::Callout {
                target: NormPoint { x: 2500, y: 1400 },
                tone: SemanticTone::Action,
                number: 1,
                text: "Before touching the circuit, disconnect power and verify the supply reads zero volts.".to_string(),
            },
            Annotation::Callout {
                target: NormPoint { x: 6900, y: 3600 },
                tone: SemanticTone::Warning,
                number: 2,
                text: "Match the probe ground to the reference node; the wrong ground can hide the real waveform.".to_string(),
            },
            Annotation::Callout {
                target: NormPoint { x: 2500, y: 5900 },
                tone: SemanticTone::Expected,
                number: 3,
                text: "After power is restored, expect a stable trace inside the highlighted range before recording data.".to_string(),
            },
            Annotation::Callout {
                target: NormPoint { x: 6900, y: 8200 },
                tone: SemanticTone::Recovery,
                number: 4,
                text: "If the trace clips or disappears, power down, check polarity, and repeat the measurement.".to_string(),
            },
        ];
        let first = encode_png(&render_annotated_for_test(&source, &annotations).unwrap()).unwrap();
        let second =
            encode_png(&render_annotated_for_test(&source, &annotations).unwrap()).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            digest(&first),
            "6d40cf4afa1ae1f0ad4455c4046246bab2bd764b70aff8551c3ad2a7ee28599a"
        );
        if let Some(path) = std::env::var_os("GUIDE_WATCHER_DENSE_GOLDEN_OUTPUT") {
            std::fs::write(path, &first).unwrap();
        }
        let decoded = decode_png_rgba(&first).unwrap();
        assert_eq!((decoded.width, decoded.height), (1400, 1280));
    }

    #[test]
    fn strict_packet_rejects_unknown_fields_and_unsafe_input_paths() {
        let unknown = br#"{"schema_version":2,"sources":[],"decision":"no-purposeful-visual","no_visuals_rationale":"text-only","inputs":[],"needs":[],"procedure_steps":[],"assets":[],"extra":true}"#;
        assert!(serde_json::from_slice::<VisualPacket>(unknown).is_err());
        let nested_unknown = serde_json::json!({
            "schema_version": 2,
            "sources": [],
            "decision": "purposeful-visuals",
            "inputs": [{
                "kind": "local-png", "id": "input-one", "path": "one.png",
                "sha256": "a".repeat(64), "width_px": 1, "height_px": 1,
                "evidence_class": "source",
                "provenance": {"type": "book-source", "source": "book", "locator": "page 1", "transformation": "none", "command": "bad"},
                "rights": {"basis": "licensed", "reuse_scope": "private-study", "attribution": "book"}
            }],
            "needs": [], "procedure_steps": [], "assets": []
        });
        assert!(validate_packet_keys(&nested_unknown)
            .unwrap_err()
            .contains("unknown field"));
        for path in [
            "../x.png",
            "/x.png",
            "https://example/x.png",
            "x.svg",
            "a\\x.png",
        ] {
            assert!(safe_relative_path(path).is_err(), "accepted {path}");
        }
    }

    #[test]
    fn visual_capture_rejects_packet_and_local_input_links() {
        let root = scratch_dir();
        let target = root.join("packet-target.json");
        std::fs::write(&target, b"{}").unwrap();
        let packet_link = root.join("lecture.pdf.guide-visuals.json");
        if make_file_symlink(&target, &packet_link) {
            let mut budget = PreflightResourceBudget::new();
            let error =
                capture_visual_file(&packet_link, MAX_PACKET_BYTES, "visual packet", &mut budget)
                    .unwrap_err();
            assert!(
                error.contains("symlink, junction, or reparse point"),
                "{error}"
            );
        }

        let real_subdirectory = root.join("real-subdirectory");
        std::fs::create_dir(&real_subdirectory).unwrap();
        std::fs::write(real_subdirectory.join("input.png"), b"not needed").unwrap();
        let local_root = root.join("lecture.pdf.guide-visuals");
        std::fs::create_dir(&local_root).unwrap();
        let linked_subdirectory = local_root.join("linked");
        if make_dir_symlink(&real_subdirectory, &linked_subdirectory) {
            let mut budget = PreflightResourceBudget::new();
            let error = capture_visual_file(
                &linked_subdirectory.join("input.png"),
                MAX_INPUT_BYTES,
                "visual local input",
                &mut budget,
            )
            .unwrap_err();
            assert!(
                error.contains("symlink, junction, or reparse point"),
                "{error}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn visual_recheck_rejects_requested_path_replaced_by_a_link() {
        let root = scratch_dir();
        let requested = root.join("lecture.pdf.guide-visuals.json");
        let bytes = b"{}".to_vec();
        std::fs::write(&requested, &bytes).unwrap();
        let mut budget = PreflightResourceBudget::new();
        let (_, dependency) =
            capture_visual_file(&requested, MAX_PACKET_BYTES, "visual packet", &mut budget)
                .unwrap();
        let packet_sha256 = digest(&bytes);
        let material = VisualMaterial {
            packet_path: requested.clone(),
            packet_bytes: bytes.clone(),
            packet_sha256: packet_sha256.clone(),
            dependencies: vec![dependency],
            contract: VisualContract {
                schema_version: 2,
                packet_sha256,
                decision: VisualDecision::NoPurposefulVisual,
                no_visuals_rationale: Some("No visual is useful for this fixture.".to_string()),
                needs: Vec::new(),
                procedure_steps: Vec::new(),
                assets: Vec::new(),
            },
            compiled_assets: Vec::new(),
        };
        let replacement = root.join("replacement.json");
        std::fs::write(&replacement, &bytes).unwrap();
        std::fs::remove_file(&requested).unwrap();
        if make_file_symlink(&replacement, &requested) {
            let error = recheck_visual_material(&material).unwrap_err();
            assert!(
                error.contains("symlink, junction, or reparse point"),
                "{error}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cumulative_decoded_input_budget_rejects_many_individually_valid_images() {
        let mut budget = VisualDecodeBudget::new();
        for _ in 0..4 {
            budget
                .charge(crate::png_validation::MAX_PNG_DECODED_BYTES)
                .unwrap();
        }
        let error = budget.charge(4).unwrap_err();
        assert!(error.contains("cumulative decoded-RGBA budget"), "{error}");
    }

    #[test]
    fn normalized_geometry_and_diagram_references_are_bounded() {
        assert!(validate_rect(
            NormRect {
                x: 9_999,
                y: 0,
                width: 2,
                height: 10,
            },
            "test",
        )
        .is_err());
        let diagram = DiagramSpec {
            width_px: 640,
            height_px: 360,
            title: "Bounded".to_string(),
            nodes: vec![DiagramNode {
                id: "known".to_string(),
                rect: NormRect {
                    x: 100,
                    y: 100,
                    width: 2000,
                    height: 2500,
                },
                label: "Known".to_string(),
                tone: SemanticTone::Focus,
                shape: DiagramShape::Box,
            }],
            edges: vec![DiagramEdge {
                from: "known".to_string(),
                to: "missing".to_string(),
                label: "bad".to_string(),
                tone: SemanticTone::Wrong,
            }],
        };
        assert!(validate_diagram(&diagram)
            .unwrap_err()
            .contains("unknown node"));
    }

    #[test]
    fn diagram_title_wraps_instead_of_rejecting_a_teacher_facing_title() {
        let diagram = DiagramSpec {
            width_px: 960,
            height_px: 640,
            title: "From program to process: how the operating system creates, schedules, isolates, and stops running work"
                .to_string(),
            nodes: vec![DiagramNode {
                id: "process".to_string(),
                rect: NormRect {
                    x: 1_000,
                    y: 3_000,
                    width: 8_000,
                    height: 3_000,
                },
                label: "A running program managed by the operating system".to_string(),
                tone: SemanticTone::Focus,
                shape: DiagramShape::RoundedBox,
            }],
            edges: Vec::new(),
        };

        validate_diagram(&diagram).unwrap();
        render_diagram_for_test(&diagram).unwrap();
    }

    #[test]
    fn diagram_node_label_wraps_using_rendered_width() {
        let diagram = DiagramSpec {
            width_px: 640,
            height_px: 360,
            title: "Node load".to_string(),
            nodes: vec![DiagramNode {
                id: "node-load".to_string(),
                rect: NormRect {
                    x: 1_000,
                    y: 3_000,
                    width: 2_500,
                    height: 3_500,
                },
                label: "WWWWWWWWWWWW".to_string(),
                tone: SemanticTone::Focus,
                shape: DiagramShape::RoundedBox,
            }],
            edges: Vec::new(),
        };

        validate_diagram(&diagram).unwrap();
        render_diagram_for_test(&diagram).unwrap();
    }

    #[test]
    fn diagram_node_label_uses_the_largest_readable_scale_that_fits() {
        let diagram = DiagramSpec {
            width_px: 640,
            height_px: 360,
            title: "Task states".to_string(),
            nodes: vec![DiagramNode {
                id: "tasks".to_string(),
                rect: NormRect {
                    x: 1_000,
                    y: 3_000,
                    width: 2_500,
                    height: 2_250,
                },
                label: "A ready task waits in the operating system run queue".to_string(),
                tone: SemanticTone::Focus,
                shape: DiagramShape::RoundedBox,
            }],
            edges: Vec::new(),
        };

        validate_diagram(&diagram).unwrap();
        let (_, scale) = diagram_node_label_layout(&diagram.nodes[0].label, 160, 81).unwrap();
        assert_eq!(scale, 1);
        render_diagram_for_test(&diagram).unwrap();
    }

    #[test]
    fn diagram_edge_label_wraps_using_rendered_width() {
        let node = |id: &str, x: u16| DiagramNode {
            id: id.to_string(),
            rect: NormRect {
                x,
                y: 3_500,
                width: 1_500,
                height: 2_000,
            },
            label: id.to_string(),
            tone: SemanticTone::Focus,
            shape: DiagramShape::RoundedBox,
        };
        let diagram = DiagramSpec {
            width_px: 640,
            height_px: 360,
            title: "Task dispatch".to_string(),
            nodes: vec![node("ready", 4_500), node("running", 7_000)],
            edges: vec![DiagramEdge {
                from: "ready".to_string(),
                to: "running".to_string(),
                label: "ready tasks wait for dispatch".to_string(),
                tone: SemanticTone::Action,
            }],
        };

        validate_diagram(&diagram).unwrap();
        render_diagram_for_test(&diagram).unwrap();
    }

    #[test]
    fn diagram_validation_rejects_all_text_overflow_paths_and_accepts_dense_wrapping() {
        let node = |id: &str, x: u16, y: u16, width: u16, height: u16, label: &str| DiagramNode {
            id: id.to_string(),
            rect: NormRect {
                x,
                y,
                width,
                height,
            },
            label: label.to_string(),
            tone: SemanticTone::Focus,
            shape: DiagramShape::RoundedBox,
        };
        let title_overflow = DiagramSpec {
            width_px: 320,
            height_px: 240,
            title: "W".repeat(160),
            nodes: vec![node("one", 500, 2500, 4000, 4000, "Fits")],
            edges: Vec::new(),
        };
        assert!(validate_diagram(&title_overflow)
            .unwrap_err()
            .contains("title does not fit"));

        let node_overflow = DiagramSpec {
            width_px: 640,
            height_px: 360,
            title: "Node fit".to_string(),
            nodes: vec![node(
                "tiny",
                500,
                2500,
                500,
                500,
                &"long physical procedure wording ".repeat(5),
            )],
            edges: Vec::new(),
        };
        assert!(validate_diagram(&node_overflow)
            .unwrap_err()
            .contains("node label does not fit"));

        let edge_overflow = DiagramSpec {
            width_px: 640,
            height_px: 360,
            title: "Edge fit".to_string(),
            nodes: vec![
                node("one", 6000, 2500, 1800, 2500, "First"),
                node("two", 8000, 2500, 1800, 2500, "Second"),
            ],
            edges: vec![DiagramEdge {
                from: "one".to_string(),
                to: "two".to_string(),
                label: "This edge label is intentionally far too long for the remaining canvas"
                    .to_string(),
                tone: SemanticTone::Warning,
            }],
        };
        assert!(validate_diagram(&edge_overflow)
            .unwrap_err()
            .contains("edge label does not fit"));

        let dense_valid = DiagramSpec {
            width_px: 1600,
            height_px: 1000,
            title: "Teacher walkthrough of a safe measurement sequence".to_string(),
            nodes: vec![
                node(
                    "before",
                    400,
                    2600,
                    2100,
                    4200,
                    "Disconnect power and inspect every connection",
                ),
                node(
                    "action",
                    2800,
                    2600,
                    2100,
                    4200,
                    "Select voltage mode before placing probes",
                ),
                node(
                    "expected",
                    5200,
                    2600,
                    2100,
                    4200,
                    "Restore power and observe a stable reading",
                ),
                node(
                    "recovery",
                    7600,
                    2600,
                    2100,
                    4200,
                    "Power down and correct polarity if unstable",
                ),
            ],
            edges: vec![
                DiagramEdge {
                    from: "before".to_string(),
                    to: "action".to_string(),
                    label: "then".to_string(),
                    tone: SemanticTone::Action,
                },
                DiagramEdge {
                    from: "action".to_string(),
                    to: "expected".to_string(),
                    label: "observe".to_string(),
                    tone: SemanticTone::Expected,
                },
                DiagramEdge {
                    from: "expected".to_string(),
                    to: "recovery".to_string(),
                    label: "if wrong".to_string(),
                    tone: SemanticTone::Recovery,
                },
            ],
        };
        validate_diagram(&dense_valid).unwrap();
        render_diagram_for_test(&dense_valid).unwrap();
    }

    #[test]
    fn diagram_rejects_self_duplicate_and_repeated_zero_length_labeled_edges() {
        let node = |id: &str, x: u16, y: u16, width: u16, height: u16| DiagramNode {
            id: id.to_string(),
            rect: NormRect {
                x,
                y,
                width,
                height,
            },
            label: id.to_string(),
            tone: SemanticTone::Focus,
            shape: DiagramShape::RoundedBox,
        };
        let edge = |from: &str, to: &str, label: &str| DiagramEdge {
            from: from.to_string(),
            to: to.to_string(),
            label: label.to_string(),
            tone: SemanticTone::Action,
        };
        let base = |nodes: Vec<DiagramNode>, edges: Vec<DiagramEdge>| DiagramSpec {
            width_px: 1200,
            height_px: 700,
            title: "Bounded edge work".to_string(),
            nodes,
            edges,
        };

        let self_edge = base(
            vec![node("a", 1000, 3000, 2000, 2500)],
            vec![edge("a", "a", &"long label ".repeat(10))],
        );
        assert!(validate_diagram(&self_edge)
            .unwrap_err()
            .contains("self-edges"));

        let duplicate = base(
            vec![
                node("a", 1000, 3000, 2000, 2500),
                node("b", 7000, 3000, 2000, 2500),
            ],
            vec![edge("a", "b", "first"), edge("a", "b", "second")],
        );
        assert!(validate_diagram(&duplicate)
            .unwrap_err()
            .contains("duplicate directed edges"));

        let zero_length = base(
            vec![
                node("a", 2000, 2500, 2000, 3000),
                node("b", 2500, 3000, 1000, 2000),
            ],
            (0..MAX_DIAGRAM_EDGES)
                .map(|index| edge("a", "b", &format!("edge label {index:02}")))
                .collect(),
        );
        assert!(validate_diagram(&zero_length)
            .unwrap_err()
            .contains("zero visual length"));
    }

    #[test]
    fn worst_case_text_work_charge_bounds_every_supported_glyph_raster() {
        for scale in [2, 3] {
            for character in ' '..='~' {
                let (metrics, bitmap) = teacher_font().rasterize(character, font_px(scale));
                assert_eq!(bitmap.len(), metrics.width * metrics.height);
                assert!(
                    bitmap.len() as u64 <= WORST_CASE_GLYPH_PIXEL_WORK,
                    "glyph {character:?} at scale {scale} exceeds the charged bound"
                );
            }
        }
        let text = "Every charged ASCII byte bounds one complete glyph raster pass.";
        assert_eq!(
            estimate_text_pixel_work(text).unwrap(),
            text.len() as u64 * WORST_CASE_GLYPH_PIXEL_WORK
        );
    }

    #[test]
    fn learner_text_rejects_markdown_breakout_multiline_and_overlength_values() {
        for unsafe_text in [
            "Alt closes the bracket ] then injects a link",
            "Caption line one\nCaption line two",
            "Entity ambiguity &copy;",
            "Escaped ambiguity \\ bracket",
            "Inline `code` cannot be learner visual text",
        ] {
            assert!(require_learner_text(unsafe_text, "learner text", 320).is_err());
        }
        assert!(require_learner_text(&"x".repeat(321), "learner text", 320).is_err());
        require_learner_text(
            "Annotated source showing the meter in voltage mode.",
            "learner text",
            320,
        )
        .unwrap();
    }

    #[test]
    fn shared_visual_text_contract_vectors_match_the_python_verifier() {
        let fixture_path = Path::new(&crate::config::guide_lint_script())
            .with_file_name("visual_text_contract_cases.json");
        let fixture: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&fixture_path).expect("read shared visual text contract fixture"),
        )
        .expect("parse shared visual text contract fixture");
        for (group, accepted) in [("accepted", true), ("rejected", false)] {
            for case in fixture[group].as_array().expect("contract case list") {
                let value = case["value"].as_str().expect("contract value");
                let max = case["max_bytes"].as_u64().expect("contract max") as usize;
                assert_eq!(
                    require_learner_text(value, "learner text", max).is_ok(),
                    accepted,
                    "shared contract mismatch for {value:?}"
                );
            }
        }
    }

    #[test]
    fn procedure_sources_require_exact_app_owned_step_mapping() {
        let plan = weekly_lab_plan();
        let need = procedure_need("need-connect", &["connect-probe"], "unit-001");
        let step = physical_step("connect-probe", "unit-001", "need-connect");
        let spec = annotated_spec("connect-probe", "need-connect");
        let source_input = BoundInput {
            id: "source-input".to_string(),
            rgba: RgbaImage::solid(32, 32, [255, 255, 255, 255]).unwrap(),
            evidence_class: EvidenceClass::Source,
            source_unit_ids: vec!["unit-001".to_string()],
            procedure_step_ids: Vec::new(),
            origin: InputOrigin::SourceUnit,
            provenance: VisualProvenance {
                kind: "course-source".to_string(),
                source: "week-01.html".to_string(),
                locator: "unit-001".to_string(),
                transformation: "rendered".to_string(),
            },
            rights: RightsMetadata {
                basis: RightsBasis::CourseProvided,
                reuse_scope: ReuseScope::PrivateStudy,
                attribution: "Course source".to_string(),
            },
        };
        validate_asset_semantics(
            &spec,
            Some(&source_input),
            &[&need],
            std::slice::from_ref(&step),
            &plan,
        )
        .unwrap();

        let mut unmapped_step = step.clone();
        unmapped_step.source_unit_ids.clear();
        let error = validate_asset_semantics(
            &spec,
            Some(&source_input),
            &[&need],
            &[unmapped_step],
            &plan,
        )
        .unwrap_err();
        assert!(error.contains("not explicitly mapped"), "{error}");

        let mut frame_input = source_input;
        frame_input.origin = InputOrigin::ContextFrame;
        frame_input.source_unit_ids.clear();
        frame_input.procedure_step_ids = vec!["different-step".to_string()];
        let error = validate_asset_semantics(&spec, Some(&frame_input), &[&need], &[step], &plan)
            .unwrap_err();
        assert!(
            error.contains("not present on its source context frame"),
            "{error}"
        );
    }

    #[test]
    fn one_need_covering_two_steps_cannot_hide_a_missing_physical_primary() {
        let need = procedure_need(
            "need-two-steps",
            &["connect-probe", "energize-circuit"],
            "unit-001",
        );
        let steps = vec![
            physical_step("connect-probe", "unit-001", "need-two-steps"),
            physical_step("energize-circuit", "unit-001", "need-two-steps"),
        ];
        let packet = VisualPacket {
            schema_version: 2,
            sources: Vec::new(),
            decision: VisualDecision::PurposefulVisuals,
            no_visuals_rationale: None,
            inputs: Vec::new(),
            needs: vec![need],
            procedure_steps: steps,
            assets: Vec::new(),
        };
        let asset = CompiledVisualAsset {
            catalog: VisualCatalogEntry {
                id: "asset-connect-probe".to_string(),
                filename: "asset-connect-probe.png".to_string(),
                sha256: "a".repeat(64),
                spec_sha256: "b".repeat(64),
                width_px: 32,
                height_px: 32,
                kind: VisualAssetKind::AnnotatedSource,
                evidence_class: EvidenceClass::Source,
                need_ids: vec!["need-two-steps".to_string()],
                source_unit_ids: vec!["unit-001".to_string()],
                procedure_step_ids: vec!["connect-probe".to_string()],
                learning_purpose: "Show the connection.".to_string(),
                alt: "Annotated connection point.".to_string(),
                caption: "Connect the probe here.".to_string(),
                explanation: "Use the source marker.".to_string(),
                provenance: VisualProvenance {
                    kind: "annotated-source".to_string(),
                    source: "week-01.html".to_string(),
                    locator: "unit-001".to_string(),
                    transformation: "annotated".to_string(),
                },
                rights: RightsMetadata {
                    basis: RightsBasis::CourseProvided,
                    reuse_scope: ReuseScope::PrivateStudy,
                    attribution: "Course source".to_string(),
                },
            },
            bytes: encode_png(&RgbaImage::solid(32, 32, [255, 255, 255, 255]).unwrap()).unwrap(),
        };
        let error = validate_need_coverage(&packet, &[asset], &weekly_lab_plan()).unwrap_err();
        assert!(error.contains("energize-circuit"), "{error}");
    }

    #[test]
    fn rgba_decoder_rejects_truncated_png_after_a_valid_header() {
        let image = RgbaImage::solid(2, 2, [1, 2, 3, 255]).unwrap();
        let mut bytes = encode_png(&image).unwrap();
        bytes.truncate(40);
        assert!(decode_png_rgba(&bytes).is_err());
    }

    #[test]
    fn rgba_decoder_rejects_bytes_after_iend() {
        let image = RgbaImage::solid(2, 2, [1, 2, 3, 255]).unwrap();
        let mut bytes = encode_png(&image).unwrap();
        bytes.extend_from_slice(b"hidden trailing payload");
        assert!(decode_png_rgba(&bytes).is_err());
    }
}
