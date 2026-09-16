import base64
import os
import shutil
import tempfile
import unittest

import embed_guide_images as eg

PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg=="
)


class EmbedGuideImages(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="gw-embed-")
        os.makedirs(os.path.join(self.dir, "G_Guide_assets"))
        with open(os.path.join(self.dir, "G_Guide_assets", "a b.png"), "wb") as handle:
            handle.write(PNG)
        self.guide = os.path.join(self.dir, "G_Guide.md")
        self.text = "\n".join([
            "# Guide",
            "",
            "![alt text](G_Guide_assets/a%20b.png)",
            "",
            "*Figure: A.*",
            "**What to notice:** x.",
            "",
            "```md",
            "![not an image line](G_Guide_assets/a%20b.png)",
            "```",
            "",
            "![remote](https://example.com/x.png)",
            "",
            "<!-- guide-watcher:complete:v3 bundle_id=x -->",
            "",
        ])
        with open(self.guide, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(self.text)

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_embeds_local_images_and_leaves_everything_else_untouched(self):
        target, result = eg.convert(self.guide)
        self.assertEqual(os.path.join(self.dir, "G_Guide_portable.md"), target)
        self.assertEqual(1, result.embedded)
        self.assertEqual(1, result.skipped_remote)
        self.assertEqual([], result.missing)
        out = open(target, encoding="utf-8").read()
        expected = "![alt text](data:image/png;base64," + base64.b64encode(PNG).decode() + ")"
        self.assertIn(expected, out)
        self.assertIn("![not an image line](G_Guide_assets/a%20b.png)", out)  # inside a code fence
        self.assertIn("![remote](https://example.com/x.png)", out)
        self.assertIn("<!-- guide-watcher:complete:v3 bundle_id=x -->", out)
        # everything except the one image line is identical
        self.assertEqual(
            [l for l in self.text.split("\n") if not l.startswith("![alt")],
            [l for l in out.split("\n") if not l.startswith("![alt")],
        )
        # the original is untouched and the copy is idempotent
        self.assertEqual(self.text, open(self.guide, encoding="utf-8").read())
        again, result2 = eg.convert(target)
        self.assertEqual(target, again)
        self.assertEqual(0, result2.embedded)
        self.assertEqual(1, result2.already_embedded)

    def test_missing_images_stop_the_run_unless_allowed(self):
        os.remove(os.path.join(self.dir, "G_Guide_assets", "a b.png"))
        target, result = eg.convert(self.guide)
        self.assertEqual("", target)
        self.assertEqual([(3, "G_Guide_assets/a%20b.png")], result.missing)
        self.assertFalse(os.path.exists(os.path.join(self.dir, "G_Guide_portable.md")))
        target, result = eg.convert(self.guide, allow_missing=True)
        self.assertTrue(os.path.exists(target))
        self.assertIn("![alt text](G_Guide_assets/a%20b.png)", open(target, encoding="utf-8").read())

    def test_crlf_guides_keep_their_line_endings_and_out_cannot_clobber_the_original(self):
        with open(self.guide, "w", encoding="utf-8", newline="\r\n") as handle:
            handle.write(self.text)
        target, result = eg.convert(self.guide)
        raw = open(target, "rb").read()
        self.assertIn(b"\r\n", raw)
        self.assertNotIn(b"\r\r\n", raw)
        with self.assertRaises(ValueError):
            eg.convert(self.guide, out_path=self.guide)

    def test_paths_with_parentheses_are_embedded(self):
        os.makedirs(os.path.join(self.dir, "CH02_(1)_Guide_assets"))
        with open(os.path.join(self.dir, "CH02_(1)_Guide_assets", "v.png"), "wb") as handle:
            handle.write(PNG)
        guide = os.path.join(self.dir, "CH02_(1)_Guide.md")
        with open(guide, "w", encoding="utf-8", newline="\n") as handle:
            handle.write("# G\n\n![v](CH02_(1)_Guide_assets/v.png)\n\n*Figure: v.*\n**What to notice:** v.\n")
        target, result = eg.convert(guide)
        self.assertEqual(1, result.embedded)
        self.assertEqual([], result.missing)
        self.assertIn("![v](data:image/png;base64,", open(target, encoding="utf-8").read())

    def test_cli_reports_and_exits_nonzero_on_failure(self):
        self.assertEqual(0, eg.main([self.guide]))
        self.assertEqual(1, eg.main([os.path.join(self.dir, "nope.md")]))
        self.assertEqual(2, eg.main([self.guide, self.guide, "--out", "x.md"]))


if __name__ == "__main__":
    unittest.main()
