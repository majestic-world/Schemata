import type { CsvPreview, CsvReadOptions } from "../api/types";
import { autoMap, type MappingDraft } from "./importMapping";

/** Identifies what a preview was read from: the file and every option that changes how it is parsed. */
export const csvSource = (path: string, options: CsvReadOptions): string =>
  JSON.stringify([path, options.delimiter, options.quote, options.hasHeader, options.encoding]);

/**
 * Preview of the CSV file being imported. Each read carries a request number; only the latest one may settle,
 * so a slow response for an earlier file or option set never replaces the current preview or its mapping.
 */
export type CsvPreviewState =
  | { status: "idle" }
  | { status: "loading"; request: number; source: string }
  | { status: "ready"; request: number; source: string; preview: CsvPreview; draft: MappingDraft }
  | { status: "failed"; request: number; source: string; error: string };

/** Starts a new read, discarding the previous preview and mapping. */
export const loadingPreview = (request: number, source: string): CsvPreviewState => ({ status: "loading", request, source });

/** Applies a read's result, unless a newer read (or a reset) superseded it. */
export function settlePreview(
  state: CsvPreviewState,
  request: number,
  outcome: { preview: CsvPreview; targets: string[] } | { error: string },
): CsvPreviewState {
  if (state.status !== "loading" || state.request !== request) return state;
  return "error" in outcome
    ? { status: "failed", request, source: state.source, error: outcome.error }
    : { status: "ready", request, source: state.source, preview: outcome.preview, draft: autoMap(outcome.preview.headers, outcome.targets) };
}

/** The preview and mapping, only when they were read from `source` (the file and options currently selected). */
export const previewFor = (state: CsvPreviewState, source: string) => (state.status === "ready" && state.source === source ? state : null);
