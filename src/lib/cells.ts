import type { CellValue } from "../api/types";

export type CellDisplay = { text: string; tone: "null" | "number" | "text" | "bytes" };

const formatBytes = (n: number) => (n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(1)} KB` : `${(n / 1048576).toFixed(1)} MB`);

export function displayCell(v: CellValue): CellDisplay {
  switch (v.type) {
    case "null":
      return { text: "NULL", tone: "null" };
    case "int":
    case "float":
      return { text: String(v.value), tone: "number" };
    case "bigInt":
    case "decimal":
      return { text: v.value, tone: "number" };
    case "bytes":
      return { text: `0x${v.value.previewHex}${v.value.len * 2 > v.value.previewHex.length ? "…" : ""} (${formatBytes(v.value.len)})`, tone: "bytes" };
    case "text":
    case "dateTime":
    case "json":
      return { text: v.value, tone: "text" };
    default: {
      const never: never = v;
      return never;
    }
  }
}

/** Plain-text form for the clipboard; NULL becomes an empty field. */
export function cellText(v: CellValue): string {
  return v.type === "null" ? "" : displayCell(v).text;
}

/** Tab-separated rows; tabs and newlines inside values are flattened to spaces. */
export function toTsv(rows: CellValue[][]): string {
  return rows.map((r) => r.map((v) => cellText(v).replace(/[\t\r\n]+/g, " ")).join("\t")).join("\n");
}
