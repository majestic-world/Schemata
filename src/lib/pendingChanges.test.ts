import { describe, expect, it } from "vitest";
import type { CellValue, ColumnInfo, ColumnMeta } from "../api/types";
import { addRow, buildChangeSet, cellValue, deleteRows, emptyPending, isDirty, isUnset, literalDefault, setCell } from "./pendingChanges";

const t = (value: string): CellValue => ({ type: "text", value });
const n = (value: number): CellValue => ({ type: "int", value });
const NULL: CellValue = { type: "null" };
const col = (name: string, hidden = false): ColumnMeta => ({ name, typeName: "", hidden });

const columns = [col("id"), col("name"), col("qty")];
const page: CellValue[][] = [
  [n(1), t("a"), n(10)],
  [n(2), t("b"), n(20)],
  [n(3), t("c"), n(30)],
];
const existing = (index: number) => ({ kind: "existing" as const, index });

describe("pending grid changes", () => {
  it("typing a cell back to its original value leaves nothing to commit", () => {
    let p = setCell(emptyPending(), existing(0), "name", t("x"), t("a"));
    expect(isDirty(p)).toBe(true);
    p = setCell(p, existing(0), "name", t("a"), t("a"));
    expect(isDirty(p)).toBe(false);
  });

  it("NULL and empty string are different edits", () => {
    const p = setCell(emptyPending(), existing(1), "name", NULL, t(""));
    expect(isDirty(p)).toBe(true);
    expect(cellValue(p, existing(1), "name", t(""))).toEqual(NULL);
  });

  it("builds updates, inserts and deletes keyed by the row identity", () => {
    let p = setCell(emptyPending(), existing(2), "qty", n(31), n(30));
    const added = addRow(p, []);
    p = setCell(added.pending, added.row, "id", n(4), NULL);
    p = setCell(p, added.row, "name", t("d"), NULL);
    p = deleteRows(p, [existing(0)]);
    const { changes, rowOf } = buildChangeSet(p, page, columns, ["id"]);
    expect(changes).toEqual([
      { kind: "update", key: [n(3)], values: [{ column: "qty", value: n(31) }] },
      { kind: "insert", values: [{ column: "id", value: n(4) }, { column: "name", value: t("d") }] },
      { kind: "delete", key: [n(1)] },
    ]);
    expect(rowOf).toEqual([existing(2), { kind: "new", index: 0 }, existing(0)]);
  });

  it("deleting an edited row drops its edits; deleting a new row discards it", () => {
    let p = setCell(emptyPending(), existing(1), "name", t("zz"), t("b"));
    const added = addRow(p, []);
    p = deleteRows(added.pending, [existing(1), added.row]);
    const { changes } = buildChangeSet(p, page, columns, ["id"]);
    expect(changes).toEqual([{ kind: "delete", key: [n(2)] }]);
  });

  it("uses hidden identity columns such as the SQLite rowid", () => {
    const withRowid = [col("__rowid__", true), col("name")];
    const rows: CellValue[][] = [[n(7), t("x")]];
    const p = setCell(emptyPending(), existing(0), "name", t("y"), t("x"));
    expect(buildChangeSet(p, rows, withRowid, ["__rowid__"]).changes).toEqual([
      { kind: "update", key: [n(7)], values: [{ column: "name", value: t("y") }] },
    ]);
  });
});

const info = (name: string, def: string | null, autoIncrement = false): ColumnInfo => ({
  name, dataType: "", nullable: true, default: def, autoIncrement, primaryKey: false, comment: null,
});

describe("new row defaults", () => {
  it("reads constant defaults and leaves expressions to the database", () => {
    expect(literalDefault("0")).toEqual(n(0));
    expect(literalDefault("-12")).toEqual(n(-12));
    expect(literalDefault("18446744073709551615")).toEqual({ type: "bigInt", value: "18446744073709551615" });
    expect(literalDefault("1.50")).toEqual({ type: "decimal", value: "1.50" });
    expect(literalDefault("'it''s'")).toEqual(t("it's"));
    expect(literalDefault("''")).toEqual(t(""));
    expect(literalDefault("CURRENT_TIMESTAMP")).toBeNull();
    expect(literalDefault("current_timestamp()")).toBeNull();
    expect(literalDefault("(uuid())")).toBeNull();
    expect(literalDefault("'a' || 'b'")).toBeNull();
  });

  it("prefills constant defaults; other columns stay unset and are omitted from the insert", () => {
    const { pending, row } = addRow(emptyPending(), [
      info("id", null, true),
      info("success", "0"),
      info("status", "'new'"),
      info("created_at", "CURRENT_TIMESTAMP"),
    ]);
    expect(isUnset(pending, row, "id")).toBe(true);
    expect(isUnset(pending, row, "created_at")).toBe(true);
    expect(isUnset(pending, row, "success")).toBe(false);
    expect(buildChangeSet(pending, page, columns, ["id"]).changes).toEqual([
      { kind: "insert", values: [{ column: "success", value: n(0) }, { column: "status", value: t("new") }] },
    ]);
  });
});
