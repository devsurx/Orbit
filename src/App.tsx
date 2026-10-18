import { useCallback, useEffect, useMemo, useState } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";

import RadialMenu, { RING_SIZE } from "./components/RadialMenu";
import { dispatchPetal, hideRing, menuFor, onMenu, currentWindow } from "./lib/ipc";
import type { DispatchAck, MenuModel, Petal } from "./lib/types";

interface DragState {
  active: boolean;
  paths: string[];
}

const IDLE: DragState = { active: false, paths: [] };

export default function App() {
  const [model, setModel] = useState<MenuModel | null>(null);
  const [activeCategory, setActiveCategory] = useState<string | null>(null);
  const [hoveredPetal, setHoveredPetal] = useState<Petal | null>(null);
  const [drag, setDrag] = useState<DragState>(IDLE);
  const [ack, setAck] = useState<DispatchAck | null>(null);
  const [error, setError] = useState<string | null>(null);

  const files = useMemo(
    () => (model?.files ?? []).map((file) => file.path).filter((path): path is string => Boolean(path)),
    [model],
  );

  // Ask the registry for the idle ring, then follow whatever the shell pushes.
  useEffect(() => {
    let unlistenMenu: UnlistenFn | undefined;
    let unlistenDrag: UnlistenFn | undefined;
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
      });

      unlistenDrag = await currentWindow().listen<DragState>("orbit://drag", (event) => {
        setDrag(event.payload);
      });
    };

    void boot();
    return () => {
      cancelled = true;
      unlistenMenu?.();
      unlistenDrag?.();
    };
  }, []);

  // Esc always gets you out, whatever has focus.
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

  const pick = useCallback(
    async (petal: Petal) => {
      if (petal.disabled) {
        setAck({ ok: false, message: petal.disabled.reason, detail: petal.disabled.hint });
        return;
      }
      try {
        const result = await dispatchPetal(petal, files);
        setAck(result);
        if (result.ok) {
          window.setTimeout(() => void hideRing(), 700);
        }
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

        {model && !idle && (
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
          error={error}
        />
      </div>
    </div>
  );
}

interface HubProps {
  model: MenuModel | null;
  idle: boolean;
  dragging: boolean;
  dropped: number;
  hoveredPetal: Petal | null;
  ack: DispatchAck | null;
  error: string | null;
}

function Hub({ model, idle, dragging, dropped, hoveredPetal, ack, error }: HubProps) {
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
    ? (ack.detail ?? (ack.ok ? "Job queue lands in M2" : "Unavailable"))
    : hoveredPetal
      ? hoveredPetal.disabled?.reason ?? hintFor(hoveredPetal)
      : idle
        ? "or press Ctrl+Shift+Space"
        : "Esc to cancel";

  return (
    <div className="pointer-events-none absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 text-center">
      <div className="flex h-[172px] w-[172px] flex-col items-center justify-center rounded-full bg-ink-900/85 px-6 ring-1 ring-white/10 backdrop-blur-md">
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
