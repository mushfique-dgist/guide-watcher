"""Formatting checks for study guides (see guide_format_standard.md).

The checks live in guide_lint.py (the app freezes that single file as the verifier); this
module re-exports them for tests and direct use:  python guide_format.py <guide.md>
"""
import importlib.util
import os
import sys

_LINT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "guide_lint.py")
_spec = importlib.util.spec_from_file_location("guide_lint_for_format", _LINT)
_lint = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_lint)

format_must_fix = _lint.format_must_fix
format_advisory = _lint.format_advisory
MAX_PARAGRAPH_WORDS = _lint.MAX_PARAGRAPH_WORDS


def render(text):
    out = []
    for title, items in format_must_fix(text):
        out.append(f"[MUST FIX] {title} ({len(items)})")
        out.extend(f"   {item}" for item in items)
    for title, items in format_advisory(text):
        out.append(f"[ADVISORY] {title} ({len(items)})")
        out.extend(f"   {item}" for item in items)
    return "\n".join(out)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print(__doc__)
        sys.exit(2)
    with open(sys.argv[1], encoding="utf-8") as handle:
        report = render(handle.read())
    print(report if report else "[FORMAT] nothing to fix")
