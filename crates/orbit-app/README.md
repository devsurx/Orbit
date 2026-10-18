# `orbit-app` — the desktop shell 🪟

The Tauri half of Orbit: a tray icon, a trigger, and one frameless window that
renders the ring. It contains **no conversion logic** — every decision comes from
[`orbit-registry`](../orbit-registry).

```text
  tray icon ─────┐
  in-app hotkey ─┼─▶  overlay window  ──IPC──▶  React ring  (../../src)
  compositor ────┘        ▲
   (single-instance)      │ drop
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
| Window hidden on close instead of quitting | `WindowEvent::CloseRequested` |

## IPC contract

| Direction | Name | Payload |
|---|---|---|
| JS → Rust | `menu_for(paths)` | `FileSummary[]`-bearing `MenuModel` |
| JS → Rust | `dispatch(petalKind, reference, files)` | `DispatchAck` |
| Rust → JS | `orbit://menu` | `MenuModel` |
| Rust → JS | `orbit://drag` | `{ active: boolean, paths: string[] }` |

`dispatch` currently acknowledges the intent and says so — real execution is
milestone M2. This keeps M1 honest: the ring is real, the jobs are not yet.

## Running it

```bash
# from the repository root
npm install          # once
npm run build        # produces ./dist, which Tauri embeds
cargo run -p orbit-app --features custom-protocol
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
  and highlights it. Picking one round-trips through IPC and echoes back into the
  hub: `Convert to PNG — 1 file, via conversion engine`.
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
