import type { CellValue, ColumnInfo, ColumnMeta, RowChange } from "../api/types";

/** A row of the grid: one already in the database (page index) or one being inserted. */
export type RowRef = { kind: "existing"; index: number } | { kind: "new"; index: number };

/** Grid edits waiting to be committed. Immutable: every operation returns a new value. */
export type Pending = {
  updates: Record<number, Record<string, CellValue>>;
  inserts: Record<string, CellValue>[];
  deletes: number[];
};

export const emptyPending = (): Pending => ({ updates: {}, inserts: [], deletes: [] });

const sameValue = (a: CellValue, b: CellValue) => JSON.stringify(a) === JSON.stringify(b);

export function setCell(p: Pending, row: RowRef, column: string, value: CellValue, original: CellValue): Pending {
  if (row.kind === "new") {
    const inserts = p.inserts.map((r, i) => (i === row.index ? { ...r, [column]: value } : r));
    return { ...p, inserts };
  }
  const current = { ...(p.updates[row.index] ?? {}) };
  if (sameValue(value, original)) delete current[column];
  else current[column] = value;
  const updates = { ...p.updates };
  if (Object.keys(current).length) updates[row.index] = current;
  else delete updates[row.index];
  return { ...p, updates };
}

/**
 * Value of a constant column default (`0`, `-1.5`, `'text'`), or null when the default is an expression
 * (`CURRENT_TIMESTAMP`, `(uuid())`, …) only the database can evaluate.
 */
export function literalDefault(expr: string): CellValue | null {
  const s = expr.trim();
  if (/^[+-]?\d+$/.test(s)) {
    const n = Number(s);
    return Number.isSafeInteger(n) ? { type: "int", value: n } : { type: "bigInt", value: s.replace(/^\+/, "") };
  }
  if (/^[+-]?(\d+\.\d*|\.\d+)([eE][+-]?\d+)?$/.test(s)) return { type: "decimal", value: s.replace(/^\+/, "") };
  const quoted = /^'((?:[^']|'')*)'$/.exec(s);
  return quoted ? { type: "text", value: quoted[1].replace(/''/g, "'") } : null;
}

/** New row prefilled with the constant defaults; other columns are left for the database to fill. */
export function addRow(p: Pending, columns: ColumnInfo[]): { pending: Pending; row: RowRef } {
  const values: Record<string, CellValue> = {};
  for (const c of columns) {
    if (c.generated) continue;
    const v = c.default === null || c.autoIncrement ? null : literalDefault(c.default);
    if (v) values[c.name] = v;
  }
  return { pending: { ...p, inserts: [...p.inserts, values] }, row: { kind: "new", index: p.inserts.length } };
}

/** New-row columns with no value yet: the insert omits them, so the database applies their default. */
export const isUnset = (p: Pending, row: RowRef, column: string) => row.kind === "new" && !(column in (p.inserts[row.index] ?? {}));

/** Existing rows are marked for deletion (dropping their edits); new rows are simply removed. */
export function deleteRows(p: Pending, rows: RowRef[]): Pending {
  const existing = rows.filter((r) => r.kind === "existing").map((r) => r.index);
  const removedNew = new Set(rows.filter((r) => r.kind === "new").map((r) => r.index));
  const updates = { ...p.updates };
  existing.forEach((i) => delete updates[i]);
  return {
    updates,
    inserts: p.inserts.filter((_, i) => !removedNew.has(i)),
    deletes: [...new Set([...p.deletes, ...existing])].sort((a, b) => a - b),
  };
}

export const changeCount = (p: Pending) => Object.keys(p.updates).length + p.inserts.length + p.deletes.length;
export const isDirty = (p: Pending) => changeCount(p) > 0;

export const isDeleted = (p: Pending, row: RowRef) => row.kind === "existing" && p.deletes.includes(row.index);
export const isEdited = (p: Pending, row: RowRef, column: string) =>
  row.kind === "existing" && p.updates[row.index] !== undefined && column in p.updates[row.index];

/** Value shown for a cell, taking pending edits into account. New rows show their typed values or NULL. */
export function cellValue(p: Pending, row: RowRef, column: string, original: CellValue | undefined): CellValue {
  if (row.kind === "new") return p.inserts[row.index]?.[column] ?? { type: "null" };
  return p.updates[row.index]?.[column] ?? original ?? { type: "null" };
}

/**
 * Renders the pending edits as a change set: updates, then inserts, then deletes.
 * `rowOf[i]` is the grid row behind change `i`, used to point at a failing row.
 */
export function buildChangeSet(
  p: Pending,
  pageRows: CellValue[][],
  columns: ColumnMeta[],
  identity: string[],
): { changes: RowChange[]; rowOf: RowRef[] } {
  const keyIdx = identity.map((name) => {
    const i = columns.findIndex((c) => c.name === name);
    if (i < 0) throw new Error(`Identity column ${name} is not in the result`);
    return i;
  });
  const keyOf = (row: number) => keyIdx.map((i) => pageRows[row][i]);
  const changes: RowChange[] = [];
  const rowOf: RowRef[] = [];
  for (const [row, values] of Object.entries(p.updates).sort((a, b) => Number(a[0]) - Number(b[0]))) {
    const index = Number(row);
    if (p.deletes.includes(index)) continue;
    changes.push({ kind: "update", key: keyOf(index), values: Object.entries(values).map(([column, value]) => ({ column, value })) });
    rowOf.push({ kind: "existing", index });
  }
  p.inserts.forEach((values, index) => {
    changes.push({ kind: "insert", values: Object.entries(values).map(([column, value]) => ({ column, value })) });
    rowOf.push({ kind: "new", index });
  });
  for (const index of p.deletes) {
    changes.push({ kind: "delete", key: keyOf(index) });
    rowOf.push({ kind: "existing", index });
  }
  return { changes, rowOf };
}
