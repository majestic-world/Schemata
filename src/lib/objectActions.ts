import type { QueryClient } from "@tanstack/react-query";
import { api } from "../api/client";
import type { ObjectKind } from "../api/types";
import { confirmAction, promptAction } from "../store/confirm";
import { closeTabsFor, useTabs } from "../store/tabs";
import { notify, notifyError } from "../store/toasts";
import { useWorkspace } from "../store/workspace";

export type ObjectRef = { connectionId: string; database: string; name: string; kind: ObjectKind };

function refreshObjects(qc: QueryClient, o: { connectionId: string; database: string }) {
  void qc.invalidateQueries({ queryKey: ["objects", o.connectionId, o.database] });
  void qc.invalidateQueries({ queryKey: ["tableInfo", o.connectionId, o.database] });
  void qc.invalidateQueries({ queryKey: ["page", o.connectionId, o.database] });
  void qc.invalidateQueries({ queryKey: ["count", o.connectionId, o.database] });
}

export async function dropObject(o: ObjectRef, qc: QueryClient) {
  const what = o.kind === "view" ? "view" : "table";
  const ok = await confirmAction({
    title: `Delete ${what}`,
    message: `Delete the ${what} "${o.name}"${o.kind === "table" ? " and all of its data" : ""}? This cannot be undone.`,
    confirmLabel: `Delete ${what}`,
    danger: true,
  });
  if (!ok) return;
  try {
    await api.dropObject(o.connectionId, o.database, o.name, o.kind);
    closeTabsFor(o.connectionId, o.database, o.name);
    useWorkspace.getState().select({ connectionId: o.connectionId, database: o.database });
    refreshObjects(qc, o);
    notify(`Deleted ${what}`, o.name);
  } catch (e) {
    notifyError(`Could not delete ${o.name}`, e);
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
