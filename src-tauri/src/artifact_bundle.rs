use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Digest;
use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::source_context::{decode_png_dimensions, PredecessorManifest, SourceSnapshot};
use crate::visual_assets::{
    EvidenceClass, ProcedureStepDefinition, RightsMetadata, VisualAssetKind, VisualCatalogEntry,
    VisualMaterial, VisualProvenance,
};

pub const ARTIFACTS_START: &str = "<<<GUIDE_WATCHER_ARTIFACTS_V1>>>";
pub const ARTIFACTS_END: &str = "<<<END_GUIDE_WATCHER_ARTIFACTS_V1>>>";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SupplementaryResource {
    pub sha256: String,
    pub page_count: usize,
    pub title: String,
    pub primary: bool,
}

pub fn supplementary_research_instructions(resources: &[SupplementaryResource]) -> String {
    let resource_metadata = serde_json::to_string(resources)
        .expect("supplementary resource metadata must serialize to JSON");
    let primary_guidance = if resources.iter().any(|resource| {
        resource.primary && resource.title == "Operating Systems: Three Easy Pieces"
    }) {
        " If the lecture explicitly assigns Operating Systems: Three Easy Pieces Chapter 2, follow that assignment and connect its explanation of CPU virtualization, private address spaces, concurrency, and persistence to the corresponding guide sections."
    } else {
        ""
    };
    let example = r#"{"research_notes":[{"sha256":"...","status":"used","pages":[3],"locator":"Chapter 2","explanation":"...","guide_heading":"CPU virtualization","evidence":"The operating system ...","integrations":[{"pages":[3],"locator":"Chapter 2, section ...","guide_heading":"CPU virtualization","evidence":"The operating system ..."},{"pages":[4],"locator":"Chapter 2, section ...","guide_heading":"Private address spaces","evidence":"Each process ..."}]}]}"#;
    format!(
        "APP-OWNED SUPPLEMENTARY RESOURCE ASSESSMENTS: {resource_metadata}\nEach object gives the exact frozen SHA-256, PDF page count, content-recognized title, and whether that book is primary. In coverage_manifest.research_notes include exactly one object per resource, with sha256, status (used or not_relevant), pages (nonempty unique 1-based PDF page positions actually assessed), locator (chapter/section detail), and explanation (specific evidence and relevance to this lecture). For status used also supply guide_heading (the exact visible heading text only, without leading Markdown # markers) and evidence (an exact nonempty Markdown substring from an explanatory paragraph under that heading which uses this resource; preserve any Markdown formatting in the substring, and do not quote a code block, image, or HTML comment). For not_relevant explain why the inspected content does not support this lecture. A primary resource must be used and is central to the guide. Its research_notes object must also contain an integrations array with at least two entries. Every entry must contain pages (nonempty unique in-range 1-based PDF page positions actually read), locator (chapter/section), guide_heading (an exact visible guide heading), and evidence (an exact explanatory Markdown paragraph excerpt under that heading that teaches with the book's material). integrations must use distinct guide headings and distinct evidence excerpts. Honor lecture-assigned chapters or sections before heuristic excerpts: read the actual relevant complete sections and distinguish PDF positions from printed page labels. The guide body never tells the student what to read; provenance lives in research_notes and in parenthetical citations. Use the main book throughout the matching guide sections, connect its code or examples to the lecture output when present, and assess other books separately, using them only where they add distinct relevant value.{primary_guidance} Compact primary example (replace every value with exact frozen evidence): {example}. Assess all supplied resources, never invent page citations, and never force unrelated content into the guide. Use an empty research_notes list when no resources are supplied."
    )
}

pub fn validate_supplementary_research(
    coverage: &Value,
    guide: &str,
    resources: &[SupplementaryResource],
) -> Result<(), String> {
    if resources.iter().any(|resource| {
        resource.primary && (resource.sha256.trim().is_empty() || resource.title.trim().is_empty())
    }) {
        return Err(
            "primary supplementary resource metadata requires a known SHA-256 and title".into(),
        );
    }
    let notes = coverage.get("research_notes").and_then(Value::as_array);
    if resources.is_empty() && notes.is_none() {
        return Ok(());
    }
    let notes = notes.ok_or("research_notes must assess every supplied supplementary resource")?;
    if notes.len() != resources.len() {
        return Err(
            "research_notes must contain exactly one assessment per supplementary resource".into(),
        );
    }
    let mut seen = HashSet::new();
    for note in notes {
        let sha = note.get("sha256").and_then(Value::as_str).unwrap_or("");
        let resource = resources
            .iter()
            .find(|resource| resource.sha256 == sha)
            .ok_or_else(|| format!("research_notes contains unknown resource SHA-256: {sha}"))?;
        if !seen.insert(sha) {
            return Err(format!("research_notes repeats resource {sha}"));
        }
        for field in ["locator", "explanation"] {
            if note
                .get(field)
                .and_then(Value::as_str)
                .is_none_or(|s| s.trim().is_empty())
            {
                return Err(format!("research_notes {sha} requires nonempty {field}"));
            }
        }
        validate_resource_pages(
            note.get("pages"),
            resource.page_count,
            &format!("research_notes {sha}"),
        )?;
        match note.get("status").and_then(Value::as_str) {
            Some("not_relevant") if resource.primary => {
                return Err(format!(
                    "research_notes {sha} is the primary textbook and must have status used"
                ));
            }
            Some("not_relevant") => {}
            Some("used") => {
                let heading = note
                    .get("guide_heading")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let evidence = note
                    .get("evidence")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                if heading.trim().is_empty() || evidence.is_empty() {
                    return Err(format!("research_notes {sha} must quote guide evidence under its real guide_heading; guide_heading must contain visible text only, without Markdown # markers"));
                }
                if !heading_contains_evidence(guide, heading, evidence)? {
                    return Err(format!("research_notes {sha} must quote guide evidence under its real guide_heading; guide_heading must contain visible text only, without Markdown # markers"));
                }
                if resource.primary {
                    validate_primary_resource_use(note, guide, resource)?;
                }
            }
            _ => {
                return Err(format!(
                    "research_notes {sha} status must be used or not_relevant"
                ))
            }
        }
    }
    Ok(())
}

fn validate_resource_pages(
    value: Option<&Value>,
    page_count: usize,
    context: &str,
) -> Result<(), String> {
    let pages = value
        .and_then(Value::as_array)
        .filter(|pages| !pages.is_empty())
        .ok_or_else(|| format!("{context} requires nonempty PDF pages"))?;
    let mut seen_pages = HashSet::new();
    for page in pages {
        let page = page
            .as_u64()
            .filter(|page| *page > 0 && *page <= page_count as u64)
            .ok_or_else(|| {
                format!(
                    "{context} page must be a 1-based position within the {page_count}-page frozen PDF"
                )
            })?;
        if !seen_pages.insert(page) {
            return Err(format!("{context} repeats PDF page {page}"));
        }
    }
    Ok(())
}

fn validate_primary_resource_use(
    note: &Value,
    guide: &str,
    resource: &SupplementaryResource,
) -> Result<(), String> {
    let sha = &resource.sha256;
    // A `reading_path` used to be required here, and its "evidence must visibly tell the
    // student what to read" rule is what put "First, read Chapter 2 at PDF pages 44-49"
    // sections into every guide. The guide is the reading; only integrations are checked.
    let integrations = note
        .get("integrations")
        .and_then(Value::as_array)
        .filter(|entries| entries.len() >= 2)
        .ok_or_else(|| {
            format!("research_notes {sha} is primary and requires at least two integrations")
        })?;
    let mut headings = HashSet::new();
    let mut evidence_excerpts = HashSet::new();
    for (index, entry) in integrations.iter().enumerate() {
        let (heading, evidence) = validate_primary_resource_entry(
            entry,
            guide,
            resource,
            &format!("integrations[{index}]"),
        )?;
        if !headings.insert(heading) {
            return Err(format!(
                "research_notes {sha} integrations must use distinct real guide headings"
            ));
        }
        if !evidence_excerpts.insert(evidence) {
            return Err(format!(
                "research_notes {sha} integrations must use distinct exact explanatory evidence"
            ));
        }
    }
    Ok(())
}

fn validate_primary_resource_entry(
    entry: &Value,
    guide: &str,
    resource: &SupplementaryResource,
    entry_name: &str,
) -> Result<(String, String), String> {
    let context = format!("research_notes {} {entry_name}", resource.sha256);
    validate_resource_pages(entry.get("pages"), resource.page_count, &context)?;
    let locator = entry.get("locator").and_then(Value::as_str).unwrap_or("");
    if locator.trim().is_empty() {
        return Err(format!(
            "{context} requires a nonempty chapter/section locator"
        ));
    }
    let heading = entry
        .get("guide_heading")
        .and_then(Value::as_str)
        .unwrap_or("");
    let evidence = entry
        .get("evidence")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if heading.trim().is_empty() || evidence.is_empty() {
        return Err(format!(
            "{context} must quote exact explanatory paragraph evidence under its visible guide_heading"
        ));
    }
    if !heading_contains_evidence(guide, heading, evidence)? {
        return Err(format!(
            "{context} must quote exact explanatory paragraph evidence under its visible guide_heading"
        ));
    }
    Ok((heading.to_string(), evidence.to_string()))
}

fn heading_contains_evidence(guide: &str, expected: &str, evidence: &str) -> Result<bool, String> {
    use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};
    let mut heading = None::<(HeadingLevel, String)>;
    let mut section_level = None::<HeadingLevel>;
    let mut matching_headings = 0;
    let mut found_evidence = false;
    let mut paragraph_start = None;
    let mut paragraph_has_html_or_image = false;
    for (event, range) in Parser::new(guide).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                if section_level.is_some_and(|owner_level| level <= owner_level) {
                    section_level = None;
                }
                heading = Some((level, String::new()));
            }
            Event::Text(text) if heading.is_some() => {
                heading.as_mut().unwrap().1.push_str(&text);
            }
            Event::Code(text) if heading.is_some() => {
                heading.as_mut().unwrap().1.push_str(&text);
            }
            Event::End(TagEnd::Heading(_)) => {
                let (level, text) = heading.take().ok_or_else(|| {
                    "Markdown parser ended a heading that was not open".to_string()
                })?;
                if text == expected {
                    matching_headings += 1;
                    section_level = Some(level);
                }
            }
            Event::Start(Tag::Paragraph) => {
                paragraph_start = Some(range.start);
                paragraph_has_html_or_image = false;
            }
            Event::Html(_) | Event::InlineHtml(_) | Event::Start(Tag::Image { .. }) => {
                paragraph_has_html_or_image = true;
            }
            Event::End(TagEnd::Paragraph) => {
                if let Some(start) = paragraph_start.take() {
                    if section_level.is_some()
                        && !paragraph_has_html_or_image
                        && guide[start..range.end].contains(evidence)
                    {
                        found_evidence = true;
                    }
                }
            }
            _ => {}
        }
    }
    if matching_headings > 1 {
        return Err(format!(
            "guide_heading {expected:?} is ambiguous because it matches multiple visible Markdown headings"
        ));
    }
    Ok(found_evidence)
}

fn canonicalize_typographic_dashes(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            '\u{2010}' | '\u{2011}' | '\u{2013}' | '\u{2014}' => '-',
            _ => character,
        })
        .collect()
}

fn canonicalize_markdown_heading_dashes(markdown: &str) -> Result<String, String> {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};

    let mut in_heading = false;
    let mut edits = Vec::new();
    for (event, range) in Parser::new(markdown).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { .. }) => in_heading = true,
            Event::End(TagEnd::Heading(_)) => in_heading = false,
            Event::Text(text) if in_heading => {
                let original = &markdown[range.clone()];
                let visible_dash_count = text.chars().filter(|c| is_typographic_dash(*c)).count();
                let source_dash_count =
                    original.chars().filter(|c| is_typographic_dash(*c)).count();
                if visible_dash_count != source_dash_count {
                    return Err(
                        "Markdown heading contains an unsupported encoded typographic dash; use the literal Unicode character or ASCII hyphen"
                            .to_string(),
                    );
                }
                let canonical = canonicalize_typographic_dashes(original);
                if canonical != original {
                    edits.push((range.start, range.end, canonical));
                }
            }
            _ => {}
        }
    }
    apply_markdown_edits(markdown, edits)
}

fn is_typographic_dash(character: char) -> bool {
    matches!(character, '\u{2010}' | '\u{2011}' | '\u{2013}' | '\u{2014}')
}

#[derive(Debug)]
struct HeadingTarget {
    visible: String,
    anchor: Option<String>,
}

fn canonicalize_heading_references(
    markdown: &str,
    coverage: &mut Value,
    placements: &mut [VisualPlacement],
) -> Result<String, String> {
    let original_headings = heading_targets(markdown)?;
    let canonical = canonicalize_markdown_heading_dashes(markdown)?;
    let canonical_headings = heading_targets(&canonical)?;
    if original_headings.len() != canonical_headings.len() {
        return Err(
            "heading structure changed while canonicalizing typographic dashes".to_string(),
        );
    }

    let mut visible_mappings = HashMap::new();
    let mut anchor_mappings = HashMap::new();
    for (original, normalized) in original_headings.iter().zip(&canonical_headings) {
        match visible_mappings.get(&original.visible) {
            Some(existing) if existing != &normalized.visible => {
                return Err(format!(
                    "visible heading mapping is ambiguous for {:?}",
                    original.visible
                ));
            }
            Some(_) => {}
            None => {
                visible_mappings.insert(original.visible.clone(), normalized.visible.clone());
            }
        }
        match (&original.anchor, &normalized.anchor) {
            (Some(original), Some(normalized)) => {
                anchor_mappings.insert(original.clone(), normalized.clone());
            }
            (None, None) => {}
            _ => {
                return Err(
                    "native heading target changed kind while canonicalizing typographic dashes"
                        .to_string(),
                );
            }
        }
    }

    map_model_heading_fields(coverage, &visible_mappings, &anchor_mappings);
    for placement in placements {
        map_exact_value(&mut placement.section_anchor, &anchor_mappings);
    }
    let canonical = rewrite_local_heading_fragments(&canonical, &anchor_mappings)?;
    validate_nonempty_markdown_images(&canonical)?;
    Ok(canonical)
}

fn heading_targets(markdown: &str) -> Result<Vec<HeadingTarget>, String> {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};

    let mut current = None::<(String, usize)>;
    let mut headings = Vec::new();
    let mut duplicate_bases = HashMap::<String, usize>::new();
    let mut anchor_owners = HashMap::<String, String>::new();
    for (event, range) in Parser::new(markdown).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { .. }) => {
                current = Some((String::new(), range.start));
            }
            Event::Text(text) | Event::Code(text) if current.is_some() => {
                current.as_mut().unwrap().0.push_str(&text);
            }
            Event::End(TagEnd::Heading(_)) => {
                let (visible, source_offset) = current.take().ok_or_else(|| {
                    "Markdown parser ended a heading that was not open".to_string()
                })?;
                let anchor = native_heading_slug(markdown, source_offset).map(|base| {
                    let index = duplicate_bases.entry(base.clone()).or_default();
                    let anchor = if *index == 0 {
                        base
                    } else {
                        format!("{base}-{index}")
                    };
                    *index += 1;
                    anchor
                });
                if let Some(anchor) = &anchor {
                    if let Some(owner) = anchor_owners.insert(anchor.clone(), visible.clone()) {
                        return Err(format!(
                            "native Markdown heading anchor {anchor:?} is ambiguous between {owner:?} and {visible:?}"
                        ));
                    }
                }
                headings.push(HeadingTarget { visible, anchor });
            }
            _ => {}
        }
    }
    Ok(headings)
}

fn native_heading_slug(markdown: &str, offset: usize) -> Option<String> {
    let line_start = markdown[..offset]
        .rfind('\n')
        .map_or(0, |position| position + 1);
    let line_end = markdown[offset..]
        .find('\n')
        .map_or(markdown.len(), |position| offset + position);
    let line = markdown[line_start..line_end].trim_end_matches('\r');
    let marker_count = line.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&marker_count) {
        return None;
    }
    let after_markers = &line[marker_count..];
    if !after_markers
        .chars()
        .next()
        .is_some_and(char::is_whitespace)
    {
        return None;
    }
    let title = after_markers.trim_start_matches(char::is_whitespace);
    if title.is_empty() {
        return None;
    }
    let title = title
        .trim_end_matches(char::is_whitespace)
        .trim_end_matches('#')
        .trim_end_matches(char::is_whitespace);
    let title = strip_html_tags(title);
    let mut slug = String::new();
    let mut separator_pending = false;
    for character in title.trim().chars().flat_map(char::to_lowercase) {
        if character == ' ' || character == '-' {
            separator_pending = !slug.is_empty();
        } else if character == '_' || character.is_alphanumeric() {
            if separator_pending {
                slug.push('-');
                separator_pending = false;
            }
            slug.push(character);
        }
    }
    Some(slug)
}

fn strip_html_tags(value: &str) -> String {
    let mut stripped = String::with_capacity(value.len());
    let mut cursor = 0;
    while let Some(relative_start) = value[cursor..].find('<') {
        let start = cursor + relative_start;
        stripped.push_str(&value[cursor..start]);
        let Some(relative_end) = value[start + 1..].find('>') else {
            stripped.push_str(&value[start..]);
            return stripped;
        };
        let end = start + 1 + relative_end;
        if end == start + 1 {
            stripped.push_str("<>");
        }
        cursor = end + 1;
    }
    stripped.push_str(&value[cursor..]);
    stripped
}

fn map_model_heading_fields(
    value: &mut Value,
    visible_mappings: &HashMap<String, String>,
    anchor_mappings: &HashMap<String, String>,
) {
    match value {
        Value::Array(items) => {
            for item in items {
                map_model_heading_fields(item, visible_mappings, anchor_mappings);
            }
        }
        Value::Object(fields) => {
            for (name, field) in fields {
                if name == "guide_heading" {
                    if let Value::String(text) = field {
                        map_exact_value(text, visible_mappings);
                    }
                } else if name == "guide_anchor" {
                    if let Value::String(text) = field {
                        map_exact_value(text, anchor_mappings);
                    }
                } else {
                    map_model_heading_fields(field, visible_mappings, anchor_mappings);
                }
            }
        }
        _ => {}
    }
}

fn map_exact_value(value: &mut String, mappings: &HashMap<String, String>) {
    if let Some(mapped) = mappings.get(value) {
        value.clone_from(mapped);
    }
}

fn rewrite_local_heading_fragments(
    markdown: &str,
    mappings: &HashMap<String, String>,
) -> Result<String, String> {
    use pulldown_cmark::{Event, LinkType, Parser, Tag};

    let mut parser = Parser::new(markdown).into_offset_iter();
    let mut edits = Vec::new();
    for (event, range) in parser.by_ref() {
        let Event::Start(Tag::Link {
            link_type: LinkType::Inline,
            dest_url,
            ..
        }) = event
        else {
            continue;
        };
        let Some(fragment) = dest_url.strip_prefix('#') else {
            continue;
        };
        let Some(mapped) = mappings.get(fragment) else {
            continue;
        };
        if mapped == fragment {
            continue;
        }
        let expected = dest_url.as_ref();
        let replacement = format!("#{mapped}");
        let destination = inline_destination_range(&markdown[range.clone()], expected)?;
        edits.push((
            range.start + destination.start,
            range.start + destination.end,
            replacement,
        ));
    }
    for (_, definition) in parser.reference_definitions().iter() {
        let Some(fragment) = definition.dest.strip_prefix('#') else {
            continue;
        };
        let Some(mapped) = mappings.get(fragment) else {
            continue;
        };
        if mapped == fragment {
            continue;
        }
        let expected = definition.dest.as_ref();
        let destination =
            reference_destination_range(&markdown[definition.span.clone()], expected)?;
        edits.push((
            definition.span.start + destination.start,
            definition.span.start + destination.end,
            format!("#{mapped}"),
        ));
    }
    apply_markdown_edits(markdown, edits)
}

fn inline_destination_range(
    source: &str,
    expected: &str,
) -> Result<std::ops::Range<usize>, String> {
    destination_range_after_delimiters(source, "](", expected)
}

fn reference_destination_range(
    source: &str,
    expected: &str,
) -> Result<std::ops::Range<usize>, String> {
    destination_range_after_delimiters(source, "]:", expected)
}

fn destination_range_after_delimiters(
    source: &str,
    delimiter: &str,
    expected: &str,
) -> Result<std::ops::Range<usize>, String> {
    let mut matches = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find(delimiter) {
        let after_delimiter = cursor + relative + delimiter.len();
        let mut start = after_delimiter;
        while source[start..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
        {
            start += source[start..].chars().next().unwrap().len_utf8();
        }
        let angle_wrapped = source[start..].starts_with('<');
        if angle_wrapped {
            start += 1;
        }
        let mut end = start;
        for character in source[start..].chars() {
            if (angle_wrapped && character == '>')
                || (!angle_wrapped && (character.is_whitespace() || character == ')'))
            {
                break;
            }
            end += character.len_utf8();
        }
        if source.get(start..end) == Some(expected) {
            matches.push(start..end);
        }
        cursor = after_delimiter;
    }
    match matches.as_slice() {
        [range] => Ok(range.clone()),
        [] => Err(format!(
            "could not preserve local Markdown fragment {expected:?} because its source spelling is unsupported"
        )),
        _ => Err(format!(
            "could not preserve local Markdown fragment {expected:?} because its source destination is ambiguous"
        )),
    }
}

fn apply_markdown_edits(
    source: &str,
    mut edits: Vec<(usize, usize, String)>,
) -> Result<String, String> {
    if edits.is_empty() {
        return Ok(source.to_string());
    }
    edits.sort_unstable_by_key(|(start, _, _)| *start);
    let mut edited = String::with_capacity(source.len());
    let mut cursor = 0;
    for (start, end, replacement) in edits {
        if start < cursor || end < start || end > source.len() {
            return Err("Markdown canonicalization produced overlapping source edits".to_string());
        }
        edited.push_str(&source[cursor..start]);
        edited.push_str(&replacement);
        cursor = end;
    }
    edited.push_str(&source[cursor..]);
    Ok(edited)
}

fn validate_nonempty_markdown_images(markdown: &str) -> Result<(), String> {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};

    let mut image_alt_has_text = None;
    for event in Parser::new(markdown) {
        match event {
            Event::Start(Tag::Image { dest_url, .. }) => {
                if dest_url.trim().is_empty() {
                    return Err("Markdown image destination must be nonempty".to_string());
                }
                image_alt_has_text = Some(false);
            }
            Event::Text(text) | Event::Code(text) if image_alt_has_text.is_some() => {
                if !text.trim().is_empty() {
                    image_alt_has_text = Some(true);
                }
            }
            Event::End(TagEnd::Image) => {
                if image_alt_has_text.take() != Some(true) {
                    return Err("Markdown image alt text must be nonempty".to_string());
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelArtifacts {
    coverage_manifest: Value,
    visual_placements: Vec<VisualPlacement>,
    #[serde(default)]
    verification_harness: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VisualPlacement {
    asset_id: String,
    section_anchor: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct AssetCatalogEntry {
    pub id: String,
    pub public_path: String,
    pub readable_path: String,
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

#[derive(Clone, Debug)]
pub struct StagedLearnerAssets {
    pub working_dir: PathBuf,
    pub public_prefix: String,
    pub catalog: Vec<AssetCatalogEntry>,
    pub visual_packet_sha256: String,
    pub required_need_ids: Vec<String>,
    pub procedure_steps: Vec<ProcedureStepDefinition>,
}

#[derive(Debug, Serialize)]
struct RequiredNeedAssetMapping<'a> {
    required_need_id: &'a str,
    eligible_asset_ids: Vec<&'a str>,
}

fn required_need_asset_mapping(assets: &StagedLearnerAssets) -> Vec<RequiredNeedAssetMapping<'_>> {
    assets
        .required_need_ids
        .iter()
        .map(|need_id| RequiredNeedAssetMapping {
            required_need_id: need_id,
            eligible_asset_ids: assets
                .catalog
                .iter()
                .filter(|entry| entry.need_ids.iter().any(|id| id == need_id))
                .map(|entry| entry.id.as_str())
                .collect(),
        })
        .collect()
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};

    let mut source = std::fs::File::open(path)
        .map_err(|error| format!("could not open source for hashing: {error}"))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = source
            .read(&mut buffer)
            .map_err(|error| format!("could not hash source: {error}"))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub fn stage_rendered_assets(
    visual_material: &VisualMaterial,
    staging_root: &Path,
    output_path: &Path,
) -> Result<StagedLearnerAssets, String> {
    crate::visual_assets::recheck_visual_material(visual_material)?;
    let output_dir = output_path.parent().unwrap_or_else(|| Path::new("."));
    let guide_stem = output_path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "could not derive the learner-assets directory name".to_string())?;
    let public_prefix = format!("{guide_stem}_assets");
    let final_dir = output_dir.join(&public_prefix);
    if final_dir.exists() {
        return Err(format!(
            "refusing to overwrite existing learner-assets directory: {}",
            final_dir.display()
        ));
    }
    let catalog_dir = staging_root.join("visual-catalog");
    if catalog_dir.exists() {
        return Err(format!(
            "visual catalog staging directory already exists: {}",
            catalog_dir.display()
        ));
    }
    std::fs::create_dir(&catalog_dir)
        .map_err(|error| format!("could not create private visual catalog: {error}"))?;
    let working_dir = staging_root.join("learner-assets");
    std::fs::create_dir(&working_dir)
        .map_err(|error| format!("could not create learner-assets staging directory: {error}"))?;
    let mut catalog = Vec::with_capacity(visual_material.compiled_assets.len());
    for compiled in &visual_material.compiled_assets {
        let path = catalog_dir.join(&compiled.catalog.filename);
        atomic_write(&path, &compiled.bytes)?;
        let (width_px, height_px) = png_dimensions(&path)?;
        if sha256_file(&path)? != compiled.catalog.sha256
            || width_px != compiled.catalog.width_px
            || height_px != compiled.catalog.height_px
        {
            return Err(format!(
                "private visual catalog changed while materializing {}",
                compiled.catalog.id
            ));
        }
        catalog.push(catalog_entry(&compiled.catalog, &path, &public_prefix));
    }
    Ok(StagedLearnerAssets {
        working_dir,
        public_prefix,
        catalog,
        visual_packet_sha256: visual_material.packet_sha256.clone(),
        required_need_ids: visual_material
            .contract
            .needs
            .iter()
            .map(|need| need.id.clone())
            .collect(),
        procedure_steps: visual_material.contract.procedure_steps.clone(),
    })
}

fn catalog_entry(
    entry: &VisualCatalogEntry,
    readable_path: &Path,
    public_prefix: &str,
) -> AssetCatalogEntry {
    AssetCatalogEntry {
        id: entry.id.clone(),
        public_path: format!("{public_prefix}/{}", entry.filename),
        readable_path: readable_path.to_string_lossy().to_string(),
        sha256: entry.sha256.clone(),
        spec_sha256: entry.spec_sha256.clone(),
        width_px: entry.width_px,
        height_px: entry.height_px,
        kind: entry.kind,
        evidence_class: entry.evidence_class,
        need_ids: entry.need_ids.clone(),
        source_unit_ids: entry.source_unit_ids.clone(),
        procedure_step_ids: entry.procedure_step_ids.clone(),
        learning_purpose: entry.learning_purpose.clone(),
        alt: entry.alt.clone(),
        caption: entry.caption.clone(),
        explanation: entry.explanation.clone(),
        provenance: entry.provenance.clone(),
        rights: entry.rights.clone(),
    }
}

pub fn artifact_instructions(
    output_name: &str,
    expected_guide_kind: &str,
    assets: &StagedLearnerAssets,
    source_snapshot: &SourceSnapshot,
    predecessor_manifest: &PredecessorManifest,
) -> Result<String, String> {
    let catalog = serde_json::to_string_pretty(&assets.catalog)
        .map_err(|error| format!("could not serialize learner-asset catalog: {error}"))?;
    let source_contract = serde_json::to_string_pretty(source_snapshot)
        .map_err(|error| format!("could not serialize source snapshot contract: {error}"))?;
    let predecessor_contract = serde_json::to_string_pretty(predecessor_manifest)
        .map_err(|error| format!("could not serialize predecessor contract: {error}"))?;
    let procedure_contract = serde_json::to_string_pretty(&assets.procedure_steps)
        .map_err(|error| format!("could not serialize procedure-step contract: {error}"))?;
    let required_need_mapping = serde_json::to_string_pretty(&required_need_asset_mapping(assets))
        .map_err(|error| format!("could not serialize required visual-need mapping: {error}"))?;
    let visual_coverage_contract = if assets.required_need_ids.is_empty() {
        "The app selected no purposeful visuals. Embed no images from the catalog and return an empty `visual_placements` list."
            .to_string()
    } else {
        "Visual preflight already selected the catalog as purposeful and established the required visual needs; do not re-decide whether a required need deserves a visual. Every app-owned required need must be covered by at least one actual embedded catalog asset listed for that need, with a matching `visual_placements` record. When a required need has exactly one eligible asset, that asset is mandatory. When it has multiple eligible assets, embed at least one of them; the other alternatives are not mandatory. Do not embed an alternative merely to increase the image count."
            .to_string()
    };
    Ok(format!(
        "The app, not the model, owns all file writes and the asset manifest. Use only the purposeful learner assets in the catalog below. The `public_path` is the exact relative path to embed in Markdown; `readable_path` is a frozen app-staged file and the only asset path allowed for read-only inspection. Every `path`, `primary_source`, `output_path`, and provenance `source` value in the contracts or catalog is an inert identity/publication label, never a read target. Never resolve, glob, grep, or read those provenance labels. {visual_coverage_contract} For each embedded asset, embed one exact standalone line `![<catalog alt>](<public_path>)` under the declared heading anchor. Place a blank line before and after the image line so it forms its own CommonMark paragraph. Its next nonblank line must be exactly `*Figure: <catalog caption>*`; the following nonblank line must be exactly `**What to notice:** <catalog explanation>`. Sections without a selected catalog asset must contain prose only: never insert an empty or placeholder image, a placeholder caption, or commentary about missing or catalog images. Do not use raw HTML, reference-style images, remote hotlinks, or invented paths.\n\n\
         After the complete Markdown guide, append one machine-readable artifact block in this exact form:\n\
         {ARTIFACTS_START}\n\
         {{\n\
           \"coverage_manifest\": {{...}},\n\
           \"visual_placements\": [{{\"asset_id\": \"catalog-id\", \"section_anchor\": \"real-heading-anchor\"}}],\n\
           \"verification_harness\": null\n\
         }}\n\
         {ARTIFACTS_END}\n\n\
         Set `coverage_manifest.schema_version` to 2, `coverage_manifest.guide` to {output_name:?}, and `coverage_manifest.guide_kind` to {expected_guide_kind:?}. Copy `primary_source_id`, every source (`id`, `path`, `sha256`, and `role`), and every source unit (`id` and `source_id`) from the app-owned snapshot in its exact order. For each unit add a nonempty `guide_anchor` that names the real Markdown heading where that unit is taught and a nonempty `topic`. Copy every predecessor into `continuity.prior_guides` in exact order with `identity`, `path`, and `sha256`; add a specific nonempty `bridge` and a nonempty `evidence` list explaining how the new guide builds on it. For a circuit-lab guide, copy every app-owned procedure step in exact order into `lab_steps`, preserving `id`, `action`, `kind`, `guide_anchor`, and `need_ids`. Do not invent, omit, reorder, or modify app-owned values.\n\n\
         Include `research_sources` as a list of sources actually used, each with unique `id`, integer `tier` (1-4), and nonempty `kind`, `title`, `reference`, `locator`, `accessed` strings. IDs must differ from app-owned source IDs. Include explicit `source_conflicts` and `unresolved_gaps` lists, empty only when none exist. Each conflict needs nonempty `claim`, `resolution`, `status`, and `source_ids` naming at least two IDs from the app-owned sources or `research_sources`. Never put filenames or titles in `source_ids`. Set `continuity.prerequisites` to a list of prerequisite explanations and `continuity.next_bridge` to a concrete forward connection. Set `visual_plan.required` and its nonempty `rationale` to match the selected teaching assets. Set `coverage_manifest.verification_plan.required` to false, explain that app-owned deterministic checks validate supported arithmetic without executing model-authored code, and set `verification_harness` to null. Model-authored executable harnesses are always forbidden.\n\n\
         APP-OWNED SOURCE SNAPSHOT CONTRACT:\n{source_contract}\n\n\
         APP-OWNED PREDECESSOR CONTRACT:\n{predecessor_contract}\n\n\
         APP-OWNED PROCEDURE STEP CONTRACT:\n{procedure_contract}\n\n\
         APP-OWNED REQUIRED NEED TO ELIGIBLE ASSET MAPPING:\n{required_need_mapping}\n\n\
         LEARNER ASSET CATALOG:\n{catalog}"
    ))
}

pub fn materialize_model_artifacts(
    candidate_path: &Path,
    verify_dir: &Path,
    assets: &StagedLearnerAssets,
    expected_guide_name: &str,
    expected_guide_kind: &str,
    source_snapshot: &SourceSnapshot,
    predecessor_manifest: &PredecessorManifest,
) -> Result<(), String> {
    let response = std::fs::read_to_string(candidate_path)
        .map_err(|error| format!("model output was not readable: {error}"))?;
    let (guide, artifact_json) = split_artifact_response(&response)?;
    let mut artifacts: ModelArtifacts = serde_json::from_str(artifact_json)
        .map_err(|error| format!("model artifact block is not valid JSON: {error}"))?;
    let guide = canonicalize_heading_references(
        guide,
        &mut artifacts.coverage_manifest,
        &mut artifacts.visual_placements,
    )?;
    validate_artifacts(
        &artifacts,
        &guide,
        assets,
        expected_guide_name,
        expected_guide_kind,
        source_snapshot,
        predecessor_manifest,
    )?;

    let coverage = serde_json::to_string_pretty(&artifacts.coverage_manifest)
        .map_err(|error| format!("could not serialize coverage manifest: {error}"))?;
    atomic_write(
        &verify_dir.join("coverage_manifest.json"),
        coverage.as_bytes(),
    )?;
    ensure_empty_asset_directory(&assets.working_dir)?;
    let mut records = Vec::with_capacity(artifacts.visual_placements.len());
    for placement in &artifacts.visual_placements {
        let entry = assets
            .catalog
            .iter()
            .find(|entry| entry.id == placement.asset_id)
            .ok_or_else(|| format!("unknown visual placement: {}", placement.asset_id))?;
        let source = Path::new(&entry.readable_path);
        ensure_plain_file(source)?;
        let bytes = std::fs::read(source)
            .map_err(|error| format!("could not read compiled visual {}: {error}", entry.id))?;
        if format!("{:x}", sha2::Sha256::digest(&bytes)) != entry.sha256 {
            return Err(format!(
                "compiled visual changed before publication: {}",
                entry.id
            ));
        }
        let filename = Path::new(&entry.public_path)
            .file_name()
            .ok_or_else(|| format!("visual has no safe filename: {}", entry.id))?;
        atomic_write(&assets.working_dir.join(filename), &bytes)?;
        records.push(serde_json::json!({
            "id": entry.id,
            "path": entry.public_path,
            "sha256": entry.sha256,
            "spec_sha256": entry.spec_sha256,
            "width_px": entry.width_px,
            "height_px": entry.height_px,
            "kind": entry.kind,
            "evidence_class": entry.evidence_class,
            "need_ids": entry.need_ids,
            "source_unit_ids": entry.source_unit_ids,
            "procedure_step_ids": entry.procedure_step_ids,
            "learning_purpose": entry.learning_purpose,
            "section_anchor": placement.section_anchor,
            "alt": entry.alt,
            "caption": entry.caption,
            "explanation": entry.explanation,
            "provenance": entry.provenance,
            "rights": entry.rights,
        }));
    }
    let asset_manifest = serde_json::to_vec_pretty(&serde_json::json!({
        "schema_version": 2,
        "visual_packet_sha256": assets.visual_packet_sha256,
        "assets": records,
    }))
    .map_err(|error| format!("could not serialize app-owned asset manifest: {error}"))?;
    atomic_write(
        &assets.working_dir.join("asset_manifest.json"),
        &asset_manifest,
    )?;

    let harness_path = verify_dir.join("verify.py");
    if harness_path.exists() {
        return Err(
            "verification directory contains forbidden model-authored executable code".to_string(),
        );
    }
    atomic_write(candidate_path, guide.trim_end().as_bytes())?;
    Ok(())
}

fn ensure_empty_asset_directory(path: &Path) -> Result<(), String> {
    ensure_plain_directory(path)?;
    if std::fs::read_dir(path)
        .map_err(|error| format!("could not inspect learner-assets directory: {error}"))?
        .next()
        .transpose()
        .map_err(|error| format!("could not inspect learner-assets entry: {error}"))?
        .is_some()
    {
        return Err(format!(
            "learner-assets attempt directory must be empty before app materialization: {}",
            path.display()
        ));
    }
    Ok(())
}

/// Split a complete model response into the guide body and the whole artifact block
/// including its markers, so a later pass can rewrite the body and reattach the block intact.
pub fn detach_artifact_block(response: &str) -> Result<(&str, &str), String> {
    let start = response
        .rfind(ARTIFACTS_START)
        .ok_or_else(|| "model response is missing the artifact block".to_string())?;
    if response[..start].contains(ARTIFACTS_START) {
        return Err("model response contains more than one artifact block".to_string());
    }
    let relative_end = response[start..]
        .find(ARTIFACTS_END)
        .ok_or_else(|| "model artifact block is missing its end marker".to_string())?;
    let end = start + relative_end + ARTIFACTS_END.len();
    if !response[end..].trim().is_empty() {
        return Err("model artifact block must be the final response content".to_string());
    }
    if response[..start].trim().is_empty() {
        return Err(
            "model response contains no Markdown guide before the artifact block".to_string(),
        );
    }
    Ok((&response[..start], &response[start..end]))
}

fn split_artifact_response(response: &str) -> Result<(&str, &str), String> {
    let start = response
        .rfind(ARTIFACTS_START)
        .ok_or_else(|| "model response is missing the artifact block".to_string())?;
    if response[..start].contains(ARTIFACTS_START) {
        return Err("model response contains more than one artifact block".to_string());
    }
    let json_start = start + ARTIFACTS_START.len();
    let tail = &response[json_start..];
    let relative_end = tail
        .find(ARTIFACTS_END)
        .ok_or_else(|| "model artifact block is missing its end marker".to_string())?;
    let json_end = json_start + relative_end;
    let after = &response[json_end + ARTIFACTS_END.len()..];
    if !after.trim().is_empty() {
        return Err("model artifact block must be the final response content".to_string());
    }
    let guide = &response[..start];
    if guide.trim().is_empty() {
        return Err(
            "model response contains no Markdown guide before the artifact block".to_string(),
        );
    }
    Ok((guide, response[json_start..json_end].trim()))
}

fn validate_artifacts(
    artifacts: &ModelArtifacts,
    _guide: &str,
    assets: &StagedLearnerAssets,
    expected_guide_name: &str,
    expected_guide_kind: &str,
    source_snapshot: &SourceSnapshot,
    predecessor_manifest: &PredecessorManifest,
) -> Result<(), String> {
    let coverage = artifacts
        .coverage_manifest
        .as_object()
        .ok_or_else(|| "coverage_manifest must be a JSON object".to_string())?;
    if coverage.get("guide").and_then(Value::as_str) != Some(expected_guide_name) {
        return Err(format!(
            "coverage_manifest.guide must be {expected_guide_name:?}"
        ));
    }
    if coverage.get("guide_kind").and_then(Value::as_str) != Some(expected_guide_kind) {
        return Err(format!(
            "coverage_manifest.guide_kind must be {expected_guide_kind:?}"
        ));
    }
    validate_source_coverage(coverage, source_snapshot)?;
    validate_continuity(coverage, predecessor_manifest)?;
    validate_procedure_coverage(coverage, expected_guide_kind, &assets.procedure_steps)?;
    validate_verification_contract(coverage, artifacts.verification_harness.as_deref())?;
    let mut placements = HashSet::new();
    let unit_anchors = coverage
        .get("units")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|unit| {
            let unit = unit.as_object()?;
            Some((
                unit.get("id")?.as_str()?.to_string(),
                unit.get("guide_anchor")?.as_str()?.to_string(),
            ))
        })
        .collect::<HashMap<_, _>>();
    let mut covered_needs = HashSet::new();
    for placement in &artifacts.visual_placements {
        if placement.section_anchor.trim().is_empty() {
            return Err("visual placement section_anchor must be nonempty".to_string());
        }
        if !placements.insert(placement.asset_id.as_str()) {
            return Err(format!(
                "visual placement is duplicated: {}",
                placement.asset_id
            ));
        }
        let entry = assets
            .catalog
            .iter()
            .find(|entry| entry.id == placement.asset_id)
            .ok_or_else(|| {
                format!(
                    "visual placement references unknown asset: {}",
                    placement.asset_id
                )
            })?;
        for unit_id in &entry.source_unit_ids {
            if unit_anchors.get(unit_id).map(String::as_str)
                != Some(placement.section_anchor.as_str())
            {
                return Err(format!(
                    "visual {} must be placed in the same guide section as linked source unit {}",
                    entry.id, unit_id
                ));
            }
        }
        covered_needs.extend(entry.need_ids.iter().map(String::as_str));
    }
    let missing_needs = required_need_asset_mapping(assets)
        .into_iter()
        .filter(|mapping| !covered_needs.contains(mapping.required_need_id))
        .map(|mapping| {
            format!(
                "{} (eligible asset IDs: [{}])",
                mapping.required_need_id,
                mapping.eligible_asset_ids.join(", ")
            )
        })
        .collect::<Vec<_>>();
    if !missing_needs.is_empty() {
        return Err(format!(
            "visual placements are missing required app-owned visual needs: {}",
            missing_needs.join("; ")
        ));
    }
    if assets.required_need_ids.is_empty() && !artifacts.visual_placements.is_empty() {
        return Err("no-purposeful-visual contract requires empty visual_placements".to_string());
    }
    Ok(())
}

fn validate_verification_contract(
    coverage: &serde_json::Map<String, Value>,
    harness: Option<&str>,
) -> Result<(), String> {
    let plan = coverage
        .get("verification_plan")
        .and_then(Value::as_object)
        .ok_or_else(|| "coverage_manifest.verification_plan must be a JSON object".to_string())?;
    let required = plan
        .get("required")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            "coverage_manifest.verification_plan.required must be boolean".to_string()
        })?;
    if plan
        .get("rationale")
        .and_then(Value::as_str)
        .is_none_or(|value| value.trim().is_empty())
    {
        return Err("coverage_manifest.verification_plan.rationale must be nonempty".to_string());
    }
    if required {
        return Err(
            "coverage_manifest.verification_plan.required must be false; executable model-authored verification is forbidden"
                .to_string(),
        );
    }
    let has_harness = harness.is_some_and(|value| !value.trim().is_empty());
    match has_harness {
        true => Err(
            "verification_harness is forbidden when verification_plan.required is false"
                .to_string(),
        ),
        false => Ok(()),
    }
}

fn validate_procedure_coverage(
    coverage: &serde_json::Map<String, Value>,
    expected_guide_kind: &str,
    expected: &[ProcedureStepDefinition],
) -> Result<(), String> {
    if expected_guide_kind != "circuit-lab" {
        if !expected.is_empty() {
            return Err("non-Circuit guide has an app-owned procedure-step contract".to_string());
        }
        return Ok(());
    }
    let actual = coverage
        .get("lab_steps")
        .and_then(Value::as_array)
        .ok_or_else(|| "coverage_manifest.lab_steps must copy every app-owned step".to_string())?;
    if actual.len() != expected.len() {
        return Err(
            "coverage_manifest.lab_steps does not match the app-owned step count".to_string(),
        );
    }
    for (record, step) in actual.iter().zip(expected) {
        let record = record
            .as_object()
            .ok_or_else(|| "each coverage lab step must be a JSON object".to_string())?;
        let expected_kind = serde_json::to_value(step.kind)
            .expect("procedure kind serializes")
            .as_str()
            .expect("procedure kind is a string")
            .to_string();
        let actual_need_ids = record
            .get("need_ids")
            .and_then(Value::as_array)
            .and_then(|items| {
                items
                    .iter()
                    .map(|item| item.as_str().map(str::to_string))
                    .collect::<Option<Vec<_>>>()
            });
        if record.get("id").and_then(Value::as_str) != Some(step.id.as_str())
            || record.get("action").and_then(Value::as_str) != Some(step.action.as_str())
            || record.get("kind").and_then(Value::as_str) != Some(expected_kind.as_str())
            || record.get("guide_anchor").and_then(Value::as_str)
                != Some(step.guide_anchor.as_str())
            || actual_need_ids.as_ref() != Some(&step.need_ids)
        {
            return Err(format!(
                "coverage lab step must copy app-owned id, action, kind, guide_anchor, and need_ids: {}",
                step.id
            ));
        }
    }
    Ok(())
}

fn validate_source_coverage(
    coverage: &serde_json::Map<String, Value>,
    snapshot: &SourceSnapshot,
) -> Result<(), String> {
    if coverage.get("schema_version").and_then(Value::as_u64) != Some(2) {
        return Err("coverage_manifest.schema_version must be 2".to_string());
    }
    if coverage.get("primary_source_id").and_then(Value::as_str)
        != Some(snapshot.primary_source_id.as_str())
    {
        return Err(
            "coverage_manifest.primary_source_id does not match the app-owned snapshot".to_string(),
        );
    }
    let sources = coverage
        .get("sources")
        .and_then(Value::as_array)
        .ok_or_else(|| "coverage_manifest.sources must be a JSON list".to_string())?;
    if sources.len() != snapshot.sources.len() {
        return Err(
            "coverage_manifest.sources must contain every selected source exactly once".to_string(),
        );
    }
    for (record, expected) in sources.iter().zip(&snapshot.sources) {
        let record = record
            .as_object()
            .ok_or_else(|| "each coverage source must be a JSON object".to_string())?;
        for (field, expected_value) in [
            ("id", expected.id.as_str()),
            ("path", expected.path.as_str()),
            ("sha256", expected.sha256.as_str()),
            ("role", expected.role.as_str()),
        ] {
            if record.get(field).and_then(Value::as_str) != Some(expected_value) {
                return Err(format!(
                    "coverage source {field} does not match the app-owned snapshot for {}",
                    expected.id
                ));
            }
        }
    }

    let units = coverage
        .get("units")
        .and_then(Value::as_array)
        .ok_or_else(|| "coverage_manifest.units must be a JSON list".to_string())?;
    if units.len() != snapshot.unit_ids.len() {
        return Err(
            "coverage_manifest.units must contain every selected source unit exactly once"
                .to_string(),
        );
    }
    let source_for_unit = snapshot
        .sources
        .iter()
        .flat_map(|source| {
            source
                .unit_ids
                .iter()
                .map(move |unit| (unit.as_str(), source.id.as_str()))
        })
        .collect::<std::collections::HashMap<_, _>>();
    let mut seen = HashSet::new();
    for record in units {
        let record = record
            .as_object()
            .ok_or_else(|| "each coverage unit must be a JSON object".to_string())?;
        let id = record
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "each coverage unit must have a string id".to_string())?;
        if !seen.insert(id) {
            return Err(format!("coverage unit is duplicated: {id}"));
        }
        let expected_source = source_for_unit
            .get(id)
            .ok_or_else(|| format!("coverage unit is not in the app-owned snapshot: {id}"))?;
        if record.get("source_id").and_then(Value::as_str) != Some(*expected_source) {
            return Err(format!("coverage unit has the wrong source_id: {id}"));
        }
        for field in ["guide_anchor", "topic"] {
            if record
                .get(field)
                .and_then(Value::as_str)
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err(format!("coverage unit {id} must have a nonempty {field}"));
            }
        }
    }
    if seen.len() != source_for_unit.len() {
        return Err("coverage units do not exactly match the app-owned source units".to_string());
    }
    Ok(())
}

fn validate_continuity(
    coverage: &serde_json::Map<String, Value>,
    manifest: &PredecessorManifest,
) -> Result<(), String> {
    let continuity = coverage
        .get("continuity")
        .and_then(Value::as_object)
        .ok_or_else(|| "coverage_manifest.continuity must be a JSON object".to_string())?;
    let prior = continuity
        .get("prior_guides")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "coverage_manifest.continuity.prior_guides must be a JSON list".to_string()
        })?;
    if prior.len() != manifest.prior_guides.len() {
        return Err(
            "continuity.prior_guides must contain every bound predecessor exactly once".to_string(),
        );
    }
    for (record, expected) in prior.iter().zip(&manifest.prior_guides) {
        let record = record
            .as_object()
            .ok_or_else(|| "each continuity prior-guide record must be an object".to_string())?;
        for (field, expected_value) in [
            ("identity", expected.generation_identity.as_str()),
            ("path", expected.path.as_str()),
            ("sha256", expected.sha256.as_str()),
        ] {
            if record.get(field).and_then(Value::as_str) != Some(expected_value) {
                return Err(format!(
                    "continuity prior-guide {field} does not match {}",
                    expected.generation_identity
                ));
            }
        }
        if record
            .get("bridge")
            .and_then(Value::as_str)
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err(format!(
                "continuity prior guide {} requires a nonempty bridge",
                expected.generation_identity
            ));
        }
        let evidence = record
            .get("evidence")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                format!(
                    "continuity prior guide {} requires an evidence list",
                    expected.generation_identity
                )
            })?;
        if evidence.is_empty()
            || evidence
                .iter()
                .any(|item| item.as_str().is_none_or(|value| value.trim().is_empty()))
        {
            return Err(format!(
                "continuity prior guide {} requires nonempty evidence",
                expected.generation_identity
            ));
        }
    }
    Ok(())
}

fn png_dimensions(path: &Path) -> Result<(u32, u32), String> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("could not inspect staged PNG: {error}"))?;
    decode_png_dimensions(&bytes).map_err(|error| {
        format!(
            "staged learner asset is not a fully decodable PNG ({}): {error}",
            path.display()
        )
    })
}

fn ensure_plain_directory(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        format!(
            "frozen render directory is missing or unreadable ({}): {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() || is_reparse(&metadata)
    {
        return Err(format!(
            "frozen render directory is a symlink, reparse point, or non-directory: {}",
            path.display()
        ));
    }
    Ok(())
}

fn ensure_plain_file(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        format!(
            "frozen render is missing or unreadable ({}): {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() || is_reparse(&metadata)
    {
        return Err(format!(
            "frozen render is a symlink, reparse point, or non-file: {}",
            path.display()
        ));
    }
    Ok(())
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

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "artifact".to_string());
    let temp = parent.join(format!(".{name}.{}.tmp", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|error| format!("could not stage {}: {error}", path.display()))?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("could not write {}: {error}", path.display()));
    }
    drop(file);
    if path.exists() {
        std::fs::remove_file(path)
            .map_err(|error| format!("could not replace {}: {error}", path.display()))?;
    }
    std::fs::rename(&temp, path)
        .map_err(|error| format!("could not publish {}: {error}", path.display()))?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn assert_file_hash_on_small_stack(
    test_name: &str,
    hash: fn(&Path) -> Result<String, String>,
) {
    const CHILD_ENV: &str = "GUIDE_WATCHER_HASH_STACK_TEST_CHILD";
    if std::env::var(CHILD_ENV).ok().as_deref() != Some(test_name) {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--nocapture"])
            .env(CHILD_ENV, test_name)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "small-stack hash subprocess failed: {}\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    std::thread::Builder::new()
        .name("bounded-file-hash".to_string())
        .stack_size(256 * 1024)
        .spawn(move || {
            let root =
                std::env::temp_dir().join(format!("guide-watcher-hash-test-{}", Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            let path = root.join("source.bin");
            assert!(hash(&path).is_err());
            assert!(hash(&root).is_err());
            for size in [
                0,
                1,
                1024 * 1024 - 1,
                1024 * 1024,
                1024 * 1024 + 1,
                2 * 1024 * 1024 + 19,
            ] {
                let bytes: Vec<u8> = (0..size)
                    .map(|index| ((index / 4096 + index) % 251) as u8)
                    .collect();
                std::fs::write(&path, &bytes).unwrap();
                let expected = format!("{:x}", sha2::Sha256::digest(&bytes));
                assert_eq!(hash(&path).unwrap(), expected);
            }
            std::fs::remove_dir_all(root).unwrap();
        })
        .unwrap()
        .join()
        .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;

    fn resource_assessment() -> Value {
        serde_json::json!({"sha256": "a".repeat(64), "status": "used", "pages": [1, 3],
            "locator": "Chapter 1, process abstraction", "explanation": "The process example explains CPU sharing.",
            "guide_heading": "CPU sharing", "evidence": "Each process receives a virtual CPU."})
    }

    fn resource(sha: char, page_count: usize, primary: bool) -> SupplementaryResource {
        SupplementaryResource {
            sha256: sha.to_string().repeat(64),
            page_count,
            title: if primary {
                "Operating Systems: Three Easy Pieces".to_string()
            } else {
                "Supporting systems book".to_string()
            },
            primary,
        }
    }

    fn primary_assessment() -> Value {
        serde_json::json!({
            "sha256": "a".repeat(64),
            "status": "used",
            "pages": [1, 3],
            "locator": "Chapter 2, introduction",
            "explanation": "The chapter establishes the abstractions used across the guide.",
            "guide_heading": "CPU sharing",
            "evidence": "Each process receives a virtual CPU.",
            "integrations": [{
                "pages": [1],
                "locator": "Chapter 2, CPU virtualization",
                "guide_heading": "CPU sharing",
                "evidence": "Each process receives a virtual CPU."
            }, {
                "pages": [3],
                "locator": "Chapter 2, memory virtualization",
                "guide_heading": "Private address spaces",
                "evidence": "The same virtual address can name different physical memory."
            }]
        })
    }

    fn primary_guide() -> &'static str {
        "# CPU sharing\n\nEach process receives a virtual CPU. Read Chapter 2, CPU virtualization (PDF pages 1-2).\n\n# Private address spaces\n\nThe same virtual address can name different physical memory.\n"
    }

    #[test]
    fn supplementary_assessments_accept_real_page_and_guide_evidence() {
        let expected = vec![resource('a', 3, false)];
        let coverage = serde_json::json!({"research_notes": [resource_assessment()]});
        let guide = "# CPU sharing\n\nEach process receives a virtual CPU.\n";
        assert!(validate_supplementary_research(&coverage, guide, &expected).is_ok());
        let mut not_relevant = resource_assessment();
        not_relevant["status"] = "not_relevant".into();
        not_relevant
            .as_object_mut()
            .unwrap()
            .remove("guide_heading");
        not_relevant.as_object_mut().unwrap().remove("evidence");
        assert!(validate_supplementary_research(
            &serde_json::json!({"research_notes": [not_relevant]}),
            "",
            &expected
        )
        .is_ok());
    }

    #[test]
    fn supplementary_assessments_reject_missing_duplicate_and_foreign_resources() {
        let expected = vec![resource('a', 3, false), resource('b', 5, false)];
        for notes in [
            serde_json::json!([]),
            serde_json::json!([resource_assessment()]),
            serde_json::json!([resource_assessment(), resource_assessment()]),
        ] {
            assert!(validate_supplementary_research(
                &serde_json::json!({"research_notes": notes}),
                "# CPU sharing\nEach process receives a virtual CPU.",
                &expected
            )
            .is_err());
        }
        assert!(validate_supplementary_research(&serde_json::json!({}), "", &expected).is_err());
        let mut foreign = resource_assessment();
        foreign["sha256"] = "c".repeat(64).into();
        assert!(validate_supplementary_research(
            &serde_json::json!({"research_notes": [foreign]}),
            "",
            &expected[..1]
        )
        .is_err());
    }

    #[test]
    fn supplementary_assessments_reject_bad_pages_status_and_empty_fields() {
        let expected = vec![resource('a', 3, false)];
        let guide = "# CPU sharing\nEach process receives a virtual CPU.";
        for pages in [
            serde_json::json!([]),
            serde_json::json!([0]),
            serde_json::json!([4]),
            serde_json::json!([-1]),
            serde_json::json!([1.5]),
            serde_json::json!(["1"]),
            serde_json::json!([1, 1]),
        ] {
            let mut note = resource_assessment();
            note["pages"] = pages;
            assert!(validate_supplementary_research(
                &serde_json::json!({"research_notes": [note]}),
                guide,
                &expected
            )
            .is_err());
        }
        for field in [
            "status",
            "locator",
            "explanation",
            "guide_heading",
            "evidence",
        ] {
            let mut note = resource_assessment();
            note[field] = " ".into();
            assert!(
                validate_supplementary_research(
                    &serde_json::json!({"research_notes": [note]}),
                    guide,
                    &expected
                )
                .is_err(),
                "{field}"
            );
        }
    }

    #[test]
    fn supplementary_assessments_require_evidence_under_actual_heading() {
        let expected = vec![resource('a', 3, false)];
        let coverage = serde_json::json!({"research_notes": [resource_assessment()]});
        for guide in [
            "# Other\nEach process receives a virtual CPU.",
            "# CPU sharing\nNo supporting passage.\n# Other\nEach process receives a virtual CPU.",
            "# CPU sharing\n\n```\nEach process receives a virtual CPU.\n```",
            "# CPU sharing\n\n<!-- Each process receives a virtual CPU. -->",
            "# CPU sharing\n\nComment <!-- Each process receives a virtual CPU. -->",
            "# CPU sharing\n\n![Each process receives a virtual CPU.](image.png)",
            "```\n# CPU sharing\nEach process receives a virtual CPU.\n```",
        ] {
            assert!(validate_supplementary_research(&coverage, guide, &expected).is_err());
        }
        assert!(validate_supplementary_research(
            &coverage,
            "CPU sharing\n===========\nEach process receives a virtual CPU.",
            &expected
        )
        .is_ok());
    }

    #[test]
    fn supplementary_evidence_follows_markdown_section_ancestry() {
        let expected = vec![resource('a', 3, false)];
        let coverage = serde_json::json!({"research_notes": [resource_assessment()]});
        for guide in [
            "# CPU sharing\n\nEach process receives a virtual CPU.",
            "# CPU sharing\n\n### Analogy\n\nEach process receives a virtual CPU.",
            "# CPU sharing\n\n## Child\n\n#### Deep child\n\nEach process receives a virtual CPU.",
            "# CPU sharing\n\n```text\n# Other\n```\n\n### Child\n\nEach process receives a virtual CPU.",
        ] {
            validate_supplementary_research(&coverage, guide, &expected)
                .expect("a declared section owns paragraphs in its descendant subsections");
        }

        for guide in [
            "Each process receives a virtual CPU.\n\n# CPU sharing\n\nNo supporting passage.",
            "# CPU sharing\n\nEach process receives something like a virtual CPU.",
            "# CPU sharing\n\nNo supporting passage.\n\n# Sibling\n\nEach process receives a virtual CPU.",
            "## CPU sharing\n\n### Child\n\nNo supporting passage.\n\n# Higher-level exit\n\nEach process receives a virtual CPU.",
        ] {
            assert!(validate_supplementary_research(&coverage, guide, &expected).is_err());
        }

        let mut leaf = resource_assessment();
        leaf["guide_heading"] = "Analogy".into();
        assert!(validate_supplementary_research(
            &serde_json::json!({"research_notes": [leaf]}),
            "# CPU sharing\n\n### Analogy\n\nNo supporting passage.\n\n### Cycle\n\nEach process receives a virtual CPU.",
            &expected,
        )
        .is_err());
    }

    #[test]
    fn supplementary_evidence_preserves_exact_unicode_titles_and_rejects_ambiguity() {
        let expected = vec![resource('a', 3, false)];
        let mut unicode = resource_assessment();
        unicode["guide_heading"] = "메모리 映射".into();
        validate_supplementary_research(
            &serde_json::json!({"research_notes": [unicode]}),
            "## 메모리 `映射`\n\n### 깊은 절\n\nEach process receives a virtual CPU.\n\n## 다음 절\n",
            &expected,
        )
        .expect("visible Unicode and inline-code heading text should remain exact");

        let coverage = serde_json::json!({"research_notes": [resource_assessment()]});
        let error = validate_supplementary_research(
            &coverage,
            "# CPU sharing\n\nEach process receives a virtual CPU.\n\n# CPU sharing\n\nNo supporting passage.",
            &expected,
        )
        .unwrap_err();
        assert!(
            error.contains("guide_heading \"CPU sharing\" is ambiguous"),
            "{error}"
        );
    }

    #[test]
    fn primary_supplementary_resource_requires_distinct_integrations_but_no_reading_path() {
        let expected = vec![resource('a', 3, true)];
        let valid = serde_json::json!({"research_notes": [primary_assessment()]});
        assert!(validate_supplementary_research(&valid, primary_guide(), &expected).is_ok());

        let mut not_relevant = primary_assessment();
        not_relevant["status"] = "not_relevant".into();
        let error = validate_supplementary_research(
            &serde_json::json!({"research_notes": [not_relevant]}),
            primary_guide(),
            &expected,
        )
        .unwrap_err();
        assert!(
            error.contains("primary textbook") && error.contains("status used"),
            "{error}"
        );

        // A reading_path is neither required nor validated any more: the guide teaches, it
        // does not assign reading.
        let mut with_stale_reading_path = primary_assessment();
        with_stale_reading_path["reading_path"] = serde_json::json!([{"pages": [1], "locator": "x", "guide_heading": "nowhere", "evidence": "Read everything."}]);
        assert!(validate_supplementary_research(
            &serde_json::json!({"research_notes": [with_stale_reading_path]}),
            primary_guide(),
            &expected,
        )
        .is_ok());

        {
            let mut assessment = primary_assessment();
            assessment.as_object_mut().unwrap().remove("integrations");
            let error = validate_supplementary_research(
                &serde_json::json!({"research_notes": [assessment]}),
                primary_guide(),
                &expected,
            )
            .unwrap_err();
            assert!(error.contains("integrations"), "{error}");
        }

        let mut one_integration = primary_assessment();
        let first_integration = one_integration["integrations"][0].clone();
        one_integration["integrations"] = serde_json::json!([first_integration]);
        let error = validate_supplementary_research(
            &serde_json::json!({"research_notes": [one_integration]}),
            primary_guide(),
            &expected,
        )
        .unwrap_err();
        assert!(error.contains("at least two integrations"), "{error}");

        let mut repeated_heading = primary_assessment();
        repeated_heading["integrations"][1]["guide_heading"] = "CPU sharing".into();
        repeated_heading["integrations"][1]["evidence"] =
            "A second exact explanatory sentence.".into();
        let guide = "# CPU sharing\n\nEach process receives a virtual CPU. A second exact explanatory sentence. Read Chapter 2, CPU virtualization (PDF pages 1-2).";
        let error = validate_supplementary_research(
            &serde_json::json!({"research_notes": [repeated_heading]}),
            guide,
            &expected,
        )
        .unwrap_err();
        assert!(error.contains("distinct real guide headings"), "{error}");
    }

    #[test]
    fn primary_paths_reuse_real_paragraph_evidence_and_bounded_unique_pages() {
        let expected = vec![resource('a', 3, true)];
        let mut assessment = primary_assessment();
        assessment["integrations"][0]["pages"] = serde_json::json!([2, 2]);
        let error = validate_supplementary_research(
            &serde_json::json!({"research_notes": [assessment]}),
            primary_guide(),
            &expected,
        )
        .unwrap_err();
        assert!(
            error.contains("integrations[0]") && error.contains("repeats PDF page"),
            "{error}"
        );

        let mut assessment = primary_assessment();
        assessment["integrations"][1]["pages"] = serde_json::json!([4]);
        let error = validate_supplementary_research(
            &serde_json::json!({"research_notes": [assessment]}),
            primary_guide(),
            &expected,
        )
        .unwrap_err();
        assert!(
            error.contains("integrations[1]") && error.contains("3-page frozen PDF"),
            "{error}"
        );

        for evidence in [
            "<!-- The same virtual address can name different physical memory. -->",
            "![The same virtual address can name different physical memory.](memory.png)",
            "```\nThe same virtual address can name different physical memory.\n```",
        ] {
            let mut assessment = primary_assessment();
            assessment["integrations"][1]["evidence"] =
                "The same virtual address can name different physical memory.".into();
            let guide = format!("# CPU sharing\n\nEach process receives a virtual CPU. Read Chapter 2, CPU virtualization (PDF pages 1-2).\n# Private address spaces\n\n{evidence}");
            assert!(validate_supplementary_research(
                &serde_json::json!({"research_notes": [assessment]}),
                &guide,
                &expected,
            )
            .is_err());
        }
    }

    #[test]
    fn heading_dash_canonicalization_is_syntax_scoped() {
        let markdown = "# ASCII stability 한글 U+2212 −\n\n# All‐four‑supported–dashes—here\n\n# [Virtual‑CPU](https://example.test/range–value) and `Non‑Atomic`\n\nProse Virtual‑CPU and range 2.1–2.5 stay unchanged.\n\n```md\n# Fenced‑heading\n```\n";
        let canonical = canonicalize_markdown_heading_dashes(markdown)
            .expect("literal heading dashes should be supported");
        assert!(canonical.contains("# ASCII stability 한글 U+2212 −"));
        assert!(canonical.contains("# All-four-supported-dashes-here"));
        assert!(canonical
            .contains("# [Virtual-CPU](https://example.test/range–value) and `Non‑Atomic`"));
        assert!(canonical.contains("Prose Virtual‑CPU and range 2.1–2.5 stay unchanged."));
        assert!(canonical.contains("# Fenced‑heading"));
        assert!(!canonical.contains("# [Virtual‑CPU]"));
    }

    #[test]
    fn heading_mappings_do_not_touch_provenance_or_evidence() {
        let markdown = "# 9. The Virtual‑CPU Illusion\n";
        let mut value = serde_json::json!({
            "sources": [{
                "id": "source‑identity",
                "path": "https://example.test/source–path",
                "sha256": "hash‑value"
            }],
            "units": [{"guide_anchor": "9-the-virtualcpu-illusion", "topic": "CPU‑topic"}],
            "research_notes": [{
                "guide_heading": "9. The Virtual‑CPU Illusion",
                "locator": "Sections 2.1–2.5",
                "evidence": "The Virtual‑CPU quote stays exact."
            }]
        });
        let canonical = canonicalize_heading_references(markdown, &mut value, &mut [])
            .expect("real heading mappings should canonicalize metadata");
        assert_eq!(canonical, "# 9. The Virtual-CPU Illusion\n");
        assert_eq!(
            value["units"][0]["guide_anchor"],
            "9-the-virtual-cpu-illusion"
        );
        assert_eq!(
            value["research_notes"][0]["guide_heading"],
            "9. The Virtual-CPU Illusion"
        );
        assert_eq!(value["sources"][0]["id"], "source‑identity");
        assert_eq!(
            value["sources"][0]["path"],
            "https://example.test/source–path"
        );
        assert_eq!(value["sources"][0]["sha256"], "hash‑value");
        assert_eq!(value["units"][0]["topic"], "CPU‑topic");
        assert_eq!(
            value["research_notes"][0]["evidence"],
            "The Virtual‑CPU quote stays exact."
        );
        assert_eq!(value["research_notes"][0]["locator"], "Sections 2.1–2.5");
    }

    #[test]
    fn inline_code_dash_in_a_heading_is_not_fuzzily_matched() {
        let coverage = serde_json::json!({"research_notes": [{
            "sha256": "a".repeat(64),
            "status": "used",
            "pages": [1],
            "locator": "Chapter 1",
            "explanation": "Code spelling matters.",
            "guide_heading": "Virtual-CPU",
            "evidence": "Exact explanatory evidence."
        }]});
        let error = validate_supplementary_research(
            &coverage,
            "# `Virtual‑CPU`\n\nExact explanatory evidence.\n",
            &[resource('a', 1, false)],
        )
        .unwrap_err();
        assert!(error.contains("real guide_heading"), "{error}");
    }

    #[test]
    fn canonicalized_duplicate_integration_headings_are_still_rejected() {
        let mut assessment = primary_assessment();
        assessment["integrations"][0]["guide_heading"] = "Virtual‑CPU".into();
        assessment["integrations"][0]["evidence"] = "First evidence.".into();
        assessment["integrations"][1]["guide_heading"] = "Virtual-CPU".into();
        assessment["integrations"][1]["evidence"] = "Second evidence.".into();
        let mut coverage = serde_json::json!({"research_notes": [assessment]});
        let guide = "# CPU sharing\n\nEach process receives a virtual CPU. Read Chapter 2, CPU virtualization (PDF pages 1-2).\n\n# Virtual‑CPU\n\nFirst evidence.\n\n# Virtual-CPU\n\nSecond evidence.\n";
        let guide = canonicalize_heading_references(guide, &mut coverage, &mut [])
            .expect("duplicate headings have deterministic native anchors");
        let error = validate_supplementary_research(&coverage, &guide, &[resource('a', 3, true)])
            .unwrap_err();
        assert!(
            error.contains("guide_heading \"Virtual-CPU\" is ambiguous"),
            "{error}"
        );
    }

    #[test]
    fn primary_resource_metadata_requires_a_known_identity() {
        let mut expected = resource('a', 3, true);
        expected.title.clear();
        let error = validate_supplementary_research(
            &serde_json::json!({"research_notes": [primary_assessment()]}),
            primary_guide(),
            &[expected],
        )
        .unwrap_err();
        assert!(error.contains("known SHA-256 and title"), "{error}");
    }

    #[test]
    fn supplementary_assessments_handle_no_resources_without_invented_entries() {
        assert!(validate_supplementary_research(&serde_json::json!({}), "", &[]).is_ok());
        assert!(validate_supplementary_research(
            &serde_json::json!({"research_notes": []}),
            "",
            &[]
        )
        .is_ok());
        assert!(validate_supplementary_research(
            &serde_json::json!({"research_notes": [resource_assessment()]}),
            "",
            &[]
        )
        .is_err());
        let instruction = supplementary_research_instructions(&[resource('a', 3, true)]);
        assert!(instruction.contains(&"a".repeat(64)) && instruction.contains("research_notes"));
        assert!(instruction.contains("without leading Markdown # markers"));
        assert!(instruction.contains("\"integrations\""));
        assert!(
            !instruction.contains("reading_path") && instruction.contains("never tells the student what to read"),
            "the writer must not be asked for reading instructions"
        );
        assert!(instruction.contains("lecture-assigned chapters or sections"));
    }

    fn scratch_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "guide-watcher-artifact-bundle-test-{}",
            Uuid::new_v4()
        ));
        std::fs::create_dir(&path).expect("create scratch directory");
        path
    }

    #[test]
    fn file_hashing_is_bounded_on_a_small_stack() {
        assert_file_hash_on_small_stack(
            "artifact_bundle::tests::file_hashing_is_bounded_on_a_small_stack",
            sha256_file,
        );
    }

    fn staged_fixture(root: &Path) -> StagedLearnerAssets {
        let working_dir = root.join("learner-assets");
        std::fs::create_dir(&working_dir).expect("create staged assets");
        let catalog_dir = root.join("visual-catalog");
        std::fs::create_dir(&catalog_dir).expect("create visual catalog");
        let image = catalog_dir.join("source-slide-001.png");
        std::fs::write(&image, png_fixture(1, 1)).expect("write image");
        StagedLearnerAssets {
            working_dir,
            public_prefix: "Lecture_Guide_assets".to_string(),
            catalog: vec![AssetCatalogEntry {
                id: "source-slide-001".to_string(),
                public_path: "Lecture_Guide_assets/source-slide-001.png".to_string(),
                readable_path: image.to_string_lossy().to_string(),
                sha256: sha256_file(&image).unwrap(),
                spec_sha256: "b".repeat(64),
                width_px: 1,
                height_px: 1,
                kind: VisualAssetKind::SourceCrop,
                evidence_class: EvidenceClass::Source,
                need_ids: vec!["need-one".to_string()],
                source_unit_ids: vec!["source-test-unit-001".to_string()],
                procedure_step_ids: Vec::new(),
                learning_purpose: "Show the source state".to_string(),
                alt: "Diagram".to_string(),
                caption: "Caption.".to_string(),
                explanation: "Detail.".to_string(),
                provenance: VisualProvenance {
                    kind: "source-crop".to_string(),
                    source: "lecture.pdf".to_string(),
                    locator: "slide 1".to_string(),
                    transformation: "Rendered; unchanged.".to_string(),
                },
                rights: RightsMetadata {
                    basis: crate::visual_assets::RightsBasis::CourseProvided,
                    reuse_scope: crate::visual_assets::ReuseScope::PrivateStudy,
                    attribution: "Course lecture".to_string(),
                },
            }],
            visual_packet_sha256: "c".repeat(64),
            required_need_ids: vec!["need-one".to_string()],
            procedure_steps: Vec::new(),
        }
    }

    fn catalog_asset(
        base: &AssetCatalogEntry,
        id: &str,
        need_ids: &[&str],
        source_unit_ids: &[&str],
    ) -> AssetCatalogEntry {
        let mut entry = base.clone();
        entry.id = id.to_string();
        entry.public_path = format!("Lecture_Guide_assets/{id}.png");
        entry.need_ids = need_ids.iter().map(|need| (*need).to_string()).collect();
        entry.source_unit_ids = source_unit_ids
            .iter()
            .map(|unit| (*unit).to_string())
            .collect();
        entry
    }

    fn reproduced_omission_fixture(root: &Path) -> StagedLearnerAssets {
        let mut staged = staged_fixture(root);
        staged.catalog[0].need_ids = vec!["need-lecture".to_string()];
        staged.catalog.extend([
            catalog_asset(
                &staged.catalog[0],
                "auto-source-visual-007",
                &["need-ostep-007"],
                &[],
            ),
            catalog_asset(
                &staged.catalog[0],
                "auto-source-visual-020",
                &["need-ostep-020"],
                &[],
            ),
            catalog_asset(
                &staged.catalog[0],
                "auto-source-visual-010",
                &["need-csapp-060"],
                &[],
            ),
        ]);
        staged.required_need_ids = [
            "need-lecture",
            "need-ostep-007",
            "need-ostep-020",
            "need-csapp-060",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        staged
    }

    fn valid_artifacts() -> ModelArtifacts {
        let response = response(
            "Lecture_Guide_assets/source-slide-001.png",
            "Lecture_Guide.md",
        );
        let (_, artifact_json) = split_artifact_response(&response).unwrap();
        serde_json::from_str(artifact_json).unwrap()
    }

    fn png_fixture(width: u32, height: u32) -> Vec<u8> {
        png_fixture_with_value(width, height, 0)
    }

    fn png_fixture_with_value(width: u32, height: u32, value: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![value; width as usize * height as usize * 4])
            .unwrap();
        drop(writer);
        bytes
    }

    fn visual_material(bytes: Vec<u8>) -> VisualMaterial {
        let sha = format!("{:x}", sha2::Sha256::digest(&bytes));
        let entry = VisualCatalogEntry {
            id: "source-slide-001".to_string(),
            filename: "source-slide-001.png".to_string(),
            sha256: sha,
            spec_sha256: "b".repeat(64),
            width_px: 1,
            height_px: 1,
            kind: VisualAssetKind::SourceCrop,
            evidence_class: EvidenceClass::Source,
            need_ids: vec!["need-one".to_string()],
            source_unit_ids: vec!["source-test-unit-001".to_string()],
            procedure_step_ids: Vec::new(),
            learning_purpose: "Show the source state".to_string(),
            alt: "Diagram".to_string(),
            caption: "Caption.".to_string(),
            explanation: "Detail.".to_string(),
            provenance: VisualProvenance {
                kind: "source-crop".to_string(),
                source: "lecture.pdf".to_string(),
                locator: "slide 1".to_string(),
                transformation: "Rendered; unchanged.".to_string(),
            },
            rights: RightsMetadata {
                basis: crate::visual_assets::RightsBasis::CourseProvided,
                reuse_scope: crate::visual_assets::ReuseScope::PrivateStudy,
                attribution: "Course lecture".to_string(),
            },
        };
        VisualMaterial {
            packet_path: PathBuf::from("packet.guide-visuals.json"),
            packet_bytes: b"{}".to_vec(),
            packet_sha256: "c".repeat(64),
            dependencies: Vec::new(),
            contract: crate::visual_assets::VisualContract {
                schema_version: 2,
                packet_sha256: "c".repeat(64),
                decision: crate::visual_assets::VisualDecision::PurposefulVisuals,
                no_visuals_rationale: None,
                needs: vec![crate::visual_assets::VisualNeed {
                    id: "need-one".to_string(),
                    kind: crate::visual_assets::NeedKind::Concept,
                    source_unit_ids: vec!["source-test-unit-001".to_string()],
                    procedure_step_ids: Vec::new(),
                    learner_question: "What is shown?".to_string(),
                    misconception_prevented: "Missing source state".to_string(),
                    evidence_requirement: crate::visual_assets::EvidenceRequirement::Source,
                }],
                procedure_steps: Vec::new(),
                assets: vec![entry.clone()],
            },
            compiled_assets: vec![crate::visual_assets::CompiledVisualAsset {
                catalog: entry,
                bytes,
            }],
        }
    }

    fn contracts() -> (SourceSnapshot, PredecessorManifest) {
        (
            SourceSnapshot {
                schema_version: 2,
                primary_source_id: "source-test".to_string(),
                sources: vec![crate::source_context::SourceSnapshotEntry {
                    id: "source-test".to_string(),
                    path: "lecture.pdf".to_string(),
                    name: "lecture.pdf".to_string(),
                    sha256: "a".repeat(64),
                    size_bytes: 3,
                    role: "primary".to_string(),
                    unit_kind: "pdf-page".to_string(),
                    unit_count: 1,
                    unit_ids: vec!["source-test-unit-001".to_string()],
                }],
                unit_ids: vec!["source-test-unit-001".to_string()],
            },
            PredecessorManifest {
                schema_version: 1,
                prior_guides: Vec::new(),
            },
        )
    }

    fn response(asset_path: &str, guide_name: &str) -> String {
        let asset_id = if asset_path.contains("outside") {
            "outside"
        } else {
            "source-slide-001"
        };
        format!(
            "# Section 1\n\n![Diagram]({asset_path})\n\n*Figure: Caption.*\n\n**What to notice:** Detail.\n\n{ARTIFACTS_START}\n{{\"coverage_manifest\":{{\"schema_version\":2,\"guide\":{guide_name:?},\"guide_kind\":\"lecture\",\"primary_source_id\":\"source-test\",\"sources\":[{{\"id\":\"source-test\",\"path\":\"lecture.pdf\",\"sha256\":\"{}\",\"role\":\"primary\"}}],\"units\":[{{\"id\":\"source-test-unit-001\",\"source_id\":\"source-test\",\"guide_anchor\":\"section-1\",\"topic\":\"topic\"}}],\"continuity\":{{\"prior_guides\":[]}},\"verification_plan\":{{\"required\":false,\"rationale\":\"No recomputable claims in this fixture.\"}}}},\"visual_placements\":[{{\"asset_id\":{asset_id:?},\"section_anchor\":\"section-1\"}}],\"verification_harness\":null}}\n{ARTIFACTS_END}\n",
            "a".repeat(64),
        )
    }

    fn with_research_note(response: String, guide_heading: &str, evidence: &str) -> String {
        let notes = serde_json::to_string(&vec![serde_json::json!({
            "sha256": "a".repeat(64),
            "status": "used",
            "pages": [1],
            "locator": "Chapter 1",
            "explanation": "The resource supports this section.",
            "guide_heading": guide_heading,
            "evidence": evidence
        })])
        .unwrap();
        response.replacen(
            "\"verification_plan\":",
            &format!("\"research_notes\":{notes},\"verification_plan\":"),
            1,
        )
    }

    #[test]
    fn artifact_prompt_requires_mapped_need_coverage_without_model_reselection() {
        let root = scratch_dir();
        let mut staged = staged_fixture(&root);
        staged.catalog.push(catalog_asset(
            &staged.catalog[0],
            "source-slide-alternative",
            &["need-one"],
            &[],
        ));
        let (snapshot, predecessors) = contracts();

        let prompt = artifact_instructions(
            "Lecture_Guide.md",
            "lecture",
            &staged,
            &snapshot,
            &predecessors,
        )
        .unwrap();

        assert!(prompt.contains(
            "Visual preflight already selected the catalog as purposeful and established the required visual needs"
        ));
        assert!(prompt.contains(
            "Every app-owned required need must be covered by at least one actual embedded catalog asset listed for that need"
        ));
        assert!(prompt.contains(
            "When a required need has exactly one eligible asset, that asset is mandatory"
        ));
        assert!(prompt.contains(
            "When it has multiple eligible assets, embed at least one of them; the other alternatives are not mandatory"
        ));
        assert!(!prompt.contains("Select each asset that materially aids understanding"));
        assert!(
            prompt.contains("Sections without a selected catalog asset must contain prose only")
        );
        assert!(prompt.contains("never insert an empty or placeholder image"));
        assert!(prompt.contains("commentary about missing or catalog images"));
        let mapping = serde_json::to_value(required_need_asset_mapping(&staged)).unwrap();
        assert_eq!(
            mapping,
            serde_json::json!([{
                "required_need_id": "need-one",
                "eligible_asset_ids": ["source-slide-001", "source-slide-alternative"]
            }])
        );
        assert!(prompt.contains("APP-OWNED REQUIRED NEED TO ELIGIBLE ASSET MAPPING:"));
        assert!(prompt.contains("\"required_need_id\": \"need-one\""));
        assert!(prompt.contains("\"source-slide-alternative\""));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reports_every_omitted_book_need_with_deterministic_eligible_asset_ids() {
        let root = scratch_dir();
        let staged = reproduced_omission_fixture(&root);
        assert!(staged.catalog[1..]
            .iter()
            .all(|entry| entry.source_unit_ids.is_empty()));
        let artifacts = valid_artifacts();
        let (snapshot, predecessors) = contracts();

        let error = validate_artifacts(
            &artifacts,
            "# Section 1\n",
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .unwrap_err();

        assert_eq!(
            error,
            "visual placements are missing required app-owned visual needs: need-ostep-007 (eligible asset IDs: [auto-source-visual-007]); need-ostep-020 (eligible asset IDs: [auto-source-visual-020]); need-csapp-060 (eligible asset IDs: [auto-source-visual-010])"
        );
        assert!(!error.contains(root.to_string_lossy().as_ref()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn accepts_full_required_need_coverage_including_book_assets() {
        let root = scratch_dir();
        let staged = reproduced_omission_fixture(&root);
        let mut artifacts = valid_artifacts();
        artifacts
            .visual_placements
            .extend(staged.catalog[1..].iter().map(|entry| VisualPlacement {
                asset_id: entry.id.clone(),
                section_anchor: "section-1".to_string(),
            }));
        let (snapshot, predecessors) = contracts();

        validate_artifacts(
            &artifacts,
            "# Section 1\n",
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .expect("all required needs are covered");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn accepts_one_eligible_alternative_without_demanding_every_asset() {
        let root = scratch_dir();
        let mut staged = staged_fixture(&root);
        staged.catalog.push(catalog_asset(
            &staged.catalog[0],
            "book-alternative",
            &["need-one"],
            &[],
        ));
        let mut artifacts = valid_artifacts();
        artifacts.visual_placements = vec![VisualPlacement {
            asset_id: "book-alternative".to_string(),
            section_anchor: "section-1".to_string(),
        }];
        let (snapshot, predecessors) = contracts();

        validate_artifacts(
            &artifacts,
            "# Section 1\n",
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .expect("one eligible alternative covers the required need");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn no_purposeful_visual_contract_requires_and_instructs_empty_placements() {
        let root = scratch_dir();
        let mut staged = staged_fixture(&root);
        staged.required_need_ids.clear();
        let artifacts = valid_artifacts();
        let (snapshot, predecessors) = contracts();

        let error = validate_artifacts(
            &artifacts,
            "# Section 1\n",
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .unwrap_err();
        assert_eq!(
            error,
            "no-purposeful-visual contract requires empty visual_placements"
        );

        let mut artifacts = artifacts;
        artifacts.visual_placements.clear();
        validate_artifacts(
            &artifacts,
            "# Section 1\n",
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .expect("empty placements satisfy the no-purposeful-visual contract");
        staged.catalog.clear();
        let prompt = artifact_instructions(
            "Lecture_Guide.md",
            "lecture",
            &staged,
            &snapshot,
            &predecessors,
        )
        .unwrap();
        assert!(prompt.contains(
            "The app selected no purposeful visuals. Embed no images from the catalog and return an empty `visual_placements` list."
        ));
        assert!(prompt.contains("APP-OWNED REQUIRED NEED TO ELIGIBLE ASSET MAPPING:\n[]"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_a_partial_rendered_source_snapshot() {
        let root = scratch_dir();
        let output = root.join("Lecture_Guide.md");
        let mut material = visual_material(png_fixture(1, 1));
        material.compiled_assets[0].bytes = png_fixture_with_value(1, 1, 255);
        let error = stage_rendered_assets(&material, &root, &output)
            .expect_err("mutated compiled visual must fail before guide writing");
        assert!(error.contains("compiled visual changed"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_a_count_preserving_valid_png_replacement_after_materialization() {
        let root = scratch_dir();
        let output = root.join("Lecture_Guide.md");
        let staged = stage_rendered_assets(&visual_material(png_fixture(1, 1)), &root, &output)
            .expect("stage app-compiled visual catalog");
        assert!(staged.working_dir.read_dir().unwrap().next().is_none());
        assert!(Path::new(&staged.catalog[0].readable_path).is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn materializes_manifests_and_strips_the_private_artifact_block() {
        let root = scratch_dir();
        let verify = root.join("verify");
        std::fs::create_dir(&verify).unwrap();
        let staged = staged_fixture(&root);
        let candidate = root.join("candidate.md");
        let response = response(
            "Lecture_Guide_assets/source-slide-001.png",
            "Lecture_Guide.md",
        );
        let expected_guide = split_artifact_response(&response).unwrap().0.trim_end();
        std::fs::write(&candidate, &response).unwrap();
        let (snapshot, predecessors) = contracts();

        materialize_model_artifacts(
            &candidate,
            &verify,
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .expect("materialize artifacts");
        let guide = std::fs::read_to_string(&candidate).unwrap();
        assert_eq!(guide, expected_guide);
        assert!(!guide.contains(ARTIFACTS_START));
        assert!(verify.join("coverage_manifest.json").is_file());
        assert!(staged.working_dir.join("asset_manifest.json").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn materialized_research_evidence_can_belong_to_a_parent_section() {
        let root = scratch_dir();
        let verify = root.join("verify");
        std::fs::create_dir(&verify).unwrap();
        let staged = staged_fixture(&root);
        let candidate = root.join("candidate.md");
        let evidence = "OSTEP explains the program cycle in this nested subsection.";
        let response = response(
            "Lecture_Guide_assets/source-slide-001.png",
            "Lecture_Guide.md",
        )
        .replacen(
            "# Section 1\n\n",
            &format!("# Section 1\n\n### 1.1 Program cycle\n\n{evidence}\n\n"),
            1,
        );
        let response = with_research_note(response, "Section 1", evidence);
        std::fs::write(&candidate, response).unwrap();
        let (snapshot, predecessors) = contracts();

        materialize_model_artifacts(
            &candidate,
            &verify,
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .expect("materialize a guide with nested research evidence");

        let guide = std::fs::read_to_string(&candidate).unwrap();
        let coverage: Value = serde_json::from_str(
            &std::fs::read_to_string(verify.join("coverage_manifest.json")).unwrap(),
        )
        .unwrap();
        validate_supplementary_research(&coverage, &guide, &[resource('a', 1, false)])
            .expect("the materialized parent section should own nested paragraph evidence");
        assert!(staged.working_dir.join("asset_manifest.json").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn materialization_canonicalizes_real_heading_and_anchor_dash_cases() {
        let root = scratch_dir();
        let verify = root.join("verify");
        std::fs::create_dir(&verify).unwrap();
        let staged = staged_fixture(&root);
        let candidate = root.join("candidate.md");
        let response = response(
            "Lecture_Guide_assets/source-slide-001.png",
            "Lecture_Guide.md",
        )
        .replacen(
            "# Section 1",
            "[Current anchor](#9-the-virtual-cpu-illusion-and-its-hardware-limit)\n\n# 9. The Virtual‑CPU Illusion and Its Hardware Limit",
            1,
        )
        .replace(
            "section-1",
            "9-the-virtualcpu-illusion-and-its-hardware-limit",
        );
        std::fs::write(&candidate, response).unwrap();
        let (snapshot, predecessors) = contracts();

        materialize_model_artifacts(
            &candidate,
            &verify,
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .expect("supported heading dashes canonicalize before validation and materialization");

        let guide = std::fs::read_to_string(&candidate).unwrap();
        assert!(guide.starts_with(
            "[Current anchor](#9-the-virtual-cpu-illusion-and-its-hardware-limit)\n\n# 9. The Virtual-CPU Illusion and Its Hardware Limit\n"
        ));
        assert!(!guide.contains('‑'));
        let coverage: Value = serde_json::from_str(
            &std::fs::read_to_string(verify.join("coverage_manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            coverage["units"][0]["guide_anchor"],
            "9-the-virtual-cpu-illusion-and-its-hardware-limit"
        );
        let assets: Value = serde_json::from_str(
            &std::fs::read_to_string(staged.working_dir.join("asset_manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            assets["assets"][0]["section_anchor"],
            "9-the-virtual-cpu-illusion-and-its-hardware-limit"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn materialization_preserves_local_link_and_anchor_identity() {
        let root = scratch_dir();
        let verify = root.join("verify");
        std::fs::create_dir(&verify).unwrap();
        let staged = staged_fixture(&root);
        let candidate = root.join("candidate.md");
        let evidence = "ASCII stays byte-for-byte; prose Virtual‑CPU and 2.1–2.5 stay unchanged.";
        let response = response(
            "Lecture_Guide_assets/source-slide-001.png",
            "Lecture_Guide.md",
        )
        .replacen(
            "# Section 1",
            &format!(
                "[CPU](#virtualcpu) and [CPU titled](<#virtualcpu> \"keep–title\") and [current](#9-the-virtual-cpu-illusion-and-its-hardware-limit).\n\n[CPU reference][cpu]\n\n[cpu]: #virtualcpu \"reference–title\"\n\n`[code](#virtualcpu)` and [external](https://example.test/path#virtualcpu \"external–title\").\n\n# Virtual‑CPU\n\n{evidence}"
            ),
            1,
        )
        .replace("\"guide_anchor\":\"section-1\"", "\"guide_anchor\":\"virtualcpu\"")
        .replace(
            "\"section_anchor\":\"section-1\"",
            "\"section_anchor\":\"virtualcpu\"",
        )
        .replacen(
            &format!("\n\n{ARTIFACTS_START}"),
            &format!(
                "\n\n# 9. The Virtual‑CPU Illusion and Its Hardware Limit\n\nCanonical target.\n\n{ARTIFACTS_START}"
            ),
            1,
        );
        let response = with_research_note(response, "Virtual‑CPU", evidence);
        std::fs::write(&candidate, response).unwrap();
        let (snapshot, predecessors) = contracts();

        materialize_model_artifacts(
            &candidate,
            &verify,
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .expect("heading targets and local fragments should retain section identity");

        let guide = std::fs::read_to_string(&candidate).unwrap();
        assert!(guide.contains("[CPU](#virtual-cpu)"));
        assert!(guide.contains("[CPU titled](<#virtual-cpu> \"keep–title\")"));
        assert!(guide.contains("[cpu]: #virtual-cpu \"reference–title\""));
        assert!(guide.contains("[current](#9-the-virtual-cpu-illusion-and-its-hardware-limit)"));
        assert!(guide.contains("`[code](#virtualcpu)`"));
        assert!(
            guide.contains("[external](https://example.test/path#virtualcpu \"external–title\")")
        );
        assert!(guide.contains(evidence));
        assert!(guide.contains("# Virtual-CPU\n"));
        let coverage: Value = serde_json::from_str(
            &std::fs::read_to_string(verify.join("coverage_manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(coverage["units"][0]["guide_anchor"], "virtual-cpu");
        assert_eq!(
            coverage["research_notes"][0]["guide_heading"],
            "Virtual-CPU"
        );
        assert_eq!(coverage["research_notes"][0]["evidence"], evidence);
        let assets: Value = serde_json::from_str(
            &std::fs::read_to_string(staged.working_dir.join("asset_manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(assets["assets"][0]["section_anchor"], "virtual-cpu");
        validate_supplementary_research(&coverage, &guide, &[resource('a', 1, false)])
            .expect("mapped visible heading should retain exact evidence matching");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn materialization_preserves_inline_code_in_visible_heading_metadata() {
        let root = scratch_dir();
        let verify = root.join("verify");
        std::fs::create_dir(&verify).unwrap();
        let staged = staged_fixture(&root);
        let candidate = root.join("candidate.md");
        let evidence = "Evidence keeps `x‑y`, https://example.test/a–b#frag, identity‑bytes, −, and 한글 exact.";
        let response = response(
            "Lecture_Guide_assets/source-slide-001.png",
            "Lecture_Guide.md",
        )
        .replacen(
            "# Section 1",
            &format!("# The `Non‑Atomic` update\n\n{evidence}"),
            1,
        )
        .replace(
            "\"guide_anchor\":\"section-1\"",
            "\"guide_anchor\":\"the-nonatomic-update\"",
        )
        .replace(
            "\"section_anchor\":\"section-1\"",
            "\"section_anchor\":\"the-nonatomic-update\"",
        )
        .replacen(
            &format!("\n\n{ARTIFACTS_START}"),
            &format!(
                "\n\n# Mixed Virtual‑CPU and `Non‑Atomic` − 한글\n\nMixed heading body.\n\n{ARTIFACTS_START}"
            ),
            1,
        );
        let response = with_research_note(response, "The Non‑Atomic update", evidence);
        std::fs::write(&candidate, response).unwrap();
        let (snapshot, predecessors) = contracts();

        materialize_model_artifacts(
            &candidate,
            &verify,
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .expect("inline code should remain exact while ordinary heading text canonicalizes");

        let guide = std::fs::read_to_string(&candidate).unwrap();
        assert!(guide.contains("# The `Non‑Atomic` update\n"));
        assert!(guide.contains("# Mixed Virtual-CPU and `Non‑Atomic` − 한글\n"));
        assert!(guide.contains(evidence));
        let coverage: Value = serde_json::from_str(
            &std::fs::read_to_string(verify.join("coverage_manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            coverage["research_notes"][0]["guide_heading"],
            "The Non‑Atomic update"
        );
        assert_eq!(coverage["research_notes"][0]["evidence"], evidence);
        validate_supplementary_research(&coverage, &guide, &[resource('a', 1, false)])
            .expect("preserved code spelling should match exact visible heading evidence");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn materialization_numbers_duplicate_heading_targets_deterministically() {
        let root = scratch_dir();
        let verify = root.join("verify");
        std::fs::create_dir(&verify).unwrap();
        let staged = staged_fixture(&root);
        let candidate = root.join("candidate.md");
        let response = response(
            "Lecture_Guide_assets/source-slide-001.png",
            "Lecture_Guide.md",
        )
        .replacen(
            "# Section 1",
            "[first](#virtualcpu) [second](#virtualcpu-1)\n\n# Virtual‑CPU",
            1,
        )
        .replace(
            "\"guide_anchor\":\"section-1\"",
            "\"guide_anchor\":\"virtualcpu\"",
        )
        .replace(
            "\"section_anchor\":\"section-1\"",
            "\"section_anchor\":\"virtualcpu\"",
        )
        .replacen(
            &format!("\n\n{ARTIFACTS_START}"),
            &format!("\n\n# Virtual‑CPU\n\nSecond section.\n\n{ARTIFACTS_START}"),
            1,
        );
        std::fs::write(&candidate, response).unwrap();
        let (snapshot, predecessors) = contracts();

        materialize_model_artifacts(
            &candidate,
            &verify,
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .expect("duplicate headings should retain their occurrence identity");

        let guide = std::fs::read_to_string(&candidate).unwrap();
        assert!(guide.contains("[first](#virtual-cpu) [second](#virtual-cpu-1)"));
        assert_eq!(guide.matches("# Virtual-CPU\n").count(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn materialization_rejects_ambiguous_heading_anchor_collisions() {
        let root = scratch_dir();
        let verify = root.join("verify");
        std::fs::create_dir(&verify).unwrap();
        let staged = staged_fixture(&root);
        let candidate = root.join("candidate.md");
        let response = response(
            "Lecture_Guide_assets/source-slide-001.png",
            "Lecture_Guide.md",
        )
        .replacen("# Section 1", "# A‑B", 1)
        .replace("\"guide_anchor\":\"section-1\"", "\"guide_anchor\":\"ab\"")
        .replace(
            "\"section_anchor\":\"section-1\"",
            "\"section_anchor\":\"ab\"",
        )
        .replacen(
            &format!("\n\n{ARTIFACTS_START}"),
            &format!("\n\n# A‑B\n\n# A-B-1\n\n{ARTIFACTS_START}"),
            1,
        );
        std::fs::write(&candidate, &response).unwrap();
        let (snapshot, predecessors) = contracts();

        let error = materialize_model_artifacts(
            &candidate,
            &verify,
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .unwrap_err();

        assert!(
            error.contains("heading anchor \"a-b-1\" is ambiguous"),
            "{error}"
        );
        assert_eq!(std::fs::read_to_string(&candidate).unwrap(), response);
        assert!(!verify.join("coverage_manifest.json").exists());
        assert!(staged.working_dir.read_dir().unwrap().next().is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn materialization_rejects_ambiguous_visible_heading_mappings() {
        let root = scratch_dir();
        let verify = root.join("verify");
        std::fs::create_dir(&verify).unwrap();
        let staged = staged_fixture(&root);
        let candidate = root.join("candidate.md");
        let response = response(
            "Lecture_Guide_assets/source-slide-001.png",
            "Lecture_Guide.md",
        )
        .replacen("# Section 1", "# A‑B", 1)
        .replace("\"guide_anchor\":\"section-1\"", "\"guide_anchor\":\"ab\"")
        .replace(
            "\"section_anchor\":\"section-1\"",
            "\"section_anchor\":\"ab\"",
        )
        .replacen(
            &format!("\n\n{ARTIFACTS_START}"),
            &format!("\n\n# `A‑B`\n\nCode section.\n\n{ARTIFACTS_START}"),
            1,
        );
        std::fs::write(&candidate, &response).unwrap();
        let (snapshot, predecessors) = contracts();

        let error = materialize_model_artifacts(
            &candidate,
            &verify,
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .unwrap_err();

        assert!(
            error.contains("visible heading mapping is ambiguous"),
            "{error}"
        );
        assert_eq!(std::fs::read_to_string(&candidate).unwrap(), response);
        assert!(!verify.join("coverage_manifest.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn materialization_rejects_encoded_heading_dashes_before_publication() {
        let root = scratch_dir();
        let verify = root.join("verify");
        std::fs::create_dir(&verify).unwrap();
        let staged = staged_fixture(&root);
        let candidate = root.join("candidate.md");
        let response = response(
            "Lecture_Guide_assets/source-slide-001.png",
            "Lecture_Guide.md",
        )
        .replacen("# Section 1", "# Virtual&ndash;CPU", 1);
        std::fs::write(&candidate, &response).unwrap();
        let (snapshot, predecessors) = contracts();

        let error = materialize_model_artifacts(
            &candidate,
            &verify,
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .unwrap_err();

        assert!(
            error.contains("unsupported encoded typographic dash"),
            "{error}"
        );
        assert_eq!(std::fs::read_to_string(&candidate).unwrap(), response);
        assert!(!verify.join("coverage_manifest.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn materialization_rejects_an_empty_image_destination() {
        let root = scratch_dir();
        let verify = root.join("verify");
        std::fs::create_dir(&verify).unwrap();
        let staged = staged_fixture(&root);
        let candidate = root.join("candidate.md");
        let response = response(
            "Lecture_Guide_assets/source-slide-001.png",
            "Lecture_Guide.md",
        )
        .replace(
            "![Diagram](Lecture_Guide_assets/source-slide-001.png)",
            "![There is no learner image for this section.]()",
        );
        std::fs::write(&candidate, &response).unwrap();
        let (snapshot, predecessors) = contracts();

        let error = materialize_model_artifacts(
            &candidate,
            &verify,
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .unwrap_err();

        assert!(
            error.contains("image destination must be nonempty"),
            "{error}"
        );
        assert_eq!(std::fs::read_to_string(&candidate).unwrap(), response);
        assert!(!verify.join("coverage_manifest.json").exists());
        assert!(staged.working_dir.read_dir().unwrap().next().is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_unknown_asset_paths_and_trailing_content() {
        let root = scratch_dir();
        let verify = root.join("verify");
        std::fs::create_dir(&verify).unwrap();
        let staged = staged_fixture(&root);
        let candidate = root.join("candidate.md");
        let (snapshot, predecessors) = contracts();
        std::fs::write(&candidate, response("../outside.png", "Lecture_Guide.md")).unwrap();
        let error = materialize_model_artifacts(
            &candidate,
            &verify,
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .unwrap_err();
        assert!(error.contains("unknown asset"));

        let invalid = format!(
            "{}unexpected",
            response(
                "Lecture_Guide_assets/source-slide-001.png",
                "Lecture_Guide.md"
            )
        );
        std::fs::write(&candidate, invalid).unwrap();
        let error = materialize_model_artifacts(
            &candidate,
            &verify,
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .unwrap_err();
        assert!(error.contains("final response content"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_a_model_selected_guide_kind() {
        let root = scratch_dir();
        let verify = root.join("verify");
        std::fs::create_dir(&verify).unwrap();
        let staged = staged_fixture(&root);
        let candidate = root.join("candidate.md");
        std::fs::write(
            &candidate,
            response(
                "Lecture_Guide_assets/source-slide-001.png",
                "Lecture_Guide.md",
            ),
        )
        .unwrap();
        let (snapshot, predecessors) = contracts();

        let error = materialize_model_artifacts(
            &candidate,
            &verify,
            &staged,
            "Lecture_Guide.md",
            "circuit-lab",
            &snapshot,
            &predecessors,
        )
        .unwrap_err();
        assert!(error.contains("guide_kind"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_an_unnecessary_model_authored_verification_harness() {
        let root = scratch_dir();
        let verify = root.join("verify");
        std::fs::create_dir(&verify).unwrap();
        let staged = staged_fixture(&root);
        let candidate = root.join("candidate.md");
        let response = response(
            "Lecture_Guide_assets/source-slide-001.png",
            "Lecture_Guide.md",
        )
        .replace(
            "\"verification_harness\":null",
            "\"verification_harness\":\"assert True\\nassert True\\nassert True\"",
        );
        std::fs::write(&candidate, response).unwrap();
        let (snapshot, predecessors) = contracts();

        let error = materialize_model_artifacts(
            &candidate,
            &verify,
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .unwrap_err();
        assert!(error.contains("forbidden"), "{error}");
        assert!(!verify.join("verify.py").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preexisting_model_code_is_rejected_without_deletion_or_execution() {
        let root = scratch_dir();
        let verify = root.join("verify");
        std::fs::create_dir(&verify).unwrap();
        let marker = root.join("must-not-exist");
        let harness = verify.join("verify.py");
        std::fs::write(
            &harness,
            format!("from pathlib import Path\nPath({marker:?}).touch()\n"),
        )
        .unwrap();
        let staged = staged_fixture(&root);
        let candidate = root.join("candidate.md");
        std::fs::write(
            &candidate,
            response(
                "Lecture_Guide_assets/source-slide-001.png",
                "Lecture_Guide.md",
            ),
        )
        .unwrap();
        let (snapshot, predecessors) = contracts();

        let error = materialize_model_artifacts(
            &candidate,
            &verify,
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .unwrap_err();

        assert!(error.contains("forbidden model-authored executable code"));
        assert!(harness.is_file());
        assert!(!marker.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn app_forbids_model_code_even_with_six_arithmetic_equalities() {
        let root = scratch_dir();
        let verify = root.join("verify");
        std::fs::create_dir(&verify).unwrap();
        let staged = staged_fixture(&root);
        let candidate = root.join("candidate.md");
        let response = response(
            "Lecture_Guide_assets/source-slide-001.png",
            "Lecture_Guide.md",
        )
        .replacen(
            "# Section 1",
            "# Section 1\n\n1 + 1 = 2\n\n2 + 2 = 4\n\n3 + 3 = 6\n\n4 + 4 = 8\n\n5 + 5 = 10\n\n6 + 6 = 12",
            1,
        );
        std::fs::write(&candidate, response).unwrap();
        let (snapshot, predecessors) = contracts();

        materialize_model_artifacts(
            &candidate,
            &verify,
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .unwrap();
        assert!(!verify.join("verify.py").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_omitted_support_sources_and_unexplained_predecessors() {
        let root = scratch_dir();
        let staged = staged_fixture(&root);
        let (mut snapshot, _) = contracts();
        snapshot
            .sources
            .push(crate::source_context::SourceSnapshotEntry {
                id: "source-support".to_string(),
                path: "worksheet.docx".to_string(),
                name: "worksheet.docx".to_string(),
                sha256: "b".repeat(64),
                size_bytes: 4,
                role: "support".to_string(),
                unit_kind: "docx-document".to_string(),
                unit_count: 1,
                unit_ids: vec!["source-support-unit-001".to_string()],
            });
        snapshot
            .unit_ids
            .push("source-support-unit-001".to_string());
        let predecessors = PredecessorManifest {
            schema_version: 1,
            prior_guides: vec![crate::course_plan::BoundPredecessor {
                course_profile: "course".to_string(),
                generation_identity: "course:lecture:1".to_string(),
                sequence_key: "lecture 1".to_string(),
                path: "Lecture_1_Guide.md".to_string(),
                sha256: "c".repeat(64),
            }],
        };
        let coverage = serde_json::json!({
            "schema_version": 2,
            "guide": "Lecture_Guide.md",
            "guide_kind": "lecture",
            "primary_source_id": "source-test",
            "sources": [{
                "id": "source-test", "path": "lecture.pdf", "sha256": "a".repeat(64), "role": "primary"
            }],
            "units": [{
                "id": "source-test-unit-001", "source_id": "source-test", "guide_anchor": "one", "topic": "one"
            }, {
                "id": "source-support-unit-001", "source_id": "source-support", "guide_anchor": "two", "topic": "two"
            }],
            "continuity": {"prior_guides": []}
        });
        let artifacts = ModelArtifacts {
            coverage_manifest: coverage,
            visual_placements: Vec::new(),
            verification_harness: None,
        };
        let error = validate_artifacts(
            &artifacts,
            "# One\n",
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .unwrap_err();
        assert!(error.contains("every selected source"), "{error}");

        let mut fixed_sources = artifacts.coverage_manifest.clone();
        fixed_sources["sources"] = serde_json::json!([{
            "id": "source-test", "path": "lecture.pdf", "sha256": "a".repeat(64), "role": "primary"
        }, {
            "id": "source-support", "path": "worksheet.docx", "sha256": "b".repeat(64), "role": "support"
        }]);
        let artifacts = ModelArtifacts {
            coverage_manifest: fixed_sources,
            visual_placements: Vec::new(),
            verification_harness: None,
        };
        let error = validate_artifacts(
            &artifacts,
            "# One\n",
            &staged,
            "Lecture_Guide.md",
            "lecture",
            &snapshot,
            &predecessors,
        )
        .unwrap_err();
        assert!(error.contains("every bound predecessor"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }
}
