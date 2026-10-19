// Mirrors the `orbit-registry` menu model exactly (serde, snake_case).
// If a field is added on the Rust side, it belongs here too.

export type Kind =
  | "image"
  | "video"
  | "audio"
  | "document"
  | "pdf"
  | "text"
  | "data"
  | "other";

export type PetalKind = "conversion" | "action" | "preset" | "operator";

/** Widget kind for an option, mirroring `orbit_registry::OptionKind`. */
export type OptionKind =
  | "enum"
  | "multi_enum"
  | "range"
  | "int"
  | "number"
  | "bool"
  | "text"
  | "bytes"
  | "duration"
  | "time"
  | "time_range"
  | "region"
  | "path"
  | "format"
  | "percent";

/** Bisection settings for size-targeted compression. */
export interface SearchSpec {
  params: string[];
  max_attempts: number;
}

/**
 * One entry of the registry's option catalog.
 *
 * The panel renders these directly, so a field's label, range, allowed values
 * and visibility rule are the registry's rather than a copy kept in the UI.
 */
export interface OptionSpec {
  key: string;
  label: string;
  type: OptionKind;
  values?: unknown[];
  labels?: Record<string, string>;
  min?: number;
  max?: number;
  default?: unknown;
  unit?: string;
  /** Only show this field when every listed key already equals the value. */
  show_if?: Record<string, unknown>;
  placeholder?: string;
  help?: string;
  search?: SearchSpec;
}

export interface Disabled {
  code: string;
  reason: string;
  hint?: string;
}

export interface ChainStep {
  action: string;
  options: Record<string, unknown>;
}

export interface Petal {
  id: string;
  label: string;
  icon: string;
  kind: PetalKind;
  reference: string;
  engine?: string;
  options?: string[];
  defaults?: Record<string, unknown>;
  steps?: ChainStep[];
  badge?: string;
  disabled?: Disabled;
  score: number;
  coverage: number;
  total: number;
  needs_options: boolean;
}

export interface CategoryView {
  id: string;
  label: string;
  icon: string;
  accent?: string;
  order: number;
  petals: Petal[];
  hidden: number;
}

export interface FileSummary {
  name: string;
  path?: string;
  format?: string;
  format_label?: string;
  kind: Kind;
  ext: string;
  size_bytes: number;
}

export interface MenuModel {
  files: FileSummary[];
  categories: CategoryView[];
  candidates: number;
  shared_kind?: Kind;
}

export interface DispatchAck {
  ok: boolean;
  message: string;
  detail?: string;
}

/** The global shortcut as the shell has it, mirroring `settings::HotkeyStatus`. */
export interface HotkeyStatus {
  /** The accelerator in force, or null when the shortcut is off. */
  accelerator: string | null;
  /** The same thing written the way a person would say it. */
  label: string;
  /** Whether the registration actually took. */
  registered: boolean;
  /** One line explaining a registration that cannot work, if any. */
  note?: string;
}

/**
 * One state change of a queued job, mirroring `orbit_jobs::JobEvent`.
 *
 * The Rust side flattens the state enum under a `state` tag, so the terminal
 * and progress fields sit beside the id rather than nested.
 */
export type JobState =
  | { state: "queued" }
  | { state: "running"; percent?: number }
  | { state: "done"; path: string }
  | { state: "failed"; message: string }
  | { state: "cancelled" };

export type JobEvent = { id: number; label: string } & JobState;
