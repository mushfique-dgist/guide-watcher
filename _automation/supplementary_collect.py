#!/usr/bin/env python3
"""Collect supplementary lecture transcripts for a course folder, once, with a full report.

The model's job is only to *propose* sources (a candidates JSON); this script does the
fetching deterministically and never skips anything silently. Every candidate ends up in
`_supplementary/collection_report.md` with one of these statuses:

  ok            transcript saved to _supplementary/<id>/transcript.md
  no_captions   the video has no usable transcript (disabled or not in an accepted language)
  blocked       the network refused us (HTTP 403/429, IP block); retrying later may help
  unavailable   the video/page does not exist or is private
  invalid       the candidate record itself is malformed (bad id, bad URL, unknown kind)
  network_error could not reach the host after retries

Usage:
  supplementary_collect.py plan-template
  supplementary_collect.py collect <course_dir> --candidates <candidates.json>
                           [--timeout 30] [--retries 3] [--languages en,en-US,en-GB] [--strict]
  supplementary_collect.py status <course_dir>

Files under `<course_dir>/_supplementary/`:
  manifest.json          merged by candidate id; carries status, sha256, word count, lecture_map
  collection_report.md   the human-readable outcome of the last run
  <id>/transcript.md     the transcript with a metadata header; paragraphs carry [mm:ss] stamps
"""
from __future__ import annotations

import argparse
import datetime as _dt
import hashlib
import html as _html
import io
import json
import os
import re
import sys
import tempfile
import time
from dataclasses import dataclass, field
from typing import Callable, Dict, List, Optional
from urllib.parse import parse_qs, urlparse

SUPPLEMENTARY_DIR = "_supplementary"
MANIFEST_NAME = "manifest.json"
REPORT_NAME = "collection_report.md"
TRANSCRIPT_NAME = "transcript.md"
ID_PATTERN = re.compile(r"^[a-z0-9][a-z0-9._-]{0,80}$")
YOUTUBE_ID = re.compile(r"^[A-Za-z0-9_-]{11}$")
ACCEPTED_KINDS = ("youtube", "url")
DEFAULT_LANGUAGES = ("en", "en-US", "en-GB")
PARAGRAPH_SECONDS = 60
USER_AGENT = "GuideWatcher-SupplementaryCollector/1.0 (personal study use)"


# ── Statuses ────────────────────────────────────────────────────────────────


@dataclass
class FetchResult:
    status: str                      # ok | no_captions | blocked | unavailable | network_error
    detail: str = ""
    text: str = ""                   # rendered transcript body (without the header)
    duration_seconds: Optional[int] = None
    retryable: bool = False


@dataclass
class Fetchers:
    """Injection point so tests run without the network."""
    youtube: Callable[[str, List[str], int], FetchResult]
    url: Callable[[str, int], FetchResult]
    sleep: Callable[[float], None] = time.sleep


# ── Candidate validation ────────────────────────────────────────────────────


def youtube_video_id(url: str) -> Optional[str]:
    parsed = urlparse(url)
    if parsed.scheme not in ("http", "https"):
        return None
    host = parsed.netloc.lower()
    if host in ("youtu.be", "www.youtu.be"):
        candidate = parsed.path.strip("/").split("/")[0]
    elif host.endswith("youtube.com"):
        if parsed.path == "/watch":
            candidate = (parse_qs(parsed.query).get("v") or [""])[0]
        else:
            parts = [p for p in parsed.path.split("/") if p]
            candidate = parts[1] if len(parts) >= 2 and parts[0] in ("shorts", "embed", "live", "v") else ""
    else:
        return None
    return candidate if YOUTUBE_ID.match(candidate or "") else None


def validate_candidate(candidate: dict) -> Optional[str]:
    """Return an error message, or None when the record is usable."""
    if not isinstance(candidate, dict):
        return "candidate is not an object"
    cid = candidate.get("id")
    if not isinstance(cid, str) or not ID_PATTERN.match(cid):
        return "id must match ^[a-z0-9][a-z0-9._-]{0,80}$"
    kind = candidate.get("kind")
    if kind not in ACCEPTED_KINDS:
        return f"kind must be one of {ACCEPTED_KINDS}"
    url = candidate.get("url")
    if not isinstance(url, str) or urlparse(url).scheme not in ("http", "https"):
        return "url must be an http(s) URL"
    if kind == "youtube" and not youtube_video_id(url):
        return "url is not a recognizable YouTube video URL"
    if not isinstance(candidate.get("title"), str) or not candidate["title"].strip():
        return "title is required"
    return None


# ── Rendering ────────────────────────────────────────────────────────────────


def _stamp(seconds: float) -> str:
    total = int(seconds)
    hours, rest = divmod(total, 3600)
    minutes, secs = divmod(rest, 60)
    return f"{hours:d}:{minutes:02d}:{secs:02d}" if hours else f"{minutes:02d}:{secs:02d}"


def render_caption_paragraphs(snippets: List[dict], paragraph_seconds: int = PARAGRAPH_SECONDS) -> str:
    """Group caption snippets ({text,start,duration}) into ~1-minute paragraphs with a stamp."""
    paragraphs: List[str] = []
    current: List[str] = []
    current_start: Optional[float] = None
    for snippet in snippets:
        text = re.sub(r"\s+", " ", str(snippet.get("text", ""))).strip()
        if not text or text in ("[Music]", "[Applause]"):
            continue
        start = float(snippet.get("start", 0.0))
        if current_start is None:
            current_start = start
        if start - current_start >= paragraph_seconds and current:
            paragraphs.append(f"[{_stamp(current_start)}] " + " ".join(current))
            current, current_start = [], start
        current.append(text)
    if current:
        paragraphs.append(f"[{_stamp(current_start or 0.0)}] " + " ".join(current))
    return "\n\n".join(paragraphs) + ("\n" if paragraphs else "")


def html_to_text(page: str) -> str:
    page = re.sub(r"(?is)<(script|style|noscript)\b.*?</\1>", " ", page)
    page = re.sub(r"(?i)<br\s*/?>|</p>|</div>|</h[1-6]>|</li>|</tr>", "\n", page)
    page = re.sub(r"<[^>]+>", " ", page)
    page = _html.unescape(page)
    lines = [re.sub(r"[ \t]+", " ", line).strip() for line in page.splitlines()]
    text = "\n".join(line for line in lines if line)
    return re.sub(r"\n{3,}", "\n\n", text).strip() + "\n"


def transcript_document(candidate: dict, result: FetchResult, fetched: str) -> str:
    header = [
        f"# {candidate['title'].strip()}",
        "",
        f"- id: {candidate['id']}",
        f"- kind: {candidate['kind']}",
        f"- url: {candidate['url']}",
        f"- source: {candidate.get('source', '').strip() or 'unknown'}",
        f"- license_note: {candidate.get('license_note', '').strip() or 'personal study use; not redistributed'}",
        f"- topics: {', '.join(candidate.get('topics') or []) or 'unassigned'}",
        f"- fetched: {fetched}",
    ]
    if result.duration_seconds is not None:
        header.append(f"- duration_seconds: {result.duration_seconds}")
    header += ["", "---", "", result.text.rstrip() + "\n"]
    return "\n".join(header)


# ── Network fetchers (real) ─────────────────────────────────────────────────


def fetch_youtube_real(video_id: str, languages: List[str], timeout: int) -> FetchResult:
    try:
        from youtube_transcript_api import YouTubeTranscriptApi  # type: ignore
        from youtube_transcript_api import _errors as yt_errors  # type: ignore
    except ImportError:
        return FetchResult("network_error", "youtube-transcript-api is not installed (pip install youtube-transcript-api==1.2.4)")
    try:
        fetched = YouTubeTranscriptApi().fetch(video_id, languages=list(languages))
    except Exception as exc:  # classified below; the library's hierarchy varies by version
        name = type(exc).__name__
        message = f"{name}: {str(exc).strip().splitlines()[0] if str(exc).strip() else ''}"
        if name in ("TranscriptsDisabled", "NoTranscriptFound", "NotTranslatable", "TranslationLanguageNotAvailable"):
            return FetchResult("no_captions", message)
        if name in ("IpBlocked", "RequestBlocked", "TooManyRequests"):
            return FetchResult("blocked", message, retryable=True)
        if name in ("VideoUnavailable", "VideoUnplayable", "InvalidVideoId", "AgeRestricted"):
            return FetchResult("unavailable", message)
        if name in ("CouldNotRetrieveTranscript", "YouTubeRequestFailed", "YouTubeDataUnparsable"):
            return FetchResult("network_error", message, retryable=True)
        if isinstance(exc, getattr(yt_errors, "CouldNotRetrieveTranscript", ())):
            return FetchResult("network_error", message, retryable=True)
        return FetchResult("network_error", message, retryable=True)
    snippets = [{"text": s.text, "start": s.start, "duration": s.duration} for s in fetched]
    if not snippets:
        return FetchResult("no_captions", "transcript is empty")
    last = snippets[-1]
    return FetchResult(
        "ok",
        f"{len(snippets)} caption snippets ({getattr(fetched, 'language_code', '?')})",
        render_caption_paragraphs(snippets),
        duration_seconds=int(last["start"] + last["duration"]),
    )


def fetch_url_real(url: str, timeout: int) -> FetchResult:
    try:
        import requests  # type: ignore
    except ImportError:
        return FetchResult("network_error", "requests is not installed")
    try:
        response = requests.get(url, headers={"User-Agent": USER_AGENT}, timeout=timeout)
    except requests.RequestException as exc:  # type: ignore[attr-defined]
        return FetchResult("network_error", f"{type(exc).__name__}: {exc}", retryable=True)
    if response.status_code in (401, 403, 429):
        return FetchResult("blocked", f"HTTP {response.status_code}", retryable=response.status_code == 429)
    if response.status_code in (404, 410):
        return FetchResult("unavailable", f"HTTP {response.status_code}")
    if response.status_code >= 500:
        return FetchResult("network_error", f"HTTP {response.status_code}", retryable=True)
    if response.status_code != 200:
        return FetchResult("network_error", f"HTTP {response.status_code}")
    content_type = response.headers.get("content-type", "").lower()
    if "pdf" in content_type or url.lower().endswith(".pdf"):
        try:
            import pymupdf  # type: ignore
        except ImportError:
            return FetchResult("network_error", "pymupdf is not installed; cannot read a PDF transcript")
        document = pymupdf.open(stream=response.content, filetype="pdf")
        pages = [page.get_text() for page in document]
        text = "\n\n".join(f"--- page {i + 1} ---\n{page.strip()}" for i, page in enumerate(pages) if page.strip())
    else:
        text = html_to_text(response.text)
    if len(text.split()) < 50:
        return FetchResult("unavailable", f"page has no usable text ({len(text.split())} words)")
    return FetchResult("ok", f"{len(text.split())} words", text)


REAL_FETCHERS = Fetchers(youtube=fetch_youtube_real, url=fetch_url_real)


# ── Storage ─────────────────────────────────────────────────────────────────


def _atomic_write(path: str, data: str) -> None:
    directory = os.path.dirname(path)
    os.makedirs(directory, exist_ok=True)
    fd, temp = tempfile.mkstemp(prefix=".tmp-", dir=directory)
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(data)
        os.replace(temp, path)
    except BaseException:
        try:
            os.remove(temp)
        except OSError:
            pass
        raise


def load_manifest(course_dir: str) -> dict:
    path = os.path.join(course_dir, SUPPLEMENTARY_DIR, MANIFEST_NAME)
    if not os.path.isfile(path):
        return {"schema_version": 1, "items": []}
    with io.open(path, encoding="utf-8") as handle:
        data = json.load(handle)
    if not isinstance(data, dict) or not isinstance(data.get("items"), list):
        raise ValueError(f"malformed manifest: {path}")
    return data


def save_manifest(course_dir: str, manifest: dict) -> None:
    manifest["schema_version"] = 1
    manifest["updated"] = _now()
    _atomic_write(
        os.path.join(course_dir, SUPPLEMENTARY_DIR, MANIFEST_NAME),
        json.dumps(manifest, indent=1, ensure_ascii=False) + "\n",
    )


def _now() -> str:
    return _dt.datetime.now(_dt.timezone.utc).replace(microsecond=0).isoformat()


# ── Collection ──────────────────────────────────────────────────────────────


@dataclass
class Outcome:
    id: str
    status: str
    detail: str
    path: Optional[str] = None
    words: int = 0


def fetch_with_retries(candidate: dict, fetchers: Fetchers, languages: List[str], timeout: int, retries: int) -> FetchResult:
    attempt = 0
    while True:
        if candidate["kind"] == "youtube":
            result = fetchers.youtube(youtube_video_id(candidate["url"]), languages, timeout)
        else:
            result = fetchers.url(candidate["url"], timeout)
        if result.status == "ok" or not result.retryable or attempt >= retries:
            return result
        attempt += 1
        fetchers.sleep(min(30.0, 2.0 ** attempt))


def collect(course_dir: str, candidates: List[dict], fetchers: Fetchers = REAL_FETCHERS,
            languages: List[str] = list(DEFAULT_LANGUAGES), timeout: int = 30, retries: int = 3) -> List[Outcome]:
    if not os.path.isdir(course_dir):
        raise ValueError(f"course folder does not exist: {course_dir}")
    manifest = load_manifest(course_dir)
    by_id: Dict[str, dict] = {item.get("id"): item for item in manifest["items"] if isinstance(item, dict)}
    outcomes: List[Outcome] = []
    seen: set = set()
    for candidate in candidates:
        error = validate_candidate(candidate)
        cid = candidate.get("id") if isinstance(candidate, dict) else None
        label = cid if isinstance(cid, str) else "<no id>"
        if error:
            outcomes.append(Outcome(label, "invalid", error))
            continue
        if cid in seen:
            outcomes.append(Outcome(cid, "invalid", "duplicate id in candidates"))
            continue
        seen.add(cid)
        result = fetch_with_retries(candidate, fetchers, languages, timeout, retries)
        fetched = _now()
        record = dict(by_id.get(cid) or {})
        record.update({
            "id": cid,
            "kind": candidate["kind"],
            "url": candidate["url"],
            "title": candidate["title"].strip(),
            "source": (candidate.get("source") or "").strip(),
            "topics": list(candidate.get("topics") or []),
            "license_note": (candidate.get("license_note") or "").strip(),
            "status": result.status,
            "status_detail": result.detail,
            "last_attempt": fetched,
        })
        record.setdefault("lecture_map", [])
        record.setdefault("audited", False)
        if result.status == "ok":
            document = transcript_document(candidate, result, fetched)
            relative = os.path.join(SUPPLEMENTARY_DIR, cid, TRANSCRIPT_NAME)
            _atomic_write(os.path.join(course_dir, relative), document)
            record.update({
                "file": relative.replace(os.sep, "/"),
                "sha256": hashlib.sha256(document.encode("utf-8")).hexdigest(),
                "word_count": len(result.text.split()),
                "duration_seconds": result.duration_seconds,
                "fetched": fetched,
            })
            outcomes.append(Outcome(cid, "ok", result.detail, record["file"], record["word_count"]))
        else:
            outcomes.append(Outcome(cid, result.status, result.detail))
        by_id[cid] = record
    manifest["items"] = [by_id[key] for key in sorted(by_id)]
    save_manifest(course_dir, manifest)
    _atomic_write(os.path.join(course_dir, SUPPLEMENTARY_DIR, REPORT_NAME), render_report(outcomes))
    return outcomes


def render_report(outcomes: List[Outcome]) -> str:
    counts: Dict[str, int] = {}
    for outcome in outcomes:
        counts[outcome.status] = counts.get(outcome.status, 0) + 1
    lines = [
        "# Supplementary collection report",
        "",
        f"Run at {_now()}. {len(outcomes)} candidate(s): "
        + ", ".join(f"{status} {count}" for status, count in sorted(counts.items())) + ".",
        "",
        "Nothing is skipped silently: every candidate has a row. `blocked` and `network_error` "
        "rows are worth retrying later; `no_captions` and `unavailable` need a different source.",
        "",
        "| id | status | detail | words | file |",
        "|---|---|---|---|---|",
    ]
    for outcome in outcomes:
        lines.append(
            f"| {outcome.id} | {outcome.status} | {outcome.detail.replace('|', '/')} | "
            f"{outcome.words or ''} | {outcome.path or ''} |"
        )
    return "\n".join(lines) + "\n"


# ── CLI ─────────────────────────────────────────────────────────────────────


PLAN_TEMPLATE = {
    "schema_version": 1,
    "course": "Operating Systems",
    "candidates": [
        {
            "id": "remzi-cs537-day1",
            "kind": "youtube",
            "url": "https://www.youtube.com/watch?v=LVxN7ZkGh3w",
            "title": "CS537 Day 1: intro, CPU virtualization, limited direct execution",
            "source": "UW-Madison CS537 (Remzi Arpaci-Dusseau)",
            "topics": ["processes", "limited direct execution"],
            "license_note": "YouTube captions; personal study use",
        },
        {
            "id": "mit6004-timesharing",
            "kind": "url",
            "url": "https://ocw.mit.edu/courses/6-004-computation-structures-spring-2017/resources/17-2-3-timesharing/",
            "title": "MIT 6.004 17.2.3 Timesharing (transcript page)",
            "source": "MIT OpenCourseWare",
            "topics": ["timer interrupt", "context switch"],
            "license_note": "CC BY-NC-SA",
        },
    ],
}


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("plan-template", help="print an example candidates.json")
    collect_parser = sub.add_parser("collect", help="fetch every candidate and write the report")
    collect_parser.add_argument("course_dir")
    collect_parser.add_argument("--candidates", required=True)
    collect_parser.add_argument("--timeout", type=int, default=30)
    collect_parser.add_argument("--retries", type=int, default=3)
    collect_parser.add_argument("--languages", default=",".join(DEFAULT_LANGUAGES))
    collect_parser.add_argument("--strict", action="store_true", help="exit 2 when any candidate did not succeed")
    status_parser = sub.add_parser("status", help="print the manifest summary")
    status_parser.add_argument("course_dir")
    args = parser.parse_args(argv)

    if args.command == "plan-template":
        print(json.dumps(PLAN_TEMPLATE, indent=1))
        return 0
    if args.command == "status":
        manifest = load_manifest(args.course_dir)
        for item in manifest["items"]:
            print(f"{item.get('status', '?'):14s} {item.get('id')}  ({item.get('word_count', 0)} words)  {item.get('status_detail', '')}")
        return 0
    with io.open(args.candidates, encoding="utf-8") as handle:
        plan = json.load(handle)
    candidates = plan.get("candidates") if isinstance(plan, dict) else None
    if not isinstance(candidates, list) or not candidates:
        print("ERROR: candidates file must contain a non-empty 'candidates' list", file=sys.stderr)
        return 2
    outcomes = collect(
        args.course_dir, candidates,
        languages=[lang.strip() for lang in args.languages.split(",") if lang.strip()],
        timeout=args.timeout, retries=args.retries,
    )
    print(render_report(outcomes))
    failed = [o for o in outcomes if o.status != "ok"]
    return 2 if (failed and args.strict) else 0


if __name__ == "__main__":
    sys.exit(main())
