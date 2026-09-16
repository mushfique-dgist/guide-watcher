import unittest

import guide_format as gf


class MustFix(unittest.TestCase):
    def test_slide_narration_and_reading_instructions_are_caught_but_citations_are_not(self):
        text = "\n".join([
            "# 1. Topic",
            "",
            "Lecture PDF page 22 returns to the intuition slide with the two questions.",
            "",
            "The proof has three parts (CLRS, PDF page 49). This is a citation, not narration.",
            "",
            "First, read Chapter 2, Section 2.1 Insertion sort at PDF pages 44-49.",
            "",
            "## 21.3 Questions to Bring to Class",
        ])
        blocks = dict(gf.format_must_fix(text))
        self.assertIn("SLIDE NARRATION IN THE BODY", blocks)
        self.assertEqual(1, len(blocks["SLIDE NARRATION IN THE BODY"]))
        self.assertIn("line 3", blocks["SLIDE NARRATION IN THE BODY"][0])
        self.assertIn("READING INSTRUCTIONS IN THE BODY", blocks)
        self.assertIn("line 7", blocks["READING INSTRUCTIONS IN THE BODY"][0])
        self.assertIn("SECTIONS FOR ADMINISTRATIVE SLIDES", blocks)

    def test_clean_body_has_nothing_to_fix(self):
        text = "# 1. Topic\n\n## 1.1 Idea\n\nInsertion sort keeps a sorted prefix (CLRS Section 2.1).\n\n> **💡 Pattern to recognize.** Invariants survive iterations.\n"
        self.assertEqual([], gf.format_must_fix(text))
        self.assertEqual([], gf.format_advisory(text))


class Advisory(unittest.TestCase):
    def test_bare_expressions_are_flagged_unless_in_code_or_math(self):
        bare = "# 1. T\n\n## 1.1 S\n\nValues smaller than 2: one, so p = 1. The scan shifts k − p = 3 values and exits with j = p − 1 = 0.\n"
        blocks = dict(gf.format_advisory(bare))
        self.assertIn("BARE EXPRESSIONS IN PROSE", blocks)
        coded = "# 1. T\n\n## 1.1 S\n\nValues smaller than `2`: one, so `p = 1`. The scan shifts `k − p = 3` values and exits with `j = p − 1 = 0`.\n"
        self.assertNotIn("BARE EXPRESSIONS IN PROSE", dict(gf.format_advisory(coded)))
        math = "# 1. T\n\n## 1.1 S\n\nThe sum is $\\sum i = n(n+1)/2$ and the bound is $T(n) = O(n^2)$.\n"
        self.assertNotIn("BARE EXPRESSIONS IN PROSE", dict(gf.format_advisory(math)))

    def test_long_paragraphs_examples_and_practice_structure(self):
        long = "# 1. T\n\n## 1.1 S\n\n" + "word " * 130 + "\n"
        self.assertIn("PARAGRAPHS OVER 120 WORDS", dict(gf.format_advisory(long)))
        bad_example = "# 1. T\n\n## 1.1 S\n\n**Worked example (medium)**. Take the array and shift twice, giving the result.\n"
        self.assertIn("EXAMPLES WITHOUT THE SKELETON", dict(gf.format_advisory(bad_example)))
        for lead in ["**A harder example — the counter is not just plus one (hard).**", "**Medium — compute the two numbers.**", "**A lost-update trace (medium), under the model.**"]:
            text = "# 1. T\n\n## 1.1 S\n\n" + lead + " Prose without the skeleton.\n"
            self.assertIn("EXAMPLES WITHOUT THE SKELETON", dict(gf.format_advisory(text)), lead)
        self.assertNotIn("EXAMPLES WITHOUT THE SKELETON", dict(gf.format_advisory("# 1. T\n\n## 1.1 S\n\n**Definition.** A process is a running program.\n")))
        good_example = "\n".join([
            "# 1. T", "", "## 1.1 S", "",
            "> **🧮 Worked example (medium) — one insertion**", ">",
            "> **Given.** `L = [1, 3]`, `x = 2`", ">",
            "> **Steps.**", "> 1. compare `3 > 2`, shift", "> 2. place `2` in cell `1`", ">",
            "> **Result.** `[1, 2, 3]`", "> **Check.** `2 + 1 = 3` cells ✔", "",
        ])
        self.assertNotIn("EXAMPLES WITHOUT THE SKELETON", dict(gf.format_advisory(good_example)))
        old_practice = "# 20. Exam-Style Practice\n\n**Q1 (medium).** What is 2 + 2?\n\n*Solution.* 4.\n"
        self.assertIn("PRACTICE ITEMS WITHOUT STRUCTURE", dict(gf.format_advisory(old_practice)))
        new_practice = "\n".join([
            "# 20. Exam-Style Practice", "", "### Q1 (medium) — arithmetic", "",
            "> What is `2 + 2`?", "", "**Solution.** **4.**", "", "- `2 + 2 = 4`", "", "---", "",
        ])
        self.assertNotIn("PRACTICE ITEMS WITHOUT STRUCTURE", dict(gf.format_advisory(new_practice)))

    def test_sections_without_substructure_are_flagged(self):
        flat = "# 3. Flat\n\n" + ("word " * 120 + "\n\n") * 2 + "# 4. Structured\n\n## 4.1 Part\n\n" + "word " * 250 + "\n"
        blocks = dict(gf.format_advisory(flat))
        self.assertIn("SECTIONS WITHOUT SUB-STRUCTURE", blocks)
        self.assertEqual(1, len(blocks["SECTIONS WITHOUT SUB-STRUCTURE"]))
        self.assertIn("'3. Flat'", blocks["SECTIONS WITHOUT SUB-STRUCTURE"][0])


if __name__ == "__main__":
    unittest.main()
