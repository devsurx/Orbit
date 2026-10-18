# `orbit-app` — the desktop shell 🪟

The Tauri half of Orbit: a tray icon, a global hotkey, and one frameless
transparent window that renders the ring. It contains **no conversion logic** —
every decision comes from [`orbit-registry`](../orbit-registry).

```text
  tray icon ─┐
             ├─▶  overlay window  ──IPC──▶  React ring  (../../src)
  hotkey ────┘        ▲
                      │ drop
                  file manager
```

## What it owns

| Concern | Where |
|---|---|
| Registry + engine probing + menu building | `Orbit` struct in `src/main.rs` |
| Tray icon, "Open ring" / "Reload engines" / "Quit" | `setup_tray` |
| Global hotkey (`Ctrl/Cmd+Shift+Space`) | plugin registration in `main` |
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

**Verified on Linux (Hyprland, Wayland, WebKitGTK 2.54):**

- The overlay is a working drop target and renders the full ring — hub, category
  ring, and petal fan — driven entirely by `orbit-registry`.
- `DragDropEvent` carries `Vec<PathBuf>`; no polling or platform code needed for
  the drop itself. Per-OS work in M4 is only about *detecting the Shift-drag
  gesture*.
- Drag events arrive on `tauri::Window`, **not** `WebviewWindow` — a subtle
  signature difference that costs a compile cycle if you guess.
- A Tauri debug build without `custom-protocol` serves `devUrl` and shows an
  empty window. Always pass the feature for a standalone binary.
- The CSP must include `connect-src ipc: http://ipc.localhost` or every
  `invoke()` is silently blocked and the UI renders an empty ring.

**Linux window transparency (platform limitation):**

`transparent: true` produces a toplevel that is never mapped — nothing renders,
under Wayland or XWayland, with or without the DMA-BUF workaround. Linux
therefore uses an opaque window via `tauri.linux.conf.json` and paints its own
dark panel; macOS/Windows keep real translucency.

**Not yet verified:** an actual human drag from a file manager onto the ring.
The code path is wired and the drop payload type is confirmed, but M1 was signed
off on the command-line "Open with Orbit" route, which exercises the identical
`classify → menu → emit` pipeline.


## Platform notes

| Platform | Hotkey | Tray | Drop |
|---|---|---|---|
| Linux (X11 / XWayland) | ✅ | ✅ | ✅ |
| Linux (Wayland) | ✅ | ✅ | ✅ |
| macOS | ✅ | ✅ | ✅ |
| Windows | ✅ | ✅ | ✅ |

Wayland is fine for **all three** of the above; it only blocks the *global
Shift-drag hook*, which is M4's problem and is documented in the root README.
