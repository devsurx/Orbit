<div align="center">

# 🌀 Orbit

### Summon a file converter with a flick of your wrist.

Shift-drag any file, choose a radial petal, walk away. No drag-and-drop onto a
tiny window, no "Save As" dialog, no upload, no account.

<br />

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL%202.0-6e4aff.svg)](https://www.mozilla.org/en-US/MPL/2.0/)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-2ea043.svg)](#-platform-support)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-dea584.svg?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Status](https://img.shields.io/badge/status-M1%20in%20progress-f5a623.svg)](#-roadmap)
[![No telemetry](https://img.shields.io/badge/telemetry-none-2ea043.svg)](#-privacy)

</div>

---

```text
                          ╭──────────╮
                  ╭───────┤   MP4 🎬 ├───────╮
                  │       ╰──────────╯       │
            ╭─────┴─────╮              ╭─────┴─────╮
            │   WebP 🖼  │   ┌──────┐   │   MP3 🎵  │
            ╰─────┬─────╯   │ 2 files│  ╰─────┬─────╯
                  │         └──────┘         │
            ╭─────┴─────╮              ╭─────┴─────╮
            │   GIF  🎞  │              │   WAV 🎧  │
            ╰─────┬─────╯              ╰─────┬─────╯
                  ╰───────╮  Convert  ╭──────╯
                          ╰───────────╯
                     esc ✕ cancel
```

<sub><i>Design mock — the ring adapts to what you dragged. Two files, one run.</i></sub>

---

## ✨ What it is

Orbit is a **background utility** that lives in your tray and only appears when
you want it. Drag files while holding <kbd>Shift</kbd> and a translucent ring
opens under your cursor with *exactly* the actions that fit those files —
convert, compress, resize, trim, OCR, strip metadata, merge a PDF, and more.

It is **local-first**: everything runs on your machine through native engines
you already trust (`FFmpeg`, ImageMagick, Ghostscript, …). Nothing is uploaded.

> Run one action, or apply a preset to fifty files while you make coffee.

## 🎯 Highlights

| | |
|---|---|
| 🖱️ **Shift-drag trigger** | No window to find. The ring appears where your cursor is. |
| 🧠 **Context-aware** | A `.mov` offers video petals; a folder of `.png` offers image tools. |
| 🔁 **150+ conversion pairs** | Generated from a data-driven registry — add a format, not code. |
| 🧰 **Editing, not just converting** | Resize, crop, trim, rotate, strip EXIF, watermark, merge, split. |
| ⚙️ **Batch & presets** | Name a workflow once, reuse it forever. |
| 🗂️ **Watch folders** | Drop a file in a folder, get a converted one out. |
| 🔎 **OCR & documents** | Turn scans into searchable PDFs and editable text. |
| 📜 **History & undo tree** | Every job is recorded; revisit or re-run any step. |
| ⌨️ **Universal fallbacks** | Global hotkey, tray target, command palette, "Open with Orbit", CLI. |
| 🕵️ **Private by design** | Zero telemetry, zero network calls, no account, no cloud. |

## 🖥️ Platform support

The ring is always a real drop target. What varies per OS is only *how Orbit
learns that a Shift-drag started* — and that is genuinely blocked on Wayland
by design, so we say so instead of shipping something flaky.

| Platform | Shift-drag | Fallbacks | Notes |
|---|---|---|---|
| **Windows 10/11** | ✅ | hotkey · tray · palette | Low-level mouse/keyboard hooks + drop overlay. |
| **macOS 12+** | ✅ | hotkey · tray · palette | `NSEvent` monitors; needs Accessibility permission. |
| **Linux — X11** | ✅ | hotkey · tray · palette | XInput2 / XRecord + XDND. Tested via XWayland too. |
| **Linux — Wayland** | ⚠️ | hotkey · tray · palette · Open with | Clients cannot see global Shift-drag. Documented, not faked. |

> ⚠️ On Wayland the global-hotkey and shell integrations are **first-class**, not
> consolation prizes: <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Space</kbd> puts the
> ring wherever your pointer is.

## 🚀 Build from source

**Prerequisites:** Rust 1.85+ (2024 edition), Node 20+, and the platform
webview/Tauri dependencies.

<details open>
<summary><b>Linux (Debian / Ubuntu)</b></summary>

```bash
sudo apt-get update
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
  librsvg2-dev libxdo-dev patchelf
```
</details>

<details>
<summary><b>macOS</b></summary>

```bash
xcode-select --install
```
</details>

<details>
<summary><b>Windows</b></summary>

Install the **WebView2** runtime (preinstalled on Windows 11) and the MSVC
build tools. Everything else comes from Cargo.
</details>

```bash
git clone https://github.com/devsurx/Orbit.git
cd Orbit

# Registry + core crates (no system deps needed)
cargo test -p orbit-registry

# Inspect the generated ring for any file type
cargo run -p orbit-registry --example menu -- mov png heic
cargo run -p orbit-registry --example menu -- --pairs
```

## 🧠 How it works

Orbit has **no hardcoded conversion list**. Formats, engines, rules, actions and
chains live in [`registry/`](registry/) as JSON, and
[`orbit-registry`](crates/orbit-registry) validates them on load and expands
them into a ranked, context-aware menu.

```text
formats.json ─┐
engines.json ─┤
conversions.json ─┼─▶  orbit-registry  ─▶  MenuModel  ─▶  radial UI
actions.json ─┤        (probe · select ·      (petals)      (React)
chains.json ──┘         rank · validate)
```

- **`requires`** holds capability tokens (`op:*`), never engine names, so any
  engine that can do the job works.
- Read/write support comes from each format's `readers` / `writers` lists.
- Rules target *families* (`@writable:image`), and the pair matrix multiplies
  them out — **845 source → target pairs today**, from 22 rules.
- The registry **fails loudly** with a readable list when data is inconsistent.

```console
$ cargo run -p orbit-registry --example menu -- --pairs
pairs                    845
engines installed        11
  image -> image                 160
  audio -> audio                  99
  audio -> video                  48
  ...
```

## 🗂️ Project layout

```text
Orbit/
├── registry/                 # 📚 the source of truth (JSON, no code)
│   ├── formats.json          #    63 formats, with readers/writers and rank
│   ├── engines.json          #    FFmpeg, ImageMagick, Ghostscript, …
│   ├── conversions.json      #    22 rules → the whole pair matrix
│   ├── actions.json          #    editing verbs (resize, trim, OCR, …)
│   ├── chains.json           #    multi-step recipes
│   ├── presets.json          #    seeded batch workflows
│   ├── options.json          #    shared option catalog
│   └── categories.json       #    ring sections
├── crates/
│   ├── orbit-registry/       # 🧠 load · validate · build the ring model
│   │   ├── src/
│   │   ├── tests/
│   │   └── examples/menu.rs  #    dev tool: "what does a .mov offer?"
│   ├── orbit-app/            # 🪟 Tauri shell: tray, hotkey, overlay  (M1)
│   └── orbit-cli/            # ⌨️  headless `orbit convert …`        (M5)
└── src/                      # ⚛️  React + Vite + Tailwind radial UI  (M1)
```

## 🗺️ Roadmap

| | Milestone | State |
|---|---|---|
| **M1** | Tray app, global hotkey, transparent overlay, radial menu UI | 🚧 in progress |
| **M2** | Job runner, worker pool, image & audio conversion | ⏳ planned |
| **M3** | Video / PDF / document engines + editing tools | ⏳ planned |
| **M4** | Per-OS Shift-drag trigger + shell integration | ⏳ planned |
| **M5** | Presets, batch, history, watch folders, OCR, CLI | ⏳ planned |
| **M6** | Onboarding, settings, polish, packaging, CI | ⏳ planned |

## 🔒 Privacy

No telemetry. No analytics. No network calls except an **optional, user-triggered**
update check. Your files never leave your machine.

## 🤝 Contributing

The easiest, highest-impact contribution is **registry data**: a new format, a
missing conversion rule, a better default option. It is JSON, it is validated by
the test suite, and you can check your change in seconds:

```bash
cargo test -p orbit-registry
cargo run -p orbit-registry --example menu -- <your-ext>
```

Adding a format or rule keeps `readers`/`writers` honest or the loader will tell
you exactly what is wrong. A `CONTRIBUTING.md` and a `THIRD_PARTY_LICENSES.md`
arrive with the packaging milestone.

## 📄 License

[Mozilla Public License 2.0](LICENSE) © surx.
