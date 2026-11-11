import { create } from "zustand";
import type { AppError, ScriptResult } from "../api/types";

export type QueryRun = {
  result: ScriptResult | null;
  /** Error that prevented the script from running at all (e.g. connection lost). */
  failure: AppError | null;
  running: boolean;
  executionId: string | null;
  startedAt: number;
  finishedAt: number | null;
};

type QueryResultsState = {
  runs: Record<string, QueryRun | undefined>;
  set: (tabId: string, run: QueryRun | undefined) => void;
  patch: (tabId: string, p: Partial<QueryRun>) => void;
};

export const useQueryResults = create<QueryResultsState>((set) => ({
  runs: {},
  set: (tabId, run) => set((s) => ({ runs: { ...s.runs, [tabId]: run } })),
  patch: (tabId, p) => set((s) => ({ runs: { ...s.runs, [tabId]: s.runs[tabId] ? { ...s.runs[tabId]!, ...p } : undefined } })),
}));
