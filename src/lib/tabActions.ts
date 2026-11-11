import { confirmAction, useConfirm } from "../store/confirm";
import { hasDesignerEdits, useDesignerDrafts } from "../store/designerDrafts";
import { hasPendingEdits, useGridStates } from "../store/gridState";
import { useOperationActivity } from "../store/operationActivity";
import { useQueryResults } from "../store/queryResults";
import { useTabs, type Tab } from "../store/tabs";
import { notify } from "../store/toasts";

export function isTabDirty(tab: Tab) {
  return hasPendingEdits(tab.id) || hasDesignerEdits(tab.id) || tab.kind === "newTable" || (tab.kind === "query" && tab.sql !== tab.savedSql);
}

export function isTabBusy(tab: Tab) {
  return !!useGridStates.getState().states[tab.id]?.committing || !!useDesignerDrafts.getState().drafts[tab.id]?.busy || !!useQueryResults.getState().runs[tab.id]?.running;
}

let confirmingTabs = false;

/** Confirms without closing anything, so object mutations can wait for success before discarding tabs. */
export async function confirmDiscardTabs(ids: readonly string[]): Promise<boolean> {
  if (confirmingTabs || useConfirm.getState().request) return false;
  confirmingTabs = true;
  try {
    const selected = new Set(ids);
    while (true) {
      const tabs = useTabs.getState().tabs.filter((tab) => selected.has(tab.id));
      if (tabs.some(isTabBusy)) {
        notify("Operation in progress", "Wait for this tab's query or save to finish before closing or renaming it.");
        return false;
      }
      const dirty = tabs.filter(isTabDirty);
      if (dirty.length === 0) return true;
      const snapshot = tabSnapshot(tabs);
      if (!(await confirmAction({
        title: "Discard changes",
        message: tabs.length === 1
          ? "This tab has unsaved changes. Continue and discard them?"
          : "Some selected tabs have unsaved changes. Continue and discard those changes?",
        confirmLabel: "Discard",
        danger: true,
      }))) return false;
      const current = useTabs.getState().tabs.filter((tab) => selected.has(tab.id));
      if (tabSnapshot(current) === snapshot && !current.some(isTabBusy)) return true;
    }
  } finally {
    confirmingTabs = false;
  }
}

/** Confirm before closing any tab in a group, so cancelling never partially closes it. */
export async function requestCloseTabs(ids: readonly string[]) {
  if (typeof document !== "undefined" && document.activeElement instanceof HTMLElement) document.activeElement.blur();
  if (!(await confirmDiscardTabs(ids))) return;
  const selected = new Set(ids);
  const { tabs, close } = useTabs.getState();
  if (tabs.some((tab) => selected.has(tab.id) && isTabBusy(tab))) return;
  for (const tab of tabs) if (selected.has(tab.id)) close(tab.id);
}

function tabSnapshot(tabs: readonly Tab[]) {
  return JSON.stringify(tabs.map((tab) => ({
    tab,
    pending: useGridStates.getState().states[tab.id]?.pending,
    committing: useGridStates.getState().states[tab.id]?.committing,
    designer: useDesignerDrafts.getState().drafts[tab.id],
  })));
}

function appSnapshot() {
  return JSON.stringify({
    tabs: tabSnapshot(useTabs.getState().tabs),
    queries: Object.entries(useQueryResults.getState().runs).filter(([, run]) => run?.running).map(([id, run]) => [id, run?.executionId]),
    operations: [...useOperationActivity.getState().active],
  });
}

let closingApp = false;

/** All native close paths share this guard. Approval never clears stores before the native close succeeds. */
export async function requestAppClose(close: () => Promise<void>): Promise<boolean> {
  if (closingApp || confirmingTabs || useConfirm.getState().request) return false;
  closingApp = true;
  try {
    while (true) {
      if (Object.values(useGridStates.getState().states).some((state) => state.committing) ||
          Object.values(useDesignerDrafts.getState().drafts).some((draft) => draft?.busy)) {
        notify("Save in progress", "Wait for the pending save or table alteration to finish before closing Schemata.");
        return false;
      }
      const tabs = useTabs.getState().tabs;
      const dirty = tabs.some(isTabDirty);
      const running = Object.values(useQueryResults.getState().runs).some((run) => run?.running) || useOperationActivity.getState().active.size > 0;
      const snapshot = appSnapshot();
      if ((dirty || running) && !(await confirmAction({
        title: "Close Schemata",
        message: [dirty ? "Unsaved SQL, grid or table-design changes will be discarded." : "", running ? "Queries or transfer operations are still running and will be interrupted. Completed database changes cannot be undone." : "", "Close Schemata?"].filter(Boolean).join(" "),
        confirmLabel: "Close Schemata",
        danger: true,
      }))) return false;
      const current = appSnapshot();
      if (Object.values(useGridStates.getState().states).some((state) => state.committing) ||
          Object.values(useDesignerDrafts.getState().drafts).some((draft) => draft?.busy)) continue;
      if (snapshot !== current) continue;
      await close();
      return true;
    }
  } finally {
    closingApp = false;
  }
}
