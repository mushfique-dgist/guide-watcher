# Guide Watcher

Guide Watcher turns the material you are studying into long, worked study guides: it reads your
slides, your textbooks and the guides it has already written, then writes a new one and refuses to
call it finished until it passes its own checks.

It was built for one person's university courses and is being generalised so it works for any
subject and any source material. Point it at a folder of lectures and it produces a guide per
lecture, each one carrying worked examples, figures lifted from your own slides, and exam-style
practice.

## What makes a guide

Every guide is written in two phases by two different models, and neither is trusted on its own.

1. **Collect.** The app extracts the text and page images from your material, ranks which figures
   are worth teaching from, pulls relevant pages from any books in the course folder, and freezes
   all of it into an evidence packet with checksums. Nothing later can read a file that is not in
   that packet.
2. **Write.** A second model drafts the guide against that packet, then deepens it in bounded
   passes, then fixes what a checker reports: sections with no worked example, terms used before
   they are defined, figures never referenced in the prose, arithmetic that does not add up.
   A guide is published only after it passes; a model finishing its response proves nothing.

The writing rules live in plain text in `_automation/`, so you can read and change what the guide
is asked to be. They encode the things that separate a useful guide from a wall of text: describe
a thing before naming it, open a section with the problem it solves, give an example the number of
examples its difficulty earns, and never narrate the slides.

## Status

- **Windows** is the only tested platform today. Making it run on macOS and Linux is the next
  substantial piece of work; the paths, the installer and the taskbar integration are the parts
  that assume Windows.
- **You need two model CLIs installed and signed in**: OpenAI Codex and Claude Code. The app never
  asks for, stores, or sees an API key — it runs the CLIs you have already authenticated, and
  falls back from one to the other when a provider is unavailable.
- **Your files are never modified.** The app reads your material and writes new Markdown guides
  beside it.

## Setup

1. Install Rust, Node.js 22, and Python 3.13.
2. `npm install`
3. Copy `guide-watcher.example.json` to `guide-watcher.local.json` and edit it: your course folder,
   and where you put the `_automation` folder. That file is git-ignored, so your folder layout
   never reaches the repository.
4. `pip install pymupdf markdown-it-py youtube-transcript-api requests` — the verifier pins the
   exact `markdown-it-py` version in `_automation/requirements-verifier.txt` and refuses to run
   against a different one.
5. Sign in to both CLIs: `claude` and `codex login`.
6. `npx tauri build`, then install from `src-tauri/target/release/bundle/`.

If you would rather hand this to an AI assistant, point it at `SETUP_FOR_AI.md`, which is written
for that purpose. If you already run an older build, point it at `UPDATE_FOR_AI.md` instead: it
carries your machine's settings forward and asks you for whatever it cannot work out on its own.

## Using it

Choose lecture files, or let it scan your course folder for new material. It shows what it is
about to read, asks one question about how thorough to be, and starts. A guide takes one to three
hours depending on the size of the lecture, so it tells you when it is done: a desktop
notification, and a count on the taskbar icon that clears when you look at the window.

The interface is deliberately plain by default. Everything it hides — every model, effort level
and fallback, the full technical log — comes back with a single **Power mode** switch in Settings.

## Command line

The same pipeline runs headless, which is also how it is tested:

```
guide-watcher-cli check <lecture.pdf | guide.prep.md>   # run the whole preflight, start no model
guide-watcher-cli generate <lecture.pdf>
guide-watcher-cli resume <guide.prep.md>
guide-watcher-cli supplementary plan|collect|status <course folder>
```

`check` is the honest gate: it does everything a real run does up to the moment a model would
start, and reports exactly what a run would report.

## Repository layout

| Path | What it holds |
|---|---|
| `src/` | The Svelte interface |
| `src-tauri/src/` | The Rust pipeline: source capture, visual selection, the two provider phases, publication |
| `scripts/` | Install and maintenance helpers, including settings migration between versions |
| `_automation/` | Ships separately: the writing rules, the depth contract, and the verifier |

## Licence

MIT. See `LICENSE`.
