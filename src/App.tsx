import { useCallback, useEffect, useMemo, useState } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";

import RadialMenu, { RING_SIZE } from "./components/RadialMenu";
import { dispatchPetal, hideRing, menuFor, onJob, onMenu, currentWindow } from "./lib/ipc";
import type { DispatchAck, JobEvent, MenuModel, Petal } from "./lib/types";

interface DragState {
  active: boolean;
  paths: string[];
}

const IDLE: DragState = { active: false, paths: [] };

/** How many finished jobs to keep around for the summary. */
const JOB_HISTORY = 200;

/** How long to leave a finished batch on screen before getting out of the way. */
const LINGER_MS = 1200;

export default function App() {
  const [model, setModel] = useState<MenuModel | null>(null);
  const [activeCategory, setActiveCategory] = useState<string | null>(null);
  const [hoveredPetal, setHoveredPetal] = useState<Petal | null>(null);
  const [drag, setDrag] = useState<DragState>(IDLE);
  const [ack, setAck] = useState<DispatchAck | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [jobs, setJobs] = useState<Map<number, JobEvent>>(() => new Map());

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
    let cancelled = false;

    const boot = async () => {
      try {
        const initial = await menuFor([]);
        if (!cancelled) {
          setModel(initial);
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
    };

    void boot();
    return () => {
      cancelled = true;
      unlistenMenu?.();
      unlistenDrag?.();
      unlistenJob?.();
    };
  }, []);

  // Esc always gets you out, whatever has focus. Jobs keep running.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        void hideRing();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  // Once nothing is in flight, step aside — unless something failed, in which
  // case the ring stays up until it is read or dismissed.
  useEffect(() => {
    if (progress.total === 0 || progress.inFlight > 0 || progress.failed > 0) {
      return;
    }
    const timer = window.setTimeout(() => void hideRing(), LINGER_MS);
    return () => window.clearTimeout(timer);
  }, [progress.total, progress.inFlight, progress.failed]);

  const pick = useCallback(
    async (petal: Petal) => {
      if (petal.disabled) {
        setAck({ ok: false, message: petal.disabled.reason, detail: petal.disabled.hint });
        return;
      }
      setJobs(new Map());
      try {
        setAck(await dispatchPetal(petal, files));
      } catch (cause) {
        setAck({ ok: false, message: String(cause) });
      }
    },
    [files],
  );

  const idle = (model?.files.length ?? 0) === 0;

  return (
    <div
      className="flex h-full w-full items-center justify-center"
      onClick={() => void hideRing()}
      role="presentation"
    >
      <div
        className="relative"
        style={{ width: RING_SIZE, height: RING_SIZE }}
        onClick={(event) => event.stopPropagation()}
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

        {model && !idle && progress.inFlight === 0 && (
          <RadialMenu
            model={model}
            activeCategory={activeCategory}
            hoveredPetal={hoveredPetal}
            onHoverCategory={setActiveCategory}
            onHoverPetal={setHoveredPetal}
            onPick={pick}
          />
        )}

        <Hub
          model={model}
          idle={drag.active || idle}
          dragging={drag.active}
          dropped={drag.paths.length}
          hoveredPetal={hoveredPetal}
          ack={ack}
          progress={progress}
          error={error}
        />
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
    firstError,
  };
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
  return `${progress.done} file${progress.done === 1 ? "" : "s"} done`;
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
}

function Hub({ model, idle, dragging, dropped, hoveredPetal, ack, progress, error }: HubProps) {
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
        ? "or press Ctrl+Shift+Space"
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

function hintFor(petal: Petal): string {
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
}
