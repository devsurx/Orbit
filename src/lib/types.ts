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
