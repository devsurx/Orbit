# Contributing to Orbit

Thanks for being here. Orbit is small on purpose — a tray app, a registry of what
can become what, and the argv to make it happen — and most useful contributions
are **data, not code**.

---

## The fastest contribution: registry data

Everything the ring knows lives in [`registry/`](registry) as JSON, validated by
the test suite. A new format, a conversion rule that should exist, a better
default, a missing engine — all of it is a JSON edit plus one command:

```bash
cargo test -p orbit-registry     # validates every file and every cross-reference
cargo run -p orbit-registry --example menu -- .mov   # what does this file offer now?
```

`menu` is the interactive way to check your work: give it an extension and it
prints the ring that extension would produce, with the engine that would run each
petal and the pairs it covers.

### Adding a format — `registry/formats.json`

```json
{
  "id": "avif",
  "label": "AVIF",
  "kind": "image",
  "extensions": ["avif"],
  "mimes": ["image/avif"],
  "signature": ["???????ftypavif"],
  "can": { "read": true, "write": true },
  "readers": ["vips", "magick"],
  "writers": ["vips", "magick"],
  "properties": { "lossy": true, "alpha": true },
  "tags": ["modern", "web"],
  "rank": 40
}
```

- **`readers` / `writers`** name *engines*, and they are what makes a conversion
  possible. Adding a format to `writers` for an engine while the engine cannot
  actually write it is the one mistake the loader cannot catch.
- **`rank`** breaks ties when several formats are equally good targets. Higher is
  preferred; `720p`-style variants and web formats sit higher, archival lower.
- **`can`** is what the format is *for* in principle; the engine lists are what
  actually happens today. Keep them honest and separate.
- **`signature`** uses `?` as a wildcard byte. Add it when the format is
  detectable from content, so a file with the wrong extension is still handled.

### Adding a conversion rule — `registry/conversions.json`

```json
{
  "id": "image-to-avif",
  "label": "AVIF",
  "kind": "conversion",
  "from": { "kind": "image", "not": { "format": "svg" } },
  "to": "avif",
  "engines": ["vips", "magick"],
  "requires": ["op:raster"],
  "defaults": { "quality": 82 },
  "options": ["quality"],
  "rank": 60
}
```

- **`to`** is either a concrete format id or `@writable:<kind>`, which expands to
  every format of that kind that some engine can write. The second form is how
  one rule produces hundreds of pairs — there are 841 today from 22 rules.
- **`requires`** holds **only `op:*` capability tokens**, never a format
  condition. A rule says what it needs (`op:pdf_split`); the engines declare who
  provides it. That indirection is why removing an engine or trimming its
  capabilities shows up as a validation error instead of a failed command at 2am.
- **`engines`** is a preference order. The first available one runs. Put the fast,
  lossless or most faithful engine first.
- **`only`** restricts an expansion to a listed set when `@writable:` would
  otherwise overreach.

### Adding an action — `registry/actions.json`

Actions are the non-conversion petals: resize, trim, strip metadata, merge, OCR.

```json
{
  "id": "watermark",
  "label": "Watermark",
  "category": "edit",
  "icon": "stamp",
  "applies_to": ["image"],
  "engines": { "ffmpeg": { "op": "op:overlay" } },
  "options": ["text", "position", "opacity"],
  "defaults": { "position": "bottom-right", "opacity": 45 },
  "rank": 70,
  "tag": "watermarked",
  "output_ext": "keep"
}
```

Two fields decide how the action behaves when more than one file is selected:

- **`ring: false`** hides it from the ring and leaves it to the command palette
  or chains. `convert` uses this: the ring builds format-specific petals from the
  rules instead of one vague "Convert to…".
- **`combines: true`** means *many inputs, one output* — a merge. The action then
  receives every source in one job, ordered by the `merge_order` option, instead
  of one job per file.
- **`tag`** is what goes in the parentheses: `photo (watermarked).jpg`. Use
  `output_ext: "keep"` to stay in the source format, or name an extension.

### Adding an option — `registry/options.json`

```json
{
  "key": "position",
  "label": "Position",
  "type": "select",
  "choices": ["top-left", "top-right", "bottom-left", "bottom-right"],
  "default": "bottom-right",
  "show_if": { "key": "mode", "equals": "text" }
}
```

The widget, its range, its label, its placeholder and its `show_if` rule are all
data. The webview fetches this catalogue once — there is no second copy of a
label or a boundary in TypeScript, so a new option never means touching the UI.

Supported `type`s: `text`, `int`, `float`, `bool`, `select`, `file`, `dir`,
`color`, `range`. Add a `min`/`max` for numeric types; the frontend and the
planner both honour them.

### Adding an engine — `registry/engines.json`

```json
{
  "id": "inkscape",
  "label": "Inkscape",
  "binary": "inkscape",
  "alternate_binaries": ["inkscape1"],
  "version_args": ["--version"],
  "min_version": "1.0",
  "license": "GPL-3.0-or-later",
  "distribution": "system",
  "platforms": ["linux-x64", "macos-arm64", "windows-x64"],
  "provides": ["op:vector_raster", "op:svg"],
  "progress": "none",
  "cost": 4,
  "note": "Only ever executed as a separate process; see THIRD_PARTY_LICENSES.md."
}
```

`provides` is the important one — the `op:*` tokens that rules require. An engine
that claims a token it does not implement makes rules select it and then fail, so
claim exactly what the code in [`crates/orbit-jobs/src/plan.rs`](crates/orbit-jobs/src/plan.rs)
can actually build a command line for.

A brand-new engine also needs an argv builder in `plan.rs`; start from the
existing `build_*` functions, which are all shaped the same way.

---

## Working on the code

### Setup

```bash
git clone https://github.com/devsurx/Orbit && cd Orbit
npm ci
cargo build -p orbit-app --features custom-protocol
```

Requirements: **Rust 1.85+** (2024 edition), **Node 20+**, and the usual Tauri
system dependencies — `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`,
`libayatana-appindicator3-dev` and `librsvg2-dev` on Debian/Ubuntu.

**The `custom-protocol` feature is required for a runnable binary.** Without it
the webview points at the dev server and the app shows a blank ring.

```bash
# front end first: the Rust side embeds dist/ and does not rebuild it for you
npm run build && cargo build -p orbit-app --features custom-protocol

./target/debug/orbit-app /path/to/photo.png   # open the ring for a file
./target/debug/orbit-app                      # tray only, then the hotkey
```

There are no engines to install in order to develop: the built-in `orbit-native`
engine and the JSON registry make the registry and resolver tests run anywhere. The
end-to-end tests in [`crates/orbit-jobs/tests/real_engines.rs`](crates/orbit-jobs/tests/real_engines.rs)
**skip themselves loudly** when a tool is missing, so a machine with no FFmpeg
still gives a green run — read the output to see what actually ran.

### The checks

```bash
npm run typecheck                                          # TypeScript
npm run build                                              # the webview bundle
cargo fmt --all -- --check                                 # formatting
cargo clippy --workspace --all-targets -- -D warnings      # lints, warnings are errors
cargo test --workspace                                     # everything
```

The workspace is linted with `clippy::pedantic` **and `-D warnings`**, plus
`missing_docs = "warn"` and `unsafe_code = "deny"`. `too_many_lines` is capped at
100. If you find yourself wanting an `#[allow]`, prefer splitting the function —
the engine builders are long because there are a lot of engines, not because one
function needs to be clever.

### The invariants

These are the promises the README makes. A change that breaks one of them is a
bug even if every test passes:

| Invariant | Where it lives |
|---|---|
| **Never overwrite a source file.** Output is `name (tag).ext`, and a fresh name is searched for until one is free. | `orbit-jobs/src/output.rs` |
| **Publish atomically.** Results are written to a scratch file and renamed into place, so a cancelled run never leaves a half-file where a good one used to be. | `orbit-jobs/src/output.rs` |
| **No network. Ever.** No telemetry, no analytics, no update pings. | the whole workspace; there is no HTTP client dependency |
| **Never block the UI.** Jobs run on a capped worker pool; the ring stays interactive and cancellable. | `orbit-jobs/src/runner.rs` |
| **Execute engines, never link them.** GPL/AGPL engines are subprocesses. argv is built in the planner and run in the runner. | `orbit-jobs/src/plan.rs` |
| **Say what is true.** If a petal cannot do something yet, or a shortcut cannot be delivered on this platform, the note says so instead of failing quietly. | registry `note` fields, `WAYLAND_NOTE` |

### Style

- **Comments explain *why*.** The code says what it does. A comment earns its
  place by recording the reason, the quirk, or the thing that was tried and
  failed — `pdftoppm` wanting a prefix without an extension is a good comment; a
  restatement of the next line is not.
- **Errors are for users.** `ValueUnsupported`, not `InvalidInput`; name the
  engine that *can* do the thing when one exists.
- **Commits** read as `type(scope): what changed, in the imperative`, where
  `type` is one of `feat`, `fix`, `docs`, `test`, `refactor`, `chore`. The body is
  for the reasoning and the trade-off, not a diff summary.

### Where things live

| Path | What |
|---|---|
| `registry/*.json` | All the data: formats, rules, actions, engines, options |
| `crates/orbit-registry` | Loading, validating and querying that data |
| `crates/orbit-jobs/src/resolve.rs` | Petal pick → concrete jobs |
| `crates/orbit-jobs/src/plan.rs` | Job → argv, or a native op. Pure, no I/O, heavily unit-tested |
| `crates/orbit-jobs/src/native.rs` | The engine that is not a process |
| `crates/orbit-jobs/src/runner.rs` | Worker pool, cancellation, progress |
| `crates/orbit-app` | Tauri shell: tray, hotkey, overlay, settings, IPC |
| `src/` | React radial menu and options panel |

---

## Pull requests

- One concern per pull request, with the reasoning in the description.
- Add a test that would fail without the change. For a registry edit, that usually
  means an assertion in `crates/orbit-registry/tests/`; for a planner change, a
  case in `plan.rs`'s own tests; for a real command line, a test in
  `real_engines.rs` gated on the tool being installed.
- Run the checks above. CI runs them too, including installers on all three
  platforms from `main`.

If you are unsure whether something belongs in the registry or in code: **if a
different person might reasonably want it configured differently, it belongs in
the registry.**
