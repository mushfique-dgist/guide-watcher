#!/usr/bin/env python3
"""Carry one machine's Guide Watcher settings into a newer copy of the source.

Guide Watcher is configured in Rust source, not in a settings file: the semester folders, the
course list, and the paths to the automation scripts and external tools are constants. A user who
installed an earlier build and adapted it to their own computer must not lose that when they take
a newer build.

This script copies exactly the parts that describe *their computer* from an older checkout into a
newer one, and reports everything else that differs so a person or an AI assistant can decide.
It never guesses about product decisions such as model names or pinned versions.

    python scripts/port_local_settings.py --old <old checkout>            # report only
    python scripts/port_local_settings.py --old <old checkout> --apply    # write the changes

Always finish with `cargo test --manifest-path src-tauri/Cargo.toml --all-targets`: if the newer
build changed the shape of the course list, the compiler says so and the remaining edit is small
and obvious.
"""
from __future__ import annotations

import argparse
import io
import json
import os
import re
import sys

# Constants that describe this computer rather than the product. Matched by suffix, plus a few
# names that are user identity rather than a path.
PATH_SUFFIXES = ("_ROOT", "_DIR", "_FILE", "_SCRIPT", "_EXECUTABLE", "_PATH", "_CLI")
NAMED_LOCAL = {"DEFAULT_COURSE_PROFILE", "CIRCUIT_LAB_COURSE_ID", "WATCH_DIR"}
# Whole regions that describe the user's courses.
REGIONS = [
    ("src-tauri/src/config.rs", "course_profiles: vec![", "]"),
    ("src-tauri/src/course_plan.rs", "let specifications = [", "]"),
]
CONST_FILES = ["src-tauri/src/config.rs", "src-tauri/src/course_plan.rs"]
CONST = re.compile(r'pub(?:\(crate\))? const ([A-Z0-9_]+): &str =\s*("(?:[^"\\]|\\.)*")\s*;', re.S)


def read(path: str) -> str:
    with io.open(path, encoding="utf-8") as handle:
        return handle.read()


def write(path: str, text: str) -> None:
    with io.open(path, "w", encoding="utf-8", newline="\n") as handle:
        handle.write(text)


def looks_local(name: str, value: str) -> bool:
    """Whether a constant describes this computer rather than the product."""
    if name in NAMED_LOCAL:
        return True
    if name.endswith(PATH_SUFFIXES):
        return True
    unquoted = value[1:-1]
    return bool(re.match(r"^[a-zA-Z]:[\\/]", unquoted)) or unquoted.startswith("/")


def constants(text: str) -> dict[str, str]:
    return {match.group(1): match.group(2) for match in CONST.finditer(text)}


def region(text: str, anchor: str, closer: str) -> tuple[int, int] | None:
    """The span of a bracketed region that starts at `anchor`, matching brackets so nested
    entries and strings containing brackets cannot end it early."""
    start = text.find(anchor)
    if start < 0:
        return None
    opener = anchor[-1]
    index = start + len(anchor) - 1
    depth = 0
    in_string = False
    escaped = False
    while index < len(text):
        character = text[index]
        if in_string:
            if escaped:
                escaped = False
            elif character == "\\":
                escaped = True
            elif character == '"':
                in_string = False
        elif character == '"':
            in_string = True
        elif character == opener:
            depth += 1
        elif character == closer:
            depth -= 1
            if depth == 0:
                return start, index + 1
        index += 1
    return None


def port(old_root: str, new_root: str, apply: bool) -> dict:
    report = {"carried": [], "review": [], "missing": [], "applied": bool(apply)}
    for relative in CONST_FILES:
        old_path, new_path = os.path.join(old_root, relative), os.path.join(new_root, relative)
        if not (os.path.isfile(old_path) and os.path.isfile(new_path)):
            report["missing"].append(relative)
            continue
        old_text, new_text = read(old_path), read(new_path)
        old_constants, new_constants = constants(old_text), constants(new_text)
        updated = new_text
        for name, new_value in new_constants.items():
            old_value = old_constants.get(name)
            if old_value is None or old_value == new_value:
                continue
            entry = {"file": relative, "name": name,
                     "yours": json.loads(old_value), "shipped": json.loads(new_value)}
            if looks_local(name, old_value):
                pattern = re.compile(
                    r'(pub(?:\(crate\))? const ' + re.escape(name) + r': &str =\s*)'
                    + re.escape(new_value) + r'(\s*;)', re.S)
                updated, count = pattern.subn(lambda m: m.group(1) + old_value + m.group(2), updated, count=1)
                if count:
                    report["carried"].append(entry)
                else:
                    report["review"].append({**entry, "why": "could not be rewritten automatically"})
            else:
                report["review"].append({**entry, "why": "not a path: decide whether this is yours or a product change"})
        for relative_region, anchor, closer in REGIONS:
            if relative_region != relative:
                continue
            old_span, new_span = region(old_text, anchor, closer), region(updated, anchor, closer)
            if not old_span or not new_span:
                report["review"].append({"file": relative, "name": anchor,
                                         "why": "the course list was not found in both copies; port it by hand"})
                continue
            old_block, new_block = old_text[old_span[0]:old_span[1]], updated[new_span[0]:new_span[1]]
            if old_block.strip() == new_block.strip():
                continue
            if old_block.count("(") != old_block.count(")"):
                report["review"].append({"file": relative, "name": anchor, "why": "unbalanced course list"})
                continue
            updated = updated[:new_span[0]] + old_block + updated[new_span[1]:]
            report["carried"].append({"file": relative, "name": anchor,
                                      "yours": f"{old_block.count('(')} entries", "shipped": f"{new_block.count('(')} entries",
                                      "note": "your course list replaced the shipped one; if the newer build added a field, the compiler will point at it"})
        if apply and updated != new_text:
            write(new_path, updated)
    return report


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--old", required=True, help="the checkout that is configured for this computer")
    parser.add_argument("--new", default=os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                        help="the newer checkout to update (default: the one holding this script)")
    parser.add_argument("--apply", action="store_true", help="write the changes instead of only reporting them")
    arguments = parser.parse_args(argv)
    if os.path.abspath(arguments.old) == os.path.abspath(arguments.new):
        print("ERROR: --old and --new are the same directory.", file=sys.stderr)
        return 2
    report = port(arguments.old, arguments.new, arguments.apply)
    print(json.dumps(report, indent=2, ensure_ascii=False))
    verb = "Carried" if arguments.apply else "Would carry"
    print(f"\n{verb} {len(report['carried'])} local setting(s). "
          f"{len(report['review'])} difference(s) need a decision. "
          f"{len(report['missing'])} file(s) missing.", file=sys.stderr)
    if not arguments.apply:
        print("Re-run with --apply to write these changes.", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
