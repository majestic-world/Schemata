import { create } from "zustand";
import type { AppError, ScriptResult } from "../api/types";

export type QueryRun = {
  readonly connectionId: string;
  readonly database: string | null;
  result: ScriptResult | null;
  /** Command-level failure; earlier statements may already have executed. */
  failure: AppError | null;
  running: boolean;
  readonly executionId: string;
  startedAt: number;
  finishedAt: number | null;
};

type QueryResultsState = {
  runs: Record<string, QueryRun | undefined>;
  set: (tabId: string, run: QueryRun | undefined) => void;
  patch: (tabId: string, p: Partial<Omit<QueryRun, "connectionId" | "database" | "executionId">>, executionId?: string) => void;
};

export const useQueryResults = create<QueryResultsState>((set) => ({
  runs: {},
  set: (tabId, run) => set((s) => ({ runs: { ...s.runs, [tabId]: run } })),
  patch: (tabId, p, executionId) => set((s) => {
    const run = s.runs[tabId];
    if (!run || (executionId !== undefined && run.executionId !== executionId)) return s;
    return { runs: { ...s.runs, [tabId]: { ...run, ...p } } };
  }),
}));
