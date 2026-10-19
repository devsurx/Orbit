import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import type { DispatchAck, HotkeyStatus, JobEvent, MenuModel, OptionSpec, Petal } from "./types";

/** Event the Rust shell emits whenever the ring's contents change. */
export const MENU_EVENT = "orbit://menu";

/** Event the Rust shell emits for every job state change. */
export const JOB_EVENT = "orbit://job";

/** Event the Rust shell emits when the global shortcut changes. */
export const HOTKEY_EVENT = "orbit://hotkey";

/** The global shortcut as it stands, so the ring names the real one. */
export function hotkeyStatus(): Promise<HotkeyStatus> {
  return invoke<HotkeyStatus>("hotkey_status");
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

export function currentWindow() {
  return getCurrentWindow();
}
