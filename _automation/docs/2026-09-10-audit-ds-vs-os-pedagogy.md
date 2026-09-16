# Pedagogy audit: OS Lecture 2/3 guides vs. the three reference guides

Files audited (all read end to end):

- OS2 = `5th Semester/Operating Systems/2-What_is_OS_Guide.md` (1708 lines, 28 sections, 38 figures)
- OS3 = `5th Semester/Operating Systems/3-From_Program_to_Process_Guide.md` (967 lines, 27 sections, 32 figures)
- DS12 = `4th Semester/Data Structure/Week_12_-_Graph_-_Part_1_Guide.md` (3085 lines, 0 images, 224 fenced blocks)
- DS5 = `4th Semester/Data Structure/Week_5_-_Linked_List,_Tree_Guide.md` (3175 lines, 0 images, 274 fenced blocks)
- VM16 = `4th Semester/System Programming/16_VM_Systems_Guide.md` (1807 lines, 0 images, 90 fenced blocks)

Line numbers below refer to these files.

---

## 1. Figure pattern

**The reference guides contain no images at all.** Every visual is an ASCII block placed at the exact point the argument needs it, and the sentence after it *uses a specific cell or element*:

- VM16 L154-166: TLB table, then "How to read this: in set 0, way 1, there is a valid entry with tag `09` mapping to PPN `0D`. So if the CPU ever generates a VA whose VPN has TLBI = 0 and TLBT = 09, that's a TLB hit." Then L168-174 makes the reader compute one lookup against the table before moving on.
- DS12 L121-137 (5-vertex graph) is followed by an edge inventory table (L145-156), a degree table (L160-166), and an arithmetic check `3+3+4+4+6 = 20 = 2×10` (L168) that verifies the drawing.
- DS5 L185-216: the diagram is redrawn *inside* the trace (Before / Step 1 / Step 2 / Step 3), so the visual is the intermediate state, not an illustration of it.

**The OS guides: 70/70 figures use the identical triplet** (image / `*Figure:*` / `**What to notice:**`; counts verified 38=38=38 and 32=32=32). Classification, where (a) = surrounding prose names specific elements and the argument continues from them, (b) = triplet present but the prose does not otherwise use the image, (c) = two or more figures within ~12 lines of each other.

| Guide | (a) woven | (b) captioned-only | (c) back-to-back clusters |
|---|---|---|---|
| OS2 | 25 | 13 | 4 clusters (L476/L488, L592/L605, L881/L891, L1052/L1060) |
| OS3 | ~30 | ~2 | 8 clusters covering 17 of 32 figures (L152/162, L196/204, L250/258, L312/320, L334/346, L384/397, L558/566, L777/785/793) |

Representative (a): OS2 L180→L186-188 (reads Task boxes / OS bar / arrows, then corrects the "every instruction goes through the kernel" misreading); OS2 L737→L743-751 (CSAPP Fig 9.9, counts the four arrows and finds two landing on PP 7); OS3 L232→L238-240 (quotes the slide's byte string `0x55 0x89 0xe5 …` and the `0x401040 <sum>` presentation issue); OS3 L651→L657 (traces all four state transitions from the arrows).

Representative (b): **9 of OS2's 13 captioned-only figures are screenshots of code or terminal transcripts that the guide has already transcribed in full directly above them** — L537 (cpu.c, code at L504-525), L592 (four-copy transcript, L569-588), L605 (OSTEP page of the same cpu.c), L792 (mem.c), L834 (mem transcript), L1016, L1052, L1060 (thread.c three times in 44 lines), L1088 (thread results), L1259 (open/write/close). The prose after each discusses the *code*, not the image (e.g. L598 "Two precise points" are about `& ;` notation and concurrency-vs-parallelism, both invisible in the picture). Also L869 (bulk order): L875 immediately switches to defining response time without touching the figure.

Representative (c): OS3 L777/L785/L793 — three images of `struct proc` within 16 lines, each with its own triplet; OS3 L312/L320 — lecture ELF stack then CSAPP ELF stack, prose at L326 adds only "illustrates this same relocatable object file".

**Two further observations specific to the pattern.**

1. The `**What to notice:**` line is frequently a one-sentence paraphrase that the next paragraph then repeats. OS3 L562 ("The timeline separates repeated context switches from the scheduling policy") vs L564 ("The timeline shows processes A, B, and C running in turn, with a Context Switch arrow at every boundary and a Scheduling Policy brace"). OS2 L541 vs nothing (subsection ends). This is the formulaic feel the user senses: the scaffold is filled even when there is nothing to fill it with.
2. The 7 *original* diagrams (OS2 L284, L448, L705, L937, L1417; OS3 L397, L704) are the closest to reference practice, but each is followed by 5-9 paragraphs of design rationale rather than concept: OS2 L290 "a numbered list is a poor medium for…", L456 "They are two panels rather than two lines on one frame on purpose", L945 "the legend sits above rather than relying on bar order". VM16 never explains why a diagram was drawn; it just uses it. The hand-enrichment also left seams: OS2 L947 and L953 make the same "students skip" point twice; OS2 L1512-1518 restates L1490-1500 almost verbatim; OS3 L396-397 and L411-412 are missing blank lines; OS3 L360 has an unclosed quotation mark.

Verdict: the triplet is not harmful where the figure is a diagram and (a) applies (~55 of 70). It is dead weight for code/transcript screenshots and for the second member of every back-to-back pair. The reference guides' answer to "is a picture needed here?" is "only if the next sentence reads a cell off it."

---

## 2. Define-before-use

### OS2

| Term | First use | Defined | Gap |
|---|---|---|---|
| privileged / hardware protection | L53, L188, L246 | L264 (user/kernel mode) | 210 lines |
| system call | L198, L202 (§3.1) | L250, L263 (§4.2) | 50 |
| mechanism / policy | L216, L222 (crux box) | L326-327 (§5) | 110 |
| thread | L172, L224 | L975 (§17) | 800 |
| address space | L196, L310, L345 | L633 (§11.2) | 440 |
| context switch | L352 | L418 (§8) | 66 |
| time slice / quantum | L344, L350 | L426 | 80 |
| page / evict / page table | L345-346, L358 | page: L682; page table: **never** (used L684, L719, L733, L749 as a known object) | 340 / never |
| file descriptor, descriptor table, fd 0/1/2 | L266, L300 | L1251 (§23) | 985 |
| data race | L875 ("not a data race") | L1022 | 150 |
| interrupt / timer interrupt | L80, L156, L188, L343 | **never** (only used) | — |
| cache, cache locality, instruction cache | L168, L460, L464 | **never** | — |
| TLB ("hardware cache of translations") | L168, L719, L1403 | **never named or defined** | — |
| limited direct execution | L188 | never (explicitly deferred to a later lecture at L188/L314) | — |
| sequential consistency | L172 | **never** | — |
| hypervisor | L384 | **never** | — |
| fork | L735 | **never** in OS2 | — |
| page cache | L1297, L1666 | **never** | — |
| makespan | L947 | **never** | — |
| scheduler | L202, L327 | implicit only | — |

Terms handled correctly (defined at first use): kernel L53, CPU/core L49, program counter L86, Von Neumann L91, trap L264, user/kernel mode L264, protection fault L304, busy-wait L545, ASLR L842, response time / head-of-line blocking L875, undefined behavior L1022, inode L1332, journaling L1312, pipe L1496.

### OS3

| Term | First use | Defined | Gap |
|---|---|---|---|
| register, stack | L44 | L158 / L168 | 115 (L48 promises re-teaching, which is honest) |
| PCB (unexpanded acronym) / process list | L429 (§15), L590 | L759 (§24) | 330 |
| Ready / Blocked | L487, L509 (§18-19) | L647-649 (§23) | 160 |
| blocking events | L564 | L649 | 85 |
| trap frame (`tf`) | L783 | L845 | 62 |
| symbol table / relocation record | L240 | loosely L300 | 60 |
| kernel | L60, L170 | **never** (assumed from OS2) | — |
| trap, "hardware traps into the kernel" | L629 | **never** in OS3 (OS2 L264) | — |
| interrupt / timer interrupt | L144, L443, L564 | **never** | — |
| user/kernel mode bit, protection fault | L376 | **never** in OS3 | — |
| system call interface | L481 | **never** in OS3 | — |
| page, paging | L342, L629 | **never** | — |
| quantum | L531 | **never** in OS3 (OS2 L426) | — |
| thrashing | L531 | **never** | — |
| calling convention, callee/caller-saved | L224, L845-847 | **never** (used as known) | — |
| System V ABI | L240 | **never expanded** | — |
| position-independent code | L296 | **never** | — |
| little-endian | L294 | **never** | — |
| brk | L374 | **never** | — |
| signal | L583, L586 | loosely L586 ("kill sends a signal") | — |
| kernel stack (`kstack`), page directory / page-table root | L783, L851 | **never** | — |
| `.init` section | L340 | **never** | — |

Handled correctly: process L42-44, ISA/microarchitecture L100, CISC/RISC L90, PC/EIP/RIP and condition codes L158, prologue/frame L222, `static` linkage L281, name mangling L304, ELF L310, section vs segment L326, thread L420, machine state L423, logical control flow L445, file descriptors L604, page fault L629, zombie L773, utilization L676.

The structural problem: OS3 L48 says "this guide re-teaches every prerequisite it needs" but relies on OS2 for trap, quantum, kernel mode, and system call; OS2 L45 says "there is no earlier guide to lean on" and yet uses the crux box's own vocabulary (mechanism, policy, thread) 100-800 lines before defining it. The reference guides define on first contact: DS12 L113-115 (endpoint, incident, adjacent, degree, isolated all defined in the sentence they first appear), VM16 L46-57 (N, M, P defined before any address is written).

---

## 3. Analogies and intuition

| Concept | OS2 | OS3 | Quality |
|---|---|---|---|
| Program vs process | Cookbook/chef L47, before formal | Recipe card/cook L42, before formal L44 | Structural (card=program, cooking=execution, whole activity=process). Good. |
| Fetch-decode-execute | Ice-cream mapping L65-72 before formal L82 | none | Structural, six-way mapping, limits stated L80. Good. |
| System call / trap | Kiosk + wall L236-246 before formal L248; guide extends it at L270 (analogy never shows the return path) and L304 (knocking vs caught in the act) | none (assumed) | Structural. Best analogy in either guide. |
| Time-sharing | One worker in time slots L408; translucent workers L416 | none | Thin but structural; the real bridge is the arithmetic at L424+. |
| Address space / memory virtualization | Shared bowl, "same name, different bowl" L619-625 before formal L633 | none | Structural (name=virtual address, bowl=frame). Good. |
| Virtualization in general | "own" → "illusion" + "indirection" L368; "virtual machine" L196 | "conjure a private machine" L471 | No analogy. Abstract in both. |
| Mechanism vs policy | Definitions first L326-327; operational test L335 comes after | Definitions L553-556; substitutability argument L564 | No analogy anywhere. The test "possible vs choose" is good but arrives post-definition. |
| Limited direct execution | Named L188, deferred | Named L963 | No bridge. |
| Concurrency / race | Bulk-order analogy L867 is for *scheduling*; guide itself says L875 "not a data race". thread.c race goes straight to load/inc/store at L1108 | none | **Missing.** No everyday bridge for the lost update (two clerks both reading the same balance) anywhere. |
| Persistence | Refrigerator L1197, hedged L1207 | none | Decorative; guide says so. |
| Context switch | none | none (swtch described L572) | **Missing.** No bookmark/pause-and-resume bridge. |
| ISA vs microarchitecture | — | "same binary on laptop and server" L104, after definition L100 | Familiar-experience bridge, decent. |
| Time vs space sharing | — | Disk-block contrast L515 + table L519-525, after definition | Concrete contrast, good; the "oversubscription symptom" row L527 is the kind of intuition the reference guides lead with. |
| Lazy loading | — | "much of a program is never touched" L631 | Reasoning, not analogy; adequate. |
| PCB / process states | — | "sleeping vs busy-waiting" L659 | Contrast only. |

Reference comparison: DS12 L1460-1466 gives maze/string/paint *then* the explicit mapping "The string is the recursion stack. The paint is a vertex/edge label" *then* pseudocode; L2007 ripple-in-a-pond before BFS pseudocode. DS5 L59-61 lockers vs scavenger hunt (locker number=index, clue=pointer) before any code; L2650 flooding a tree; L2834 tracing an outline with a finger for Euler tour. VM16 L39 "you can hand-trace a 14-bit system... the 64-bit one is just more of the same" (scale bridge), L1399 COW as "lazy evaluation" (bridge to a known CS idea), L1640 "treat a file like an array". Every reference analogy is placed *before* the formal object, and each states the correspondence.

OS2 does this competently because it inherits and maps the lecture's ice-cream analogies. OS3 has exactly one analogy (L42) in 967 lines; everything from §3 to §24 is introduced by definition or by reading a slide.

---

## 4. Worked examples

Counting only instances traced with intermediate state or a concrete computed result. Labels in parentheses.

### OS2

| Section | Count | Notes |
|---|---|---|
| §2 | 3 (M/H/C) | L106 table, L126 PC table, L158 count. L170/L172 are labeled hard/challenging but are caveats, not examples. |
| §3 | 0 | §3.1's "exam-style question" L202 is answered inline, no trace. |
| §4 | 2 + 1 argument | L300, L302 traces; L306 is an argument. |
| §5 | 3 (M/H/C) | Classification exercises, no state. |
| §6, §7 | 0 | §6 teaches indirection with no example. |
| §8 | 3 (M/H/C) | L432, L438, L464 computations. |
| §9 | 0 | 8 tasks / 4 cores is only worked in Q4 at L1609. |
| §10 | 1 (C) | L611 thought exercise; transcripts are evidence, not traces. |
| §11, §12 | 0 | §12 teaches load/store addressing with no address until §13. |
| §13 | 4 (H/M/H/C) | L676, L689, L721, L729. Strongest section. |
| §14, §15 | 0 | §15 defines concurrent vs parallel with no interleaving shown until §20. |
| §16 | 3 (M/H/C) | L913, L925, L955 tables. |
| §17, §18, §19 | 0 | §17 introduces shared address space with no picture of two stacks in one space. |
| §20 | 4 | L1116 table, L1145 arithmetic, L1155 (H), L1163 (C) table. |
| §21, §22 | 0 | |
| §23 | 3 (M/H/C) | L1277, L1285, L1293. |
| §24 | 3 (M/H/C) | L1330, L1336, L1346. |
| §25 | 3 (M/H/C) | Argument-style scorecards. |
| §26 | 3 (M/H/C) | Reasoning essays. |
| §27 | 12 Q&A | |

Mechanism/computation sections with zero examples: **§6, §9, §12, §15, §17, §22**.

### OS3

| Section | Count | Notes |
|---|---|---|
| §5 | 1 real + 2 nominal | L127 (M) and L129 (H) are one-sentence descriptions with no state; only L131 (C) is worked. |
| §7 | 1 | L186 `1+2=3`. |
| §8 | 1 | L224 register trace. |
| §9 | 0 | Assembling: no "encode this one instruction" example. |
| §10 | 3 (M/H/C) | L270, L283, L298. |
| §11 | 0 | ELF format: no header/section byte example. |
| §12, §13, §14 | 0 | Loading / memory image: no concrete file-to-address layout. |
| §15 | 1 | L427, no state. |
| §16, §17, §18 | 0 | |
| §19 | 1 (M) | L529 classification. |
| §20 | **0** | Context switch, the central mechanism of the lecture: `swtch` described at L572 but never traced with register values. |
| §21 | 1 | L588 shell. |
| §22 | 3 (M/H/C) | L612, L625, L633. |
| §23 | 3 (M/H/C) | L669, L678, L720 tables. Strongest section. |
| §24 | 3 (M/H/C) | L830, L843, L849. |
| §25 | 10 Q&A | |

Mechanism sections with zero examples: **§9, §11, §12, §13, §16, §17, §18, §20**.

### Density comparison

- DS5 has a state trace in essentially every operation section (§3 L144-163, §4 L185-216 and L223-280, §5, §6, §7, §8, §10 L688-697, §11 L781-791, §12 L892-927, §13 L1089-1104, §19 L1419-1498, §20-§21, §25-§26, §32 L2703-2822, §33 L3004-3093). DS5 also uses the exact labels "Example 1 (Medium) / Example 2 (Harder) / Example 3 (Challenging)" (L223, L242, L252) — the OS triplet is inherited from here. The difference is length: DS5 examples are 5-15 lines of state; OS examples are 10-30 line essays.
- DS12: §3, §4 (5 path/cycle examples), §5, §6, §7, §14.7, §16 (both traversals), §18 (9-step DFS with stack per step), §20.1, §21.2, §23, §24.4, §26.3, §28.1, §29 (4). No mechanism section lacks a trace.
- VM16: one unlabeled example per section (§2.3, §3.2, §4.3, §5, §6 + §6.8, §9.4, §10.4, §11.5, §13.5, §14.4 ×3, §17.4, §20.4), always in the same section as the mechanism.

The OS guides cluster examples into "Three Worked Examples" subsections (11 such triplets in OS2, 5 in OS3) and leave the intervening definition-heavy sections dry. The reference guides spread one example per mechanism.

---

## 5. What the reference guides do that the OS guides don't (or do less)

1. **Verify the visual arithmetically right after showing it.** DS12 L168: "The sum of all degrees is 3+3+4+4+6 = 20 = 2×10, matching Σdeg(v)=2m... any time you draw a graph and the degree sum is not exactly 2m, you have miscounted." DS12 L319-323 does the same for in/out-degree. OS: OS2 L696 checks one split arithmetically, but no slide reproduction is ever checked against a computation.

2. **Ask a question, make the reader answer, then answer.** VM16 L211-215 "Pop Quiz... Which VPNs in the first 16 are not present? `01, 04, 06, 07, 0B, 0C`"; VM16 L340 "> Why not check the page table? Because the TLB is a cache..."; DS12 L1287 "When does each one win?". OS: present but rare — OS2 L731 "Which process wrote it?", OS3 L356 "where did the two extra regions come from?". Most OS paragraphs assert first.

3. **Show the bug or wrong intuition first, then the fix.** DS5 L242-250 "What if you accidentally swap steps 2 and 3? ... `newest.next = newest` (circular!)"; DS5 L345-356 dangling `tail`; DS12 L2000 "Forgetting it gives you a false cycle of length 2 for every edge". OS: does this well in places — OS2 L438 "a student often reasons: make the slice tiny", L1597 Q1, OS3 L824 five-field `struct context`. Present.

4. **State the iteration/tie-break rule before the trace.** DS12 L1361 "assume each vertex's neighbor list is iterated in numeric order", L1549, L2716. OS: OS2 L1116 states the interleaving assumption; OS3 L669 states units. Partial — but no OS trace is order-dependent in the way traversals are, so this is lower priority.

5. **Fixed-format state snapshot at every step.** DS12 L1563-1639 prints `Vertex labels: ... Stack: [A, B, D]` after each step; VM16 L374-388 ends the trace with a summary table and "If you can reproduce this table cold, you have the simple memory system mastered." OS: tables at OS2 L117, L141, L1118, L1171 and OS3 L671-698 do this. Absent for the two mechanisms that most need it: context switch (OS3 §20) and loading (OS3 §12).

6. **End every section with a boxed "insight" that names the pattern.** DS12 has 27 `★ Insight` boxes, VM16 has 20; each is 2-4 bullets like "The Σdeg(v)=2m identity is the *reason* DFS is O(n+m)" (L1742). OS: "The pattern to recognize:" appears (OS2 L168, L348, L1283; OS3 L281, L533, L841) but buried mid-example; sections end on a caveat or a transition sentence, never a summary. Not done.

7. **Point at one specific cell after every table.** VM16 L164 "in set 0, way 1, there is a valid entry with tag `09` mapping to PPN `0D`"; L201 "VPN `0x03` maps to PPN `0x02`"; L279 "line 5 has tag `0x0D`, is valid, and holds `[36, 72, F0, 1D]`". OS: the `What to notice` line is the analogue but usually generic ("The indented loop ties one second of active spinning to one printed label", OS2 L541). Cell-specific only at OS2 L741, OS3 L238. Mostly not done.

8. **Put the diagram inside the trace, redrawn per step.** DS5 L185-216 (Before / Step 1 / Step 2 / Step 3 / Result). VM16 L1343-1378 redraws the COW picture before and after the write. OS: original charts (OS2 L937, OS3 L704) come *after* the tables as a single summary; slide reproductions are static. Not done.

9. **Unsolved practice with new inputs at the end.** VM16 L1799-1807: "Re-do address translations #1 and #2 with the book closed. Then try VA = `0x0040`, `0x008C`, `0x0214`." OS: exam questions come with full solutions only; nothing is left for the reader to do. Not done.

10. **Scale bridge / deliberately tiny machine.** VM16 L39 "you can hand-trace a 14-bit system... the 64-bit one is just more of the same." OS2 L682 "Set up a deliberately small machine so the numbers stay readable" — done in §13, not elsewhere.

### Summary of the gap

The OS guides are stronger than the references on *precision* (misconception hedges, source discrepancies, undefined-behavior caveats) and roughly equal on the strong sections (OS2 §13, §16, §20; OS3 §10, §22, §23). They are weaker on (i) terminology order — 10 acronyms/terms used without ever being defined in OS3, 8 in OS2, plus several 100-1000-line gaps; (ii) analogies in OS3, which has one; (iii) examples for the definitional sections and, critically, for the context switch; (iv) figure economy — 9 code screenshots and 8 back-to-back pairs add scaffolding without information; and (v) section endings, which never crystallise the pattern the way the reference insight boxes do.
