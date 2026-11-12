import type { Engine } from "../api/types";

/**
 * One statement of a script: `from`/`to` bound its trimmed text (leading comments included), `code` is where its
 * code starts (null when it has none), `end` is just past its delimiter.
 */
type Chunk = { start: number; from: number; to: number; end: number; code: number | null; delimiter: string };

/** Uppercases ASCII letters only, like Rust's `to_ascii_uppercase`. */
const asciiUpper = (s: string) => s.replace(/[a-z]+/g, (w) => w.toUpperCase());

/**
 * Splits a script like the backend's `splitter.rs` does (quotes, comments, MySQL `DELIMITER`, SQLite trigger
 * bodies), keeping offsets. Chunks without code (blank lines, comments) are kept so a cursor can fall in them.
 */
function chunks(sql: string, engine: Engine): Chunk[] {
  const mysql = engine !== "sqlite";
  const out: Chunk[] = [];
  let delimiter = ";";
  let state: { kind: "normal" } | { kind: "block" } | { kind: "quote"; close: string } = { kind: "normal" };
  let start = 0;
  let code: number | null = null;
  let word = "";
  let words: string[] = [];
  let depth = 0;

  const finishWord = () => {
    if (!word) return;
    const upper = asciiUpper(word);
    word = "";
    if (words.length < 4) words.push(upper);
    if (!mysql && isSqliteTrigger(words)) {
      if (upper === "BEGIN" || upper === "CASE") depth++;
      else if (upper === "END") depth--;
    }
  };
  const push = (to: number, end: number) => {
    const text = sql.slice(start, to);
    const lead = text.length - text.trimStart().length;
    const trail = text.length - text.trimEnd().length;
    out.push({ start, from: start + lead, to: Math.max(start + lead, to - trail), end, code, delimiter });
    start = end;
    code = null;
    words = [];
    depth = 0;
  };

  let lineStart = 0;
  while (lineStart < sql.length) {
    const nl = sql.indexOf("\n", lineStart);
    const lineEnd = nl < 0 ? sql.length : nl + 1;
    const trimmed = sql.slice(lineStart, lineEnd).trim();
    if (mysql && state.kind === "normal" && code === null && trimmed.length > 10 && trimmed.slice(0, 10).toUpperCase() === "DELIMITER ") {
      delimiter = trimmed.slice(10).trim();
      start = lineEnd;
      lineStart = lineEnd;
      continue;
    }
    let i = lineStart;
    while (i < lineEnd) {
      const c = sql[i];
      const next = i + 1 < lineEnd ? sql[i + 1] : undefined;
      if (state.kind === "block") {
        if (c === "*" && next === "/") {
          state = { kind: "normal" };
          i++;
        }
      } else if (state.kind === "quote") {
        const close = state.close;
        if (mysql && c === "\\" && close !== "`") {
          if (next !== undefined) i++;
        } else if (c === close) {
          if (next === close && close !== "]") i++;
          else state = { kind: "normal" };
        }
      } else {
        const after = i + 2 < lineEnd ? sql[i + 2] : undefined;
        if ((c === "-" && next === "-" && (!mysql || after === undefined || /\s/.test(after))) || (mysql && c === "#")) {
          // Comments stay in the statement text but do not count as code.
          finishWord();
          break;
        }
        if (c === "/" && next === "*") {
          finishWord();
          // MySQL executable comments (/*! ... */) are code.
          if (mysql && after === "!") code ??= i;
          state = { kind: "block" };
          i += 2;
          continue;
        }
        if (i + delimiter.length <= lineEnd && asciiUpper(sql.slice(i, i + delimiter.length)) === asciiUpper(delimiter)) {
          // The pending word (e.g. a trigger's END) must be counted first.
          finishWord();
          if (depth <= 0) {
            push(i, i + delimiter.length);
            i += delimiter.length;
            continue;
          }
        }
        const close = c === "'" || c === '"' || c === "`" ? c : c === "[" && !mysql ? "]" : null;
        if (close) {
          finishWord();
          code ??= i;
          state = { kind: "quote", close };
          words.push("\0");
          i++;
          continue;
        }
        if (/[\p{L}\p{N}_]/u.test(c)) word += c;
        else finishWord();
        if (!/\s/.test(c)) code ??= i;
      }
      i++;
    }
    lineStart = lineEnd;
  }
  finishWord();
  if (start < sql.length || out.length === 0) push(sql.length, sql.length);
  return out;
}

function isSqliteTrigger(words: string[]) {
  const [a, b, c] = words;
  return a === "CREATE" && (b === "TRIGGER" || ((b === "TEMP" || b === "TEMPORARY") && c === "TRIGGER"));
}

/**
 * The statement the cursor is in, as a script that runs exactly that statement (wrapped in its `DELIMITER` when the
 * script changed it), and its range for highlighting. Before a statement's code, the cursor belongs to the statement
 * ending on its line, else to the statement starting on its line; on other blank or comment lines it is null.
 */
export function statementAt(sql: string, cursor: number, engine: Engine): { from: number; to: number; script: string } | null {
  const all = chunks(sql, engine);
  const at = all.findIndex((c) => c.start <= cursor && cursor <= c.end);
  if (at < 0) return null;
  let chunk: Chunk | undefined = all[at];
  if (chunk.code === null || cursor < chunk.code) {
    const previous = all[at - 1];
    const startsHere = chunk.code !== null && !sql.slice(cursor, chunk.code).includes("\n");
    chunk = previous?.code != null && previous.end === chunk.start && !sql.slice(previous.end, cursor).includes("\n")
      ? previous
      : startsHere ? chunk : undefined;
    if (!chunk) return null;
  }
  const text = sql.slice(chunk.from, chunk.to);
  return { from: chunk.from, to: chunk.to, script: chunk.delimiter === ";" ? text : `DELIMITER ${chunk.delimiter}\n${text}\n${chunk.delimiter}\n` };
}

const WRITES = ["INSERT", "UPDATE", "DELETE", "REPLACE", "INTO", "FOR", "RETURNING", "CALL", "LOCK", "NEXTVAL", "SETVAL"];

/**
 * Mirror of the backend's `is_read_only_query`: whether running a statement again (to load more rows or export
 * them) cannot repeat side effects. Conservative; the backend refuses the others as well.
 */
export function isReadOnlyQuery(sql: string): boolean {
  let rest = sql.trimStart();
  for (;;) {
    if (rest.startsWith("--") || rest.startsWith("#")) {
      const nl = rest.indexOf("\n");
      rest = nl < 0 ? "" : rest.slice(nl + 1).trimStart();
    } else if (rest.startsWith("/*")) {
      const close = rest.indexOf("*/", 2);
      rest = close < 0 ? "" : rest.slice(close + 2).trimStart();
    } else {
      break;
    }
  }
  rest = rest.replace(/^[(\s]+/, "");
  const first = asciiUpper(rest.split(/[^A-Za-z0-9_]/)[0]);
  const words = asciiUpper(sql).split(/[^A-Za-z0-9_]/);
  const writes = WRITES.some((w) => words.includes(w));
  switch (first) {
    case "SELECT":
    case "WITH":
    case "VALUES":
    case "TABLE":
      return !writes;
    case "SHOW":
    case "DESCRIBE":
    case "DESC":
    case "EXPLAIN":
      return true;
    default:
      return false;
  }
}
