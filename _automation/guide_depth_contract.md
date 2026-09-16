# Canonical Guide Creator Contract

This is the quality bar for every course guide written through Codex, Claude, Guide Watcher, or a manual recovery run. The guide is not a slide summary. It is a complete teaching artifact: a student should be able to read the guide alone, understand the whole lecture, and solve representative exam or lab tasks without reconstructing missing reasoning.

## Unit of Work and Complete Coverage

Create one Markdown guide for one lecture deck or one course week. That guide must teach the content of every source slide, page, section, or explicitly selected video segment. Organize the prose by concepts and dependencies rather than mechanically creating a heading for each slide. Every source unit is mapped in the coverage manifest, but only substantive units are taught: an administrative unit (title, outline, "Any questions?", thank-you, blank) is mapped to the nearest real section and gets no prose of its own.

Before generation, record every explicitly selected primary and support source in `.gwverify/source_snapshot.json`, including its stable source ID, absolute path, SHA-256 hash, and ordered source-unit IDs. Before completion, create `.gwverify/coverage_manifest.json` whose `sources` list exactly copies that app-owned source set in order and whose `units` list has exactly one record per source unit. Each unit record must name its owning source ID, topic, and real Markdown heading anchor where the unit is taught. Duplicate, missing, unknown, or unmapped sources or units are failures. The manifest is audit evidence; it does not replace teaching the material in the guide.

## Knowledge Scaffolding and Cumulative Continuity

1. Begin with the minimum prerequisite bridge needed for this lecture. Re-teach prerequisites briefly enough that the guide remains usable on its own; do not merely say “as you already know.”
2. Within the guide, introduce ideas in dependency order and explicitly connect each new mechanism to the ideas that make it possible.
3. Before drafting a later guide, read every app-bound predecessor in semantic course order. Reuse its notation and terminology where correct, recall the exact ideas needed now, and avoid teaching later material as if the course had restarted. For each predecessor, copy its identity, absolute path, and SHA-256 into `continuity.prior_guides`, then add a nonempty bridge and concrete evidence used from that guide; omission is a hard failure.
4. End with a concrete forward bridge: what the student can now do and which part of the next lecture this enables.
5. Record prerequisites, prior-guide links, and the forward bridge in the coverage manifest. If no prior guide exists, record an empty list rather than inventing one. Also record explicit visual and computational-verification plans with honest rationales. Executable verification is always false: fixed app-owned checks validate supported arithmetic, and model-authored code is forbidden.

## Source Hierarchy, Conflicts, and No Invention

Use sources in this order unless the instructor explicitly says otherwise:

1. the assigned lecture deck, LMS announcement, lab handout, rubric, and instructor-provided video;
2. the assigned textbook and official standards or documentation;
3. reputable university lectures, primary research, and authoritative technical references;
4. secondary tutorials only to clarify, never to override higher-tier course material.

Keep the app-owned `sources` list unchanged while researching. Put additional textbook, standards, web, and video provenance in `research_sources`; give each research source a stable ID and tier, and preserve its title, author or publisher, URL or local filename, chapter/page/slide/timestamp locator, and access date where applicable. Paraphrase copyrighted sources instead of copying long passages. If sources disagree, keep both source IDs and claims in the source-conflict log, explain which source governs the guide and why, and leave the issue unresolved when evidence is insufficient. Never silently “correct” the instructor, invent missing measurements, infer an unstated deadline, create a substitute figure with different values, or present a model-generated diagram as a source figure.

## Non-Negotiable Teaching Depth

1. Prefer explanatory depth over token thrift. Quiet, long generation is expected.
2. Explain the mechanism, not only the result. Every important step needs the reason it is valid.
3. Define notation inline before using it. A novice should not have to infer symbols from context.
4. Use real lecture values. Inspect rendered source images for arrows, labels, edge weights, table entries, addresses, and intermediate states.
5. Give medium, hard, and challenging worked examples for every major concept when the material supports them. Label which is which. Place them inside the section that teaches the mechanism, directly after it, never in a separate examples-only subsection at the end. Show intermediate states and explain why each step follows. After each example, state in one sentence the pattern the reader should recognize and the misconception it prevents. Follow the solved examples with two or three "try it yourself" variants that change the inputs and give only the final answers.
6. For computational guides, show enough intermediate state for independent recomputation. The fixed native gate checks supported scalar arithmetic directly. Never produce `.gwverify/verify.py` or any executable model-authored verifier.
7. **Traced instance standard.** Every mechanical, algorithmic, or stateful concept gets a complete trace over ONE concrete, fully specified small instance, built like this:
   - Give the instance explicitly and completely: the actual values, the actual structure, small enough to follow by hand.
   - Derive the intermediate data rather than asserting it, and include at least one arithmetic self-check that must hold if the setup is right.
   - State every tie-breaking and iteration convention *before* tracing, so the reader can reproduce the same trace and not merely follow it.
   - Number the steps and show the FULL state after each one, not only the part that changed.
   - Give the reason each step is taken, including why an alternative was skipped.
   - Close with what the trace discovered and what would change under a different input.
8. Perform a TA-style second pass: independently recompute traces, compare figure facts against sources, remove false starts and stale numbers, and check later sections for the same depth as early sections.
9. **Describe before you name.** Bold a term exactly once, at the sentence that defines it, and never use it in plain text in an earlier section; an unavoidable early mention is tagged "(defined in Section N)". Expand every acronym at first use; declare synonyms once at the definition; contrast near-synonyms in one sentence when they first meet. The verifier's advisory lists terms used before their definition, and the pedagogy pass must clear it.
10. **Problem first.** Every mechanism section opens with what goes wrong without the mechanism, shows the strawman where the source supports one, lists the failures as numbered problems, and fixes them in order. Each new abstraction gets one analogy with an explicit part-to-part mapping, one sentence on where the analogy breaks, and then the formal definition. Every protection or synchronization rule gets an adversary paragraph showing the interleaving or program that exploits its absence.
11. **Section closers.** Every teaching section ends with a `**Pattern to recognize.**` paragraph: the transferable pattern in one sentence, the misconception it prevents, and the question the next section answers.
12. **Slides are the limit.** Supplementary sources (books, lecture transcripts, standards, web) may explain, re-teach a prerequisite, or supply an example for what the lecture covers; they may not add topics absent from the slides and syllabus. Relevance to the current lecture is the admission test for every supplementary passage, and the context collector enforces a fixed budget per lecture.
13. **Presentation.** Values, expressions, and state never sit bare in a sentence (inline code, `$…$` math, lists, tables); traces are numbered steps with a state table; every worked example carries the Given / Steps / Result / Check skeleton inside a labeled callout; definitions, misconceptions, and `Pattern to recognize` closers are callouts; practice items are `### Qn` headings with a blockquoted question and a bold answer line; no paragraph exceeds 120 words; every major section has sub-structure. The verifier's self-check enforces these.
14. **Body hygiene.** The body never narrates the deck ("PDF page 22 returns to…"), never issues reading instructions ("read Chapter 2 at pages 44–49"), and never contains a section written for an administrative slide; such units are mapped in the manifest to the nearest real section. Source provenance lives in citations, captions, and the manifest.
15. **Exam-style practice.** The guide has one major section titled `# N. Exam-Style Practice …` with at least eight `**Qk (difficulty).**` items and full `*Solution.*` blocks. When the source pack contains an `## EXAM PATTERN` (a past exam found in the course folder), every question archetype in it appears at least once at the exam's difficulty and once above it, built from this lecture's material and never copied. The native gate requires this section whenever a past exam was supplied.

For a substantial computational deck with at least 20 slides, use at least 240 words per slide and at least `min(20, max(10, slide_count // 2))` major numbered sections unless the deck is demonstrably mostly administrative. **That is a shallow-output floor the gate enforces, not a target, and a guide that lands near it is not finished.** Aim for roughly 450 words per slide, carried by the worked examples and traced instances above rather than by padding. The guide is produced in more than one pass: an initial complete draft; then bounded depth passes that add worked examples and traces while the guide is under the target; then a pedagogy pass that receives the verifier's advisory — sections without a traced example, terms used before their definition, figures not woven into prose, sections far thinner than the rest — and fixes exactly those items. Every pass may only add subsections and prose or insert one-sentence definitions at a term's first use. No pass may add, remove, reorder, renumber, or reword a `# N. Title` heading, touch any image line, caption, or 'What to notice' line, or alter the private artifact block — app-owned manifests bind to all of those.

## Example Economy

Depth is difficulty, not repetition. A words-per-slide target is a ceiling for material that does
not need it, never a quota to fill, and meeting it by adding a third and fourth example of an idea
the reader already has is padding that reads as padding. One worked example for a definition or a
classification; two for a mechanism with a trap, the second harder or failing differently; three
only for an idea whose layers change the method. If you cannot say what a further example teaches
that the previous one did not, leave it out and write a `**Pattern to recognize.**` line instead.
The space saved goes to the ideas that are genuinely hard and to the exam-style practice section,
where volume is useful. A lecture of easy ideas produces a shorter guide, and that is correct.

## Visual Teaching Contract

ASCII art is a last resort, not the default. Before guide generation, the app must create or load a strict source-bound visual packet and compile every declared asset locally. A reviewed `<PrimaryFilename>.guide-visuals.json` packet takes precedence. When it is absent, Guide Watcher must render every frozen lecture slide or page and relevant pages from supplied textbooks, have the context model inspect those renders directly, exclude decorative or duplicate images, and rank purposeful candidates across the lecture and books. Automatic lecture visuals use faithful source crops with their explanations outside the image. Do not add arrows, markers, labels, or generated redraws over source content. Reviewed annotations and technical diagrams must point to the intended detail, preserve source facts, and pass a visual check for text obstruction, overlapping labels, and crossing connectors. For Circuit Lab the app must instead capture the source-bound LMS video, let the context model select distinct instructional moments, and require content-aware coordinates that point to the actual control, probe, terminal, connection, display, or safety state for each procedure step. A generic center callout is invalid. Automatic visuals never authorize invented labels, altered values, or decorative images. A lecture may use the explicit app-owned `no-purposeful-visual` decision only when the material genuinely gains nothing from a visual; the packet must explain why. There is no image quota. Add a visual when it answers a learner question, prevents a named misconception, or supplies necessary spatial/procedural evidence, and omit decoration or redundant copies. Figure economy is part of the contract: a screenshot of code or text already typed into the guide is redundant; two figures back to back are a failure of weaving; the paragraph after every `**What to notice:**` line must read specific elements off the figure and continue the argument, never paraphrase the notice line or discuss how the figure was drawn; and one number in any figure that shows a trace or computation is verified in the prose beside it.

Choose the most faithful and useful visual in this order:

1. a source slide, book figure, rights-cleared web figure, lab frame, or video frame cropped to the relevant region and recorded with a precise locator and attribution;
2. an annotation of that source visual that highlights the exact path, control, state, value, or misconception being discussed without changing source facts;
3. a deterministic technical diagram produced by the app's bounded declarative diagram language when precision matters;
4. a clearly labeled generated educational visual when it adds intuition and is not being used as factual evidence;
5. a table; then ASCII only when the medium truly communicates the idea better.

Put learner-visible files in `<GuideStem>_assets/`, never in `.gwverify`. The writer returns only an app-catalog asset ID and the exact heading anchor where it belongs. The app alone creates schema-2 `asset_manifest.json` and publishes only selected purposeful PNGs. Every selected asset must have exactly one manifest record, one PNG, and one standalone Markdown image line in its declared section. Surround the standalone image line with blank lines so it forms its own CommonMark paragraph. The next nonblank line must be the exact catalog `*Figure: ...*` caption and the following nonblank line the exact catalog `**What to notice:** ...` explanation. Raw HTML and reference-style image syntax are forbidden. Its app-owned record binds the visual need, source units or procedure steps, SHA-256, specification SHA-256, dimensions, provenance, rights basis, reuse scope, attribution, and transformation history. Remote URLs are inert provenance labels and are never fetched during generation; arbitrary SVG, HTML, scripts, or commands are not visual inputs.

When a launcher keeps writers read-only, return its required typed artifact block after the complete guide. The launcher must validate and materialize the coverage manifest and asset manifest into a hidden job staging area, then publish the learner assets, verification evidence, and guide together with the guide last as the commit point. Do not attempt unauthorized file writes or place learner assets in the verification directory.

Use source crops only within the permitted educational context and preserve attribution and rights metadata. Generated visuals are explanatory supplements, must be labeled as generated, and cannot stand in for factual source evidence. A figure-heavy source unit is not covered until its important spatial information is explained in prose. Duplicate bytes, duplicate render specifications, duplicate learning purposes, decorative images, and visuals placed outside the section teaching their linked source unit are failures.

## Course-Specific Contracts

### Circuit Theory and Measurement Lab

Treat the complete LMS announcement, that Friday’s instructor video, its timestamped transcript, and the assigned handout as the authority for the week. Do not infer that every experiment in a general manual is assigned. Circuit Lab cannot opt out of visuals. Before guide research or writing, the capture phase must hash-bind the announcement and actual video-element frames, preserve the transcript, require every procedure step to cite supporting transcript-segment IDs, locate the real point of interest in each frozen frame, pass those targets through a separate visual-review phase, and define the complete app-owned procedure inventory. Copy every step's `id`, `kind` (`physical` or `conceptual`), `guide_anchor`, and `need_ids` into `coverage_manifest.lab_steps` exactly, in order. Never omit, add, relabel, downgrade, or move a step. Then teach each contracted step with this sequence:

The `action` field is part of the immutable ordered procedure contract. Copy it exactly into `coverage_manifest.lab_steps`; explanations may expand around it but may not replace or paraphrase it.

1. **Goal** — the observable result of the step.
2. **Exact action** — equipment, terminal, range, setting, probe placement, and order of operations.
3. **Annotated frame** — a learner-visible image with the relevant control or connection marked.
4. **Expected state** — what the instrument, circuit, or software should show before continuing.
5. **Common wrong state** — the likely wiring, range, grounding, polarity, or reading error and how to recognize it.
6. **Recovery and verification** — how to return safely to a known state and prove the step succeeded.

Include safety and power-off boundaries where relevant. Never invent measured values; distinguish instructor-supplied expected values, theoretical predictions, and measurements the student must collect.

Every physical step requires a unique `primary_visual_asset_id` whose selected asset is app-rendered `annotated-source` evidence, bound only to that step, backed by a source unit explicitly mapped to that step or a frozen video frame that names that step, and embedded in the contracted section. A deterministic or generated diagram may supplement it but cannot replace it. Every supplemental image must also name the same step and section. Add distinct before, action, expected, wrong-state, and recovery images whenever they materially clarify the procedure; more useful Circuit Lab pictures are encouraged, but repeated or decorative images do not count.

### Scientific Writing

Create one guide per course week. Preserve an existing verified week unless the user asks for repair. Week-numbered files belong to that week; unnumbered files in the Scientific Writing course folder are shared support and must be considered for every new week. Set `guide_kind` to `scientific-writing-week` and record the week, discussion prompts, and assignment checkpoints in the coverage manifest. Build later weeks on the previous week’s vocabulary, paper, rubric, and discussion work. Separate the instructor’s requirements from textbook guidance, APA rules, sample-paper observations, and your own teaching explanation. Make guides discussion-ready: include close-reading prompts, evidence-to-claim exercises, revision decisions, likely group questions, and assignment checkpoints. Log conflicting dates or rubric wording instead of silently choosing one, and never invent instructor intent.

## Consistency of Examples and Final Prose

Before returning the guide, check every worked example, practice answer, cross-reference, and summary against the qualifications established in the explanation. State the assumptions of a simplified model beside any numerical bound or guarantee derived from it; never silently turn that model into a guarantee about a real implementation. Keep the named actors, quantities, and cause-and-effect relationships consistent throughout. An earlier caveat does not excuse a contradictory later answer.

Return finished teaching prose only. Do not narrate writing, repair, image selection, missing illustrations, or decisions not to invent a figure. A section without a selected image is simply prose: it must contain no image-like placeholder or explanation of that absence. Perform the review silently; the response begins with the guide title and ends with the required private artifact block, without a preamble or afterword.

## Completion Rule

Only the native gate may append `<!-- guide-watcher:complete:v1 -->`. It may do so only after:

- the source hash still matches the pre-generation snapshot;
- every source unit appears exactly once in the coverage manifest and maps to the guide;
- cumulative-continuity, source-conflict, unresolved-gap, visual-plan, and computational-verification-plan fields are present;
- every embedded learner asset exists, stays inside the learner asset folder, and has complete accessibility and provenance metadata;
- supported scalar arithmetic passes the fixed app-owned checker and more complex traces survive an independent recomputation pass;
- depth, structure, and prose-hygiene checks pass; and
- the completion-marker write succeeds atomically.

If any later repair fails a gate, the marker must be absent. A marker written by a model is not evidence of completion.
