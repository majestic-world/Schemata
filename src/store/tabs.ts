import { create } from "zustand";
import type { ObjectKind } from "../api/types";

export type TableView = "data" | "structure" | "info";

export type TableTab = {
  kind: "table";
  id: string;
  connectionId: string;
  database: string;
  name: string;
  objectKind: ObjectKind;
  view: TableView;
};

export type Tab = TableTab;

export const OBJECTS_TAB = "objects";

type TabsState = {
  tabs: Tab[];
  active: string;
  /** Total row count per table tab, shown in the status bar. */
  rowCounts: Record<string, number | undefined>;
  openTable: (t: Omit<TableTab, "kind" | "id" | "view">, view?: TableView) => void;
  close: (id: string) => void;
  activate: (id: string) => void;
  setView: (id: string, view: TableView) => void;
  setRowCount: (id: string, n: number | undefined) => void;
};

export const tableTabId = (connectionId: string, database: string, name: string) => `table:${connectionId}/${database}/${name}`;

export const useTabs = create<TabsState>((set) => ({
  tabs: [],
  active: OBJECTS_TAB,
  rowCounts: {},
  openTable: (t, view = "data") =>
    set((s) => {
      const id = tableTabId(t.connectionId, t.database, t.name);
      const existing = s.tabs.find((x) => x.id === id);
      const tabs = existing
        ? s.tabs.map((x) => (x.id === id ? { ...x, view } : x))
        : [...s.tabs, { ...t, kind: "table" as const, id, view }];
      return { tabs, active: id };
    }),
  close: (id) =>
    set((s) => {
      const idx = s.tabs.findIndex((t) => t.id === id);
      const tabs = s.tabs.filter((t) => t.id !== id);
      const active = s.active !== id ? s.active : (tabs[idx] ?? tabs[idx - 1])?.id ?? OBJECTS_TAB;
      const rowCounts = { ...s.rowCounts };
      delete rowCounts[id];
      return { tabs, active, rowCounts };
    }),
  activate: (active) => set({ active }),
  setView: (id, view) => set((s) => ({ tabs: s.tabs.map((t) => (t.id === id ? { ...t, view } : t)) })),
  setRowCount: (id, n) => set((s) => ({ rowCounts: { ...s.rowCounts, [id]: n } })),
}));

/** Closes every tab that belongs to a connection (and optionally a database or table). */
export function closeTabsFor(connectionId: string, database?: string, name?: string) {
  const { tabs, close } = useTabs.getState();
  tabs
    .filter((t) => t.connectionId === connectionId && (!database || t.database === database) && (!name || t.name === name))
    .forEach((t) => close(t.id));
}
