"""Adversarial regression tests for the canonical guide completion gate.

Run directly with ``python test_guide_lint.py`` after installing the pinned
verifier dependency from ``requirements-verifier.txt``.
"""

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import zlib


HERE = Path(__file__).resolve().parent
LINT = HERE / "guide_lint.py"
SENTINEL = "<!-- guide-watcher:complete:v1 -->"


def rgba_png(width=1, height=1, value=0):
    def chunk(kind, payload):
        return len(payload).to_bytes(4, "big") + kind + payload + (zlib.crc32(kind + payload) & 0xFFFFFFFF).to_bytes(4, "big")
    ihdr = width.to_bytes(4, "big") + height.to_bytes(4, "big") + bytes([8, 6, 0, 0, 0])
    raw = b"".join(b"\x00" + bytes([value, value, value, 255]) * width for _ in range(height))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


def _load_lint_module():
    spec = importlib.util.spec_from_file_location("guide_lint_contract", LINT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class GuideLintContractTests(unittest.TestCase):
    def test_technical_limitations_are_not_writer_self_correction(self):
        lint = _load_lint_module()
        for text in (
            "An abstraction hides messy mechanism behind a simple interface.",
            "What addresses can and cannot tell you.",
            "A process cannot read another process's private memory.",
            "The CPU cannot tell which application owns a virtual address alone.",
            "We cannot tell which process will run next because scheduling is nondeterministic.",
            "We cannot see another process address space without kernel assistance.",
            "I cannot read another process's private memory from this process.",
            "A process may hold on to a lock while waiting for I/O.",
        ):
            with self.subTest(text=text):
                self.assertEqual(lint.find_matches(text, lint.RAMBLING_PATTERNS, prose_only=True), [])

    def test_drilling_is_reported_but_a_genuine_escalation_is_left_alone(self):
        lint = _load_lint_module()

        def example(difficulty, filler=0):
            return (
                f"> **\U0001f9ee Worked example ({difficulty}) \u2014 one instance**\n>\n"
                f"> **Given.** An instance. {'word ' * filler}\n>\n"
                "> **Steps.** Work it through.\n>\n> **Result.** A number.\n>\n"
                "> **Check.** It adds up.\n\n"
            )

        def guide(title, body):
            return f"# 1. A Major Section\n\n## {title}\n\nProse that teaches the idea.\n\n{body}"

        def findings(text):
            return lint.advisory_example_overload(text)

        # Two examples is the working shape, at any length.
        self.assertEqual(findings(guide("1.1 Capability or Choice",
                                        example("medium") + example("harder"))), [])

        # Three that escalate through a substantial subsection are a ladder, not a drill.
        ladder = example("medium", 420) + example("harder", 420) + example("challenging", 420)
        self.assertEqual(findings(guide("1.1 Translating an Address", ladder)), [])

        # Three short ones are the same exercise three times.
        thin = example("medium") + example("harder") + example("challenging")
        items = findings(guide("1.1 Capability or Choice", thin))
        self.assertEqual(1, len(items), items)
        self.assertIn("short examples", items[0])
        self.assertIn("Pattern to recognize", items[0])

        # Four of one idea is padding whatever they contain.
        padded = ladder + example("challenging", 420)
        items = findings(guide("1.1 Translating an Address", padded))
        self.assertTrue(any("4 examples of one idea" in item for item in items), items)

        # A difficulty claimed twice is an escalation that was never made.
        repeated = example("harder", 420) + example("harder", 420) + example("medium", 420)
        items = findings(guide("1.1 Two Tasks on One Array", repeated))
        self.assertEqual(1, len(items), items)
        self.assertIn("'harder' used more than once", items[0])

        # A title that counts its examples is the clearest symptom, even at two.
        items = findings(guide("1.1 Three Worked Examples on Mechanism Versus Policy",
                               example("medium", 420) + example("harder", 420)))
        self.assertEqual(1, len(items), items)
        self.assertIn("counts its examples", items[0])
        # A subsection number is not a count.
        self.assertEqual(findings(guide("3.2 Worked Example \u2014 One Instruction",
                                        example("medium", 420))), [])

        # Volume belongs in the exam-style practice section, so it is exempt.
        exam = "# 9. Exam-Style Practice\n\n## 9.1 Questions\n\nProse.\n\n" + thin * 2
        self.assertEqual(findings(exam), [])

        # The report wires the advisory in under its own heading.
        self.assertIn("SECTIONS THAT DRILL ONE IDEA",
                      lint.render_pedagogy_advisory(guide("1.1 Capability or Choice", thin)))

    def test_writer_extraction_failures_still_fail(self):
        lint = _load_lint_module()
        for text in (
            "I cannot tell which direction the arrow points.",
            "We could not extract the diagram.",
            "Unable to read the slide labels.",
            "The layout is too messy to interpret.",
            "Wait, let me re-extract the PDF.",
            "Hold on, that trace is wrong.",
        ):
            with self.subTest(text=text):
                self.assertTrue(lint.find_matches(text, lint.RAMBLING_PATTERNS, prose_only=True))

    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory()
        self.root = Path(self.temp_dir.name)
        self.source = self.root / "lecture.pdf"
        self.source.write_bytes(b"immutable source bytes")
        self.guide = self.root / "Lecture_Guide.md"
        self.verify = self.root / ".Lecture_Guide.md.gwverify"
        self.verify.mkdir()
        self.assets = self.root / "Lecture_Guide_assets"
        self.assets.mkdir()

        digest = hashlib.sha256(self.source.read_bytes()).hexdigest()
        self.original_hash = digest
        source_identity = str(self.source.resolve()).replace("\\", "/")
        if os.name == "nt":
            source_identity = source_identity.lower()
        self.source_id = "source-" + hashlib.sha256(source_identity.encode("utf-8")).hexdigest()
        self.unit_ids = [f"{self.source_id}-unit-{number:03d}" for number in range(1, 4)]
        self.snapshot = {
            "schema_version": 2,
            "primary_source_id": self.source_id,
            "sources": [{
                "id": self.source_id,
                "path": str(self.source.resolve()),
                "name": self.source.name,
                "sha256": digest,
                "size_bytes": self.source.stat().st_size,
                "role": "primary",
                "unit_kind": "slide",
                "unit_count": 3,
                "unit_ids": self.unit_ids,
            }],
            "unit_ids": self.unit_ids,
        }
        self._write_json(self.verify / "source_snapshot.json", self.snapshot)
        self._write_json(
            self.verify / "predecessor_manifest.json",
            {"schema_version": 1, "prior_guides": []},
        )
        rendered = self.verify / "sources" / self.source_id / "renders"
        rendered.mkdir(parents=True)
        for number in range(1, 4):
            (rendered / f"slide_{number:02d}.png").write_bytes(b"render")

        self.coverage = {
            "schema_version": 2,
            "guide": self.guide.name,
            "guide_kind": "lecture",
            "primary_source_id": self.source_id,
            "sources": [
                {
                    "id": self.source_id,
                    "path": str(self.source.resolve()),
                    "sha256": digest,
                    "role": "primary",
                }
            ],
            "units": [
                {"id": self.unit_ids[0], "source_id": self.source_id, "guide_anchor": "1-foundations", "topic": "The first idea"},
                {"id": self.unit_ids[1], "source_id": self.source_id, "guide_anchor": "1-foundations", "topic": "The second idea"},
                {"id": self.unit_ids[2], "source_id": self.source_id, "guide_anchor": "2-applications", "topic": "The third idea"},
            ],
            "continuity": {
                "prerequisites": ["basic notation"],
                "prior_guides": [],
                "next_bridge": "Use these foundations in the next lecture.",
            },
            "source_conflicts": [],
            "unresolved_gaps": [],
            "visual_plan": {
                "required": True,
                "rationale": "A source figure needs a faithful annotated explanation.",
            },
            "verification_plan": {
                "required": False,
                "rationale": "This small fixture contains no recomputable claims.",
            },
        }
        self._write_json(self.verify / "coverage_manifest.json", self.coverage)
        self._write_valid_asset()
        self._write_valid_guide()

    def tearDown(self):
        self.temp_dir.cleanup()

    @staticmethod
    def _write_json(path: Path, value: object) -> None:
        path.write_text(json.dumps(value, indent=2), encoding="utf-8")

    def _write_valid_asset(self) -> None:
        image = self.assets / "annotated-source.png"
        image.write_bytes(rgba_png())
        packet_sha = "c" * 64
        owned_asset = {
            "id": "annotated-source",
            "filename": "annotated-source.png",
            "sha256": hashlib.sha256(image.read_bytes()).hexdigest(),
            "spec_sha256": "b" * 64,
            "width_px": 1,
            "height_px": 1,
            "kind": "annotated-source",
            "evidence_class": "source",
            "need_ids": ["need-foundation"],
            "source_unit_ids": [self.unit_ids[0]],
            "procedure_step_ids": [],
            "learning_purpose": "Show the source forwarding decision",
            "alt": "Annotated packet flow with the forwarding decision highlighted",
            "caption": "Annotated source figure showing the decision boundary.",
            "explanation": "Follow the highlighted arrow before comparing the two outcomes.",
            "provenance": {
                "type": "annotated-source",
                "source": "lecture.pdf",
                "locator": "slide 2",
                "transformation": "Cropped and annotated; labels and values unchanged.",
            },
            "rights": {
                "basis": "course-provided",
                "reuse_scope": "private-study",
                "attribution": "Course lecture",
            },
        }
        self._write_json(self.verify / "visual_contract.json", {
            "schema_version": 2,
            "packet_sha256": packet_sha,
            "decision": "purposeful-visuals",
            "needs": [{
                "id": "need-foundation",
                "kind": "concept",
                "source_unit_ids": [self.unit_ids[0]],
                "procedure_step_ids": [],
                "learner_question": "Where is the forwarding decision?",
                "misconception_prevented": "Confusing ingress with forwarding",
                "evidence_requirement": "source",
            }],
            "procedure_steps": [],
            "assets": [owned_asset],
        })
        asset_manifest = {
            "schema_version": 2,
            "visual_packet_sha256": packet_sha,
            "assets": [
                {
                    **owned_asset,
                    "path": "Lecture_Guide_assets/annotated-source.png",
                    "section_anchor": "1-foundations",
                }
            ],
        }
        self._write_json(self.assets / "asset_manifest.json", asset_manifest)

    def _write_valid_guide(self, include_marker: bool = False) -> None:
        marker = f"\n{SENTINEL}\n" if include_marker else "\n"
        self.guide.write_text(
            "# Lecture foundations\n\n"
            "# 1. Foundations\n\n"
            "The prerequisite is introduced before the mechanism and connected to prior knowledge.\n\n"
            "![Annotated packet flow with the forwarding decision highlighted]"
            "(Lecture_Guide_assets/annotated-source.png)\n\n"
            "*Figure: Annotated source figure showing the decision boundary.*\n\n"
            "**What to notice:** Follow the highlighted arrow before comparing the two outcomes.\n\n"
            "# 2. Applications\n\n"
            "The later idea uses the foundation explicitly and closes with a forward bridge.\n"
            + marker,
            encoding="utf-8",
        )

    def _update_asset_digest(self) -> None:
        image = self.assets / "annotated-source.png"
        digest = hashlib.sha256(image.read_bytes()).hexdigest()
        manifest_path = self.assets / "asset_manifest.json"
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        manifest["assets"][0]["sha256"] = digest
        self._write_json(manifest_path, manifest)
        contract_path = self.verify / "visual_contract.json"
        contract = json.loads(contract_path.read_text(encoding="utf-8"))
        contract["assets"][0]["sha256"] = digest
        self._write_json(contract_path, contract)

    def _configure_valid_circuit(self) -> None:
        self.coverage["guide_kind"] = "circuit-lab"
        self.coverage["lab_steps"] = [{
            "id": "step-meter-range",
            "kind": "physical",
            "guide_anchor": "1-foundations",
            "need_ids": ["need-foundation"],
            "goal": "Select the safe meter range.",
            "action": "Turn the range control before connecting probes.",
            "expected": "The range indicator matches the planned measurement.",
            "wrong": "The meter remains in current mode.",
            "recovery": "Disconnect, select voltage mode, and verify the display.",
            "primary_visual_asset_id": "annotated-source",
            "visual_evidence": [{"asset_id": "annotated-source", "role": "action"}],
        }]
        self._write_json(self.verify / "coverage_manifest.json", self.coverage)
        for path in (self.assets / "asset_manifest.json", self.verify / "visual_contract.json"):
            value = json.loads(path.read_text(encoding="utf-8"))
            value["assets"][0]["procedure_step_ids"] = ["step-meter-range"]
            if path.name == "visual_contract.json":
                value["needs"][0]["kind"] = "procedure-step"
                value["needs"][0]["procedure_step_ids"] = ["step-meter-range"]
                value["procedure_steps"] = [{
                    "id": "step-meter-range",
                    "action": "Turn the range control before connecting probes.",
                    "kind": "physical",
                    "guide_anchor": "1-foundations",
                    "need_ids": ["need-foundation"],
                    "source_unit_ids": [self.unit_ids[0]],
                    "required_evidence": ["action"],
                }]
            self._write_json(path, value)

    def _run(self, *extra: str) -> subprocess.CompletedProcess[str]:
        command = [
            sys.executable,
            str(LINT),
            str(self.guide),
            "--verify-dir",
            str(self.verify),
            "--source",
            str(self.source),
            "--expected-source-sha",
            self.original_hash,
        ]
        if "--seal" not in extra:
            command.append("--no-harness")
        command.extend(extra)
        return subprocess.run(
            command,
            capture_output=True,
            text=True,
            cwd=self.root,
        )

    def test_valid_contract_passes(self):
        result = self._run()
        self.assertEqual(0, result.returncode, result.stdout + result.stderr)

    def test_shared_visual_text_contract_matches_rust_preflight_vectors(self):
        module = _load_lint_module()
        fixture = json.loads(
            (HERE / "visual_text_contract_cases.json").read_text(encoding="utf-8")
        )
        for group, accepted in (("accepted", True), ("rejected", False)):
            for case in fixture[group]:
                actual = module._canonical_visual_text(
                    case["value"], case["max_bytes"]
                )
                self.assertEqual(
                    actual == case["value"], accepted, msg=repr(case["value"])
                )

    def test_lecture_can_use_app_owned_no_purposeful_visual_rationale(self):
        self.coverage["visual_plan"] = {
            "required": False,
            "rationale": "The source is a prose-only policy notice with no spatial relationship to teach.",
        }
        self._write_json(self.verify / "coverage_manifest.json", self.coverage)
        packet_sha = "d" * 64
        self._write_json(self.verify / "visual_contract.json", {
            "schema_version": 2,
            "packet_sha256": packet_sha,
            "decision": "no-purposeful-visual",
            "no_visuals_rationale": "The source is prose-only and a picture would be decorative.",
            "needs": [],
            "procedure_steps": [],
            "assets": [],
        })
        self._write_json(self.assets / "asset_manifest.json", {
            "schema_version": 2,
            "visual_packet_sha256": packet_sha,
            "assets": [],
        })
        (self.assets / "annotated-source.png").unlink()
        self.guide.write_text(
            "# Lecture foundations\n\n# 1. Foundations\n\nThe prose policy is explained in full.\n\n"
            "# 2. Applications\n\nThe implications and forward bridge are explained.\n",
            encoding="utf-8",
        )
        result = self._run()
        self.assertEqual(0, result.returncode, result.stdout + result.stderr)

    def test_expected_guide_kind_rejects_model_selected_kind(self):
        result = self._run("--expected-guide-kind", "circuit-lab")
        self.assertNotEqual(0, result.returncode)
        self.assertIn("GUIDE KIND MISMATCH", result.stdout)

    def test_fixed_arithmetic_verifier_accepts_rounding_and_rejects_wrong_results(self):
        module = _load_lint_module()
        self.assertEqual([], module.check_arithmetic_equalities("1 / 3 = 0.333\n4 × 5 = 20"))
        failures = module.check_arithmetic_equalities("4 × 5 = 19")
        self.assertEqual(1, len(failures))
        self.assertIn("evaluates to 20", failures[0])

    def test_pedagogy_advisory_targets_real_gaps_and_stays_quiet_on_good_guides(self):
        module = _load_lint_module()
        good = "\n".join([
            "# 1. Bridge",
            "",
            "A **process** is a running program.",
            "",
            "# 2. Scheduling",
            "",
            "Round robin rotates.",
            "",
            "> **🧮 Worked example (medium) — two jobs**",
            ">",
            "> **Given.** `q = 2`",
            ">",
            "> **Steps.**",
            "> 1. run `A`",
            ">",
            "> **Result.** `[A, B]`",
            "> **Check.** `1 + 1 = 2` ✔",
            "",
            "![a](x_assets/a.png)",
            "",
            "*Figure: A.*",
            "**What to notice:** the gap.",
            "",
            "In the figure the gap at t=3 is the switch.",
            "",
            "# 3. Where This Leads Next",
            "",
            "next.",
        ])
        self.assertEqual("", module.render_pedagogy_advisory(good))
        # A bare bold example marker still counts as a worked example for the pedagogy check
        # (and is then reported by the formatting check for lacking the skeleton).
        plain = "# 1. Bridge\n\n## 1.1 Idea\n\n" + "word " * 210 + "\n\n**Example 1 (Medium): two jobs**\n\ntrace here\n"
        report = module.render_pedagogy_advisory(plain)
        self.assertNotIn("SECTIONS WITHOUT A WORKED EXAMPLE", report)
        self.assertIn("EXAMPLES WITHOUT THE SKELETON", report)

        bad = "\n".join([
            "# 1. Bridge",
            "",
            "The scheduler consults the PCB and picks a process.",
            "",
            "# 2. Data Structures",
            "",
            "The **PCB** is the process control block.",
            "",
            "![a](x_assets/a.png)",
            "",
            "*Figure: A.*",
            "**What to notice:** the gap.",
            "",
            "![b](x_assets/b.png)",
            "",
            "*Figure: B.*",
            "**What to notice:** the field.",
            "",
            "# 3. Context Switch",
            "",
            "Registers are saved.",
        ])
        report = module.render_pedagogy_advisory(bad)
        self.assertIn("SECTIONS WITHOUT A WORKED EXAMPLE", report)
        self.assertIn("'2. Data Structures'", report)
        self.assertIn("'3. Context Switch'", report)
        self.assertIn("TERMS USED BEFORE THEY ARE DEFINED", report)
        self.assertIn("'PCB' is used at line 3", report)
        # A tagged early mention is a deliberate deferral, not a gap.
        tagged = bad.replace("consults the PCB and", "consults the PCB (defined in Section 2) and")
        self.assertNotIn("'PCB' is used", module.render_pedagogy_advisory(tagged))
        self.assertIn("FIGURES NOT WOVEN INTO PROSE", report)
        self.assertIn("followed by another figure", report)
        self.assertIn("followed by a heading or rule", report)

    def test_draft_self_check_reports_hard_failures_before_teaching_gaps(self):
        module = _load_lint_module()
        draft = "\n".join([
            "# T",
            "",
            "# 1. Bridge",
            "",
            "Wait, let me redo this. The sum is 4 + 5 = 10 and ![the figure above] shows it.",
            "",
            "```c",
            "int x;",
        ])
        report = module.render_pedagogy_advisory(draft, require_exam_practice=True)
        order = [line for line in report.splitlines() if line.startswith("[")]
        self.assertTrue(order[0].startswith("[MUST FIX]"), order)
        self.assertIn("[MUST FIX] ARITHMETIC EQUALITY (1)", report)
        self.assertIn("[MUST FIX] UNBALANCED CODE FENCES", report)
        self.assertIn("[MUST FIX] RAMBLING / VISIBLE SELF-CORRECTION", report)
        self.assertIn("[MUST FIX] UNSUPPORTED IMAGE SYNTAX (1)", report)
        self.assertIn("[MUST FIX] EXAM PRACTICE SECTION (1)", report)
        self.assertEqual("", module.render_pedagogy_advisory("# T\n\n# 1. Where This Leads Next\n\nfine.\n"))

    def test_exam_practice_gate_requires_a_named_section_with_solved_questions(self):
        module = _load_lint_module()
        self.assertTrue(module.check_exam_practice("# 1. Intro\n\ntext\n"))
        thin = "# 1. Intro\n\n# 2. Exam-Style Practice\n\n**Q1 (medium).** a?\n\n*Solution.* b\n"
        failures = module.check_exam_practice(thin)
        self.assertEqual(1, len(failures))
        self.assertIn("has 1 questions", failures[0])
        full = "# 1. Intro\n\n# 2. Exam-Style Practice\n\n" + "".join(
            f"**Q{i} (medium).** a?\n\n*Solution.* b\n\n" for i in range(1, 9)
        )
        self.assertEqual([], module.check_exam_practice(full))

    def test_arithmetic_verifier_reads_whole_chains_and_percentages(self):
        # Regressions from a real run: every one of these correct self-checks was flagged
        # because the tail of a chain was read alone, or a trailing percent was ignored.
        module = _load_lint_module()
        correct = "\n".join([
            "$0 + 1 + 2 + 3 + 4 + 5 = 15$",
            "$100 + 20 + 20 + 18 = 158$ B",
            "so $7 - 3 + 1 = 5$ TCP/IP layers",
            r"overhead = $58/158 = 36.7\%$",
            "efficiency 56/256 = 21.9%",
            "$158 \\times 8 = (160 - 2)\\times 8 = 1280 - 16 = 1264$",
            "precedence holds: 2 + 3 * 4 = 14",
            "header 36-28 = 8 bytes",
            "5 + -3 = 2 and -4 + 6 = 2 and 10 - -2 = 12",
            "2 * -3 + 10 = 4",
        ])
        self.assertEqual([], module.check_arithmetic_equalities(correct))
        # Genuinely wrong chains are still caught, with the whole expression named.
        failures = module.check_arithmetic_equalities("1 + 2 + 3 = 7")
        self.assertEqual(1, len(failures))
        self.assertIn("'1 + 2 + 3 = 7' evaluates to 6", failures[0])
        failures = module.check_arithmetic_equalities("2 + 3 * 4 = 20")
        self.assertIn("evaluates to 14", failures[0])
        # A wrong percentage is caught against the percentage, not the raw ratio.
        failures = module.check_arithmetic_equalities("1/4 = 30%")
        self.assertIn("evaluates to 25", failures[0])
        # Division by zero is reported, never crashes.
        failures = module.check_arithmetic_equalities("5 / 0 = 1")
        self.assertIn("could not recompute", failures[0])
        failures = module.check_arithmetic_equalities("36-28 = 9")
        self.assertIn("evaluates to 8", failures[0])
        # The Unicode minus is subtraction too: whole chains, not their ASCII tails.
        self.assertEqual([], module.check_arithmetic_equalities("shifts = 4 − 1 + 1 = 4 and p = 5 − 2 = 3"))
        self.assertIn("evaluates to 3", module.check_arithmetic_equalities("5 − 2 = 4")[0])
        # A leading sign belongs to the chain: −3 + 10 − 3 = 4 dB is correct as written.
        self.assertEqual([], module.check_arithmetic_equalities("net: −3 + 10 − 3 = 4 dB and -2 + 1 = −1"))
        # Hex literals are not products: `0x200000` must never be read as 0 times 200000.
        self.assertEqual([], module.check_arithmetic_equalities("VA 0x200000 = 2097152 and PA = 0x0C3A70 = 801392"))
        self.assertEqual([], module.check_arithmetic_equalities("3 x 4 = 12"))
        self.assertIn("evaluates to 12", module.check_arithmetic_equalities("3 x 4 = 13")[0])

    def test_app_owned_course_profile_enforces_dense_depth_without_output_arithmetic(self):
        rendered = self.verify / "sources" / self.source_id / "renders"
        for path in rendered.iterdir():
            path.unlink()
        self.unit_ids = [
            f"{self.source_id}-unit-{number:03d}" for number in range(1, 21)
        ]
        self.snapshot["sources"][0]["unit_count"] = len(self.unit_ids)
        self.snapshot["sources"][0]["unit_ids"] = self.unit_ids
        self.snapshot["unit_ids"] = self.unit_ids
        self.coverage["units"] = [
            {
                "id": unit_id,
                "source_id": self.source_id,
                "guide_anchor": "1-foundations",
                "topic": f"Bound source unit {number}",
            }
            for number, unit_id in enumerate(self.unit_ids, start=1)
        ]
        for number in range(1, 21):
            (rendered / f"slide_{number:02d}.png").write_bytes(b"render")
        self._write_json(self.verify / "source_snapshot.json", self.snapshot)
        self._write_json(self.verify / "coverage_manifest.json", self.coverage)
        self._write_valid_guide()

        legacy_heuristic = self._run()
        self.assertEqual(0, legacy_heuristic.returncode, legacy_heuristic.stdout)
        bound = self._run("--expected-course-profile", "computer-algorithms")
        self.assertNotEqual(0, bound.returncode)
        self.assertIn("DEPTH CONTRACT not met", bound.stdout)
        self.assertIn("minimum floor is 4800", bound.stdout)

    def test_false_verification_plan_rejects_extra_harness_without_executing_it(self):
        (self.verify / "verify.py").write_text(
            "raise RuntimeError('must never execute')\n", encoding="utf-8"
        )
        env = os.environ.copy()
        env["PATH"] = ""
        result = subprocess.run(
            [
                sys.executable,
                str(LINT),
                str(self.guide),
                "--verify-dir",
                str(self.verify),
                "--source",
                str(self.source),
                "--expected-source-sha",
                self.original_hash,
            ],
            capture_output=True,
            text=True,
            cwd=self.root,
            env=env,
        )
        self.assertNotEqual(0, result.returncode)
        self.assertIn("UNEXPECTED VERIFICATION HARNESS", result.stdout)
        self.assertIn("it was not executed", result.stdout)
        self.assertNotIn("Docker", result.stdout + result.stderr)

    def test_staged_candidate_can_use_final_guide_name_and_physical_asset_staging(self):
        staged_guide = self.root / ".Lecture_Guide.md.job.hybrid.tmp"
        staged_assets = self.root / ".Lecture_Guide_assets.job.tmp"
        staged_assets.mkdir()
        (staged_assets / "annotated-source.png").write_bytes(
            (self.assets / "annotated-source.png").read_bytes()
        )
        (staged_assets / "asset_manifest.json").write_bytes(
            (self.assets / "asset_manifest.json").read_bytes()
        )
        staged_guide.write_bytes(self.guide.read_bytes())

        result = subprocess.run(
            [
                sys.executable,
                str(LINT),
                str(staged_guide),
                "--verify-dir",
                str(self.verify),
                "--source",
                str(self.source),
                "--expected-source-sha",
                self.original_hash,
                "--expected-guide-name",
                self.guide.name,
                "--assets-dir",
                str(staged_assets),
                "--assets-prefix",
                self.assets.name,
                "--no-harness",
            ],
            capture_output=True,
            text=True,
            cwd=self.root,
        )
        self.assertEqual(0, result.returncode, result.stdout + result.stderr)

    def test_staged_asset_mapping_still_rejects_a_logical_path_escape(self):
        staged_assets = self.root / ".Lecture_Guide_assets.job.tmp"
        staged_assets.mkdir()
        (staged_assets / "asset_manifest.json").write_bytes(
            (self.assets / "asset_manifest.json").read_bytes()
        )
        escaped = self.guide.read_text(encoding="utf-8").replace(
            "Lecture_Guide_assets/annotated-source.png",
            "Other_assets/annotated-source.png",
        )
        self.guide.write_text(escaped, encoding="utf-8")

        result = subprocess.run(
            [
                sys.executable,
                str(LINT),
                str(self.guide),
                "--verify-dir",
                str(self.verify),
                "--source",
                str(self.source),
                "--expected-source-sha",
                self.original_hash,
                "--assets-dir",
                str(staged_assets),
                "--assets-prefix",
                self.assets.name,
                "--no-harness",
            ],
            capture_output=True,
            text=True,
            cwd=self.root,
        )
        self.assertNotEqual(0, result.returncode)
        self.assertIn("ASSET PATH ESCAPE", result.stdout)

    def test_missing_slide_coverage_fails(self):
        self.coverage["units"].pop()
        self._write_json(self.verify / "coverage_manifest.json", self.coverage)
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("MISSING SOURCE-UNIT COVERAGE", result.stdout)

    def test_duplicate_slide_coverage_fails(self):
        self.coverage["units"].append(dict(self.coverage["units"][0]))
        self._write_json(self.verify / "coverage_manifest.json", self.coverage)
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("DUPLICATE SOURCE-UNIT COVERAGE", result.stdout)

    def test_missing_embedded_asset_fails(self):
        (self.assets / "annotated-source.png").unlink()
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("MISSING LEARNER ASSET", result.stdout)

    def test_missing_asset_provenance_fails(self):
        manifest_path = self.assets / "asset_manifest.json"
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        del manifest["assets"][0]["provenance"]["locator"]
        self._write_json(manifest_path, manifest)
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("ASSET PROVENANCE", result.stdout)

    def test_asset_digest_or_dimensions_cannot_be_forged(self):
        manifest_path = self.assets / "asset_manifest.json"
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        manifest["assets"][0]["sha256"] = "0" * 64
        manifest["assets"][0]["width_px"] = 999
        self._write_json(manifest_path, manifest)
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("ASSET INTEGRITY", result.stdout)

    def test_png_crc_and_complete_zlib_stream_are_verified(self):
        image = self.assets / "annotated-source.png"
        corrupted = bytearray(image.read_bytes())
        corrupted[-13] ^= 1
        image.write_bytes(corrupted)
        self._update_asset_digest()
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("ASSET INTEGRITY", result.stdout)
        self.assertIn("CRC mismatch", result.stdout)

    def test_truncated_png_with_valid_signature_is_rejected(self):
        image = self.assets / "annotated-source.png"
        image.write_bytes(image.read_bytes()[:-7])
        self._update_asset_digest()
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("ASSET INTEGRITY", result.stdout)

    def test_valid_crc_ancillary_chunk_outside_app_png_profile_is_rejected(self):
        image = self.assets / "annotated-source.png"
        data = image.read_bytes()
        kind = b"tEXt"
        payload = b"Comment\x00not emitted by the deterministic renderer"
        chunk = (
            len(payload).to_bytes(4, "big")
            + kind
            + payload
            + (zlib.crc32(kind + payload) & 0xFFFFFFFF).to_bytes(4, "big")
        )
        image.write_bytes(data[:33] + chunk + data[33:])
        self._update_asset_digest()
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("ASSET INTEGRITY", result.stdout)

    def test_schema_one_asset_manifest_is_rejected(self):
        path = self.assets / "asset_manifest.json"
        manifest = json.loads(path.read_text(encoding="utf-8"))
        manifest["schema_version"] = 1
        self._write_json(path, manifest)
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("INVALID ASSET MANIFEST", result.stdout)

    def test_visual_manifest_png_markdown_bijection_rejects_duplicate_occurrence(self):
        text = self.guide.read_text(encoding="utf-8")
        image_block = (
            "![Annotated packet flow with the forwarding decision highlighted]"
            "(Lecture_Guide_assets/annotated-source.png)\n\n"
            "*Figure: Annotated source figure showing the decision boundary.*\n\n"
            "**What to notice:** Follow the highlighted arrow before comparing the two outcomes.\n\n"
        )
        self.guide.write_text(text + image_block, encoding="utf-8")
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("ASSET BIJECTION", result.stdout)

    def test_unselected_png_in_asset_directory_is_rejected(self):
        (self.assets / "decorative.png").write_bytes(rgba_png(value=90))
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("ASSET BIJECTION", result.stdout)

    def test_visual_must_occur_in_declared_and_source_mapped_section(self):
        path = self.assets / "asset_manifest.json"
        manifest = json.loads(path.read_text(encoding="utf-8"))
        manifest["assets"][0]["section_anchor"] = "2-applications"
        self._write_json(path, manifest)
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("IMAGE EXPLANATION", result.stdout)
        self.assertIn("VISUAL SOURCE-SECTION EVIDENCE", result.stdout)

    def test_circuit_physical_step_requires_bound_annotated_source_primary(self):
        self._configure_valid_circuit()
        self.coverage["lab_steps"][0]["primary_visual_asset_id"] = "missing-primary"
        self._write_json(self.verify / "coverage_manifest.json", self.coverage)
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("CIRCUIT LAB VISUAL MAPPING", result.stdout)

        self._configure_valid_circuit()
        result = self._run()
        self.assertEqual(0, result.returncode, result.stdout + result.stderr)

    def test_model_cannot_relabel_omit_or_add_app_owned_circuit_steps(self):
        self._configure_valid_circuit()
        self.coverage["lab_steps"][0]["kind"] = "conceptual"
        self._write_json(self.verify / "coverage_manifest.json", self.coverage)
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("APP-OWNED CIRCUIT STEP INVENTORY", result.stdout)

        self.coverage["lab_steps"] = []
        self._write_json(self.verify / "coverage_manifest.json", self.coverage)
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("APP-OWNED CIRCUIT STEP INVENTORY", result.stdout)

        self._configure_valid_circuit()
        extra = dict(self.coverage["lab_steps"][0])
        extra["id"] = "step-invented"
        self.coverage["lab_steps"].append(extra)
        self._write_json(self.verify / "coverage_manifest.json", self.coverage)
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("APP-OWNED CIRCUIT STEP INVENTORY", result.stdout)

    def test_selected_image_hidden_in_comment_or_fence_does_not_count(self):
        original = self.guide.read_text(encoding="utf-8")
        image_start = original.index("![Annotated")
        image_end = original.index("# 2. Applications")
        block = original[image_start:image_end]
        image_line = (
            "![Annotated packet flow with the forwarding decision highlighted]"
            "(Lecture_Guide_assets/annotated-source.png)"
        )
        for hidden in (
            f"<!--\n{block}\n-->\n\n",
            f"```markdown\n{block}\n```\n\n",
            f"````markdown\n``` not a closing fence\n{block}\n````\n\n",
            block.replace(
                image_line,
                f"> ```markdown\n> {image_line}\n> ```",
                1,
            ),
            block.replace(
                image_line,
                f"- ```markdown\n  {image_line}\n  ```",
                1,
            ),
            block.replace(image_line, f"`\n{image_line}\n`", 1),
            block.replace(image_line, f">     {image_line}", 1),
            block.replace("![Annotated", r"\![Annotated", 1),
        ):
            self.guide.write_text(
                original[:image_start] + hidden + original[image_end:],
                encoding="utf-8",
            )
            result = self._run()
            self.assertNotEqual(0, result.returncode)
            self.assertIn("UNSUPPORTED IMAGE SYNTAX", result.stdout)

    def test_commonmark_parser_counts_only_actual_visible_image_nodes(self):
        module = _load_lint_module()
        expected_version = module._expected_markdown_it_version()
        self.assertEqual(
            f"markdown-it-py=={expected_version}\n".encode("ascii"),
            (HERE / "requirements-verifier.txt").read_bytes(),
        )
        valid = (
            "![Annotated packet flow with the forwarding decision highlighted]"
            "(Lecture_Guide_assets/annotated-source.png)"
        )
        images, _, errors, _ = module._markdown_images(valid)
        self.assertEqual(1, len(images))
        self.assertEqual([], errors)

        images, _, errors, _ = module._markdown_images(
            "Ordinary prose with [brackets], an exclamation point!, and a path.png."
        )
        self.assertEqual([], images)
        self.assertEqual([], errors)

        for hidden in (
            f"> ```markdown\n> {valid}\n> ```",
            f"- ```markdown\n  {valid}\n  ```",
            f"`\n{valid}\n`",
            f">     {valid}",
            rf"\{valid}",
        ):
            images, _, errors, _ = module._markdown_images(hidden)
            self.assertEqual([], images)
            self.assertTrue(errors, hidden)

        for invalid in (
            b"markdown-it-py==4.2.0",
            b"markdown-it-py==4.2.0\r\n",
            b"markdown-it-py>=4.2.0\n",
            b"markdown-it-py==4.2.0\nextra==1.0.0\n",
        ):
            invalid_lock = self.root / "invalid-requirements.txt"
            invalid_lock.write_bytes(invalid)
            with self.assertRaises(RuntimeError):
                module._expected_markdown_it_version(invalid_lock)

        alternate_lock = self.root / "alternate-requirements.txt"
        alternate_lock.write_bytes(b"markdown-it-py==9.8.7\n")
        module.VERIFIER_REQUIREMENTS_PATH = alternate_lock
        module.markdown_it = type("MarkdownItPackage", (), {"__version__": "9.8.7"})()
        images, _, errors, _ = module._markdown_images(valid)
        self.assertEqual(1, len(images))
        self.assertEqual([], errors)

        module.MarkdownIt = None
        module.markdown_it = None
        images, _, errors, _ = module._markdown_images(valid)
        self.assertEqual([], images)
        self.assertIn("is required; found not installed", errors[0])

    def test_hidden_html_container_and_loose_visual_prose_cannot_satisfy_placement(self):
        original = self.guide.read_text(encoding="utf-8")
        image_start = original.index("![Annotated")
        image_end = original.index("# 2. Applications")
        block = original[image_start:image_end]
        self.guide.write_text(
            original[:image_start] + f'<div hidden>\n{block}\n</div>\n\n' + original[image_end:],
            encoding="utf-8",
        )
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("UNSUPPORTED IMAGE SYNTAX", result.stdout)

        for altered in (
            block.replace(
                "![Annotated packet flow with the forwarding decision highlighted]"
                "(Lecture_Guide_assets/annotated-source.png)",
                "Prefix ![Annotated packet flow with the forwarding decision highlighted]"
                "(Lecture_Guide_assets/annotated-source.png)",
            ),
            block.replace(
                "*Figure: Annotated source figure showing the decision boundary.*",
                "Unrelated prose mentions *Figure: Annotated source figure showing the decision boundary.* later.",
            ),
            block.replace(
                "**What to notice:** Follow the highlighted arrow before comparing the two outcomes.",
                "This paragraph is unrelated.\n\n**What to notice:** Follow the highlighted arrow before comparing the two outcomes.",
            ),
        ):
            self.guide.write_text(
                original[:image_start] + altered + original[image_end:],
                encoding="utf-8",
            )
            result = self._run()
            self.assertNotEqual(0, result.returncode)
            self.assertIn("IMAGE EXPLANATION", result.stdout)

    def test_reference_html_remote_and_unmanifested_images_are_rejected(self):
        original = self.guide.read_text(encoding="utf-8")
        inline = (
            "![Annotated packet flow with the forwarding decision highlighted]"
            "(Lecture_Guide_assets/annotated-source.png)"
        )
        replacements = [
            (
                "![Annotated packet flow with the forwarding decision highlighted][figure-one]\n"
                "[figure-one]: Lecture_Guide_assets/annotated-source.png",
                "UNSUPPORTED IMAGE SYNTAX",
            ),
            (
                '<img src="Lecture_Guide_assets/annotated-source.png" alt="hidden">',
                "UNSUPPORTED IMAGE SYNTAX",
            ),
            (
                '<picture><source srcset="Lecture_Guide_assets/annotated-source.png"></picture>',
                "UNSUPPORTED IMAGE SYNTAX",
            ),
            ("![Remote](https://example.com/remote.png)", "ASSET PATH ESCAPE"),
            ("![Embedded](data:image/png;base64,AAAA)", "ASSET PATH ESCAPE"),
            ("![Local](file:///tmp/image.png)", "UNSUPPORTED IMAGE SYNTAX"),
        ]
        for replacement, expected in replacements:
            self.guide.write_text(original.replace(inline, replacement), encoding="utf-8")
            result = self._run()
            self.assertNotEqual(0, result.returncode)
            self.assertIn(expected, result.stdout)

        self.guide.write_text(
            original + "\n![Unmanifested](Lecture_Guide_assets/other.png)\n",
            encoding="utf-8",
        )
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("UNREGISTERED LEARNER ASSET", result.stdout)

    def test_empty_markdown_alt_text_fails(self):
        text = self.guide.read_text(encoding="utf-8").replace(
            "![Annotated packet flow with the forwarding decision highlighted]", "![]"
        )
        self.guide.write_text(text, encoding="utf-8")
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("IMAGE ALT TEXT", result.stdout)

    def test_markdown_asset_path_escape_fails(self):
        outside = self.root / "outside.png"
        outside.write_bytes(b"not allowed")
        text = self.guide.read_text(encoding="utf-8").replace(
            "Lecture_Guide_assets/annotated-source.png", "../outside.png"
        )
        self.guide.write_text(text, encoding="utf-8")
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("ASSET PATH ESCAPE", result.stdout)

    def test_manifest_asset_path_escape_fails(self):
        manifest_path = self.assets / "asset_manifest.json"
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        manifest["assets"][0]["path"] = "../outside.png"
        self._write_json(manifest_path, manifest)
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("ASSET PATH ESCAPE", result.stdout)

    def test_source_mutation_fails_integrity_gate(self):
        self.source.write_bytes(b"changed after snapshot")
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("SOURCE INTEGRITY", result.stdout)

    def test_rewritten_source_and_manifests_cannot_replace_trusted_hash(self):
        self.source.write_bytes(b"coordinated rewrite")
        rewritten_hash = hashlib.sha256(self.source.read_bytes()).hexdigest()
        self.snapshot["sources"][0]["sha256"] = rewritten_hash
        self.snapshot["sources"][0]["size_bytes"] = self.source.stat().st_size
        self.coverage["sources"][0]["sha256"] = rewritten_hash
        self._write_json(self.verify / "source_snapshot.json", self.snapshot)
        self._write_json(self.verify / "coverage_manifest.json", self.coverage)
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("trusted pre-generation SHA-256", result.stdout)

    def test_coverage_hash_mismatch_fails(self):
        self.coverage["sources"][0]["sha256"] = "0" * 64
        self._write_json(self.verify / "coverage_manifest.json", self.coverage)
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("SOURCE HASH MISMATCH", result.stdout)

    def test_missing_source_register_fails(self):
        del self.coverage["sources"]
        self._write_json(self.verify / "coverage_manifest.json", self.coverage)
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("SOURCE REGISTER", result.stdout)

    def test_scientific_writing_support_worksheet_cannot_be_omitted(self):
        worksheet = self.root / "Week 3 Worksheet.docx"
        worksheet.write_bytes(b"worksheet source bytes")
        identity = str(worksheet.resolve()).replace("\\", "/")
        if os.name == "nt":
            identity = identity.lower()
        worksheet_id = "source-" + hashlib.sha256(identity.encode("utf-8")).hexdigest()
        worksheet_sha = hashlib.sha256(worksheet.read_bytes()).hexdigest()
        worksheet_unit = f"{worksheet_id}-unit-001"
        self.snapshot["sources"].append({
            "id": worksheet_id,
            "path": str(worksheet.resolve()),
            "name": worksheet.name,
            "sha256": worksheet_sha,
            "size_bytes": worksheet.stat().st_size,
            "role": "support",
            "unit_kind": "docx-document",
            "unit_count": 1,
            "unit_ids": [worksheet_unit],
        })
        self.snapshot["unit_ids"].append(worksheet_unit)
        self.coverage["units"].append({
            "id": worksheet_unit,
            "source_id": worksheet_id,
            "guide_anchor": "2-applications",
            "topic": "worksheet practice",
        })
        self._write_json(self.verify / "source_snapshot.json", self.snapshot)
        self._write_json(self.verify / "coverage_manifest.json", self.coverage)

        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("SOURCE REGISTER", result.stdout)

    def test_every_bound_predecessor_requires_exact_bridge_and_evidence(self):
        predecessor = self.root / "Week_1_Guide.md"
        predecessor.write_text("# Previous guide\n", encoding="utf-8")
        predecessor_sha = hashlib.sha256(predecessor.read_bytes()).hexdigest()
        manifest_entry = {
            "course_profile": "scientific-writing",
            "generation_identity": "scientific-writing:scientific-writing-week:week-01",
            "sequence_key": "week-01",
            "path": str(predecessor.resolve()),
            "sha256": predecessor_sha,
        }
        self._write_json(
            self.verify / "predecessor_manifest.json",
            {"schema_version": 1, "prior_guides": [manifest_entry]},
        )
        self.coverage["continuity"]["prior_guides"] = [{
            "identity": manifest_entry["generation_identity"],
            "path": manifest_entry["path"],
            "sha256": predecessor_sha,
            "bridge": "This lecture applies the earlier thesis structure.",
            "evidence": ["The prior guide introduced claim-evidence links."],
        }]
        self._write_json(self.verify / "coverage_manifest.json", self.coverage)
        self.assertEqual(0, self._run().returncode)

        self.coverage["continuity"]["prior_guides"][0]["evidence"] = []
        self._write_json(self.verify / "coverage_manifest.json", self.coverage)
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("CUMULATIVE CONTINUITY", result.stdout)

    def test_malformed_manifest_ids_fail_cleanly(self):
        self.coverage["units"][0]["id"] = ["not", "hashable"]
        self.coverage["sources"][0]["id"] = {"not": "a string"}
        self._write_json(self.verify / "coverage_manifest.json", self.coverage)
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertNotIn("Traceback", result.stdout + result.stderr)
        self.assertIn("MISSING SOURCE-UNIT COVERAGE", result.stdout)

    def test_missing_picture_explanation_fails(self):
        text = self.guide.read_text(encoding="utf-8").replace("**What to notice:**", "**Explanation:**")
        self.guide.write_text(text, encoding="utf-8")
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("IMAGE EXPLANATION", result.stdout)

    def test_circuit_lab_requires_complete_step_template(self):
        self.coverage["guide_kind"] = "circuit-lab"
        self._write_json(self.verify / "coverage_manifest.json", self.coverage)
        result = self._run()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("CIRCUIT LAB STEP CONTRACT", result.stdout)

    def test_snapshot_source_cli_hashes_and_counts_html_units(self):
        html = self.root / "lecture.html"
        html.write_text(
            "<script>const a = `This is the first sufficiently long instructional section.`;"
            "const b = `This is the second sufficiently long instructional section.`;</script>",
            encoding="utf-8",
        )
        snapshot_dir = self.root / "snapshot-only"
        result = subprocess.run(
            [
                sys.executable,
                str(LINT),
                "--snapshot-source",
                str(html),
                "--verify-dir",
                str(snapshot_dir),
            ],
            capture_output=True,
            text=True,
        )
        self.assertEqual(0, result.returncode, result.stdout + result.stderr)
        snapshot = json.loads((snapshot_dir / "source_snapshot.json").read_text(encoding="utf-8"))
        self.assertEqual("section", snapshot["sources"][0]["unit_kind"])
        self.assertEqual(2, len(snapshot["unit_ids"]))
        self.assertTrue(all(unit.startswith(snapshot["primary_source_id"]) for unit in snapshot["unit_ids"]))
        self.assertEqual(
            hashlib.sha256(html.read_bytes()).hexdigest(),
            snapshot["sources"][0]["sha256"],
        )

    def test_seal_removes_premature_marker_when_any_gate_fails(self):
        self._write_valid_guide(include_marker=True)
        self.coverage["units"].pop()
        self._write_json(self.verify / "coverage_manifest.json", self.coverage)
        result = self._run("--seal")
        self.assertNotEqual(0, result.returncode)
        self.assertNotIn(SENTINEL, self.guide.read_text(encoding="utf-8"))

    def test_seal_appends_one_marker_only_after_all_gates_pass(self):
        result = self._run("--seal")
        self.assertEqual(0, result.returncode, result.stdout + result.stderr)
        self.assertEqual(1, self.guide.read_text(encoding="utf-8").count(SENTINEL))

    def test_seal_rejects_no_harness_bypass(self):
        result = self._run("--seal", "--no-harness")
        self.assertNotEqual(0, result.returncode)
        self.assertIn("SEAL CONFIGURATION", result.stdout)
        self.assertNotIn(SENTINEL, self.guide.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
