"""
guide_lint.py — Native verification gate for guide-watcher study guides.
================================================================================
This is the CODE half of the verification phase. The model is *instructed* to
self-review (template Phase 2), but prompts are not reliable, so this script
independently enforces the quality bar with a non-zero exit code that the
launcher's gate loop drives to 0.

It catches the recurring symptoms seen in generated guides and enforces the
evidence contract needed for cumulative, visual-first course guides:
  1. RAMBLING / visible self-correction left in the final text
     ("wait, let me re-extract", "hmm", "honestly the layout is messy",
      "let me give you a clean version", "I can't read the PDF", ...).
  2. HAND-WAVING placeholders (TODO/FIXME/XXX, "<placeholder>", "[fill in]").
  3. Structural breakage (unbalanced ``` code fences).
  4. MISSING SOURCE COVERAGE — every source unit must appear exactly once in a
     machine-readable coverage manifest and point to a real guide section.
  5. BROKEN VISUAL EVIDENCE — learner-facing images must exist in the guide's
     asset folder and carry alt text, captions, explanations, and provenance.
  6. SOURCE MUTATION — the source hash captured before generation must still
     match the live source and the coverage manifest.
  7. INCORRECT BASIC COMPUTATION — supported arithmetic equalities are
     recomputed by this fixed app-owned verifier. Model-authored code is never
     executed.

Usage:
    python guide_lint.py <guide.md> [--verify-dir DIR] [--source SOURCE]
                         [--seal]
    python guide_lint.py --snapshot-source SOURCE --verify-dir DIR

The verification directory, containing app-owned manifests and rendered slides,
defaults to a sibling folder of the guide: <guide_dir>/.<guide_name>.gwverify/.
Any verify.py file is rejected and never executed.

Exit codes:  0 = clean (warnings allowed),  1 = one or more hard failures.
"""

import os
import re
import sys
import argparse
import hashlib
import json
import tempfile
import urllib.parse
import zipfile
import zlib
from decimal import Decimal, InvalidOperation
from pathlib import Path

try:
    import markdown_it
    from markdown_it import MarkdownIt
except ImportError:
    markdown_it = None
    MarkdownIt = None


VERIFIER_REQUIREMENTS_PATH = Path(__file__).with_name("requirements-verifier.txt")


def _expected_markdown_it_version(path=None):
    lock_path = Path(path or VERIFIER_REQUIREMENTS_PATH)
    try:
        lock_bytes = lock_path.read_bytes()
    except OSError as exc:
        raise RuntimeError(f"could not read verifier dependency lock: {exc}") from exc
    match = re.fullmatch(rb"markdown-it-py==([0-9]+\.[0-9]+\.[0-9]+)\n", lock_bytes)
    if match is None:
        raise RuntimeError(
            "verifier dependency lock must be exactly one markdown-it-py==X.Y.Z line ending in LF"
        )
    return match.group(1).decode("ascii")


# ── Rambling / visible-thinking phrases (PROSE only) ──────────────────────────
# Curated so legitimate teaching voice is NOT flagged:
#   "let me walk you through", "let me show you", "let's trace", "recall",
#   "notice", "imagine" all pass. Only self-correction / thinking-out-loud hits.
RAMBLING_PATTERNS = [
    r"\blet me re-?(extract|read|examine|check|do|try|trace|draw|design|derive|construct|reconstruct|consider|count|calculate|compute|verify|work\s+through\s+again)\b",
    r"\blet me just\b",
    r"\blet me lock\s+in\b",
    r"\blet me give you a clean(er)?\b",
    r"\blet me redo\b",
    r"\blet me fix\b",
    r"\blet me assume\b",
    r"\bwait,?\s+let me\b",
    r"\bactually,?\s+wait\b",
    r"\bwait\s*[—–-]\s",          # "Wait —" style interjection
    r"(?im)^\s*wait[.,]",          # line beginning "Wait." / "Wait,"
    r"\bhold on\b(?!\s+to\b)",
    r"\bhmm+\b",
    r"\boops\b",
    r"\bmy mistake\b",
    r"\bnever\s?mind\b",
    r"\bscratch that\b",
    r"\bon second thought\b",
    r"\bcome to think of it\b",
    r"\byou know what\b",
    r"\bhonestly,",
    r"\bto be honest\b",
    r"\bis a (bit\s+)?mess\b",
    r"\b(?:the|this)\s+(?:pdf|slide|figure|diagram|layout|extraction)\s+is\s+(?:a\s+bit\s+|too\s+)?messy\b",
    r"\bhard to (capture|draw|render|represent)\b",
    r"\btricky to (draw|capture|render)\b",
    r"\bclean(er)?\s+(version|illustrative)\b",
    r"\bclean illustrative\b",
    r"\b(?:I|we)\s+(?:couldn'?t|could not|cannot|can'?t|am unable to|are unable to)\s+(?:read|extract|render|make out|parse|tell|see)\b[^.!?\n]{0,100}\b(?:pdf|slide|figure|diagram|arrow|labels?|source image)\b",
    r"\b(?:couldn'?t|could not|cannot|can'?t|unable to)\s+(?:read|extract|render|make out|parse|see)\s+(?:the|this)\s+(?:pdf|slide|figure|diagram|source image)\b",
    r"\bthe pdf (doesn'?t|does not|won'?t|isn'?t)\b",
    r"\bfrom the pdf carefully\b",
    r"\bre-?extract\b",
]

# ── Hand-waving placeholders (scanned everywhere) ─────────────────────────────
PLACEHOLDER_PATTERNS = [
    r"\bTODO\b",
    r"\bFIXME\b",
    r"\bXXX\b",
    r"<placeholder>",
    r"\[fill in[^\]]*\]",
    r"\[insert[^\]]*\]",
]

# Arithmetic equality like "2 + 3 = 5" / "8 - 2 = 6" / "5 × 4 = 20"
# A whole left-hand chain (`a op b op c ...`), the stated result, and an optional
# percent sign. Matching the whole chain is what stops `0 + 1 + 2 = 3` from being read
# as the false statement `1 + 2 = 3`... or rather `4 + 5 = 15` on a longer sum.
ARITH_EQ = re.compile(
    r"(?<![\w.])"
    r"([-−]?\d+(?:\.\d+)?(?:(?:\s*[+\-−*×/]\s*|\s+x\s+)[-−]?\d+(?:\.\d+)?)+)"
    r"\s*=\s*([-−]?\d+(?:\.\d+)?)\s*(\\?%)?(?![\w.])"
)
_ARITH_TOKEN = re.compile(r"\d+(?:\.\d+)?|[+\-−*×/]|(?<=\s)x(?=\s)")


def _evaluate_arithmetic_chain(expression):
    """Evaluate `a op b op c ...` with × / before + -, left to right, in Decimal.

    Tokens alternate operand / operator; a `-` in operand position is a unary minus, so
    `36-28` is a subtraction while `5 + -3` and `-4 + 6` keep their negative operands.
    Returns None on a malformed chain or division by zero so the caller can report it.
    """
    values, operators, pending_ops = [], [], []
    expect_operand, negate = True, False
    for token in _ARITH_TOKEN.findall(expression):
        if expect_operand:
            if token in ("-", "−"):
                negate = not negate
                continue
            if not token[0].isdigit():
                return None
            operand = -Decimal(token) if negate else Decimal(token)
            negate = False
            if pending_ops and pending_ops[-1] in {"*", "×", "x", "/"}:
                operator = pending_ops.pop()
                left = values.pop()
                if operator == "/":
                    if operand == 0:
                        return None
                    values.append(left / operand)
                else:
                    values.append(left * operand)
            else:
                if pending_ops:
                    operators.append(pending_ops.pop())
                values.append(operand)
            expect_operand = False
        else:
            if token[0].isdigit():
                return None
            pending_ops.append(token)
            expect_operand = True
    if expect_operand or pending_ops or not values:
        return None
    total = values[0]
    for operator, value in zip(operators, values[1:]):
        total = total + value if operator == "+" else total - value  # "-" or U+2212
    return total
# Threshold above which a guide receives the dense computational depth floor.
COMPUTATIONAL_THRESHOLD = 6
DEPTH_WORDS_PER_SLIDE = 240
DEPTH_MIN_SLIDES = 20
DENSE_DEPTH_COURSE_PROFILES = {
    "computer-networks",
    "computer-algorithms",
    "operating-systems",
}
GUIDE_COMPLETE_SENTINEL = "<!-- guide-watcher:complete:v1 -->"
SOURCE_SNAPSHOT_NAME = "source_snapshot.json"
COVERAGE_MANIFEST_NAME = "coverage_manifest.json"
ASSET_MANIFEST_NAME = "asset_manifest.json"
PROVENANCE_TYPES = {
    "source-crop",
    "annotated-source",
    "book-source",
    "web-source",
    "video-frame",
    "deterministic-diagram",
    "generated-educational",
}


def split_prose_and_code(text):
    """Return (prose_lines, in_code_flags) where in_code_flags[i] is True if
    line i sits inside a ``` fenced block. Lines are 1-indexed via enumerate."""
    in_code = False
    flags = []
    for line in text.splitlines():
        is_fence = line.lstrip().startswith("```")
        if is_fence:
            # The fence line itself belongs to neither prose nor inner-code scan.
            flags.append(None)
            in_code = not in_code
        else:
            flags.append(in_code)
    return flags


def find_matches(text, patterns, prose_only=False):
    """Yield (line_no, matched_text, line_text) for each pattern hit.

    Matching is case-insensitive: guides capitalize sentence starts, so
    "Hmm", "Wait, let me", "Honestly," and "Let me re-extract" must be caught
    just like their lowercase forms."""
    lines = text.splitlines()
    code_flags = split_prose_and_code(text)
    hits = []
    for i, line in enumerate(lines):
        in_code = code_flags[i]
        if in_code is None:           # the ``` fence line itself
            continue
        if prose_only and in_code:    # skip code when scanning prose-only checks
            continue
        for pat in patterns:
            m = re.search(pat, line, re.IGNORECASE)
            if m:
                hits.append((i + 1, m.group(0).strip(), line.strip()))
    return hits


def check_fences(text):
    """Return list of failure strings if ``` fences are unbalanced."""
    count = sum(1 for ln in text.splitlines() if ln.lstrip().startswith("```"))
    if count % 2 != 0:
        return [f"Unbalanced code fences: found {count} ``` lines (must be even)."]
    return []


def check_ellipsis_in_code(text):
    """WARN on '...' / '…' inside fenced code blocks (often an incomplete diagram,
    but legitimate in math sequences like v0, v1, ..., vk — so warn, don't fail)."""
    lines = text.splitlines()
    code_flags = split_prose_and_code(text)
    warns = []
    for i, line in enumerate(lines):
        if code_flags[i] is True and ("..." in line or "…" in line):
            warns.append((i + 1, line.strip()))
    return warns


def check_arithmetic_equalities(text):
    """Recompute bounded scalar arithmetic without running model-authored code."""
    failures = []
    for line_number, line in enumerate(text.splitlines(), start=1):
        for match in ARITH_EQ.finditer(line):
            expression, stated_text, percent = match.groups()
            try:
                stated = Decimal(stated_text.replace("−", "-"))
                actual = _evaluate_arithmetic_chain(expression)
            except (InvalidOperation, ZeroDivisionError):
                actual = None
            if actual is None:
                failures.append(
                    f"line {line_number}: could not recompute {match.group(0)!r}"
                )
                continue
            if percent:
                # `58/158 = 36.7%` states a percentage of the computed ratio.
                actual = actual * 100
            decimal_places = max(0, -stated.as_tuple().exponent)
            tolerance = Decimal(5).scaleb(-(decimal_places + 1)) if decimal_places else Decimal(0)
            if abs(actual - stated) > tolerance:
                failures.append(
                    f"line {line_number}: {match.group(0)!r} evaluates to {actual}"
                )
    return failures


def _count_rendered_slides_in_tree(root):
    try:
        return sum(
            1
            for _, _, names in os.walk(root)
            for name in names
            if re.fullmatch(r"slide_\d+\.png", name)
        )
    except OSError:
        return 0


def count_rendered_slides(verify_dir):
    """Count source-owned renders, falling back to legacy recursive layouts.

    A current verification root owns its source list through source_snapshot.json.
    Supplementary textbook candidates and asset thumbnails elsewhere under that
    root are not lecture/support source units and must not raise the depth floor.
    Calls on an individual source renders directory, and legacy roots without a
    snapshot, retain the historical recursive behavior.
    """
    snapshot_path = os.path.join(verify_dir, SOURCE_SNAPSHOT_NAME)
    if not os.path.isfile(snapshot_path):
        return _count_rendered_slides_in_tree(verify_dir)

    snapshot, snapshot_error = _load_json(snapshot_path)
    sources = snapshot.get("sources") if isinstance(snapshot, dict) else None
    if snapshot_error or not isinstance(sources, list) or not sources:
        return 0

    source_ids = []
    for source in sources:
        source_id = source.get("id") if isinstance(source, dict) else None
        if (
            not isinstance(source_id, str)
            or not re.fullmatch(r"source-[0-9a-f]{64}", source_id)
            or source_id in source_ids
        ):
            return 0
        source_ids.append(source_id)
    return sum(
        _count_rendered_slides_in_tree(
            os.path.join(verify_dir, "sources", source_id, "renders")
        )
        for source_id in source_ids
    )


def count_words(text):
    return len(re.findall(r"\b[\w']+\b", text))


def count_major_sections(text):
    return sum(1 for line in text.splitlines() if re.match(r"^#\s+\d+\.", line))


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def png_dimensions(path):
    """Fully validate the app's bounded, non-interlaced RGBA8 PNG profile."""
    compressed_limit = 25 * 1024 * 1024
    decoded_limit = 64 * 1024 * 1024
    if Path(path).stat().st_size > compressed_limit:
        raise ValueError("PNG exceeds compressed-size limit")
    data = Path(path).read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("invalid PNG signature")
    cursor = 8
    chunks = []
    saw_iend = False
    saw_idat = False
    ended_idat = False
    while cursor < len(data):
        if cursor + 12 > len(data):
            raise ValueError("truncated PNG chunk")
        length = int.from_bytes(data[cursor:cursor + 4], "big")
        kind = data[cursor + 4:cursor + 8]
        end = cursor + 12 + length
        if end > len(data):
            raise ValueError("truncated PNG chunk payload")
        payload = data[cursor + 8:cursor + 8 + length]
        if not re.fullmatch(rb"[A-Za-z]{4}", kind):
            raise ValueError("PNG chunk type is invalid")
        expected_crc = int.from_bytes(data[cursor + 8 + length:end], "big")
        if zlib.crc32(kind + payload) & 0xFFFFFFFF != expected_crc:
            raise ValueError(f"PNG {kind!r} CRC mismatch")
        chunks.append((kind, payload))
        if kind == b"IDAT":
            if ended_idat:
                raise ValueError("PNG IDAT chunks must be contiguous")
            saw_idat = True
        elif saw_idat and kind != b"IEND":
            ended_idat = True
        cursor = end
        if kind == b"IEND":
            saw_iend = True
            break
    if not saw_iend or cursor != len(data):
        raise ValueError("PNG is missing IEND or contains trailing bytes")
    ihdr = [payload for kind, payload in chunks if kind == b"IHDR"]
    iend = [payload for kind, payload in chunks if kind == b"IEND"]
    if (
        len(ihdr) != 1
        or chunks[0][0] != b"IHDR"
        or len(ihdr[0]) != 13
        or len(iend) != 1
        or iend[0]
        or not saw_idat
        or any(kind not in {b"IHDR", b"IDAT", b"IEND"} for kind, _ in chunks)
    ):
        raise ValueError("PNG must contain one leading IHDR and one IEND")
    header = ihdr[0]
    width = int.from_bytes(header[0:4], "big")
    height = int.from_bytes(header[4:8], "big")
    bit_depth, color_type, compression, filter_method, interlace = header[8:13]
    if not (1 <= width <= 8192 and 1 <= height <= 8192):
        raise ValueError(f"invalid PNG dimensions {width}x{height}")
    if (bit_depth, color_type, compression, filter_method, interlace) != (8, 6, 0, 0, 0):
        raise ValueError("learner PNG must be RGBA8, non-interlaced, standard compression/filter")
    compressed = b"".join(payload for kind, payload in chunks if kind == b"IDAT")
    if not compressed:
        raise ValueError("PNG has no IDAT data")
    expected_size = height * (1 + width * 4)
    if expected_size > decoded_limit:
        raise ValueError("decoded PNG exceeds 64 MiB")
    decoder = zlib.decompressobj()
    raw = decoder.decompress(compressed, expected_size + 1)
    if len(raw) != expected_size or not decoder.eof or decoder.unused_data or decoder.unconsumed_tail:
        raise ValueError("PNG zlib stream is incomplete, oversized, concatenated, or has trailing data")
    stride = width * 4
    prior = bytearray(stride)
    for row in range(height):
        offset = row * (stride + 1)
        filter_type = raw[offset]
        if filter_type > 4:
            raise ValueError(f"PNG scanline {row + 1} uses invalid filter {filter_type}")
        encoded = raw[offset + 1:offset + 1 + stride]
        decoded = bytearray(stride)
        for index, value in enumerate(encoded):
            left = decoded[index - 4] if index >= 4 else 0
            above = prior[index]
            upper_left = prior[index - 4] if index >= 4 else 0
            if filter_type == 0:
                predictor = 0
            elif filter_type == 1:
                predictor = left
            elif filter_type == 2:
                predictor = above
            elif filter_type == 3:
                predictor = (left + above) // 2
            else:
                p = left + above - upper_left
                pa, pb, pc = abs(p - left), abs(p - above), abs(p - upper_left)
                predictor = left if pa <= pb and pa <= pc else above if pb <= pc else upper_left
            decoded[index] = (value + predictor) & 0xFF
        prior = decoded
    return width, height


def _atomic_write_text(path, text):
    """Replace a text file atomically without leaving a partial marker/manifest."""
    path = os.path.abspath(path)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    fd, temp_path = tempfile.mkstemp(
        prefix=f".{os.path.basename(path)}.", suffix=".tmp", dir=os.path.dirname(path)
    )
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(text)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temp_path, path)
    except Exception:
        try:
            os.unlink(temp_path)
        except OSError:
            pass
        raise


def _unit_ids(kind, count):
    width = max(3, len(str(max(1, count))))
    return [f"{kind}-{index:0{width}d}" for index in range(1, count + 1)]


def _source_id(source_path):
    identity = os.path.abspath(source_path).replace("\\", "/")
    if os.name == "nt":
        identity = identity.lower()
    return "source-" + hashlib.sha256(identity.encode("utf-8")).hexdigest()


def infer_source_units(source_path):
    """Return (kind, count) using deterministic, file-local structure only."""
    suffix = Path(source_path).suffix.lower()
    if suffix == ".pdf":
        try:
            import fitz  # PyMuPDF is already required by render_slides.py

            with fitz.open(source_path) as document:
                return "slide", document.page_count
        except Exception as exc:  # noqa: BLE001 - report actionable source failure
            raise ValueError(f"could not count PDF pages: {exc}") from exc
    if suffix == ".pptx":
        try:
            with zipfile.ZipFile(source_path) as archive:
                count = sum(
                    1
                    for name in archive.namelist()
                    if re.fullmatch(r"ppt/slides/slide\d+\.xml", name)
                )
        except (OSError, zipfile.BadZipFile) as exc:
            raise ValueError(f"could not count PPTX slides: {exc}") from exc
        if count < 1:
            raise ValueError("PPTX contains no slide XML files")
        return "slide", count
    if suffix in {".html", ".htm"}:
        try:
            html = Path(source_path).read_text(encoding="utf-8", errors="replace")
        except OSError as exc:
            raise ValueError(f"could not read HTML: {exc}") from exc
        templates = [item for item in re.findall(r"`(.*?)`", html, re.DOTALL) if len(item.strip()) >= 30]
        if templates:
            return "section", len(templates)
        sections = re.findall(r"<(?:section|article)\b", html, re.IGNORECASE)
        return "section", max(1, len(sections))
    return "document", 1


def record_source_snapshot(source_path, verify_dir):
    source_path = os.path.abspath(source_path)
    if not os.path.isfile(source_path):
        raise ValueError(f"source file not found: {source_path}")
    unit_kind, unit_count = infer_source_units(source_path)
    if unit_count < 1:
        raise ValueError("source contains no countable units")
    source_id = _source_id(source_path)
    unit_ids = [f"{source_id}-unit-{index:03d}" for index in range(1, unit_count + 1)]
    snapshot = {
        "schema_version": 2,
        "primary_source_id": source_id,
        "sources": [{
            "id": source_id,
            "path": source_path,
            "name": os.path.basename(source_path),
            "sha256": sha256_file(source_path),
            "size_bytes": os.path.getsize(source_path),
            "role": "primary",
            "unit_kind": unit_kind,
            "unit_count": unit_count,
            "unit_ids": unit_ids,
        }],
        "unit_ids": unit_ids,
    }
    snapshot_path = os.path.join(os.path.abspath(verify_dir), SOURCE_SNAPSHOT_NAME)
    _atomic_write_text(snapshot_path, json.dumps(snapshot, indent=2, ensure_ascii=False) + "\n")
    return snapshot_path, snapshot


def _load_json(path):
    try:
        with open(path, encoding="utf-8") as handle:
            return json.load(handle), None
    except FileNotFoundError:
        return None, f"missing file: {path}"
    except (OSError, json.JSONDecodeError) as exc:
        return None, f"could not read valid JSON from {path}: {exc}"


def _heading_anchors(text):
    anchors = set()
    duplicates = {}
    for line in text.splitlines():
        match = re.match(r"^#{1,6}\s+(.+?)\s*#*\s*$", line)
        if not match:
            continue
        heading = re.sub(r"<[^>]+>", "", match.group(1)).strip().lower()
        slug = re.sub(r"[^\w\- ]", "", heading, flags=re.UNICODE)
        slug = re.sub(r"[\s-]+", "-", slug).strip("-")
        index = duplicates.get(slug, 0)
        duplicates[slug] = index + 1
        anchors.add(slug if index == 0 else f"{slug}-{index}")
    return anchors


def check_source_and_coverage(
    text,
    guide_path,
    verify_dir,
    source_path,
    trusted_source_sha=None,
    expected_guide_name=None,
    expected_guide_kind=None,
    expected_course_profile=None,
):
    failures = []
    snapshot_path = os.path.join(verify_dir, SOURCE_SNAPSHOT_NAME)
    snapshot, snapshot_error = _load_json(snapshot_path)
    if snapshot_error:
        return [("MISSING SOURCE SNAPSHOT", [snapshot_error])], None

    required_snapshot_fields = {"schema_version", "primary_source_id", "sources", "unit_ids"}
    if not isinstance(snapshot, dict) or not required_snapshot_fields.issubset(snapshot):
        return [("INVALID SOURCE SNAPSHOT", [f"required fields missing in {snapshot_path}"])], None
    expected_ids = snapshot.get("unit_ids")
    valid_expected_ids = (
        isinstance(expected_ids, list)
        and len(expected_ids) == len(set(expected_ids))
        and all(isinstance(item, str) and item for item in expected_ids)
    )
    snapshot_sources = snapshot.get("sources")
    source_errors = []
    if snapshot.get("schema_version") != 2 or not valid_expected_ids:
        source_errors.append("schema_version must be 2 and unit_ids must be unique strings.")
    if not isinstance(snapshot_sources, list) or not snapshot_sources:
        source_errors.append("sources must be a non-empty list.")
        snapshot_sources = []
    source_by_id = {}
    unit_owner = {}
    for index, source in enumerate(snapshot_sources):
        if not isinstance(source, dict):
            source_errors.append(f"source {index + 1}: must be an object")
            continue
        source_id = source.get("id")
        if not isinstance(source_id, str) or not re.fullmatch(r"source-[0-9a-f]{64}", source_id):
            source_errors.append(f"source {index + 1}: invalid stable source id")
            continue
        if source_id in source_by_id:
            source_errors.append(f"duplicate source id: {source_id}")
        source_by_id[source_id] = source
        source_units = source.get("unit_ids")
        count = source.get("unit_count")
        if (
            not isinstance(source_units, list)
            or not isinstance(count, int)
            or count < 1
            or len(source_units) != count
            or source_units != [f"{source_id}-unit-{number:03d}" for number in range(1, count + 1)]
        ):
            source_errors.append(f"{source_id}: unit count/IDs are inconsistent")
            source_units = []
        for unit_id in source_units:
            if unit_id in unit_owner:
                source_errors.append(f"duplicate source-unit id: {unit_id}")
            unit_owner[unit_id] = source_id
        if source.get("role") not in {"primary", "support"}:
            source_errors.append(f"{source_id}: role must be primary or support")
        for field in ("path", "name", "unit_kind"):
            if not isinstance(source.get(field), str) or not source.get(field, "").strip():
                source_errors.append(f"{source_id}: missing {field}")
        digest = source.get("sha256")
        if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
            source_errors.append(f"{source_id}: invalid SHA-256")
        path = source.get("path")
        if isinstance(path, str):
            live_path = os.path.abspath(path)
            if not os.path.isfile(live_path):
                source_errors.append(f"{source_id}: source file is missing: {live_path}")
            else:
                if sha256_file(live_path) != digest:
                    source_errors.append(f"{source_id}: live source SHA-256 changed")
                if os.path.getsize(live_path) != source.get("size_bytes"):
                    source_errors.append(f"{source_id}: live source size changed")
                if os.path.basename(live_path) != source.get("name"):
                    source_errors.append(f"{source_id}: source filename changed")
        rendered_dir = os.path.join(verify_dir, "sources", str(source_id), "renders")
        rendered = count_rendered_slides(rendered_dir)
        if source.get("unit_kind") in {"pdf-page", "pptx-slide", "slide"} and rendered != count:
            source_errors.append(
                f"{source_id}: snapshot records {count} units but {rendered} rendered PNGs exist"
            )
    if set(expected_ids or []) != set(unit_owner):
        source_errors.append("top-level unit_ids do not exactly match all per-source unit IDs")
    primary_id = snapshot.get("primary_source_id")
    primary_source = source_by_id.get(primary_id)
    if not primary_source or primary_source.get("role") != "primary":
        source_errors.append("primary_source_id must identify the primary source")
        primary_source = {}
    if sum(1 for source in snapshot_sources if isinstance(source, dict) and source.get("role") == "primary") != 1:
        source_errors.append("exactly one snapshot source must have role primary")
    if source_errors:
        failures.append(("INVALID SOURCE SNAPSHOT", source_errors[:40]))
        live_integrity_errors = [error for error in source_errors if "live source" in error]
        if live_integrity_errors:
            failures.append(("SOURCE INTEGRITY", live_integrity_errors[:40]))
    expected_hash = primary_source.get("sha256")
    source_kinds = {
        source.get("unit_kind")
        for source in snapshot_sources
        if isinstance(source, dict) and isinstance(source.get("unit_kind"), str)
    }

    if trusted_source_sha is not None:
        if not re.fullmatch(r"[0-9a-f]{64}", trusted_source_sha):
            failures.append(("SOURCE INTEGRITY", ["trusted source SHA-256 argument is malformed."]))
        elif expected_hash != trusted_source_sha:
            failures.append((
                "SOURCE INTEGRITY",
                ["snapshot SHA-256 does not match the trusted pre-generation SHA-256 held by the launcher."],
            ))

    if source_path:
        source_path = os.path.abspath(source_path)
        integrity = []
        snapshot_primary_path = os.path.abspath(primary_source.get("path", ""))
        try:
            same_primary = os.path.samefile(source_path, snapshot_primary_path)
        except OSError:
            same_primary = os.path.normcase(source_path) == os.path.normcase(snapshot_primary_path)
        if not same_primary:
            integrity.append("launcher primary source path does not match primary_source_id")
        if integrity:
            failures.append(("SOURCE INTEGRITY", integrity))

    manifest_path = os.path.join(verify_dir, COVERAGE_MANIFEST_NAME)
    manifest, manifest_error = _load_json(manifest_path)
    if manifest_error:
        failures.append(("MISSING COVERAGE MANIFEST", [manifest_error]))
        return failures, None
    if not isinstance(manifest, dict) or manifest.get("schema_version") != 2:
        failures.append(("INVALID COVERAGE MANIFEST", ["schema_version must be 2."]))
        return failures, manifest

    expected_guide_name = expected_guide_name or os.path.basename(guide_path)
    if manifest.get("guide") != expected_guide_name:
        failures.append(("COVERAGE GUIDE MISMATCH", ["manifest guide field does not name this guide."]))
    guide_kind = manifest.get("guide_kind")
    if guide_kind not in {"lecture", "course-week", "circuit-lab", "scientific-writing-week"}:
        failures.append((
            "GUIDE KIND",
            ["guide_kind must be lecture, course-week, circuit-lab, or scientific-writing-week."],
        ))
    elif expected_guide_kind is not None and guide_kind != expected_guide_kind:
        failures.append((
            "GUIDE KIND MISMATCH",
            [f"manifest guide_kind {guide_kind!r} does not match planned {expected_guide_kind!r}."],
        ))
    sources = manifest.get("sources")
    source_errors = []
    if not isinstance(sources, list):
        source_errors.append("sources must be a source list.")
        sources = []
    expected_source_records = [
        {field: source.get(field) for field in ("id", "path", "sha256", "role")}
        for source in snapshot_sources
        if isinstance(source, dict)
    ]
    actual_source_records = [
        {field: source.get(field) for field in ("id", "path", "sha256", "role")}
        for source in sources
        if isinstance(source, dict)
    ]
    if len(actual_source_records) != len(sources) or actual_source_records != expected_source_records:
        source_errors.append("coverage sources must exactly match the app-owned snapshot in order")
        if any(
            actual.get("sha256") != expected.get("sha256")
            for actual, expected in zip(actual_source_records, expected_source_records)
        ):
            failures.append(("SOURCE HASH MISMATCH", ["coverage source digest does not match the app-owned snapshot."]))
    source_ids = [record.get("id") for record in actual_source_records if isinstance(record.get("id"), str)]
    if manifest.get("primary_source_id") != primary_id:
        source_errors.append("primary_source_id does not match the app-owned snapshot")
    if source_errors:
        failures.append(("SOURCE REGISTER", source_errors[:40]))

    units = manifest.get("units")
    if not isinstance(units, list):
        failures.append(("INVALID COVERAGE MANIFEST", ["units must be a list."]))
        units = []
    ids = [
        unit.get("id")
        for unit in units
        if isinstance(unit, dict) and isinstance(unit.get("id"), str)
    ]
    duplicates_found = sorted({item for item in ids if ids.count(item) > 1})
    if duplicates_found:
        failures.append(("DUPLICATE SOURCE-UNIT COVERAGE", [", ".join(duplicates_found)]))
    safe_expected_ids = expected_ids if valid_expected_ids else []
    missing = sorted(set(safe_expected_ids) - set(ids))
    extra = sorted(set(ids) - set(safe_expected_ids))
    if missing:
        failures.append(("MISSING SOURCE-UNIT COVERAGE", [", ".join(missing)]))
    if extra:
        failures.append(("UNKNOWN SOURCE-UNIT COVERAGE", [", ".join(extra)]))

    anchors = _heading_anchors(text)
    bad_units = []
    for unit in units:
        if not isinstance(unit, dict):
            bad_units.append("non-object unit entry")
            continue
        unit_id = unit.get("id", "<missing id>")
        if not isinstance(unit_id, str) or unit.get("source_id") != unit_owner.get(unit_id):
            bad_units.append(f"{unit_id}: source_id does not match the source snapshot")
        anchor = unit.get("guide_anchor")
        topic = unit.get("topic")
        if not isinstance(anchor, str) or anchor not in anchors:
            bad_units.append(f"{unit_id}: missing/unknown guide_anchor {anchor!r}")
        if not isinstance(topic, str) or not topic.strip():
            bad_units.append(f"{unit_id}: topic is empty")
    if bad_units:
        failures.append(("INVALID SOURCE-UNIT MAPPING", bad_units[:40]))

    continuity = manifest.get("continuity")
    continuity_errors = []
    if not isinstance(continuity, dict):
        continuity_errors.append("continuity must be an object.")
    else:
        if not isinstance(continuity.get("prerequisites"), list):
            continuity_errors.append("continuity.prerequisites must be a list.")
        prior_guides = continuity.get("prior_guides")
        predecessor_path = os.path.join(verify_dir, "predecessor_manifest.json")
        predecessor_manifest, predecessor_error = _load_json(predecessor_path)
        if predecessor_error:
            continuity_errors.append(predecessor_error)
            expected_prior = []
        elif not isinstance(predecessor_manifest, dict) or predecessor_manifest.get("schema_version") != 1:
            continuity_errors.append("predecessor manifest schema_version must be 1")
            expected_prior = []
        else:
            expected_prior = predecessor_manifest.get("prior_guides")
            if not isinstance(expected_prior, list):
                continuity_errors.append("predecessor manifest prior_guides must be a list")
                expected_prior = []
            else:
                for item in expected_prior:
                    if not isinstance(item, dict):
                        continue
                    path = item.get("path")
                    digest = item.get("sha256")
                    if not isinstance(path, str) or not os.path.isfile(path):
                        continuity_errors.append(f"bound predecessor is missing: {path!r}")
                    elif not isinstance(digest, str) or sha256_file(path) != digest:
                        continuity_errors.append(f"bound predecessor digest changed: {path}")
        if not isinstance(prior_guides, list):
            continuity_errors.append("continuity.prior_guides must be a list.")
        else:
            actual_prior = []
            for item in prior_guides:
                if not isinstance(item, dict):
                    continuity_errors.append("continuity prior-guide entry must be an object")
                    continue
                actual_prior.append({field: item.get(field) for field in ("identity", "path", "sha256")})
                if not isinstance(item.get("bridge"), str) or not item.get("bridge", "").strip():
                    continuity_errors.append(f"{item.get('identity')!r}: bridge must be nonempty")
                evidence = item.get("evidence")
                if not isinstance(evidence, list) or not evidence or not all(isinstance(value, str) and value.strip() for value in evidence):
                    continuity_errors.append(f"{item.get('identity')!r}: evidence must contain nonempty strings")
            expected_prior_contract = [
                {
                    "identity": item.get("generation_identity"),
                    "path": item.get("path"),
                    "sha256": item.get("sha256"),
                }
                for item in expected_prior
                if isinstance(item, dict)
            ]
            if len(expected_prior_contract) != len(expected_prior) or actual_prior != expected_prior_contract:
                continuity_errors.append("continuity.prior_guides must exactly match the app-owned predecessor manifest in order")
        if not isinstance(continuity.get("next_bridge"), str) or not continuity.get("next_bridge", "").strip():
            continuity_errors.append("continuity.next_bridge must explain the forward connection.")
    if continuity_errors:
        failures.append(("CUMULATIVE CONTINUITY", continuity_errors))

    research_sources = manifest.get("research_sources", [])
    research_errors = []
    research_ids = []
    if not isinstance(research_sources, list):
        research_errors.append("research_sources must be a list when present")
        research_sources = []
    for source in research_sources:
        if not isinstance(source, dict):
            research_errors.append("research source entry must be an object")
            continue
        source_id = source.get("id")
        if not isinstance(source_id, str) or not source_id.strip():
            research_errors.append("research source has no stable id")
        else:
            research_ids.append(source_id)
        if source.get("tier") not in {1, 2, 3, 4}:
            research_errors.append(f"{source_id!r}: tier must be 1, 2, 3, or 4")
        for field in ("kind", "title", "reference", "locator", "accessed"):
            if not isinstance(source.get(field), str) or not source.get(field, "").strip():
                research_errors.append(f"{source_id!r}: missing {field}")
    duplicate_research_ids = {item for item in research_ids if research_ids.count(item) > 1}
    if duplicate_research_ids or set(research_ids).intersection(source_ids):
        research_errors.append("research source IDs must be unique and distinct from app-owned source IDs")
    if research_errors:
        failures.append(("RESEARCH SOURCE REGISTER", research_errors[:40]))

    conflicts = manifest.get("source_conflicts")
    if not isinstance(conflicts, list):
        failures.append(("SOURCE CONFLICT LOG", ["source_conflicts must be an explicit list, even when empty."]))
    else:
        conflict_errors = []
        valid_source_ids = set(source_ids).union(research_ids)
        for index, conflict in enumerate(conflicts, start=1):
            if not isinstance(conflict, dict):
                conflict_errors.append(f"conflict {index}: must be an object")
                continue
            for field in ("claim", "resolution", "status"):
                if not isinstance(conflict.get(field), str) or not conflict.get(field, "").strip():
                    conflict_errors.append(f"conflict {index}: missing {field}")
            referenced = conflict.get("source_ids")
            if (
                not isinstance(referenced, list)
                or len(referenced) < 2
                or not all(isinstance(item, str) for item in referenced)
            ):
                conflict_errors.append(f"conflict {index}: source_ids must name at least two sources")
            elif not set(referenced).issubset(valid_source_ids):
                conflict_errors.append(f"conflict {index}: references an unknown source ID")
        if conflict_errors:
            failures.append(("SOURCE CONFLICT LOG", conflict_errors[:40]))
    if not isinstance(manifest.get("unresolved_gaps"), list):
        failures.append(("UNRESOLVED SOURCE GAPS", ["unresolved_gaps must be an explicit list, even when empty."]))

    if guide_kind == "circuit-lab":
        lab_steps = manifest.get("lab_steps")
        lab_errors = []
        if not isinstance(lab_steps, list) or not lab_steps:
            lab_errors.append("lab_steps must contain every procedure step.")
        else:
            step_ids = set()
            for index, step in enumerate(lab_steps, start=1):
                if not isinstance(step, dict):
                    lab_errors.append(f"step {index}: must be an object")
                    continue
                for field in (
                    "id",
                    "kind",
                    "guide_anchor",
                    "need_ids",
                    "goal",
                    "action",
                    "expected",
                    "wrong",
                    "recovery",
                ):
                    if field == "need_ids":
                        if not isinstance(step.get(field), list) or not step.get(field):
                            lab_errors.append(f"step {index}: missing need_ids")
                    elif not isinstance(step.get(field), str) or not step.get(field, "").strip():
                        lab_errors.append(f"step {index}: missing {field}")
                if step.get("id") in step_ids:
                    lab_errors.append(f"step {index}: duplicate stable id")
                step_ids.add(step.get("id"))
                if step.get("kind") not in {"physical", "conceptual"}:
                    lab_errors.append(f"step {index}: kind must be physical or conceptual")
                if step.get("guide_anchor") not in anchors:
                    lab_errors.append(f"step {index}: guide_anchor is not a real heading")
                if step.get("kind") == "physical":
                    if not isinstance(step.get("primary_visual_asset_id"), str) or not step.get("primary_visual_asset_id", "").strip():
                        lab_errors.append(f"step {index}: physical step needs primary_visual_asset_id")
                    if not isinstance(step.get("visual_evidence"), list) or not step.get("visual_evidence"):
                        lab_errors.append(f"step {index}: physical step needs visual_evidence")
        if lab_errors:
            failures.append(("CIRCUIT LAB STEP CONTRACT", lab_errors[:40]))
    elif guide_kind == "scientific-writing-week":
        writing_errors = []
        if not isinstance(manifest.get("week"), str) or not manifest.get("week", "").strip():
            writing_errors.append("week must identify the course week.")
        if not isinstance(manifest.get("discussion_prompts"), list) or not manifest.get("discussion_prompts"):
            writing_errors.append("discussion_prompts must contain the discussion-ready questions.")
        if not isinstance(manifest.get("assignment_checkpoints"), list):
            writing_errors.append("assignment_checkpoints must be an explicit list, even when empty.")
        if writing_errors:
            failures.append(("SCIENTIFIC WRITING WEEK CONTRACT", writing_errors))

    visual_contract, visual_contract_error = _load_json(
        os.path.join(verify_dir, "visual_contract.json")
    )
    if visual_contract_error:
        failures.append(("VISUAL CONTRACT", [visual_contract_error]))
        visual_contract = {}
    elif (
        not isinstance(visual_contract, dict)
        or visual_contract.get("schema_version") != 2
        or visual_contract.get("decision") not in {"purposeful-visuals", "no-purposeful-visual"}
        or not isinstance(visual_contract.get("needs"), list)
        or not isinstance(visual_contract.get("procedure_steps"), list)
        or not isinstance(visual_contract.get("assets"), list)
        or not re.fullmatch(r"[0-9a-f]{64}", str(visual_contract.get("packet_sha256", "")))
    ):
        failures.append(("VISUAL CONTRACT", ["visual_contract.json is not valid app-owned schema 2."]))
        visual_contract = {}
    visual_plan = manifest.get("visual_plan")
    if not isinstance(visual_plan, dict):
        failures.append(("VISUAL PLAN", ["visual_plan must be an object."]))
    else:
        required = visual_plan.get("required")
        rationale = visual_plan.get("rationale")
        if not isinstance(required, bool):
            failures.append(("VISUAL PLAN", ["visual_plan.required must be boolean."]))
        if not isinstance(rationale, str) or not rationale.strip():
            failures.append(("VISUAL PLAN", ["visual_plan.rationale must explain the visual decision."]))
        contract_requires = visual_contract.get("decision") == "purposeful-visuals"
        if isinstance(required, bool) and required != contract_requires:
            failures.append(("VISUAL PLAN", ["visual_plan.required must match the app-owned visual decision."]))
        if visual_contract.get("decision") == "no-purposeful-visual":
            reason = visual_contract.get("no_visuals_rationale")
            if not isinstance(reason, str) or not reason.strip():
                failures.append(("VISUAL CONTRACT", ["no-purposeful-visual requires an app-owned rationale."]))
            if guide_kind == "circuit-lab":
                failures.append(("VISUAL CONTRACT", ["Circuit Lab cannot opt out of purposeful visual evidence."]))

    procedure_errors = []
    owned_steps = visual_contract.get("procedure_steps", [])
    owned_needs = {
        item.get("id"): item
        for item in visual_contract.get("needs", [])
        if isinstance(item, dict) and isinstance(item.get("id"), str)
    }
    if guide_kind != "circuit-lab":
        if owned_steps:
            procedure_errors.append("non-Circuit guide cannot have app-owned procedure steps")
    else:
        expected_roles = {"before", "action", "expected", "wrong-state", "recovery"}
        if not isinstance(owned_steps, list) or not owned_steps:
            procedure_errors.append("Circuit Lab requires a nonempty app-owned procedure step inventory")
            owned_steps = []
        seen_owned = set()
        for index, step in enumerate(owned_steps, start=1):
            if not isinstance(step, dict) or set(step) != {
                "id", "action", "kind", "guide_anchor", "need_ids", "source_unit_ids", "required_evidence"
            }:
                procedure_errors.append(f"app-owned step {index}: invalid strict schema")
                continue
            step_id = step.get("id")
            if not isinstance(step_id, str) or not step_id or step_id in seen_owned:
                procedure_errors.append(f"app-owned step {index}: invalid or duplicate id")
            else:
                seen_owned.add(step_id)
            if step.get("kind") not in {"physical", "conceptual"}:
                procedure_errors.append(f"app-owned step {index}: invalid kind")
            if not isinstance(step.get("action"), str) or not step.get("action", "").strip():
                procedure_errors.append(f"app-owned step {index}: missing action")
            if not isinstance(step.get("guide_anchor"), str) or not step.get("guide_anchor"):
                procedure_errors.append(f"app-owned step {index}: missing guide_anchor")
            need_ids = step.get("need_ids")
            if (
                not isinstance(need_ids, list)
                or not need_ids
                or not all(isinstance(item, str) for item in need_ids)
                or len(need_ids) != len(set(need_ids))
                or any(need_id not in owned_needs for need_id in need_ids)
            ):
                procedure_errors.append(f"app-owned step {index}: invalid need_ids")
            source_units = step.get("source_unit_ids")
            if (
                not isinstance(source_units, list)
                or not all(isinstance(item, str) for item in source_units)
                or len(source_units) != len(set(source_units))
            ):
                procedure_errors.append(f"app-owned step {index}: invalid source_unit_ids")
            required = step.get("required_evidence")
            if (
                not isinstance(required, list)
                or not all(isinstance(item, str) for item in required)
                or len(required) != len(set(required))
                or not set(required).issubset(expected_roles)
                or (step.get("kind") == "physical" and not required)
                or (step.get("kind") == "conceptual" and required)
            ):
                procedure_errors.append(f"app-owned step {index}: invalid required_evidence")

        actual_steps = manifest.get("lab_steps")
        if not isinstance(actual_steps, list) or len(actual_steps) != len(owned_steps):
            procedure_errors.append("coverage lab_steps must have the exact app-owned step count")
        else:
            for index, (actual, owned) in enumerate(zip(actual_steps, owned_steps), start=1):
                if not isinstance(actual, dict) or not isinstance(owned, dict):
                    continue
                for field in ("id", "kind", "guide_anchor", "need_ids"):
                    if actual.get(field) != owned.get(field):
                        procedure_errors.append(
                            f"step {index}: {field} differs from the app-owned procedure contract"
                        )
    if procedure_errors:
        failures.append(("APP-OWNED CIRCUIT STEP INVENTORY", procedure_errors[:40]))

    verification_plan = manifest.get("verification_plan")
    if not isinstance(verification_plan, dict):
        failures.append(("VERIFICATION PLAN", ["verification_plan must be an object."]))
    else:
        if not isinstance(verification_plan.get("required"), bool):
            failures.append(("VERIFICATION PLAN", ["verification_plan.required must be boolean."]))
        if not isinstance(verification_plan.get("rationale"), str) or not verification_plan.get("rationale", "").strip():
            failures.append(("VERIFICATION PLAN", ["verification_plan.rationale must explain the decision."]))
    return failures, manifest


def _markdown_images(text):
    lines = text.splitlines()
    images = []
    syntax_errors = []
    headings = []
    try:
        expected_version = _expected_markdown_it_version()
    except RuntimeError as exc:
        syntax_errors.append(str(exc))
        return images, lines, syntax_errors, headings
    if MarkdownIt is None or markdown_it is None or getattr(
        markdown_it, "__version__", None
    ) != expected_version:
        actual = getattr(markdown_it, "__version__", "not installed")
        syntax_errors.append(
            f"markdown-it-py=={expected_version} is required; found {actual}"
        )
        return images, lines, syntax_errors, headings

    try:
        tokens = MarkdownIt("commonmark", {"html": True}).parse(text)
    except Exception as exc:
        syntax_errors.append(f"CommonMark parser failed: {exc}")
        return images, lines, syntax_errors, headings

    parsed_image_count = 0
    for token_index, token in enumerate(tokens):
        if token.type in {"html_block", "html_inline"}:
            content = token.content.strip()
            if not re.fullmatch(r"<!--[\s\S]*?-->", content):
                line = token.map[0] + 1 if token.map else 0
                syntax_errors.append(
                    f"line {line}: raw HTML is forbidden in learner guide content"
                )
        if token.type == "heading_open" and token.map:
            inline = tokens[token_index + 1] if token_index + 1 < len(tokens) else None
            if inline is not None and inline.type == "inline":
                headings.append((token.map[0] + 1, int(token.tag[1:]), inline.content))
        if token.type != "inline" or not token.children:
            continue
        for child in token.children:
            if child.type == "html_inline":
                content = child.content.strip()
                if not re.fullmatch(r"<!--[\s\S]*?-->", content):
                    line = token.map[0] + 1 if token.map else 0
                    syntax_errors.append(
                        f"line {line}: raw HTML is forbidden in learner guide content"
                    )
                continue
            if child.type != "image":
                continue
            parsed_image_count += 1
            line_index = token.map[0] if token.map else 0
            path = child.attrs.get("src", "") if isinstance(child.attrs, dict) else ""
            alt = child.content
            images.append(
                {
                    "alt": alt,
                    "path": path,
                    "line": line_index + 1,
                    "index": line_index,
                }
            )
            expected_source = f"![{alt}]({path})"
            raw_line = lines[line_index] if line_index < len(lines) else ""
            if token.content != expected_source or raw_line != expected_source:
                syntax_errors.append(
                    f"line {line_index + 1}: image must use one standalone canonical inline node"
                )

    raw_image_markers = text.count("![")
    if raw_image_markers != parsed_image_count:
        syntax_errors.append(
            "image-like syntax exists outside rendered CommonMark image nodes"
        )
    return images, lines, syntax_errors, headings


def _safe_asset_path(raw_path, guide_dir, assets_dir, assets_prefix=None):
    decoded = urllib.parse.unquote(raw_path).replace("\\", "/")
    parsed = urllib.parse.urlparse(decoded)
    if parsed.scheme or parsed.netloc or decoded.startswith(("/", "//")):
        return None
    logical_assets_dir = (
        (Path(guide_dir) / assets_prefix).resolve()
        if assets_prefix
        else Path(assets_dir).resolve()
    )
    logical_candidate = (Path(guide_dir) / decoded).resolve()
    try:
        relative = logical_candidate.relative_to(logical_assets_dir)
    except ValueError:
        return None
    physical_assets_dir = Path(assets_dir).resolve()
    candidate = (physical_assets_dir / relative).resolve()
    try:
        candidate.relative_to(physical_assets_dir)
    except ValueError:
        return None
    return candidate


def _canonical_visual_text(value, max_bytes):
    if not isinstance(value, str) or not value or len(value.encode("utf-8")) > max_bytes:
        return None
    if value.strip() != value or " ".join(value.split()) != value:
        return None
    if any(
        ord(character) < 32
        or 0x7F <= ord(character) <= 0x9F
        or character in "\\[]<>&`*_"
        for character in value
    ):
        return None
    return value


def check_learner_assets(
    text,
    guide_path,
    coverage_manifest,
    assets_dir_override=None,
    assets_prefix=None,
    verify_dir=None,
):
    failures = []
    guide_dir = os.path.dirname(os.path.abspath(guide_path))
    guide_stem = Path(guide_path).stem
    assets_prefix = assets_prefix or f"{guide_stem}_assets"
    assets_dir = os.path.abspath(
        assets_dir_override or os.path.join(guide_dir, assets_prefix)
    )
    images, lines, image_syntax_errors, parsed_headings = _markdown_images(text)
    if image_syntax_errors:
        failures.append(("UNSUPPORTED IMAGE SYNTAX", image_syntax_errors[:40]))
    coverage = coverage_manifest if isinstance(coverage_manifest, dict) else {}
    visual_plan = coverage.get("visual_plan") if isinstance(coverage.get("visual_plan"), dict) else {}
    visual_required = bool(visual_plan.get("required"))
    if visual_required and not images:
        failures.append(("MISSING REQUIRED LEARNER VISUAL", ["visual_plan requires visuals but the guide embeds none."]))

    visual_contract, contract_error = _load_json(
        os.path.join(verify_dir, "visual_contract.json") if verify_dir else ""
    )
    if contract_error or not isinstance(visual_contract, dict):
        failures.append(("VISUAL CONTRACT", [contract_error or "visual contract must be an object"]));
        visual_contract = {}

    manifest_path = os.path.join(assets_dir, ASSET_MANIFEST_NAME)
    manifest, manifest_error = _load_json(manifest_path)
    if manifest_error:
        failures.append(("MISSING ASSET MANIFEST", [manifest_error]))
        records = []
    elif (
        not isinstance(manifest, dict)
        or manifest.get("schema_version") != 2
        or not isinstance(manifest.get("assets"), list)
        or manifest.get("visual_packet_sha256") != visual_contract.get("packet_sha256")
    ):
        failures.append(("INVALID ASSET MANIFEST", ["schema_version must be 2, assets must be a list, and packet SHA must match the app-owned visual contract."]))
        records = []
    else:
        records = manifest["assets"]

    records_by_path = {}
    provenance_errors = []
    binary_errors = []
    path_errors = []
    record_ids = set()
    seen_sha = set()
    seen_specs = set()
    seen_purposes = set()
    contract_assets = {
        item.get("id"): item
        for item in visual_contract.get("assets", [])
        if isinstance(item, dict) and isinstance(item.get("id"), str)
    }
    contract_needs = {
        item.get("id"): item
        for item in visual_contract.get("needs", [])
        if isinstance(item, dict) and isinstance(item.get("id"), str)
    }
    for record in records:
        if not isinstance(record, dict):
            provenance_errors.append("non-object asset record")
            continue
        raw_path = record.get("path")
        asset_id = record.get("id")
        if not isinstance(asset_id, str) or asset_id in record_ids or asset_id not in contract_assets:
            provenance_errors.append(f"invalid, duplicate, or unknown asset id: {asset_id!r}")
        else:
            record_ids.add(asset_id)
        if not isinstance(raw_path, str):
            provenance_errors.append("asset record has no path")
            continue
        resolved = _safe_asset_path(raw_path, guide_dir, assets_dir, assets_prefix)
        if resolved is None:
            path_errors.append(f"manifest path escapes {Path(assets_dir).name}: {raw_path}")
            continue
        normalized = resolved.as_posix().lower()
        if normalized in records_by_path:
            provenance_errors.append(f"duplicate asset record: {raw_path}")
        records_by_path[normalized] = record
        expected_sha = record.get("sha256")
        spec_sha = record.get("spec_sha256")
        purpose = record.get("learning_purpose")
        width_px = record.get("width_px")
        height_px = record.get("height_px")
        if not isinstance(expected_sha, str) or not re.fullmatch(r"[0-9a-f]{64}", expected_sha):
            binary_errors.append(f"{raw_path}: sha256 must be a lowercase SHA-256 digest")
        elif expected_sha in seen_sha:
            binary_errors.append(f"{raw_path}: duplicate PNG bytes are forbidden")
        else:
            seen_sha.add(expected_sha)
        if not isinstance(spec_sha, str) or not re.fullmatch(r"[0-9a-f]{64}", spec_sha) or spec_sha in seen_specs:
            provenance_errors.append(f"{raw_path}: invalid or duplicate spec_sha256")
        else:
            seen_specs.add(spec_sha)
        normalized_purpose = purpose.strip().lower() if isinstance(purpose, str) else ""
        if not normalized_purpose or normalized_purpose in seen_purposes:
            provenance_errors.append(f"{raw_path}: learning_purpose is missing or duplicated")
        else:
            seen_purposes.add(normalized_purpose)
        if not isinstance(width_px, int) or width_px < 1:
            binary_errors.append(f"{raw_path}: width_px must be a positive integer")
        if not isinstance(height_px, int) or height_px < 1:
            binary_errors.append(f"{raw_path}: height_px must be a positive integer")
        if resolved.is_file():
            try:
                actual_sha = sha256_file(resolved)
                actual_width, actual_height = png_dimensions(resolved)
            except (OSError, ValueError) as exc:
                binary_errors.append(f"{raw_path}: invalid learner image: {exc}")
            else:
                if expected_sha != actual_sha:
                    binary_errors.append(f"{raw_path}: file SHA-256 does not match asset manifest")
                if (width_px, height_px) != (actual_width, actual_height):
                    binary_errors.append(
                        f"{raw_path}: dimensions {actual_width}x{actual_height} do not match asset manifest"
                    )
        for field, max_bytes in (("alt", 240), ("caption", 320), ("explanation", 640)):
            if _canonical_visual_text(record.get(field), max_bytes) is None:
                provenance_errors.append(f"{raw_path}: {field} is not canonical Markdown-safe learner text")
        owned = contract_assets.get(asset_id)
        if isinstance(owned, dict):
            immutable_fields = (
                "filename", "sha256", "spec_sha256", "width_px", "height_px", "kind",
                "evidence_class", "need_ids", "source_unit_ids", "procedure_step_ids",
                "learning_purpose", "alt", "caption", "explanation", "provenance", "rights",
            )
            for field in immutable_fields:
                if field == "filename":
                    if Path(str(raw_path)).name != owned.get(field):
                        provenance_errors.append(f"{raw_path}: filename differs from app-owned contract")
                elif record.get(field) != owned.get(field):
                    provenance_errors.append(f"{raw_path}: {field} differs from app-owned contract")
        need_ids = record.get("need_ids")
        if not isinstance(need_ids, list) or not need_ids or any(item not in contract_needs for item in need_ids):
            provenance_errors.append(f"{raw_path}: need_ids must reference app-owned visual needs")
        section_anchor = record.get("section_anchor")
        if not isinstance(section_anchor, str) or section_anchor not in _heading_anchors(text):
            provenance_errors.append(f"{raw_path}: section_anchor is missing or not a guide heading")
        provenance = record.get("provenance")
        if not isinstance(provenance, dict):
            provenance_errors.append(f"{raw_path}: missing provenance object")
            continue
        for field in ("type", "source", "locator", "transformation"):
            if not isinstance(provenance.get(field), str) or not provenance.get(field, "").strip():
                provenance_errors.append(f"{raw_path}: missing provenance.{field}")
        if provenance.get("type") not in PROVENANCE_TYPES:
            provenance_errors.append(f"{raw_path}: unsupported provenance.type {provenance.get('type')!r}")
        locator = str(provenance.get("locator", ""))
        if provenance.get("type") in {"source-crop", "annotated-source", "book-source", "web-source", "video-frame"}:
            if not re.search(r"(?i)\b(slide|page|figure|section|frame)\b|\b\d{1,2}:\d{2}(?::\d{2})?\b", locator):
                provenance_errors.append(
                    f"{raw_path}: provenance.locator needs a slide/page/figure/section/frame or video timestamp"
                )
        elif provenance.get("type") in {"deterministic-diagram", "generated-educational"}:
            if not re.search(r"(?i)\b(not applicable|n/?a|original)\b", locator):
                provenance_errors.append(
                    f"{raw_path}: original/generated visual locator must explain that source location is not applicable"
                )

    if path_errors:
        failures.append(("ASSET PATH ESCAPE", path_errors))
    if provenance_errors:
        failures.append(("ASSET PROVENANCE", provenance_errors[:40]))
    if binary_errors:
        failures.append(("ASSET INTEGRITY", binary_errors[:40]))

    png_files = []
    try:
        png_files = [path for path in Path(assets_dir).iterdir() if path.is_file() and path.suffix.lower() == ".png"]
    except OSError as exc:
        failures.append(("ASSET INTEGRITY", [f"could not enumerate learner-assets directory: {exc}"]))
    if len(records) != len(images) or len(png_files) != len(records):
        failures.append(("ASSET BIJECTION", [
            f"expected one manifest record, PNG, and Markdown occurrence per selected asset; records={len(records)}, PNGs={len(png_files)}, images={len(images)}"
        ]))

    missing_assets = []
    missing_records = []
    alt_errors = []
    explanation_errors = []
    image_path_counts = {}
    heading_ancestry_for_line = {}
    heading_ancestry = []
    duplicate_slugs = {}
    headings_by_line = {
        line_number: (level, heading)
        for line_number, level, heading in parsed_headings
    }
    for line_number, _line in enumerate(lines, start=1):
        if line_number in headings_by_line:
            level, heading = headings_by_line[line_number]
            heading = heading.strip().lower()
            slug = re.sub(r"[^\w\- ]", "", heading, flags=re.UNICODE)
            slug = re.sub(r"[\s-]+", "-", slug).strip("-")
            index = duplicate_slugs.get(slug, 0)
            duplicate_slugs[slug] = index + 1
            anchor = slug if index == 0 else f"{slug}-{index}"
            while heading_ancestry and heading_ancestry[-1][0] >= level:
                heading_ancestry.pop()
            heading_ancestry.append((level, anchor))
        heading_ancestry_for_line[line_number] = tuple(
            anchor for _level, anchor in heading_ancestry
        )
    for image in images:
        resolved = _safe_asset_path(image["path"], guide_dir, assets_dir, assets_prefix)
        if resolved is None:
            failures.append(("ASSET PATH ESCAPE", [f"guide image path escapes {Path(assets_dir).name}: {image['path']}"]))
            continue
        if not image["alt"]:
            alt_errors.append(f"line {image['line']}: image has empty alt text")
        if not resolved.is_file():
            missing_assets.append(f"line {image['line']}: {image['path']}")
        record = records_by_path.get(resolved.as_posix().lower())
        if record is None:
            missing_records.append(f"line {image['line']}: no manifest record for {image['path']}")
            continue
        normalized = resolved.as_posix().lower()
        image_path_counts[normalized] = image_path_counts.get(normalized, 0) + 1
        if record.get("section_anchor") not in heading_ancestry_for_line.get(image["line"], ()):
            explanation_errors.append(
                f"line {image['line']}: image is not inside its declared section_anchor"
            )
        if image["alt"] != record.get("alt"):
            alt_errors.append(f"line {image['line']}: Markdown alt text does not match asset manifest")
        expected_image = f"![{record.get('alt', '')}]({record.get('path', '')})"
        if lines[image["index"]] != expected_image:
            explanation_errors.append(
                f"line {image['line']}: learner image must occupy one exact canonical line"
            )
        caption = record.get("caption", "")
        explanation = record.get("explanation", "")
        following = image["index"] + 1
        while following < len(lines) and not lines[following].strip():
            following += 1
        expected_caption = f"*Figure: {caption}*"
        if following >= len(lines) or lines[following] != expected_caption:
            explanation_errors.append(f"line {image['line']}: missing the manifest caption after the image")
        else:
            following += 1
            while following < len(lines) and not lines[following].strip():
                following += 1
        expected_explanation = f"**What to notice:** {explanation}"
        if following >= len(lines) or lines[following] != expected_explanation:
            explanation_errors.append(f"line {image['line']}: missing the manifest explanation after the image")
    if missing_assets:
        failures.append(("MISSING LEARNER ASSET", missing_assets))
    if missing_records:
        failures.append(("UNREGISTERED LEARNER ASSET", missing_records))
    if alt_errors:
        failures.append(("IMAGE ALT TEXT", alt_errors))
    if explanation_errors:
        failures.append(("IMAGE EXPLANATION", explanation_errors))
    duplicate_occurrences = [path for path, count in image_path_counts.items() if count != 1]
    if duplicate_occurrences:
        failures.append(("ASSET BIJECTION", ["each selected visual must occur exactly once in Markdown"] + duplicate_occurrences[:20]))

    unit_anchors = {
        item.get("id"): item.get("guide_anchor")
        for item in coverage.get("units", [])
        if isinstance(item, dict) and isinstance(item.get("id"), str)
    }
    section_errors = []
    covered_needs = set()
    for record in records:
        if not isinstance(record, dict):
            continue
        anchor = record.get("section_anchor")
        for unit_id in record.get("source_unit_ids", []):
            if unit_anchors.get(unit_id) != anchor:
                section_errors.append(f"{record.get('id')}: source unit {unit_id} maps to a different section")
        covered_needs.update(record.get("need_ids", []))
    if section_errors:
        failures.append(("VISUAL SOURCE-SECTION EVIDENCE", section_errors[:40]))
    missing_needs = sorted(set(contract_needs) - covered_needs)
    if missing_needs:
        failures.append(("MISSING PURPOSEFUL VISUAL", missing_needs[:40]))

    if coverage.get("guide_kind") == "circuit-lab" and isinstance(coverage.get("lab_steps"), list):
        embedded = {
            resolved.as_posix().lower()
            for image in images
            if (
                resolved := _safe_asset_path(
                    image["path"], guide_dir, assets_dir, assets_prefix
                )
            ) is not None
        }
        unmapped_steps = []
        records_by_id = {
            record.get("id"): record for record in records
            if isinstance(record, dict) and isinstance(record.get("id"), str)
        }
        coverage_steps_by_id = {
            step.get("id"): step
            for step in coverage["lab_steps"]
            if isinstance(step, dict) and isinstance(step.get("id"), str)
        }
        owned_steps = [
            step for step in visual_contract.get("procedure_steps", [])
            if isinstance(step, dict) and isinstance(step.get("id"), str)
        ]
        owned_steps_by_id = {step["id"]: step for step in owned_steps}
        primary_ids = set()
        evidence_pairs = set()
        evidence_roles = {"before", "action", "expected", "wrong-state", "recovery"}
        for index, owned_step in enumerate(owned_steps, start=1):
            if owned_step.get("kind") != "physical":
                continue
            step_id = owned_step.get("id")
            step = coverage_steps_by_id.get(step_id, {})
            primary_id = step.get("primary_visual_asset_id")
            primary = records_by_id.get(primary_id)
            if primary_id in primary_ids:
                unmapped_steps.append(f"step {index}: primary annotated evidence is reused by another physical step")
            primary_ids.add(primary_id)
            if not isinstance(primary, dict):
                unmapped_steps.append(f"step {index}: primary visual is not selected")
            else:
                primary_path = _safe_asset_path(primary.get("path", ""), guide_dir, assets_dir, assets_prefix)
                if (
                    primary.get("kind") != "annotated-source"
                    or primary.get("evidence_class") != "source"
                    or primary.get("procedure_step_ids") != [step_id]
                    or primary.get("section_anchor") != owned_step.get("guide_anchor")
                    or primary_path is None
                    or primary_path.as_posix().lower() not in embedded
                ):
                    unmapped_steps.append(
                        f"step {index}: primary evidence must be an embedded annotated source in the step section and bound to {step_id}"
                    )
            evidence = step.get("visual_evidence", [])
            seen_evidence = set()
            for item in evidence if isinstance(evidence, list) else []:
                if not isinstance(item, dict) or set(item) != {"asset_id", "role"}:
                    unmapped_steps.append(f"step {index}: visual_evidence entries require only asset_id and role")
                    continue
                asset_id, role = item.get("asset_id"), item.get("role")
                if role not in evidence_roles or asset_id in seen_evidence:
                    unmapped_steps.append(f"step {index}: invalid role or duplicate visual_evidence asset")
                    continue
                seen_evidence.add(asset_id)
                record = records_by_id.get(asset_id)
                resolved = _safe_asset_path(record.get("path", ""), guide_dir, assets_dir, assets_prefix) if isinstance(record, dict) else None
                if resolved is None or resolved.as_posix().lower() not in embedded:
                    unmapped_steps.append(f"step {index}: visual_evidence asset is not selected and embedded")
                elif (
                    step_id not in record.get("procedure_step_ids", [])
                    or record.get("section_anchor") != owned_step.get("guide_anchor")
                ):
                    unmapped_steps.append(
                        f"step {index}: visual_evidence asset must be bound to {step_id} and embedded in the step section"
                    )
                else:
                    evidence_pairs.add((step_id, asset_id))
            seen_roles = {
                item.get("role")
                for item in (evidence if isinstance(evidence, list) else [])
                if isinstance(item, dict)
            }
            missing_roles = set(owned_step.get("required_evidence", [])) - seen_roles
            if missing_roles:
                unmapped_steps.append(
                    f"step {index}: missing app-required evidence roles {sorted(missing_roles)}"
                )
            if primary_id not in seen_evidence:
                unmapped_steps.append(
                    f"step {index}: primary_visual_asset_id must also appear in visual_evidence"
                )
        for record in records:
            if not isinstance(record, dict):
                continue
            asset_id = record.get("id")
            for step_id in record.get("procedure_step_ids", []):
                step = owned_steps_by_id.get(step_id)
                if step is None:
                    unmapped_steps.append(
                        f"{asset_id}: selected visual references unknown procedure step {step_id}"
                    )
                elif record.get("section_anchor") != step.get("guide_anchor"):
                    unmapped_steps.append(
                        f"{asset_id}: selected visual is outside procedure step {step_id} section"
                    )
                elif step.get("kind") == "physical" and (step_id, asset_id) not in evidence_pairs:
                    unmapped_steps.append(
                        f"{asset_id}: selected visual bound to physical step {step_id} is missing from that step's visual_evidence"
                    )
        if unmapped_steps:
            failures.append(("CIRCUIT LAB VISUAL MAPPING", unmapped_steps))
    return failures


# ── Pedagogy advisory and exam-practice gate ─────────────────────────────────
# The advisory is deliberately NOT a gate: its heuristics have false positives, and the
# app feeds the report to a bounded deepening pass that decides what to fix. The exam gate
# is a gate because it is unambiguous: a section exists with enough solved questions.
EXAMPLE_MARKER = re.compile(
    r"^(?:>\s*)?(?:\*\*[^*\n]{0,90}?\b(?:example|trace|worked|walkthrough|medium|harder?|challenging|"
    r"classification|Q\d+)\b"
    r"|#{2,4} .*\b(?:example|trace|walkthrough|worked)\b"
    r"|\*Solution\b)",
    re.IGNORECASE,
)
DEFINITION_CUE = re.compile(
    r"\*\*\s*(?:\([^)]{0,40}\)\s*)?(?:is|are|means|refers|denotes|names|describes|:|—|–|-)",
    re.IGNORECASE,
)
DEFINITION_LEAD = re.compile(r"(?:called|known as|termed|named|define[sd]? as)\s+\*\*$", re.IGNORECASE)
EMPHASIS_STOPWORDS = {
    "time", "memory", "code", "data", "four", "three", "two", "one", "first", "second", "third",
    "never", "always", "only", "not", "before", "after", "same", "different", "every", "each",
    "all", "none", "both", "many", "more", "less", "fast", "slow", "true", "false", "yes", "no",
}
# Sections that legitimately teach without a traced example: navigation, history, bridges.
NON_MECHANISM_SECTION = re.compile(
    r"\b(?:reading path|where this leads|table of contents|prerequisite|review|bridge|"
    r"practice|synthesis|history|got here|design goals|what you should|overview|roadmap|how to use|scope|guiding questions|setting the)\b",
    re.IGNORECASE,
)
BOLD_TERM = re.compile(r"\*\*([A-Za-z][A-Za-z0-9\- ]{2,40}?)\*\*")
EXAM_SECTION_TITLE = re.compile(r"^#\s+\d+\.\s+.*\bexam\b", re.IGNORECASE)
QUESTION_MARKER = re.compile(r"^\*\*Q\d+\b")
SOLUTION_MARKER = re.compile(r"^\*Solution\b|^\*\*Solution\b|^\*\*Answer\b", re.IGNORECASE)
EXAM_MIN_QUESTIONS = 8


def major_sections(text):
    """[(title, start_line, end_line)] for `# N. Title` sections; 1-indexed, end inclusive."""
    lines = text.splitlines()
    starts = [(i + 1, line) for i, line in enumerate(lines) if re.match(r"^#\s+\d+\.", line)]
    sections = []
    for index, (start, heading) in enumerate(starts):
        end = starts[index + 1][0] - 1 if index + 1 < len(starts) else len(lines)
        sections.append((heading.lstrip("# ").strip(), start, end))
    return sections


def _prose_lines(text):
    """(line_number, line) for lines outside code fences."""
    flags = split_prose_and_code(text)
    return [
        (i + 1, line)
        for i, (line, flag) in enumerate(zip(text.splitlines(), flags))
        if flag is False
    ]


EXAMPLE_DRILL_HEADING = re.compile(
    # An optional section number first, so its own digits cannot be read as the count.
    r"^#{0,3}\s*(?:\d+(?:\.\d+)*\.?\s+)?"
    r"(?:two|three|four|five|six|seven|eight|nine|ten|\d+)\s+(?:more\s+)?"
    r"(?:worked\s+|reasoning\s+)?(?:examples?|problems?|drills?|exercises?)\b",
    re.IGNORECASE,
)
MAX_EXAMPLES_PER_SECTION = 2


def teaching_subsections(text):
    """[(title, start_line, end_line)] for `## N.M Title` subsections, which is where a single
    idea is taught. Falls back to `# N.` sections for a guide written without subsections."""
    lines = text.splitlines()
    starts = [(i + 1, line) for i, line in enumerate(lines) if re.match(r"^##\s+\d+\.\d", line)]
    if not starts:
        return major_sections(text)
    sections = []
    for index, (line_number, heading) in enumerate(starts):
        stop = starts[index + 1][0] - 1 if index + 1 < len(starts) else len(lines)
        sections.append((heading.lstrip("# ").strip(), line_number, stop))
    return sections


EXAMPLE_DIFFICULTY = re.compile(r"\((medium|hard|harder|challenging)\)", re.IGNORECASE)
# Below this, three examples are three short drills rather than three steps up in difficulty.
THIN_SUBSECTION_WORDS = 1200


def advisory_example_overload(text):
    """Subsections that drill one idea instead of escalating through it.

    A ladder earns its examples: each one is harder or fails differently, and each costs real
    words to work through. A drill repeats one classification with fresh nouns - most visible on
    definitional ideas, which is what readers complain about. The tests below separate the two
    without needing to understand the material, and the exam-style practice section is exempt
    because volume is the point there.
    """
    lines = text.splitlines()
    exam_spans = [
        (start, end) for title, start, end in major_sections(text)
        if EXAM_SECTION_TITLE.match("# " + title)
    ]
    items = []
    for title, start, end in teaching_subsections(text):
        if any(start >= exam_start and end <= exam_end for exam_start, exam_end in exam_spans):
            continue
        body = lines[start:end]
        leads = [line.strip() for line in body if EXAMPLE_LEAD.match(line.strip())]
        words = len(" ".join(body).split())
        if len(leads) > MAX_EXAMPLES_PER_SECTION + 1:
            items.append(
                f"'{title}' (lines {start}-{end}) works {len(leads)} examples of one idea; keep "
                f"the one that shows the mechanism and the one that is hardest or fails "
                f"differently, and fold the rest into a `**Pattern to recognize.**` line"
            )
        elif len(leads) > MAX_EXAMPLES_PER_SECTION and words < THIN_SUBSECTION_WORDS:
            items.append(
                f"'{title}' (lines {start}-{end}) runs {len(leads)} short examples in {words} "
                f"words; that is one idea drilled rather than escalated. Keep the hardest, and "
                f"say the shape once in a `**Pattern to recognize.**` line"
            )
        difficulties = [
            match.group(1).lower()
            for lead in leads
            for match in [EXAMPLE_DIFFICULTY.search(lead)] if match
        ]
        repeated = {level for level in difficulties if difficulties.count(level) > 1}
        if repeated and len(leads) > MAX_EXAMPLES_PER_SECTION:
            items.append(
                f"'{title}' has {len(leads)} examples with the difficulty '{sorted(repeated)[0]}' "
                f"used more than once; examples in one subsection must step up, not repeat a level"
            )
        if EXAMPLE_DRILL_HEADING.match(title.strip()):
            items.append(
                f"'{title}' counts its examples in the title; a title names the question the "
                f"section answers, not how many drills it contains"
            )
    return items


def advisory_sections_without_examples(text):
    lines = text.splitlines()
    items = []
    for title, start, end in major_sections(text):
        if NON_MECHANISM_SECTION.search(title):
            continue
        body = lines[start:end]
        if not any(EXAMPLE_MARKER.match(line.strip()) for line in body):
            items.append(
                f"section '{title}' (lines {start}-{end}) teaches without a labeled worked "
                f"example or trace"
            )
    return items


def advisory_terms_used_before_definition(text):
    """A bold phrase is treated as the point where a term is defined. Report terms whose
    plain-text use appears in an EARLIER major section than that definition."""
    sections = major_sections(text)
    if not sections:
        return []

    def section_of(line_number):
        for index, (_, start, end) in enumerate(sections):
            if start <= line_number <= end:
                return index
        return -1

    prose = _prose_lines(text)
    first_bold = {}
    for number, line in prose:
        if len(BOLD_TERM.findall(line)) >= 3:
            continue  # list emphasis such as "**code**, **data**, and **stack**"
        for match in BOLD_TERM.finditer(line):
            term = match.group(1).strip()
            words = term.split()
            if words[0].lower() in {"q", "example", "worked", "medium", "hard", "harder",
                                    "challenging", "solution", "answer", "what", "figure",
                                    "note", "warning", "tip", "why", "key", "step", "a", "an", "the"}:
                continue
            if re.match(r"^(?:Q|Line |Step |Slide |Page )\d+", term, re.IGNORECASE) or term.lower() in EMPHASIS_STOPWORDS:
                continue
            is_acronym = term.isupper() and len(term) >= 2
            if not (is_acronym or len(words) >= 2 or len(term) >= 7):
                continue
            after = line[match.end():match.end() + 12]
            before = line[max(0, match.start() - 24):match.start() + 2]
            if not (DEFINITION_CUE.match("**" + after) or DEFINITION_LEAD.search(before)):
                continue
            first_bold.setdefault(term.lower(), (number, term))
    items = []
    for key, (definition_line, term) in first_bold.items():
        definition_section = section_of(definition_line)
        if definition_section <= 0:
            continue
        pattern = re.compile(r"\b" + re.escape(term) + r"\b", re.IGNORECASE)
        tagged = re.compile(
            r"\b" + re.escape(term) + r"\b[^\n]{0,40}?\(defined in section", re.IGNORECASE
        )
        for number, line in prose:
            if number >= definition_line:
                break
            if tagged.search(line):
                break  # the first mention was deliberately deferred; later ones inherit it
            use_section = section_of(number)
            if use_section < 0:
                continue  # title block and table of contents are not teaching prose
            if pattern.search(line) and use_section < definition_section and "(defined in section" not in line.lower():
                items.append(
                    f"'{term}' is used at line {number} (section {section_of(number) + 1}) but "
                    f"first defined in bold at line {definition_line} (section {definition_section + 1})"
                )
                break
    return items[:40]


def advisory_unwoven_figures(text):
    lines = text.splitlines()
    items = []
    for i, line in enumerate(lines):
        if not line.startswith("**What to notice:**"):
            continue
        j = i + 1
        while j < len(lines) and not lines[j].strip():
            j += 1
        following = lines[j] if j < len(lines) else ""
        if j >= len(lines) or following.startswith(("![", "#", "---")):
            items.append(
                f"figure ending at line {i + 1} is followed by "
                f"{'another figure' if following.startswith('![') else 'a heading or rule'}"
                f", not by prose that uses it"
            )
    return items


def advisory_thin_sections(text):
    sections = major_sections(text)
    if len(sections) < 4:
        return []
    lines = text.splitlines()
    counts = [(title, start, end, count_words("\n".join(lines[start:end]))) for title, start, end in sections]
    teaching = [c for c in counts if not NON_MECHANISM_SECTION.search(c[0])]
    if len(teaching) < 4:
        return []
    ordered = sorted(c[3] for c in teaching)
    median = ordered[len(ordered) // 2]
    return [
        f"section '{title}' (lines {start}-{end}) has {words} words; the median teaching "
        f"section has {median}"
        for title, start, end, words in teaching
        if words < median * 0.4
    ]



# ── Formatting standard (guide_format_standard.md) ─────────────────────────────
MAX_PARAGRAPH_WORDS = 120
BARE_EXPRESSION_LIMIT = 2  # a paragraph may mention one expression inline; two or more must be structured

EXPRESSION = re.compile(r"\b[A-Za-z]['\w]*(?:\[[^\]]{1,12}\])?\s*(?:=|≤|≥|≠|<=|>=|<|>)\s*[-−]?[\w\[\]]+")
CODE_SPAN = re.compile(r"`[^`\n]*`")
MATH_SPAN = re.compile(r"\$[^$\n]+\$")
SLIDE_NARRATION = re.compile(
    r"(?:\bLecture\s+)?\bPDF\s+page\s+\d+\b(?![^\n]{0,40}\))"  # not inside a citation like (PDF page 49)
    r"|\bslide\s+\d+\s+(?:shows|returns|answers|attaches|is\s+a|repeats|opens|closes)\b"
    r"|\bprinted\s+slide\s+number\b"
    r"|\bpages?\s+\d+\s+(?:then|returns|answers|attaches)\b",
    re.IGNORECASE,
)
READING_INSTRUCTION = re.compile(
    r"\b(?:First|Second|Third|Then|Next|Finally)?,?\s*\bread\s+(?:Chapter|Section|Ch\.|§)\s*[\dA-Z]"
    r"|\bread\s+[^.\n]{0,60}\bPDF\s+pages?\s+\d+"
    r"|\bat\s+PDF\s+pages\s+\d+\s*[–-]\s*\d+",
    re.IGNORECASE,
)
ADMIN_SECTION = re.compile(
    r"^#{1,3}\s+.*\b(?:Questions?\s+to\s+Bring|Any\s+Questions?|Thank\s+you|Outline\s+of\s+This\s+Lecture)\b",
    re.IGNORECASE | re.MULTILINE,
)
EXAMPLE_LEAD = re.compile(
    r"^(?:>\s*)?\*\*(?:[^*\n]{0,80}?\b(?:worked|example|trace)\b|[^*\n]{0,80}?\((?:medium|hard|harder|challenging)\)|(?:medium|harder?|challenging)\s*[\u2014\u2013-])",
    re.IGNORECASE,
)
EXAMPLE_PARTS = ("**Given", "**Steps", "**Result", "**Check")
QUESTION_MARKER = re.compile(r"^\*\*Q(\d+)\b")
QUESTION_HEADING = re.compile(r"^###\s+Q(\d+)\b")
SOLUTION_LINE = re.compile(r"^\*\*Solution\.\*\*\s+\*\*")


def _prose_paragraphs(text):
    """(start_line, paragraph) for paragraphs that are ordinary prose (not headings, lists,
    tables, code, figures, or blockquotes)."""
    paragraphs = []
    lines = text.splitlines()
    in_code = False
    start = None
    buffer = []
    for index, line in enumerate(lines, 1):
        if line.lstrip().startswith("```"):
            in_code = not in_code
            continue
        if in_code:
            continue
        if line.strip():
            if start is None:
                start = index
            buffer.append(line)
        elif buffer:
            paragraphs.append((start, "\n".join(buffer)))
            start, buffer = None, []
    if buffer:
        paragraphs.append((start, "\n".join(buffer)))
    prose = []
    for start, paragraph in paragraphs:
        first = paragraph.lstrip()
        if re.match(r"^(#|\||!\[|\*Figure|\*\*What to notice|[-*]\s|\d+\.\s|>|<!--)", first):
            continue
        prose.append((start, paragraph))
    return prose


def _strip_code_and_math(paragraph):
    return MATH_SPAN.sub(" ", CODE_SPAN.sub(" ", paragraph))


def format_must_fix(text):
    blocks = []
    narration = []
    reading = []
    for start, paragraph in _prose_paragraphs(text):
        plain = _strip_code_and_math(paragraph)
        for match in SLIDE_NARRATION.finditer(plain):
            narration.append(f"line {start}: {match.group(0)!r} - teach the idea, not the deck")
            break
        for match in READING_INSTRUCTION.finditer(plain):
            reading.append(f"line {start}: {match.group(0)!r} - the guide is the reading; keep provenance in the manifest")
            break
    if narration:
        blocks.append(("SLIDE NARRATION IN THE BODY", narration[:40]))
    if reading:
        blocks.append(("READING INSTRUCTIONS IN THE BODY", reading[:40]))
    admin = [
        f"line {text[:m.start()].count(chr(10)) + 1}: {m.group(0).strip()!r} - administrative slides get no section"
        for m in ADMIN_SECTION.finditer(text)
    ]
    if admin:
        blocks.append(("SECTIONS FOR ADMINISTRATIVE SLIDES", admin))
    return blocks


def format_advisory(text):
    blocks = []
    lines = text.splitlines()
    bare, long_paragraphs = [], []
    for start, paragraph in _prose_paragraphs(text):
        words = len(paragraph.split())
        if words > MAX_PARAGRAPH_WORDS:
            long_paragraphs.append(f"line {start}: {words} words - split at the argument's joints or use a list")
        plain = _strip_code_and_math(paragraph)
        expressions = EXPRESSION.findall(plain)
        if len(expressions) >= BARE_EXPRESSION_LIMIT:
            sample = ", ".join(e.strip() for e in expressions[:3])
            bare.append(f"line {start}: {len(expressions)} bare expressions ({sample}) - set values in code spans, a list, or a table")
    if bare:
        blocks.append(("BARE EXPRESSIONS IN PROSE", bare[:60]))
    if long_paragraphs:
        blocks.append(("PARAGRAPHS OVER 120 WORDS", long_paragraphs[:60]))

    skeleton = []
    for index, line in enumerate(lines):
        if EXAMPLE_LEAD.match(line.strip()):
            window = "\n".join(lines[index:index + 25])
            present = sum(1 for part in EXAMPLE_PARTS if part in window)
            if present < 3:
                skeleton.append(f"line {index + 1}: example lacks the Given / Steps / Result / Check skeleton ({present}/4 present)")
    if skeleton:
        blocks.append(("EXAMPLES WITHOUT THE SKELETON", skeleton[:60]))

    practice = []
    for index, line in enumerate(lines):
        stripped = line.strip()
        if QUESTION_MARKER.match(stripped):
            practice.append(f"line {index + 1}: {stripped[:24]!r} is a bold marker; each item is a `### Qn (difficulty) - topic` heading with the question in a blockquote")
        elif QUESTION_HEADING.match(stripped):
            window = lines[index + 1:index + 30]
            if not any(l.strip().startswith(">") for l in window[:4]):
                practice.append(f"line {index + 1}: question text is not in a blockquote")
            if not any(SOLUTION_LINE.match(l.strip()) for l in window):
                practice.append(f"line {index + 1}: no `**Solution.** **<answer>**` line - the answer comes first, in bold")
    if practice:
        blocks.append(("PRACTICE ITEMS WITHOUT STRUCTURE", practice[:60]))

    unstructured = []
    section_start = None
    section_title = None
    def flush(end):
        if section_start is None:
            return
        body = "\n".join(lines[section_start:end])
        # Short sections (a bridge, a scope note) need no internal structure.
        if len(body.split()) >= 200 and not re.search(r"^##\s|^>\s*\*\*", body, re.MULTILINE):
            unstructured.append(f"'{section_title}' (line {section_start}) has no subsection or callout")
    for index, line in enumerate(lines):
        if re.match(r"^#\s+\d+\.", line):
            flush(index)
            section_start, section_title = index + 1, line.lstrip("# ").strip()
    flush(len(lines))
    unstructured = [u for u in unstructured if not re.search(r"(?i)table of contents|reading path|where this leads", u)]
    if unstructured:
        blocks.append(("SECTIONS WITHOUT SUB-STRUCTURE", unstructured[:40]))
    return blocks


def draft_must_fix_items(text, require_exam_practice=False):
    """Hard failures the native gate would report, checkable on a bare draft (no manifests)."""
    blocks = []
    arithmetic = check_arithmetic_equalities(text)
    if arithmetic:
        blocks.append(("ARITHMETIC EQUALITY", arithmetic))
    fences = check_fences(text)
    if fences:
        blocks.append(("UNBALANCED CODE FENCES", fences))
    ramble = find_matches(text, RAMBLING_PATTERNS, prose_only=True)
    if ramble:
        blocks.append(("RAMBLING / VISIBLE SELF-CORRECTION",
                       [f"line {n}: {m!r}" for n, m, _ in ramble[:40]]))
    placeholders = find_matches(text, PLACEHOLDER_PATTERNS, prose_only=True)
    if placeholders:
        blocks.append(("PLACEHOLDERS", [f"line {n}: {m!r}" for n, m, _ in placeholders[:40]]))
    stray = [
        f"line {i + 1}: image-like syntax `![` that is not a standalone image line"
        for i, line in enumerate(text.splitlines())
        if "![" in line and not line.startswith("![")
    ]
    if stray:
        blocks.append(("UNSUPPORTED IMAGE SYNTAX", stray[:40]))
    if require_exam_practice:
        exam = check_exam_practice(text)
        if exam:
            blocks.append(("EXAM PRACTICE SECTION", exam))
    blocks.extend(format_must_fix(text))
    return blocks


def render_pedagogy_advisory(text, require_exam_practice=False):
    """Plain-text report consumed by the app's pedagogy pass and by the writer harness's
    self-check. Empty string means nothing to fix. MUST FIX blocks come first."""
    blocks = [
        ("MUST FIX: " + title, items)
        for title, items in draft_must_fix_items(text, require_exam_practice)
    ]
    blocks += [
        ("SECTIONS WITHOUT A WORKED EXAMPLE", advisory_sections_without_examples(text)),
        ("SECTIONS THAT DRILL ONE IDEA", advisory_example_overload(text)),
        ("TERMS USED BEFORE THEY ARE DEFINED", advisory_terms_used_before_definition(text)),
        ("FIGURES NOT WOVEN INTO PROSE", advisory_unwoven_figures(text)),
        ("SECTIONS MUCH THINNER THAN THE REST", advisory_thin_sections(text)),
    ]
    blocks += format_advisory(text)
    out = []
    for title, items in blocks:
        if items:
            tag = "[MUST FIX]" if title.startswith("MUST FIX: ") else "[ADVISORY]"
            out.append(f"{tag} {title.removeprefix('MUST FIX: ')} ({len(items)})")
            out.extend(f"   {item}" for item in items)
    return "\n".join(out)


def check_exam_practice(text):
    """Hard gate used only when the app supplied a past-exam pattern file."""
    lines = text.splitlines()
    exam_sections = [s for s in major_sections(text) if EXAM_SECTION_TITLE.match("# " + s[0])]
    if not exam_sections:
        return [
            "no major `# N.` section whose title contains the word 'exam'; a past exam was "
            "supplied, so the guide needs an exam-style practice section modeled on it"
        ]
    failures = []
    for title, start, end in exam_sections:
        body = [line.strip() for line in lines[start:end]]
        questions = sum(1 for line in body if QUESTION_MARKER.match(line))
        solutions = sum(1 for line in body if SOLUTION_MARKER.match(line))
        if questions < EXAM_MIN_QUESTIONS or solutions < EXAM_MIN_QUESTIONS:
            failures.append(
                f"section '{title}' has {questions} questions (`**Q1.**` style) and {solutions} "
                f"solutions (`*Solution.*` style); at least {EXAM_MIN_QUESTIONS} of each are required"
            )
    return failures


def _without_completion_marker(text):
    return text.replace(GUIDE_COMPLETE_SENTINEL, "").rstrip() + "\n"


def _seal_guide(guide_path, text):
    sealed = _without_completion_marker(text).rstrip() + "\n\n" + GUIDE_COMPLETE_SENTINEL + "\n"
    _atomic_write_text(guide_path, sealed)


def main():
    parser = argparse.ArgumentParser(description="Lint + verify a generated study guide.")
    parser.add_argument("guide", nargs="?", help="Path to the generated <Name>_Guide.md")
    parser.add_argument("--verify-dir", default=None,
                        help="Dir holding immutable manifests and rendered verification assets "
                             "(default: <guide_dir>/.<guide_name>.gwverify/)")
    parser.add_argument("--source", default=None,
                        help="Original source path; validates it against the pre-generation snapshot")
    parser.add_argument("--expected-source-sha", default=None,
                        help="Trusted pre-generation SHA-256 retained by the launcher")
    parser.add_argument("--expected-guide-name", default=None,
                        help="Final guide filename when linting a staged candidate")
    parser.add_argument(
        "--expected-guide-kind",
        choices=("lecture", "course-week", "circuit-lab", "scientific-writing-week"),
        default=None,
        help="App-planned guide kind that the coverage manifest must preserve",
    )
    parser.add_argument(
        "--expected-course-profile",
        choices=(
            "computer-networks",
            "computer-algorithms",
            "operating-systems",
            "circuit-lab",
            "scientific-writing",
            "legacy-fourth-semester",
        ),
        default=None,
        help="App-planned course profile used for immutable depth-policy selection",
    )
    parser.add_argument("--assets-dir", default=None,
                        help="Physical staged learner-assets directory")
    parser.add_argument("--assets-prefix", default=None,
                        help="Logical sibling asset-directory name used by Markdown paths")
    parser.add_argument("--snapshot-source", default=None,
                        help="Record source SHA-256 and unit count before generation, then exit")
    parser.add_argument("--no-harness", action="store_true",
                        help=argparse.SUPPRESS)
    parser.add_argument("--seal", action="store_true",
                        help="Atomically append the completion marker only if every gate passes; "
                             "remove any premature marker on failure")
    parser.add_argument("--advisory", action="store_true",
                        help="Print the pedagogy advisory for the guide (worked examples, "
                             "define-before-use, figure weaving, thin sections) and exit 0")
    parser.add_argument("--require-exam-practice", action="store_true",
                        help="Require an exam-style practice section; set by the app when a "
                             "past-exam pattern file exists in the course folder")
    args = parser.parse_args()

    if args.snapshot_source:
        if not args.verify_dir:
            parser.error("--snapshot-source requires --verify-dir")
        try:
            path, snapshot = record_source_snapshot(args.snapshot_source, args.verify_dir)
        except ValueError as exc:
            print(f"ERROR: {exc}", file=sys.stderr)
            sys.exit(1)
        print(
            f"source_snapshot: {snapshot['sources'][0]['name']} "
            f"({snapshot['sources'][0]['unit_count']} "
            f"{snapshot['sources'][0]['unit_kind']}(s)) -> {path}"
        )
        sys.exit(0)

    if not args.guide:
        parser.error("guide is required unless --snapshot-source is used")

    if not os.path.isfile(args.guide):
        print(f"ERROR: guide not found: {args.guide}", file=sys.stderr)
        sys.exit(1)

    with open(args.guide, encoding="utf-8") as f:
        text = f.read()

    if args.advisory:
        report = render_pedagogy_advisory(text, args.require_exam_practice)
        print(report if report else "[ADVISORY] nothing to fix")
        sys.exit(0)

    # A model or stale earlier run may have written the marker itself. In seal
    # mode, remove it before evaluating any gate so failure can never leave a
    # false completion signal behind.
    if args.seal and GUIDE_COMPLETE_SENTINEL in text:
        try:
            text = _without_completion_marker(text)
            _atomic_write_text(args.guide, text)
        except OSError as exc:
            print(f"ERROR: could not remove premature completion marker: {exc}", file=sys.stderr)
            sys.exit(1)

    guide_dir = os.path.dirname(os.path.abspath(args.guide))
    guide_name = os.path.basename(args.guide)
    verify_dir = os.path.abspath(
        args.verify_dir or os.path.join(guide_dir, "." + guide_name + ".gwverify")
    )
    harness_path = os.path.join(verify_dir, "verify.py")

    failures = []   # list of (category, [details])
    warnings = []   # list of (category, [details])

    if args.seal and args.no_harness:
        failures.append((
            "SEAL CONFIGURATION",
            ["--seal cannot be combined with --no-harness; completion must run every applicable gate."],
        ))

    # 1. Rambling / self-correction (prose only)
    ramble = find_matches(text, RAMBLING_PATTERNS, prose_only=True)
    if ramble:
        failures.append((
            "RAMBLING / visible self-correction (remove these from the final guide)",
            [f"L{ln}: \"{mt}\"  —  {lt[:90]}" for ln, mt, lt in ramble[:40]],
        ))

    # 2. Placeholders / hand-waving (everywhere)
    ph = find_matches(text, PLACEHOLDER_PATTERNS, prose_only=False)
    if ph:
        failures.append((
            "HAND-WAVING placeholder markers",
            [f"L{ln}: \"{mt}\"  —  {lt[:90]}" for ln, mt, lt in ph[:40]],
        ))

    # 3. Structural: fences
    fence_fail = check_fences(text)
    if fence_fail:
        failures.append(("STRUCTURE", fence_fail))

    # WARN: ellipsis inside code blocks
    ell = check_ellipsis_in_code(text)
    if ell:
        warnings.append((
            "'...' inside code blocks (verify each is a real math sequence, not a dropped diagram element)",
            [f"L{ln}: {lt[:90]}" for ln, lt in ell[:20]],
        ))

    # 4. Source integrity, exhaustive coverage, cumulative continuity, visuals
    source_failures, coverage_manifest = check_source_and_coverage(
        text,
        args.guide,
        verify_dir,
        args.source,
        args.expected_source_sha,
        args.expected_guide_name,
        args.expected_guide_kind,
        args.expected_course_profile,
    )
    failures.extend(source_failures)
    failures.extend(
        check_learner_assets(
            text,
            args.guide,
            coverage_manifest,
            args.assets_dir,
            args.assets_prefix,
            verify_dir,
        )
    )

    # 5. Fixed arithmetic verifier and executable-code prohibition
    arith_count = len(ARITH_EQ.findall(text))
    declared_verification = bool(
        isinstance(coverage_manifest, dict)
        and isinstance(coverage_manifest.get("verification_plan"), dict)
        and coverage_manifest["verification_plan"].get("required") is True
    )
    app_requires_verification = False
    is_computational = (
        args.expected_course_profile in DENSE_DEPTH_COURSE_PROFILES
        or (
            args.expected_course_profile is None
            and arith_count >= COMPUTATIONAL_THRESHOLD
        )
    )
    harness_exists = os.path.isfile(harness_path)
    slide_count = count_rendered_slides(verify_dir)
    word_count = count_words(text)
    major_sections = count_major_sections(text)
    arithmetic_failures = check_arithmetic_equalities(text)
    if arithmetic_failures:
        failures.append(("ARITHMETIC EQUALITY failed", arithmetic_failures))

    if is_computational and slide_count >= DEPTH_MIN_SLIDES:
        min_words = slide_count * DEPTH_WORDS_PER_SLIDE
        min_sections = min(20, max(10, slide_count // 2))
        depth_items = []
        if word_count < min_words:
            depth_items.append(
                f"only {word_count} words for {slide_count} rendered slides; "
                f"minimum floor is {min_words} ({DEPTH_WORDS_PER_SLIDE} words/slide)."
            )
        if major_sections < min_sections:
            depth_items.append(
                f"only {major_sections} major numbered sections; minimum floor is {min_sections}."
            )
        if depth_items:
            failures.append((
                "DEPTH CONTRACT not met",
                depth_items + [
                    "Expand the guide to Claude-level explanatory depth: more mechanism, bridge explanations, "
                    "worked examples, and verified intermediate states."
                ],
            ))

    if args.require_exam_practice:
        exam_failures = check_exam_practice(text)
        if exam_failures:
            failures.append(("EXAM PRACTICE SECTION", exam_failures))

    if declared_verification != app_requires_verification:
        failures.append((
            "VERIFICATION PLAN MISMATCH",
            ["coverage_manifest.verification_plan.required does not match the "
             f"app-owned decision ({app_requires_verification})."],
        ))

    if harness_exists:
        failures.append((
            "UNEXPECTED VERIFICATION HARNESS",
            ["verify.py is always forbidden; fixed app-owned checks are used and model-authored "
             "code is rejected; it was not executed."],
        ))

    if args.seal and not failures:
        try:
            _seal_guide(args.guide, text)
        except OSError as exc:
            failures.append(("COMPLETION MARKER WRITE", [str(exc)]))

    # ── Report ────────────────────────────────────────────────────────────────
    print("=" * 70)
    print(f"guide_lint: {guide_name}")
    print(f"  arithmetic steps: {arith_count}  (computational: {is_computational})")
    print(f"  words: {word_count}  major sections: {major_sections}  rendered slides: {slide_count}")
    print(f"  forbidden model code: {'found' if harness_exists else 'absent'} @ {harness_path}")
    print("=" * 70)

    for cat, details in warnings:
        print(f"\n[WARN] {cat}")
        for d in details:
            print(f"   {d}")

    if not failures:
        marker_note = " Completion marker sealed atomically." if args.seal else ""
        print(f"\nRESULT: PASS — no hard failures.{marker_note}")
        sys.exit(0)

    for cat, details in failures:
        print(f"\n[FAIL] {cat}")
        for d in details:
            print(f"   {d}")

    total = sum(len(d) for _, d in failures)
    print(f"\nRESULT: FAIL — {len(failures)} category(ies), {total} item(s). Fix and re-run.")
    sys.exit(1)


if __name__ == "__main__":
    main()
