import type { QueryClient } from "@tanstack/react-query";
import { api } from "../api/client";
import { ENGINE_LABEL, type Engine, type IncomingForeignKey, type ObjectKind } from "../api/types";
import { confirmAction, promptAction } from "../store/confirm";
import { closeTabsFor, useTabs } from "../store/tabs";
import { notify, notifyError } from "../store/toasts";
import { useWorkspace } from "../store/workspace";
import { useFavorites } from "../store/favorites";
import { plural } from "./format";
import { confirmDiscardTabs } from "./tabActions";

export type ObjectRef = { connectionId: string; database: string; name: string; kind: ObjectKind };

function refreshObjects(qc: QueryClient, o: { connectionId: string; database: string }) {
  void qc.invalidateQueries({ queryKey: ["objects", o.connectionId, o.database] });
  void qc.invalidateQueries({ queryKey: ["tableInfo", o.connectionId, o.database] });
  void qc.invalidateQueries({ queryKey: ["page", o.connectionId, o.database] });
  void qc.invalidateQueries({ queryKey: ["count", o.connectionId, o.database] });
}

/** Removing a table's rows: dropping it, emptying it (`DELETE`) or truncating it (MySQL `TRUNCATE`). */
type Removal = "drop" | "empty" | "truncate";

const REMOVAL_VERB: Record<Removal, { ing: string; base: string }> = {
  drop: { ing: "deleting", base: "delete" },
  empty: { ing: "emptying", base: "empty" },
  truncate: { ing: "truncating", base: "truncate" },
};

/**
 * What removing every row of `table` does to the tables whose foreign keys reference it, for the confirmation; empty
 * when none does. Deleting rows (SQLite always, MySQL and MariaDB when emptying) applies their ON DELETE actions (rows
 * deleted or set to NULL). MySQL and MariaDB refuse to drop a referenced table, so a drop may fail (it succeeds once
 * the referencing tables are dropped first), and always refuse to truncate one.
 */
export function referencesNotice(engine: Engine | null, table: string, refs: readonly IncomingForeignKey[], removal: Removal): string {
  if (refs.length === 0) return "";
  const verb = REMOVAL_VERB[removal];
  const action = (r: IncomingForeignKey) => `ON DELETE ${r.onDelete.toUpperCase()}`;
  if (engine !== "sqlite" && removal !== "empty") {
    const list = refs.map((r) => `"${r.table}" (${action(r)})`).join(", ");
    const outcome = engine === null
      ? `Depending on the database, rows in ${refs.length === 1 ? "that table" : "those tables"} may be deleted or set to NULL, or ${verb.ing} "${table}" may fail.`
      : removal === "truncate"
        ? `${ENGINE_LABEL[engine]} refuses to truncate a table other tables reference, so this will fail; use Empty Table instead.`
        : `${ENGINE_LABEL[engine]} refuses to ${verb.base} a table other tables reference, so this may fail.`;
    return `Referenced by ${list}. ${outcome}`;
  }
  const effects = refs.map((r) => {
    switch (r.onDelete.toUpperCase()) {
      case "CASCADE":
        return `rows in "${r.table}" that reference it will be deleted (${action(r)})`;
      case "SET NULL":
        return `references to it in "${r.table}" will be set to NULL (${action(r)})`;
      case "SET DEFAULT":
        return `references to it in "${r.table}" will be set to their default (${action(r)})`;
      default:
        return `${verb.ing} it fails while rows in "${r.table}" reference it (${action(r)})`;
    }
  });
  return `Other tables reference "${table}": ${effects.join("; ")}.`;
}

/** Engine of a connection, from the cached connection list; null when it cannot be told. */
async function engineOf(qc: QueryClient, connectionId: string): Promise<Engine | null> {
  try {
    const connections = await qc.ensureQueryData({ queryKey: ["connections"], queryFn: api.listConnections });
    return connections.find((c) => c.id === connectionId)?.engine ?? null;
  } catch {
    return null;
  }
}

/** The confirmation notes about tables referencing each of `tables`; a failed lookup is said rather than hidden. */
async function referencesNotes(qc: QueryClient, scope: { connectionId: string; database: string }, tables: readonly string[], removal: Removal): Promise<string> {
  if (tables.length === 0) return "";
  const engine = await engineOf(qc, scope.connectionId);
  const notes = await Promise.all(
    tables.map(async (table) => {
      try {
        return referencesNotice(engine, table, await api.incomingForeignKeys(scope.connectionId, scope.database, table), removal);
      } catch (e) {
        return `Could not check which tables reference "${table}": ${e && typeof e === "object" && "message" in e ? String(e.message) : String(e)}.`;
      }
    }),
  );
  return notes.filter(Boolean).map((n) => ` ${n}`).join("");
}

export async function dropObjects(scope: Pick<ObjectRef, "connectionId" | "database">, objects: readonly Pick<ObjectRef, "name" | "kind">[], qc: QueryClient) {
  if (objects.length === 0) return;
  const noun = objects.every((o) => o.kind === "table") ? "table" : objects.every((o) => o.kind === "view") ? "view" : "object";
  const what = objects.length === 1 ? noun : plural(objects.length, noun);
  const references = await referencesNotes(qc, scope, objects.filter((o) => o.kind === "table").map((o) => o.name), "drop");
  const ok = await confirmAction({
    title: `Delete ${what}`,
    message: `Delete ${objects.length === 1 ? `the ${noun}` : what} ${objects.map((o) => `"${o.name}"`).join(", ")}${objects.some((o) => o.kind === "table") ? " and all contained data" : ""}? This cannot be undone.${references}`,
    confirmLabel: `Delete ${what}`,
    danger: true,
  });
  if (!ok) return;

  let deleted = 0;
  try {
    for (const object of objects) {
      await api.dropObject(scope.connectionId, scope.database, object.name, object.kind);
      closeTabsFor(scope.connectionId, scope.database, object.name);
      useFavorites.getState().remove([{ ...scope, name: object.name }]);
      deleted++;
    }
    notify(`Deleted ${what}`, objects.map((o) => o.name).join(", "));
  } catch (error) {
    notifyError(`Could not delete ${objects[deleted].name}`, error);
    if (deleted > 0) notify(`Deleted ${deleted} of ${plural(objects.length, noun)}`);
  } finally {
    if (deleted > 0) {
      const workspace = useWorkspace.getState();
      if (workspace.selection?.connectionId === scope.connectionId && workspace.selection.database === scope.database) {
        workspace.selectObjects({ ...scope, objects: objects.slice(deleted) });
      }
      refreshObjects(qc, scope);
    }
  }
}

/** Empty Table: deletes every row with `DELETE`, keeping the structure and the autoincrement counter. */
export async function emptyTable(o: ObjectRef, qc: QueryClient) {
  const references = await referencesNotes(qc, o, [o.name], "empty");
  const ok = await confirmAction({
    title: "Empty table",
    message: `Delete every row of "${o.name}"? The structure and the auto-increment counter are kept. This cannot be undone.${references}`,
    confirmLabel: "Empty Table",
    danger: true,
  });
  if (!ok) return;
  try {
    await api.emptyTable(o.connectionId, o.database, o.name);
    refreshObjects(qc, o);
    notify("Table emptied", o.name);
  } catch (e) {
    notifyError(`Could not empty ${o.name}`, e);
  }
}

/** Truncate Table: deletes every row and resets the autoincrement counter, keeping the structure. */
export async function truncateTable(o: ObjectRef, qc: QueryClient) {
  const references = await referencesNotes(qc, o, [o.name], "truncate");
  const ok = await confirmAction({
    title: "Truncate table",
    message: `Delete every row of "${o.name}" and reset its auto-increment counter? The structure is kept. This cannot be undone.${references}`,
    confirmLabel: "Truncate Table",
    danger: true,
  });
  if (!ok) return;
  try {
    await api.truncateTable(o.connectionId, o.database, o.name);
    refreshObjects(qc, o);
    notify("Table truncated", o.name);
  } catch (e) {
    notifyError(`Could not truncate ${o.name}`, e);
  }
}

export async function renameTable(o: ObjectRef, qc: QueryClient) {
  const to = await promptAction({ title: "Rename table", message: "", confirmLabel: "Rename", input: { label: "New name", initial: o.name } });
  if (!to || to === o.name) return;
  const tabs = useTabs.getState().tabs.filter((tab) => tab.kind === "table" && tab.connectionId === o.connectionId && tab.database === o.database && tab.name === o.name);
  if (!(await confirmDiscardTabs(tabs.map((tab) => tab.id)))) return;
  // Discard only after approval, before the asynchronous rename can race with new edits in these tabs.
  for (const tab of tabs) useTabs.getState().close(tab.id);
  try {
    await api.renameTable(o.connectionId, o.database, o.name, to);
    useFavorites.getState().rename(o, to);
    useWorkspace.getState().select({ connectionId: o.connectionId, database: o.database, object: { name: to, kind: o.kind } });
    refreshObjects(qc, o);
    notify("Table renamed", `${o.name} → ${to}`);
  } catch (e) {
    notifyError(`Could not rename ${o.name}`, e);
  }
}

export async function copyCreateStatement(o: ObjectRef) {
  try {
    const info = await api.tableInfo(o.connectionId, o.database, o.name);
    await navigator.clipboard.writeText(info.createSql);
    notify("CREATE statement copied");
  } catch (e) {
    notifyError("Could not copy the CREATE statement", e);
  }
}

export const designTable = (o: ObjectRef) =>
  useTabs.getState().openTable({ connectionId: o.connectionId, database: o.database, name: o.name, objectKind: o.kind }, "structure");
