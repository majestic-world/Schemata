/** Keys typed further apart than this start a new query, like Windows Explorer. */
export const TYPEAHEAD_RESET_MS = 1000;

/**
 * Windows Explorer-style type-to-select. Keys typed in quick succession build a prefix query;
 * a single or repeated letter (`p`, `pp`) cycles through the names starting with it, while a
 * longer prefix (`prod`) keeps the current name when it still matches.
 *
 * The returned function yields the index to select, `-1` when nothing matches, or `undefined`
 * when the key does not belong to the query (a space that would start one) and should be left alone.
 */
export function createTypeahead(resetMs = TYPEAHEAD_RESET_MS) {
  let query = "";
  let last = -Infinity;
  return (key: string, now: number, names: readonly string[], current: number): number | undefined => {
    const fresh = now - last > resetMs;
    if (fresh && key.trim() === "") return undefined;
    query = fresh ? key.toLocaleLowerCase() : query + key.toLocaleLowerCase();
    last = now;
    const repeated = [...query].every((c) => c === query[0]);
    const prefix = repeated ? query[0] : query;
    const start = current < 0 ? 0 : current + (repeated ? 1 : 0);
    for (let i = 0; i < names.length; i++) {
      const index = (start + i) % names.length;
      if (names[index].toLocaleLowerCase().startsWith(prefix)) return index;
    }
    return -1;
  };
}
