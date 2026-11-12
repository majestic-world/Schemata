import { beforeEach, describe, expect, it } from "vitest";
import type { TableDesign } from "../api/types";
import { hasDesignerEdits, useDesignerDrafts } from "./designerDrafts";

const design: TableDesign = {
  name: "items",
  columns: [{ originalName: "id", name: "id", dataType: "INTEGER", nullable: false, default: null, autoIncrement: false, primaryKey: true, comment: null }],
  indexes: [{ name: "by_id", columns: ["id"], kind: "normal", method: null, comment: null }],
  foreignKeys: [],
  options: { engine: null, charset: null, collation: null, comment: null },
};

beforeEach(() => useDesignerDrafts.setState(useDesignerDrafts.getInitialState(), true));

describe("per-tab table designs", () => {
  it("commits buffered text to its own row before rows are deleted or reordered", () => {
    const name = { ...design.columns[0], originalName: "name", name: "name", primaryKey: false };
    const qty = { ...name, originalName: "qty", name: "qty" };
    const wide = { ...design, columns: [design.columns[0], name, qty] };
    const store = useDesignerDrafts.getState();
    store.initialize("one", wide, wide);
    store.input("one", "column:1", "title");
    store.input("one", "index:0", "id, title");
    store.restructure("one", (d) => ({ ...d, columns: d.columns.filter((_, j) => j !== 0), indexes: [] }));
    let draft = useDesignerDrafts.getState().drafts.one;
    expect(draft?.inputs).toEqual({});
    expect(draft?.design.columns.map((c) => c.name)).toEqual(["title", "qty"]);

    store.input("one", "column:0", "label");
    store.restructure("one", (d) => ({ ...d, columns: [d.columns[1], d.columns[0]] }));
    store.flush("one");
    draft = useDesignerDrafts.getState().drafts.one;
    expect(draft?.design.columns.map((c) => c.name)).toEqual(["qty", "label"]);
  });

  it("keeps the original and edited design across remounts and metadata refreshes", () => {
    const store = useDesignerDrafts.getState();
    store.initialize("one", design, design);
    store.update("one", (value) => ({ ...value, name: "edited" }));
    store.initialize("other", design, design);
    store.initialize("one", { ...design, name: "refetched" }, { ...design, name: "refetched" });

    expect(useDesignerDrafts.getState().drafts.one?.original).toEqual(design);
    expect(useDesignerDrafts.getState().drafts.one?.design.name).toBe("edited");
    expect(useDesignerDrafts.getState().drafts.other?.design).toEqual(design);
    expect(hasDesignerEdits("one")).toBe(true);
    expect(hasDesignerEdits("other")).toBe(false);
  });

  it("retains unblurred text and commits column references before reviewing SQL", () => {
    const store = useDesignerDrafts.getState();
    store.initialize("one", design, design);
    store.input("one", "column:0", " item_id ");
    store.initialize("one", design, design);
    expect(useDesignerDrafts.getState().drafts.one?.inputs["column:0"]).toBe(" item_id ");
    expect(hasDesignerEdits("one")).toBe(true);

    store.flush("one");
    expect(useDesignerDrafts.getState().drafts.one?.design.columns[0].name).toBe("item_id");
    expect(useDesignerDrafts.getState().drafts.one?.design.indexes[0].columns).toEqual(["item_id"]);
    expect(useDesignerDrafts.getState().drafts.one?.inputs).toEqual({});
  });

  it("saves a new baseline, supports reverting later edits, and forgets closed drafts", () => {
    const store = useDesignerDrafts.getState();
    store.initialize("new", null, design);
    expect(hasDesignerEdits("new")).toBe(true);
    const saved = { ...design, name: "created" };
    store.saved("new", saved);
    expect(hasDesignerEdits("new")).toBe(false);
    store.update("new", (value) => ({ ...value, name: "changed" }));
    store.input("new", "column:0", "typing");
    store.revert("new");
    expect(useDesignerDrafts.getState().drafts.new?.design).toEqual(saved);
    expect(useDesignerDrafts.getState().drafts.new?.inputs).toEqual({});
    expect(hasDesignerEdits("new")).toBe(false);
    store.forget("new");
    expect(useDesignerDrafts.getState().drafts.new).toBeUndefined();
  });

  it("does not mutate or reinitialize a design while DDL is being applied", () => {
    const store = useDesignerDrafts.getState();
    store.initialize("one", design, design);
    store.setBusy("one", true);
    store.update("one", (value) => ({ ...value, name: "changed" }));
    store.input("one", "column:0", "typing");
    store.initialize("one", { ...design, name: "other" }, { ...design, name: "other" });
    expect(useDesignerDrafts.getState().drafts.one?.design).toEqual(design);
    expect(useDesignerDrafts.getState().drafts.one?.inputs).toEqual({});
  });
});
