export const formatCount = (n: number | null | undefined) => (n == null ? "—" : n.toLocaleString("en-US"));
export const orDash = (v: string | null | undefined) => (v == null || v === "" ? "—" : v);
