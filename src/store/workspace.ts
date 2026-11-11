import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { ObjectKind } from "../api/types";

export type SelectedObject = { name: string; kind: ObjectKind };

export type Selection = {
  connectionId: string;
  database?: string;
  object?: SelectedObject;
};

type WorkspaceState = {
  selection: Selection | null;
  /** Same database as selection; the first object is the primary object shown by the inspector. */
  selectedObjects: SelectedObject[];
  search: string;
  /** Expanded tree nodes, keyed by node path. */
  expanded: Record<string, boolean>;
  /** Connections that were open, reopened on the next start. */
  openAtExit: string[];
  setOpenAtExit: (ids: string[]) => void;
  select: (s: Selection | null) => void;
  selectObjects: (selection: { connectionId: string; database: string; objects: SelectedObject[] }) => void;
  setSearch: (q: string) => void;
  setExpanded: (key: string, open: boolean) => void;
  collapseAll: () => void;
};

export const useWorkspace = create<WorkspaceState>()(
  persist(
    (set) => ({
      selection: null,
      selectedObjects: [],
      search: "",
      expanded: {},
      openAtExit: [],
      setOpenAtExit: (openAtExit) => set({ openAtExit }),
      select: (selection) => set({ selection, selectedObjects: selection?.object ? [selection.object] : [] }),
      selectObjects: ({ connectionId, database, objects }) => set({
        selection: { connectionId, database, object: objects[0] },
        selectedObjects: objects,
      }),
      setSearch: (search) => set({ search }),
      setExpanded: (key, open) => set((s) => ({ expanded: { ...s.expanded, [key]: open } })),
      collapseAll: () => set({ expanded: {} }),
    }),
    { name: "schematadb.workspace", partialize: (s) => ({ expanded: s.expanded, openAtExit: s.openAtExit }) },
  ),
);
