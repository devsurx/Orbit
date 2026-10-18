# `orbit-app` — the desktop shell 🪟

The Tauri half of Orbit: a tray icon, a trigger, and one frameless window that
renders the ring. It contains **no conversion logic** — every decision comes from
[`orbit-registry`](../orbit-registry), and every command line from
[`orbit-jobs`](../orbit-jobs).

```text
  tray icon ─────┐                    ┌─▶ worker pool ─▶ FFmpeg · ImageMagick · vips
  in-app hotkey ─┼─▶ overlay window ─▶│         │
  compositor ────┘         ▲    IPC   │         ▼
   (single-instance)       │          └─▶ orbit://job ─▶ progress in the hub
                        drop
                    file manager
```

## What it owns

| Concern | Where |
|---|---|
| Registry + engine probing + menu building | `Orbit` struct in `src/main.rs` |
| Tray icon, "Open ring" / "Reload engines" / "Quit" | `setup_tray` |
| In-app global hotkey (`Ctrl/Cmd+Shift+Space`) | `register_hotkey` |
| Second launch → hand its files to the live instance | `tauri-plugin-single-instance` in `main` |
| Ring placement at the pointer | `position_at_cursor` |
| Drag-drop → ring update | `handle_drag` |
| Checking a pick against the ring that offered it | `Orbit::petal_for` |
| Petal pick → concrete jobs | `dispatch` → `orbit_jobs::resolve_pick` |
| Worker pool, ids, shutdown | `Jobs` struct, built in `setup` |
| Progress → webview | the `Runner` emitter, `emit_to(RING, JOB_EVENT, …)` |
| Window hidden on close instead of quitting | `WindowEvent::CloseRequested` |

## IPC contract

| Direction | Name | Payload |
|---|---|---|
| JS → Rust | `menu_for(paths)` | `FileSummary[]`-bearing `MenuModel` |
| JS → Rust | `dispatch(petalKind, reference, files)` | `DispatchAck` |
| Rust → JS | `orbit://menu` | `MenuModel` |
| Rust → JS | `orbit://drag` | `{ active: boolean, paths: string[] }` |
| Rust → JS | `orbit://job` | `{ id, label, state, … }` (see below) |

`dispatch` is the only command that can start work, and it trusts the webview for
exactly **two** strings: the petal's kind and its reference. Everything else — the
option values, the engine, the extension, the output word — is looked up in the
`MenuModel` the shell actually sent, so a stale or hostile window cannot invent a
petal or run one against options that were never on offer. It resolves one job per
file, queues them, and answers with a summary; the jobs then report themselves:

```jsonc
{ "id": 1, "label": "Convert to WebP", "state": "queued" }
{ "id": 1, "label": "Convert to WebP", "state": "running", "percent": 41 }
{ "id": 1, "label": "Convert to WebP", "state": "done", "path": "/…/photo (converted).webp" }
{ "id": 1, "label": "Convert to WebP", "state": "failed", "message": "…" }
```

A pick that suits only *some* of the dragged files still succeeds: `DispatchAck`
reports the count and the first reason in `detail`. The ring only leaves the screen
once nothing is in flight — and it **stays** up when something failed, so the
reason is readable instead of being swallowed.

## Running it

```bash
# from the repository root
npm install          # once
npm run build        # produces ./dist, which Tauri embeds
cargo run -p orbit-app --features custom-protocol

# Or hand it files directly, which is also the easiest way to see the ring:
cargo run -p orbit-app --features custom-protocol -- ~/Pictures/photo.png

# No window needed to exercise the pipeline itself:
cargo test -p orbit-jobs
```

`custom-protocol` is what switches Tauri from the Vite dev server to the frontend
embedded in the binary. Without it you get a window with no page in it. For
live-reloading frontend work use the Tauri CLI, which wires Vite to the window and
sets the feature itself:

```bash
npm run tauri dev
```

## M1 spike: does the overlay really receive drops?

The load-bearing question for the whole product: does Tauri 2 hand us **real
file paths** when a file is dropped on an always-on-top, frameless window — and
can the same window be interactive?

Steps (Linux X11/Wayland, macOS, Windows):

1. Launch the app; confirm the **tray icon** appears.
2. Press <kbd>Ctrl/Cmd</kbd>+<kbd>Shift</kbd>+<kbd>Space</kbd> — the ring should
   appear centred on the pointer.
3. Drag a file (e.g. a `.png` or `.mov`) from your file manager onto the ring.
   On `DragDropEvent::Enter` the ring lights up; on `Drop` it should rebuild and
   show that file's conversions.
4. Press <kbd>Esc</kbd> or click outside — the ring hides; the tray remains.
5. Repeat (2) — the ring reopens with the **same** selection.

### Findings

**Verified on Linux (Hyprland, Wayland, WebKitGTK 2.54), by screenshot:**

- The overlay renders the whole ring — hub, category ring and petal fan — driven
  entirely by `orbit-registry`, from the idle state through to a live selection.
- Hovering a category fans its petals out. Hovering a petal previews it in the hub
  and highlights it. Picking one round-trips through IPC — in M1 that echoed a stub
  acknowledgement back into the hub; M2 replaced the stub with the real job
  pipeline below.
- `DragDropEvent` carries `Vec<PathBuf>`, so *receiving* files needs no platform
  code; M4 only has to detect the Shift-drag gesture that summons the ring.
- Drag events arrive on `tauri::Window`, **not** `WebviewWindow` — a signature
  difference that costs a compile cycle if you guess.

**Four traps this milestone paid for:**

1. **`custom-protocol` is load-bearing.** Without it a debug build sets Tauri's
   `dev = true` and loads `devUrl`, so the window comes up with no page in it.
2. **The CSP must include `connect-src ipc: http://ipc.localhost`.** Tauri does not
   add the IPC origin for you, and without it every `invoke()` fails *silently* —
   you get a blank ring and no error.
3. **A filling CSS animation replaces your transform utilities.** `animate-petal`
   used `both`, so its final `transform: scale(1)` overrode
   `-translate-x-1/2 -translate-y-1/2`: every petal chip was pinned by its
   top-left corner instead of centred on the ring, which shoved the wide ones past
   the window edge and quietly killed the hover scale. The centring translate now
   lives *inside* the keyframes and the animation no longer fills forwards. The
   category chips measured perfectly throughout, which is the tell — they have no
   animation.
4. **Editing `tauri.linux.conf.json` does not re-run the build script.** The merged
   config is baked in at compile time, so a config-only change can silently fail to
   ship. Touch `build.rs` when the window config changes.

**Geometry budget.** The overlay is 640×640 with petals at r=238. Petals meet the
window edge first at the horizontal, so the invariant is roughly
`R_PETAL + half the widest chip ≤ RING_SIZE / 2` — 238 + 64 ≤ 320, with room to
spare.

**Linux window transparency (platform limitation):**

`transparent: true` produces a toplevel that is never mapped — nothing renders,
under Wayland or XWayland, with or without the DMA-BUF workaround. Linux therefore
uses an opaque window via `tauri.linux.conf.json`, paints its own dark panel, and
sets `backgroundColor` so the area outside the panel's corner radius is dark too;
without that it shows the platform default, a light grey wedge in each corner.
macOS/Windows keep real translucency.

## M2: does it actually convert?

The shell's whole contribution to a conversion is *four lines*: look up the petal
in the model that was last sent, build a `Pick`, call `orbit_jobs::resolve_pick`,
and `submit` each request. Everything hard — which engine, which flags, which name,
whether the name is free — belongs to `orbit-jobs`, which is why it can be tested
without a window.

**Verified by running the binary, on this machine (Linux, Hyprland, Wayland):**

| Run | What the hub said | What the disk said |
|---|---|---|
| Convert a PNG | `Convert to JPEG — 1 file` / `1 file done` | `holiday (converted).jpg` — valid JPEG, 640×400; source byte-identical |
| Convert two WAVs | `Convert to MP3 — 2 files` / `0 of 2 done` while running | `first (converted).mp3` **and** `second (converted).mp3`, each named after its own source |
| Same pick twice | — | the second run produced `holiday (converted 2).jpg`, proving the no-clobber rule |
| Compress a PDF | `Compress — 1 file` / `1 of 1 failed — compress on ghostscript lands in M3` | nothing written: the plan refused before spawning |

No `.orbit-*` scratch file survived any run, and the ring stayed on screen after the
failure instead of disappearing with the message unread.

**The cursor cannot be moved on this Hyprland build** (its config is Lua, so
`hyprctl dispatch movecursor` does not parse; `ydotool`/`wtype` are not installed).
These runs therefore drove `dispatch` from the frontend at boot — the identical
call a click makes, with the same arguments — and the temporary hook was removed
before committing. Hover, which needs the pointer, was verified in M1 the same way.

**Not yet verified:** a human drag from a file manager onto the ring. The handler is
wired and the payload type is confirmed, but every screenshot so far came from the
"Open with Orbit" command-line route, which drives the identical
`classify → menu → emit` pipeline.

## Platform notes

| Platform | In-app hotkey | Compositor binding | Tray |
|---|---|---|---|
| Linux (X11 / XWayland) | ✅ | — | ✅ |
| Linux (Wayland) | ❌ | ✅ | ✅ |
| macOS | ✅ | — | ✅ |
| Windows | ✅ | — | ✅ |

`global-hotkey` picks an **X11-only** backend on Linux
(`#[cfg(target_os = "linux")] → x11/mod.rs`); no Wayland path exists. Under Wayland
the `XGrabKey` grab is accepted and then never fires, because the compositor owns
input. Orbit detects this and says so at startup instead of failing silently.

That is why the shell registers `tauri-plugin-single-instance`: it lets a
*compositor* keybinding invoke `orbit-app`, which forwards its arguments to the
running instance and exits instead of starting a second copy. See "Global hotkeys
on Wayland" in the root README.
