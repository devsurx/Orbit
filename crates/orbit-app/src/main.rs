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
mod trigger;
mod watch;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use orbit_jobs::{
    HistoryEntry, HistoryStore, Job, JobEvent, JobRequest, JobState, Pick, PickKind, Runner,
    resolve_pick,
};
use orbit_registry::{
    EngineStatus, MenuModel, MenuRequest, OptionSpec, Petal, PetalKind, Registry, SourceFile,
};
use serde::Serialize;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, Runtime, State, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::settings::{HotkeyStatus, Settings, WatchFolder};
use crate::trigger::TriggerStatus;

/// Ring window label; must match `tauri.conf.json`.
const RING: &str = "ring";
/// Event carrying the current ring contents to the webview.
const MENU_EVENT: &str = "orbit://menu";
/// Event carrying drag-hover state, so the ring can light up before a drop.
const DRAG_EVENT: &str = "orbit://drag";
/// Event carrying job progress, completion and failure.
const JOB_EVENT: &str = "orbit://job";
/// Event carrying a changed global shortcut, so the ring can update its hint.
const HOTKEY_EVENT: &str = "orbit://hotkey";
/// Event telling every window that the settings document changed.
const SETTINGS_EVENT: &str = "orbit://settings";
/// Event telling the settings window that the history changed.
const HISTORY_EVENT: &str = "orbit://history";
/// Label of the settings window, created on first use.
const SETTINGS_WINDOW: &str = "settings";

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
    /// The local record of what was written.
    history: HistoryStore,
    /// Paths Orbit published recently, so a watch folder can ignore its own
    /// output when that output lands beside the source.
    written: Arc<Mutex<BTreeSet<PathBuf>>>,
    /// Set by a drop, read by the trigger's release timer.
    drag_seen: AtomicBool,
    /// Whether the ring on screen was opened by Shift-drag.
    triggered: AtomicBool,
    /// Bumped every time the trigger opens the ring, so a stale timer cannot
    /// hide a newer drag.
    trigger_generation: AtomicU64,
    /// Whether the input listener is running, and why not when it is not.
    trigger: Mutex<TriggerStatus>,
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
            history: HistoryStore::new(HistoryStore::default_path()),
            written: Arc::new(Mutex::new(BTreeSet::new())),
            drag_seen: AtomicBool::new(false),
            triggered: AtomicBool::new(false),
            trigger_generation: AtomicU64::new(0),
            trigger: Mutex::new(TriggerStatus::unavailable(
                true,
                "The listener starts with the app.",
            )),
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
    /// What each queued job is, so a terminal event can be written to history
    /// with the files it came from.
    meta: Arc<Mutex<BTreeMap<u64, JobMeta>>>,
}

/// The parts of a submission that the job event does not carry.
#[derive(Debug, Clone)]
struct JobMeta {
    label: String,
    action: String,
    sources: Vec<PathBuf>,
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

/// Queue a resolved batch and remember what it was.
///
/// Used by the ring and by watch folders, so both produce the same events and
/// the same history entries.
fn enqueue(
    jobs: &Jobs,
    label: &str,
    action: &str,
    requests: Vec<JobRequest>,
) -> Result<usize, String> {
    if requests.is_empty() {
        return Ok(0);
    }
    let runner = jobs
        .runner
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut queued = 0;
    for request in requests {
        let id = jobs.next_id.fetch_add(1, Ordering::Relaxed);
        let meta = JobMeta {
            label: label.to_owned(),
            action: action.to_owned(),
            sources: request.sources.clone(),
        };
        if !runner.submit(Job {
            id,
            label: label.to_owned(),
            request,
        }) {
            return Err("Orbit is shutting down".to_owned());
        }
        if let Ok(mut map) = jobs.meta.lock() {
            map.insert(id, meta);
        }
        queued += 1;
    }
    Ok(queued)
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
        JobState::DoneSeries { paths } => {
            for path in paths {
                eprintln!("orbit: wrote {}", path.display());
            }
        }
        JobState::Failed { message } => eprintln!("orbit: failed {} — {message}", event.label),
        JobState::Cancelled => eprintln!("orbit: cancelled {}", event.label),
        JobState::Queued | JobState::Running { .. } => {}
    }
}

/// Write a terminal job to the local history and remember what it produced.
fn record_event(
    history: &HistoryStore,
    meta: &Mutex<BTreeMap<u64, JobMeta>>,
    written: &Mutex<BTreeSet<PathBuf>>,
    event: &JobEvent,
) {
    let (outputs, ok, message) = match &event.state {
        JobState::Done { path } => (vec![path.clone()], true, None),
        JobState::DoneSeries { paths } => (paths.clone(), true, None),
        JobState::Failed { message } => (Vec::new(), false, Some(message.clone())),
        JobState::Cancelled => (Vec::new(), false, Some("cancelled".to_owned())),
        JobState::Queued | JobState::Running { .. } => return,
    };

    // Recorded before the history lookup: the watch loops must ignore this
    // output even if the job's metadata has already gone.
    if let Ok(mut set) = written.lock() {
        for path in &outputs {
            set.insert(path.clone());
        }
        // The set only exists to break the watch loop; it is not a record.
        if set.len() > 4096 {
            set.clear();
        }
    }

    let Some(meta) = meta.lock().ok().and_then(|mut map| map.remove(&event.id)) else {
        return;
    };
    let entry = HistoryEntry::now(meta.label, meta.action, meta.sources, outputs, ok, message);
    if let Err(error) = history.append(&entry) {
        eprintln!("orbit: could not record history ({error})");
    }
}

/// Rebuild the tray menu so "Recent files" reflects what just finished.
fn refresh_tray<R: Runtime>(app: &AppHandle<R>) {
    let status = app
        .try_state::<Orbit>()
        .map_or_else(HotkeyStatus::off, |orbit| orbit.hotkey());
    if let Ok(menu) = tray_menu(app, &status) {
        if let Some(tray) = app.tray_by_id("orbit") {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

/// Show a file in the platform's file manager.
fn reveal(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .status()
            .is_ok()
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .status()
            .is_ok()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let folder = path.parent().unwrap_or(path);
        std::process::Command::new("xdg-open")
            .arg(folder)
            .spawn()
            .is_ok()
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = path;
        false
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

    // A folder chosen in settings wins over "beside the source"; a request that
    // already carries one (the CLI does) keeps it.
    let output_dir = state
        .settings
        .lock()
        .ok()
        .and_then(|settings| settings.output_dir.clone());
    let mut requests = resolution.requests;
    if let Some(dir) = output_dir {
        for request in &mut requests {
            if request.output_dir.is_none() {
                request.output_dir = Some(dir.clone());
            }
        }
    }

    if let Err(error) = enqueue(&jobs, &label, &reference, requests) {
        // Only possible after shutdown, which the tray drives.
        return DispatchAck::refused(error);
    }

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

/// Put the ring under the cursor and show it, taking keyboard focus.
fn show_ring<R: Runtime>(app: &AppHandle<R>) {
    if let Some(orbit) = app.try_state::<Orbit>() {
        orbit.triggered.store(false, Ordering::Relaxed);
        // Invalidate any release timer the Shift-drag trigger left pending.
        orbit.trigger_generation.fetch_add(1, Ordering::Relaxed);
    }
    show_ring_with(app, true);
}

/// The common part of every way the ring appears.
///
/// `focus` is false when the window is summoned in the middle of an operating
/// system drag: taking focus then would cancel the drag before the files can be
/// dropped. The window is made focusable again the moment a drop lands.
fn show_ring_with<R: Runtime>(app: &AppHandle<R>, focus: bool) {
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
    if focus {
        let _ = window.set_focusable(true);
    }
    let _ = window.show();
    if focus {
        let _ = window.set_focus();
    }
}

/// Shift-drag crossed the threshold: show the ring without stealing the drag.
pub fn show_from_trigger<R: Runtime>(app: &AppHandle<R>) {
    let Some(orbit) = app.try_state::<Orbit>() else {
        return;
    };
    if !orbit.settings.lock().is_ok_and(|settings| settings.trigger) {
        return;
    }
    let Some(window) = app.get_webview_window(RING) else {
        return;
    };
    // A drag that arrives while the ring is already open is just passing over
    // it; do not re-centre the window under the pointer.
    if window.is_visible().unwrap_or(false) {
        return;
    }

    orbit.drag_seen.store(false, Ordering::Relaxed);
    orbit.triggered.store(true, Ordering::Relaxed);
    orbit.trigger_generation.fetch_add(1, Ordering::Relaxed);
    // Windows needs the window itself to be non-activating before it is shown;
    // `show` then uses SW_SHOWNOACTIVATE. macOS and X11 only need the focus flag.
    let _ = window.set_focusable(false);
    show_ring_with(app, false);
}

/// The mouse button came up: if no file was dropped, get out of the way.
pub fn release_from_trigger<R: Runtime>(app: &AppHandle<R>) {
    let Some(orbit) = app.try_state::<Orbit>() else {
        return;
    };
    if !orbit.triggered.swap(false, Ordering::Relaxed) {
        return;
    }
    let generation = orbit.trigger_generation.load(Ordering::Relaxed);

    let handle = app.clone();
    // A drop and the button release arrive in either order; wait past both.
    let spawned = std::thread::Builder::new()
        .name("orbit-trigger-release".to_owned())
        .spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(450));
            let Some(orbit) = handle.try_state::<Orbit>() else {
                return;
            };
            if orbit.trigger_generation.load(Ordering::Relaxed) != generation {
                return;
            }
            if orbit.drag_seen.load(Ordering::Relaxed) {
                return;
            }
            if let Some(window) = handle.get_webview_window(RING) {
                if window.is_visible().unwrap_or(false) {
                    let _ = window.set_focusable(true);
                    let _ = window.hide();
                }
            }
        });
    if let Err(error) = spawned {
        eprintln!("orbit: could not schedule the ring to close ({error})");
    }
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
    let _ = app.emit_to(SETTINGS_WINDOW, SETTINGS_EVENT, ());
}

/// The tray menu, rebuilt whenever the shortcut changes.
///
/// The running combination gets a tick, because a tray menu has no radio
/// buttons and the current choice otherwise has nowhere to be seen.
fn tray_menu<R: Runtime>(app: &AppHandle<R>, hotkey: &HotkeyStatus) -> tauri::Result<Menu<R>> {
    let open = MenuItem::with_id(app, "open", "Open ring", true, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let reload = MenuItem::with_id(app, "reload", "Reload engines", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Orbit", true, None::<&str>)?;

    let recent = Submenu::with_id(app, "recent", "Recent files", true)?;
    let entries = app
        .try_state::<Orbit>()
        .map_or_else(Vec::new, |orbit| orbit.history.load(8));
    let mut listed = 0;
    for entry in &entries {
        for path in entry.outputs.iter().take(1) {
            recent.append(&MenuItem::with_id(
                app,
                format!("recent:{}", path.display()),
                truncate(&entry.menu_label(), 52),
                true,
                None::<&str>,
            )?)?;
            listed += 1;
        }
    }
    if listed == 0 {
        recent.append(&MenuItem::with_id(
            app,
            "recent:none",
            "Nothing yet",
            false,
            None::<&str>,
        )?)?;
    } else {
        recent.append(&PredefinedMenuItem::separator(app)?)?;
        recent.append(&MenuItem::with_id(
            app,
            "history:clear",
            "Clear history",
            true,
            None::<&str>,
        )?)?;
    }

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

    Menu::with_items(
        app,
        &[&open, &recent, &settings_item, &shortcut, &reload, &quit],
    )
}

/// Cut a menu label down to something a tray can show.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(max.saturating_sub(1)).collect();
    short.push('…');
    short
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
            if let Some(path) = id.strip_prefix("recent:") {
                if path != "none" {
                    reveal(Path::new(path));
                }
                return;
            }
            match id.as_str() {
                "open" => show_ring(app),
                "settings" => open_settings_window(app, None),
                "history:clear" => {
                    if let Some(orbit) = app.try_state::<Orbit>() {
                        if let Err(error) = orbit.history.clear() {
                            eprintln!("orbit: could not clear history ({error})");
                        }
                    }
                    refresh_tray(app);
                    let _ = app.emit_to(SETTINGS_WINDOW, HISTORY_EVENT, ());
                }
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

/// One shortcut the settings window offers.
#[derive(Debug, Serialize)]
struct HotkeyPreset {
    accelerator: String,
    label: String,
}

/// Everything the settings window renders, in one document.
///
/// A flat view model: the bools are independent switches a form binds to, not
/// one state machine.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Serialize)]
struct SettingsView {
    hotkey: Option<String>,
    hotkey_label: String,
    hotkey_registered: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    hotkey_note: Option<String>,
    hotkey_presets: Vec<HotkeyPreset>,
    trigger: bool,
    trigger_listening: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    trigger_note: Option<String>,
    output_dir: Option<String>,
    onboarded: bool,
    watch: Vec<WatchFolder>,
    history_path: String,
    settings_path: String,
    version: String,
    wayland: bool,
}

/// Snapshot the settings and the state of both triggers.
fn settings_view(state: &Orbit) -> SettingsView {
    let settings = state
        .settings
        .lock()
        .map(|settings| settings.clone())
        .unwrap_or_default();
    let hotkey = state.hotkey();
    let trigger = state.trigger.lock().map_or_else(
        |_| TriggerStatus::unavailable(settings.trigger, "Status unavailable"),
        |status| status.clone(),
    );

    SettingsView {
        hotkey: settings.hotkey,
        hotkey_label: hotkey.label,
        hotkey_registered: hotkey.registered,
        hotkey_note: hotkey.note,
        hotkey_presets: settings::PRESETS
            .iter()
            .map(|(accelerator, label)| HotkeyPreset {
                accelerator: (*accelerator).to_owned(),
                label: (*label).to_owned(),
            })
            .collect(),
        trigger: settings.trigger,
        trigger_listening: trigger.listening,
        trigger_note: trigger.note,
        output_dir: settings.output_dir.map(|path| path.display().to_string()),
        onboarded: settings.onboarded,
        watch: settings.watch,
        history_path: state.history.path().display().to_string(),
        settings_path: state.settings_path.display().to_string(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        wayland: on_wayland(),
    }
}

/// Save the current document and tell every window.
fn persist_settings<R: Runtime>(app: &AppHandle<R>, state: &Orbit) -> bool {
    let snapshot = state.settings.lock().ok().map(|settings| settings.clone());
    let Some(settings) = snapshot else {
        return false;
    };
    if let Err(error) = settings.save(&state.settings_path) {
        eprintln!("orbit: could not save settings ({error})");
        return false;
    }
    let _ = app.emit_to(RING, SETTINGS_EVENT, ());
    let _ = app.emit_to(SETTINGS_WINDOW, SETTINGS_EVENT, ());
    refresh_tray(app);
    true
}

/// Show the settings window, creating it the first time.
fn open_settings_window<R: Runtime>(app: &AppHandle<R>, section: Option<&str>) {
    if let Some(window) = app.get_webview_window(SETTINGS_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.emit(SETTINGS_EVENT, ());
        if let Some(section) = section {
            let _ = window.emit("orbit://section", section);
        }
        return;
    }

    // A hash route, not a query: the production asset resolver looks up
    // `index.html` and the fragment stays in the webview.
    let hash = section.map_or_else(
        || "#settings".to_owned(),
        |section| format!("#settings/{section}"),
    );
    let url = WebviewUrl::App(format!("index.html{hash}").into());
    match WebviewWindowBuilder::new(app, SETTINGS_WINDOW, url)
        .title("Orbit settings")
        .inner_size(680.0, 800.0)
        .min_inner_size(540.0, 560.0)
        // Avoid a white flash before the first paint on an opaque window.
        .background_color(tauri::window::Color(11, 11, 18, 255))
        .center()
        .build()
    {
        Ok(window) => {
            let _ = window.set_focus();
        }
        Err(error) => eprintln!("orbit: could not open settings ({error})"),
    }
}

/// The settings document, plus what is actually running.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn get_settings(state: State<'_, Orbit>) -> SettingsView {
    if std::env::var_os("ORBIT_TRACE").is_some() {
        eprintln!("orbit: settings window asked for its document");
    }
    settings_view(&state)
}

/// Just the Shift-drag state, for the ring's idle hint.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn trigger_status(state: State<'_, Orbit>) -> TriggerStatus {
    state.trigger.lock().map_or_else(
        |_| TriggerStatus::unavailable(false, "Status unavailable"),
        |status| status.clone(),
    )
}

/// Open (or focus) the settings window.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn open_settings(app: AppHandle, section: Option<String>) {
    open_settings_window(&app, section.as_deref());
}

/// Turn Shift-drag on or off immediately.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn set_trigger(app: AppHandle, state: State<'_, Orbit>, enabled: bool) -> SettingsView {
    if let Ok(mut settings) = state.settings.lock() {
        settings.trigger = enabled;
    }
    if let Ok(mut status) = state.trigger.lock() {
        status.enabled = enabled;
    }
    persist_settings(&app, &state);
    settings_view(&state)
}

/// Choose the folder every result goes to; an empty string means "beside the
/// source".
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn set_output_dir(
    app: AppHandle,
    state: State<'_, Orbit>,
    dir: Option<String>,
) -> Result<SettingsView, String> {
    let path = dir
        .map(|dir| dir.trim().to_owned())
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from);
    let path = match path {
        Some(path) if path.is_dir() => Some(
            std::fs::canonicalize(&path).map_err(|error| format!("{}: {error}", path.display()))?,
        ),
        Some(path) => return Err(format!("{} is not a folder", path.display())),
        None => None,
    };
    if let Ok(mut settings) = state.settings.lock() {
        settings.output_dir = path;
    }
    persist_settings(&app, &state);
    Ok(settings_view(&state))
}

/// Dismiss the first-run welcome.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn mark_onboarded(app: AppHandle, state: State<'_, Orbit>) -> SettingsView {
    if let Ok(mut settings) = state.settings.lock() {
        settings.onboarded = true;
    }
    persist_settings(&app, &state);
    settings_view(&state)
}

/// Change the global shortcut from the settings window.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn choose_hotkey(app: AppHandle, accelerator: Option<String>) -> HotkeyStatus {
    set_hotkey(&app, accelerator.as_deref());
    let _ = app.emit_to(SETTINGS_WINDOW, SETTINGS_EVENT, ());
    app.try_state::<Orbit>()
        .map_or_else(HotkeyStatus::off, |orbit| orbit.hotkey())
}

/// Recent jobs, newest first.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn history(state: State<'_, Orbit>, limit: Option<usize>) -> Vec<HistoryEntry> {
    state.history.load(limit.unwrap_or(50).clamp(1, 1000))
}

/// Forget every recorded job.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn clear_history(app: AppHandle, state: State<'_, Orbit>) -> bool {
    match state.history.clear() {
        Ok(()) => {
            refresh_tray(&app);
            let _ = app.emit_to(SETTINGS_WINDOW, HISTORY_EVENT, ());
            true
        }
        Err(error) => {
            eprintln!("orbit: could not clear history ({error})");
            false
        }
    }
}

/// Show one result in the file manager.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn reveal_path(path: String) -> bool {
    reveal(Path::new(&path))
}

/// One option in the watch-folder editor.
#[derive(Debug, Serialize)]
struct WatchChoice {
    kind: String,
    reference: String,
    label: String,
    hint: String,
}

/// Everything a watch folder can be pointed at.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn watch_choices(state: State<'_, Orbit>) -> Vec<WatchChoice> {
    let mut choices = Vec::new();
    for action in state.registry.actions() {
        choices.push(WatchChoice {
            kind: "action".to_owned(),
            reference: action.id.clone(),
            label: action.label.clone(),
            hint: "Action".to_owned(),
        });
    }
    for chain in state.registry.chains() {
        choices.push(WatchChoice {
            kind: "preset".to_owned(),
            reference: chain.id.clone(),
            label: chain.label.clone(),
            hint: "Preset".to_owned(),
        });
    }
    for format in state.registry.formats() {
        if format.can.write {
            choices.push(WatchChoice {
                kind: "conversion".to_owned(),
                reference: format.id.clone(),
                label: format!("→ {}", format.id.to_uppercase()),
                hint: format.kind.to_string(),
            });
        }
    }
    choices
}

/// Add or retarget the rule for a folder.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn add_watch(
    app: AppHandle,
    state: State<'_, Orbit>,
    path: String,
    kind: String,
    reference: String,
) -> Result<SettingsView, String> {
    let dir = PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(format!("{} is not a folder", dir.display()));
    }
    // Absolute and symlink-free, so the scanner and the output set agree on
    // what "this file" means.
    let dir = std::fs::canonicalize(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let kind = kind.to_ascii_lowercase();
    let known = match kind.as_str() {
        "conversion" => state
            .registry
            .format(&reference)
            .is_some_and(|format| format.can.write),
        "action" => state.registry.action(&reference).is_some(),
        "preset" => state.registry.chain(&reference).is_some(),
        _ => false,
    };
    if !known {
        return Err(format!("{reference} is not a {kind} Orbit knows"));
    }

    if let Ok(mut settings) = state.settings.lock() {
        // One rule per folder: retargeting is clearer than quietly running two
        // conversions on every file that lands there.
        if let Some(existing) = settings.watch.iter_mut().find(|rule| rule.path == dir) {
            existing.kind = kind;
            existing.reference = reference;
            existing.enabled = true;
        } else {
            settings.watch.push(WatchFolder {
                path: dir,
                kind,
                reference,
                options: BTreeMap::new(),
                enabled: true,
            });
        }
    }
    if !persist_settings(&app, &state) {
        return Err("the settings file could not be written".to_owned());
    }
    Ok(settings_view(&state))
}

/// Remove a folder from the watch list.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn remove_watch(app: AppHandle, state: State<'_, Orbit>, index: usize) -> SettingsView {
    if let Ok(mut settings) = state.settings.lock() {
        if index < settings.watch.len() {
            settings.watch.remove(index);
        }
    }
    persist_settings(&app, &state);
    settings_view(&state)
}

/// Pause or resume one rule without forgetting it.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn set_watch_enabled(
    app: AppHandle,
    state: State<'_, Orbit>,
    index: usize,
    enabled: bool,
) -> SettingsView {
    if let Ok(mut settings) = state.settings.lock() {
        if let Some(rule) = settings.watch.get_mut(index) {
            rule.enabled = enabled;
        }
    }
    persist_settings(&app, &state);
    settings_view(&state)
}

/// Centre the window on the pointer, which is what makes this feel radial.
///
/// The window's *physical* size is what matters: at 200 % scaling (every Retina
/// display, and many Windows laptops) it is twice the CSS size, and subtracting
/// half of the logical size would put the cursor in the top-left quadrant. The
/// position is then clamped to the monitor under the cursor, because a ring
/// that is half off-screen near an edge is not radial.
#[allow(clippy::cast_possible_truncation)] // screen coordinates are tiny next to i32
fn position_at_cursor<R: Runtime>(window: &WebviewWindow<R>) -> tauri::Result<()> {
    let cursor = window.cursor_position()?;
    let physical = window.inner_size()?;
    let mut x = cursor.x - f64::from(physical.width) / 2.0;
    let mut y = cursor.y - f64::from(physical.height) / 2.0;

    if let Ok(Some(monitor)) = window.monitor_from_point(cursor.x, cursor.y) {
        let origin = monitor.position();
        let size = monitor.size();
        let left = f64::from(origin.x);
        let top = f64::from(origin.y);
        let right = left + f64::from(size.width);
        let bottom = top + f64::from(size.height);
        x = x.clamp(left, (right - f64::from(physical.width)).max(left));
        y = y.clamp(top, (bottom - f64::from(physical.height)).max(top));
    }

    window.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))
}

#[allow(clippy::too_many_lines)] // the builder: plugins, commands, setup, run loop
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
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            eprintln!("orbit: already running — handing the request to the live instance");
            let requested_hotkey = hotkey_from_args(argv.iter().skip(1));
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

            // A second launch can carry `--hotkey=…` as well as files; a script
            // that changes the shortcut should not have to quit the app first.
            match requested_hotkey {
                HotkeyArg::Absent => {}
                HotkeyArg::Off => set_hotkey(app, None),
                HotkeyArg::Custom(accelerator) => set_hotkey(app, Some(&accelerator)),
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
            let written = orbit.written.clone();
            let history = orbit.history.clone();
            let configured_trigger = orbit
                .settings
                .lock()
                .map_or(true, |settings| settings.trigger);

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
            // thread the job happens to be running on. Each terminal event is
            // also written to the local history and added to the set of files
            // watch folders must ignore.
            let meta = Arc::new(Mutex::new(BTreeMap::new()));
            let jobs_meta = Arc::clone(&meta);
            let jobs_written = Arc::clone(&written);
            let jobs_history = history.clone();
            let handle = app.handle().clone();
            app.manage(Jobs {
                runner: Mutex::new(Runner::new(worker_count(), move |event| {
                    announce(&event);
                    let terminal = matches!(
                        &event.state,
                        JobState::Done { .. }
                            | JobState::DoneSeries { .. }
                            | JobState::Failed { .. }
                            | JobState::Cancelled
                    );
                    record_event(&jobs_history, &jobs_meta, &jobs_written, &event);
                    let _ = handle.emit_to(RING, JOB_EVENT, event);
                    if terminal {
                        let _ = handle.emit_to(SETTINGS_WINDOW, HISTORY_EVENT, ());
                        // Menu construction is main-thread work on macOS; worker
                        // threads only ever ask for it.
                        let tray_handle = handle.clone();
                        let _ = handle.run_on_main_thread(move || {
                            refresh_tray(&tray_handle);
                        });
                    }
                })),
                next_id: AtomicU64::new(1),
                meta,
            });

            setup_tray(app.handle())?;

            // The input listener starts now and checks the setting on every
            // event, so the toggle needs no restart.
            let mut trigger_status = trigger::start(app.handle());
            trigger_status.enabled = configured_trigger;
            if let Some(orbit) = app.try_state::<Orbit>() {
                if let Ok(mut current) = orbit.trigger.lock() {
                    *current = trigger_status;
                }
            }
            watch::start(app.handle().clone());

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

            // A fresh install gets the setup window once, with the three facts
            // that make the app make sense: how to summon it, where files go,
            // and that nothing leaves the machine.
            let first_run = app.try_state::<Orbit>().is_some_and(|orbit| {
                orbit
                    .settings
                    .lock()
                    .is_ok_and(|settings| !settings.onboarded)
            });
            if first_run && !opened_with_files {
                open_settings_window(app.handle(), Some("welcome"));
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            menu_for,
            dispatch,
            option_specs,
            hotkey_status,
            get_settings,
            trigger_status,
            open_settings,
            set_trigger,
            set_output_dir,
            mark_onboarded,
            choose_hotkey,
            history,
            clear_history,
            reveal_path,
            watch_choices,
            add_watch,
            remove_watch,
            set_watch_enabled
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
            // A file landed: the trigger's release timer must not hide the ring,
            // and the window may accept keyboard input again.
            if let Some(orbit) = app.try_state::<Orbit>() {
                orbit.drag_seen.store(true, Ordering::Relaxed);
                orbit.triggered.store(false, Ordering::Relaxed);
            }
            let _ = window.set_focusable(true);
            let _ = window.set_focus();
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
