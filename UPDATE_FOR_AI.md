# Updating an existing Guide Watcher install — instructions for the AI assistant

You are updating a computer that **already runs an older Guide Watcher build**. Someone adapted
that build to their own machine: their folders, their courses, their tool paths. Your job is to
bring the app up to this package's version **without losing a single thing they configured**, and
to leave them with a working app rather than a half-finished one.

If this machine has **no** previous install, stop reading here and follow `README_FIRST_SETUP_FOR_AI.md`
instead. That is the first-time setup; this document is the upgrade.

**Your goal is a working app on this specific machine, not a completed checklist.** These steps
describe the shape of the work; they cannot describe this computer. Use your own judgement
throughout, and when what you see does not match what is written here, work out why before you
proceed. Two habits make the difference between an update that works and one that only looks
finished:

1. **Verify rather than assume.** After each step, check the thing you just changed actually
   holds — the path exists, the build compiles, the preflight passes on their real material.
2. **Ask instead of guessing.** You cannot know their folder layout, their courses, or what they
   have installed. Every guess you make is a way for this to fail silently an hour later. The
   questions worth asking are listed at the end; ask them the moment you need them.

---

## What you must not do

- **Never touch their content.** Guides (`*_Guide.md`), prep packets (`*.prep.md`), guide asset
  folders, `_supplementary/`, past exam files, retained `.gwfailed` / `.gwwork` workspaces and the
  History database in `%LOCALAPPDATA%\com.mushfique.guide-watcher\` all belong to the user. The
  update never edits, moves or deletes them.
- **Never invent a path.** If you cannot find a folder the old config points at, ask.
- **Never install while the app is open or a guide is running.** The installer refuses when the app
  is open, and stopping a running guide can waste hours of work. Check first, wait or ask.
- **Never report success you did not verify.** Every gate in step 5 must actually pass.

## What carries over by itself

These live outside the source tree and survive an update untouched: the History database, the
user's generated guides and prep packets, their per-machine interface settings (theme, zoom,
sidebar width, Power mode, saved generation defaults in the app's `localStorage`), and their
provider sign-ins (Codex and Claude keep their own credentials).

---

## What is new in this version

Tell the user about these once the update is done; the first one will otherwise look like lost
settings, because it changes what they see on launch.

- **A simple interface by default, with Power mode to restore the old one.** One switch in
  Settings & help. Off: the review screen asks one question (Quick / Balanced / Thorough) instead
  of showing ten model and effort dropdowns, and a run shows four steps with a time estimate. On:
  every control and the technical log come back exactly as before. Identical pipeline either way.
- **Windows notifications and a taskbar count** when a guide finishes, a run stops early, or new
  lecture files appear, plus a matching badge on the tray icon.
- **Only one app window can run at once.** A second launch focuses the running one.
- **A new colour scheme**, in both dark and light.
- **Time estimates that scale with the lecture** instead of quoting the same range for every deck.
- **The backup writer takes over far more reliably.** Any provider failure that produces no guide
  now falls through to the next writer, rather than only the failures whose wording was recognised.
- **Saved contexts survive change better.** Revised or newly available predecessor guides, and
  figures whose source is no longer collected, are reported and worked around instead of refusing.
- **Example economy in the guide rules.** Guides no longer give three worked examples to every
  concept regardless of difficulty; the automation files carry this, so it applies as soon as
  step 3 is done.

## Step 1 — Find both copies, and back up

1. Locate the **old checkout**: the source folder they built the current app from. It contains
   `src-tauri/src/config.rs`. If they do not know where it is, search the machine for
   `config.rs` containing `FIFTH_SEMESTER_ROOT`, and confirm with them before using it.
2. Unzip this package to a **new** folder next to it. Do not unzip over the old one.
3. Copy the old checkout to a backup folder, and tell the user where you put it.

If there is no old checkout at all (they installed from a binary someone handed them), you cannot
port settings automatically. Ask them for their semester folder path and course list, then edit the
new `src-tauri/src/config.rs` and `src-tauri/src/course_plan.rs` by hand, following the patterns
already in those files.

## Step 2 — Carry their machine settings into the new source

Guide Watcher is configured in Rust source, not a settings file, so this step is real work.

```
python scripts/port_local_settings.py --old "<old checkout>"            # report only
python scripts/port_local_settings.py --old "<old checkout>" --apply    # write it
```

The script copies the constants that describe *their computer* (semester roots, the `_automation`
file paths, node / ffmpeg / uv executables, their LMS course id) and their whole course list, in
both `config.rs` and `course_plan.rs`. It prints three lists:

- **carried** — done for you; skim it to confirm the paths look like their machine.
- **review** — a value that differs but is not obviously a path. Decide each one: a model name or a
  pinned version is a product decision, so keep the **shipped** value; anything that names their
  machine, account or course is theirs, so restore **theirs**.
- **missing** — a file the script could not find in one of the copies. Handle it by hand.

If this version changed the shape of the course list (a new field on a course), the ported block
will not compile. That is expected and easy: the compiler names the missing field, and you add it
to each of their courses, copying the pattern from the shipped list you just replaced. The fields
to be aware of are the course id, label, root folder, guide mode, guide kind, the lecture file
naming rule, and the display order.

## Step 3 — Update the automation folder in place

`config.rs` points at a folder (usually `.../4th Semester/_automation/`) holding the prompt, the
depth contract, the verifier and its pinned requirements. Those files are **product**, not user
configuration: copy this package's `_automation/` contents over theirs, at whatever path their
constants point to.

Two cautions:

- The app refuses to start a guide when these files change mid-run, and it verifies them by hash.
  Only replace them while no guide is running.
- If their copy differs from the previous package's copy because **they** edited it (a course-
  specific instruction they added), show them the difference and ask before overwriting.

Keep their `_supplementary/` folders and any past-exam Markdown files exactly where they are.

## Step 4 — Install the prerequisites this version needs

Check, and ask the user to fix anything missing rather than installing silently on their behalf:

- Rust toolchain, Node.js 22 with `npm install` run in the new folder, Python 3.13 on `PATH`.
- Python packages for the verifier and helpers: `pymupdf`, `markdown-it-py` (the exact version in
  `_automation/requirements-verifier.txt`, which the verifier enforces), `youtube-transcript-api`,
  `requests`.
- Claude Code CLI and OpenAI Codex CLI installed **and signed in**, with the account actually
  entitled to the models in `config.rs`.

## Step 5 — Verify before you build

Run all of these from the new checkout and do not continue past a failure:

```
cargo test --manifest-path src-tauri/Cargo.toml --all-targets
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run test:frontend
cargo build --release --manifest-path src-tauri/Cargo.toml --bin guide-watcher-cli
python "<their _automation>/preflight_sweep.py" --cli "<new>/src-tauri/target/release/guide-watcher-cli.exe" --semester "<their semester folder>"
```

The sweep is the honest gate: it runs the app's real preflight over every lecture and prep packet
in their courses without starting any model. Read its output rather than only its exit code:

- `ok` — fine. `ok (drift)` — fine, and the note explains what changed since a context was saved.
- `existing` — expected for a lecture whose guide or prep packet already exists.
- `FAIL` — fix it now. A failure here is usually a real configuration problem on this machine: a
  course root that does not exist, a renamed folder, a missing textbook, or a lecture naming rule
  that does not match their files.

If the sweep reports no targets at all, their course roots are wrong. Go back to step 2.

## Step 6 — Build, install, and confirm

1. Confirm the app is closed (including its tray icon) and no guide is running.
2. `npx tauri build`
3. `.\scripts\install-windows.ps1 -InstallerPath ".\src-tauri\target\release\bundle\nsis\Guide Watcher_2.0.0_x64-setup.exe" -ExpectedBinaryPath ".\src-tauri\target\release\guide-watcher.exe"`
   The script refuses while the app is open and prints the SHA-256 it installed. Windows PowerShell
   5.1 or PowerShell 7 both work.
4. Launch it, then confirm with the user: their courses appear, their History is intact, and a
   `check` on one of their prep packets still passes.

## Step 7 — Tell them what changed on their screen

This version defaults to a **simple interface**, so an existing user may think settings vanished.
Say this plainly:

> The app now opens in a simplified mode: one choice of how thorough to be, and progress in four
> steps. Every control you had is still there — turn on **Power mode** in Settings & help and your
> screens come back exactly as they were, including all the model and effort settings and the
> technical log. The switch is remembered on this computer.

Also worth mentioning: Windows notifications and a taskbar count when a guide finishes or needs
them, a new colour scheme, only one app window can run at a time now, and time estimates that
scale with the size of the lecture.

---

## Ask the user — do not guess

Ask immediately, in plain words, whenever any of these is true. Each one blocks a working app, and
guessing produces a broken install that looks finished.

| What you find | What to ask for |
|---|---|
| No old checkout, or two candidates | "Which folder did you build the current Guide Watcher from?" |
| A course root in their config does not exist | "Your settings point at `<path>`, which I cannot find. What is the correct folder for `<course>`?" |
| Courses changed since they set it up | "Which courses should this semester have, and what is each one's folder?" |
| Lecture files do not match the naming rule for a course | "Your `<course>` lectures are named like `<example>`. Should the app treat those as its lecture files?" |
| A tool path is wrong (node, ffmpeg, uv) | "Where is `<tool>` installed on this machine, or should I leave that feature switched off?" |
| A CLI is missing or signed out | "Please sign in to `<Claude Code / Codex>` in a terminal, then tell me when it is done." |
| The account cannot use a configured model | "Your settings ask for `<model>`, which this account cannot use. Which model should it use instead?" |
| Python packages missing or the wrong version | "Please run `<exact pip command>`, then tell me when it finishes." |
| The sweep FAILs on their material | Quote the exact message and ask the specific question it implies. Never paper over it. |
| A guide is running | "A guide is still being written. Should I wait, or would you rather stop it and update now?" |
| Their `_automation` files were edited by hand | "You changed `<file>`. Do you want to keep your version, or take the new one?" |

When you need several of these, ask for them together in one short message rather than one at a
time, and state plainly what will not work until they answer.

---

## If something goes wrong

- **It does not compile after porting settings.** Almost always the course list shape. The compiler
  names the field; add it to each of their courses.
- **The app starts but shows no courses.** Their course roots do not exist or are not readable.
- **Every guide refuses at preflight.** Check the `_automation` paths in `config.rs` and that the
  files they point at exist.
- **The installer refuses.** The app is open. Close it including the tray icon.
- **They want the old build back.** The backup from step 1 rebuilds and reinstalls exactly as before;
  their guides and History were never touched.
