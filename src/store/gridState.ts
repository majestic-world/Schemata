import { create } from "zustand";
import type { Filter, Page, SortSpec, TableInfo } from "../api/types";
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
  /** The page the pending edits refer to. Pending edits address rows by position, so while there are
   * any the grid keeps showing (and keys commits by) this snapshot instead of refetched data. */
  snapshot: Page | null;
  snapshotInfo: TableInfo | null;
  /** Locks the tab, including unmounted grids, until its submitted change set settles. */
  committing: boolean;
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
  snapshot: null,
  snapshotInfo: null,
  committing: false,
  failedRow: null,
});

type Store = {
  states: Record<string, GridState>;
  update: (tabId: string, f: (s: GridState) => Partial<GridState>) => void;
  beginCommit: (tabId: string) => GridState | null;
  finishCommit: (tabId: string, submitted: Pending, saved: boolean, failedRow?: RowRef | null) => void;
  /** Drops the tab's pending edits and their frozen page; false (nothing dropped) while a commit is running. */
  discard: (tabId: string) => boolean;
  forget: (tabId: string) => void;
};

export const useGridStates = create<Store>((set, get) => ({
  states: {},
  update: (tabId, f) => set((s) => {
    const current = s.states[tabId] ?? initial();
    if (current.committing) return s;
    return { states: { ...s.states, [tabId]: { ...current, ...f(current) } } };
  }),
  beginCommit: (tabId) => {
    const current = get().states[tabId];
    if (!current || current.committing || !isDirty(current.pending)) return null;
    set((s) => ({ states: { ...s.states, [tabId]: { ...current, committing: true } } }));
    return current;
  },
  finishCommit: (tabId, submitted, saved, failedRow = null) => set((s) => {
    const current = s.states[tabId];
    // A response for a closed/recreated tab must never erase another change set.
    if (!current?.committing || current.pending !== submitted) return s;
    return {
      states: {
        ...s.states,
        [tabId]: {
          ...current,
          committing: false,
          failedRow: saved ? null : failedRow,
          ...(saved ? { pending: emptyPending(), snapshot: null, snapshotInfo: null } : {}),
        },
      },
    };
  }),
  discard: (tabId) => {
    const current = get().states[tabId];
    if (current?.committing) return false;
    if (current) set((s) => ({ states: { ...s.states, [tabId]: { ...current, pending: emptyPending(), snapshot: null, snapshotInfo: null, failedRow: null } } }));
    return true;
  },
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

export type CommitOutcome = { saved: boolean; failedRow: RowRef | null };

/**
 * Locks the tab and submits its pending edits through `save`. The tab is unlocked when `save` settles, also when it
 * throws (e.g. while building the change set), which then counts as not saved and is rethrown.
 */
export async function commitPending(tabId: string, save: (submitted: GridState) => Promise<CommitOutcome>): Promise<void> {
  const { beginCommit, finishCommit } = useGridStates.getState();
  const submitted = beginCommit(tabId);
  if (!submitted) return;
  let outcome: CommitOutcome = { saved: false, failedRow: null };
  try {
    outcome = await save(submitted);
  } finally {
    finishCommit(tabId, submitted.pending, outcome.saved, outcome.failedRow);
  }
}
