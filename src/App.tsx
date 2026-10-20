import { useCallback, useEffect, useMemo, useState } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";

import RadialMenu, { RING_SIZE } from "./components/RadialMenu";
import OptionsPanel, { isVisible, type OptionValues } from "./components/OptionsPanel";
import {
  currentWindow,
  dispatchPetal,
  hideRing,
  hotkeyStatus,
  menuFor,
  onHotkey,
  onJob,
  onMenu,
  onSettings,
  openSettings,
  optionSpecs,
  triggerStatus,
} from "./lib/ipc";
import type {
  DispatchAck,
  HotkeyStatus,
  JobEvent,
  MenuModel,
  OptionSpec,
  Petal,
  TriggerStatus,
} from "./lib/types";

interface DragState {
  active: boolean;
  paths: string[];
}

const IDLE: DragState = { active: false, paths: [] };

/** How many finished jobs to keep around for the summary. */
const JOB_HISTORY = 200;

/** How long to leave a finished batch on screen before getting out of the way. */
const LINGER_MS = 1200;

/** A petal whose options are open, and the values currently shown. */
interface PanelState {
  petal: Petal;
  values: OptionValues;
}

export default function App() {
  const [model, setModel] = useState<MenuModel | null>(null);
  const [activeCategory, setActiveCategory] = useState<string | null>(null);
  const [hoveredPetal, setHoveredPetal] = useState<Petal | null>(null);
  const [drag, setDrag] = useState<DragState>(IDLE);
  const [ack, setAck] = useState<DispatchAck | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [jobs, setJobs] = useState<Map<number, JobEvent>>(() => new Map());
  const [specs, setSpecs] = useState<Record<string, OptionSpec>>({});
  const [hotkey, setHotkey] = useState<HotkeyStatus | null>(null);
  const [trigger, setTrigger] = useState<TriggerStatus | null>(null);
  const [panel, setPanel] = useState<PanelState | null>(null);

  const files = useMemo(
    () => (model?.files ?? []).map((file) => file.path).filter((path): path is string => Boolean(path)),
    [model],
  );

  const progress = useMemo(() => summarise(jobs), [jobs]);

  // Ask the registry for the idle ring, then follow whatever the shell pushes.
  useEffect(() => {
    let unlistenMenu: UnlistenFn | undefined;
    let unlistenDrag: UnlistenFn | undefined;
    let unlistenJob: UnlistenFn | undefined;
    let unlistenHotkey: UnlistenFn | undefined;
    let unlistenSettings: UnlistenFn | undefined;
    let cancelled = false;

    const boot = async () => {
      try {
        const [initial, catalog, status, dragging] = await Promise.all([
          menuFor([]),
          optionSpecs(),
          hotkeyStatus(),
          triggerStatus(),
        ]);
        if (!cancelled) {
          setModel(initial);
          setSpecs(catalog);
          setHotkey(status);
          setTrigger(dragging);
        }
      } catch (cause) {
        if (!cancelled) {
          setError(String(cause));
        }
      }

      unlistenMenu = await onMenu((next) => {
        setModel(next);
        setActiveCategory(null);
        setHoveredPetal(null);
        setAck(null);
        setPanel(null);
        // A new ring describes a new selection, so last time's jobs are done.
        setJobs(new Map());
      });

      unlistenDrag = await currentWindow().listen<DragState>("orbit://drag", (event) => {
        setDrag(event.payload);
      });

      unlistenJob = await onJob((job) => {
        setJobs((previous) => {
          const next = new Map(previous);
          next.delete(job.id);
          next.set(job.id, job);
          while (next.size > JOB_HISTORY) {
            const oldest = next.keys().next().value;
            if (oldest === undefined) {
              break;
            }
            next.delete(oldest);
          }
          return next;
        });
      });

      unlistenHotkey = await onHotkey(setHotkey);

      unlistenSettings = await onSettings(() => {
        // The trigger can be turned on or off in the settings window; the idle
        // hint should not keep claiming a gesture that is switched off.
        void triggerStatus()
          .then((status) => {
            if (!cancelled) {
              setTrigger(status);
            }
          })
          .catch(() => undefined);
      });
    };

    void boot();
    return () => {
      cancelled = true;
      unlistenMenu?.();
      unlistenDrag?.();
      unlistenJob?.();
      unlistenHotkey?.();
      unlistenSettings?.();
    };
  }, []);

  // Once nothing is in flight, step aside — unless something failed, in which
  // case the ring stays up until it is read or dismissed.
  useEffect(() => {
    if (
      panel ||
      progress.total === 0 ||
      progress.inFlight > 0 ||
      progress.failed > 0
    ) {
      return;
    }
    const timer = window.setTimeout(() => void hideRing(), LINGER_MS);
    return () => window.clearTimeout(timer);
  }, [panel, progress.total, progress.inFlight, progress.failed]);

  /** Run a petal with an optional set of panel values. */
  const pick = useCallback(
    async (petal: Petal, values?: OptionValues) => {
      if (petal.disabled) {
        setAck({ ok: false, message: petal.disabled.reason, detail: petal.disabled.hint });
        return;
      }
      setJobs(new Map());
      try {
        setAck(await dispatchPetal(petal, files, values));
      } catch (cause) {
        setAck({ ok: false, message: String(cause) });
      }
    },
    [files],
  );

  /**
   * Open the options panel for a petal.
   *
   * The starting values are the petal's own defaults where it has them, and the
   * registry's default otherwise, so the panel always opens showing what a plain
   * drop would have done.
   */
  const openPanel = useCallback(
    (petal: Petal) => {
      if (petal.disabled) {
        setAck({ ok: false, message: petal.disabled.reason, detail: petal.disabled.hint });
        return;
      }
      const values: OptionValues = {};
      for (const key of petal.options ?? []) {
        const spec = specs[key];
        const fallback = spec?.default;
        values[key] = petal.defaults?.[key] ?? fallback ?? "";
      }
      setPanel({ petal, values });
      setActiveCategory(null);
      setHoveredPetal(null);
    },
    [specs],
  );

  /** Run the petal the panel is showing, with the values on screen. */
  const runPanel = useCallback(async () => {
    if (!panel) {
      return;
    }
    const { petal, values } = panel;
    setPanel(null);
    await pick(petal, values);
  }, [panel, pick]);

  // Esc gets you out: the panel closes first, then the ring. Jobs keep running.
  // Enter runs whatever the panel is showing. Registered on every render so the
  // handlers never close over a stale panel.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        if (panel) {
          setPanel(null);
        } else {
          void hideRing();
        }
        return;
      }
      if (event.key === "Enter" && panel) {
        event.preventDefault();
        void runPanel();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  const idle = (model?.files.length ?? 0) === 0;

  // The panel's fields, in catalog order, minus anything its own rules hide.
  const visibleSpecs = useMemo(() => {
    if (!panel) {
      return [];
    }
    return (panel.petal.options ?? [])
      .map((key) => specs[key])
      .filter((spec): spec is OptionSpec => Boolean(spec))
      .filter((spec) => isVisible(spec, panel.values));
  }, [panel, specs]);

  const changeOption = useCallback((key: string, value: unknown) => {
    setPanel((current) =>
      current ? { ...current, values: { ...current.values, [key]: value } } : current,
    );
  }, []);

  return (
    <div
      className="flex h-full w-full items-center justify-center"
      onClick={() => void hideRing()}
      role="presentation"
    >
      <div
        className="relative"
        style={{ width: RING_SIZE, height: RING_SIZE }}
        role="presentation"
      >
        {/* Backdrop: a glassy panel where the OS composites an alpha window
            (macOS/Windows), a plain dark panel on Linux where WebKitGTK cannot
            render a transparent one. Either way the ring reads as one surface. */}
        <div
          className={[
            "absolute inset-0 rounded-[2.25rem] bg-ink-900/90 ring-1 backdrop-blur-xl transition-colors duration-150",
            drag.active ? "ring-accent/70" : "ring-white/[0.07]",
          ].join(" ")}
        />

        {/* The one affordance that is always there — except behind the
            options panel, where it would cover the panel's own Esc. */}
        {!panel && (
          <button
            type="button"
            onClick={(event) => {
              event.stopPropagation();
              void openSettings();
            }}
            className="pointer-events-auto absolute right-5 top-5 z-20 rounded-full bg-ink-800/80 px-3 py-2 text-[11px] text-white/50 ring-1 ring-white/10 transition-colors hover:bg-ink-700 hover:text-white/90"
            title="Orbit settings, history and watch folders"
          >
            Settings
          </button>
        )}

        {model && !idle && progress.inFlight === 0 && !panel && (
          <RadialMenu
            model={model}
            activeCategory={activeCategory}
            hoveredPetal={hoveredPetal}
            onHoverCategory={setActiveCategory}
            onHoverPetal={setHoveredPetal}
            onPick={pick}
            onOptions={openPanel}
          />
        )}

        {panel ? (
          <OptionsPanel
            title={panel.petal.label}
            subtitle={
              panel.petal.engine ? `via ${panel.petal.engine}` : hintFor(panel.petal)
            }
            specs={visibleSpecs}
            values={panel.values}
            onChange={changeOption}
            onRun={() => void runPanel()}
            onClose={() => setPanel(null)}
          />
        ) : (
          <Hub
            model={model}
            idle={drag.active || idle}
            dragging={drag.active}
            dropped={drag.paths.length}
            hoveredPetal={hoveredPetal}
            ack={ack}
            progress={progress}
            error={error}
            hotkey={hotkey}
            trigger={trigger}
          />
        )}
      </div>
    </div>
  );
}

/** What the hub knows about the jobs this ring started. */
interface Progress {
  /** Jobs seen since the ring was last rebuilt. */
  total: number;
  /** Accepted or under way. */
  inFlight: number;
  /** Finished and published. */
  done: number;
  /** Gave up, with a reason. */
  failed: number;
  /** Stopped by the user. */
  cancelled: number;
  /** Mean completion of the jobs that report it, or null when none do. */
  percent: number | null;
  /** Where the finished ones were published, in completion order. */
  outputs: string[];
  /** First failure message, for the one-line summary. */
  firstError: string | null;
}

const NOTHING: Progress = {
  total: 0,
  inFlight: 0,
  done: 0,
  failed: 0,
  cancelled: 0,
  percent: null,
  outputs: [],
  firstError: null,
};

function summarise(jobs: Map<number, JobEvent>): Progress {
  if (jobs.size === 0) {
    return NOTHING;
  }

  let inFlight = 0;
  let done = 0;
  let failed = 0;
  let cancelled = 0;
  let percentTotal = 0;
  let percentCount = 0;
  const outputs: string[] = [];
  let firstError: string | null = null;

  for (const job of jobs.values()) {
    switch (job.state) {
      case "queued":
        inFlight += 1;
        break;
      case "running":
        inFlight += 1;
        if (typeof job.percent === "number") {
          percentTotal += job.percent;
          percentCount += 1;
        }
        break;
      case "done":
        done += 1;
        outputs.push(job.path);
        break;
      case "done_series":
        // One job, many files: a PDF split into pages, a video into frames.
        done += 1;
        outputs.push(...job.paths);
        break;
      case "failed":
        failed += 1;
        firstError ??= job.message;
        break;
      case "cancelled":
        cancelled += 1;
        break;
    }
  }

  return {
    total: jobs.size,
    inFlight,
    done,
    failed,
    cancelled,
    percent: percentCount > 0 ? percentTotal / percentCount : null,
    outputs,
    firstError,
  };
}

/** Last path segment of a path, for naming a file or a folder. */
function lastSegment(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : path;
}

/** The folder a published file landed in: "Pictures", not the full path. */
function folderOf(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.length > 1 ? parts[parts.length - 2] : "/";
}

/** One line describing a batch, or null when there is nothing to say. */
function progressLine(progress: Progress): string | null {
  if (progress.total === 0) {
    return null;
  }
  if (progress.inFlight > 0) {
    const percent =
      progress.percent === null ? "" : ` · ${Math.round(progress.percent)}%`;
    return `${progress.done} of ${progress.total} done${percent}`;
  }
  if (progress.failed > 0) {
    return `${progress.failed} of ${progress.total} failed — ${progress.firstError ?? "unknown error"}`;
  }
  if (progress.cancelled === progress.total) {
    return `${progress.total} cancelled`;
  }

  // Everything landed. A single result is named; a batch says which folder,
  // because the files are written *beside their sources* and that is the one
  // thing worth confirming before the ring goes away.
  if (progress.outputs.length === 1) {
    return lastSegment(progress.outputs[0]);
  }
  const folders = new Set(progress.outputs.map(folderOf));
  const where = folders.size === 1 ? ` in ${[...folders][0]}` : "";
  const count = progress.outputs.length > 0 ? progress.outputs.length : progress.done;
  return `${count} files${where}`;
}

interface HubProps {
  model: MenuModel | null;
  idle: boolean;
  dragging: boolean;
  dropped: number;
  hoveredPetal: Petal | null;
  ack: DispatchAck | null;
  progress: Progress;
  error: string | null;
  hotkey: HotkeyStatus | null;
  trigger: TriggerStatus | null;
}

function Hub({
  model,
  idle,
  dragging,
  dropped,
  hoveredPetal,
  ack,
  progress,
  error,
  hotkey,
  trigger,
}: HubProps) {
  const busy = progress.inFlight > 0;

  const title = ack
    ? ack.message
    : hoveredPetal
      ? hoveredPetal.label
      : dragging
        ? `Drop ${dropped} file${dropped === 1 ? "" : "s"}`
        : idle
          ? "Drop files here"
          : summaryTitle(model);

  const subtitle = ack
    ? (progressLine(progress) ?? ack.detail ?? (ack.ok ? "Queued" : "Unavailable"))
    : hoveredPetal
      ? hoveredPetal.disabled?.reason ?? hintFor(hoveredPetal)
      : idle
        ? hotkeyHint(hotkey, trigger)
        : "Esc to cancel";

  return (
    <div className="pointer-events-none absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 text-center">
      <div
        className={[
          "flex h-[172px] w-[172px] flex-col items-center justify-center rounded-full px-6 ring-1 backdrop-blur-md transition-colors duration-200",
          busy ? "bg-ink-900/85 ring-accent/30" : "bg-ink-900/85 ring-white/10",
        ].join(" ")}
      >
        <p className="line-clamp-2 text-sm font-semibold text-white/90">{title}</p>
        <p className="mt-1 line-clamp-2 text-[10px] leading-tight text-white/45">{subtitle}</p>
        {error && <p className="mt-1 text-[10px] text-red-300/80">{error}</p>}
      </div>
    </div>
  );
}

function summaryTitle(model: MenuModel | null): string {
  const count = model?.files.length ?? 0;
  if (count === 0) {
    return "Orbit";
  }
  const first = model?.files[0];
  return count === 1
    ? `${first?.name ?? "file"}`
    : `${count} files`;
}

/** What to say under the idle hub about the way in. */
function hotkeyHint(
  hotkey: HotkeyStatus | null,
  trigger: TriggerStatus | null,
): string {
  const drag = trigger?.enabled && trigger.listening;
  const key = !hotkey
    ? null
    : hotkey.registered
      ? hotkey.label
      : hotkey.accelerator
        ? // Registered with the platform but not deliverable, which on Linux
          // means Wayland; the README says why in full.
          null
        : null;

  if (drag && key) {
    return `Shift-drag a file, or press ${key}`;
  }
  if (drag) {
    return "Shift-drag a file here";
  }
  if (key) {
    return `or press ${key}`;
  }
  if (hotkey?.accelerator) {
    return "shortcut unavailable here — open from the tray";
  }
  return "open from the tray icon";
}

function hintFor(petal: Petal): string {
  const how = (() => {
    switch (petal.kind) {
      case "conversion":
        return petal.engine ? `via ${petal.engine}` : "convert";
      case "preset":
        return "preset";
      case "operator":
        return "more options";
      default:
        return petal.engine ? `via ${petal.engine}` : "action";
    }
  })();
  // The options panel is easy to miss, so the hint says how to reach it.
  return (petal.options?.length ?? 0) > 0 ? `${how} · Alt for options` : how;
}
