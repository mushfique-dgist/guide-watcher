<div align="center">

# Guide Watcher

**Turn a folder of lectures into study guides worth reading.**

Point it at your course material. It reads your slides, your textbooks and the guides it has
already written, then writes a new guide with worked examples, your own figures and exam-style
practice, and refuses to call it finished until it passes its own checks.

[![Download](https://img.shields.io/badge/Download-Windows%20installer-2ea88a?style=for-the-badge&logo=windows&logoColor=white)](https://github.com/mushfique-dgist/guide-watcher/releases/latest)
[![Release](https://img.shields.io/github/v/release/mushfique-dgist/guide-watcher?style=for-the-badge&color=2ea88a)](https://github.com/mushfique-dgist/guide-watcher/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/mushfique-dgist/guide-watcher/total?style=for-the-badge&color=2ea88a)](https://github.com/mushfique-dgist/guide-watcher/releases)
[![Licence](https://img.shields.io/github/license/mushfique-dgist/guide-watcher?style=for-the-badge&color=2ea88a)](LICENSE)

![Guide Watcher workspace](docs/media/workspace.png)

</div>

## What it does

A study guide it writes is not a summary. For a 115-slide lecture it produced 57,592 words with
139 worked examples, 35 figures taken from the slides themselves, and ten exam-style questions
modelled on a past paper you supply. It took 76 minutes and told you when it was done.

| Capability | What it means in practice |
| --- | --- |
| **Reads what you already have** | Your slides, the textbooks in the same folder, and every guide it wrote for earlier lectures, so each guide builds on the last instead of repeating it. |
| **Teaches, rather than narrating slides** | Rules it must follow: describe a thing before naming it, open each section with the problem it solves, give an analogy and say where the analogy breaks, and never write "any questions?" because a slide said it. |
| **Checks its own work** | Arithmetic that does not add up, sections with no worked example, terms used before they are defined, figures never mentioned in the prose. A guide is published only after it passes. |
| **Keeps going when a model gives out** | Two providers. If one hits a usage limit or is signed out, the run moves to the next rather than dying, and continues a half-finished draft in the same voice. |
| **Leaves your files alone** | It reads your material and writes new Markdown beside it. Nothing you own is modified. |

## Screenshots

| Four steps and a clock, while it works | One question instead of ten |
|---|---|
| ![Progress](docs/media/progress.png) | ![Settings](docs/media/presets.png) |

The interface is deliberately plain. Everything it hides, every model, effort level and fallback,
and the full technical log, comes back with one **Power mode** switch. Light and dark are both
first-class.

![Light theme](docs/media/light.png)

## Install

### Windows

[**Download the installer**](https://github.com/mushfique-dgist/guide-watcher/releases/latest)
(`.exe`), or the `.msi` if your machine is managed. Then read *Before your first guide* below,
because the app needs two things from you before it can write anything.

### macOS and Linux

Not yet. The pipeline is portable Rust; the installer, the taskbar badge and the path handling
are the parts that still assume Windows. It is the next piece of work.

### Build it yourself

```bash
npm install
cp guide-watcher.example.json guide-watcher.local.json   # then edit it
npx tauri build
```

## Before your first guide

Guide Watcher does not ship a model and never asks for an API key. It drives two command-line
tools you install and sign in to yourself, so your subscription and your credentials stay yours.

1. **Install and sign in to both CLIs.** [Claude Code](https://claude.com/claude-code) writes the
   guides; [OpenAI Codex](https://developers.openai.com/codex/cli) collects the material.
2. **Install Python 3.13** and the verifier's packages:
   `pip install pymupdf markdown-it-py youtube-transcript-api requests`.
3. **Put the `_automation` folder somewhere** and point the settings file at it. It holds the
   writing rules and the checker, in plain text, so you can read and change what a guide must be.
4. **Edit `guide-watcher.local.json`**: your course folder, and one entry per subject.

```jsonc
{
  "watch_dir": "C:/Users/you/Documents/Study",
  "courses": [
    { "id": "photography", "label": "Photography", "folder": "Photography",
      "lecture_files": "^lesson[0-9]+.*\\.pdf$" }
  ]
}
```

`lecture_files` is optional. It says which file names start a guide, so a folder can hold both
your lectures and the textbook that is not one. Leave it out and every supported file counts.

Prefer to hand the whole thing to an AI assistant? Point it at
[`SETUP_FOR_AI.md`](SETUP_FOR_AI.md). Already running an older build? Point it at
[`UPDATE_FOR_AI.md`](UPDATE_FOR_AI.md), which carries your settings forward and asks you for
whatever it cannot work out on its own.

## How a guide is made

Two phases, two different models, neither trusted on its own.

1. **Collect.** Extract the text and page images, rank which figures are worth teaching from, pull
   the relevant pages out of any book in the folder, and freeze all of it into an evidence packet
   with checksums. Nothing later may read a file that is not in that packet.
2. **Write.** Draft against that packet, deepen it in bounded passes, then fix exactly what the
   checker reports. A model finishing its response proves nothing; only the checks do.

If a run dies halfway, the next attempt picks up the draft it left rather than starting over, and
follows a style plan the first writer recorded so the seams do not show.

## Command line

The same pipeline runs headless, which is also how it is tested:

```bash
guide-watcher-cli check <lecture.pdf | guide.prep.md>   # full preflight, starts no model
guide-watcher-cli generate <lecture.pdf>
guide-watcher-cli resume <guide.prep.md>
guide-watcher-cli supplementary plan|collect|status <course folder>
```

`check` is the honest gate: everything a real run does up to the moment a model would start.

## Repository

| Path | What it holds |
|---|---|
| `src/` | The Svelte interface |
| `src-tauri/src/` | The Rust pipeline: source capture, figure selection, both provider phases, publication |
| `scripts/` | Install helpers, and settings migration between versions |
| `_automation/` | Ships alongside: the writing rules, the depth contract, the checker |

## Licence

MIT. See [LICENSE](LICENSE).
