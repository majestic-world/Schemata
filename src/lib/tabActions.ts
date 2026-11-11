import { confirmAction } from "../store/confirm";
import { hasPendingEdits } from "../store/gridState";
import { useTabs } from "../store/tabs";

/** Confirm before closing any tab in a group, so cancelling never partially closes it. */
export async function requestCloseTabs(ids: readonly string[]) {
  const selected = new Set(ids);
  const tabs = useTabs.getState().tabs.filter((tab) => selected.has(tab.id));
  if (tabs.length === 0) return;

  const dirty = tabs.some((tab) => hasPendingEdits(tab.id) || (tab.kind === "query" && tab.sql !== tab.savedSql));
  if (dirty && !(await confirmAction({
    title: "Discard changes",
    message: tabs.length === 1
      ? "This tab has unsaved changes. Close it and discard them?"
      : "Some selected tabs have unsaved changes. Close all selected tabs and discard those changes?",
    confirmLabel: "Discard",
    danger: true,
  }))) return;

  const { close } = useTabs.getState();
  for (const tab of tabs) close(tab.id);
}
