import { create } from "zustand";
import { api } from "../api/client";
import type { ObjectKind } from "../api/types";
import { useGridStates } from "./gridState";
import { useDesignerDrafts } from "./designerDrafts";
import { useQueryResults } from "./queryResults";
import { useWorkspace } from "./workspace";

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

/** Designer for a table that does not exist yet. */
export type NewTableTab = { kind: "newTable"; id: string; connectionId: string; database: string };

export type QueryTab = {
  kind: "query";
  id: string;
  connectionId: string;
  database: string | null;
  title: string;
  filePath: string | null;
  sql: string;
  /** Text as last saved/opened; differs from `sql` when the tab has unsaved changes. */
  savedSql: string;
};

export type Tab = TableTab | NewTableTab | QueryTab;

export const OBJECTS_TAB = "objects";

type TabsState = {
  tabs: Tab[];
  active: string;
  /** Total row count per table tab, shown in the status bar. */
  rowCounts: Record<string, number | undefined>;
  openTable: (t: Omit<TableTab, "kind" | "id" | "view">, view?: TableView) => void;
  newTable: (connectionId: string, database: string) => void;
  newQuery: (q: { connectionId: string; database: string | null; sql?: string; title?: string; filePath?: string | null }) => void;
  updateQuery: (id: string, p: Partial<Omit<QueryTab, "kind" | "id">>) => void;
  /** Replaces a tab in place (e.g. a new-table designer by the created table). */
  replace: (id: string, t: Omit<TableTab, "kind" | "id" | "view">, view: TableView) => void;
  close: (id: string) => void;
  activate: (id: string) => void;
  setView: (id: string, view: TableView) => void;
  setRowCount: (id: string, n: number | undefined) => void;
};

/** Each part is encoded, so names containing `/` (`a/b` + `c` vs `a` + `b/c`) never share an id. */
export const tableTabId = (connectionId: string, database: string, name: string) =>
  `table:${[connectionId, database, name].map(encodeURIComponent).join("/")}`;

/** Table-tab activation updates the tree, inspector and object actions to the same object. */
function selectTable(tab: Tab | undefined) {
  if (tab?.kind !== "table") return;
  useWorkspace.getState().select({
    connectionId: tab.connectionId,
    database: tab.database,
    object: { name: tab.name, kind: tab.objectKind },
  });
}

export const useTabs = create<TabsState>((set, get) => ({
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
      selectTable(tabs.find((tab) => tab.id === id));
      return { tabs, active: id };
    }),
  newTable: (connectionId, database) =>
    set((s) => {
      const id = `new:${crypto.randomUUID()}`;
      return { tabs: [...s.tabs, { kind: "newTable", id, connectionId, database }], active: id };
    }),
  newQuery: (q) =>
    set((s) => {
      const id = `query:${crypto.randomUUID()}`;
      const n = s.tabs.filter((t) => t.kind === "query").length + 1;
      const tab: QueryTab = {
        kind: "query",
        id,
        connectionId: q.connectionId,
        database: q.database,
        title: q.title ?? `Untitled Query ${n}`,
        filePath: q.filePath ?? null,
        sql: q.sql ?? "",
        savedSql: q.filePath ? (q.sql ?? "") : "",
      };
      return { tabs: [...s.tabs, tab], active: id };
    }),
  updateQuery: (id, p) => set((s) => ({ tabs: s.tabs.map((t) => (t.id === id && t.kind === "query" ? { ...t, ...p } : t)) })),
  replace: (id, t, view) =>
    set((s) => {
      useDesignerDrafts.getState().forget(id);
      useGridStates.getState().forget(id);
      const nid = tableTabId(t.connectionId, t.database, t.name);
      const tab: TableTab = { ...t, kind: "table", id: nid, view };
      selectTable(tab);
      return { tabs: s.tabs.filter((x) => x.id !== nid).map((x) => (x.id === id ? tab : x)), active: nid };
    }),
  close: (id) => {
    const closing = get().tabs.find((t) => t.id === id);
    if (closing?.kind === "query") {
      useQueryResults.getState().set(id, undefined);
      // Best-effort: the server rolls back the tab's open transaction; a lost connection has no session left.
      void api.closeEditorSession(closing.connectionId, id).catch(() => undefined);
    }
    set((s) => {
      useGridStates.getState().forget(id);
      useDesignerDrafts.getState().forget(id);
      const idx = s.tabs.findIndex((t) => t.id === id);
      const tabs = s.tabs.filter((t) => t.id !== id);
      const active = s.active !== id ? s.active : (tabs[idx] ?? tabs[idx - 1])?.id ?? OBJECTS_TAB;
      if (s.active === id) selectTable(tabs.find((tab) => tab.id === active));
      const rowCounts = { ...s.rowCounts };
      delete rowCounts[id];
      return { tabs, active, rowCounts };
    });
  },
  activate: (active) => set((s) => {
    selectTable(s.tabs.find((tab) => tab.id === active));
    return { active };
  }),
  setView: (id, view) => set((s) => ({ tabs: s.tabs.map((t) => (t.id === id && t.kind === "table" ? { ...t, view } : t)) })),
  setRowCount: (id, n) => set((s) => ({ rowCounts: { ...s.rowCounts, [id]: n } })),
}));

/** Closes every tab that belongs to a connection (and optionally a database or table). */
export function closeTabsFor(connectionId: string, database?: string, name?: string) {
  const { tabs, close } = useTabs.getState();
  tabs
    .filter((t) => t.connectionId === connectionId && (!database || t.database === database) && (!name || (t.kind === "table" && t.name === name)))
    .forEach((t) => close(t.id));
}
