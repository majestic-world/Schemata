import { QueryClient } from "@tanstack/react-query";
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../api/client";
import { useConfirm } from "../store/confirm";
import { OBJECTS_TAB, useTabs } from "../store/tabs";
import { useWorkspace } from "../store/workspace";
import { dropObjects, renameTable } from "./objectActions";
import { useDesignerDrafts } from "../store/designerDrafts";
import { useGridStates } from "../store/gridState";
import { setCell } from "./pendingChanges";

vi.hoisted(() => {
  const values = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => { values.set(key, value); },
    removeItem: (key: string) => { values.delete(key); },
  });
});

const scope = { connectionId: "connection", database: "main" };
const objects = ["first", "second", "third"].map((name) => ({ name, kind: "table" as const }));
let client: QueryClient;

function open(name: string) {
  useTabs.getState().openTable({ ...scope, name, objectKind: "table" });
  return useTabs.getState().active;
}

beforeEach(() => {
  useConfirm.getState().settle(null);
  useTabs.setState(useTabs.getInitialState(), true);
  useWorkspace.setState(useWorkspace.getInitialState(), true);
  useDesignerDrafts.setState(useDesignerDrafts.getInitialState(), true);
  useGridStates.setState(useGridStates.getInitialState(), true);
  useWorkspace.getState().selectObjects({ ...scope, objects });
  client = new QueryClient();
  client.setQueryData(["objects", scope.connectionId, scope.database], objects);
});
afterEach(() => {
  useConfirm.getState().settle(null);
  client.clear();
  vi.restoreAllMocks();
  useTabs.setState(useTabs.getInitialState(), true);
  useWorkspace.setState(useWorkspace.getInitialState(), true);
  useDesignerDrafts.setState(useDesignerDrafts.getInitialState(), true);
  useGridStates.setState(useGridStates.getInitialState(), true);
});
afterAll(() => vi.unstubAllGlobals());

describe("deleting selected database objects", () => {
  it("cancelling preserves all tabs and the entire selection without deleting anything", async () => {
    const remove = vi.spyOn(api, "dropObject").mockResolvedValue(undefined);
    const tabs = objects.map((object) => open(object.name));
    useTabs.getState().activate(OBJECTS_TAB);
    useWorkspace.getState().selectObjects({ ...scope, objects });
    const operation = dropObjects(scope, objects, client);
    useConfirm.getState().settle(null);
    await operation;

    expect(remove).not.toHaveBeenCalled();
    expect(useTabs.getState().tabs.map((tab) => tab.id)).toEqual(tabs);
    expect(useWorkspace.getState().selectedObjects).toEqual(objects);
  });

  it("closes only deleted tabs and clears the object selection without leaving the database", async () => {
    vi.spyOn(api, "dropObject").mockResolvedValue(undefined);
    objects.forEach((object) => open(object.name));
    const kept = open("not-selected");
    const operation = dropObjects(scope, objects, client);
    useConfirm.getState().settle("ok");
    await operation;

    expect(useTabs.getState().tabs.map((tab) => tab.id)).toEqual([kept]);
    expect(useWorkspace.getState().selection).toEqual({ ...scope, object: undefined });
    expect(useWorkspace.getState().selectedObjects).toEqual([]);
    expect(client.getQueryState(["objects", scope.connectionId, scope.database])?.isInvalidated).toBe(true);
  });

  it("a failure keeps the failed and unattempted objects selected and their tabs open", async () => {
    vi.spyOn(api, "dropObject")
      .mockResolvedValueOnce(undefined)
      .mockRejectedValueOnce({ kind: "sql", message: "Referenced by a foreign key", sqlState: "23000" });
    const tabs = objects.map((object) => open(object.name));
    const operation = dropObjects(scope, objects, client);
    useConfirm.getState().settle("ok");
    await operation;

    expect(useTabs.getState().tabs.map((tab) => tab.id)).toEqual(tabs.slice(1));
    expect(useWorkspace.getState().selectedObjects).toEqual(objects.slice(1));
    expect(client.getQueryState(["objects", scope.connectionId, scope.database])?.isInvalidated).toBe(true);
  });

  it("does not overwrite a different database selected while deletion is in progress", async () => {
    vi.spyOn(api, "dropObject").mockResolvedValue(undefined);
    const operation = dropObjects(scope, objects, client);
    useWorkspace.getState().select({ connectionId: "other", database: "other-db" });
    useConfirm.getState().settle("ok");
    await operation;

    expect(useWorkspace.getState().selection).toEqual({ connectionId: "other", database: "other-db" });
    expect(useWorkspace.getState().selectedObjects).toEqual([]);
  });
});

describe("renaming tables with open drafts", () => {
  it("cancels the entire rename before touching the database or discarding grid/designer edits", async () => {
    const rename = vi.spyOn(api, "renameTable").mockResolvedValue(undefined);
    const id = open("first");
    const design = { name: "first", columns: [], indexes: [], foreignKeys: [], options: { engine: null, charset: null, collation: null, comment: null } };
    useDesignerDrafts.getState().initialize(id, design, design);
    useDesignerDrafts.getState().input(id, "column:0", "draft");
    useGridStates.getState().update(id, (state) => ({
      pending: setCell(state.pending, { kind: "existing", index: 0 }, "name", { type: "text", value: "new" }, { type: "text", value: "old" }),
    }));
    const grid = useGridStates.getState().states[id];
    const draft = useDesignerDrafts.getState().drafts[id];
    const operation = renameTable({ ...scope, name: "first", kind: "table" }, client);
    useConfirm.getState().settle("renamed");
    await Promise.resolve();
    expect(useConfirm.getState().request?.title).toBe("Discard changes");
    useConfirm.getState().settle(null);
    await operation;

    expect(rename).not.toHaveBeenCalled();
    expect(useTabs.getState().tabs.map((tab) => tab.id)).toEqual([id]);
    expect(useDesignerDrafts.getState().drafts[id]).toBe(draft);
    expect(useGridStates.getState().states[id]).toBe(grid);
  });

  it("renames only after discard approval and leaves unrelated tabs intact", async () => {
    const rename = vi.spyOn(api, "renameTable").mockResolvedValue(undefined);
    const id = open("first");
    const kept = open("other");
    const design = { name: "first", columns: [], indexes: [], foreignKeys: [], options: { engine: null, charset: null, collation: null, comment: null } };
    useDesignerDrafts.getState().initialize(id, design, design);
    useDesignerDrafts.getState().update(id, (value) => ({ ...value, name: "draft" }));
    const operation = renameTable({ ...scope, name: "first", kind: "table" }, client);
    useConfirm.getState().settle("renamed");
    await Promise.resolve();
    expect(rename).not.toHaveBeenCalled();
    useConfirm.getState().settle("ok");
    await operation;

    expect(rename).toHaveBeenCalledWith(scope.connectionId, scope.database, "first", "renamed");
    expect(useDesignerDrafts.getState().drafts[id]).toBeUndefined();
    expect(useTabs.getState().tabs.map((tab) => tab.id)).toEqual([kept]);
    expect(useWorkspace.getState().selection?.object?.name).toBe("renamed");
  });

  it("does not rename a table while its DDL is running", async () => {
    const rename = vi.spyOn(api, "renameTable").mockResolvedValue(undefined);
    const id = open("first");
    const design = { name: "first", columns: [], indexes: [], foreignKeys: [], options: { engine: null, charset: null, collation: null, comment: null } };
    useDesignerDrafts.getState().initialize(id, design, design);
    useDesignerDrafts.getState().setBusy(id, true);
    const operation = renameTable({ ...scope, name: "first", kind: "table" }, client);
    useConfirm.getState().settle("renamed");
    await operation;
    expect(rename).not.toHaveBeenCalled();
    expect(useTabs.getState().tabs.map((tab) => tab.id)).toEqual([id]);
  });
});
