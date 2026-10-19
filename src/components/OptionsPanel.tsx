import { useEffect, useRef } from "react";

import type { OptionSpec } from "../lib/types";

/** Option values, keyed the way the registry keys them. */
export type OptionValues = Record<string, unknown>;

/** True when a spec's `show_if` conditions all hold for the current values. */
export function isVisible(spec: OptionSpec, values: OptionValues): boolean {
  if (!spec.show_if) {
    return true;
  }
  return Object.entries(spec.show_if).every(([key, wanted]) => values[key] === wanted);
}

/** A string form of any option value, for text and number inputs. */
function asText(value: unknown): string {
  if (value === null || value === undefined) {
    return "";
  }
  return String(value);
}

/** A number form of any option value, or null when it is not one. */
function asNumber(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }
  if (typeof value === "string" && value.trim() !== "") {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

/** Human name for an enum value, falling back to the raw value. */
function labelOf(spec: OptionSpec, value: unknown): string {
  const raw = asText(value);
  return spec.labels?.[raw] ?? raw;
}

interface Props {
  /** Petal label, used as the panel's heading. */
  title: string;
  /** Secondary line: which engine will run, and what to press. */
  subtitle: string;
  /** Visible specs, in catalog order. */
  specs: OptionSpec[];
  values: OptionValues;
  onChange: (key: string, value: unknown) => void;
  onRun: () => void;
  onClose: () => void;
}

/**
 * The options panel: a keyboard-first card over the ring.
 *
 * Alt or a long press opens it; Enter runs the pick with what is shown, Esc
 * closes it without running anything. Every field is either a native control or
 * a row of buttons, so Tab walks the list and the ring stays usable without a
 * pointer.
 */
export default function OptionsPanel({
  title,
  subtitle,
  specs,
  values,
  onChange,
  onRun,
  onClose,
}: Props) {
  const first = useRef<HTMLDivElement | null>(null);

  // Land on the first control so the panel answers the keyboard immediately.
  useEffect(() => {
    const control = first.current?.querySelector<HTMLElement>(
      "input, select, button:not([disabled])",
    );
    control?.focus();
  }, [specs]);

  return (
    <div
      className="absolute inset-x-16 inset-y-10 z-10 flex flex-col overflow-hidden rounded-3xl bg-ink-900/95 ring-1 ring-white/10 backdrop-blur-xl"
      onClick={(event) => event.stopPropagation()}
      role="dialog"
      aria-label={`${title} options`}
    >
      <header className="flex items-baseline justify-between gap-3 px-5 pt-4">
        <div className="min-w-0">
          <p className="truncate text-sm font-semibold text-white/90">{title}</p>
          <p className="truncate text-[10px] text-white/45">{subtitle}</p>
        </div>
        <button
          type="button"
          onClick={onClose}
          className="rounded-full px-2 py-1 text-[10px] text-white/45 ring-1 ring-white/10 hover:bg-ink-700 hover:text-white/80"
          title="Close without running (Esc)"
        >
          Esc
        </button>
      </header>

      <div
        ref={first}
        className="mt-3 flex-1 space-y-3 overflow-y-auto px-5 pb-3 [scrollbar-width:thin]"
      >
        {specs.length === 0 && (
          <p className="pt-6 text-center text-[11px] text-white/40">
            This petal has nothing to adjust.
          </p>
        )}
        {specs.map((spec) => (
          <Field
            key={spec.key}
            spec={spec}
            value={values[spec.key]}
            onChange={(value) => onChange(spec.key, value)}
          />
        ))}
      </div>

      <footer className="flex items-center justify-between gap-3 border-t border-white/[0.07] px-5 py-3">
        <p className="text-[10px] text-white/40">Enter to run · Esc to cancel</p>
        <button
          type="button"
          onClick={onRun}
          className="rounded-full bg-accent px-4 py-1.5 text-xs font-semibold text-white shadow-[0_8px_24px_rgba(110,74,255,.45)] hover:bg-accent-soft"
        >
          Run
        </button>
      </footer>
    </div>
  );
}

interface FieldProps {
  spec: OptionSpec;
  value: unknown;
  onChange: (value: unknown) => void;
}

/** One labelled option, drawn according to its registry kind. */
function Field({ spec, value, onChange }: FieldProps) {
  return (
    <label className="block">
      <span className="flex items-baseline justify-between gap-2">
        <span className="text-[11px] font-medium text-white/75">{spec.label || spec.key}</span>
        {spec.unit && <span className="text-[10px] text-white/35">{spec.unit}</span>}
      </span>
      <span className="mt-1 block">
        <Control spec={spec} value={value} onChange={onChange} />
      </span>
      {spec.help && <span className="mt-1 block text-[10px] leading-tight text-white/35">{spec.help}</span>}
    </label>
  );
}

/** The widget for an option, chosen from its registry kind. */
function Control({ spec, value, onChange }: FieldProps) {
  const choices = spec.values ?? [];

  switch (spec.type) {
    case "enum":
      return (
        <span className="flex flex-wrap gap-1">
          {choices.map((choice) => {
            const isOn = asText(choice) === asText(value);
            return (
              <button
                key={asText(choice)}
                type="button"
                onClick={() => onChange(choice)}
                className={[
                  "rounded-full px-2.5 py-1 text-[11px] transition-colors duration-75",
                  isOn
                    ? "bg-accent text-white"
                    : "bg-ink-800 text-white/60 ring-1 ring-white/10 hover:bg-ink-700 hover:text-white/90",
                ].join(" ")}
              >
                {labelOf(spec, choice)}
              </button>
            );
          })}
        </span>
      );

    case "multi_enum":
      return (
        <span className="flex flex-wrap gap-1">
          {choices.map((choice) => {
            const chosen = Array.isArray(value) ? value.map(asText) : [];
            const isOn = chosen.includes(asText(choice));
            return (
              <button
                key={asText(choice)}
                type="button"
                onClick={() =>
                  onChange(
                    isOn
                      ? chosen.filter((item) => item !== asText(choice))
                      : [...chosen, asText(choice)],
                  )
                }
                className={[
                  "rounded-full px-2.5 py-1 text-[11px] transition-colors duration-75",
                  isOn
                    ? "bg-accent text-white"
                    : "bg-ink-800 text-white/60 ring-1 ring-white/10 hover:bg-ink-700 hover:text-white/90",
                ].join(" ")}
              >
                {labelOf(spec, choice)}
              </button>
            );
          })}
        </span>
      );

    case "range": {
      const min = spec.min ?? 0;
      const max = spec.max ?? 100;
      const current = asNumber(value) ?? min;
      return (
        <span className="flex items-center gap-2">
          <input
            type="range"
            min={min}
            max={max}
            value={current}
            onChange={(event) => onChange(Number(event.target.value))}
            className="h-1 flex-1 accent-[#6e4aff]"
          />
          <span className="w-10 text-right text-[11px] tabular-nums text-white/70">{current}</span>
        </span>
      );
    }

    case "bool":
      return (
        <input
          type="checkbox"
          checked={value === true}
          onChange={(event) => onChange(event.target.checked)}
          className="h-4 w-4 accent-[#6e4aff]"
        />
      );

    case "int":
    case "number":
    case "percent":
    case "bytes":
      return (
        <input
          type="number"
          value={asText(value)}
          min={spec.min}
          max={spec.max}
          step={spec.type === "int" || spec.type === "percent" ? 1 : "any"}
          placeholder={spec.placeholder}
          onChange={(event) => {
            const text = event.target.value;
            // Keep the field editable while it is empty; the registry's default
            // covers the moment the job is actually run.
            onChange(text === "" ? "" : Number(text));
          }}
          className="w-full rounded-lg bg-ink-800 px-2.5 py-1.5 text-[12px] text-white/90 ring-1 ring-white/10 outline-none focus:ring-accent/60"
        />
      );

    default:
      return (
        <input
          type="text"
          value={asText(value)}
          placeholder={spec.placeholder}
          onChange={(event) => onChange(event.target.value)}
          className="w-full rounded-lg bg-ink-800 px-2.5 py-1.5 text-[12px] text-white/90 ring-1 ring-white/10 outline-none focus:ring-accent/60"
        />
      );
  }
}
