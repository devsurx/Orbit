import type { CSSProperties } from "react";

import type { MenuModel, Petal } from "../lib/types";

/**
 * The overlay window is square; everything is positioned from this size.
 *
 * 640 with petals at 238 leaves ~30 px of slack for the widest chip label at the
 * horizontal, which is where the ring runs out of window first.
 */
export const RING_SIZE = 640;
const CX = RING_SIZE / 2;
const CY = RING_SIZE / 2;
const R_CATEGORY = 146;
const R_PETAL = 238;

function polar(radius: number, degrees: number): { x: number; y: number } {
  const rad = (degrees * Math.PI) / 180;
  return { x: CX + radius * Math.cos(rad), y: CY + radius * Math.sin(rad) };
}

/** Categories sit on an even circle, first one straight up. */
function categoryAngle(index: number, count: number): number {
  return -90 + (360 / count) * index;
}

/** Petals fan out around their category, never wrapping the whole circle. */
function petalAngle(index: number, count: number, center: number): number {
  if (count <= 1) {
    return center;
  }
  // Keep the fan inside the two neighbouring categories so petals never sit on
  // top of a category chip on the inner ring.
  const spread = Math.min(140, Math.max(50, count * 20));
  const start = center - spread / 2;
  return start + (spread / (count - 1)) * index;
}

function guideRing(radius: number): CSSProperties {
  return {
    left: CX - radius,
    top: CY - radius,
    width: radius * 2,
    height: radius * 2,
  };
}

/**
 * Conversions wear their target format. Everything else wears its label — except
 * the overflow petal, which says how many petals were hidden rather than echoing
 * the "More" *category* sitting on the ring behind it.
 */
function tokenOf(petal: Petal, hidden = 0): string {
  if (petal.kind === "conversion") {
    return petal.icon.toUpperCase();
  }
  if (petal.id === "op:more") {
    return hidden > 0 ? `+${hidden} more` : "More";
  }
  return petal.label.replace(/…/g, "").trim();
}

interface Props {
  model: MenuModel;
  activeCategory: string | null;
  hoveredPetal: Petal | null;
  onHoverCategory: (id: string | null) => void;
  onHoverPetal: (petal: Petal | null) => void;
  onPick: (petal: Petal) => void;
}

export default function RadialMenu({
  model,
  activeCategory,
  hoveredPetal,
  onHoverCategory,
  onHoverPetal,
  onPick,
}: Props) {
  const categories = model.categories;
  const count = categories.length;
  const active = categories.find((category) => category.id === activeCategory) ?? null;
  const activeIndex = active ? categories.findIndex((category) => category.id === active.id) : -1;

  return (
    <div className="relative animate-bloom" style={{ width: RING_SIZE, height: RING_SIZE }}>
      <div
        className="pointer-events-none absolute rounded-full border border-white/[0.07]"
        style={guideRing(R_CATEGORY)}
      />
      <div
        className="pointer-events-none absolute rounded-full border border-white/[0.045]"
        style={guideRing(R_PETAL + 20)}
      />

      {categories.map((category, index) => {
        const { x, y } = polar(R_CATEGORY, categoryAngle(index, count));
        const isActive = category.id === activeCategory;
        return (
          <button
            // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions
            key={category.id}
            type="button"
            onMouseEnter={() => onHoverCategory(category.id)}
            onClick={() => onHoverCategory(isActive ? null : category.id)}
            className={[
              "absolute -translate-x-1/2 -translate-y-1/2 whitespace-nowrap rounded-full px-3 py-2",
              "text-[11px] font-medium tracking-wide transition-colors duration-75",
              isActive
                ? "bg-accent text-white shadow-[0_0_0_1px_rgba(255,255,255,.14),0_8px_30px_rgba(110,74,255,.45)]"
                : "bg-ink-800/85 text-white/70 ring-1 ring-white/10 hover:bg-ink-700/90 hover:text-white",
            ].join(" ")}
            style={{ left: x, top: y }}
            title={category.label}
          >
            {category.label}
            {category.hidden > 0 && (
              <span className="ml-1 text-[10px] text-white/40">+{category.hidden}</span>
            )}
          </button>
        );
      })}

      {active?.petals.map((petal, index) => {
        const angle = petalAngle(index, active.petals.length, categoryAngle(activeIndex, count));
        const { x, y } = polar(R_PETAL, angle);
        const isDisabled = Boolean(petal.disabled);
        const isHot = hoveredPetal?.id === petal.id && !isDisabled;
        return (
          <button
            key={petal.id}
            type="button"
            disabled={isDisabled}
            onMouseEnter={() => onHoverPetal(petal)}
            onMouseLeave={() => onHoverPetal(null)}
            onClick={() => onPick(petal)}
            className={[
              "absolute -translate-x-1/2 -translate-y-1/2 animate-petal rounded-2xl px-3 py-2",
              "max-w-[8rem] truncate text-xs font-semibold transition-all duration-75",
              isDisabled
                ? "cursor-not-allowed bg-ink-800/50 text-white/25 ring-1 ring-white/5"
                : isHot
                  ? "scale-[1.08] bg-accent text-white shadow-[0_10px_34px_rgba(110,74,255,.5)]"
                  : "bg-ink-800/90 text-white/85 ring-1 ring-white/10 hover:bg-ink-700",
            ].join(" ")}
            style={{ left: x, top: y }}
            title={petal.disabled?.reason ?? petal.label}
          >
            {tokenOf(petal, active.hidden)}
            {petal.badge && (
              <span className="absolute -right-1 -top-2 rounded-full bg-ink-900/90 px-1.5 py-0.5 text-[9px] font-normal text-white/55 ring-1 ring-white/10">
                {petal.coverage}/{petal.total}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}
