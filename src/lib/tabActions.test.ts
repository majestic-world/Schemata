import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { useConfirm } from "../store/confirm";
import { hasPendingEdits, useGridStates } from "../store/gridState";
import { OBJECTS_TAB, useTabs } from "../store/tabs";
import { setCell } from "./pendingChanges";
import { requestCloseTabs } from "./tabActions";

function reset() {
  useConfirm.getState().settle(null);
  useTabs.setState(useTabs.getInitialState(), true);
  useGridStates.setState(useGridStates.getInitialState(), true);
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
});
