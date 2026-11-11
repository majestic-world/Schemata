import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { OBJECTS_TAB, tableTabId, useTabs } from "./tabs";
import { useWorkspace } from "./workspace";

function reset() {
  useTabs.setState(useTabs.getInitialState(), true);
  useWorkspace.setState(useWorkspace.getInitialState(), true);
}

beforeEach(reset);
afterEach(reset);

describe("active table selection", () => {
  it("selects the opened object instead of leaving its database selected", () => {
    useWorkspace.getState().select({ connectionId: "server", database: "catalog" });
    useTabs.getState().openTable({ connectionId: "server", database: "catalog", name: "products", objectKind: "table" });

    expect(useWorkspace.getState().selection).toEqual({
      connectionId: "server", database: "catalog", object: { name: "products", kind: "table" },
    });
    expect(useWorkspace.getState().selectedObjects).toEqual([{ name: "products", kind: "table" }]);
  });

  it("follows activation across identically named objects in different databases and connections", () => {
    const targets = [
      { connectionId: "first", database: "catalog", name: "products", objectKind: "table" as const },
      { connectionId: "first", database: "archive", name: "products", objectKind: "table" as const },
      { connectionId: "second", database: "catalog", name: "products", objectKind: "view" as const },
    ];
    for (const target of targets) useTabs.getState().openTable(target);

    for (const target of targets) {
      useWorkspace.getState().select({ connectionId: "unrelated", database: "other" });
      useTabs.getState().activate(tableTabId(target.connectionId, target.database, target.name));
      expect(useWorkspace.getState().selection).toEqual({
        connectionId: target.connectionId, database: target.database,
        object: { name: target.name, kind: target.objectKind },
      });
    }
  });

  it("restores selection when reopening or reactivating the same table tab", () => {
    const target = { connectionId: "server", database: "catalog", name: "products", objectKind: "table" as const };
    useTabs.getState().openTable(target);
    const id = useTabs.getState().active;
    useWorkspace.getState().select({ connectionId: "server", database: "catalog" });
    useTabs.getState().openTable(target, "structure");
    expect(useWorkspace.getState().selection?.object).toEqual({ name: "products", kind: "table" });

    useWorkspace.getState().select(null);
    useTabs.getState().activate(id);
    expect(useWorkspace.getState().selection?.object).toEqual({ name: "products", kind: "table" });

    useTabs.getState().activate(OBJECTS_TAB);
    useWorkspace.getState().select({ connectionId: "server", database: "archive" });
    useTabs.getState().activate(OBJECTS_TAB);
    expect(useWorkspace.getState().selection).toEqual({ connectionId: "server", database: "archive" });
  });

  it("selects the surviving active table on close without changing selection for background closes", () => {
    const tabs = useTabs.getState();
    tabs.openTable({ connectionId: "server", database: "left", name: "products", objectKind: "table" });
    const left = useTabs.getState().active;
    tabs.openTable({ connectionId: "server", database: "middle", name: "products", objectKind: "table" });
    const middle = useTabs.getState().active;
    tabs.openTable({ connectionId: "server", database: "right", name: "products", objectKind: "table" });
    const right = useTabs.getState().active;
    tabs.activate(middle);
    useWorkspace.getState().select({ connectionId: "server", database: "browsing" });

    tabs.close(right);
    expect(useTabs.getState().active).toBe(middle);
    expect(useWorkspace.getState().selection).toEqual({ connectionId: "server", database: "browsing" });

    tabs.close(middle);
    expect(useTabs.getState().active).toBe(left);
    expect(useWorkspace.getState().selection).toEqual({
      connectionId: "server", database: "left", object: { name: "products", kind: "table" },
    });
  });

  it("selects a newly created table when its designer tab is replaced", () => {
    useTabs.getState().newTable("server", "catalog");
    const designer = useTabs.getState().active;
    useTabs.getState().replace(designer, {
      connectionId: "server", database: "catalog", name: "created", objectKind: "table",
    }, "structure");
    expect(useTabs.getState().active).toBe(tableTabId("server", "catalog", "created"));
    expect(useWorkspace.getState().selection).toEqual({
      connectionId: "server", database: "catalog", object: { name: "created", kind: "table" },
    });
  });
});
