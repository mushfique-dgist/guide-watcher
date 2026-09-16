import io
import json
import os
import shutil
import tempfile
import unittest

import supplementary_collect as sc


def fake_fetchers(youtube=None, url=None, sleeps=None):
    sleeps = sleeps if sleeps is not None else []
    return sc.Fetchers(
        youtube=youtube or (lambda vid, langs, timeout: sc.FetchResult("no_captions", "none")),
        url=url or (lambda u, timeout: sc.FetchResult("unavailable", "HTTP 404")),
        sleep=sleeps.append,
    )


def candidate(cid="remzi-day1", kind="youtube", url="https://www.youtube.com/watch?v=LVxN7ZkGh3w", **extra):
    record = {"id": cid, "kind": kind, "url": url, "title": "Day 1", "source": "CS537", "topics": ["processes"]}
    record.update(extra)
    return record


class CandidateValidation(unittest.TestCase):
    def test_youtube_ids_are_extracted_from_every_common_url_shape(self):
        for url in [
            "https://www.youtube.com/watch?v=LVxN7ZkGh3w",
            "https://youtu.be/LVxN7ZkGh3w",
            "https://www.youtube.com/shorts/LVxN7ZkGh3w",
            "https://youtube.com/watch?v=LVxN7ZkGh3w&list=PL123&t=10s",
        ]:
            self.assertEqual("LVxN7ZkGh3w", sc.youtube_video_id(url), url)
        for url in ["https://vimeo.com/123", "ftp://youtu.be/LVxN7ZkGh3w", "https://www.youtube.com/watch?v=short"]:
            self.assertIsNone(sc.youtube_video_id(url), url)

    def test_malformed_candidates_are_reported_not_fetched(self):
        self.assertIsNone(sc.validate_candidate(candidate()))
        self.assertIn("id must match", sc.validate_candidate(candidate(cid="../escape")))
        self.assertIn("id must match", sc.validate_candidate(candidate(cid="Upper Case")))
        self.assertIn("kind", sc.validate_candidate(candidate(kind="podcast")))
        self.assertIn("http", sc.validate_candidate(candidate(url="file:///etc/passwd")))
        self.assertIn("YouTube", sc.validate_candidate(candidate(url="https://example.com/x")))
        self.assertIn("title", sc.validate_candidate(candidate(title="  ")))
        self.assertEqual("candidate is not an object", sc.validate_candidate("nope"))


class Rendering(unittest.TestCase):
    def test_captions_group_into_stamped_paragraphs_and_drop_noise(self):
        snippets = [
            {"text": "[Music]", "start": 0.0, "duration": 5.0},
            {"text": "welcome to", "start": 5.0, "duration": 2.0},
            {"text": "the course", "start": 7.0, "duration": 2.0},
            {"text": "next minute", "start": 70.0, "duration": 2.0},
            {"text": "an hour in", "start": 3700.0, "duration": 2.0},
        ]
        text = sc.render_caption_paragraphs(snippets)
        self.assertEqual(
            "[00:05] welcome to the course\n\n[01:10] next minute\n\n[1:01:40] an hour in\n", text
        )
        self.assertEqual("", sc.render_caption_paragraphs([]))

    def test_html_is_reduced_to_text_without_scripts(self):
        page = "<html><head><style>x{}</style><script>evil()</script></head><body><h1>Title</h1><p>One &amp; two.</p><p>Three.</p></body></html>"
        self.assertEqual("Title\nOne & two.\nThree.\n", sc.html_to_text(page))


class Collection(unittest.TestCase):
    def setUp(self):
        self.course = tempfile.mkdtemp(prefix="gw-supp-")

    def tearDown(self):
        shutil.rmtree(self.course, ignore_errors=True)

    def test_every_candidate_gets_a_status_and_ok_ones_are_stored_atomically(self):
        def youtube(vid, langs, timeout):
            if vid == "LVxN7ZkGh3w":
                return sc.FetchResult("ok", "2 snippets (en)", "[00:00] hello world\n", duration_seconds=90)
            return sc.FetchResult("no_captions", "TranscriptsDisabled")

        def url(u, timeout):
            if "ocw" in u:
                return sc.FetchResult("ok", "60 words", "MITOCW transcript " + "word " * 60)
            return sc.FetchResult("unavailable", "HTTP 404")

        outcomes = sc.collect(self.course, [
            candidate(),
            candidate(cid="no-caps", url="https://youtu.be/AAAAAAAAAAA"),
            candidate(cid="ocw-page", kind="url", url="https://ocw.mit.edu/x", title="OCW"),
            candidate(cid="gone", kind="url", url="https://example.com/gone", title="Gone"),
            candidate(cid="../bad"),
            candidate(),  # duplicate id
        ], fetchers=fake_fetchers(youtube, url))
        self.assertEqual(outcomes[0].status, "ok")
        self.assertEqual(outcomes[1].status, "no_captions")
        self.assertEqual(outcomes[2].status, "ok")
        self.assertEqual(outcomes[3].status, "unavailable")
        self.assertEqual(outcomes[4].status, "invalid")
        self.assertEqual(outcomes[5].status, "invalid")
        self.assertIn("duplicate", outcomes[5].detail)

        transcript = io.open(os.path.join(self.course, "_supplementary", "remzi-day1", "transcript.md"), encoding="utf-8").read()
        self.assertTrue(transcript.startswith("# Day 1\n\n- id: remzi-day1\n- kind: youtube\n"))
        self.assertIn("- duration_seconds: 90", transcript)
        self.assertTrue(transcript.endswith("---\n\n[00:00] hello world\n"))
        self.assertFalse(os.path.exists(os.path.join(self.course, "_supplementary", "no-caps")))
        self.assertFalse(any(name.startswith(".tmp-") for name in os.listdir(os.path.join(self.course, "_supplementary"))))

        manifest = json.load(io.open(os.path.join(self.course, "_supplementary", "manifest.json"), encoding="utf-8"))
        ids = [item["id"] for item in manifest["items"]]
        self.assertEqual(["gone", "no-caps", "ocw-page", "remzi-day1"], ids)
        ok = next(item for item in manifest["items"] if item["id"] == "remzi-day1")
        self.assertEqual("ok", ok["status"])
        self.assertEqual("_supplementary/remzi-day1/transcript.md", ok["file"])
        self.assertEqual(64, len(ok["sha256"]))
        self.assertEqual([], ok["lecture_map"])
        self.assertFalse(ok["audited"])
        failed = next(item for item in manifest["items"] if item["id"] == "no-caps")
        self.assertEqual("no_captions", failed["status"])
        self.assertNotIn("file", failed)

        report = io.open(os.path.join(self.course, "_supplementary", "collection_report.md"), encoding="utf-8").read()
        for needle in ["| remzi-day1 | ok |", "| no-caps | no_captions |", "| gone | unavailable |", "| ../bad | invalid |"]:
            self.assertIn(needle, report)
        self.assertIn("invalid 2", report)

    def test_blocked_and_network_errors_retry_with_backoff_then_report(self):
        calls = []
        sleeps = []

        def youtube(vid, langs, timeout):
            calls.append(vid)
            return sc.FetchResult("blocked", "IpBlocked", retryable=True)

        outcomes = sc.collect(self.course, [candidate()], fetchers=fake_fetchers(youtube, sleeps=sleeps), retries=2)
        self.assertEqual(3, len(calls))
        self.assertEqual([2.0, 4.0], sleeps)
        self.assertEqual("blocked", outcomes[0].status)
        # A non-retryable failure is not retried.
        calls.clear()
        sc.collect(self.course, [candidate()], fetchers=fake_fetchers(lambda v, l, t: sc.FetchResult("unavailable", "gone")), retries=2)
        self.assertEqual(0, len(calls))

    def test_rerun_merges_the_manifest_and_keeps_audit_fields(self):
        ok = lambda v, l, t: sc.FetchResult("ok", "x", "[00:00] text\n", duration_seconds=10)
        sc.collect(self.course, [candidate()], fetchers=fake_fetchers(ok))
        path = os.path.join(self.course, "_supplementary", "manifest.json")
        manifest = json.load(io.open(path, encoding="utf-8"))
        manifest["items"][0]["lecture_map"] = ["2-What_is_OS"]
        manifest["items"][0]["audited"] = True
        io.open(path, "w", encoding="utf-8").write(json.dumps(manifest))
        sc.collect(self.course, [candidate(), candidate(cid="second", url="https://youtu.be/BBBBBBBBBBB")], fetchers=fake_fetchers(ok))
        manifest = json.load(io.open(path, encoding="utf-8"))
        first = next(item for item in manifest["items"] if item["id"] == "remzi-day1")
        self.assertEqual(["2-What_is_OS"], first["lecture_map"])
        self.assertTrue(first["audited"])
        self.assertEqual(2, len(manifest["items"]))

    def test_cli_strict_mode_fails_when_anything_did_not_succeed(self):
        plan = os.path.join(self.course, "plan.json")
        io.open(plan, "w", encoding="utf-8").write(json.dumps({"candidates": [candidate(cid="no-caps", url="https://youtu.be/AAAAAAAAAAA")]}))
        original = sc.REAL_FETCHERS
        sc.REAL_FETCHERS = fake_fetchers()
        try:
            sc.collect.__defaults__ = (sc.REAL_FETCHERS,) + sc.collect.__defaults__[1:]
            self.assertEqual(2, sc.main(["collect", self.course, "--candidates", plan, "--strict"]))
            self.assertEqual(0, sc.main(["collect", self.course, "--candidates", plan]))
        finally:
            sc.collect.__defaults__ = (original,) + sc.collect.__defaults__[1:]
            sc.REAL_FETCHERS = original
        self.assertEqual(0, sc.main(["status", self.course]))
        self.assertEqual(0, sc.main(["plan-template"]))


if __name__ == "__main__":
    unittest.main()
