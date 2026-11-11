import type { ColumnMapping } from "../api/types";

/** Target for each CSV column; `null` skips the column. */
export type MappingDraft = (string | null)[];

const normalize = (s: string) => s.toLowerCase().replace(/[^a-z0-9]/g, "");

/** Maps CSV headers to table columns with the same name, ignoring case, spaces and punctuation. */
export function autoMap(headers: string[], targets: string[]): MappingDraft {
  const byKey = new Map(targets.map((t) => [normalize(t), t]));
  const used = new Set<string>();
  return headers.map((h) => {
    const t = byKey.get(normalize(h));
    if (!t || used.has(t)) return null;
    used.add(t);
    return t;
  });
}

/** Assigns `target` to CSV column `index`, unassigning any other column that pointed at it. */
export function assign(draft: MappingDraft, index: number, target: string | null): MappingDraft {
  return draft.map((t, i) => (i === index ? target : target !== null && t === target ? null : t));
}

export function toMapping(draft: MappingDraft): ColumnMapping[] {
  return draft.flatMap((target, source) => (target ? [{ source, target }] : []));
}
