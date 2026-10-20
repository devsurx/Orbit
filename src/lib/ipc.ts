import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import type {
  DispatchAck,
  HistoryEntry,
  HotkeyStatus,
  JobEvent,
  MenuModel,
  OptionSpec,
  Petal,
  SettingsView,
  TriggerStatus,
  WatchChoice,
} from "./types";

/** Event the Rust shell emits whenever the ring's contents change. */
export const MENU_EVENT = "orbit://menu";

/** Event the Rust shell emits for every job state change. */
export const JOB_EVENT = "orbit://job";

/** Event the Rust shell emits when the global shortcut changes. */
export const HOTKEY_EVENT = "orbit://hotkey";

/** Event the Rust shell emits when the settings document changes. */
export const SETTINGS_EVENT = "orbit://settings";

/** Event telling an open settings window to jump to a section. */
export const SECTION_EVENT = "orbit://section";

/** Event telling an open settings window that the history changed. */
export const HISTORY_EVENT = "orbit://history";

/** The global shortcut as it stands, so the ring names the real one. */
export function hotkeyStatus(): Promise<HotkeyStatus> {
  return invoke<HotkeyStatus>("hotkey_status");
}

/** Whether Shift-drag is listening, for the idle hint. */
export function triggerStatus(): Promise<TriggerStatus> {
  return invoke<TriggerStatus>("trigger_status");
}

/** Follow shortcut changes made from the tray. */
export function onHotkey(handler: (status: HotkeyStatus) => void): Promise<UnlistenFn> {
  return listen<HotkeyStatus>(HOTKEY_EVENT, (event) => handler(event.payload));
}

/** Build the ring for a set of paths. An empty list asks for the idle ring. */
export function menuFor(paths: string[]): Promise<MenuModel> {
  return invoke<MenuModel>("menu_for", { paths });
}

/**
 * The registry's whole option catalog, fetched once.
 *
 * A petal only names the keys it accepts; this is where the widget for each key
 * is described.
 */
export function optionSpecs(): Promise<Record<string, OptionSpec>> {
  return invoke<Record<string, OptionSpec>>("option_specs");
}

/**
 * Ask the shell to carry out a petal. Progress arrives on `orbit://job`.
 *
 * `options` are the panel's values; omitted or empty means the petal's own
 * defaults, which is what a plain drop uses.
 */
export function dispatchPetal(
  petal: Petal,
  files: string[],
  options?: Record<string, unknown>,
): Promise<DispatchAck> {
  return invoke<DispatchAck>("dispatch", {
    petalKind: petal.kind,
    reference: petal.reference,
    files,
    options: options ?? null,
  });
}

/** Hide the overlay (Esc, backdrop click, or finishing an action). */
export function hideRing(): Promise<void> {
  return getCurrentWindow().hide();
}

/** Resize the ring so it hugs the cursor instead of the window centre. */
export function onMenu(handler: (model: MenuModel) => void): Promise<UnlistenFn> {
  return listen<MenuModel>(MENU_EVENT, (event) => handler(event.payload));
}

/** Follow the jobs this ring started, so the hub can show what happened. */
export function onJob(handler: (job: JobEvent) => void): Promise<UnlistenFn> {
  return listen<JobEvent>(JOB_EVENT, (event) => handler(event.payload));
}

// --- Settings -------------------------------------------------------------

/** The settings document and the live state of each trigger. */
export function getSettings(): Promise<SettingsView> {
  return invoke<SettingsView>("get_settings");
}

/** Follow changes made in another window. */
export function onSettings(handler: () => void): Promise<UnlistenFn> {
  return listen(SETTINGS_EVENT, () => handler());
}

/** Follow newly finished jobs while the settings window is open. */
export function onHistory(handler: () => void): Promise<UnlistenFn> {
  return listen(HISTORY_EVENT, () => handler());
}

/** Follow "open at this section" requests. */
export function onSection(handler: (section: string) => void): Promise<UnlistenFn> {
  return listen<string>(SECTION_EVENT, (event) => handler(event.payload));
}

/** Open or focus the settings window. */
export function openSettings(section?: string): Promise<void> {
  return invoke("open_settings", { section: section ?? null });
}

/** Turn Shift-drag on or off. */
export function setTrigger(enabled: boolean): Promise<SettingsView> {
  return invoke<SettingsView>("set_trigger", { enabled });
}

/** Choose where results go; `null` means "beside the source". */
export function setOutputDir(dir: string | null): Promise<SettingsView> {
  return invoke<SettingsView>("set_output_dir", { dir });
}

/** Dismiss the first-run welcome for good. */
export function markOnboarded(): Promise<SettingsView> {
  return invoke<SettingsView>("mark_onboarded");
}

/** Change the global shortcut from the settings window. */
export function chooseHotkey(accelerator: string | null): Promise<HotkeyStatus> {
  return invoke<HotkeyStatus>("choose_hotkey", { accelerator });
}

/** The recent jobs, newest first. */
export function history(limit = 50): Promise<HistoryEntry[]> {
  return invoke<HistoryEntry[]>("history", { limit });
}

/** Forget every recorded job. */
export function clearHistory(): Promise<boolean> {
  return invoke<boolean>("clear_history");
}

/** Show one result in the file manager. */
export function revealPath(path: string): Promise<boolean> {
  return invoke<boolean>("reveal_path", { path });
}

/** Everything a watch folder can be pointed at. */
export function watchChoices(): Promise<WatchChoice[]> {
  return invoke<WatchChoice[]>("watch_choices");
}

/** Add a folder, or retarget the rule already set for it. */
export function addWatch(
  path: string,
  kind: string,
  reference: string,
): Promise<SettingsView> {
  return invoke<SettingsView>("add_watch", { path, kind, reference });
}

/** Remove a folder from the watch list. */
export function removeWatch(index: number): Promise<SettingsView> {
  return invoke<SettingsView>("remove_watch", { index });
}

/** Pause or resume one watch rule. */
export function setWatchEnabled(index: number, enabled: boolean): Promise<SettingsView> {
  return invoke<SettingsView>("set_watch_enabled", { index, enabled });
}

export function currentWindow() {
  return getCurrentWindow();
}
