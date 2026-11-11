import { create } from "zustand";
import type { Filter, SortSpec } from "../api/types";
import { emptyPending, isDirty, type Pending, type RowRef } from "../lib/pendingChanges";

export type DraftFilter = { column: string; op: Filter["op"]; value: string };

/** Paging, filtering and pending edits of one table tab; kept outside the component so switching tabs
 * or views does not lose uncommitted edits. */
export type GridState = {
  pageSize: number;
  offset: number;
  sort: SortSpec | null;
  filters: Filter[];
  drafts: DraftFilter[];
  showFilters: boolean;
  pending: Pending;
  failedRow: RowRef | null;
};

const initial = (): GridState => ({
  pageSize: 500,
  offset: 0,
  sort: null,
  filters: [],
  drafts: [],
  showFilters: false,
  pending: emptyPending(),
  failedRow: null,
});

type Store = {
  states: Record<string, GridState>;
  update: (tabId: string, f: (s: GridState) => Partial<GridState>) => void;
  forget: (tabId: string) => void;
};

export const useGridStates = create<Store>((set) => ({
  states: {},
  update: (tabId, f) => set((s) => {
    const current = s.states[tabId] ?? initial();
    return { states: { ...s.states, [tabId]: { ...current, ...f(current) } } };
  }),
  forget: (tabId) => set((s) => {
    const states = { ...s.states };
    delete states[tabId];
    return { states };
  }),
}));

export function useGridState(tabId: string): [GridState, (f: (s: GridState) => Partial<GridState>) => void] {
  const state = useGridStates((s) => s.states[tabId]) ?? initialState;
  const update = useGridStates((s) => s.update);
  return [state, (f) => update(tabId, f)];
}

const initialState = initial();

export const hasPendingEdits = (tabId: string) => {
  const s = useGridStates.getState().states[tabId];
  return !!s && isDirty(s.pending);
};
