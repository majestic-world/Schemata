import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useConfirm } from "../store/confirm";
import { hasPendingEdits, useGridStates } from "../store/gridState";
import { OBJECTS_TAB, useTabs } from "../store/tabs";
import { useDesignerDrafts } from "../store/designerDrafts";
import { useOperationActivity } from "../store/operationActivity";
import { useQueryResults } from "../store/queryResults";
import { setCell } from "./pendingChanges";
import { requestAppClose, requestCloseTabs } from "./tabActions";

function reset() {
  useConfirm.getState().settle(null);
  useTabs.setState(useTabs.getInitialState(), true);
  useGridStates.setState(useGridStates.getInitialState(), true);
  useDesignerDrafts.setState(useDesignerDrafts.getInitialState(), true);
  useOperationActivity.setState(useOperationActivity.getInitialState(), true);
  useQueryResults.setState(useQueryResults.getInitialState(), true);
}

function table(name: string) {
  useTabs.getState().openTable({ connectionId: "connection", database: "main", name, objectKind: "table" });
  return useTabs.getState().active;
}

beforeEach(reset);
afterEach(reset);

describe("closing tabs", () => {
  it("closes only the requested clean tabs, retaining an unrelated unsaved query", async () => {
    const left = table("left");
    const right = table("right");
    useTabs.getState().newQuery({ connectionId: "connection", database: "main", sql: "SELECT 1" });
    const query = useTabs.getState().active;
    useTabs.getState().setRowCount(left, 12);
    useTabs.getState().setRowCount(right, 34);
    useTabs.getState().activate(left);

    await requestCloseTabs([left, right]);

    expect(useTabs.getState().tabs.map((tab) => tab.id)).toEqual([query]);
    expect(useTabs.getState().active).toBe(query);
    expect(useTabs.getState().rowCounts).toEqual({});
    expect(useConfirm.getState().request).toBeNull();
  });

  it("cancelling a group containing unsaved SQL keeps every tab and the active tab", async () => {
    const clean = table("clean");
    useTabs.getState().newQuery({ connectionId: "connection", database: "main", sql: "SELECT 42" });
    const query = useTabs.getState().active;
    const before = useTabs.getState().tabs;

    const closing = requestCloseTabs([clean, query]);
    expect(useConfirm.getState().request?.danger).toBe(true);
    useConfirm.getState().settle(null);
    await closing;

    expect(useTabs.getState().tabs).toEqual(before);
    expect(useTabs.getState().active).toBe(query);
  });

  it("confirms pending grid edits before discarding them and returns to Objects", async () => {
    const id = table("edited");
    useGridStates.getState().update(id, (state) => ({
      pending: setCell(state.pending, { kind: "existing", index: 0 }, "name", { type: "text", value: "new" }, { type: "text", value: "old" }),
    }));

    const closing = requestCloseTabs([id]);
    expect(hasPendingEdits(id)).toBe(true);
    expect(useTabs.getState().active).toBe(id);
    expect(useConfirm.getState().request?.danger).toBe(true);
    useConfirm.getState().settle("ok");
    await closing;

    expect(useTabs.getState().tabs).toEqual([]);
    expect(useTabs.getState().active).toBe(OBJECTS_TAB);
    expect(useGridStates.getState().states[id]).toBeUndefined();
  });

  it("ignores stale targets and the permanent Objects tab", async () => {
    const id = table("kept");
    await requestCloseTabs(["already-closed", OBJECTS_TAB]);
    expect(useTabs.getState().tabs.map((tab) => tab.id)).toEqual([id]);
    expect(useTabs.getState().active).toBe(id);
    expect(useConfirm.getState().request).toBeNull();
  });

  it("keeps a group intact when an unblurred designer edit is not discarded", async () => {
    const id = table("designed");
    const clean = table("clean");
    const design = { name: "designed", columns: [], indexes: [], foreignKeys: [], options: { engine: null, charset: null, collation: null, comment: null } };
    useDesignerDrafts.getState().initialize(id, design, design);
    useDesignerDrafts.getState().input(id, "column:0", "typing");
    const closing = requestCloseTabs([id, clean]);
    expect(useConfirm.getState().request?.danger).toBe(true);
    useConfirm.getState().settle(null);
    await closing;
    expect(useTabs.getState().tabs.map((tab) => tab.id)).toEqual([id, clean]);
    expect(useDesignerDrafts.getState().drafts[id]?.inputs["column:0"]).toBe("typing");

    const discard = requestCloseTabs([id]);
    useConfirm.getState().settle("ok");
    await discard;
    expect(useDesignerDrafts.getState().drafts[id]).toBeUndefined();
    expect(useTabs.getState().tabs.map((tab) => tab.id)).toEqual([clean]);
  });

  it("does not close a tab during a grid commit or table alteration", async () => {
    const id = table("saving");
    useGridStates.getState().update(id, () => ({ committing: true }));
    await requestCloseTabs([id]);
    expect(useTabs.getState().tabs.map((tab) => tab.id)).toEqual([id]);
    expect(useConfirm.getState().request).toBeNull();
    useGridStates.setState(useGridStates.getInitialState(), true);
    const design = { name: "saving", columns: [], indexes: [], foreignKeys: [], options: { engine: null, charset: null, collation: null, comment: null } };
    useDesignerDrafts.getState().initialize(id, design, design);
    useDesignerDrafts.getState().setBusy(id, true);
    await requestCloseTabs([id]);
    expect(useTabs.getState().tabs.map((tab) => tab.id)).toEqual([id]);
  });
});

describe("native application exit", () => {
  it("cancels without changing any draft and suppresses repeated close requests", async () => {
    useTabs.getState().newQuery({ connectionId: "connection", database: "main", sql: "SELECT 1" });
    const tabs = useTabs.getState().tabs;
    const close = vi.fn(async () => {});
    const closing = requestAppClose(close);
    expect(useConfirm.getState().request?.title).toBe("Close Schemata");
    expect(await requestAppClose(close)).toBe(false);
    useConfirm.getState().settle(null);
    expect(await closing).toBe(false);
    expect(close).not.toHaveBeenCalled();
    expect(useTabs.getState().tabs).toBe(tabs);
  });

  it("requires approval for running queries and transfer operations even with clean SQL", async () => {
    useTabs.getState().newQuery({ connectionId: "connection", database: "main" });
    const id = useTabs.getState().active;
    useQueryResults.getState().set(id, { connectionId: "connection", database: "main", result: null, failure: null, running: true, executionId: "query", startedAt: 1, finishedAt: null });
    useOperationActivity.getState().begin("export");
    const close = vi.fn(async () => {});
    const closing = requestAppClose(close);
    expect(useConfirm.getState().request?.message).toContain("still running");
    useConfirm.getState().settle("ok");
    expect(await closing).toBe(true);
    expect(close).toHaveBeenCalledOnce();
    expect(useQueryResults.getState().runs[id]?.running).toBe(true);
    expect(useOperationActivity.getState().active.has("export")).toBe(true);
  });

  it("blocks exit during a grid save without invoking native close", async () => {
    const id = table("saving");
    useGridStates.getState().update(id, () => ({ committing: true }));
    const close = vi.fn(async () => {});
    expect(await requestAppClose(close)).toBe(false);
    expect(close).not.toHaveBeenCalled();
    expect(useConfirm.getState().request).toBeNull();
  });

  it("reconfirms if an edit changes while the exit dialog is open", async () => {
    useTabs.getState().newQuery({ connectionId: "connection", database: "main", sql: "SELECT 1" });
    const id = useTabs.getState().active;
    const close = vi.fn(async () => {});
    const closing = requestAppClose(close);
    useTabs.getState().updateQuery(id, { sql: "SELECT 2" });
    useConfirm.getState().settle("ok");
    await Promise.resolve();
    await Promise.resolve();
    expect(close).not.toHaveBeenCalled();
    expect(useConfirm.getState().request?.title).toBe("Close Schemata");
    useConfirm.getState().settle(null);
    expect(await closing).toBe(false);
    expect(useTabs.getState().tabs[0]).toMatchObject({ sql: "SELECT 2" });
  });

  it("does not overwrite another confirmation dialog", async () => {
    const answer = useConfirm.getState().ask({ title: "Other action", message: "", confirmLabel: "Continue" });
    const close = vi.fn(async () => {});
    expect(await requestAppClose(close)).toBe(false);
    expect(useConfirm.getState().request?.title).toBe("Other action");
    useConfirm.getState().settle(null);
    await answer;
  });
});
