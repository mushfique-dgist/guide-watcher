# Guide Watcher — setup instructions for an AI assistant

You are reading this because someone handed you this zip and asked you to get Guide Watcher running on their machine. Follow it top to bottom. Everything below was true for the reference machine on 2026-09-11 (Windows 11, Rust 1.94, Node 22.16, Python 3.13.3, Claude Code CLI 2.1.266); where a step depends on the new machine, it says so.

Guide Watcher turns a course folder of lecture PDFs (plus textbooks, past exams, and collected lecture transcripts) into long, verified Markdown study guides. It is a Tauri v2 desktop app (Rust backend, Svelte frontend) with a headless CLI. It drives two external model CLIs: **OpenAI Codex CLI** (`codex`, ChatGPT login) for phase 1 (reading slides and books, ranking figures, collecting context) and **Claude Code CLI** (`claude`, Anthropic login) for phase 2 (writing, deepening, repairing) inside a locked-down tool harness. It owns every file write, verifies every guide with a Python verifier, and only then seals it.

## 0. What is in the zip

```
guide-watcher/            the app: src/ (Svelte), src-tauri/ (Rust), scripts/, tests/, README.md, this file
_automation/              files the app reads at run time (they are hashed and frozen per job):
  guide_prompt.txt          the writer template (teaching rules, formatting standard, output contract)
  guide_depth_contract.md   the quality contract every guide must meet
  guide_format_standard.md  the formatting standard (also inlined in the template)
  guide_lint.py             the verifier/gate and the writer's self-check (--advisory)
  guide_format.py           standalone entry point for the formatting checks
  render_slides.py          renders PDF/PPTX pages to PNG for the vision phase
  requirements-verifier.txt pinned Python dependency of the verifier (markdown-it-py==4.2.0)
  supplementary_collect.py  deterministic lecture-transcript collector (used by `supplementary collect`)
  test_*.py                 unit tests for the Python parts
  docs/                     the 2026-09-10 audit, lecture-technique study, and roadmap
```

Not included, on purpose: `node_modules/`, `src-tauri/target/` (build output, ~11 GB), `.git/`, and two optional integrations that live in other repositories (the DGIST LMS agent used only by the Circuit Lab course, and the `faster-whisper` transcription helper). The app builds and runs without them for ordinary lecture courses.

## 1. Prerequisites to install on the new machine

1. **Rust** stable toolchain (rustup; `rustc 1.94` or newer) with the MSVC target on Windows.
2. **Node.js 22** and npm. Then `npm install` inside `guide-watcher/`.
3. **Tauri v2 Windows prerequisites**: Microsoft Visual Studio Build Tools with the "Desktop development with C++" workload, and the WebView2 runtime (present on Windows 11). NSIS is fetched by the Tauri CLI automatically.
4. **Python 3.13** on `PATH` as `python`, with: `pip install pymupdf==1.27.1 markdown-it-py==4.2.0 youtube-transcript-api==1.2.4 requests`. The verifier refuses to run if `markdown-it-py` is not exactly the version in `_automation/requirements-verifier.txt`.
5. **Claude Code CLI**: install per Anthropic's instructions and log in (`claude` must work in a terminal). The app looks for `claude.exe` in `%USERPROFILE%\.local\bin`, the npm global directory, and WinGet links. The writer model is `claude-opus-4-8`; the account must have access to it.
6. **OpenAI Codex CLI**: `npm install -g @openai/codex`, then `codex login` (ChatGPT account). The app looks for it in `%LOCALAPPDATA%\OpenAI\Codex\bin`, `%LOCALAPPDATA%\Microsoft\WinGet\Links`, the npm global package's vendored binary, and the Codex desktop app. Models used: `gpt-5.6-luna` (default), `gpt-5.6-terra`, `gpt-5.6-sol` (fallbacks).
7. **Git Bash** (comes with Git for Windows). The writer harness runs `./verify_draft.sh` through Claude Code's Bash tool, which uses Git Bash on Windows.

## 2. Paths and course settings you MUST change

Everything machine-specific is a constant in `guide-watcher/src-tauri/src/config.rs`. Edit these before building:

| constant | what it is | set to |
|---|---|---|
| `WATCH_DIR`, `FIFTH_SEMESTER_ROOT` | the semester folder the app watches and scans | the user's semester folder |
| `FOURTH_SEMESTER_ROOT` | the legacy "previous semester" root (only used by the legacy profile) | any existing folder, or the same as above |
| `TEMPLATE_FILE`, `DEPTH_CONTRACT_FILE`, `RENDER_SLIDES_SCRIPT`, `GUIDE_LINT_SCRIPT`, `GUIDE_LINT_REQUIREMENTS`, `SUPPLEMENTARY_COLLECT_SCRIPT` | the six `_automation` files | wherever you unpack `_automation/` (use forward slashes) |
| `DGIST_LMS_*`, `DGIST_FFMPEG_EXECUTABLE`, `DGIST_FFPROBE_EXECUTABLE`, `TRANSCRIPTION_*`, `CIRCUIT_LAB_*` | Circuit Lab / video transcription only | leave as is unless that course is used; they are never touched for lecture courses |

Course profiles are in `src-tauri/src/course_plan.rs`, function `configured_courses()`: each entry is `(id, display name, folder = "<FIFTH_SEMESTER_ROOT>/<Course Folder Name>", guide mode, guide kind, lecture-filename rule, order)`. The reference set is Computer Networks (`CH*.pdf`), Computer Algorithms (`L*.pdf`), Operating Systems (numbered `N-*.pdf`), Circuit Theory & Measurement Lab (weekly, needs the LMS agent), Scientific Writing (weekly material). For a new user: rename folders to match, or edit the entries (and the matching `LecturePrimaryRule` variants and `DENSE_DEPTH_COURSE_PROFILES` / `--expected-course-profile` choices in `guide_lint.py`) to their courses. The primary-textbook detection for Operating Systems (`is_primary_supplementary_resource` in `codex.rs`, title "Operating Systems: Three Easy Pieces") is the only book-specific rule.

`guide_lint.py` has no hardcoded paths. `supplementary_collect.py` has none either.

## 3. Build, test, install

```powershell
cd guide-watcher
npm install
npm run test:frontend                      # 61 JS tests
cargo test --manifest-path src-tauri/Cargo.toml --lib   # ~360 Rust tests
cd ..\_automation
python -m unittest test_guide_lint test_guide_format test_supplementary_collect test_render_slides
cd ..\guide-watcher
npx tauri build                            # produces src-tauri\target\release\bundle\nsis\Guide Watcher_2.0.0_x64-setup.exe
.\scripts\install-windows.ps1 -InstallerPath ".\src-tauri\target\release\bundle\nsis\Guide Watcher_2.0.0_x64-setup.exe" -ExpectedBinaryPath ".\src-tauri\target\release\guide-watcher.exe"
```

The install script runs in Windows PowerShell 5.1 or PowerShell 7, refuses to run while the app is open, and prints the SHA-256 of what it installed. It also installs `guide-watcher-cli.exe` next to the app (`%LOCALAPPDATA%\Guide Watcher\`).

## 4. How a course folder should look

```
<Semester>/<Course Folder>/
  CH01_Introduction.pdf              lecture decks (the filename rule per course decides which PDFs are lectures)
  <Textbook>.pdf                     any other PDF is treated as a supplementary book (assessed, excerpted, frozen)
  <anything>exam<anything>.md        a past exam transcription: frozen, shown to the writer, and the guide MUST then
                                     contain an Exam-Style Practice section mirroring its question types
  _supplementary/manifest.json       collected lecture transcripts (see §6); only paragraphs matching the lecture are used
  CH01_Introduction_Guide.md         output; sealed with an HTML comment receipt when it passed every gate
  CH01_Introduction_Guide_assets/    the figures the guide embeds (PNG) + asset_manifest.json
  .CH01_Introduction_Guide.md.gwverify/   app-owned audit evidence (snapshots, coverage manifest)
  CH01_Introduction_Guide.prep.md    saved phase-1 context; present only while a job is resumable
```

Rules the app enforces: it never overwrites an existing guide, prep, assets folder, or verification folder (move the old one aside to regenerate); one job per course at a time; guides within a course are written in lecture order and read their predecessors.

## 5. Running it

- App: launch Guide Watcher; use Workspace to pick files or scan the semester; **Retry from sources** regenerates (after you move the old guide away); **Resume writing** reuses a saved `.prep.md`. History and the Overview log explain each step in plain language with a "What to do next" line on failure.
- CLI (same engine; runs do **not** appear in the app's History):
  ```
  guide-watcher-cli generate <lecture.pdf>
  guide-watcher-cli resume <guide.prep.md>
  guide-watcher-cli batch <source> [source ...]
  guide-watcher-cli supplementary plan|collect|status <course_dir>
  guide-watcher-cli visuals inspect|validate <source>
  ```
- A full run is roughly 60–90 minutes: phase 1 (vision batches over every slide and relevant book pages, global figure ranking, context synthesis) then phase 2 (writer harness → depth passes → pedagogy pass → native verification with up to 3 in-place repairs → publish).

## 6. Supplementary transcripts (optional, once per course)

`guide-watcher-cli supplementary plan <course_dir>` asks the cheap Codex model (with web search) to propose free lecture sources matching the course's PDFs and writes `_supplementary/candidates.json`. Review it, then `supplementary collect <course_dir>` fetches transcripts deterministically (YouTube captions, transcript pages, PDFs) and writes `manifest.json`, one `transcript.md` per item, and `collection_report.md` with a status for every candidate (ok / no_captions / blocked / unavailable / invalid / network_error) — nothing is skipped silently. Later guide runs quote only the paragraphs that match the current lecture, under a fixed budget, and refuse a transcript that changed after collection.

## 7. Operating rules (they matter)

- **Never edit the six `_automation` files while a job is running.** The app hashes them at preflight and re-checks before each phase; a change aborts the job. Adding new files to the folder is fine.
- The writer harness is the quality mechanism: Claude drafts `draft/guide.md` section by section, writes the artifact JSON to `draft/artifacts.json`, runs `./verify_draft.sh` (the frozen `guide_lint.py --advisory`: MUST FIX hard failures + ADVISORY teaching/formatting gaps), and fixes until it reports nothing to fix. The app then runs the full gate. Do not "help" by relaxing the gate; fix the prompt or the checker.
- Quality rules live in three places and must stay consistent: `guide_prompt.txt` (what the writer is told), `guide_depth_contract.md` (the contract), `guide_lint.py` (what is checked). `docs/2026-09-10-guide-quality-audit-and-roadmap.md` records why each rule exists.
- Costs: phase 1 uses the Codex subscription; phase 2 uses the Claude subscription (`claude-opus-4-8` at high effort for one draft, up to two deepening passes, one pedagogy pass, and repairs). A guide is 20–45k words.

## 8. Verifying the setup end to end

1. `cargo test --lib` and the Python tests pass.
2. `claude --version` and `codex --version` work in a normal terminal, both logged in.
3. Put one lecture PDF in a configured course folder and run `guide-watcher-cli generate <that pdf>`; expect phase-1 log lines (`inspecting source visuals k/n`), then `Claude writing`, `deepening pass`, `Pedagogy check`, `Native verification pass`, `RESULT: PASS`, and a sealed `<name>_Guide.md` with an assets folder.
4. Run `python _automation/guide_lint.py <guide> --advisory` on the result; it should print `[ADVISORY] nothing to fix`.

If step 3 fails at "preflight dependency changed", something edited an `_automation` file mid-run. If it fails with "refusing to overwrite", move the previous guide/assets/`.gwverify` aside. If Claude fails with a quota/availability error, the app falls back to Codex for writing; any other Claude error is reported as is with the real message in the log detail.

## 9. Behaviors added after the first package (2026-09-12/13)

- **Provider fallback.** A Claude run that ends without producing a guide is always eligible for the next Claude candidate and then the Codex fallback. Eligibility is structural, not a list of known messages: a non-zero exit (or an error result, or an empty result) means the CLI never wrote anything, and a bad guide exits zero and is caught later by the verifier instead. Only cancellation and content rejection stay terminal, plus failures that happen with Claude's output already in hand (an invalid artifact block, a write error). Recognized wordings - session, weekly and per-model limits ("You've reached your Fable limit. Switch to another model to continue."), logged out, transport failures - still set the reason the History entry shows.
- **One app instance.** Launching Guide Watcher while it is already running (including from the tray, or from a build folder rather than the installed copy) now hands the arguments to the running app and focuses its window instead of starting a second process. Two processes could otherwise fight over the same course and output locks.
- **It tells you when it wants you.** A guide takes hours, so the app no longer needs the window watched. When a run finishes, when one stops early, and when the watcher finds new lecture files, Guide Watcher raises a Windows notification and puts a red count on its taskbar icon (and on the tray icon, since a window closed to the tray has no taskbar button). The count clears the moment the window takes focus. Stops you asked for yourself raise nothing.
- **Honest time estimates.** The writing step used to promise 15 to 25 minutes for every lecture. It now scales with the deck: a 115-slide lecture is quoted about 34 minutes for the draft pass, and the message says up to three deepening passes of about the same length follow.
- **Simple by default, Power mode for everything.** `Settings & help` opens with one switch. Off (the default) the daily screens ask one question instead of ten: the review screen offers Quick / Balanced / Thorough, which set effort levels only and never change which model runs, with `Customise models for this run` opening the full grid unchanged. A run shows four steps (Read, Research, Write, Check), the current phase in plain words, and the estimate counting down, with the timeline behind `Show details`. On, every screen behaves exactly as it did before: all ten model and effort controls, the course profile selector, the saved-prep entry point, the Technical log tab and the full timeline. The switch is stored per computer in `localStorage` under `guide-watcher-power-mode`; both modes run the identical pipeline.
- **Slate & Mint palette.** `src/app.css` holds the tokens. One saturated accent (`--brand`) carries actions and identity; colour otherwise only carries meaning: `--fill-done` green, `--fill-work` blue, `--fill-fail` red, `--fill-warn` amber, with lighter `--accent-*` variants for text on dark. A cancelled run stays neutral, because the user stopped it on purpose. Status badges are a coloured dot plus the word, so state reads before it is read. Both themes are defined in the same token block.
- **Continue, don't restart.** A writer that dies mid-guide leaves `.<Guide>.md.<txn>.gwfailed/work/draft/guide.md`. The next attempt for that guide finds the newest substantial draft (>= 1,500 words with a table of contents), seeds the writer harness with it, and asks the writer - possibly another model - to finish it in the same voice and format. History shows "Picking up the N-word draft the earlier attempt left".
- **Resume with a chosen model.** History -> Resume writing uses the current generation preferences (Settings -> Generation defaults); the confirmation text names the writer model. The Workspace resume panel has the same selectors.
- **Manual revisions.** A guide edited by hand should end with `<!-- guide-watcher: manual-revision <date> - <what changed> -->` (replace the app's `guide-watcher:complete:v3` receipt). The app treats such a guide as complete and usable for predecessor continuity, scan skipping, and Open guide; it is not bundle-sealed.
- **Portable copies.** `_automation/embed_guide_images.py <guide.md>` (or double-click `_automation/Make Portable Guide.cmd`) writes `<Guide>_portable.md` with every figure embedded as a data URI, for reading on another machine without the assets folder. Data URIs render in local viewers (VS Code, Markdown Preview Enhanced, Silk MD, Typora, Obsidian), not on github.com.
- **Formatting standard.** `_automation/guide_format_standard.md` is inlined into the template (Step 2b) and enforced by `guide_lint.py --advisory`; `guide_format.py` runs the checks standalone.
- **Style handoff between writers.** Before its first section the primary writer (Claude) records `draft/style_plan.md` in the workspace: the voice, running cast, notation, the analogy per abstraction and where it breaks, section openers and closers, the worked-example instances, definition placement, figure handling, and what to avoid. It is Claude's own plan for THIS guide, not a restatement of the template. The plan survives workspace resets and failed attempts, and every Codex fallback (draft, depth/pedagogy pass, repair, continuation) receives it as binding instructions, so the finished guide reads as one hand wrote it. History shows "Handing the primary writer's style plan to the Codex fallback".
- **Sibling lecture decks are never books.** Supplementary-book discovery skips files the course's lecture-naming rule recognizes (e.g. `L1 _ ....pdf` when generating `L0`), tolerates `Instructor : Name` spacing, and gives colliding book titles a file-name suffix. Before this, two sibling decks titled "Fall 2026" produced identical figure purposes and the visual catalog refused four lectures at preflight.
- **A saved figure whose source left the context is dropped, not fatal.** A prep packet collected before that rule change can reference pages of a sibling deck. On resume such figures are left out and named in the run log, and the rest of the frozen context is used. Changed image bytes and ambiguous bindings still refuse, because those mean the plan cannot be trusted. `check` on a prep packet binds the figure plan too, so it reports the same notes the resume would.
- **Preflight-only check.** `guide-watcher-cli check <lecture.pdf|guide.prep.md>` runs the full preflight (sources, frozen dependencies, predecessors, transcripts, exam patterns, visuals, workspace) and, for a prep packet, every check the writer start performs (packet re-read, source-pack binding, contract and digest agreement) - reporting JSON without starting a model; exit 1 when it would fail. `_automation/preflight_sweep.py` runs it over every course; a clean sweep is the acceptance gate before installing a build.
- **Resume tolerates predecessor drift.** A saved `.prep.md` that bound predecessor guides which were later revised, or that predates an earlier guide becoming usable, resumes against the current course plan and logs each difference instead of refusing with "bound predecessor list does not match".
