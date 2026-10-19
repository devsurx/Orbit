# Third-party licences

Orbit itself is [MPL-2.0](LICENSE). This file covers everything Orbit *uses*.

There are two kinds of dependency here, and they are licensed differently:

1. **Code linked into Orbit** — the Rust crates and the frontend packages. These
   are compiled or bundled into the binary.
2. **Engines Orbit drives** — `ffmpeg`, ImageMagick, Ghostscript and the rest.
   **These are not bundled.** Orbit looks for them on `PATH`, runs them as
   separate processes and never redistributes them, so their licences bind
   whoever installed them, not this repository. They are listed anyway, because
   you deserve to know what will run on your machine.

---

## 1. Linked into the binary

Vendored or bundled in the same sense as the app: if you redistribute Orbit, you
redistribute these too.

### Rust dependencies

Direct dependencies only; the full transitive set is pinned in
[`Cargo.lock`](Cargo.lock) and can be listed with `cargo tree` or
[`cargo-about`](https://github.com/EmbarkStudios/cargo-about).

| Crate | Licence | Used for |
|---|---|---|
| [`serde`](https://crates.io/crates/serde) | MIT OR Apache-2.0 | Registry and IPC serialisation |
| [`serde_json`](https://crates.io/crates/serde_json) | MIT OR Apache-2.0 | Registry data, option values, the settings file |
| [`thiserror`](https://crates.io/crates/thiserror) | MIT OR Apache-2.0 | Error types in the registry and job crates |
| [`image`](https://crates.io/crates/image) | MIT OR Apache-2.0 | The built-in `orbit-native` engine (PNG, JPEG, BMP, TIFF) |
| [`tauri`](https://crates.io/crates/tauri) | MIT OR Apache-2.0 | The application shell, tray, window and webview |
| [`tauri-plugin-global-shortcut`](https://crates.io/crates/tauri-plugin-global-shortcut) | MIT OR Apache-2.0 | The configurable global shortcut |
| [`tauri-plugin-single-instance`](https://crates.io/crates/tauri-plugin-single-instance) | MIT OR Apache-2.0 | Handing a second launch to the running instance |

`image` is configured without its default features, so the decoder and encoder
set is exactly PNG, JPEG, BMP and TIFF — the formats the built-in engine
implements.

### Frontend packages

| Package | Licence | Used for |
|---|---|---|
| [`@tauri-apps/api`](https://www.npmjs.com/package/@tauri-apps/api) | Apache-2.0 OR MIT | Talking to the Rust side from the webview |
| [`react`](https://www.npmjs.com/package/react) · [`react-dom`](https://www.npmjs.com/package/react-dom) | MIT | The radial menu |
| [`vite`](https://www.npmjs.com/package/vite) | MIT | Build tool (build time only) |
| [`tailwindcss`](https://www.npmjs.com/package/tailwindcss) | MIT | Styling (build time only) |
| [`typescript`](https://www.npmjs.com/package/typescript) | Apache-2.0 | Types (build time only) |
| [`@tauri-apps/cli`](https://www.npmjs.com/package/@tauri-apps/cli) | Apache-2.0 OR MIT | `tauri build` / `tauri dev` (build time only) |
| [`postcss`](https://www.npmjs.com/package/postcss) · [`autoprefixer`](https://www.npmjs.com/package/autoprefixer) | MIT | CSS pipeline (build time only) |
| [`@vitejs/plugin-react`](https://www.npmjs.com/package/@vitejs/plugin-react) | MIT | React fast refresh (build time only) |

### Runtimes the app requires

| Component | Licence | Note |
|---|---|---|
| **WebView2** (Windows) | Microsoft proprietary, redistributable | Ships with Windows 11; the installer fetches the bootstrapper if it is missing. |
| **WebKitGTK** (Linux) | LGPL-2.1-or-later | A system package (`libwebkit2gtk-4.1-0`), declared by the `.deb`. |
| **WebKit** (macOS) | System framework | Part of macOS. |

---

## 2. Engines Orbit drives

Orbit is a **front end for tools you already have**. It detects these on `PATH`
and executes them as separate processes. None of them are bundled into the
installers today, and Orbit does not modify or relicense them.

| Engine | Licence | Distribution | Note |
|---|---|---|---|
| **FFmpeg** | LGPL-2.1-or-later | detected / bundled later | Must be an **LGPL build**, not GPL, for anything redistributed. |
| **ImageMagick** | ImageMagick License (Apache-2.0-like) | detected / bundled later | |
| **libvips** | LGPL-2.1-or-later | detected / bundled later | Preferred bitmap engine when present. |
| **Ghostscript** | **AGPL-3.0-only** | detected, optional | ⚠️ **Copyleft over the network.** Only ever run as a separate process at the user's request; never linked, never bundled with Orbit. If you redistribute an Orbit build that *ships* Ghostscript, that is your AGPL obligation to meet, not Orbit's. |
| **qpdf** | Apache-2.0 | detected / bundled later | |
| **Poppler** (`pdftoppm`, `pdftotext`, `pdfinfo`) | GPL-2.0-only | system package | Executed as a program, not linked. |
| **LibreOffice** | MPL-2.0 | system install | Launched headlessly with a private user profile. |
| **Tesseract** | Apache-2.0 | detected, optional | OCR language data is licensed separately per language. |
| **Pandoc** | GPL-2.0-or-later | detected | Executed as a program, not linked. |
| **libheif** | LGPL-3.0 | detected / bundled later | HEIC/HEIF decoding. |

### Why "executed, not linked" matters

A GPL or AGPL *library* linked into Orbit would place obligations on Orbit's own
source. A GPL or AGPL *program* invoked with `std::process::Command` — which is
exactly what [`orbit-jobs`](crates/orbit-jobs/src/plan.rs) does — does not. That
is a deliberate architectural choice, not an accident, and it is the reason the
planner emits argv rather than calling into any codec library.

---

## Reporting

If you believe something here is missing, mislicensed, or wrong, please open an
issue — accuracy matters more than brevity in this file.
