use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path};

const V2_PREFIX: &str = "<!-- guide-watcher:complete:v2:sha256=";
const V3_PREFIX: &str = "<!-- guide-watcher:complete:v3 ";
const MARKER_SUFFIX: &str = " -->";
/// Trailer an owner writes when revising a sealed guide by hand. Its presence supersedes a
/// stale receipt: the guide is complete and trusted, but not app-sealed.
pub const MANUAL_REVISION_MARKER: &str = "guide-watcher: manual-revision";
/// Phrase used by the first two manual revisions before the marker above was standardized.
const LEGACY_MANUAL_REVISION_PHRASE: &str = "manually revised after its automated verification seal";
const RESERVED_PREFIX: &str = "<!-- guide-watcher:complete:v";
pub(crate) const BUNDLE_MANIFEST_NAME: &str = "bundle_manifest.v3.json";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompletionStatus {
    ValidV3,
    /// Revised by hand after sealing; complete and usable, not app-sealed.
    ManualRevision,
    InvalidV3(String),
    LegacyV2GuideOnlyValid,
    LegacyV2Invalid,
    LegacyV1Unverified,
    Unsealed,
    Missing,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum BundleMemberKind {
    File,
    Directory,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BundleMember {
    pub kind: BundleMemberKind,
    pub role: String,
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BundleOutput {
    pub guide: String,
    pub assets_dir: String,
    pub verification_dir: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BundleApp {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GuideBodyRecord {
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BundleManifest {
    pub schema_version: u32,
    pub bundle_id: String,
    pub app: BundleApp,
    pub output: BundleOutput,
    pub source_sha256: String,
    pub guide_body: GuideBodyRecord,
    pub members: Vec<BundleMember>,
    pub bundle_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SealRecord {
    pub bundle_id: String,
    pub guide_body_size: u64,
    pub guide_body_sha256: String,
    pub manifest_sha256: String,
    pub bundle_sha256: String,
    pub sealed_guide_sha256: String,
}

#[derive(Debug)]
struct V3Receipt {
    bundle_id: String,
    guide_body_size: u64,
    guide_body_sha256: String,
    manifest_sha256: String,
    bundle_sha256: String,
}

pub fn contains_reserved_marker(text: &str) -> bool {
    text.contains(RESERVED_PREFIX)
}

/// Legacy receipt writer retained only for reading old accepted guides and tests.
#[cfg(test)]
pub fn append_receipt(path: &Path) -> Result<(), String> {
    let body =
        std::fs::read(path).map_err(|error| format!("failed to read verified guide: {error}"))?;
    if body.is_empty() {
        return Err("refusing to mark an empty guide complete".to_string());
    }
    if contains_reserved_marker(&String::from_utf8_lossy(&body)) {
        return Err("refusing to seal a guide containing a reserved completion marker".to_string());
    }
    let marker = format!("\n{V2_PREFIX}{}{MARKER_SUFFIX}\n", sha256_bytes(&body));
    let mut file = OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|error| format!("failed to append completion receipt: {error}"))?;
    file.write_all(marker.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("failed to append completion receipt: {error}"))
}

/// Create the immutable v3 manifest and append a receipt to a staged guide.
/// `assets_dir` and `verification_dir` must be the exact staged trees that will
/// be renamed to the final sibling targets without any further mutation.
#[cfg(test)]
pub(crate) fn seal_v3_bundle(
    guide_path: &Path,
    assets_dir: &Path,
    verification_dir: &Path,
    final_guide: &Path,
    source_sha256: &str,
    bundle_id: &str,
) -> Result<SealRecord, String> {
    seal_v3_bundle_with_version(
        guide_path,
        assets_dir,
        verification_dir,
        final_guide,
        source_sha256,
        bundle_id,
        env!("CARGO_PKG_VERSION"),
    )
}

pub(crate) fn seal_v3_bundle_with_version(
    guide_path: &Path,
    assets_dir: &Path,
    verification_dir: &Path,
    final_guide: &Path,
    source_sha256: &str,
    bundle_id: &str,
    producer_version: &str,
) -> Result<SealRecord, String> {
    validate_uuid(bundle_id)?;
    validate_sha(source_sha256, "source SHA-256")?;
    validate_v3_producer_version(producer_version)?;
    ensure_plain_file(guide_path)?;
    ensure_plain_directory(assets_dir)?;
    ensure_plain_directory(verification_dir)?;
    let manifest_path = verification_dir.join(BUNDLE_MANIFEST_NAME);
    if std::fs::symlink_metadata(&manifest_path).is_ok() {
        return Err(format!(
            "refusing to replace an existing v3 bundle manifest: {}",
            manifest_path.display()
        ));
    }

    let body = std::fs::read(guide_path)
        .map_err(|error| format!("could not read staged guide body: {error}"))?;
    if body.is_empty() {
        return Err("refusing to seal an empty guide".to_string());
    }
    if contains_reserved_marker(&String::from_utf8_lossy(&body)) {
        return Err("staged guide contains a reserved completion marker".to_string());
    }
    let guide_body = GuideBodyRecord {
        size_bytes: body.len() as u64,
        sha256: sha256_bytes(&body),
    };
    let mut members = collect_members(assets_dir, verification_dir, true)?;
    members.sort_by(|left, right| left.path.cmp(&right.path));
    ensure_unique_members(&members)?;

    let output = bundle_output(final_guide)?;
    let bundle_sha256 =
        compute_bundle_sha256(bundle_id, &output, source_sha256, &guide_body, &members);
    let manifest = BundleManifest {
        schema_version: 3,
        bundle_id: bundle_id.to_string(),
        app: BundleApp {
            name: "guide-watcher".to_string(),
            version: producer_version.to_string(),
        },
        output,
        source_sha256: source_sha256.to_string(),
        guide_body: guide_body.clone(),
        members,
        bundle_sha256: bundle_sha256.clone(),
    };
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("could not serialize v3 bundle manifest: {error}"))?;
    manifest_bytes.push(b'\n');
    write_new_synced(&manifest_path, &manifest_bytes)?;
    sync_directory(verification_dir)?;
    let manifest_sha256 = sha256_bytes(&manifest_bytes);
    let marker = format!(
        "\n{V3_PREFIX}bundle_id={} guide_body_size={} guide_body_sha256={} manifest_sha256={} bundle_sha256={}{MARKER_SUFFIX}\n",
        bundle_id,
        guide_body.size_bytes,
        guide_body.sha256,
        manifest_sha256,
        bundle_sha256
    );
    let mut guide = OpenOptions::new()
        .append(true)
        .open(guide_path)
        .map_err(|error| format!("could not open staged guide for v3 sealing: {error}"))?;
    guide
        .write_all(marker.as_bytes())
        .and_then(|_| guide.sync_all())
        .map_err(|error| format!("could not durably seal staged guide: {error}"))?;
    let sealed_guide_sha256 = sha256_file(guide_path)?;
    Ok(SealRecord {
        bundle_id: bundle_id.to_string(),
        guide_body_size: guide_body.size_bytes,
        guide_body_sha256: guide_body.sha256,
        manifest_sha256,
        bundle_sha256,
        sealed_guide_sha256,
    })
}

/// App-sealed only (no manual revisions); production code uses `is_usable_guide`.
#[cfg(test)]
pub fn has_valid_receipt(path: &Path) -> bool {
    matches!(
        inspect_completion(path),
        CompletionStatus::ValidV3 | CompletionStatus::LegacyV2GuideOnlyValid
    )
}

pub fn has_valid_bundle_receipt(path: &Path) -> bool {
    matches!(inspect_completion(path), CompletionStatus::ValidV3)
}

/// A guide that may be read, opened, and built upon: app-sealed, or revised by hand.
pub fn is_usable_guide(path: &Path) -> bool {
    matches!(
        inspect_completion(path),
        CompletionStatus::ValidV3
            | CompletionStatus::LegacyV2GuideOnlyValid
            | CompletionStatus::ManualRevision
    )
}

fn has_manual_revision_marker(bytes: &[u8]) -> bool {
    [MANUAL_REVISION_MARKER, LEGACY_MANUAL_REVISION_PHRASE]
        .iter()
        .any(|phrase| bytes.windows(phrase.len()).any(|window| window == phrase.as_bytes()))
}

pub fn inspect_completion(path: &Path) -> CompletionStatus {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return CompletionStatus::Missing;
        }
        Err(error) => return CompletionStatus::InvalidV3(error.to_string()),
    };
    let manual = has_manual_revision_marker(&bytes);
    if bytes
        .windows(V3_PREFIX.len())
        .any(|window| window == V3_PREFIX.as_bytes())
    {
        return match validate_v3_bundle(path, &bytes) {
            Ok(()) => CompletionStatus::ValidV3,
            // The owner's revision note outranks a receipt that no longer describes the file.
            Err(_) if manual => CompletionStatus::ManualRevision,
            Err(error) => CompletionStatus::InvalidV3(error),
        };
    }
    if manual {
        return CompletionStatus::ManualRevision;
    }
    if has_valid_receipt_bytes(&bytes) {
        CompletionStatus::LegacyV2GuideOnlyValid
    } else if bytes
        .windows(V2_PREFIX.len())
        .any(|window| window == V2_PREFIX.as_bytes())
    {
        CompletionStatus::LegacyV2Invalid
    } else if bytes
        .windows(b"<!-- guide-watcher:complete:v1 -->".len())
        .any(|window| window == b"<!-- guide-watcher:complete:v1 -->")
    {
        CompletionStatus::LegacyV1Unverified
    } else {
        CompletionStatus::Unsealed
    }
}

pub fn has_valid_receipt_bytes(bytes: &[u8]) -> bool {
    let prefix = V2_PREFIX.as_bytes();
    let marker_start = match bytes
        .windows(prefix.len())
        .rposition(|window| window == prefix)
    {
        Some(index) if index > 0 && bytes[index - 1] == b'\n' => index,
        _ => return false,
    };
    let digest_start = marker_start + prefix.len();
    let digest_end = digest_start + 64;
    if digest_end + MARKER_SUFFIX.len() > bytes.len() {
        return false;
    }
    let claimed = &bytes[digest_start..digest_end];
    if !is_lower_hex_bytes(claimed)
        || !bytes[digest_end..].starts_with(MARKER_SUFFIX.as_bytes())
        || !bytes[digest_end + MARKER_SUFFIX.len()..]
            .iter()
            .all(u8::is_ascii_whitespace)
    {
        return false;
    }
    let body = &bytes[..marker_start - 1];
    sha256_bytes(body).as_bytes() == claimed
}

fn validate_v3_bundle(guide_path: &Path, bytes: &[u8]) -> Result<(), String> {
    ensure_plain_file(guide_path)?;
    let (body, receipt) = parse_v3_receipt(bytes)?;
    if body.len() as u64 != receipt.guide_body_size
        || sha256_bytes(body) != receipt.guide_body_sha256
    {
        return Err("v3 receipt does not match the exact guide body".to_string());
    }
    let parent = guide_path
        .parent()
        .ok_or_else(|| "guide has no parent directory".to_string())?;
    let output = bundle_output(guide_path)?;
    let assets_dir = parent.join(&output.assets_dir);
    let verification_dir = parent.join(&output.verification_dir);
    ensure_plain_directory(&assets_dir)?;
    ensure_plain_directory(&verification_dir)?;
    let manifest_path = verification_dir.join(BUNDLE_MANIFEST_NAME);
    ensure_plain_file(&manifest_path)?;
    let manifest_bytes = std::fs::read(&manifest_path)
        .map_err(|error| format!("could not read v3 bundle manifest: {error}"))?;
    if sha256_bytes(&manifest_bytes) != receipt.manifest_sha256 {
        return Err("v3 bundle manifest digest does not match the receipt".to_string());
    }
    let manifest: BundleManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("v3 bundle manifest is invalid JSON: {error}"))?;
    validate_manifest_shape(&manifest, &receipt, &output)?;
    let actual = collect_members(&assets_dir, &verification_dir, true)?;
    let expected = manifest
        .members
        .iter()
        .map(|member| (member.path.clone(), member.clone()))
        .collect::<BTreeMap<_, _>>();
    let actual = actual
        .into_iter()
        .map(|member| (member.path.clone(), member))
        .collect::<BTreeMap<_, _>>();
    if expected != actual {
        return Err("published bundle members differ from the sealed manifest".to_string());
    }
    let computed = compute_bundle_sha256(
        &manifest.bundle_id,
        &manifest.output,
        &manifest.source_sha256,
        &manifest.guide_body,
        &manifest.members,
    );
    if computed != manifest.bundle_sha256 || computed != receipt.bundle_sha256 {
        return Err("v3 bundle root digest is invalid".to_string());
    }
    Ok(())
}

fn parse_v3_receipt(bytes: &[u8]) -> Result<(&[u8], V3Receipt), String> {
    let start = bytes
        .windows(V3_PREFIX.len())
        .rposition(|window| window == V3_PREFIX.as_bytes())
        .ok_or_else(|| "missing v3 receipt".to_string())?;
    if start == 0 || bytes[start - 1] != b'\n' {
        return Err("v3 receipt is not on its own final line".to_string());
    }
    let tail = std::str::from_utf8(&bytes[start + V3_PREFIX.len()..])
        .map_err(|_| "v3 receipt is not valid UTF-8".to_string())?;
    let suffix = tail
        .find(MARKER_SUFFIX)
        .ok_or_else(|| "v3 receipt is unterminated".to_string())?;
    if !tail[suffix + MARKER_SUFFIX.len()..]
        .bytes()
        .all(|byte| byte.is_ascii_whitespace())
    {
        return Err("v3 receipt is not the final content".to_string());
    }
    let mut fields = BTreeMap::new();
    for field in tail[..suffix].split_ascii_whitespace() {
        let (name, value) = field
            .split_once('=')
            .ok_or_else(|| "malformed v3 receipt field".to_string())?;
        if fields.insert(name, value).is_some() {
            return Err("v3 receipt contains a duplicate field".to_string());
        }
    }
    if fields.len() != 5 {
        return Err("v3 receipt has missing, duplicate, or extra fields".to_string());
    }
    let value = |name: &str| {
        fields
            .get(name)
            .copied()
            .ok_or_else(|| format!("v3 receipt is missing {name}"))
    };
    let receipt = V3Receipt {
        bundle_id: value("bundle_id")?.to_string(),
        guide_body_size: value("guide_body_size")?
            .parse::<u64>()
            .map_err(|_| "v3 guide_body_size is invalid".to_string())?,
        guide_body_sha256: value("guide_body_sha256")?.to_string(),
        manifest_sha256: value("manifest_sha256")?.to_string(),
        bundle_sha256: value("bundle_sha256")?.to_string(),
    };
    validate_uuid(&receipt.bundle_id)?;
    validate_sha(&receipt.guide_body_sha256, "guide body SHA-256")?;
    validate_sha(&receipt.manifest_sha256, "manifest SHA-256")?;
    validate_sha(&receipt.bundle_sha256, "bundle SHA-256")?;
    Ok((&bytes[..start - 1], receipt))
}

fn validate_manifest_shape(
    manifest: &BundleManifest,
    receipt: &V3Receipt,
    output: &BundleOutput,
) -> Result<(), String> {
    if manifest.schema_version != 3
        || manifest.app.name != "guide-watcher"
        || manifest.bundle_id != receipt.bundle_id
        || &manifest.output != output
        || manifest.guide_body.size_bytes != receipt.guide_body_size
        || manifest.guide_body.sha256 != receipt.guide_body_sha256
        || manifest.bundle_sha256 != receipt.bundle_sha256
    {
        return Err("v3 bundle manifest identity does not match the receipt or output".to_string());
    }
    validate_v3_producer_version(&manifest.app.version)?;
    validate_sha(&manifest.source_sha256, "manifest source SHA-256")?;
    validate_sha(&manifest.guide_body.sha256, "manifest guide SHA-256")?;
    ensure_unique_members(&manifest.members)?;
    let mut previous: Option<&str> = None;
    for member in &manifest.members {
        validate_member(member)?;
        if previous.is_some_and(|value| value >= member.path.as_str()) {
            return Err("v3 manifest members are not strictly sorted".to_string());
        }
        previous = Some(&member.path);
    }
    Ok(())
}

fn validate_member(member: &BundleMember) -> Result<(), String> {
    if member.role != "asset" && member.role != "verification" {
        return Err("v3 manifest member has an invalid role".to_string());
    }
    validate_sha(&member.sha256, "member SHA-256")?;
    if member.kind == BundleMemberKind::Directory
        && (member.size_bytes != 0 || member.sha256 != directory_sha256())
    {
        return Err("v3 directory member has invalid canonical metadata".to_string());
    }
    let expected_prefix = if member.role == "asset" {
        "assets/"
    } else {
        "verification/"
    };
    if !member.path.starts_with(expected_prefix) || !safe_relative(&member.path) {
        return Err("v3 manifest member has an unsafe or mismatched path".to_string());
    }
    if member.path == format!("verification/{BUNDLE_MANIFEST_NAME}") {
        return Err("v3 manifest must not list itself as a member".to_string());
    }
    Ok(())
}

fn collect_members(
    assets_dir: &Path,
    verification_dir: &Path,
    omit_manifest: bool,
) -> Result<Vec<BundleMember>, String> {
    let mut members = Vec::new();
    collect_tree(assets_dir, "asset", "assets", None, &mut members)?;
    collect_tree(
        verification_dir,
        "verification",
        "verification",
        omit_manifest.then_some(BUNDLE_MANIFEST_NAME),
        &mut members,
    )?;
    members.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(members)
}

fn collect_tree(
    root: &Path,
    role: &str,
    prefix: &str,
    omitted_root_file: Option<&str>,
    members: &mut Vec<BundleMember>,
) -> Result<(), String> {
    ensure_plain_directory(root)?;
    fn walk(
        root: &Path,
        dir: &Path,
        role: &str,
        prefix: &str,
        omitted_root_file: Option<&str>,
        members: &mut Vec<BundleMember>,
    ) -> Result<(), String> {
        for entry in std::fs::read_dir(dir)
            .map_err(|error| format!("could not enumerate bundle directory: {error}"))?
        {
            let entry = entry.map_err(|error| format!("could not read bundle entry: {error}"))?;
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|_| "bundle member escaped its root".to_string())?;
            let relative_text = relative_path(relative)?;
            if dir == root && omitted_root_file.is_some_and(|name| relative_text.as_str() == name) {
                continue;
            }
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| format!("could not inspect bundle member: {error}"))?;
            reject_link_or_reparse(&path, &metadata)?;
            if metadata.is_dir() {
                members.push(BundleMember {
                    kind: BundleMemberKind::Directory,
                    role: role.to_string(),
                    path: format!("{prefix}/{relative_text}"),
                    size_bytes: 0,
                    sha256: directory_sha256(),
                });
                walk(root, &path, role, prefix, omitted_root_file, members)?;
            } else if metadata.is_file() {
                members.push(BundleMember {
                    kind: BundleMemberKind::File,
                    role: role.to_string(),
                    path: format!("{prefix}/{relative_text}"),
                    size_bytes: metadata.len(),
                    sha256: sha256_file(&path)?,
                });
            } else {
                return Err(format!(
                    "bundle contains a non-file, non-directory entry: {}",
                    path.display()
                ));
            }
        }
        Ok(())
    }
    walk(root, root, role, prefix, omitted_root_file, members)
}

fn bundle_output(guide_path: &Path) -> Result<BundleOutput, String> {
    let guide = guide_path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "guide filename is not valid Unicode".to_string())?;
    let stem = guide_path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "guide stem is not valid Unicode".to_string())?;
    Ok(BundleOutput {
        guide: guide.to_string(),
        assets_dir: format!("{stem}_assets"),
        verification_dir: format!(".{guide}.gwverify"),
    })
}

fn compute_bundle_sha256(
    bundle_id: &str,
    output: &BundleOutput,
    source_sha256: &str,
    guide: &GuideBodyRecord,
    members: &[BundleMember],
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"guide-watcher-bundle-v3\0");
    for value in [
        bundle_id,
        output.guide.as_str(),
        output.assets_dir.as_str(),
        output.verification_dir.as_str(),
        source_sha256,
        "guide",
        "guide",
        guide.sha256.as_str(),
    ] {
        update_field(&mut digest, value.as_bytes());
    }
    digest.update(guide.size_bytes.to_be_bytes());
    for member in members {
        update_field(
            &mut digest,
            match member.kind {
                BundleMemberKind::File => b"file",
                BundleMemberKind::Directory => b"directory",
            },
        );
        update_field(&mut digest, member.role.as_bytes());
        update_field(&mut digest, member.path.as_bytes());
        digest.update(member.size_bytes.to_be_bytes());
        update_field(&mut digest, member.sha256.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn directory_sha256() -> String {
    sha256_bytes(b"guide-watcher-directory-v1")
}

pub(crate) fn validate_v3_producer_version(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 64 {
        return Err("v3 manifest producer version is invalid".to_string());
    }
    match semver::Version::parse(value) {
        Ok(version) if version.to_string() == value => Ok(()),
        _ => Err("v3 manifest producer version is invalid".to_string()),
    }
}

fn update_field(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u32).to_be_bytes());
    digest.update(bytes);
}

fn ensure_unique_members(members: &[BundleMember]) -> Result<(), String> {
    let mut paths = BTreeMap::new();
    for member in members {
        if paths.insert(&member.path, ()).is_some() {
            return Err("v3 manifest contains duplicate member paths".to_string());
        }
    }
    Ok(())
}

fn safe_relative(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('\\')
        && !value.contains(':')
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn relative_path(path: &Path) -> Result<String, String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => parts.push(
                value
                    .to_str()
                    .ok_or_else(|| "bundle path is not valid Unicode".to_string())?,
            ),
            _ => return Err("bundle contains an unsafe relative path".to_string()),
        }
    }
    if parts.is_empty() {
        return Err("bundle member path is empty".to_string());
    }
    Ok(parts.join("/"))
}

fn validate_uuid(value: &str) -> Result<(), String> {
    match uuid::Uuid::parse_str(value) {
        Ok(parsed) if parsed.to_string() == value => Ok(()),
        _ => Err("bundle id is not a canonical UUID".to_string()),
    }
}

fn validate_sha(value: &str, label: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(format!("{label} is not a lowercase SHA-256 digest"))
    }
}

fn is_lower_hex_bytes(value: &[u8]) -> bool {
    value.len() == 64
        && value
            .iter()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("could not open bundle member for hashing: {error}"))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("could not hash bundle member: {error}"))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("could not create {}: {error}", path.display()))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("could not durably write {}: {error}", path.display()))
}

fn ensure_plain_file(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
    reject_link_or_reparse(path, &metadata)?;
    if !metadata.is_file() {
        return Err(format!("expected a plain file: {}", path.display()));
    }
    Ok(())
}

fn ensure_plain_directory(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
    reject_link_or_reparse(path, &metadata)?;
    if !metadata.is_dir() {
        return Err(format!("expected a plain directory: {}", path.display()));
    }
    Ok(())
}

fn reject_link_or_reparse(path: &Path, metadata: &std::fs::Metadata) -> Result<(), String> {
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "bundle contains a symbolic link: {}",
            path.display()
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(format!(
                "bundle contains a reparse point: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

/// Best effort on Windows: Rust cannot open a directory with `FILE_FLAG_BACKUP_SEMANTICS`,
/// so file contents are flushed but directory-entry persistence cannot be promised there.
fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("could not sync directory {}: {error}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {

    #[test]
    fn file_hashing_is_bounded_on_a_small_stack() {
        crate::artifact_bundle::assert_file_hash_on_small_stack(
            "completion::tests::file_hashing_is_bounded_on_a_small_stack",
            sha256_file,
        );
    }

    use super::*;
    use std::path::PathBuf;
    use uuid::Uuid;

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("guide-watcher-receipt-{}", Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn manual_revision_trailer_makes_a_guide_usable_without_a_receipt() {
        let root = TestRoot::new();
        let revised = root.0.join("L_Guide.md");
        std::fs::write(
            &revised,
            b"# Guide\n\nbody\n\n<!-- guide-watcher: manual-revision 2026-09-12 \xe2\x80\x94 reformatted; content unchanged -->\n",
        )
        .unwrap();
        assert_eq!(inspect_completion(&revised), CompletionStatus::ManualRevision);
        assert!(is_usable_guide(&revised));
        assert!(!has_valid_receipt(&revised));
        assert!(!has_valid_bundle_receipt(&revised));

        // A stale v3 receipt left behind a manual revision does not make the guide invalid.
        let stale = root.0.join("M_Guide.md");
        std::fs::write(
            &stale,
            b"# Guide\n\nbody\n\n<!-- guide-watcher:complete:v3 bundle_id=x guide_body_size=1 guide_body_sha256=0 -->\n<!-- This guide was manually revised after its automated verification seal was issued; -->\n",
        )
        .unwrap();
        assert_eq!(inspect_completion(&stale), CompletionStatus::ManualRevision);

        // Without any marker an unsealed guide stays unsealed and unusable.
        let plain = root.0.join("N_Guide.md");
        std::fs::write(&plain, b"# Guide\n\nbody\n").unwrap();
        assert_eq!(inspect_completion(&plain), CompletionStatus::Unsealed);
        assert!(!is_usable_guide(&plain));
    }

    #[test]
    fn receipt_binds_completion_to_exact_guide_bytes() {
        let root = TestRoot::new();
        let path = root.0.join("legacy.md");
        std::fs::write(&path, b"# Verified guide\n\nContent.\n").unwrap();
        append_receipt(&path).unwrap();
        assert!(has_valid_receipt(&path));
        let mut changed = std::fs::read(&path).unwrap();
        changed[2] = b'X';
        std::fs::write(&path, changed).unwrap();
        assert!(!has_valid_receipt(&path));
    }

    #[test]
    fn completion_status_distinguishes_valid_v2_from_bundle_bound_v3() {
        let root = TestRoot::new();
        let path = root.0.join("legacy.md");
        std::fs::write(&path, b"# Legacy verified guide\n").unwrap();
        append_receipt(&path).unwrap();
        assert_eq!(
            inspect_completion(&path),
            CompletionStatus::LegacyV2GuideOnlyValid
        );
        assert!(!has_valid_bundle_receipt(&path));
    }

    #[test]
    fn v3_receipt_binds_guide_manifest_and_every_member() {
        let root = TestRoot::new();
        let guide = root.0.join("L00 Guide.md");
        let assets = root.0.join("staged-assets");
        let verify = root.0.join("staged-verify");
        std::fs::create_dir(&assets).unwrap();
        std::fs::create_dir(&verify).unwrap();
        std::fs::write(&guide, b"# Guide\n").unwrap();
        std::fs::write(assets.join("figure.png"), b"png").unwrap();
        std::fs::write(
            assets.join("asset_manifest.json"),
            b"{\"schema_version\":2}\n",
        )
        .unwrap();
        std::fs::write(verify.join("coverage_manifest.json"), b"{}\n").unwrap();
        std::fs::write(verify.join("visual_packet.json"), b"{}\n").unwrap();
        std::fs::write(verify.join("visual_contract.json"), b"{}\n").unwrap();
        seal_v3_bundle(
            &guide,
            &assets,
            &verify,
            &root.0.join("L00 Guide.md"),
            &"a".repeat(64),
            &Uuid::new_v4().to_string(),
        )
        .unwrap();
        let final_assets = root.0.join("L00 Guide_assets");
        let final_verify = root.0.join(".L00 Guide.md.gwverify");
        std::fs::rename(&assets, &final_assets).unwrap();
        std::fs::rename(&verify, &final_verify).unwrap();
        assert_eq!(inspect_completion(&guide), CompletionStatus::ValidV3);

        let manifest: BundleManifest = serde_json::from_slice(
            &std::fs::read(final_verify.join(BUNDLE_MANIFEST_NAME)).unwrap(),
        )
        .unwrap();
        let member_paths = manifest
            .members
            .iter()
            .map(|member| member.path.as_str())
            .collect::<Vec<_>>();
        for required in [
            "assets/asset_manifest.json",
            "assets/figure.png",
            "verification/coverage_manifest.json",
            "verification/visual_contract.json",
            "verification/visual_packet.json",
        ] {
            assert!(member_paths.contains(&required), "missing {required}");
        }

        let original_asset = std::fs::read(final_assets.join("figure.png")).unwrap();
        std::fs::write(final_assets.join("figure.png"), b"tampered").unwrap();
        assert!(matches!(
            inspect_completion(&guide),
            CompletionStatus::InvalidV3(_)
        ));
        std::fs::write(final_assets.join("figure.png"), original_asset).unwrap();
        assert_eq!(inspect_completion(&guide), CompletionStatus::ValidV3);

        let evidence = final_verify.join("visual_contract.json");
        let original_evidence = std::fs::read(&evidence).unwrap();
        std::fs::write(&evidence, b"tampered").unwrap();
        assert!(matches!(
            inspect_completion(&guide),
            CompletionStatus::InvalidV3(_)
        ));
        std::fs::write(&evidence, original_evidence).unwrap();

        let manifest = final_verify.join(BUNDLE_MANIFEST_NAME);
        let original_manifest = std::fs::read(&manifest).unwrap();
        std::fs::write(&manifest, b"{}\n").unwrap();
        assert!(matches!(
            inspect_completion(&guide),
            CompletionStatus::InvalidV3(_)
        ));
        std::fs::write(&manifest, original_manifest).unwrap();

        let original_guide = std::fs::read(&guide).unwrap();
        let mut tampered_guide = original_guide.clone();
        tampered_guide[0] ^= 1;
        std::fs::write(&guide, tampered_guide).unwrap();
        assert!(matches!(
            inspect_completion(&guide),
            CompletionStatus::InvalidV3(_)
        ));
        std::fs::write(&guide, original_guide).unwrap();
        assert_eq!(inspect_completion(&guide), CompletionStatus::ValidV3);
    }

    #[test]
    fn extra_bundle_member_invalidates_v3_receipt() {
        let root = TestRoot::new();
        let guide = root.0.join("guide.md");
        let assets = root.0.join("assets-stage");
        let verify = root.0.join("verify-stage");
        std::fs::create_dir(&assets).unwrap();
        std::fs::create_dir(&verify).unwrap();
        std::fs::write(&guide, b"body").unwrap();
        std::fs::write(assets.join("a"), b"a").unwrap();
        std::fs::write(verify.join("v"), b"v").unwrap();
        seal_v3_bundle(
            &guide,
            &assets,
            &verify,
            &guide,
            &"b".repeat(64),
            &Uuid::new_v4().to_string(),
        )
        .unwrap();
        std::fs::rename(&assets, root.0.join("guide_assets")).unwrap();
        std::fs::rename(&verify, root.0.join(".guide.md.gwverify")).unwrap();
        std::fs::write(root.0.join("guide_assets/extra"), b"x").unwrap();
        assert!(matches!(
            inspect_completion(&guide),
            CompletionStatus::InvalidV3(_)
        ));
    }

    #[test]
    fn v3_accepts_prior_bounded_producer_versions_and_rejects_malformed_versions() {
        let root = TestRoot::new();
        let guide = root.0.join("prior.md");
        let assets = root.0.join("assets-stage");
        let verify = root.0.join("verify-stage");
        std::fs::create_dir(&assets).unwrap();
        std::fs::create_dir(&verify).unwrap();
        std::fs::write(&guide, b"prior producer body").unwrap();
        std::fs::write(assets.join("a"), b"a").unwrap();
        std::fs::write(verify.join("v"), b"v").unwrap();
        seal_v3_bundle_with_version(
            &guide,
            &assets,
            &verify,
            &guide,
            &"c".repeat(64),
            &Uuid::new_v4().to_string(),
            "1.7.4-legacy+build5",
        )
        .unwrap();
        std::fs::rename(&assets, root.0.join("prior_assets")).unwrap();
        std::fs::rename(&verify, root.0.join(".prior.md.gwverify")).unwrap();
        assert_eq!(inspect_completion(&guide), CompletionStatus::ValidV3);

        for invalid in [
            "".to_string(),
            "bad version".to_string(),
            ".".to_string(),
            "2.0.0\n".to_string(),
            "x".repeat(65),
        ] {
            assert!(
                validate_v3_producer_version(&invalid).is_err(),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn empty_directories_are_bundle_members_and_later_empty_directories_invalidate_v3() {
        let root = TestRoot::new();
        let guide = root.0.join("directories.md");
        let assets = root.0.join("assets-stage");
        let verify = root.0.join("verify-stage");
        std::fs::create_dir_all(assets.join("expected-empty")).unwrap();
        std::fs::create_dir(&verify).unwrap();
        std::fs::write(&guide, b"directory topology").unwrap();
        std::fs::write(assets.join("a"), b"a").unwrap();
        std::fs::write(verify.join("v"), b"v").unwrap();
        seal_v3_bundle(
            &guide,
            &assets,
            &verify,
            &guide,
            &"d".repeat(64),
            &Uuid::new_v4().to_string(),
        )
        .unwrap();
        let final_assets = root.0.join("directories_assets");
        let final_verify = root.0.join(".directories.md.gwverify");
        std::fs::rename(&assets, &final_assets).unwrap();
        std::fs::rename(&verify, &final_verify).unwrap();
        assert_eq!(inspect_completion(&guide), CompletionStatus::ValidV3);

        let extra_asset_dir = final_assets.join("foreign-empty");
        std::fs::create_dir(&extra_asset_dir).unwrap();
        assert!(matches!(
            inspect_completion(&guide),
            CompletionStatus::InvalidV3(_)
        ));
        std::fs::remove_dir(&extra_asset_dir).unwrap();
        assert_eq!(inspect_completion(&guide), CompletionStatus::ValidV3);

        std::fs::create_dir(final_verify.join("foreign-empty")).unwrap();
        assert!(matches!(
            inspect_completion(&guide),
            CompletionStatus::InvalidV3(_)
        ));
    }

    #[test]
    fn all_completion_marker_versions_are_reserved() {
        for version in ["v1", "v2:sha256=x", "v3 bundle_id=x", "v99"] {
            assert!(contains_reserved_marker(&format!(
                "x <!-- guide-watcher:complete:{version} -->"
            )));
        }
    }

    #[test]
    fn v1_is_unverified_and_v2_remains_read_only_compatible() {
        let root = TestRoot::new();
        let v1 = root.0.join("v1.md");
        std::fs::write(&v1, b"body\n<!-- guide-watcher:complete:v1 -->\n").unwrap();
        assert_eq!(
            inspect_completion(&v1),
            CompletionStatus::LegacyV1Unverified
        );
        assert!(!has_valid_receipt(&v1));

        let v2 = root.0.join("v2.md");
        std::fs::write(&v2, b"body").unwrap();
        append_receipt(&v2).unwrap();
        assert_eq!(
            inspect_completion(&v2),
            CompletionStatus::LegacyV2GuideOnlyValid
        );
        assert!(has_valid_receipt(&v2));
        assert!(!has_valid_bundle_receipt(&v2));
    }
}
