#!/usr/bin/env python3
"""Make a portable copy of a study guide with every figure embedded as a base64 data URI.

The copy renders on any machine in any local Markdown viewer (VS Code preview, Markdown
Preview Enhanced, Silk MD, Typora, Obsidian, ...) with no assets folder beside it. Nothing
else in the file changes: text, headings, tables, the app's completion receipt, and line
endings are preserved byte for byte outside the image lines.

Usage
  python embed_guide_images.py <guide.md> [more guides ...] [--out <file>] [--allow-missing]
  python embed_guide_images.py            # no arguments: opens a file picker (several files allowed)

Each `<Name>_Guide.md` becomes `<Name>_Guide_portable.md` in the same folder (or --out for a
single input). Existing portable copies are overwritten. Images are looked up relative to the
guide's folder; a missing image stops the run and lists every missing path unless
--allow-missing is given, in which case those links are left as they are and reported.

Not embedded: remote images (http/https), images that are already data URIs.
"""
from __future__ import annotations

import argparse
import base64
import mimetypes
import os
import re
import sys
from urllib.parse import unquote

# The target may contain balanced parentheses (`..._(1)_Guide_assets/x.png`), per CommonMark.
IMAGE_LINE = re.compile(
    r"^(?P<prefix>\s*)!\[(?P<alt>[^\]]*)\]\((?P<target>(?:[^()\s]|\([^()\s]*\))+)(?P<title>\s+\"[^\"]*\")?\)(?P<suffix>\s*)$"
)
PORTABLE_SUFFIX = "_portable.md"
MIME_BY_EXT = {
    ".png": "image/png",
    ".jpg": "image/jpeg",
    ".jpeg": "image/jpeg",
    ".gif": "image/gif",
    ".webp": "image/webp",
    ".svg": "image/svg+xml",
}


class EmbedResult:
    def __init__(self):
        self.embedded = 0
        self.skipped_remote = 0
        self.already_embedded = 0
        self.missing = []       # (line_number, target)
        self.bytes_embedded = 0


def _mime_for(path: str) -> str:
    ext = os.path.splitext(path)[1].lower()
    return MIME_BY_EXT.get(ext) or mimetypes.guess_type(path)[0] or "application/octet-stream"


def embed_images(text: str, base_dir: str, allow_missing: bool = False) -> tuple[str, EmbedResult]:
    """Return (new_text, result). Only standalone image lines are rewritten, which is the
    guides' contract; image syntax inside code fences is left alone."""
    result = EmbedResult()
    newline = "\r\n" if "\r\n" in text else "\n"
    lines = text.split(newline)
    in_code = False
    out = []
    for number, line in enumerate(lines, 1):
        if line.lstrip().startswith("```"):
            in_code = not in_code
            out.append(line)
            continue
        match = None if in_code else IMAGE_LINE.match(line)
        if not match:
            out.append(line)
            continue
        target = match.group("target")
        lower = target.lower()
        if lower.startswith(("http://", "https://")):
            result.skipped_remote += 1
            out.append(line)
            continue
        if lower.startswith("data:"):
            result.already_embedded += 1
            out.append(line)
            continue
        relative = unquote(target)
        path = relative if os.path.isabs(relative) else os.path.join(base_dir, relative)
        if not os.path.isfile(path):
            result.missing.append((number, target))
            out.append(line)
            continue
        with open(path, "rb") as handle:
            data = handle.read()
        encoded = base64.b64encode(data).decode("ascii")
        result.embedded += 1
        result.bytes_embedded += len(data)
        out.append(
            f"{match.group('prefix')}![{match.group('alt')}](data:{_mime_for(path)};base64,{encoded}"
            f"{match.group('title') or ''}){match.group('suffix')}"
        )
    if result.missing and not allow_missing:
        return text, result
    return newline.join(out), result


def portable_path(guide_path: str) -> str:
    stem, ext = os.path.splitext(guide_path)
    if stem.endswith("_portable"):
        return guide_path
    return f"{stem}{PORTABLE_SUFFIX}" if ext.lower() == ".md" else f"{guide_path}{PORTABLE_SUFFIX}"


def convert(guide_path: str, out_path: str | None = None, allow_missing: bool = False) -> tuple[str, EmbedResult]:
    guide_path = os.path.abspath(guide_path)
    if not os.path.isfile(guide_path):
        raise FileNotFoundError(guide_path)
    with open(guide_path, "rb") as handle:
        raw = handle.read()
    text = raw.decode("utf-8")
    new_text, result = embed_images(text, os.path.dirname(guide_path), allow_missing)
    if result.missing and not allow_missing:
        return "", result
    target = os.path.abspath(out_path) if out_path else portable_path(guide_path)
    # A portable copy is a derived file and may be refreshed in place; an original never is.
    if os.path.abspath(target) == guide_path and not guide_path.lower().endswith(PORTABLE_SUFFIX):
        raise ValueError("refusing to overwrite the original guide; choose a different --out")
    temp = target + ".tmp"
    with open(temp, "wb") as handle:
        handle.write(new_text.encode("utf-8"))
    os.replace(temp, target)
    return target, result


def _pick_files() -> list[str]:
    try:
        import tkinter as tk
        from tkinter import filedialog
    except ImportError:
        return []
    root = tk.Tk()
    root.withdraw()
    paths = filedialog.askopenfilenames(
        title="Choose study guide(s) to make portable",
        filetypes=[("Markdown guides", "*.md"), ("All files", "*.*")],
    )
    root.destroy()
    return list(paths)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("guides", nargs="*", help="guide .md files (omit to open a file picker)")
    parser.add_argument("--out", help="output path (only with exactly one input)")
    parser.add_argument("--allow-missing", action="store_true", help="write the copy even if some images are missing")
    args = parser.parse_args(argv)
    guides = args.guides or _pick_files()
    if not guides:
        print("No guide selected.", file=sys.stderr)
        return 2
    if args.out and len(guides) != 1:
        print("--out needs exactly one input guide", file=sys.stderr)
        return 2
    failures = 0
    for guide in guides:
        try:
            target, result = convert(guide, args.out, args.allow_missing)
        except (FileNotFoundError, ValueError, UnicodeDecodeError) as exc:
            print(f"ERROR {guide}: {exc}", file=sys.stderr)
            failures += 1
            continue
        if not target:
            failures += 1
            print(f"ERROR {guide}: {len(result.missing)} image(s) missing; nothing written (use --allow-missing to write anyway):", file=sys.stderr)
            for number, missing in result.missing:
                print(f"   line {number}: {missing}", file=sys.stderr)
            continue
        size_mb = os.path.getsize(target) / 1e6
        note = ""
        if result.missing:
            note += f"; {len(result.missing)} missing image(s) left as links"
        if result.skipped_remote:
            note += f"; {result.skipped_remote} remote image(s) left as links"
        print(f"{target}\n   {result.embedded} image(s) embedded ({result.bytes_embedded/1e6:.1f} MB of image data), file is {size_mb:.1f} MB{note}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
