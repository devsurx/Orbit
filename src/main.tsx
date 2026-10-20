import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import App from "./App";
import Settings, { type SettingsSection } from "./Settings";
import "./styles.css";

const container = document.getElementById("root");
if (!container) {
  throw new Error("Orbit: #root is missing from index.html");
}

/**
 * The settings window is the same bundle opened at a hash route. A hash rather
 * than a query string, because the production asset resolver only ever sees
 * `index.html`; the fragment is the webview's business.
 */
function route(): { view: "ring" } | { view: "settings"; section: SettingsSection } {
  const hash = window.location.hash.replace(/^#/, "");
  if (hash !== "settings" && !hash.startsWith("settings/")) {
    return { view: "ring" };
  }
  const requested = hash.slice("settings".length).replace(/^\//, "");
  const sections: SettingsSection[] = ["general", "watch", "history", "about"];
  const section = sections.find((candidate) => candidate === requested) ?? "general";
  return { view: "settings", section };
}

const page = route();

createRoot(container).render(
  <StrictMode>
    {page.view === "settings" ? <Settings initialSection={page.section} /> : <App />}
  </StrictMode>,
);
