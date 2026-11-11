import type { QueryClient } from "@tanstack/react-query";
import { api } from "../api/client";
import type { ObjectKind } from "../api/types";
import { confirmAction, promptAction } from "../store/confirm";
import { closeTabsFor, useTabs } from "../store/tabs";
import { notify, notifyError } from "../store/toasts";
import { useWorkspace } from "../store/workspace";
import { plural } from "./format";

export type ObjectRef = { connectionId: string; database: string; name: string; kind: ObjectKind };

function refreshObjects(qc: QueryClient, o: { connectionId: string; database: string }) {
  void qc.invalidateQueries({ queryKey: ["objects", o.connectionId, o.database] });
  void qc.invalidateQueries({ queryKey: ["tableInfo", o.connectionId, o.database] });
  void qc.invalidateQueries({ queryKey: ["page", o.connectionId, o.database] });
  void qc.invalidateQueries({ queryKey: ["count", o.connectionId, o.database] });
}

export async function dropObjects(scope: Pick<ObjectRef, "connectionId" | "database">, objects: readonly Pick<ObjectRef, "name" | "kind">[], qc: QueryClient) {
  if (objects.length === 0) return;
  const noun = objects.every((o) => o.kind === "table") ? "table" : objects.every((o) => o.kind === "view") ? "view" : "object";
  const what = objects.length === 1 ? noun : plural(objects.length, noun);
  const ok = await confirmAction({
    title: `Delete ${what}`,
    message: `Delete ${objects.length === 1 ? `the ${noun}` : what} ${objects.map((o) => `"${o.name}"`).join(", ")}${objects.some((o) => o.kind === "table") ? " and all contained data" : ""}? This cannot be undone.`,
    confirmLabel: `Delete ${what}`,
    danger: true,
  });
  if (!ok) return;

  let deleted = 0;
  try {
    for (const object of objects) {
      await api.dropObject(scope.connectionId, scope.database, object.name, object.kind);
      closeTabsFor(scope.connectionId, scope.database, object.name);
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

export async function truncateTable(o: ObjectRef, qc: QueryClient) {
  const ok = await confirmAction({
    title: "Empty table",
    message: `Delete every row of "${o.name}"? The structure is kept. This cannot be undone.`,
    confirmLabel: "Empty Table",
    danger: true,
  });
  if (!ok) return;
  try {
    await api.truncateTable(o.connectionId, o.database, o.name);
    refreshObjects(qc, o);
    notify("Table emptied", o.name);
  } catch (e) {
    notifyError(`Could not empty ${o.name}`, e);
  }
}

export async function renameTable(o: ObjectRef, qc: QueryClient) {
  const to = await promptAction({ title: "Rename table", message: "", confirmLabel: "Rename", input: { label: "New name", initial: o.name } });
  if (!to || to === o.name) return;
  try {
    await api.renameTable(o.connectionId, o.database, o.name, to);
    closeTabsFor(o.connectionId, o.database, o.name);
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
