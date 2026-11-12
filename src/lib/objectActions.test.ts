import { QueryClient } from "@tanstack/react-query";
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../api/client";
import { useConfirm } from "../store/confirm";
import { OBJECTS_TAB, useTabs } from "../store/tabs";
import { useWorkspace } from "../store/workspace";
import { dropObjects, emptyTable, renameTable, truncateTable } from "./objectActions";
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

/** Answers the confirmation once it is shown (deleting asks only after looking up foreign keys). */
async function answer(value: string | null) {
  await vi.waitFor(() => expect(useConfirm.getState().request).not.toBeNull());
  useConfirm.getState().settle(value);
}

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
  vi.spyOn(api, "incomingForeignKeys").mockResolvedValue([]);
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
    await answer(null);
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
    await answer("ok");
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
    await answer("ok");
    await operation;

    expect(useTabs.getState().tabs.map((tab) => tab.id)).toEqual(tabs.slice(1));
    expect(useWorkspace.getState().selectedObjects).toEqual(objects.slice(1));
    expect(client.getQueryState(["objects", scope.connectionId, scope.database])?.isInvalidated).toBe(true);
  });

  it("does not overwrite a different database selected while deletion is in progress", async () => {
    vi.spyOn(api, "dropObject").mockResolvedValue(undefined);
    const operation = dropObjects(scope, objects, client);
    useWorkspace.getState().select({ connectionId: "other", database: "other-db" });
    await answer("ok");
    await operation;

    expect(useWorkspace.getState().selection).toEqual({ connectionId: "other", database: "other-db" });
    expect(useWorkspace.getState().selectedObjects).toEqual([]);
  });
});

describe("renaming tables with open drafts", () => {
  it("cancels the entire rename before touching the database or discarding grid/designer edits", async () => {
    const rename = vi.spyOn(api, "renameTable").mockResolvedValue(undefined);
    const id = open("first");
    const design = { name: "first", columns: [], indexes: [], foreignKeys: [], options: { engine: null, charset: null, collation: null, comment: null }, primaryKeyOrder: [] };
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
    const design = { name: "first", columns: [], indexes: [], foreignKeys: [], options: { engine: null, charset: null, collation: null, comment: null }, primaryKeyOrder: [] };
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
    const design = { name: "first", columns: [], indexes: [], foreignKeys: [], options: { engine: null, charset: null, collation: null, comment: null }, primaryKeyOrder: [] };
    useDesignerDrafts.getState().initialize(id, design, design);
    useDesignerDrafts.getState().setBusy(id, true);
    const operation = renameTable({ ...scope, name: "first", kind: "table" }, client);
    useConfirm.getState().settle("renamed");
    await operation;
    expect(rename).not.toHaveBeenCalled();
    expect(useTabs.getState().tabs.map((tab) => tab.id)).toEqual([id]);
  });
});

describe("confirming the removal of referenced tables", () => {
  const connection = (engine: "mySql" | "sqlite") => client.setQueryData(["connections"], [{ id: scope.connectionId, engine }]);
  const refs = [
    { table: "orders", name: "fk_orders_0", onDelete: "CASCADE" },
    { table: "notes", name: "fk_notes_0", onDelete: "SET NULL" },
    { table: "audit", name: "fk_audit_0", onDelete: "NO ACTION" },
  ];

  it.each(["sqlite", "mySql"] as const)("%s: says which rows emptying the table deletes or sets to NULL", async (engine) => {
    connection(engine);
    vi.spyOn(api, "incomingForeignKeys").mockResolvedValue(refs);
    const empty = vi.spyOn(api, "emptyTable").mockResolvedValue(undefined);
    const operation = emptyTable({ ...scope, name: "customers", kind: "table" }, client);
    await vi.waitFor(() => expect(useConfirm.getState().request).not.toBeNull());
    const message = useConfirm.getState().request!.message;
    expect(message).toContain('rows in "orders" that reference it will be deleted (ON DELETE CASCADE)');
    expect(message).toContain('references to it in "notes" will be set to NULL (ON DELETE SET NULL)');
    expect(message).toContain('emptying it fails while rows in "audit" reference it (ON DELETE NO ACTION)');
    useConfirm.getState().settle(null);
    await operation;
    expect(empty).not.toHaveBeenCalled();
  });

  it("MySQL: warns that truncating a referenced table may fail", async () => {
    connection("mySql");
    vi.spyOn(api, "incomingForeignKeys").mockResolvedValue(refs.slice(0, 1));
    const truncate = vi.spyOn(api, "truncateTable").mockResolvedValue(undefined);
    const operation = truncateTable({ ...scope, name: "customers", kind: "table" }, client);
    await vi.waitFor(() => expect(useConfirm.getState().request).not.toBeNull());
    expect(useConfirm.getState().request!.message).toContain('Referenced by "orders" (ON DELETE CASCADE). MySQL refuses to truncate a table other tables reference, so this may fail.');
    useConfirm.getState().settle(null);
    await operation;
    expect(truncate).not.toHaveBeenCalled();
  });

  it("MySQL: lists the referencing tables and warns that deleting may fail", async () => {
    connection("mySql");
    const lookup = vi.spyOn(api, "incomingForeignKeys").mockImplementation(async (_id, _db, table) => (table === "first" ? refs.slice(0, 1) : []));
    const operation = dropObjects(scope, objects, client);
    await vi.waitFor(() => expect(useConfirm.getState().request).not.toBeNull());
    const message = useConfirm.getState().request!.message;
    expect(lookup).toHaveBeenCalledTimes(objects.length);
    expect(message).toContain('Referenced by "orders" (ON DELETE CASCADE). MySQL refuses to delete a table other tables reference, so this may fail.');
    useConfirm.getState().settle(null);
    await operation;
  });

  it("says when the references could not be checked", async () => {
    connection("sqlite");
    vi.spyOn(api, "incomingForeignKeys").mockRejectedValue({ kind: "connection", message: "lost", sqlState: null });
    const operation = truncateTable({ ...scope, name: "customers", kind: "table" }, client);
    await vi.waitFor(() => expect(useConfirm.getState().request).not.toBeNull());
    expect(useConfirm.getState().request!.message).toContain('Could not check which tables reference "customers": lost.');
    useConfirm.getState().settle(null);
    await operation;
  });

  it("does not look up references for views", async () => {
    const lookup = vi.spyOn(api, "incomingForeignKeys");
    const operation = dropObjects(scope, [{ name: "v", kind: "view" }], client);
    await answer(null);
    await operation;
    expect(lookup).not.toHaveBeenCalled();
  });
});
