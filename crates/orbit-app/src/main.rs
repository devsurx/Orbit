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

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use orbit_jobs::{Job, JobEvent, JobState, Pick, PickKind, Runner, resolve_pick};
use orbit_registry::{
    EngineStatus, MenuModel, MenuRequest, Petal, PetalKind, Registry, SourceFile,
};
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, Runtime, State, WebviewWindow, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// Ring window label; must match `tauri.conf.json`.
const RING: &str = "ring";
/// Default trigger. Every platform can have this, even where Shift-drag cannot.
const HOTKEY: &str = "CmdOrCtrl+Shift+Space";
/// Ring diameter; must match the window size in `tauri.conf.json`.
const RING_SIZE: f64 = 640.0;
/// Event carrying the current ring contents to the webview.
const MENU_EVENT: &str = "orbit://menu";
/// Event carrying drag-hover state, so the ring can light up before a drop.
const DRAG_EVENT: &str = "orbit://drag";
/// Event carrying job progress, completion and failure.
const JOB_EVENT: &str = "orbit://job";

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
}

impl Orbit {
    /// Load the registry and probe which engines exist on this machine.
    fn new() -> Result<Self, String> {
        let registry = Registry::bundled().map_err(|error| error.to_string())?;
        let statuses = detect_engines(&registry);
        Ok(Self {
            registry,
            statuses: Mutex::new(statuses),
            last_drop: Mutex::new(Vec::new()),
            model: Mutex::new(None),
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

/// Run a petal pick: resolve it into jobs and hand them to the pool.
///
/// The pick is checked against the ring as last rendered, so a stale window can
/// neither invent a petal nor run one against a different selection. Arguments
/// arrive by value; see `menu_for`.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn dispatch(
    state: State<'_, Orbit>,
    jobs: State<'_, Jobs>,
    petal_kind: String,
    reference: String,
    files: Vec<String>,
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

    let pick = Pick {
        kind,
        reference: reference.clone(),
        defaults: petal.defaults.clone(),
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

/// Whether this session is Wayland, where a client cannot own a global hotkey.
fn on_wayland() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// Register the in-process hotkey, if this platform can actually deliver one.
///
/// `global-hotkey` is X11-only on Linux: under Wayland the grab is accepted at
/// the X protocol level and then never fires, because the compositor owns input.
/// That silent success is worse than a failure, so say so out loud and point at
/// the compositor binding instead of leaving the user pressing keys at nothing.
fn register_hotkey<R: Runtime>(app: &AppHandle<R>) {
    match app.global_shortcut().register(HOTKEY) {
        Ok(()) if on_wayland() => {
            eprintln!(
                "orbit: hotkey {HOTKEY} cannot work on Wayland (the compositor owns \
                 global input).\n\
                 orbit: bind Orbit in your compositor instead — see \"Global hotkeys \
                 on Wayland\" in the README."
            );
        }
        Ok(()) => {}
        Err(error) => {
            eprintln!("orbit: could not register {HOTKEY} ({error}); use the tray icon.");
        }
    }
}

/// Centre the window on the pointer, which is what makes this feel radial.
#[allow(clippy::cast_possible_truncation)] // screen coordinates are tiny next to i32
fn position_at_cursor<R: Runtime>(window: &WebviewWindow<R>) -> tauri::Result<()> {
    let cursor = window.cursor_position()?;
    let x = (cursor.x - RING_SIZE / 2.0).round() as i32;
    let y = (cursor.y - RING_SIZE / 2.0).round() as i32;
    window.set_position(PhysicalPosition::new(x, y))
}

/// Build the tray icon. Left click opens the ring; the menu has the rest.
fn setup_tray<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open ring", true, Some(HOTKEY))?;
    let reload = MenuItem::with_id(app, "reload", "Reload engines", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Orbit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &reload, &quit])?;

    let mut builder = TrayIconBuilder::with_id("orbit")
        .tooltip("Orbit — radial converter")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
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
                    if let Ok(mut last) = orbit.last_drop.lock() {
                        *last = paths;
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
            let orbit = Orbit::new()?;

            // Files handed to us on the command line are the "Open with Orbit"
            // / "Send to" route: treat them as the current selection.
            let from_cli = cli_paths();
            let opened_with_files = !from_cli.is_empty();
            if opened_with_files {
                if let Ok(mut last) = orbit.last_drop.lock() {
                    *last = from_cli;
                }
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
            register_hotkey(app.handle());

            // macOS: no dock icon, no space in the app switcher.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            if opened_with_files {
                show_ring(app.handle());
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![menu_for, dispatch])
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
