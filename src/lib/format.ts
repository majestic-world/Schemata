export const formatCount = (n: number | null | undefined) => (n == null ? "—" : n.toLocaleString("en-US"));
export const orDash = (v: string | null | undefined) => (v == null || v === "" ? "—" : v);
/** `1 row`, `2 rows`: the count with the noun inflected for it. */
export const plural = (n: number, noun: string, pluralNoun = `${noun}s`) => `${formatCount(n)} ${n === 1 ? noun : pluralNoun}`;
