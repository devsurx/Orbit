import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import type { DispatchAck, MenuModel, Petal } from "./types";

/** Event the Rust shell emits whenever the ring's contents change. */
export const MENU_EVENT = "orbit://menu";

/** Build the ring for a set of paths. An empty list asks for the idle ring. */
export function menuFor(paths: string[]): Promise<MenuModel> {
  return invoke<MenuModel>("menu_for", { paths });
}

/** Ask the shell to carry out a petal. Execution itself lands in M2. */
export function dispatchPetal(petal: Petal, files: string[]): Promise<DispatchAck> {
  return invoke<DispatchAck>("dispatch", {
    petalKind: petal.kind,
    reference: petal.reference,
    files,
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

export function currentWindow() {
  return getCurrentWindow();
}
