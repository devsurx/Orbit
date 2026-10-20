<div align="center">

# 🌀 Orbit

### Summon a file converter with a flick of your wrist.

Summon the ring, drop files on it, choose a radial petal, walk away. No
drag-and-drop onto a tiny window, no "Save As" dialog, no upload, no account.

<sub>Shift-drag any file, pick a petal, and the result lands beside it. Local
engines, no uploads, no account.</sub>

<br />

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL%202.0-6e4aff.svg)](https://www.mozilla.org/en-US/MPL/2.0/)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-2ea043.svg)](#-platform-support)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-dea584.svg?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Status](https://img.shields.io/badge/status-v0.2.0-2ea043.svg)](#-roadmap)
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

**Shift-drag a file** anywhere in the system and the ring appears under the
pointer, already offering the actions that fit it. Pick a petal — *Convert →
WebP*, *Compress*, *Trim* — and the result lands **next to the original** as
`photo (converted).webp`. Nothing is overwritten, nothing is uploaded.

No trigger to hand? Press <kbd>Ctrl</kbd>/<kbd>⌘</kbd> + <kbd>Shift</kbd> +
<kbd>Space</kbd>, click the tray icon, or hand Orbit a file from the shell:

```bash
orbit ~/Pictures/photo.png    # the ring opens under your cursor
orbit convert --to webp ~/Pictures/*.png --out ~/Pictures/web
```

<sub>First time here? → [Build from source](#-build-from-source) · [Settings & history](#settings-history-and-watch-folders) · [The `orbit` command](#-the-orbit-command) · [Platform support](#️-platform-support)</sub>

---

## ✨ What it is

Orbit is a **background utility** that lives in your tray and only appears when
you want it. When the ring is up it is a real drop target with *exactly* the
actions that fit the files — convert, compress, resize, trim, OCR, strip
metadata, merge a PDF, and more.

It is **local-first**: everything runs on your machine through native engines
you already trust (`FFmpeg`, ImageMagick, Ghostscript, …). Nothing is uploaded.

> Run one action, or apply a preset to fifty files while you make coffee.

## 🎯 Highlights

| | |
|---|---|
| 🖱️ **A real drop target** | The ring takes files the moment it is up; nothing to aim at. |
| ⌨️ **Shift-drag-anywhere** | Hold Shift while dragging a file; the ring opens under the pointer without stealing the drag. |
| 🧠 **Context-aware** | A `.mov` offers video petals; a folder of `.png` offers image tools. |
| 🎛️ **Defaults that work, options when you want them** | Drop to run with sensible values; Alt-click or hold for the panel. |
| 🔁 **841 conversion pairs** | Generated from a data-driven registry — add a format, not code. |
| 💾 **Never clobbers your files** | Results land beside the source as `name (converted).ext`, published atomically. |
| 🧰 **Editing, not just converting** | Resize, crop, trim, rotate, strip EXIF, compress, merge, split, OCR. |
| 🔗 **Merge as one job** | Several images → one PDF or GIF; PDFs → one PDF; audio/video concatenated. |
| ⚙️ **Batch & presets** | Name a workflow once, reuse it forever. |
| 🗂️ **Watch folders** | Point Orbit at a folder; new files are converted automatically, existing ones are left alone. |
| 🔎 **OCR & documents** | Turn scans into text (images today); documents via LibreOffice/Pandoc. |
| 📜 **History** | Every result is logged locally; the tray and settings show recent files and reveal them. |
| ⌨️ **A hotkey you choose** | Pick one from the tray or settings; it is remembered between launches. |
| 🧭 **A real settings window** | Trigger, output folder, watch folders and history, with a one-time welcome. |
| 🖥️ **A headless CLI** | `orbit convert`, `action`, `preset`, `list`, `history` for scripts and cron. |
| 🕵️ **Private by design** | Zero telemetry, zero network calls, no account, no cloud. |

## 🖥️ Platform support

The ring is always a real drop target. What varies per OS is only *how Orbit
learns that a Shift-drag started* — and that is genuinely blocked on Wayland
by design, so we say so instead of shipping something flaky.

| Platform | Shift-drag | Fallbacks | Notes |
|---|---|---|---|
| **Windows 10/11** | ✅ | hotkey · tray · Open with | Low-level mouse/keyboard hooks; the window is non-activating until a file lands. |
| **macOS 12+** | ✅ | hotkey · tray · Open with | Event taps; needs Accessibility permission the first time. |
| **Linux — X11** | ✅ | hotkey · tray · Open with | XRecord through `rdev`, plus the real XDND drop target. |
| **Linux — Wayland** | ⛔ | tray · Open with · compositor binding | No global Shift-drag **and no in-app hotkey** — see below. |

> ✅ **What works on every platform:** the overlay is a real drop target, the
> tray icon, the settings window, and handing files to Orbit from the shell —
> `orbit-app photo.png` selects them and opens the ring. That last route is the
> same one "Open with Orbit" and "Send to" use. Shift-drag and the global hotkey
> work wherever the OS allows a background app to observe input.

> 🐧 **Linux and ImageMagick.** The image pipeline is built on ImageMagick **7**
> (`magick`). Debian 13, Fedora and Arch ship it; Ubuntu LTS still ships
> ImageMagick 6, whose binary is `convert` and whose flags differ, so Orbit
> reports that engine as unavailable instead of guessing a command line. Install
> ImageMagick 7, or install `libvips-tools` — vips is the preferred image engine
> anyway — and the built-in engine covers the basic raster work either way.

> ⚠️ **Wayland has no global hotkey and no global Shift-drag.** The compositor
> owns input, and a client cannot observe a drag that started in another
> application. Orbit reports both as unavailable in **Settings → General** and
> prints a note at startup rather than leaving you pressing keys or holding
> Shift at nothing. Bind the launcher in your compositor instead (below). The
> ring also cannot be centred on the pointer, because a Wayland client cannot
> read the global pointer: the compositor places the window (usually centre).
> The tray, `Open with Orbit` and the settings window all still work.

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
Microsoft — the only network access the installer ever performs, and it
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
> out of the real toolchain rather than a cross-compile. The Shift-drag hook is
> implemented against the Windows low-level input API, but *runtime* behaviour
> — the tray menu, the hook and the ring on your desktop — can only be verified
> on Windows. If something there misbehaves, an issue with the app log is the
> fastest path to a fix.

### 🍎 macOS

Shift-drag and the global shortcut need **Accessibility** permission, because
macOS treats global input as a privileged capability. The first time Orbit asks,
allow it in **System Settings → Privacy & Security → Accessibility**; if the
prompt was dismissed, add the app there manually. Without it the tray, drop
target, settings window and `Open with Orbit` all still work.

The DMG contains separate Apple-silicon and Intel builds. Nothing is notarised
yet, so the first launch may need **Open** from the context menu in Finder.

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
| PDF → image | DPI, pages (a range becomes a numbered series), quality |
| Extract frames | Frame step, frame limit, quality |
| Merge | Result (PDF / GIF / MP4 / MP3), order (as dragged / by name / oldest first) |
| Extract pages | `1-3` or `2-` — the range is written into one PDF |
| Crop | `left,top,width,height`, or ImageMagick's `WxH+X+Y` |
| OCR | Language and output format |

> 📄 **Ranges become series.** `PDF → image` with `pages=1-3` writes
> `scan (page 001).png`, `scan (page 002).png`, …; `Extract frames` with
> `frame_step=5` writes `clip (frame 001).png`, … A single page keeps the plain
> `scan (page).png` name. The whole run is reserved as a block, so no name can
> collide with an existing file, and intermediates are cleaned up on failure.
>
> 🔗 **Presets are chains.** A preset is a sequence of resolved steps with hidden
> scratch files between them; only the last step is published. The registry's
> `chains.json` is the data, and the CLI exposes the same chains.

## Settings, history and watch folders

Click **Settings** in the ring, choose **Settings…** from the tray, or let the
one-time welcome open it on first run. It is a normal window, not the ring, so
the long-form things have room:

- **Shift-drag** — on or off. On Wayland the toggle is disabled and the window
  explains why.
- **Global shortcut** — the same presets as the tray, applied immediately.
- **Output folder** — leave it empty to write beside each source, or pick a
  folder (the picker is native). Results keep their `name (converted).ext`
  convention either way.
- **Watch folders** — a folder, plus one rule: an action, a preset, or a target
  format. Existing files are only memorised; files that appear later are queued.
  A folder is scanned every two seconds, subfolders three levels deep, and a file
  must be two seconds old before it is picked up — so a copy that is still
  running is not grabbed halfway through. Orbit also ignores paths it just
  published, which is what stops a result written back into the watched folder
  from triggering another conversion.
- **History** — the newest 100 jobs, with **Show** to reveal a result in the file
  manager, and **Clear history** to forget them.

The settings document is written atomically to the app config directory
(`~/.config/app.orbit.desktop/settings.json` on Linux,
`~/Library/Application Support/app.orbit.desktop/` on macOS,
`%APPDATA%\app.orbit.desktop\` on Windows). History is a separate, append-only
JSONL file under the state directory:

| Platform | History |
|---|---|
| Linux | `~/.local/state/orbit/history.jsonl` |
| macOS | `~/Library/Application Support/Orbit/history.jsonl` |
| Windows | `%LOCALAPPDATA%\Orbit\history.jsonl` |

`ORBIT_HISTORY=/some/path.jsonl` overrides it, for tests and for people who keep
their state elsewhere. History is a convenience, never a source of truth: a line
truncated by a crash costs one entry, not the log, and the files themselves are
always the record. Nothing in either file leaves the machine.

## 🚀 Build from source

**Prerequisites:** Rust 1.85+ (2024 edition), Node 20+, and the platform
webview/Tauri dependencies.

<details open>
<summary><b>Linux (Debian / Ubuntu)</b></summary>

```bash
sudo apt-get update
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
  librsvg2-dev libxtst-dev libxi-dev patchelf
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
one word. The installers ship the GUI (`orbit-app`); the launcher and
`orbit-cli` are repository tools, so build the CLI once with
`cargo build --release -p orbit-cli` and link the launcher from the checkout:

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

The launcher also delegates headless commands to `orbit-cli`, so scripts and cron
jobs can use the same registry and the same defaults the ring uses:

```bash
orbit convert --to webp photo.png                 # beside the source
orbit convert --to png scan.pdf --option pages=1-3
orbit action extract_frames clip.mp4 --option frame_step=5
orbit preset web-ready *.jpg --out ~/Pictures/web
orbit list formats                                # what can be written
orbit history --limit 20                          # the same log the window shows
```

`--option key=value` uses exactly the keys the options panel shows (`orbit list
actions` prints them), and `--json` makes `list` and `history` script-friendly.
The GUI and the CLI share the resolver and the engines; a pick that is disabled
in the ring is refused by the CLI with the same reason.

## 🧩 First run

The first launch opens **Settings → General** with a short welcome. It says the
three things that make Orbit make sense: how to summon it, where results go, and
that nothing leaves the machine. **Got it** dismisses it for good; **Set up a
watch folder** jumps straight to the folder editor. The welcome is not a modal
and it does not block the ring: if you would rather just use the app, close the
window and press the shortcut.

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

- **Where it went:** beside the source, or in the output folder you chose in
  settings. `orbit --log` prints the paths of everything Orbit has published, if
  you would rather not go looking; the tray and settings history keep the recent
  ones.
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
│   ├── orbit-jobs/           # ⚙️  resolve · plan · run · publish
│   │   ├── src/resolve.rs    #    petal pick  →  concrete JobRequests
│   │   ├── src/plan.rs       #    JobRequest  →  argv / native op (pure)
│   │   ├── src/native.rs     #    the engine that is not a process
│   │   ├── src/output.rs     #    naming, scratch files, atomic publish
│   │   ├── src/runner.rs     #    worker pool, chains, cancellation, progress
│   │   └── src/history.rs    #    append-only JSONL log, compaction
│   ├── orbit-app/            # 🪟 Tauri shell, tray, hotkey, overlay
│   │   ├── src/trigger.rs    #    Shift-drag detector + global input listener
│   │   ├── src/settings.rs   #    persisted settings, hotkey presets
│   │   ├── src/watch.rs      #    watch-folder scanner and queueing
│   │   └── src/main.rs       #    commands, tray, dispatch, single instance
│   └── orbit-cli/            # ⌨️  headless `orbit convert …`
├── scripts/orbit             # ⌨️  `orbit [file …]` launcher + job log
├── scripts/windows/          # 🪟  "Send to → Orbit" installer
├── .github/workflows/        # 📦  CI on every push, installers on every tag
└── src/                      # ⚛️  React + Vite + Tailwind radial UI
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
git tag v0.2.0 && git push origin v0.2.0
```

## 🗺️ Roadmap

| | Milestone | State |
|---|---|---|
| **M1** | Tray app, global hotkey, transparent overlay, radial menu UI | ✅ shipped |
| **M2** | Job runner, worker pool, image & audio conversion | ✅ shipped |
| **M3** | Options panel, PDF/document/OCR engines, merging, native engine, choosable hotkey | ✅ shipped |
| **M4a** | **Windows**: installers, CI, Send to, Windows-target verification | ✅ shipped |
| **M4b** | Per-OS Shift-drag trigger and shell integration (Windows · macOS · X11) | ✅ shipped |
| **M5** | Series output (page/frame runs), presets, history, watch folders, CLI | ✅ shipped |
| **M6** | Onboarding, settings surface, polish | ✅ shipped |

### What is next

- **Optional update check** — user-triggered, off by default, the only network
  call the app would ever make.
- **PDF OCR** — the text layer path is designed, but it needs a page rasteriser
  and a round-trip to Tesseract that is honest about mixed results.
- **Re-run from history** — the log already carries the action and sources; the
  UI to put them back on the ring is the missing piece.
- **More registry data** — the matrix is generated, so new formats and actions
  are JSON, not code. See [`CONTRIBUTING.md`](CONTRIBUTING.md).

### Verification, honestly

Linux is where this release has been run end-to-end against real engines. The
Windows and macOS applications type-check against their targets on every push,
and their installers are built on the real operating systems by CI — but the
Shift-drag hook, the tray and the native folder picker cannot be *runtime*-tested
from Linux. The release notes say so, and issue reports from those platforms are
the fastest way to close the gap.

## 🔒 Privacy

No telemetry. No analytics. No accounts. No network calls at all today — not
even an update check. When the optional update check lands it will be off by
default, user-triggered and the only request the app ever makes. Your files never
leave your machine.

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
