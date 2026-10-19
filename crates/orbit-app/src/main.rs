//! Orbit's desktop shell: tray icon, global hotkey and the radial overlay.
//!
//! The shell owns three things and nothing else:
//!
//! * a **tray icon**, so Orbit is reachable on machines where global hotkeys or
//!   Shift-drag are unavailable (Wayland, locked-down macOS);
//! * a **trigger**: an in-process global hotkey where the platform allows one,
//!   plus single-instance handling so a *compositor* binding can drive Orbit
//!   where it does not (Wayland — see the README);
//! * the **overlay window**, a real drop target that renders the ring, and the
//!   **worker pool** behind it, so a picked petal turns into a file.
//!
//! Every decision — which petals exist, what they do, why one is greyed out,
//! which engine runs, where the result is written — lives in `orbit-registry`
//! and `orbit-jobs`. This file only moves values between the registry, the job
//! engine, the operating system and the webview.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod settings;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use orbit_jobs::{Job, JobEvent, JobState, Pick, PickKind, Runner, resolve_pick};
use orbit_registry::{
    EngineStatus, MenuModel, MenuRequest, OptionSpec, Petal, PetalKind, Registry, SourceFile,
};
use serde::Serialize;
use tauri::menu::{Menu, MenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, Runtime, State, WebviewWindow, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::settings::{HotkeyStatus, Settings};

/// Ring window label; must match `tauri.conf.json`.
const RING: &str = "ring";
/// Ring diameter; must match the window size in `tauri.conf.json`.
const RING_SIZE: f64 = 640.0;
/// Event carrying the current ring contents to the webview.
const MENU_EVENT: &str = "orbit://menu";
/// Event carrying drag-hover state, so the ring can light up before a drop.
const DRAG_EVENT: &str = "orbit://drag";
/// Event carrying job progress, completion and failure.
const JOB_EVENT: &str = "orbit://job";
/// Event carrying a changed global shortcut, so the ring can update its hint.
const HOTKEY_EVENT: &str = "orbit://hotkey";

/// Everything the commands need, built once at startup.
struct Orbit {
    registry: Registry,
    /// Engines detected at startup; refreshed when the user installs one.
    ///
    /// The whole status is kept, not just the ids: running a job needs the
    /// resolved executable as well as the answer to "is it there?".
    statuses: Mutex<BTreeMap<String, EngineStatus>>,
    /// Last dropped selection, so the hotkey can reopen the same ring.
    last_drop: Mutex<Vec<PathBuf>>,
    /// The ring as last rendered. A pick is checked against it, so a stale
    /// window cannot ask for a petal that was never offered.
    model: Mutex<Option<MenuModel>>,
    /// Settings as loaded, and where they came from.
    settings: Mutex<Settings>,
    settings_path: PathBuf,
    /// What the global shortcut is doing right now.
    hotkey: Mutex<HotkeyStatus>,
}

impl Orbit {
    /// Load the registry, the settings, and probe which engines exist here.
    fn new<R: Runtime>(app: &AppHandle<R>) -> Result<Self, String> {
        let registry = Registry::bundled().map_err(|error| error.to_string())?;
        let statuses = detect_engines(&registry);
        let settings_path = app.path().app_config_dir().map_or_else(
            |_| PathBuf::from("orbit-settings.json"),
            |dir| settings::settings_path(&dir),
        );
        let settings = Settings::load(&settings_path);
        Ok(Self {
            registry,
            statuses: Mutex::new(statuses),
            last_drop: Mutex::new(Vec::new()),
            model: Mutex::new(None),
            settings: Mutex::new(settings),
            settings_path,
            hotkey: Mutex::new(HotkeyStatus::off()),
        })
    }

    /// Engine statuses as of startup, or the last refresh.
    fn statuses(&self) -> BTreeMap<String, EngineStatus> {
        self.statuses
            .lock()
            .map(|map| map.clone())
            .unwrap_or_default()
    }

    /// Engine ids usable right now.
    fn available(&self) -> BTreeSet<String> {
        orbit_registry::available_ids(&self.statuses())
    }

    /// Re-probe the `PATH`, for after the user installs `FFmpeg` or similar.
    fn refresh_engines(&self) {
        let detected = detect_engines(&self.registry);
        if let Ok(mut map) = self.statuses.lock() {
            *map = detected;
        }
    }

    /// The selection the ring is currently describing.
    fn selection(&self) -> Vec<PathBuf> {
        self.last_drop
            .lock()
            .map(|paths| paths.clone())
            .unwrap_or_default()
    }

    /// Classify real paths, carrying size when the file exists.
    ///
    /// Takes the paths by value so callers can hand over their own `Vec`
    /// without cloning.
    fn classify<I, P>(&self, paths: I) -> Vec<SourceFile>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        paths
            .into_iter()
            .map(|path| {
                let path = path.as_ref();
                let size = std::fs::metadata(path).map_or(0, |meta| meta.len());
                self.registry.classify(path, size)
            })
            .collect()
    }

    /// Build the ring for a selection, bound to the engines actually present.
    ///
    /// The result is remembered, because it is what a later pick is checked
    /// against.
    fn menu(&self, files: Vec<SourceFile>) -> MenuModel {
        let mut request = MenuRequest::for_files(files);
        request.available = Some(self.available());
        let model = self.registry.menu(&request);
        if let Ok(mut stored) = self.model.lock() {
            *stored = Some(model.clone());
        }
        model
    }

    /// The ring for the last dropped selection, which is what the hotkey shows.
    fn current_menu(&self) -> MenuModel {
        self.menu(self.classify(self.selection()))
    }

    /// The global shortcut as it stands, for the ring and the tray.
    fn hotkey(&self) -> HotkeyStatus {
        self.hotkey
            .lock()
            .map_or_else(|_| HotkeyStatus::off(), |status| status.clone())
    }

    /// The petal the ring last offered for this pick, if it still offers it.
    fn petal_for(&self, kind: PickKind, reference: &str) -> Option<Petal> {
        let stored = self.model.lock().ok()?;
        let model = stored.as_ref()?;
        model
            .categories
            .iter()
            .flat_map(|category| category.petals.iter())
            .find(|petal| petal.reference == reference && pick_kind_of(petal.kind) == kind)
            .cloned()
    }
}

/// The pool that runs jobs, plus the id counter the ring needs.
struct Jobs {
    /// Locked because a `Sender` is not `Sync` on its own, and because a pick
    /// that arrives mid-shutdown should queue rather than race.
    runner: Mutex<Runner>,
    next_id: AtomicU64,
}

impl Drop for Jobs {
    fn drop(&mut self) {
        // Cancels anything still running instead of leaving engine children
        // behind after the tray has quit.
        if let Ok(mut runner) = self.runner.lock() {
            runner.shutdown();
        }
    }
}

/// How many jobs to run at once.
///
/// Half the cores, capped at four. Video encoding already uses several threads
/// internally, and a background converter that saturates the machine on a batch
/// is a converter people turn off.
fn worker_count() -> usize {
    std::thread::available_parallelism().map_or(2, |cores| (cores.get() / 2).clamp(1, 4))
}

/// Which engines of a registry are present on this machine right now.
fn detect_engines(registry: &Registry) -> BTreeMap<String, EngineStatus> {
    registry.detect_engines(
        orbit_registry::host_platform(),
        std::env::var("PATH").ok().as_deref(),
    )
}

/// Echo a job that reached a terminal state to standard error.
///
/// The ring shows the result for about a second, which is not long enough to
/// find a file a minute later. The `orbit` launcher captures this stream into
/// `~/.local/state/orbit/jobs.log`, so every published path is recorded even
/// once the ring has faded.
fn announce(event: &JobEvent) {
    match &event.state {
        JobState::Done { path } => eprintln!("orbit: wrote {}", path.display()),
        JobState::Failed { message } => eprintln!("orbit: failed {} — {message}", event.label),
        JobState::Cancelled => eprintln!("orbit: cancelled {}", event.label),
        JobState::Queued | JobState::Running { .. } => {}
    }
}

/// The pick kind a ring petal stands for.
fn pick_kind_of(kind: PetalKind) -> PickKind {
    match kind {
        PetalKind::Conversion => PickKind::Conversion,
        PetalKind::Action => PickKind::Action,
        PetalKind::Preset => PickKind::Preset,
        PetalKind::Operator => PickKind::Operator,
    }
}

/// What the webview shows after a petal is picked.
#[derive(Debug, Serialize)]
struct DispatchAck {
    /// Whether jobs were queued.
    ok: bool,
    /// One-line summary, shown in the hub.
    message: String,
    /// Secondary line: engine, counts or the reason nothing ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

impl DispatchAck {
    /// Nothing ran, and this is why.
    fn refused(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
            detail: None,
        }
    }

    /// Adds the second line to a refusal.
    fn because(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

/// Drag-hover payload for the webview.
#[derive(Debug, Clone, Serialize)]
struct DragPayload {
    /// True while files are held over the window.
    active: bool,
    /// File names currently being dragged.
    paths: Vec<String>,
}

/// Build the ring for the current selection, or for an explicit set of paths.
///
/// Arguments arrive by value because that is what Tauri's IPC layer builds them
/// from; `State` is a borrow wrapper it hands over for the same reason.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn menu_for(state: State<'_, Orbit>, paths: Vec<String>) -> MenuModel {
    if paths.is_empty() {
        // No argument means "what am I looking at right now", which is what the
        // ring asks on mount — this keeps the initial render correct even if the
        // `orbit://menu` event raced ahead of the webview.
        state.current_menu()
    } else {
        state.menu(state.classify(paths))
    }
}

/// The whole option catalog, keyed by option key.
///
/// The ring fetches this once and renders the options panel from it, so the
/// panel's fields, ranges, labels and `show_if` rules are the registry's, not a
/// second copy living in the webview. Petals only name the *keys* they accept;
/// the widget for each key is described here.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn option_specs(state: State<'_, Orbit>) -> BTreeMap<String, OptionSpec> {
    state
        .registry
        .options()
        .iter()
        .map(|(key, spec)| (key.clone(), spec.clone()))
        .collect()
}

/// The global shortcut as it stands.
///
/// The ring shows it, so the hint under the hub names the combination that is
/// actually registered rather than the one Orbit shipped with.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn hotkey_status(state: State<'_, Orbit>) -> HotkeyStatus {
    state.hotkey()
}

/// Run a petal pick: resolve it into jobs and hand them to the pool.
///
/// The pick is checked against the ring as last rendered, so a stale window can
/// neither invent a petal nor run one against a different selection. `options`
/// are the panel's values, merged over the petal's defaults; keys the petal does
/// not offer are ignored rather than smuggled into a command line. Arguments
/// arrive by value; see `menu_for`.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn dispatch(
    state: State<'_, Orbit>,
    jobs: State<'_, Jobs>,
    petal_kind: String,
    reference: String,
    files: Vec<String>,
    options: Option<BTreeMap<String, serde_json::Value>>,
) -> DispatchAck {
    let Some(kind) = PickKind::from_wire(&petal_kind) else {
        return DispatchAck::refused(format!("Unrecognised pick \"{petal_kind}\""));
    };

    // The stored selection is what the ring was built from, so it is the one a
    // pick can be trusted against; the argument is the fallback for the case
    // where the window is the only thing that still knows.
    let selected = state.selection();
    let sources = if selected.is_empty() {
        selection_from_args(files)
    } else {
        selected
    };
    if sources.is_empty() {
        return DispatchAck::refused("Nothing to work on")
            .because("Drop files onto the ring first.");
    }

    let Some(petal) = state.petal_for(kind, &reference) else {
        return DispatchAck::refused("That petal is no longer on offer")
            .because("Drop the files again to rebuild the ring.");
    };
    let label = match kind {
        PickKind::Conversion => format!("Convert to {}", petal.label),
        _ => petal.label.clone(),
    };

    // The petal's defaults are what a plain drop uses; the panel's values sit
    // on top of them, filtered to the keys the petal actually declares.
    let mut values = petal.defaults.clone();
    if let Some(chosen) = options {
        for (key, value) in chosen {
            if petal.options.iter().any(|offered| offered == &key) {
                values.insert(key, value);
            }
        }
    }

    let pick = Pick {
        kind,
        reference: reference.clone(),
        defaults: values,
    };
    let resolution = match resolve_pick(&state.registry, &state.statuses(), &pick, &sources) {
        Ok(resolution) => resolution,
        Err(error) => return DispatchAck::refused(error.to_string()),
    };

    if resolution.is_empty() {
        let reason = resolution
            .first_reason()
            .unwrap_or("No file was suitable.")
            .to_owned();
        return DispatchAck::refused(format!("{label} is not possible here")).because(reason);
    }

    let queued = resolution.requests.len();
    let total = resolution.total();
    let skipped_reason = resolution.first_reason().map(str::to_owned);
    let engines: BTreeSet<String> = resolution
        .requests
        .iter()
        .map(|request| request.engine.clone())
        .collect();

    let runner = jobs
        .runner
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for request in resolution.requests {
        let id = jobs.next_id.fetch_add(1, Ordering::Relaxed);
        if !runner.submit(Job {
            id,
            label: label.clone(),
            request,
        }) {
            // Only possible after shutdown, which the tray drives.
            return DispatchAck::refused("Orbit is shutting down");
        }
    }
    drop(runner);

    let mut detail = format!(
        "via {}",
        engines.into_iter().collect::<Vec<_>>().join(" + ")
    );
    if let Some(reason) = skipped_reason {
        use std::fmt::Write as _;
        let _ = write!(detail, " — skipped {} of {total}: {reason}", total - queued);
    }
    eprintln!("orbit: {label} — {queued} job(s) queued {detail}");

    DispatchAck {
        ok: true,
        message: format!(
            "{label} — {queued} file{}",
            if queued == 1 { "" } else { "s" }
        ),
        detail: Some(detail),
    }
}

/// Put the ring under the cursor and show it.
fn show_ring<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(RING) else {
        return;
    };

    if let Some(orbit) = app.try_state::<Orbit>() {
        let model = orbit.current_menu();
        eprintln!(
            "orbit: opening ring — {} file(s), {} categories, {} candidates",
            model.files.len(),
            model.categories.len(),
            model.candidates
        );
        let _ = window.emit(MENU_EVENT, model);
    }

    let _ = position_at_cursor(&window);
    let _ = window.show();
    let _ = window.set_focus();
}

/// File paths given on the command line: the "Open with Orbit" / "Send to"
/// route, and the easy way to poke at the ring during development.
fn cli_paths() -> Vec<PathBuf> {
    selection_from_args(std::env::args_os().skip(1))
}

/// Turn command-line words into a selection, ignoring flags and anything that
/// does not exist on disk.
///
/// Used both for our own arguments and for the ones a second launch hands to the
/// already-running instance, so a compositor binding carrying a path works.
fn selection_from_args<I, S>(args: I) -> Vec<PathBuf>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    args.into_iter()
        .map(|arg| PathBuf::from(arg.as_ref()))
        .filter(|path| !path.to_string_lossy().starts_with('-'))
        .filter(|path| path.exists())
        .collect()
}

/// A shortcut named on the command line.
enum HotkeyArg {
    /// No `--hotkey` flag at all: keep what the settings say.
    Absent,
    /// `--hotkey=off`.
    Off,
    /// `--hotkey=<accelerator>`.
    Custom(String),
}

/// Read `--hotkey=<accelerator>`, or `--hotkey=off` to turn it off.
///
/// Reaching this is a deliberate act, so the value is kept as well as applied —
/// it is how a script or an installer configures the trigger.
fn hotkey_from_args<I, S>(args: I) -> HotkeyArg
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    for arg in args {
        let text = arg.as_ref().to_string_lossy().into_owned();
        let Some(value) = text.strip_prefix("--hotkey=") else {
            continue;
        };
        return if value.eq_ignore_ascii_case("off") {
            HotkeyArg::Off
        } else {
            HotkeyArg::Custom(value.to_owned())
        };
    }
    HotkeyArg::Absent
}

/// Whether this session is Wayland, where a client cannot own a global hotkey.
fn on_wayland() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// What the tray and the ring say when the compositor owns global input.
const WAYLAND_NOTE: &str =
    "Wayland does not let an app own a global shortcut; bind Orbit in your compositor instead";

/// Register `accelerator`, replacing whatever was registered before.
///
/// Always clears first: registering a second combination without unregistering
/// the first would leave both live, and the tray would then be showing a
/// shortcut the user thought they had replaced.
fn apply_hotkey<R: Runtime>(app: &AppHandle<R>, accelerator: Option<&str>) -> HotkeyStatus {
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();

    let Some(accelerator) = accelerator else {
        eprintln!("orbit: global shortcut is off — use the tray icon or \"Open with Orbit\"");
        return HotkeyStatus::off();
    };

    let label = settings::label_for(accelerator);
    let status = match shortcuts.register(accelerator) {
        Ok(()) => HotkeyStatus {
            accelerator: Some(accelerator.to_owned()),
            label,
            registered: true,
            note: None,
        },
        Err(error) => HotkeyStatus {
            accelerator: Some(accelerator.to_owned()),
            label,
            registered: false,
            note: Some(format!("{accelerator} could not be registered ({error})")),
        },
    };

    if status.registered && on_wayland() {
        // The grab is accepted at the X protocol level and then never fires,
        // because the compositor owns input. Reporting success would be a lie,
        // so the note says what to do instead.
        eprintln!("orbit: {accelerator} cannot be delivered — {WAYLAND_NOTE}");
        return HotkeyStatus {
            registered: false,
            note: Some(WAYLAND_NOTE.to_owned()),
            ..status
        };
    }

    match &status.note {
        Some(note) => eprintln!("orbit: {note}"),
        None => eprintln!("orbit: {} opens the ring", status.label),
    }
    status
}

/// Change the shortcut: persist it, apply it, and tell whatever shows it.
fn set_hotkey<R: Runtime>(app: &AppHandle<R>, choice: Option<&str>) {
    let Some(orbit) = app.try_state::<Orbit>() else {
        return;
    };
    if let Ok(mut settings) = orbit.settings.lock() {
        settings.hotkey = choice.map(str::to_owned);
        if let Err(error) = settings.save(&orbit.settings_path) {
            eprintln!("orbit: could not save the shortcut ({error})");
        }
    }

    let status = apply_hotkey(app, choice);
    if let Ok(mut current) = orbit.hotkey.lock() {
        *current = status.clone();
    }
    if let Ok(menu) = tray_menu(app, &status) {
        if let Some(tray) = app.tray_by_id("orbit") {
            let _ = tray.set_menu(Some(menu));
        }
    }
    let _ = app.emit_to(RING, HOTKEY_EVENT, status);
}

/// The tray menu, rebuilt whenever the shortcut changes.
///
/// The running combination gets a tick, because a tray menu has no radio
/// buttons and the current choice otherwise has nowhere to be seen.
fn tray_menu<R: Runtime>(app: &AppHandle<R>, hotkey: &HotkeyStatus) -> tauri::Result<Menu<R>> {
    let open = MenuItem::with_id(app, "open", "Open ring", true, None::<&str>)?;
    let reload = MenuItem::with_id(app, "reload", "Reload engines", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Orbit", true, None::<&str>)?;

    let shortcut = Submenu::with_id(app, "shortcut", "Shortcut", true)?;
    for (accelerator, label) in settings::PRESETS {
        let chosen = hotkey.accelerator.as_deref() == Some(*accelerator);
        let text = format!("{} {label}", if chosen { "✓" } else { " " });
        shortcut.append(&MenuItem::with_id(
            app,
            format!("hotkey:{accelerator}"),
            text,
            true,
            None::<&str>,
        )?)?;
    }
    shortcut.append(&MenuItem::with_id(
        app,
        "hotkey:off",
        format!(
            "{} Off",
            if hotkey.accelerator.is_none() {
                "✓"
            } else {
                " "
            }
        ),
        true,
        None::<&str>,
    )?)?;

    Menu::with_items(app, &[&open, &shortcut, &reload, &quit])
}

/// Build the tray icon. Left click opens the ring; the menu has the rest.
fn setup_tray<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let status = app
        .try_state::<Orbit>()
        .map_or_else(HotkeyStatus::off, |orbit| orbit.hotkey());
    let menu = tray_menu(app, &status)?;

    let mut builder = TrayIconBuilder::with_id("orbit")
        .tooltip("Orbit — radial converter")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref().to_owned();
            if let Some(accelerator) = id.strip_prefix("hotkey:") {
                let choice = (accelerator != "off").then_some(accelerator);
                set_hotkey(app, choice);
                return;
            }
            match id.as_str() {
                "open" => show_ring(app),
                "reload" => {
                    if let Some(orbit) = app.try_state::<Orbit>() {
                        orbit.refresh_engines();
                        let model = orbit.current_menu();
                        if let Some(window) = app.get_webview_window(RING) {
                            let _ = window.emit(MENU_EVENT, model);
                        }
                    }
                }
                "quit" => app.exit(0),
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_ring(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;
    Ok(())
}

/// Centre the window on the pointer, which is what makes this feel radial.
#[allow(clippy::cast_possible_truncation)] // screen coordinates are tiny next to i32
fn position_at_cursor<R: Runtime>(window: &WebviewWindow<R>) -> tauri::Result<()> {
    let cursor = window.cursor_position()?;
    let x = (cursor.x - RING_SIZE / 2.0).round() as i32;
    let y = (cursor.y - RING_SIZE / 2.0).round() as i32;
    window.set_position(PhysicalPosition::new(x, y))
}

fn main() {
    tauri::Builder::default()
        // Registered first: it decides whether this process is the owner of the
        // app or a request to an instance that already exists.
        //
        // This is what makes a compositor binding viable. Wayland forbids global
        // hotkeys, but it does not forbid *launching* Orbit, so `bind = ... ,
        // exec, orbit-app` becomes a working trigger — as long as the second
        // launch hands its files over and exits instead of adding another tray
        // icon and another overlay.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            eprintln!("orbit: already running — handing the request to the live instance");
            if let Some(orbit) = app.try_state::<Orbit>() {
                // `argv[0]` is the executable itself, not a file the user picked.
                let paths = selection_from_args(argv.into_iter().skip(1));
                if !paths.is_empty() {
                    eprintln!("orbit: compositor binding carried {} path(s)", paths.len());

                    // Windows' "Send to" can invoke Orbit once per selected file
                    // rather than once for the lot. While the ring is still open
                    // those arrivals are one selection, not the last of many — so
                    // they accumulate. A request that arrives while the ring is
                    // closed is a new selection, and replaces the old one.
                    let open = app
                        .get_webview_window(RING)
                        .and_then(|window| window.is_visible().ok())
                        .unwrap_or(false);

                    if let Ok(mut last) = orbit.last_drop.lock() {
                        if open {
                            for path in paths {
                                if !last.contains(&path) {
                                    last.push(path);
                                }
                            }
                        } else {
                            *last = paths;
                        }
                    }
                }
            }
            show_ring(app);
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        show_ring(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            let orbit = Orbit::new(app.handle())?;

            // Files handed to us on the command line are the "Open with Orbit"
            // / "Send to" route: treat them as the current selection.
            let from_cli = cli_paths();
            let opened_with_files = !from_cli.is_empty();
            if opened_with_files {
                if let Ok(mut last) = orbit.last_drop.lock() {
                    *last = from_cli;
                }
            }

            // The shortcut is registered from the settings as loaded, before the
            // tray is built, because the tray menu shows which one is running.
            let accelerator = orbit
                .settings
                .lock()
                .ok()
                .and_then(|settings| settings.hotkey.clone());
            let status = apply_hotkey(app.handle(), accelerator.as_deref());
            if let Ok(mut current) = orbit.hotkey.lock() {
                *current = status;
            }

            app.manage(orbit);

            // The pool reports straight to the ring, from whichever worker
            // thread the job happens to be running on.
            let handle = app.handle().clone();
            app.manage(Jobs {
                runner: Mutex::new(Runner::new(worker_count(), move |event| {
                    announce(&event);
                    let _ = handle.emit_to(RING, JOB_EVENT, event);
                })),
                next_id: AtomicU64::new(1),
            });

            setup_tray(app.handle())?;

            // A shortcut named on the command line is a one-off override that is
            // remembered, so a script (or an installer) can set the trigger up.
            match hotkey_from_args(std::env::args_os().skip(1)) {
                HotkeyArg::Absent => {}
                HotkeyArg::Off => set_hotkey(app.handle(), None),
                HotkeyArg::Custom(accelerator) => {
                    set_hotkey(app.handle(), Some(&accelerator));
                }
            }

            // macOS: no dock icon, no space in the app switcher.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            if opened_with_files {
                show_ring(app.handle());
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            menu_for,
            dispatch,
            option_specs,
            hotkey_status
        ])
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                // A tray app hides; quitting is explicit, from the tray menu.
                api.prevent_close();
                let _ = window.hide();
            }
            WindowEvent::DragDrop(drag) => handle_drag(window, drag),
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("Orbit failed to start");
}

/// Translate Tauri's drag-drop events into ring updates and drag visuals.
fn handle_drag<R: Runtime>(window: &tauri::Window<R>, event: &tauri::DragDropEvent) {
    let app = window.app_handle();
    match event {
        tauri::DragDropEvent::Enter { paths, .. } => {
            let _ = window.emit(
                DRAG_EVENT,
                DragPayload {
                    active: true,
                    paths: names(paths),
                },
            );
        }
        tauri::DragDropEvent::Drop { paths, .. } => {
            let _ = window.emit(
                DRAG_EVENT,
                DragPayload {
                    active: false,
                    paths: Vec::new(),
                },
            );
            if paths.is_empty() {
                return;
            }
            eprintln!("orbit: dropped {} path(s)", paths.len());
            if let Some(orbit) = app.try_state::<Orbit>() {
                if let Ok(mut last) = orbit.last_drop.lock() {
                    (*last).clone_from(paths);
                }
                let model = orbit.menu(orbit.classify(paths));
                eprintln!(
                    "orbit: ring rebuilt — {} categories, {} candidates",
                    model.categories.len(),
                    model.candidates
                );
                let _ = window.emit(MENU_EVENT, model);
            }
            let _ = window.show();
            let _ = window.set_focus();
        }
        tauri::DragDropEvent::Leave => {
            let _ = window.emit(
                DRAG_EVENT,
                DragPayload {
                    active: false,
                    paths: Vec::new(),
                },
            );
        }
        _ => {}
    }
}

/// File names for the drag badge, without leaking full paths to the UI.
fn names(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|path| {
            path.file_name().map_or_else(
                || path.to_string_lossy().into_owned(),
                |name| name.to_string_lossy().into_owned(),
            )
        })
        .collect()
}
