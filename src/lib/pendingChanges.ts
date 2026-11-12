import type { CellValue, ColumnInfo, ColumnMeta, Engine, RowChange } from "../api/types";

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

/** Column maps are keyed by arbitrary column names (`constructor`, `__proto__`): read own properties only. */
const ownValue = (values: Record<string, CellValue> | undefined, column: string): CellValue | undefined =>
  values !== undefined && Object.hasOwn(values, column) ? values[column] : undefined;

/** One cell edit for [`setCells`]: `original` is the saved value an edit back to it cancels. */
export type CellEdit = { row: RowRef; column: string; value: CellValue; original: CellValue };

export function setCell(p: Pending, row: RowRef, column: string, value: CellValue, original: CellValue): Pending {
  return setCells(p, [{ row, column, value, original }]);
}

/** Applies several cell edits, copying each changed row map once, so editing many cells stays linear. */
export function setCells(p: Pending, edits: readonly CellEdit[]): Pending {
  let updates = p.updates;
  let inserts = p.inserts;
  const copiedUpdates = new Set<number>();
  const copiedInserts = new Set<number>();
  // Column names are arbitrary (`__proto__`): define own properties instead of assigning them.
  const put = (values: Record<string, CellValue>, column: string, value: CellValue) =>
    Object.defineProperty(values, column, { value, enumerable: true, writable: true, configurable: true });
  for (const { row, column, value, original } of edits) {
    if (row.kind === "new") {
      if (row.index >= inserts.length) continue;
      if (inserts === p.inserts) inserts = [...p.inserts];
      if (!copiedInserts.has(row.index)) {
        inserts[row.index] = { ...inserts[row.index] };
        copiedInserts.add(row.index);
      }
      put(inserts[row.index], column, value);
      continue;
    }
    if (updates === p.updates) updates = { ...p.updates };
    if (!copiedUpdates.has(row.index)) {
      updates[row.index] = { ...(updates[row.index] ?? {}) };
      copiedUpdates.add(row.index);
    }
    if (sameValue(value, original)) delete updates[row.index][column];
    else put(updates[row.index], column, value);
  }
  for (const index of copiedUpdates) {
    if (Object.keys(updates[index]).length === 0) delete updates[index];
  }
  return updates === p.updates && inserts === p.inserts ? p : { ...p, updates, inserts };
}

/** Characters MySQL string literals spell with a backslash; any other escaped character stands for itself. */
const MYSQL_ESCAPES: Record<string, string> = { "0": "\0", b: "\b", n: "\n", r: "\r", t: "\t", Z: "\x1a" };

/**
 * Value of a constant column default (`0`, `-1.5`, `'text'`), or null when the default is an expression
 * (`CURRENT_TIMESTAMP`, `(uuid())`, …) only the database can evaluate. MySQL and MariaDB report string defaults
 * with backslash escapes (`'C:\\temp'`); SQLite literals have none, only doubled quotes.
 */
export function literalDefault(expr: string, engine: Engine): CellValue | null {
  const s = expr.trim();
  if (/^[+-]?\d+$/.test(s)) {
    const n = Number(s);
    return Number.isSafeInteger(n) ? { type: "int", value: n } : { type: "bigInt", value: s.replace(/^\+/, "") };
  }
  if (/^[+-]?(\d+\.\d*|\.\d+)([eE][+-]?\d+)?$/.test(s)) return { type: "decimal", value: s.replace(/^\+/, "") };
  if (engine === "sqlite") {
    const quoted = /^'((?:[^']|'')*)'$/.exec(s);
    return quoted ? { type: "text", value: quoted[1].replace(/''/g, "'") } : null;
  }
  const quoted = /^'((?:[^'\\]|''|\\[\s\S])*)'$/.exec(s);
  if (!quoted) return null;
  // `\%` and `\_` keep their backslash: MySQL leaves it so LIKE patterns can use them.
  const value = quoted[1].replace(/''|\\([\s\S])/g, (all, c: string | undefined) =>
    c === undefined ? "'" : c === "%" || c === "_" ? all : (MYSQL_ESCAPES[c] ?? c));
  return { type: "text", value };
}

/** New row prefilled with the constant defaults; other columns are left for the database to fill. */
export function addRow(p: Pending, columns: ColumnInfo[], engine: Engine): { pending: Pending; row: RowRef } {
  const values: Record<string, CellValue> = {};
  for (const c of columns) {
    if (c.generated) continue;
    const v = c.default === null || c.autoIncrement ? null : literalDefault(c.default, engine);
    if (v) Object.defineProperty(values, c.name, { value: v, enumerable: true, writable: true, configurable: true });
  }
  return { pending: { ...p, inserts: [...p.inserts, values] }, row: { kind: "new", index: p.inserts.length } };
}

/** New-row columns with no value yet: the insert omits them, so the database applies their default. */
export const isUnset = (p: Pending, row: RowRef, column: string) =>
  row.kind === "new" && !Object.hasOwn(p.inserts[row.index] ?? {}, column);

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
  row.kind === "existing" && p.updates[row.index] !== undefined && Object.hasOwn(p.updates[row.index], column);

/** Value shown for a cell, taking pending edits into account. New rows show their typed values or NULL. */
export function cellValue(p: Pending, row: RowRef, column: string, original: CellValue | undefined): CellValue {
  if (row.kind === "new") return ownValue(p.inserts[row.index], column) ?? { type: "null" };
  return ownValue(p.updates[row.index], column) ?? original ?? { type: "null" };
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
  // The row as loaded, so the backend can refuse to write a row that changed or was replaced meanwhile.
  const expectedOf = (row: number) => columns.flatMap((c, i) => (c.hidden ? [] : [{ column: c.name, value: pageRows[row][i] }]));
  const changes: RowChange[] = [];
  const rowOf: RowRef[] = [];
  for (const [row, values] of Object.entries(p.updates).sort((a, b) => Number(a[0]) - Number(b[0]))) {
    const index = Number(row);
    if (p.deletes.includes(index)) continue;
    changes.push({ kind: "update", key: keyOf(index), values: Object.entries(values).map(([column, value]) => ({ column, value })), expected: expectedOf(index) });
    rowOf.push({ kind: "existing", index });
  }
  p.inserts.forEach((values, index) => {
    changes.push({ kind: "insert", values: Object.entries(values).map(([column, value]) => ({ column, value })) });
    rowOf.push({ kind: "new", index });
  });
  for (const index of p.deletes) {
    changes.push({ kind: "delete", key: keyOf(index), expected: expectedOf(index) });
    rowOf.push({ kind: "existing", index });
  }
  return { changes, rowOf };
}
