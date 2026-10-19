<div align="center">

# 🌀 Orbit

### Summon a file converter with a flick of your wrist.

Summon the ring, drop files on it, choose a radial petal, walk away. No
drag-and-drop onto a tiny window, no "Save As" dialog, no upload, no account.

<sub>🚧 The Shift-drag-anywhere trigger is M4; the ring, the engines and the drop
target are already here.</sub>

<br />

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL%202.0-6e4aff.svg)](https://www.mozilla.org/en-US/MPL/2.0/)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-2ea043.svg)](#-platform-support)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-dea584.svg?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Status](https://img.shields.io/badge/status-M3%20shipped-2ea043.svg)](#-roadmap)
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

## 🚀 Quick start

Grab an installer from [Releases](https://github.com/devsurx/Orbit/releases) —
`.exe` or `.msi` on Windows, `.dmg` on macOS, `.AppImage`/`.deb`/`.rpm` on Linux —
or [build from source](#-build-from-source).

```bash
orbit ~/Pictures/photo.png    # the ring opens under your cursor
```

Pick a petal — *Convert → WebP*, *Compress*, *Trim* — and the result lands
**next to the original** as `photo (converted).webp`. Nothing is overwritten,
nothing is uploaded, and `orbit --log` prints the path of every file Orbit has
written.

<sub>First time here? → [Build from source](#-build-from-source) · [The `orbit` command](#-the-orbit-command) · [Windows](#-windows)</sub>

---

## ✨ What it is

Orbit is a **background utility** that lives in your tray and only appears when
you want it. When the ring is up it is a real drop target with *exactly* the
actions that fit the files — convert, compress, resize, trim, OCR, strip
metadata, merge a PDF, and more.

It is **local-first**: everything runs on your machine through native engines
you already trust (`FFmpeg`, ImageMagick, Ghostscript, …). Nothing is uploaded.

> 🚧 Today the ring is summoned by hotkey, tray, compositor binding or
> "Open with Orbit". The **Shift-drag-anywhere** trigger is the M4 milestone; the
> drop target itself is already real and tested, which is what that milestone
> has to hook into.

> Run one action, or apply a preset to fifty files while you make coffee.

## 🎯 Highlights

| | |
|---|---|
| 🖱️ **A real drop target** | The ring takes files the moment it is up; nothing to aim at. |
| ⌨️ **Shift-drag-anywhere** | 🚧 M4 — the global trigger that summons the ring mid-drag. |
| 🧠 **Context-aware** | A `.mov` offers video petals; a folder of `.png` offers image tools. |
| 🎛️ **Defaults that work, options when you want them** | Drop to run with sensible values; Alt-click or hold for the panel. |
| 🔁 **841 conversion pairs** | Generated from a data-driven registry — add a format, not code. |
| 💾 **Never clobbers your files** | Results land beside the source as `name (converted).ext`, published atomically. |
| 🧰 **Editing, not just converting** | Resize, crop, trim, rotate, strip EXIF, compress, merge, split, OCR. |
| 🔗 **Merge as one job** | Several images → one PDF or GIF; PDFs → one PDF; audio/video concatenated. |
| ⚙️ **Batch & presets** | Name a workflow once, reuse it forever. |
| 🗂️ **Watch folders** | Drop a file in a folder, get a converted one out. |
| 🔎 **OCR & documents** | Turn scans into text (images today); documents via LibreOffice/Pandoc. |
| 📜 **History & undo tree** | Every job is recorded; revisit or re-run any step. |
| ⌨️ **A hotkey you choose** | Pick one from the tray; it is remembered between launches. |
| 🕵️ **Private by design** | Zero telemetry, zero network calls, no account, no cloud. |

## 🖥️ Platform support

The ring is always a real drop target. What varies per OS is only *how Orbit
learns that a Shift-drag started* — and that is genuinely blocked on Wayland
by design, so we say so instead of shipping something flaky.

| Platform | Shift-drag | Fallbacks | Notes |
|---|---|---|---|
| **Windows 10/11** | 🚧 M4 | hotkey · tray · Open with | Low-level mouse/keyboard hooks + drop overlay. |
| **macOS 12+** | 🚧 M4 | hotkey · tray · Open with | `NSEvent` monitors; needs Accessibility permission. |
| **Linux — X11** | 🚧 M4 | hotkey · tray · Open with | XInput2 / XRecord + XDND. |
| **Linux — Wayland** | ⛔ | tray · Open with · compositor binding | No global Shift-drag **and no in-app hotkey** — see below. |

> ✅ **What works on every platform today:** the overlay is a real drop target,
> the global hotkey (where the OS delivers one), the tray icon, and handing files
> to Orbit from the shell — `orbit-app photo.png` selects them and opens the ring.
> That last route is the same one "Open with Orbit" and "Send to" use.

> ⚠️ **Wayland has no global hotkey path at all.** Orbit's hotkey goes through
> `XGrabKey`, which Wayland deliberately makes inert: the compositor owns input,
> and XWayland only sees events for the focused X11 window. The grab even
> *succeeds*, so the failure is silent — Orbit prints a note at startup rather
> than leaving you pressing keys at nothing.

### ⌨️ Choosing the shortcut

The default trigger is <kbd>Ctrl</kbd>/<kbd>⌘</kbd> + <kbd>Shift</kbd> +
<kbd>Space</kbd>. Change it from **tray menu → Shortcut**: four combinations and
an **Off** entry, with a ✓ beside the one running now. The choice is saved to
`settings.json` in the app's config directory (`~/.config/app.orbit.desktop/`
on Linux, `~/Library/Application Support/app.orbit.desktop/` on macOS,
`%APPDATA%\app.orbit.desktop\` on Windows) and applied immediately.

Scripts and installers can set it without the tray:

```bash
orbit-app --hotkey="Ctrl+Alt+O"   # choose a combination
orbit-app --hotkey=off            # turn the global shortcut off
```

If another application already owns the combination, Orbit reports that instead
of failing silently — pick another from the tray. It is a *global* shortcut, so
it works while Orbit is in the background, and the ring opens under the pointer.

### ⌨️ Global hotkeys on Wayland

Only the compositor may own a global shortcut, so you bind Orbit there instead.
Orbit ships **single-instance support**, which is what makes that binding a real
trigger: the second launch hands its arguments to the running instance and exits,
rather than adding a second tray icon and a second overlay.

**Hyprland** — `~/.config/hypr/hyprland.conf`:

```ini
bind = CTRL SHIFT, SPACE, exec, orbit-app
```

**sway** — `~/.config/sway/config`:

```ini
bindsym Ctrl+Shift+space exec orbit-app
```

**GNOME** — Settings → Keyboard → Custom Shortcuts, command `orbit-app`.
**KDE Plasma** — System Settings → Shortcuts → Custom, command `orbit-app`.

Use an absolute path if `orbit-app` is not on your `PATH`. Bound with a file
argument it selects that file; bound bare it reopens the ring for the last
selection.

### 🪟 Windows

Two installers, both built by CI on a real Windows runner:

| File | Install | Notes |
|---|---|---|
| **`Orbit_*_x64-setup.exe`** | Per-user, no administrator prompt | The one to pick. Installs under `%LOCALAPPDATA%\Orbit`, uninstalls from Apps & features. |
| **`Orbit_*_x64_en-US.msi`** | WiX, for managed machines | Same app, for deployments that want an MSI. |

**WebView2** is what draws the ring. Windows 11 has it; Windows 10 usually does
(Edge installs it). If it is missing, the installer fetches the bootstrapper from
Microsoft — the only network access Orbit or its installer ever performs, and it
is the runtime, not the app.

**Getting files to it.** The ring is a drop target, so once it is up you can drop
files straight onto it. To hand it files from the shell instead, right-click →
**Send to** → **Orbit**:

```powershell
# once, from a normal (non-elevated) PowerShell
powershell -ExecutionPolicy Bypass -File scripts\windows\install-send-to.ps1
```

That creates a shortcut in your personal Send-to folder, so it works for *any*
selection without registering a single file association. `-Uninstall` removes it.
Orbit is a single instance, so a multi-file selection arrives as one drop even if
Windows launches it once per file.

```powershell
# Any shell, including Explorer's address bar
Orbit.exe C:\Users\you\Pictures\photo.png
Orbit.exe --hotkey=off
```

> 🧪 **How far this has been verified.** The whole app **type-checks against
> `x86_64-pc-windows-msvc`**, which is what catches a stray Unix-only call or a
> missing `cfg` — CI runs that check on every push, from Linux. The installers
> themselves are built by the `installers` job on `windows-latest`, so they come
> out of the real toolchain rather than a cross-compile. What cannot be verified
> outside Windows is *runtime* behaviour: the tray menu, the hotkey, and the ring
> on your desktop. That is the honest state of it, and why M4 is next.

### 🤫 Known limitation: translucent windows on Linux

WebKitGTK cannot composite a transparent webview on Wayland — a
`transparent: true` Tauri window is created but **never rendered**. Orbit handles
this honestly rather than shipping a blank window:

- `tauri.conf.json` keeps real translucency, which is what macOS and Windows
  (WebView2) use.
- [`tauri.linux.conf.json`](crates/orbit-app/tauri.linux.conf.json) overrides the
  ring to an opaque window on Linux, where the ring paints its own dark panel and
  sets a matching `backgroundColor` — without one, the corners outside the panel's
  radius show the platform default, a light grey wedge.

The result is a glassy ring on macOS/Windows and a solid dark one on Linux. We
prefer that over a menu that does not appear at all.

## 🎛️ Dropping is the fast path; Alt is the precise one

A plain drop runs the petal with vetted defaults. When you want something else,
open the options panel without leaving the ring:

- **Alt-click** a petal,
- **hold** the petal for about half a second,
- **right-click** it, or
- click the **⋯** that appears under a hovered petal.

The panel is built from [`registry/options.json`](registry/options.json): the
widget, range, labels, placeholder and the "only show this when…" rule all come
from data. <kbd>Tab</kbd> walks the fields, <kbd>Enter</kbd> runs with what is on
screen, and <kbd>Esc</kbd> closes the panel without running anything.

| Petal | What the panel unlocks |
|---|---|
| Convert → PDF (image) | Paper size, margin, orientation, quality |
| PDF → image | DPI, which page, quality |
| Merge | Result (PDF / GIF / MP4 / MP3), order (as dragged / by name / oldest first) |
| Extract pages | `1-3` or `2-` — the range is written into one PDF |
| Crop | `left,top,width,height`, or ImageMagick's `WxH+X+Y` |
| OCR | Language and output format |

> 📄 **One page at a time, for now.** Rendering a *range* of PDF pages as a
> series of images — or a run of video frames — needs the series-output contract
> that lands in M5. Today `PDF → image` renders the page you choose, and
> `Extract pages` writes the chosen range into a single PDF. The registry says so
> in the petal's note rather than pretending otherwise.

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

# Core crates only — no system deps needed. The end-to-end tests skip
# themselves when FFmpeg/ImageMagick are not installed.
cargo test -p orbit-registry -p orbit-jobs

# Inspect the generated ring for any file type
cargo run -p orbit-registry --example menu -- mov png heic
cargo run -p orbit-registry --example menu -- --pairs
```

### ▶️ Running the desktop app

```bash
npm install          # once
npm run build        # UI → ./dist, which Tauri embeds into the binary

# Debug build. Starts quietly — tray only, no window.
cargo run -p orbit-app --features custom-protocol

# Then: Ctrl/Cmd+Shift+Space for the ring, or left-click the tray icon.
#       (Wayland has no in-app hotkey — bind Orbit in your compositor instead;
#        see the platform section above.)

# Change the trigger, or turn it off; the choice is remembered.
cargo run -p orbit-app --features custom-protocol -- --hotkey="Ctrl+Alt+O"
cargo run -p orbit-app --features custom-protocol -- --hotkey=off

# Or hand it files directly (the "Open with Orbit" / "Send to" route):
cargo run -p orbit-app --features custom-protocol -- ~/Pictures/photo.png
```

The ring starts **hidden** on purpose: Orbit is a background utility, not a
window you go looking for. It stays on top until you dismiss it with
<kbd>Esc</kbd> or a click outside.

### ⌨️ The `orbit` command

[`scripts/orbit`](scripts/orbit) is a small launcher that reduces all of that to
one word:

```bash
ln -s "$PWD/scripts/orbit" ~/.local/bin/orbit    # once, if ~/.local/bin is on PATH

orbit ~/Pictures/photo.png   # open the ring for these files
orbit                        # reopen the ring for the last selection
orbit --log                  # what was published, and where
```

It resolves paths, detaches so your shell comes straight back, and appends every
published file — by absolute path — to `~/.local/state/orbit/jobs.log`. That file
is the durable answer to *"where did my file go?"*; the ring only says it for a
second or so. It is local, created only by this launcher, and safe to delete.

> ⚠️ **`--features custom-protocol` is not optional for a real binary.** Without
> it Tauri builds in *dev mode* and loads `devUrl` (`http://localhost:1420`), so
> you get a window with nothing in it unless the Vite dev server happens to be
> running. This is the single most common "my Tauri app is blank" trap.
>
> For UI work with hot reload, use `npm run tauri dev` instead — the CLI sets the
> feature for you.

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
  them out — **841 source → target pairs today**, from 22 rules.
- The registry **fails loudly** with a readable list when data is inconsistent.

```console
$ cargo run -p orbit-registry --example menu -- --pairs
pairs                    841
engines installed        11
  image -> image                 160
  audio -> audio                  99
  audio -> video                  48
  ...
```

## ⚙️ Running a job

Picking a petal no longer just acknowledges the click — it converts the file.

```text
petal pick ─▶ resolve ─▶ plan ─▶ worker pool ─▶ publish ─▶ orbit://job
              (which      (argv)   (N files      (atomic,     (the hub shows
               engine)              in parallel)   no clobber)  progress)
```

- **Resolution is data-driven too.** The pick names a format or an action; the
  resolver asks the registry which engine can do it on *this* machine, which
  extension the result carries, and which word belongs in the name.
- **Results land beside the source.** The word comes from the action's `tag` in
  [`registry/actions.json`](registry/actions.json), so the naming rule has exactly
  one home:

  | You picked | You get |
  |---|---|
  | Convert → WebP | `photo (converted).webp` |
  | Compress | `photo (compressed).jpg` |
  | Audio from video | `clip (audio).mp3` |
  | Trim | `clip (trimmed).mp4` |

- **Where it went:** always next to the source file. `orbit --log` prints the
  paths of everything Orbit has published, if you would rather not go looking.
- **An existing file is never overwritten.** If the name is taken you get
  `photo (converted 2).webp`, and the original bytes are untouched.
- **Writes are atomic.** The result is built as a hidden `.orbit-*` scratch file
  in the destination folder and published with a hard link, so a half-written
  file never appears under a real name — and a crash cannot leave a torn one.
  Scratch files are cleaned up on every path, including cancellation.
- **Batches fan out.** One job per file, on half your cores (capped at four), so
  a fifty-file drop does not turn the machine into a space heater. A **merge** is
  the exception: it is one job holding every source, because one result is the
  whole point.
- **Progress streams to the ring.** The hub shows `2 of 5 done · 41%`, and stays
  open when something fails, with the engine's own reason.

Driven today, on top of FFmpeg and ImageMagick:

| Engine | What Orbit asks it to do |
|---|---|
| **FFmpeg** | Audio/video conversion, trim, extract audio, GIF with a palette, stills, slideshows, waveform/spectrum video, concatenation |
| **ImageMagick** | Raster conversion, resize, crop, rotate, flip, grayscale, strip metadata, compress, image → PDF layout, PDF page render |
| **Poppler** | PDF → text, PDF → PNG/JPEG/TIFF at a chosen DPI |
| **Ghostscript** | PDF compression (presets), PDF → image, PDF merge |
| **qpdf** | PDF page extraction, merge, metadata strip, rotate |
| **LibreOffice** | Document ⇄ PDF, document ⇄ text, document → data |
| **libheif** | HEIC/HEIF → image |
| **Tesseract** | OCR for images (PDF OCR lands with the series contract) |
| **orbit-native** | Built-in image ops (PNG/JPEG/BMP/TIFF), Markdown/plain text → HTML, CSV ↔ JSON |

The built-in **`orbit-native`** engine is compiled into Orbit, so the common image
edits and the text/table conversions work on a machine with no external tools at
all. It is deliberately narrow: three output formats for images, and only the
Markdown and CSV subset it actually implements. Anything else returns a message
naming the engine that *can* do it.

```bash
# The whole pipeline, against the real tools installed on this machine
cargo test -p orbit-jobs
```

The end-to-end tests skip themselves (loudly) when a tool is absent. On this
machine they drive real `qpdf`, `tesseract`, ImageMagick and FFmpeg — including
merging PDFs, keeping a page range, stripping metadata, reading a word back from
a PNG, laying an image onto A4, and writing a JPEG/HTML/JSON through the native
engine.

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
│   ├── orbit-jobs/           # ⚙️  resolve · plan · run · publish        (M2–M3)
│   │   ├── src/resolve.rs    #    petal pick  →  concrete JobRequests
│   │   ├── src/plan.rs       #    JobRequest  →  argv / native op (pure)
│   │   ├── src/native.rs     #    the engine that is not a process
│   │   ├── src/output.rs     #    naming, scratch files, atomic publish
│   │   └── src/runner.rs     #    worker pool, cancellation, progress
│   ├── orbit-app/            # 🪟 Tauri shell: tray, hotkey, overlay, pool (M1–M3)
│   └── orbit-cli/            # ⌨️  headless `orbit convert …`            (M5)
├── scripts/orbit             # ⌨️  `orbit [file …]` launcher + job log
├── scripts/windows/          # 🪟  "Send to → Orbit" installer
├── .github/workflows/        # 📦  CI on every push, installers on every tag
└── src/                      # ⚛️  React + Vite + Tailwind radial UI      (M1–M3)
```

## 📦 Building installers

```bash
npm ci
npm run tauri build     # installers for *this* system, in target/release/bundle/
```

Cross-compiling is not the path: bundling uses WiX and NSIS on Windows,
`hdiutil` on macOS, and `dpkg`/`rpmbuild`/`linuxdeploy` on Linux, so each
installer is built on its own machine. What you *can* do from anywhere is the
check that catches most platform breakage — the whole app, against the Windows
target:

```bash
rustup target add x86_64-pc-windows-msvc
cargo check -p orbit-app --target x86_64-pc-windows-msvc --features custom-protocol
```

| Workflow | Runs | Does |
|---|---|---|
| [`ci.yml`](.github/workflows/ci.yml) | every push and pull request | format, clippy and the test suite against real engines on Linux; the Windows-target check above; and, on `main`, installers for all three platforms as downloadable artifacts. |
| [`release.yml`](.github/workflows/release.yml) | a `v*` tag, or manually | builds the installers on all three platforms and files them under a **draft** release, so the notes can be read before anything ships. |

```bash
git tag v0.1.0 && git push origin v0.1.0
```

## 🗺️ Roadmap

| | Milestone | State |
|---|---|---|
| **M1** | Tray app, global hotkey, transparent overlay, radial menu UI | ✅ shipped |
| **M2** | Job runner, worker pool, image & audio conversion | ✅ shipped |
| **M3** | Options panel, PDF/document/OCR engines, merging, native engine, choosable hotkey | ✅ shipped |
| **M4a** | **Windows**: installers, CI, Send to, Windows-target verification | 🚚 shipping |
| **M4b** | Per-OS Shift-drag trigger and shell integration (Windows · macOS · X11) | ⏳ next |
| **M5** | Series output (page/frame runs), presets, history, watch folders, CLI | ⏳ planned |
| **M6** | Onboarding, settings surface, polish | ⏳ planned |

Windows ships *before* the Shift-drag trigger on purpose: the trigger is a small
per-OS piece of input plumbing, while everything behind it — the registry, the
engines, the options panel, the worker pool — is the same code everywhere.
Shipping the installers first leaves one feature of Windows-specific work rather
than a port.

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
you exactly what is wrong. The full guide — adding an engine, an action, an
option, the code invariants and the checks CI runs — is in
[`CONTRIBUTING.md`](CONTRIBUTING.md).

Bug reports and engine requests are just as welcome as pull requests. If a
conversion you expect is missing, the fastest fix is often a few lines of JSON,
and the issue will tell you which file.

## 📄 License

[Mozilla Public License 2.0](LICENSE) © surx.

Third-party components and the engines Orbit drives are listed, with their
licences, in [`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md).
