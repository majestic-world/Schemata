import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { ObjectKind } from "../api/types";

export type Selection = {
  connectionId: string;
  database?: string;
  object?: { name: string; kind: ObjectKind };
};

type WorkspaceState = {
  selection: Selection | null;
  search: string;
  /** Expanded tree nodes, keyed by node path. */
  expanded: Record<string, boolean>;
  /** Connections that were open, reopened on the next start. */
  openAtExit: string[];
  setOpenAtExit: (ids: string[]) => void;
  select: (s: Selection | null) => void;
  setSearch: (q: string) => void;
  setExpanded: (key: string, open: boolean) => void;
};

export const useWorkspace = create<WorkspaceState>()(
  persist(
    (set) => ({
      selection: null,
      search: "",
      expanded: {},
      openAtExit: [],
      setOpenAtExit: (openAtExit) => set({ openAtExit }),
      select: (selection) => set({ selection }),
      setSearch: (search) => set({ search }),
      setExpanded: (key, open) => set((s) => ({ expanded: { ...s.expanded, [key]: open } })),
    }),
    { name: "schematadb.workspace", partialize: (s) => ({ expanded: s.expanded, openAtExit: s.openAtExit }) },
  ),
);
