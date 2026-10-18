//! Orbit's desktop shell: tray icon, global hotkey and the radial overlay.
//!
//! The shell owns three things and nothing else:
//!
//! * a **tray icon**, so Orbit is reachable on machines where global hotkeys or
//!   Shift-drag are unavailable (Wayland, locked-down macOS);
//! * a **trigger**: an in-process global hotkey where the platform allows one,
//!   plus single-instance handling so a *compositor* binding can drive Orbit
//!   where it does not (Wayland — see the README);
//! * the **overlay window**, a real drop target that renders the ring.
//!
//! Every decision — which petals exist, what they do, why one is greyed out —
//! lives in `orbit-registry`. This file only moves values between the registry,
//! the operating system and the webview.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use orbit_registry::{EngineStatus, MenuModel, MenuRequest, Registry, SourceFile};
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

/// Everything the commands need, built once at startup.
struct Orbit {
    registry: Registry,
    /// Engines detected at startup; refreshed when the user installs one.
    available: Mutex<BTreeSet<String>>,
    /// Last dropped selection, so the hotkey can reopen the same ring.
    last_drop: Mutex<Vec<PathBuf>>,
}

impl Orbit {
    /// Load the registry and probe which engines exist on this machine.
    fn new() -> Result<Self, String> {
        let registry = Registry::bundled().map_err(|error| error.to_string())?;
        let available = detect_available(&registry);
        Ok(Self {
            registry,
            available: Mutex::new(available),
            last_drop: Mutex::new(Vec::new()),
        })
    }

    /// Engine ids usable right now.
    fn available(&self) -> BTreeSet<String> {
        self.available
            .lock()
            .map(|set| set.clone())
            .unwrap_or_default()
    }

    /// Re-probe the `PATH`, for after the user installs `FFmpeg` or similar.
    fn refresh_engines(&self) {
        let detected = detect_available(&self.registry);
        if let Ok(mut set) = self.available.lock() {
            *set = detected;
        }
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
    fn menu(&self, files: Vec<SourceFile>) -> MenuModel {
        let mut request = MenuRequest::for_files(files);
        request.available = Some(self.available());
        self.registry.menu(&request)
    }

    /// The ring for the last dropped selection, which is what the hotkey shows.
    fn current_menu(&self) -> MenuModel {
        let files = self
            .last_drop
            .lock()
            .map(|paths| paths.clone())
            .unwrap_or_default();
        self.menu(self.classify(files))
    }
}

/// Which engines of a registry are present on this machine right now.
fn detect_available(registry: &Registry) -> BTreeSet<String> {
    let statuses: std::collections::BTreeMap<String, EngineStatus> = registry.detect_engines(
        orbit_registry::host_platform(),
        std::env::var("PATH").ok().as_deref(),
    );
    orbit_registry::available_ids(&statuses)
}

/// What the webview shows after a petal is picked.
#[derive(Debug, Serialize)]
struct DispatchAck {
    /// Whether the request was accepted.
    ok: bool,
    /// One-line summary, shown in the hub.
    message: String,
    /// Secondary line: engine, file count or the reason it could not run.
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
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

/// Acknowledge a petal pick. Real execution lands in M2.
///
/// Arguments arrive by value; see `menu_for`.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn dispatch(petal_kind: String, reference: String, files: Vec<String>) -> DispatchAck {
    if files.is_empty() {
        return DispatchAck {
            ok: false,
            message: "Nothing to work on".to_owned(),
            detail: Some("Drop files onto the ring first.".to_owned()),
        };
    }
    let verb = match petal_kind.as_str() {
        "conversion" => format!("Convert to {}", reference.to_uppercase()),
        "action" => format!("Run {reference}"),
        "preset" => format!("Apply {reference}"),
        _ => reference.clone(),
    };
    let engine = match petal_kind.as_str() {
        "conversion" => "conversion engine",
        _ => "action engine",
    };
    DispatchAck {
        ok: true,
        message: verb,
        detail: Some(format!(
            "{} file{}, via {engine} — execution lands in M2",
            files.len(),
            if files.len() == 1 { "" } else { "s" }
        )),
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
