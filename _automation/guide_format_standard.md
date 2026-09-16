# Guide Formatting Standard

The same explanation reads twice as fast when its numbers, states, and steps are visually separated from the sentences around them. These rules are mandatory for every guide. Every primitive used here (blockquote callouts with a bold icon-led label, tables, inline code, `$...$` KaTeX math, numbered steps) renders identically in Markdown Preview Enhanced, VS Code's built-in preview, Silk MD, and GitHub; nothing viewer-specific (GitHub `[!NOTE]` alerts, `!!! note` admonitions, raw HTML) is used. The verifier's self-check (`guide_lint.py --advisory`) reports violations under FORMATTING; the writer fixes them before finishing.

## F1. Values, symbols, and expressions never sit bare in a sentence

Every variable, index, array, expression, or equality is set in inline code: `j = p − 1`, `A[0..i]`, `k − p = 3` shifts, `L = [1, 3, 4, 5]`. A sentence may mention *one* such item inline; a statement with two or more becomes a list item, a table cell, or a display line of its own.

Wrong: "Values smaller than 2: one, so p = 1. The scan shifts k − p = 4 − 1 = 3 values (5, 4, 3) and exits with j = p − 1 = 0."

Right:

- smaller than `2`: one value → `p = 1`
- shifts: `k − p = 4 − 1 = 3` values, namely `5, 4, 3`
- exit: `j = p − 1 = 0`

## F2. State is a table, a trace is numbered steps

Whenever three or more variables change across steps, show a table with one column per variable and one row per step (before and after). A sequence of actions is a numbered list, one action per item, the changed value in code. Never narrate a trace inside a paragraph.

## F3. Every worked example has the same visible skeleton

```
> **🧮 Worked example (medium) — <what it shows>**
>
> **Given.** `L = [1, 3, 4, 5]`, `x = 2`, `i = 4`
>
> **Steps.**
> 1. …
> 2. …
>
> **Result.** `[1, 2, 3, 4, 5]`
> **Check.** `p + (k − p) + 1 = 1 + 3 + 1 = 5` cells ✔
```

Difficulty is one of medium / harder / challenging and appears in the title. Given, Steps, Result, and Check are always present. A "Try it yourself" item lists its answers on one line in code.

## F4. Definitions, pitfalls, and closers are callouts

- `> **📘 Definition — <term>.** …` at the sentence that defines a term (the term bold exactly here).
- `> **⚠️ Misconception.** …` where a wrong intuition is corrected.
- `> **💡 Pattern to recognize.** …` as the closing paragraph of every teaching section.
- `> **📝 Note.** …` for a caveat or a source discrepancy.

A callout is a blockquote whose first line is a bold, icon-led label. This renders as a distinct panel in every Markdown viewer (GitHub, VS Code, Silk MD); GitHub `[!NOTE]` alert syntax is not used because several viewers print it literally.

## F5. Practice items are separated and answerable without reading the answer

Each item is its own `### Qn (difficulty) — <topic>` heading. The question is a blockquote. A `**Solution.**` paragraph follows after a blank line, with the final answer in bold on its own line first, then the reasoning. Items are separated by `---`.

```
### Q3 (hard) — links in a full mesh

> A fully connected mesh has eight devices. How many links, and how many ports per device?

**Solution.** **28 links, 7 ports each.**

- links: `n(n − 1)/2 = 8 × 7 / 2 = 28`
- ports per device: `n − 1 = 7`

---
```

## F6. No paragraph longer than 120 words; every section has sub-structure

A paragraph that passes 120 words is split at its argument's joints or turned into a list. Every `# N.` section has at least one `##` subsection or one callout; a section that is one continuous run of paragraphs is not finished.

## F7. Math notation

Identifiers, array states, and simple expressions use inline code (`j`, `A[0..i]`, `n(n − 1)/2`). Use `$…$` LaTeX (KaTeX renders it) for genuinely mathematical statements: sums, fractions, asymptotic bounds, invariants written as formulas — `$\sum_{i=1}^{n} i = \frac{n(n+1)}{2}$`, `$T(n) = O(n^2)$`. Never write a formula bare in a sentence, and never mix the two forms for the same quantity in one section.

## F8. What never appears in the body

- Slide narration: "Lecture PDF page 22 returns to…", "PDF page 24 attaches a green arrow…", "slide 60 is a single centred prompt". Provenance belongs in figure captions and the manifest; the body teaches the idea, not the deck.
- Reading instructions: "First, read Chapter 2 at PDF pages 44–49". The guide *is* the reading. A short "Sources used" list at the end (title, chapter) is the only reference to the book allowed outside citations.
- Sections invented for administrative slides (title, outline, "Any questions?", thank-you). Map those units in the coverage manifest to the nearest real section; write nothing about them.
- Prose about the guide itself ("this section will…", "as promised above…").
