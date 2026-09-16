#!/usr/bin/env python3
"""Adversarial preflight sweep: run `guide-watcher-cli check` on every saved prep packet and
every lecture source in the configured course folders, without starting any model, and report
what a user would hit. Run it after any change to the app, the _automation files, or a course
folder; a clean sweep is the acceptance gate.

Usage: python preflight_sweep.py [--cli <path to guide-watcher-cli.exe>] [--semester <folder>]
       [--course <name> ...] [--skip-sealed]

Exit code 0 when every target passes, 1 otherwise. Targets are lecture sources (PDF/PPTX/DOCX/
HTML files that match each course's lecture rule loosely: not textbooks, not guides) and
`*.prep.md` packets. A lecture whose guide or prep packet already exists is expected to be
refused with "refusing to overwrite"; that outcome is reported as `existing` and does not fail
the sweep (the packet itself is checked as its own target).
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time

DEFAULT_CLI = os.path.join(os.environ.get("LOCALAPPDATA", ""), "Guide Watcher", "guide-watcher-cli.exe")
DEFAULT_SEMESTER = r"C:\Users\Mushfique\Desktop\DGIST\5th Semester"
COURSES = {
    "Computer Networks": re.compile(r"^CH\d+.*\.pdf$", re.IGNORECASE),
    "Computer Algorithms": re.compile(r"^L\d+\s*_.*\.pdf$", re.IGNORECASE),
    "Operating Systems": re.compile(r"^\d+-.*\.pdf$", re.IGNORECASE),
}


def targets(semester: str, courses: list[str]):
    for course, rule in COURSES.items():
        if courses and course not in courses:
            continue
        folder = os.path.join(semester, course)
        if not os.path.isdir(folder):
            continue
        for name in sorted(os.listdir(folder)):
            path = os.path.join(folder, name)
            if not os.path.isfile(path):
                continue
            if name.lower().endswith(".prep.md"):
                yield course, "prep", path
            elif rule.match(name):
                yield course, "lecture", path


def run_check(cli: str, path: str, timeout: int = 900):
    started = time.time()
    proc = subprocess.run([cli, "check", path], capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
    text = (proc.stdout or "").strip()
    try:
        start = text.index("{")
        value = json.loads(text[start:])
    except (ValueError, json.JSONDecodeError):
        value = {"ok": False, "error": (proc.stderr or text or f"exit {proc.returncode}").strip()[:600]}
    value["seconds"] = round(time.time() - started, 1)
    value["exit"] = proc.returncode
    return value


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--cli", default=DEFAULT_CLI)
    parser.add_argument("--semester", default=DEFAULT_SEMESTER)
    parser.add_argument("--course", action="append", default=[])
    parser.add_argument("--skip-sealed", action="store_true", help="skip lectures whose guide already exists")
    args = parser.parse_args(argv)
    if not os.path.isfile(args.cli):
        print(f"ERROR: CLI not found: {args.cli}", file=sys.stderr)
        return 2
    failures = 0
    rows = []
    for course, kind, path in targets(args.semester, args.course):
        name = os.path.basename(path)
        if kind == "lecture" and args.skip_sealed:
            stem = re.sub(r"\.[^.]+$", "", name)
            guide = os.path.join(os.path.dirname(path), re.sub(r"[^A-Za-z0-9._-]+", "_", stem) + "_Guide.md")
            if os.path.isfile(guide):
                rows.append((course, kind, name, "skipped", "guide exists", 0))
                continue
        value = run_check(args.cli, path)
        if value.get("ok"):
            drift = value.get("predecessor_drift") or []
            status = "ok" if not drift else "ok (drift)"
            detail = "; ".join(drift) if drift else ""
        else:
            error = str(value.get("error", ""))
            if "refusing to overwrite existing guide" in error:
                status, detail = "existing", "guide exists (expected refusal)"
            elif "Another Guide Watcher process is already" in error:
                status, detail = "busy", "a guide is running in this course; re-run the sweep when it finishes"
            elif "refusing to overwrite existing prep packet" in error:
                status, detail = "existing", "prep packet exists (expected refusal; resume the .prep.md instead)"
            else:
                status, detail = "FAIL", error.replace("\n", " | ")[:300]
                failures += 1
        rows.append((course, kind, name, status, detail, value.get("seconds", 0)))
        print(f"[{status:>10}] {course} / {kind} / {name}  ({value.get('seconds', 0)}s)" + (f"\n             {detail}" if detail else ""))
    print()
    print(f"{len(rows)} target(s), {failures} failure(s)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
