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
        const old = design.columns[index].name;
        const name = text.trim();
        const rename = (names: string[]) => old && name ? names.map((n) => n === old ? name : n) : names;
        design = {
          ...design,
          columns: design.columns.map((c, i) => i === index ? { ...c, name } : c),
          indexes: design.indexes.map((item) => ({ ...item, columns: rename(item.columns) })),
          foreignKeys: design.foreignKeys.map((item) => ({ ...item, columns: rename(item.columns) })),
        };
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
