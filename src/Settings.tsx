import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";

import {
  addWatch,
  chooseHotkey,
  clearHistory,
  getSettings,
  history as loadHistory,
  markOnboarded,
  onHistory,
  onSection,
  onSettings,
  openSettings,
  removeWatch,
  revealPath,
  setOutputDir,
  setTrigger,
  setWatchEnabled,
  watchChoices,
} from "./lib/ipc";
import type { HistoryEntry, SettingsView, WatchChoice } from "./lib/types";

export type SettingsSection = "general" | "watch" | "history" | "about";

const SECTIONS: Array<{ id: SettingsSection; label: string; blurb: string }> = [
  { id: "general", label: "General", blurb: "How Orbit opens, and where results go" },
  { id: "watch", label: "Watch folders", blurb: "Turn a folder into a standing instruction" },
  { id: "history", label: "History", blurb: "What Orbit has written, and where" },
  { id: "about", label: "About", blurb: "Version, paths and privacy" },
];

/** One of the three ways a rule can be described, for grouping the picker. */
const RULE_GROUPS: Array<{ kind: string; label: string }> = [
  { kind: "action", label: "Actions" },
  { kind: "preset", label: "Presets" },
  { kind: "conversion", label: "Convert to" },
];

interface Props {
  /** Section from the window's hash, e.g. `#settings/history`. */
  initialSection: SettingsSection;
}

/**
 * The settings window.
 *
 * A normal, decorated window rather than a ring: this is where the long-form
 * things live — the trigger, the output folder, watch rules and the local
 * history. It reads the same settings document the ring uses and listens for
 * changes, so a tray shortcut change shows up here without a reload.
 */
export default function Settings({ initialSection }: Props) {
  const [section, setSection] = useState<SettingsSection>(initialSection);
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [choices, setChoices] = useState<WatchChoice[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    const next = await getSettings();
    setSettings(next);
  }, []);

  const refreshHistory = useCallback(async () => {
    setEntries(await loadHistory(100));
  }, []);

  useEffect(() => {
    let alive = true;
    void (async () => {
      try {
        const [next, recent, catalog] = await Promise.all([
          getSettings(),
          loadHistory(100),
          watchChoices(),
        ]);
        if (!alive) {
          return;
        }
        setSettings(next);
        setEntries(recent);
        setChoices(catalog);
      } catch (cause) {
        if (alive) {
          setError(String(cause));
        }
      }
    })();

    const unlisteners: Array<() => void> = [];
    void onSettings(() => {
      void refresh().catch((cause) => setError(String(cause)));
    }).then((off) => {
      if (alive) {
        unlisteners.push(off);
      } else {
        off();
      }
    });
    void onHistory(() => {
      void loadHistory(100)
        .then((recent) => {
          if (alive) {
            setEntries(recent);
          }
        })
        .catch((cause) => setError(String(cause)));
    }).then((off) => {
      if (alive) {
        unlisteners.push(off);
      } else {
        off();
      }
    });
    void onSection((requested) => {
      const match = SECTIONS.find((candidate) => candidate.id === requested);
      setSection(match?.id ?? "general");
    }).then((off) => {
      if (alive) {
        unlisteners.push(off);
      } else {
        off();
      }
    });

    return () => {
      alive = false;
      for (const off of unlisteners) {
        off();
      }
    };
  }, [refresh]);

  /** Run a settings mutation with consistent error and busy handling. */
  const apply = useCallback(
    async (work: () => Promise<SettingsView>, message?: string) => {
      setBusy(true);
      setError(null);
      setNotice(null);
      try {
        setSettings(await work());
        if (message) {
          setNotice(message);
        }
      } catch (cause) {
        setError(String(cause));
      } finally {
        setBusy(false);
      }
    },
    [],
  );

  const chooseDirectory = useCallback(
    async (title: string): Promise<string | null> => {
      const picked = await open({ directory: true, multiple: false, title });
      if (typeof picked === "string") {
        return picked;
      }
      return null;
    },
    [],
  );

  // The OS window title doubles as a smoke test: if React never rendered, the
  // title stays the one the Rust builder set.
  useEffect(() => {
    const label = SECTIONS.find((candidate) => candidate.id === section)?.label;
    document.title = `Orbit settings — ${label ?? "Settings"}`;
  }, [section]);

  const labelFor = useMemo(() => {
    const byKey = new Map(
      choices.map((choice) => [`${choice.kind}\u0000${choice.reference}`, choice.label]),
    );
    return (kind: string, reference: string) =>
      byKey.get(`${kind}\u0000${reference}`) ?? reference;
  }, [choices]);

  if (!settings) {
    return (
      <main className="flex h-full items-center justify-center bg-ink-900 text-sm text-white/50">
        {error ?? "Loading Orbit settings…"}
      </main>
    );
  }

  const active = SECTIONS.find((candidate) => candidate.id === section) ?? SECTIONS[0];

  return (
    <main className="h-full overflow-y-auto bg-ink-900 text-white [scrollbar-width:thin]">
      <div className="mx-auto flex min-h-full w-full max-w-5xl flex-col px-6 py-6">
        <header className="flex items-start justify-between gap-4 border-b border-white/[0.07] pb-5">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-full bg-accent/15 ring-1 ring-accent/40">
              <span className="relative block h-3 w-3 rounded-full bg-accent shadow-[0_0_16px_rgba(110,74,255,.8)]" />
              <span className="absolute h-6 w-6 rounded-full border border-accent/30" />
            </div>
            <div>
              <h1 className="text-base font-semibold tracking-tight">Orbit</h1>
              <p className="text-[11px] text-white/40">Settings · v{settings.version}</p>
            </div>
          </div>
          <button
            type="button"
            onClick={() => void getCurrentWindow().close()}
            className="rounded-full bg-ink-800 px-3 py-1.5 text-[11px] text-white/60 ring-1 ring-white/10 hover:bg-ink-700 hover:text-white"
          >
            Close
          </button>
        </header>

        {!settings.onboarded && (
          <Welcome
            onDismiss={() =>
              void apply(() => markOnboarded(), "Welcome aboard. Orbit stays in the tray.")
            }
          />
        )}

        <div className="mt-6 flex flex-1 flex-col gap-6 sm:flex-row">
          <nav className="flex shrink-0 flex-row gap-1 overflow-x-auto sm:w-48 sm:flex-col sm:overflow-visible">
            {SECTIONS.map((candidate) => (
              <button
                key={candidate.id}
                type="button"
                onClick={() => setSection(candidate.id)}
                className={[
                  "whitespace-nowrap rounded-xl px-3 py-2 text-left text-xs font-medium transition-colors",
                  candidate.id === section
                    ? "bg-accent/15 text-white ring-1 ring-accent/35"
                    : "text-white/55 hover:bg-ink-800 hover:text-white/85",
                ].join(" ")}
              >
                {candidate.label}
              </button>
            ))}
            <p className="mt-3 hidden px-3 text-[10px] leading-relaxed text-white/30 sm:block">
              Nothing here leaves your machine. Orbit has no telemetry and makes
              no network requests.
            </p>
          </nav>

          <section className="min-w-0 flex-1">
            <div className="mb-4">
              <h2 className="text-sm font-semibold text-white/90">{active.label}</h2>
              <p className="text-[11px] text-white/40">{active.blurb}</p>
            </div>

            {error && (
              <p className="mb-4 rounded-xl bg-red-500/10 px-3 py-2 text-[11px] text-red-200 ring-1 ring-red-400/20">
                {error}
              </p>
            )}
            {notice && (
              <p className="mb-4 rounded-xl bg-emerald-500/10 px-3 py-2 text-[11px] text-emerald-200 ring-1 ring-emerald-400/20">
                {notice}
              </p>
            )}

            {section === "general" && (
              <General
                settings={settings}
                busy={busy}
                onTrigger={(enabled) =>
                  void apply(() => setTrigger(enabled), enabled ? "Shift-drag is on." : "Shift-drag is off.")
                }
                onHotkey={(accelerator) =>
                  void (async () => {
                    try {
                      await chooseHotkey(accelerator);
                      await refresh();
                      setNotice(
                        accelerator
                          ? "The global shortcut was updated."
                          : "The global shortcut is off; the tray and Shift-drag still work.",
                      );
                    } catch (cause) {
                      setError(String(cause));
                    }
                  })()
                }
                onOutput={(dir) =>
                  void apply(
                    () => setOutputDir(dir),
                    dir ? "Results will be written to that folder." : "Results will sit beside their sources.",
                  )
                }
                onChooseDirectory={chooseDirectory}
              />
            )}

            {section === "watch" && (
              <Watch
                settings={settings}
                choices={choices}
                busy={busy}
                labelFor={labelFor}
                onChooseDirectory={chooseDirectory}
                onAdd={(path, kind, reference) =>
                  void (async () => {
                    await apply(() => addWatch(path, kind, reference), "Watch folder added.");
                  })()
                }
                onRemove={(index) =>
                  void apply(() => removeWatch(index), "Watch folder removed.")
                }
                onToggle={(index, enabled) =>
                  void apply(
                    () => setWatchEnabled(index, enabled),
                    enabled ? "Watch folder resumed." : "Watch folder paused.",
                  )
                }
              />
            )}

            {section === "history" && (
              <History
                entries={entries}
                settings={settings}
                onReveal={(path) =>
                  void revealPath(path).then((shown) => {
                    if (!shown) {
                      setError(`Could not show ${path}`);
                    }
                  })
                }
                onClear={() =>
                  void (async () => {
                    try {
                      if (await clearHistory()) {
                        setEntries([]);
                        setNotice("History cleared.");
                      }
                    } catch (cause) {
                      setError(String(cause));
                    }
                  })()
                }
                onRefresh={() =>
                  void refreshHistory().catch((cause) => setError(String(cause)))
                }
              />
            )}

            {section === "about" && <About settings={settings} onReveal={(path) => void revealPath(path)} />}
          </section>
        </div>
      </div>
    </main>
  );
}

interface WelcomeProps {
  onDismiss: () => void;
}

function Welcome({ onDismiss }: WelcomeProps) {
  return (
    <div className="mt-5 rounded-2xl bg-gradient-to-br from-accent/20 to-ink-800 px-5 py-4 ring-1 ring-accent/30">
      <h2 className="text-sm font-semibold text-white">Welcome to Orbit</h2>
      <p className="mt-1 max-w-2xl text-[11px] leading-relaxed text-white/60">
        Drag any file with Shift held to open the ring, or press the global
        shortcut. Pick a format or action; the result is written beside the
        source and never overwrites anything. Everything runs on this machine.
      </p>
      <div className="mt-3 flex items-center gap-3">
        <button
          type="button"
          onClick={onDismiss}
          className="rounded-full bg-accent px-3 py-1.5 text-[11px] font-semibold text-white hover:bg-accent-soft"
        >
          Got it
        </button>
        <button
          type="button"
          onClick={() => void openSettings("watch")}
          className="text-[11px] text-white/50 underline-offset-2 hover:text-white/80 hover:underline"
        >
          Set up a watch folder
        </button>
      </div>
    </div>
  );
}

interface CardProps {
  title: string;
  children: ReactNode;
  hint?: string;
}

function Card({ title, children, hint }: CardProps) {
  return (
    <section className="mb-4 rounded-2xl bg-ink-800/60 p-4 ring-1 ring-white/[0.07]">
      <h3 className="text-xs font-semibold text-white/80">{title}</h3>
      {hint && <p className="mt-0.5 text-[10px] leading-relaxed text-white/35">{hint}</p>}
      <div className="mt-3">{children}</div>
    </section>
  );
}

interface ToggleProps {
  checked: boolean;
  disabled?: boolean;
  label: string;
  onChange: (next: boolean) => void;
}

function Toggle({ checked, disabled, label, onChange }: ToggleProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={[
        "relative h-5 w-9 shrink-0 rounded-full transition-colors",
        checked ? "bg-accent" : "bg-ink-600",
        disabled ? "opacity-50" : "",
      ].join(" ")}
    >
      <span
        className={[
          "absolute top-0.5 h-4 w-4 rounded-full bg-white shadow transition-transform",
          checked ? "translate-x-[18px]" : "translate-x-0.5",
        ].join(" ")}
      />
    </button>
  );
}

interface GeneralProps {
  settings: SettingsView;
  busy: boolean;
  onTrigger: (enabled: boolean) => void;
  onHotkey: (accelerator: string | null) => void;
  onOutput: (dir: string | null) => void;
  onChooseDirectory: (title: string) => Promise<string | null>;
}

function General({
  settings,
  busy,
  onTrigger,
  onHotkey,
  onOutput,
  onChooseDirectory,
}: GeneralProps) {
  const [dirDraft, setDirDraft] = useState(settings.output_dir ?? "");

  useEffect(() => {
    setDirDraft(settings.output_dir ?? "");
  }, [settings.output_dir]);

  return (
    <>
      <Card
        title="Shift-drag"
        hint="Hold Shift while dragging files anywhere in the system and the ring appears under the pointer."
      >
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <p className="text-[11px] text-white/70">
              {settings.trigger ? "On" : "Off"}
              {!settings.trigger_listening && " · not available on this display server"}
            </p>
            {settings.trigger_note && (
              <p className="mt-1 max-w-xl text-[10px] leading-relaxed text-amber-200/70">
                {settings.trigger_note}
              </p>
            )}
          </div>
          <Toggle
            checked={settings.trigger}
            disabled={busy || !settings.trigger_listening}
            label="Shift-drag opens the ring"
            onChange={onTrigger}
          />
        </div>
      </Card>

      <Card title="Global shortcut" hint="Works from any app, even while Orbit is in the tray.">
        <div className="flex flex-wrap gap-1.5">
          {settings.hotkey_presets.map((preset) => {
            const chosen = settings.hotkey === preset.accelerator;
            return (
              <button
                key={preset.accelerator}
                type="button"
                disabled={busy}
                onClick={() => onHotkey(preset.accelerator)}
                className={[
                  "rounded-full px-3 py-1.5 text-[11px] transition-colors",
                  chosen
                    ? "bg-accent text-white"
                    : "bg-ink-800 text-white/60 ring-1 ring-white/10 hover:bg-ink-700 hover:text-white/90",
                ].join(" ")}
              >
                {preset.label}
              </button>
            );
          })}
          <button
            type="button"
            disabled={busy}
            onClick={() => onHotkey(null)}
            className={[
              "rounded-full px-3 py-1.5 text-[11px] transition-colors",
              settings.hotkey === null
                ? "bg-accent text-white"
                : "bg-ink-800 text-white/60 ring-1 ring-white/10 hover:bg-ink-700 hover:text-white/90",
            ].join(" ")}
          >
            Off
          </button>
        </div>
        <p className="mt-2 text-[10px] text-white/35">
          {settings.hotkey_registered
            ? `In force: ${settings.hotkey_label}`
            : settings.hotkey_note ?? "No shortcut is registered."}
        </p>
      </Card>

      <Card
        title="Output folder"
        hint="Results are named “name (converted).ext” and never overwrite an existing file. Leave this empty to write beside each source."
      >
        <div className="flex flex-wrap items-center gap-2">
          <input
            type="text"
            value={dirDraft}
            placeholder="Beside the source"
            onChange={(event) => setDirDraft(event.target.value)}
            className="min-w-0 flex-1 rounded-xl bg-ink-900/80 px-3 py-2 text-[11px] text-white/80 outline-none ring-1 ring-white/10 placeholder:text-white/25 focus:ring-accent/50"
          />
          <button
            type="button"
            disabled={busy}
            onClick={() =>
              void (async () => {
                const picked = await onChooseDirectory("Choose where Orbit writes results");
                if (picked) {
                  setDirDraft(picked);
                  onOutput(picked);
                }
              })()
            }
            className="rounded-xl bg-ink-800 px-3 py-2 text-[11px] text-white/70 ring-1 ring-white/10 hover:bg-ink-700 hover:text-white"
          >
            Choose…
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => onOutput(dirDraft.trim() === "" ? null : dirDraft.trim())}
            className="rounded-xl bg-accent px-3 py-2 text-[11px] font-semibold text-white hover:bg-accent-soft"
          >
            Use this folder
          </button>
          {settings.output_dir && (
            <button
              type="button"
              disabled={busy}
              onClick={() => onOutput(null)}
              className="rounded-xl px-3 py-2 text-[11px] text-white/45 hover:text-white/80"
            >
              Beside the source
            </button>
          )}
        </div>
      </Card>
    </>
  );
}

interface WatchProps {
  settings: SettingsView;
  choices: WatchChoice[];
  busy: boolean;
  labelFor: (kind: string, reference: string) => string;
  onChooseDirectory: (title: string) => Promise<string | null>;
  onAdd: (path: string, kind: string, reference: string) => void;
  onRemove: (index: number) => void;
  onToggle: (index: number, enabled: boolean) => void;
}

function Watch({
  settings,
  choices,
  busy,
  labelFor,
  onChooseDirectory,
  onAdd,
  onRemove,
  onToggle,
}: WatchProps) {
  const [path, setPath] = useState("");
  const [rule, setRule] = useState("");

  const selected = useMemo(() => {
    const [kind, reference] = rule.split("\u0000");
    return kind && reference ? { kind, reference } : null;
  }, [rule]);

  return (
    <>
      <Card
        title="Watched folders"
        hint="Files that appear after you add a folder are converted automatically. Files already there are left alone, and Orbit ignores its own output."
      >
        {settings.watch.length === 0 ? (
          <p className="text-[11px] text-white/35">No folders are watched yet.</p>
        ) : (
          <ul className="divide-y divide-white/[0.06]">
            {settings.watch.map((folder, index) => (
              <li key={`${folder.path}-${index}`} className="flex items-center gap-3 py-2.5">
                <Toggle
                  checked={folder.enabled}
                  disabled={busy}
                  label={`Watch ${folder.path}`}
                  onChange={(enabled) => onToggle(index, enabled)}
                />
                <div className="min-w-0 flex-1">
                  <p className="truncate text-[11px] text-white/80" title={folder.path}>
                    {folder.path}
                  </p>
                  <p className="text-[10px] text-white/40">
                    {labelFor(folder.kind, folder.reference)} · {folder.enabled ? "watching" : "paused"}
                  </p>
                </div>
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => onRemove(index)}
                  className="rounded-lg px-2 py-1 text-[10px] text-white/40 ring-1 ring-white/10 hover:bg-ink-700 hover:text-white/80"
                >
                  Remove
                </button>
              </li>
            ))}
          </ul>
        )}
      </Card>

      <Card title="Add a folder" hint="A folder has one rule; choosing it again replaces the rule.">
        <div className="space-y-2">
          <div className="flex flex-wrap items-center gap-2">
            <input
              type="text"
              value={path}
              placeholder="/home/you/Downloads/inbox"
              onChange={(event) => setPath(event.target.value)}
              className="min-w-0 flex-1 rounded-xl bg-ink-900/80 px-3 py-2 text-[11px] text-white/80 outline-none ring-1 ring-white/10 placeholder:text-white/25 focus:ring-accent/50"
            />
            <button
              type="button"
              disabled={busy}
              onClick={() =>
                void (async () => {
                  const picked = await onChooseDirectory("Choose a folder to watch");
                  if (picked) {
                    setPath(picked);
                  }
                })()
              }
              className="rounded-xl bg-ink-800 px-3 py-2 text-[11px] text-white/70 ring-1 ring-white/10 hover:bg-ink-700 hover:text-white"
            >
              Choose…
            </button>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <select
              value={rule}
              onChange={(event) => setRule(event.target.value)}
              className="min-w-0 flex-1 rounded-xl bg-ink-900/80 px-3 py-2 text-[11px] text-white/80 outline-none ring-1 ring-white/10 focus:ring-accent/50"
            >
              <option value="">Choose what to do…</option>
              {RULE_GROUPS.map((group) => {
                const options = choices.filter((choice) => choice.kind === group.kind);
                if (options.length === 0) {
                  return null;
                }
                return (
                  <optgroup key={group.kind} label={group.label}>
                    {options.map((choice) => (
                      <option
                        key={`${choice.kind}-${choice.reference}`}
                        value={`${choice.kind}\u0000${choice.reference}`}
                      >
                        {choice.label} ({choice.hint})
                      </option>
                    ))}
                  </optgroup>
                );
              })}
            </select>
            <button
              type="button"
              disabled={busy || !selected || path.trim() === ""}
              onClick={() => {
                if (selected) {
                  onAdd(path.trim(), selected.kind, selected.reference);
                  setPath("");
                  setRule("");
                }
              }}
              className="rounded-xl bg-accent px-3 py-2 text-[11px] font-semibold text-white hover:bg-accent-soft disabled:opacity-40"
            >
              Add folder
            </button>
          </div>
        </div>
        <p className="mt-2 text-[10px] leading-relaxed text-white/30">
          Good for download folders and scanners. The folder is checked every two
          seconds; a file must be two seconds old before it is picked up, so a
          copy that is still running is not grabbed halfway through.
        </p>
      </Card>
    </>
  );
}

interface HistoryProps {
  entries: HistoryEntry[];
  settings: SettingsView;
  onReveal: (path: string) => void;
  onClear: () => void;
  onRefresh: () => void;
}

function History({ entries, settings, onReveal, onClear, onRefresh }: HistoryProps) {
  return (
    <>
      <Card
        title="Recent results"
        hint={`A local log at ${settings.history_path}. It is only a convenience; the files themselves are the record.`}
      >
        <div className="mb-3 flex items-center gap-2">
          <button
            type="button"
            onClick={onRefresh}
            className="rounded-lg bg-ink-800 px-2.5 py-1.5 text-[10px] text-white/60 ring-1 ring-white/10 hover:bg-ink-700 hover:text-white"
          >
            Refresh
          </button>
          <button
            type="button"
            onClick={onClear}
            disabled={entries.length === 0}
            className="rounded-lg px-2.5 py-1.5 text-[10px] text-white/40 ring-1 ring-white/10 hover:bg-ink-700 hover:text-white/80 disabled:opacity-40"
          >
            Clear history
          </button>
          <button
            type="button"
            onClick={() => onReveal(settings.history_path)}
            className="ml-auto rounded-lg px-2.5 py-1.5 text-[10px] text-white/40 hover:text-white/80"
          >
            Show log
          </button>
        </div>

        {entries.length === 0 ? (
          <p className="py-6 text-center text-[11px] text-white/35">
            Nothing yet. Convert something and it will appear here.
          </p>
        ) : (
          <ul className="space-y-1.5">
            {entries.map((entry, index) => {
              const anchor = entry.outputs[0] ?? entry.sources[0];
              return (
                <li
                  key={`${entry.at}-${index}`}
                  className="rounded-xl bg-ink-900/50 px-3 py-2 ring-1 ring-white/[0.05]"
                >
                  <div className="flex items-baseline justify-between gap-3">
                    <p className="truncate text-[11px] text-white/80">
                      <span className={entry.ok ? "text-emerald-300/80" : "text-red-300/80"}>
                        {entry.ok ? "✓" : "✕"}
                      </span>{" "}
                      {entry.label}
                    </p>
                    <time className="shrink-0 text-[10px] tabular-nums text-white/30">
                      {formatTime(entry.at)}
                    </time>
                  </div>
                  <div className="mt-0.5 flex items-center gap-2">
                    <p className="min-w-0 flex-1 truncate text-[10px] text-white/40">
                      {entry.sources.map(baseName).join(", ")}
                      {entry.outputs.length > 0 && " → "}
                      {entry.outputs.length <= 2
                        ? entry.outputs.map(baseName).join(", ")
                        : `${entry.outputs.slice(0, 2).map(baseName).join(", ")} +${entry.outputs.length - 2}`}
                    </p>
                    {anchor && (
                      <button
                        type="button"
                        onClick={() => onReveal(anchor)}
                        className="shrink-0 rounded-lg px-2 py-0.5 text-[10px] text-white/40 ring-1 ring-white/10 hover:bg-ink-700 hover:text-white/80"
                      >
                        Show
                      </button>
                    )}
                  </div>
                  {entry.message && (
                    <p className="mt-1 text-[10px] leading-relaxed text-red-200/60">{entry.message}</p>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </Card>
    </>
  );
}

interface AboutProps {
  settings: SettingsView;
  onReveal: (path: string) => void;
}

function About({ settings, onReveal }: AboutProps) {
  return (
    <>
      <Card title="What Orbit is">
        <p className="text-[11px] leading-relaxed text-white/55">
          A radial file converter and editor that lives in the tray. Conversions,
          PDF operations, OCR, media trimming and presets run through the
          command-line engines already on your system. Nothing is uploaded and
          there is no telemetry.
        </p>
        <p className="mt-2 text-[10px] leading-relaxed text-white/30">
          Version {settings.version} · MPL-2.0 ·{" "}
          <span className="select-text">github.com/devsurx/Orbit</span>
        </p>
      </Card>

      <Card title="Files">
        <dl className="space-y-2 text-[10px]">
          <div className="flex items-center gap-2">
            <dt className="w-28 shrink-0 text-white/35">Settings</dt>
            <dd className="min-w-0 flex-1 truncate text-white/60" title={settings.settings_path}>
              {settings.settings_path}
            </dd>
            <button
              type="button"
              onClick={() => onReveal(settings.settings_path)}
              className="rounded-lg px-2 py-0.5 text-white/40 ring-1 ring-white/10 hover:bg-ink-700 hover:text-white/80"
            >
              Show
            </button>
          </div>
          <div className="flex items-center gap-2">
            <dt className="w-28 shrink-0 text-white/35">History</dt>
            <dd className="min-w-0 flex-1 truncate text-white/60" title={settings.history_path}>
              {settings.history_path}
            </dd>
            <button
              type="button"
              onClick={() => onReveal(settings.history_path)}
              className="rounded-lg px-2 py-0.5 text-white/40 ring-1 ring-white/10 hover:bg-ink-700 hover:text-white/80"
            >
              Show
            </button>
          </div>
        </dl>
      </Card>

      {settings.wayland && (
        <Card title="Wayland">
          <p className="text-[10px] leading-relaxed text-white/45">
            Wayland does not let an application observe global input or register
            a global shortcut. Shift-drag and the default shortcut are therefore
            unavailable here. Bind Orbit in your compositor — for Hyprland:
          </p>
          <pre className="mt-2 select-text overflow-x-auto rounded-lg bg-ink-900 p-2 text-[10px] text-white/55">
            bind = SUPER SHIFT, O, exec, orbit
          </pre>
          <p className="mt-2 text-[10px] leading-relaxed text-white/30">
            The tray, “Open with Orbit” and the settings window all still work.
          </p>
        </Card>
      )}
    </>
  );
}

/** The last path segment, for either separator. */
function baseName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : path;
}

/** A compact local timestamp. */
function formatTime(at: number): string {
  if (!Number.isFinite(at) || at <= 0) {
    return "";
  }
  const date = new Date(at);
  const now = new Date();
  const sameDay =
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate();
  return sameDay
    ? date.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })
    : date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}
