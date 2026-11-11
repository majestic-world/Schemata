import { create } from "zustand";
import { MOD } from "../lib/platform";
import { notify } from "./toasts";
import { openTransfer, type TableRef } from "./transfer";

/** Table copied in the tree or object list (Navicat-style), waiting to be pasted into a database. */
export const useTableClipboard = create<{ copied: TableRef | null }>(() => ({ copied: null }));

export function copyTable(table: TableRef) {
  useTableClipboard.setState({ copied: table });
  notify("Table copied", `Select a database and press ${MOD}+V to paste ${table.name} with its data.`);
}

/** Copies the clipboard table (structure and rows) into `target`. Returns false when nothing was copied. */
export function pasteTable(target: { connectionId: string; database: string }): boolean {
  const source = useTableClipboard.getState().copied;
  if (!source) return false;
  openTransfer({ kind: "copyTable", source, target });
  return true;
}
