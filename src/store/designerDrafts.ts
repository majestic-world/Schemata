import { create } from "zustand";
import type { TableDesign } from "../api/types";

export type DesignerDraft = {
  original: TableDesign | null;
  design: TableDesign;
  dirty: boolean;
  busy: boolean;
  /** Text fields commit on blur, but their in-progress text must also survive unmounts. */
  inputs: Record<string, string>;
};

type Store = {
  drafts: Record<string, DesignerDraft | undefined>;
  initialize: (id: string, original: TableDesign | null, design: TableDesign) => void;
  flush: (id: string) => void;
  update: (id: string, change: (design: TableDesign) => TableDesign) => void;
  /**
   * Reorders or removes columns, indexes or foreign keys. Buffered input text is keyed by row position,
   * so it is flushed into the design first; otherwise it would land on whichever row takes that position.
   */
  restructure: (id: string, change: (design: TableDesign) => TableDesign) => void;
  input: (id: string, field: string, text: string | null) => void;
  setBusy: (id: string, busy: boolean) => void;
  saved: (id: string, design: TableDesign) => void;
  revert: (id: string) => void;
  forget: (id: string) => void;
};

/**
 * Renames column `index`, following it in index and foreign key columns and in the primary key order. A key
 * column that gets its first name joins the end of the key order.
 */
export function renameColumn(design: TableDesign, index: number, name: string): TableDesign {
  const old = design.columns[index].name;
  const rename = (names: string[]) => old && name ? names.map((n) => n === old ? name : n) : names;
  const primaryKeyOrder = !old && name && design.columns[index].primaryKey && !design.primaryKeyOrder.includes(name)
    ? [...design.primaryKeyOrder, name]
    : rename(design.primaryKeyOrder);
  return {
    ...design,
    columns: design.columns.map((c, i) => i === index ? { ...c, name } : c),
    indexes: design.indexes.map((item) => ({ ...item, columns: rename(item.columns) })),
    foreignKeys: design.foreignKeys.map((item) => ({ ...item, columns: rename(item.columns) })),
    primaryKeyOrder,
  };
}

/**
 * Removes column `index` from the design, from its indexes (dropping indexes left without columns), from the
 * primary key order, and drops the foreign keys that use it (a key cannot lose one column of its pairs).
 */
export function removeColumn(design: TableDesign, index: number): TableDesign {
  const name = design.columns[index].name;
  const columns = design.columns.filter((_, i) => i !== index);
  if (!name) return { ...design, columns };
  return {
    ...design,
    columns,
    indexes: design.indexes
      .map((item) => ({ ...item, columns: item.columns.filter((c) => c !== name) }))
      .filter((item) => item.columns.length > 0),
    foreignKeys: design.foreignKeys.filter((item) => !item.columns.includes(name)),
    primaryKeyOrder: design.primaryKeyOrder.filter((c) => c !== name),
  };
}

/** Removes several columns (see `removeColumn`). */
export function removeColumns(design: TableDesign, indices: readonly number[]): TableDesign {
  return [...new Set(indices)].sort((a, b) => b - a).reduce((d, i) => (i < d.columns.length ? removeColumn(d, i) : d), design);
}

/**
 * Moves the selected columns one position up (`-1`) or down (`1`), keeping their order. A column already at the
 * edge, or blocked by a selected column that could not move, stays. Returns the selection's new positions and
 * `moved`, every column's new position by its old one (for positions kept outside the selection).
 */
export function moveColumns(design: TableDesign, selected: readonly number[], delta: -1 | 1): { design: TableDesign; selected: number[]; moved: number[] } {
  const columns = [...design.columns];
  /** Old position of the column now at each position. */
  const origin = columns.map((_, i) => i);
  const order = [...new Set(selected)].filter((i) => i < columns.length).sort((a, b) => (a - b) * -delta);
  const taken = new Set<number>();
  for (const i of order) {
    const j = i + delta;
    if (j < 0 || j >= columns.length || taken.has(j)) {
      taken.add(i);
      continue;
    }
    [columns[i], columns[j]] = [columns[j], columns[i]];
    [origin[i], origin[j]] = [origin[j], origin[i]];
    taken.add(j);
  }
  const moved = origin.map(() => 0);
  origin.forEach((old, now) => (moved[old] = now));
  return { design: { ...design, columns }, selected: [...taken].sort((a, b) => a - b), moved };
}

/** Whether `moveColumns` would move any selected column. */
export function canMoveColumns(selected: readonly number[], count: number, delta: -1 | 1): boolean {
  const chosen = new Set(selected);
  return [...chosen].some((i) => i + delta >= 0 && i + delta < count && !chosen.has(i + delta));
}

/** Adds column `index` to the end of the primary key, or removes it; key columns are NOT NULL. */
export function setPrimaryKey(design: TableDesign, index: number, on: boolean): TableDesign {
  const column = design.columns[index];
  const rest = design.primaryKeyOrder.filter((c) => c !== column.name);
  return {
    ...design,
    columns: design.columns.map((c, i) => i === index ? { ...c, primaryKey: on, nullable: on ? false : c.nullable } : c),
    primaryKeyOrder: on && column.name ? [...rest, column.name] : rest,
  };
}

const isDirty = (draft: Pick<DesignerDraft, "original" | "design" | "inputs">) =>
  draft.original === null || Object.keys(draft.inputs).length > 0 || JSON.stringify(draft.original) !== JSON.stringify(draft.design);

export const useDesignerDrafts = create<Store>((set, get) => ({
  drafts: {},
  initialize: (id, original, design) => set((s) => {
    const current = s.drafts[id];
    if (current?.dirty || current?.busy) return s;
    const draft = { original, design: structuredClone(design), inputs: {}, busy: false };
    return { drafts: { ...s.drafts, [id]: { ...draft, dirty: isDirty(draft) } } };
  }),
  flush: (id) => set((s) => {
    const draft = s.drafts[id];
    if (!draft || draft.busy || Object.keys(draft.inputs).length === 0) return s;
    let design = draft.design;
    for (const [field, text] of Object.entries(draft.inputs)) {
      const [kind, position, target] = field.split(":");
      const index = Number(position);
      const columns = text.split(",").map((name) => name.trim()).filter(Boolean);
      if (kind === "column" && design.columns[index]) {
        design = renameColumn(design, index, text.trim());
      } else if (kind === "index") {
        design = { ...design, indexes: design.indexes.map((item, i) => i === index ? { ...item, columns } : item) };
      } else if (kind === "fk") {
        design = { ...design, foreignKeys: design.foreignKeys.map((item, i) => i === index ? { ...item, ...(target === "references" ? { refColumns: columns } : { columns }) } : item) };
      }
    }
    const next = { ...draft, design, inputs: {} };
    return { drafts: { ...s.drafts, [id]: { ...next, dirty: isDirty(next) } } };
  }),
  update: (id, change) => set((s) => {
    const draft = s.drafts[id];
    if (!draft || draft.busy) return s;
    const next = { ...draft, design: change(draft.design) };
    return { drafts: { ...s.drafts, [id]: { ...next, dirty: isDirty(next) } } };
  }),
  restructure: (id, change) => {
    get().flush(id);
    get().update(id, change);
  },
  input: (id, field, text) => set((s) => {
    const draft = s.drafts[id];
    if (!draft || draft.busy) return s;
    const inputs = { ...draft.inputs };
    if (text === null) delete inputs[field];
    else inputs[field] = text;
    const next = { ...draft, inputs };
    return { drafts: { ...s.drafts, [id]: { ...next, dirty: isDirty(next) } } };
  }),
  setBusy: (id, busy) => set((s) => {
    const draft = s.drafts[id];
    return draft ? { drafts: { ...s.drafts, [id]: { ...draft, busy } } } : s;
  }),
  saved: (id, design) => set((s) => {
    const draft = s.drafts[id];
    return draft ? { drafts: { ...s.drafts, [id]: { ...draft, original: structuredClone(design), design: structuredClone(design), dirty: false, inputs: {} } } } : s;
  }),
  revert: (id) => set((s) => {
    const draft = s.drafts[id];
    return draft?.original && !draft.busy
      ? { drafts: { ...s.drafts, [id]: { ...draft, design: structuredClone(draft.original), dirty: false, inputs: {} } } }
      : s;
  }),
  forget: (id) => set((s) => {
    const drafts = { ...s.drafts };
    delete drafts[id];
    return { drafts };
  }),
}));

export const hasDesignerEdits = (id: string) => useDesignerDrafts.getState().drafts[id]?.dirty ?? false;
