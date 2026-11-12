import type { QueryClient } from "@tanstack/react-query";
import { api } from "../api/client";
import type { SavedQuery } from "../api/types";
import { confirmAction, promptAction } from "../store/confirm";
import { useTabs, type QueryTab } from "../store/tabs";
import { notify, notifyError } from "../store/toasts";

export const savedQueriesKey = (connectionId: string) => ["savedQueries", connectionId] as const;

const fileName = (p: string) => p.split(/[\\/]/).pop() ?? p;

/**
 * Saves a query tab without asking where: a tab opened from a `.sql` file writes back to it; any other tab is kept
 * in Schemata's saved queries (named on its first save) for its connection and database.
 */
export async function saveQueryTab(tab: QueryTab, qc: QueryClient) {
  const sql = tab.sql;
  const update = useTabs.getState().updateQuery;
  if (tab.filePath) {
    try {
      await api.writeTextFile(tab.filePath, sql);
      update(tab.id, { savedSql: sql });
      notify("Saved", fileName(tab.filePath));
    } catch (e) {
      notifyError("Could not save the file", e);
    }
    return;
  }
  const name = tab.savedQueryId ? tab.title : await promptAction({ title: "Save query", message: "", confirmLabel: "Save", input: { label: "Query name", initial: "" } });
  if (name === null) return;
  try {
    const saved = await api.saveQuery({ id: tab.savedQueryId, connectionId: tab.connectionId, database: tab.database, name, sql });
    update(tab.id, { savedQueryId: saved.id, title: saved.name, savedSql: sql });
    await qc.invalidateQueries({ queryKey: savedQueriesKey(tab.connectionId) });
    notify("Query saved", saved.name);
  } catch (e) {
    notifyError("Could not save the query", e);
  }
}

export async function renameSavedQuery(query: SavedQuery, qc: QueryClient) {
  const name = await promptAction({ title: "Rename query", message: "", confirmLabel: "Rename", input: { label: "New name", initial: query.name } });
  if (name === null || name.trim() === query.name) return;
  try {
    // The stored SQL is kept: unsaved edits in an open tab stay unsaved.
    const saved = await api.saveQuery({ id: query.id, connectionId: query.connectionId, database: query.database, name, sql: query.sql });
    const { tabs, updateQuery } = useTabs.getState();
    for (const tab of tabs) if (tab.kind === "query" && tab.savedQueryId === query.id) updateQuery(tab.id, { title: saved.name });
    await qc.invalidateQueries({ queryKey: savedQueriesKey(query.connectionId) });
  } catch (e) {
    notifyError(`Could not rename ${query.name}`, e);
  }
}

export async function deleteSavedQuery(query: SavedQuery, qc: QueryClient) {
  if (!(await confirmAction({ title: "Delete query", message: `Delete the saved query "${query.name}"? This cannot be undone.`, confirmLabel: "Delete", danger: true }))) return;
  try {
    await api.deleteSavedQuery(query.id);
    // An open tab keeps its text as an unsaved query.
    const { tabs, updateQuery } = useTabs.getState();
    for (const tab of tabs) if (tab.kind === "query" && tab.savedQueryId === query.id) updateQuery(tab.id, { savedQueryId: null, savedSql: "" });
    await qc.invalidateQueries({ queryKey: savedQueriesKey(query.connectionId) });
    notify("Query deleted", query.name);
  } catch (e) {
    notifyError(`Could not delete ${query.name}`, e);
  }
}
