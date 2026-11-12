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

/** Text the cell editor starts with; NULL (shown as a placeholder) and binary previews start empty. */
export function editorText(v: CellValue): string {
  return v.type === "null" || v.type === "bytes" ? "" : String(v.value);
}

/**
 * Value the cell editor saves, or null when it is not an edit. `typed` is the editor's text, or null when the user
 * never changed it, so opening and closing the editor never writes. Text controls report every line break as `\n`:
 * text equal to the start once its line breaks are normalized is unchanged, and a value written with `\r\n` keeps it.
 */
export function editedCell(start: string, typed: string | null): CellValue | null {
  if (typed === null || typed === start.replace(/\r\n?/g, "\n")) return null;
  return { type: "text", value: start.includes("\r\n") ? typed.replace(/\r?\n/g, "\r\n") : typed };
}

export type EditorKeyAction = "commit" | "newline" | "cancel" | "picker" | null;

/**
 * What a key does in the cell editor: Enter and Tab save, Shift+Enter or Alt+Enter insert a line break, Esc
 * cancels, Alt+↓ opens the date picker (when there is one). Keys typed while an IME composes are left to it.
 */
export function editorKeyAction(
  e: { key: string; shiftKey: boolean; altKey: boolean; ctrlKey: boolean; metaKey: boolean; isComposing: boolean },
  hasPicker: boolean,
): EditorKeyAction {
  if (e.isComposing) return null;
  if (e.key === "ArrowDown" && e.altKey) return hasPicker ? "picker" : null;
  if (e.key === "Enter") {
    if (e.ctrlKey || e.metaKey) return null;
    return e.shiftKey || e.altKey ? "newline" : "commit";
  }
  if (e.key === "Tab") return "commit";
  if (e.key === "Escape") return "cancel";
  return null;
}
