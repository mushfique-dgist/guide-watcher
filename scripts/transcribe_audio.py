"""Create a bounded, timestamped transcript for a captured Circuit Lab video."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path

from faster_whisper import WhisperModel


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--audio", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--model", required=True)
    parser.add_argument("--video-index", required=True, type=int)
    parser.add_argument("--expected-duration", required=True, type=float)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    audio = args.audio.resolve(strict=True)
    output = args.output.resolve(strict=False)
    if not audio.is_file() or not 0 < audio.stat().st_size <= 512 * 1024 * 1024:
        raise SystemExit("captured audio is missing, empty, or exceeds 512 MiB")
    if output.exists():
        raise SystemExit("refusing to overwrite an existing transcript")
    if args.video_index < 1 or not 0 < args.expected_duration <= 14_400:
        raise SystemExit("invalid video index or expected duration")

    model = WhisperModel(args.model, device="auto", compute_type="default")
    raw_segments, info = model.transcribe(
        str(audio),
        beam_size=5,
        vad_filter=True,
        condition_on_previous_text=True,
    )
    segments = []
    for index, segment in enumerate(raw_segments, start=1):
        text = " ".join(segment.text.split())
        if not text:
            continue
        start = max(0.0, float(segment.start))
        end = min(args.expected_duration, float(segment.end))
        if end <= start:
            continue
        segments.append(
            {
                "id": f"v{args.video_index:02}-seg-{len(segments) + 1:04}",
                "start_seconds": round(start, 3),
                "end_seconds": round(end, 3),
                "text": text,
            }
        )
    if not segments:
        raise SystemExit("speech transcription produced no usable segments")

    document = {
        "schema_version": 1,
        "language": info.language or "und",
        "duration_seconds": args.expected_duration,
        "segments": segments,
    }
    payload = (json.dumps(document, ensure_ascii=False, indent=2) + "\n").encode("utf-8")
    if len(payload) > 4 * 1024 * 1024:
        raise SystemExit("transcript exceeds the 4 MiB publication limit")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    descriptor = os.open(output, flags, 0o600)
    with os.fdopen(descriptor, "wb") as handle:
        handle.write(payload)
        handle.flush()
        os.fsync(handle.fileno())


if __name__ == "__main__":
    main()
