"""
render_slides.py — Slide rasterizer for the guide-watcher verification phase.
================================================================================
Renders every page or slide of a PDF, PPTX, or DOCX source to PNG images so the model can VISUALLY
inspect figures (graphs, diagrams, plots) instead of guessing from extracted
text. Text extraction silently drops arrowheads, edge-label positions, colors,
and spatial layout — exactly the information that makes a diagram correct. The
guide-watcher launchers call this BEFORE generation so the images are guaranteed
to exist and the generation prompt can point the model straight at them.

Usage:
    python render_slides.py <lecture.pdf|slides.pptx|handout.docx> <output_dir> [--zoom 2.0]
                            [--max-edge 1568] [--force]
                            [--max-output-bytes N] [--max-pages N]
                            [--max-pixel-work N]

Behavior:
    - Writes <output_dir>/slide_01.png, slide_02.png, ... (zero-padded).
    - Idempotent: skips pages whose PNG already exists unless --force.
    - PDF pages are rasterized directly.
    - On Windows, PPTX and DOCX are exported to a temporary PDF through the
      installed Microsoft Office application with macro automation disabled,
      then rasterized. The temporary PDF is removed.
    - HTML and plain-text inputs exit 0 with a notice.
    - Enforces caller-supplied cumulative page, encoded-byte, and output-pixel
      budgets before committing each PNG; a failed invocation removes every
      PNG or partial file that it created.
    - Exit 0 on success (or nothing-to-do), exit 1 on a real failure.

Dimension cap (why this exists):
    The legacy Claude-only pipeline fed every PNG to one request and hit a
    2000px many-image boundary. Current Guide Watcher inspects frozen renders
    with GPT-5.6 Sol in bounded 12-image batches. The renderer still guarantees
    that no PNG's longer side exceeds --max-edge (default 1568px), both to bound
    local decoded memory and to remain safe across writer fallbacks. The cap
    only LOWERS the effective zoom; small pages keep the requested zoom.

Requires: pymupdf  (pip install pymupdf)
"""

import os
import sys
import argparse
import tempfile

# Longest side (px) any emitted PNG may have. Retained as a cross-provider and
# decoded-memory boundary with headroom for metadata and rounding.
DEFAULT_MAX_EDGE = 1568
DEFAULT_MAX_OUTPUT_BYTES = 1024 * 1024 * 1024
DEFAULT_MAX_PAGES = 512
DEFAULT_MAX_PIXEL_WORK = 512 * 1024 * 1024


def _require_budget(name, value):
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise ValueError(f"{name} must be a non-negative integer")
    return value


def _capped_pixmap(page, zoom, max_edge):
    """Rasterize `page` at `zoom`, lowering the effective zoom only as needed so
    neither side exceeds `max_edge` px.

    We RE-RENDER at the reduced zoom rather than downscaling an oversized raster:
    re-rendering keeps text and thin diagram strokes crisp, while post-hoc
    resampling would blur them. A page already small enough at `zoom` is left
    exactly as requested — the cap never upscales."""
    import fitz  # PyMuPDF

    rect = page.rect
    long_pts = max(rect.width, rect.height) or 1.0  # mediabox longer side, in points
    eff = zoom
    if max_edge and long_pts * eff > max_edge:
        eff = max_edge / long_pts

    pix = page.get_pixmap(matrix=fitz.Matrix(eff, eff))

    # Defensive second pass: integer pixel rounding, rotation, or an odd
    # crop/mediabox can still land a hair over the cap. Recompute from the ACTUAL
    # raster size so the guarantee holds no matter how the estimate drifted.
    if max_edge:
        longest = max(pix.width, pix.height)
        if longest > max_edge:
            eff *= max_edge / longest
            pix = page.get_pixmap(matrix=fitz.Matrix(eff, eff))
    return pix


def render_pdf(
    pdf_path,
    out_dir,
    zoom=2.0,
    force=False,
    max_edge=DEFAULT_MAX_EDGE,
    max_output_bytes=DEFAULT_MAX_OUTPUT_BYTES,
    max_pages=DEFAULT_MAX_PAGES,
    max_pixel_work=DEFAULT_MAX_PIXEL_WORK,
    page_numbers=None,
):
    """Render each page of pdf_path to out_dir/slide_NN.png. Returns list of paths.

    No emitted PNG's longer side exceeds `max_edge` px (set max_edge=0 to disable
    the cap, e.g. for callers that are not sending the images to a vision model)."""
    try:
        import fitz  # PyMuPDF
    except ImportError:
        print("ERROR: pymupdf (fitz) is required:  pip install pymupdf", file=sys.stderr)
        sys.exit(1)

    try:
        doc = fitz.open(pdf_path)
    except Exception as e:  # noqa: BLE001 - surface any open failure to the caller
        print(f"ERROR: could not open PDF '{pdf_path}': {e}", file=sys.stderr)
        sys.exit(1)

    max_output_bytes = _require_budget("max_output_bytes", max_output_bytes)
    max_pages = _require_budget("max_pages", max_pages)
    max_pixel_work = _require_budget("max_pixel_work", max_pixel_work)
    n = doc.page_count
    if page_numbers is None:
        selected_page_numbers = list(range(1, n + 1))
    else:
        selected_page_numbers = list(page_numbers)
        if not selected_page_numbers:
            doc.close()
            raise ValueError("page_numbers must contain at least one page")
        if selected_page_numbers != sorted(set(selected_page_numbers)):
            doc.close()
            raise ValueError("page_numbers must be unique and sorted")
        if selected_page_numbers[0] < 1 or selected_page_numbers[-1] > n:
            doc.close()
            raise ValueError(f"page_numbers must be within 1..{n}")
    if len(selected_page_numbers) > max_pages:
        doc.close()
        raise RuntimeError(
            f"selected source pages exceed the remaining renderer page budget {max_pages}"
        )

    os.makedirs(out_dir, exist_ok=True)
    width = max(2, len(str(n)))  # zero-pad width tracks the page count
    written, created_this_run, partials, skipped = [], [], [], 0
    output_bytes = 0
    pixel_work = 0
    try:
        for page_number in selected_page_numbers:
            i = page_number - 1
            out = os.path.join(out_dir, f"slide_{page_number:0{width}d}.png")
            if os.path.exists(out) and not force:
                existing_bytes = os.path.getsize(out)
                existing = fitz.Pixmap(out)
                existing_work = existing.width * existing.height
                if output_bytes + existing_bytes > max_output_bytes:
                    raise RuntimeError("existing renders exceed the remaining output-byte budget")
                if pixel_work + existing_work > max_pixel_work:
                    raise RuntimeError("existing renders exceed the remaining pixel-work budget")
                output_bytes += existing_bytes
                pixel_work += existing_work
                skipped += 1
                written.append(out)
                continue

            pix = _capped_pixmap(doc[i], zoom, max_edge)
            page_work = pix.width * pix.height
            if pixel_work + page_work > max_pixel_work:
                raise RuntimeError("source renders exceed the remaining pixel-work budget")
            png_bytes = pix.tobytes("png")
            if output_bytes + len(png_bytes) > max_output_bytes:
                raise RuntimeError("source renders exceed the remaining output-byte budget")

            partial = out + ".partial"
            partials.append(partial)
            with open(partial, "xb") as handle:
                handle.write(png_bytes)
                handle.flush()
                os.fsync(handle.fileno())
            if os.path.getsize(partial) != len(png_bytes):
                raise RuntimeError(f"renderer output changed size while saving: {out}")
            os.replace(partial, out)
            partials.remove(partial)
            created_this_run.append(out)
            output_bytes += len(png_bytes)
            pixel_work += page_work
            if output_bytes > max_output_bytes or pixel_work > max_pixel_work:
                raise RuntimeError("renderer exceeded its output budget after saving")
            written.append(out)
    except Exception:
        for path in partials + created_this_run:
            try:
                os.unlink(path)
            except FileNotFoundError:
                pass
        raise
    finally:
        doc.close()
    cap_note = f", capped at {max_edge}px/side" if max_edge else ""
    print(f"render_slides: {len(written)} page(s) -> {out_dir}{cap_note}"
          + (f" ({skipped} already present, skipped)" if skipped else ""))
    return written


def _export_office_pdf(source_path, pdf_path):
    """Export a PPTX or DOCX to PDF through installed Office, with macros disabled."""
    try:
        import pythoncom
        import win32com.client
    except ImportError as exc:
        raise RuntimeError("PPTX/DOCX rendering requires pywin32 and Microsoft Office") from exc

    ext = os.path.splitext(source_path)[1].lower()
    pythoncom.CoInitialize()
    app = None
    document = None
    try:
        if ext == ".pptx":
            app = win32com.client.DispatchEx("PowerPoint.Application")
            app.AutomationSecurity = 3  # msoAutomationSecurityForceDisable
            document = app.Presentations.Open(os.path.abspath(source_path), ReadOnly=True, WithWindow=False)
            document.SaveAs(os.path.abspath(pdf_path), 32)  # ppSaveAsPDF
        elif ext == ".docx":
            app = win32com.client.DispatchEx("Word.Application")
            app.Visible = False
            app.DisplayAlerts = 0
            app.AutomationSecurity = 3
            document = app.Documents.Open(
                os.path.abspath(source_path),
                ConfirmConversions=False,
                ReadOnly=True,
                AddToRecentFiles=False,
            )
            document.ExportAsFixedFormat(os.path.abspath(pdf_path), 17)  # wdExportFormatPDF
        else:
            raise RuntimeError(f"unsupported Office source: {ext}")
    finally:
        if document is not None:
            try:
                document.Close(False)
            except Exception:
                pass
        if app is not None:
            try:
                app.Quit()
            except Exception:
                pass
        pythoncom.CoUninitialize()


def render_source(
    source_path,
    out_dir,
    zoom=2.0,
    force=False,
    max_edge=DEFAULT_MAX_EDGE,
    max_output_bytes=DEFAULT_MAX_OUTPUT_BYTES,
    max_pages=DEFAULT_MAX_PAGES,
    max_pixel_work=DEFAULT_MAX_PIXEL_WORK,
    page_numbers=None,
):
    """Render a supported source without leaving conversion artifacts behind."""
    ext = os.path.splitext(source_path)[1].lower()
    if ext == ".pdf":
        return render_pdf(
            source_path,
            out_dir,
            zoom=zoom,
            force=force,
            max_edge=max_edge,
            max_output_bytes=max_output_bytes,
            max_pages=max_pages,
            max_pixel_work=max_pixel_work,
            page_numbers=page_numbers,
        )
    if ext in {".pptx", ".docx"}:
        os.makedirs(out_dir, exist_ok=True)
        fd, converted = tempfile.mkstemp(prefix="guide-watcher-office-", suffix=".pdf", dir=out_dir)
        os.close(fd)
        try:
            os.unlink(converted)
            _export_office_pdf(source_path, converted)
            return render_pdf(
                converted,
                out_dir,
                zoom=zoom,
                force=force,
                max_edge=max_edge,
                max_output_bytes=max_output_bytes,
                max_pages=max_pages,
                max_pixel_work=max_pixel_work,
                page_numbers=page_numbers,
            )
        finally:
            try:
                os.unlink(converted)
            except FileNotFoundError:
                pass
    print(f"render_slides: '{os.path.basename(source_path)}' has no raster page renderer; nothing to render.")
    return []


def main():
    parser = argparse.ArgumentParser(description="Render lecture/course pages to PNG images.")
    parser.add_argument("source", help="Path to a PDF, PPTX, or DOCX source")
    parser.add_argument("out_dir", help="Directory to write slide_NN.png images into")
    parser.add_argument("--zoom", type=float, default=2.0,
                        help="Desired rasterization zoom factor (the longer side is still "
                             "clamped to --max-edge, so this is an upper bound on density)")
    parser.add_argument("--max-edge", type=int, default=DEFAULT_MAX_EDGE,
                        help=f"Cap (px) for the longer side of every PNG so many-image "
                             f"requests stay under Anthropic's 2000px limit "
                             f"(default {DEFAULT_MAX_EDGE}; 0 disables the cap)")
    parser.add_argument("--force", action="store_true", help="Re-render even if PNGs already exist")
    parser.add_argument(
        "--page-numbers",
        help="Optional comma-separated, unique, sorted 1-based PDF pages to render",
    )
    parser.add_argument("--max-output-bytes", type=int, default=DEFAULT_MAX_OUTPUT_BYTES,
                        help="Maximum cumulative PNG bytes permitted for this invocation")
    parser.add_argument("--max-pages", type=int, default=DEFAULT_MAX_PAGES,
                        help="Maximum page count permitted for this invocation")
    parser.add_argument("--max-pixel-work", type=int, default=DEFAULT_MAX_PIXEL_WORK,
                        help="Maximum cumulative output pixels permitted for this invocation")
    args = parser.parse_args()

    if not os.path.isfile(args.source):
        print(f"ERROR: source file not found: {args.source}", file=sys.stderr)
        sys.exit(1)

    try:
        page_numbers = None
        if args.page_numbers is not None:
            page_numbers = [int(value) for value in args.page_numbers.split(",")]
        render_source(
            args.source,
            args.out_dir,
            zoom=args.zoom,
            force=args.force,
            max_edge=args.max_edge,
            max_output_bytes=args.max_output_bytes,
            max_pages=args.max_pages,
            max_pixel_work=args.max_pixel_work,
            page_numbers=page_numbers,
        )
    except Exception as exc:  # noqa: BLE001 - surface Office/renderer failures to caller
        print(f"ERROR: could not render '{args.source}': {exc}", file=sys.stderr)
        sys.exit(1)
    sys.exit(0)


if __name__ == "__main__":
    main()
