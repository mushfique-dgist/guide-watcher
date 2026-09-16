# Guide quality audit and roadmap — 2026-09-10

This file is the durable record of the audit of the Guide Watcher pipeline, the two study guides it produced for Operating Systems (lectures 2 and 3) and Computer Networks (CH01), the guide-writing prompt, and the depth contract — plus what was changed and what remains. The two long reports it summarises are kept beside it:

- [DS-vs-OS pedagogy audit](2026-09-10-audit-ds-vs-os-pedagogy.md) — line-level comparison of the best 4th-semester guides against the OS guides.
- [Lecture teaching patterns](2026-09-10-lecture-teaching-patterns.md) — techniques extracted from real transcripts (Remzi CS537, MIT 6.S081, MIT 6.004, Kurose & Ross, CMU 15-213 and CS162 slides), with 12 promptable rules.
- Past exam transcription and archetype profile: `5th Semester/Operating Systems/OS Last year final exam questions/2025F-CSE304_Final_Exam.md`.
- Collected lecture transcripts (OS): `5th Semester/Operating Systems/_supplementary/lecture-transcripts/` with `manifest.json`.

## 1. What was measured

| | DS Week 12 | DS Week 5 | SP 16 VM | OS lec 2 (before) | OS lec 3 (before) | Networks CH01 (as published) |
|---|---|---|---|---|---|---|
| words | 25,790 | 16,835 | 15,937 | 32,589 | 18,360 | 12,561 |
| figures | 0 | 0 | 0 | 38 | 32 | 30 |
| code / trace blocks | 114 | 137 | 45 | 13 | 4 | — |
| labeled worked examples | 11 | 40 | 10 | 27 | 16 | **0** |
| sections without an example | — | — | — | 14 | 15 | 20 of 23 |

The reference guides teach with traces and tables placed inside the argument; the generated guides replaced that with captioned figures. Words per slide for CH01 was 314 — barely above the 240 floor the gate enforces, which is the Goodhart problem: floors get met, targets do not.

## 2. Findings (evidence in the two linked reports)

1. **Figure pattern.** The image / `*Figure*` / `**What to notice**` triplet is fine where the next paragraph reads specific elements off the figure (~55 of 70 in the OS guides). It is dead weight for screenshots of code already transcribed beside them (9 in lec 2), for the second member of every back-to-back pair (8 clusters in lec 3), and wherever the notice line is paraphrased by the following paragraph. Hand-added original diagrams were followed by paragraphs about why the diagram was drawn — never done in the reference guides.
2. **Define-before-use.** Lec 2 used ~18 terms 50–1000 lines before defining them (page table, interrupt, TLB, hypervisor, fork, makespan: never). Lec 3 used ~22 (kernel, trap, quantum, thrashing, calling convention: never). Reference guides define on contact.
3. **Analogies.** Lec 3 had one analogy in 967 lines; lec 2 inherited the lecture's. Neither had an analogy for the context switch or the lost update. Reference guides give an analogy with an explicit part-to-part mapping before the formal object.
4. **Worked examples.** Mechanism sections without a traced example: lec 2 §6, §9, §12, §15, §17, §22; lec 3 §9, §11–13, §16–18, §20 (the context switch was never traced with register values). Examples were clustered in "Three Worked Examples" subsections rather than beside the mechanism.
5. **Section endings.** Reference guides end every section with a boxed insight naming the pattern; the OS guides trailed off into caveats.
6. **Exam alignment.** The 2025F final tests six archetypes (negative-recognition MCQ, exact-terminology fill-ins, metric-defined schedule design, algorithm state after step k and after a large k, best/worst-case concurrency timelines, fill-in-the-blank C). 44 of 100 marks are code-level synchronization. The practice sections were essay questions only.
7. **How the best lecturers explain** (transcripts, not memory): problem first ("the crux"); strawman → numbered failures → fix in order; describe the behavior, then name it; one analogy at the moment of need followed by where it breaks; an adversary paragraph for every protection rule; a running cast on a labeled timeline; the metric defined before policies are compared; real output shown and read line by line; every unit closed with a recap and the next question. Best match for the student's stated learning style: Remzi Arpaci-Dusseau (CS537), then MIT 6.S081, then Kurose & Ross for networks.
8. **Pipeline failures observed on real runs.** (a) Lint false positives on correct arithmetic chains and percentages burned repair attempts. (b) A deepening pass that re-emits a 15k-word guide dropped the app-owned artifact block and was rejected; the second pass never ran. (c) A resume against a course folder that had gained a PDF failed because textbook ids were positional. (d) The writer output `![...]` as a figure of speech. All four are fixed (see §4).

## 3. Decisions taken

- **Floors are not targets.** Target ≈450 words per slide carried by traced instances, not padding. Depth passes run while under target; a pedagogy pass always runs afterwards against the verifier's advisory.
- **The advisory is not a gate.** Heuristics (sections without an example, terms used before their bold definition, unwoven figures, thin sections) have false positives, so they drive a bounded fix pass instead of failing a run.
- **App-owned data is never round-tripped through the model.** The artifact block is detached before a pass and reattached after it.
- **Past exams are first-class input.** Any `*exam*.md` in a course folder is frozen, shown to the writer, and its archetypes must appear in the practice section (hard gate: the section exists with ≥8 solved items).
- **Slides are the limit.** Supplementary material (books, transcripts) may only explain what the lecture covers; it must not introduce topics absent from the slides and syllabus.

## 4. What changed on 2026-09-10

Prompt/contract: describe-before-name (bold once at the definition; deferral tags "(defined in Section N)"; synonyms declared once; near-synonyms contrasted); analogy + explicit mapping + where it breaks; problem first / strawman / assumption ledger; figure economy; examples co-located with the mechanism, running cast, metric-before-comparison, adversary paragraphs, process graphs, try-it-yourself variants; `**Pattern to recognize.**` closers; mandatory Exam-Style Practice section.

Lint (`guide_lint.py`): chain- and percent-aware arithmetic with unary minus and hex-safe tokens; `--advisory`; `--require-exam-practice`.

App: exam-pattern discovery/freezing/prompting/gating; depth passes → advisory-driven pedagogy pass; artifact block detached/reattached; stray `![` rejected before lint; resume reconciles the saved visual plan against a changed folder; readable Overview log with "what to do next".

Guides: OS lec 2 (32.6k → 44.1k words, 38 → 27 figures, 16 terms defined, 14 traces, Q13–Q25 exam-format) and lec 3 (18.4k → 32.6k, 32 → 25, ~30 terms, 7 analogies, register-level context-switch trace, Q11–Q22) revised against the audit; both pass the advisory with nothing to fix and all arithmetic recomputed.

## 5. Roadmap (the parts the app must still do itself)

1. **Harness writer.** Run the writer as a tool-using Claude Code session inside the job workspace: it writes `guide.md` and `artifacts.json` as files, may run only the frozen verifier (`guide_lint.py --advisory`, arithmetic check) on its own draft, fixes what the verifier reports, and finishes when the advisory is empty. Depth and pedagogy passes edit the draft in place instead of re-emitting it. The app keeps every gate; the model gets the loop.
2. **Supplementary source collection, once per course.** A cheap model proposes candidate lectures (playlists, OCW pages) for the course's syllabus topics; a deterministic, tested script fetches transcripts with explicit per-item status (ok / no captions / blocked / unavailable) and writes `_supplementary/<source>/transcript.md` + `manifest.json` + `collection_report.md`; an audit pass maps each transcript to lectures. Nothing is silently skipped, and nothing leaves the course folder afterwards.
3. **Relevance-gated ingestion.** The context collector loads only transcript segments mapped to the current lecture, ranked by lecture-term overlap, under a fixed character budget, with the "slides are the limit" rule in the prompt.
4. **Generalize the exam and transcript handling to every course** — no course-specific code paths; discovery is by folder convention (`*exam*.md`, `_supplementary/manifest.json`).

## 6. Open questions

- Whether YouTube captions stay reachable from this network long-term (reachable on 2026-09-10; the collector must fail loudly, not skip).
- Cost: the harness writer uses more turns per guide but fewer full re-emissions; the pass-level word budget bounds it.

## 7. Status update (2026-09-10, evening)

Implemented from the roadmap:

1. **Harness writer** (`claude.rs`, `codex.rs`): the writer, every deepening pass, and every repair now run as a tool-using Claude Code session in the job workspace with `--permission-mode dontAsk --restricted --allowedTools "Edit(draft/**)" "Bash(./verify_draft.sh)"`. Verified empirically: writes only under `draft/`, one runnable command, frozen inputs untouchable, nothing outside the workspace. The writer drafts `draft/guide.md` section by section, puts the artifact JSON in `draft/artifacts.json`, runs the frozen verifier's self-check (`guide_lint.py --advisory`, which now prints MUST FIX hard failures before ADVISORY teaching gaps), and fixes until it reports nothing to fix (bounded by `HARNESS_VERIFY_ROUNDS`). Passes and repairs edit the existing draft in place instead of re-emitting it. Codex remains a response-text fallback.
2. **Supplementary collection**: `supplementary_collect.py` (deterministic fetcher: YouTube captions via youtube-transcript-api 1.2.4, transcript/course pages, PDFs; retries with backoff; per-item status ok / no_captions / blocked / unavailable / invalid / network_error; atomic writes; merged `manifest.json`; `collection_report.md`; 8 tests) and `guide-watcher-cli supplementary plan|collect|status <course_dir>` (plan = prep model with live web search proposes candidates into `_supplementary/candidates.json`; collect = the script; nothing is silently skipped).
3. **Relevance-gated ingestion** (`source_context.rs`): `_supplementary/manifest.json` items with `status: ok` are hashed as context dependencies, frozen into the authoritative input index, and quoted into a `## SUPPLEMENTARY TRANSCRIPTS` prep section only for paragraphs that match the lecture text (same scoring as textbook pages; at most 8 paragraphs per transcript under a 90k-character budget). A transcript edited after collection is refused, not silently used.
4. **Prompt rules 12-13** (real output read line by line; "why not the other way?" closers; slides are the limit) and contract item 12.
5. **Operating Systems course data**: 32 transcripts (Remzi CS537, MIT 6.S081, MIT 6.004, CS162, 15-213) in `5th Semester/Operating Systems/_supplementary/` with a manifest in the collector's schema.

Not yet done: an app UI for the supplementary commands (CLI only); an LLM audit that maps transcripts to specific lectures (`lecture_map` stays empty; relevance ranking does the gating instead); a first production run through the harness (the next generated guide is the smoke test).

**First production run through the harness (CH01 Networks, 2026-09-10 20:30-21:35):** writer draft, deepening pass 1 accepted (16,800 -> 21,437 words), depth target met, pedagogy check found nothing to fix, one in-place metadata repair, PASS. Published guide: 19,503 words, 65 labeled worked examples, 20 section closers, 10 exam-style items, 31 figures, advisory clean. The same sources on the previous build (single-shot writer, passes skipped, re-emitting repair) had produced 9,473 words with zero examples.

## 8. Formatting standard and body hygiene (2026-09-11)

User review of the Algo L1 guide found the remaining failure modes: values and traces left bare inside sentences (walls of text), practice items with no visual separation, "Textbook Reading Path" sections telling the reader what to read, and sections narrating the deck ("Lecture PDF page 22 returns to...", a section for the "Any Question?" slide). Causes and fixes:

- The reading path was forced by the app: the research-notes validator required a `reading_path` whose evidence "must visibly tell the student what to read". Removed from the validator, the writer instructions, and the phase-2 prompt (integrations evidence still required).
- Slide narration and administrative-slide sections came from the coverage contract's "never omit a unit because it is administrative". Contract now maps administrative units to the nearest section with no prose; slide narration and reading instructions are MUST FIX items in the writer's self-check.
- Formatting had no rule at all. `guide_format_standard.md` (F1-F8: code/KaTeX for every value, steps + state tables for traces, Given/Steps/Result/Check example callouts, icon-led definition/misconception/pattern callouts, `### Qn` practice items with a blockquoted question and a bold answer line, 120-word paragraphs, sub-structure per section, no viewer-specific syntax) is inlined in guide_prompt.txt as Step 2b, in the contract as items 13-14, and checked by `guide_lint.py --advisory` (FORMAT blocks; `guide_format.py` is the standalone entry point).
- Exemplar: `5th Semester/Computer Algorithms/L1___Algorithmic_Analysis_I_Guide.md` reformatted (72 example callouts, 85 callouts, 10 structured practice items, 0 narration, 0 reading instructions; both checkers clean; 182 equalities recomputed).
- Lint: Unicode minus (U+2212) is now subtraction in the arithmetic checker; blockquoted example callouts count as worked examples; sub-structure rule applies to sections of 200+ words.

## 9. Resilience and revision handling (2026-09-12/13)

- **Fallback policy.** Two real failures ended jobs without using the configured Codex fallback: "You've hit your session limit · resets 10:40pm" (not in the quota patterns) and "Not logged in · Please run /login" (classified as a permission error, which was deliberately excluded). New policy: any condition under which Claude cannot serve the request through no fault of the draft - quota/session/weekly limits, authentication (logged out, unauthorized), transport (connection lost, DNS, timeouts, 5xx) - moves to the next Claude candidate and then to the Codex fallback. Content rejection, cancellation, and genuine writer mistakes stay terminal.
- **Continue instead of restart.** A writer that dies mid-guide leaves its draft in the retained workspace (`.<Guide>.md.<txn>.gwfailed/work/draft/guide.md`). A new attempt for the same output now finds the newest substantial draft (>= 1,500 words with a table of contents), seeds the harness with it, and instructs the writer - possibly a different model - to read it completely and finish it in the same voice, notation, structure and formatting; the Codex response-text fallback gets the draft inline. History shows "Picking up the N-word draft the earlier attempt left".
- **Resume with a chosen model.** History's Resume writing now sends the user's current generation preferences (writer model/effort, fallbacks) instead of compiled defaults, and the confirmation names the model; change it under Settings -> Generation defaults first. The Workspace resume panel already offered these selectors.
- **Manual revisions are first-class.** A guide revised by hand carries `<!-- guide-watcher: manual-revision ... -->`; the app treats it as complete and usable (predecessor continuity, semester-scan skipping, Open guide) though not bundle-sealed. All six revised guides (OS 2/3, Algo L1/L2, Networks CH01/CH02) carry the marker.
- **Blank failures fixed.** Preflight/dispatch errors are recorded with their raw text as History detail.
- **Portable copies.** `_automation/embed_guide_images.py` (+ `Make Portable Guide.cmd`) writes `<Guide>_portable.md` with every figure inlined as a data URI; paths with parentheses handled.

## 10. Style handoff and the preflight gate (2026-09-13)

- **Why a style plan.** Under identical instructions Claude's guides read better than the Codex fallback's. The difference is in choices the template does not dictate: register, the running cast, how an analogy is mapped and then broken, how a section opens on a problem and closes on a question, which instance values the worked examples use. So the primary writer now records those choices for this guide in `draft/style_plan.md` before writing the first section (400-800 words; explicitly "not a restatement of the instructions"). A continuing writer reads it, or writes it from the draft's evident choices when it is missing. The plan is preserved across workspace resets and failed attempts, is picked up with a prior draft on continuation, and is appended to every Codex fallback prompt (writer, depth and pedagogy passes, repair, continuation) as "STYLE PLAN FROM THE PRIMARY WRITER ... binding instructions".
- **Adversarial preflight sweep.** `guide-watcher-cli check` runs the real preflight for one lecture or prep packet and discards the reserved workspace; `_automation/preflight_sweep.py` runs it over every course (17 targets, ~50 s each). First run: 4 lectures refused with "two visual assets claim the same learning purpose" (CH00, CH02 (3), L0, L2). Cause: supplementary-book discovery took every other PDF in the folder, the lecture heuristic missed "Instructor : Name" and decks with no instructor line, sibling decks got the title "Fall 2026", and two of them at the same page number produced identical figure purposes. Fix: the course's lecture-naming rule excludes sibling decks up front, the heuristic compacts whitespace, colliding titles get the file name appended, and the catalog error names both assets. The sweep also treats "existing prep packet" as an expected refusal.
- **Resume drift, second half.** Preflight accepted the drift and logged it, but the writer start then compared the saved GENERATION CONTRACT to the freshly bound one with plain equality and failed with "prep generation contract no longer matches the preflight-bound source and predecessor inputs". The `check` command stopped at preflight, so the sweep passed while the app still failed; that is how an unverified "fixed" claim shipped. Now the comparison is field-by-field (source, output, identity, course, kind, source digests strict; predecessors strict only when preflight reported no drift), the real resume and `check` share one binding function, and `check` on a prep packet runs the writer-start checks too. Lesson recorded: a fix is verified only when the exact user action is exercised, not a proxy of it.

## 11. Fallback eligibility stops guessing at wording (2026-09-13)

- **Third miss in a row.** After the session-limit and logged-out misses, a run died on "You've reached your Fable limit. Switch to another model to continue." - a per-model allowance whose wording no pattern matched, so the Codex fallback never ran and 20 minutes of phase 1 was wasted. A substring list will always lag the CLI's wording.
- **The structural rule.** A non-zero exit from the Claude CLI (or an `is_error` result, or an empty result) means no guide was produced. That is never a writing mistake, because a badly written guide exits zero and is caught by the verifier and the repair loop. So any such failure is now eligible for the next Claude candidate and then the Codex fallback, whatever the message says, unless the text identifies cancellation or content rejection. The new `ClaudeFailureKind::Unrecognized` carries that case. Failures that occur with Claude's output in hand (invalid artifact JSON, write errors) stay terminal as before. Pattern matching now only decides the reason shown in History, not whether the run survives.
- **Per-model limits recognized.** "reached your <model> limit" and "switch to another model" are matched by shape, so new model names need no code change.
- **One app instance.** `tauri-plugin-single-instance` is registered first in the builder; a second launch focuses the running window (showing and unminimizing it, since the app lives in the tray) and exits. Two instances - one installed, one started from the build folder - had been running at once, which also locked the build output and made a rebuild fail.

## 12. Telling the user, and honest durations (2026-09-13)

- **The problem.** A guide runs for one to three hours. Nothing reached the user unless the window was in front of them, so a finished guide, a failed run, or lecture files waiting for review could sit unnoticed for hours.
- **Two signals.** A Windows notification at the moment it happens (guide ready / Guide Watcher needs you / sources ready to review, naming the guide), and a count on the app icon that stays until the window takes focus. On Windows the taskbar count is an overlay icon, so the badge is rendered in the app: a red disc with the numeral, "9+" past nine, drawn at whatever size the target needs. The tray icon carries the same badge and a tooltip, because closing the window hides it in the tray and a hidden window has no taskbar button - which is exactly when a long run tends to finish. Stops the user asked for (window close, Stop, cancel from History) raise nothing.
- **Durations stopped lying.** "This usually takes 15-25 minutes" was quoted for every lecture. Measured: L4 with 115 slides drafted in 37 minutes, roughly a third of a minute per slide, and each deepening pass costs about the same. The phase line now carries the source-unit count and the estimate scales with it, naming the passes that follow rather than hiding them.
- **Why L4 takes hours.** 115 slides against 38-60 for a typical lecture in this semester, and the depth target scales at 450 words per slide, so the target is 51,750 words. The 35,246-word draft is well short of it, so all three enrichment passes run over a 35k-word document rather than being skipped.

## 13. When a rule change invalidates frozen context (2026-09-13)

- **What happened.** Excluding sibling lecture decks from reference books (section 10) removed pages that L3's 12 September context had already frozen into its figure plan: three pages each from L0, L1, L2 and L4, cropped as if they were textbook pages. Resuming L3 refused the whole packet with "that source is no longer present in the course folder" - untrue, since the files are all still there - and would have cost 90 minutes of collected context over 12 figures that never belonged in the guide.
- **The rule now.** Each saved figure carries its own image digest, so one that can no longer be bound is dropped and named in the run log while the rest proceed. Refusal is reserved for what actually poisons a plan: image bytes that changed under a stable identity, and bindings that became ambiguous. L3's packet keeps 122 of its 134 figures, including all 117 from its own slides.
- **The check gap, again.** `guide-watcher-cli check` stopped before the figure binding, so it passed L3 while the app failed on it - the same shape of mistake as section 11. The dry run now performs the figure re-binding and reports its notes, so the check covers every step a resume takes before a model starts. It only does so when the packet has a saved plan; without one a resume would call the vision model, which a check never does.

## 14. The interface for people who are not the author (2026-09-14)

- **The brief.** Make the app simple for a normal user without removing anything, and add restrained colour. The owner is a power user who likes the current interface, which settled the shape: simplicity is a mode, not a replacement. Options were put on a comparison page with the real app chrome rendered in each candidate palette, and five decisions were chosen: simple by default with a Power mode switch, the Slate & Mint palette, one quality choice instead of ten dropdowns, one confirm screen, and a four-step progress view with a clock.
- **Power mode.** One switch in Settings, stored per computer. Off, the review screen asks Quick / Balanced / Thorough and hides the model grid behind `Customise models for this run`; the Workspace drops the saved-prep entry point and stops naming Codex and Claude; a run shows four steps, the current phase in plain words and the estimate counting down, with the timeline behind `Show details` and no Technical log tab. On, every screen is exactly what it was. Identical pipeline either way.
- **Presets move effort, never models.** Quick, Balanced and Thorough set the six effort levels and leave every model as the user's defaults, because which model writes is an advanced decision. Balanced is the app's own default effort set, so a fresh install already matches it, and a hand-picked mix reports itself as Custom rather than being snapped to a preset.
- **Colour carries meaning only.** One mint accent for actions and identity; green finished, blue running, red failed, amber needs you; a cancelled run stays neutral because the user stopped it. The four status hues were checked with a contrast and colour-blindness validator against both grounds, which is also what produced the neutral-cancelled decision. Status badges became a dot plus a word so state reads at a glance.
- **Progress reads the same words the user does.** The four-step view derives its step from the app's own humanized phase messages rather than a second source of truth that could drift from them, and the countdown only runs while the app has actually estimated the phase.

