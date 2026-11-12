import type { Completion, CompletionSource } from "@codemirror/autocomplete";
import type { Engine } from "../api/types";
import { statementAt } from "./statements";

type Token = {
  kind: "word" | "identifier" | "string" | "comment" | "punct";
  text: string;
  from: number;
  to: number;
  /** A literal or comment still open at the end of the text (the cursor after it is inside). */
  open: boolean;
};

/** Quote-, comment- and identifier-aware tokens of one statement; unterminated literals run to its end. */
function tokenize(sql: string): Token[] {
  const tokens: Token[] = [];
  let i = 0;
  while (i < sql.length) {
    const c = sql[i];
    const start = i;
    if (/\s/.test(c)) {
      i++;
      continue;
    }
    let kind: Token["kind"];
    let open = false;
    const until = (close: string, from: number) => {
      const end = sql.indexOf(close, from);
      open = end < 0;
      return open ? sql.length : end + close.length;
    };
    if ((c === "-" && sql[i + 1] === "-") || c === "#") {
      i = until("\n", i);
      kind = "comment";
    } else if (c === "/" && sql[i + 1] === "*") {
      i = until("*/", i + 2);
      kind = "comment";
    } else if (c === "'") {
      // `''` and `\'` stay inside the literal.
      open = true;
      for (i++; i < sql.length; i++) {
        if (sql[i] === "\\") i++;
        else if (sql[i] === "'" && sql[i + 1] === "'") i++;
        else if (sql[i] === "'") {
          open = false;
          i++;
          break;
        }
      }
      i = Math.min(i, sql.length);
      kind = "string";
    } else if (c === "`" || c === '"' || c === "[") {
      // A doubled closing delimiter (``a``b``, "a""b", [a]]b]) is part of the name.
      const close = c === "[" ? "]" : c;
      open = true;
      for (i++; i < sql.length; i++) {
        if (sql[i] !== close) continue;
        if (sql[i + 1] === close) i++;
        else {
          open = false;
          i++;
          break;
        }
      }
      kind = "identifier";
    } else if (/\w/.test(c)) {
      while (i < sql.length && /\w/.test(sql[i])) i++;
      kind = "word";
    } else {
      i++;
      kind = "punct";
    }
    tokens.push({ kind, text: sql.slice(start, i), from: start, to: i, open });
  }
  return tokens;
}

/** The name a token stands for: quotes removed and doubled delimiters decoded. */
function unquote(t: Token): string {
  if (t.kind !== "identifier") return t.text;
  const close = t.text[0] === "[" ? "]" : t.text[0];
  return t.text.slice(1, t.open ? undefined : -1).replaceAll(close + close, close);
}

/** Keywords that end a FROM/UPDATE table list. */
const END_OF_LIST: Record<string, true> = {
  where: true, group: true, having: true, order: true, limit: true, offset: true, union: true, intersect: true, except: true,
  set: true, on: true, using: true, values: true, select: true, window: true, for: true, returning: true,
};
/** Keywords followed by a table name; FROM and UPDATE take a comma-separated list. */
const TABLE_AFTER: Record<string, { list: boolean }> = { from: { list: true }, update: { list: true }, join: { list: false }, into: { list: false } };

/** A table a statement names; `database` is set only when the statement qualifies it (`shop.products`). */
export type TableRef = { database: string | null; table: string };

/** The tables a statement reads or writes (FROM lists, JOINs, UPDATE, INSERT INTO), in order. */
export function statementTables(sql: string): TableRef[] {
  const tokens = tokenize(sql).filter((t) => t.kind !== "comment");
  const tables: TableRef[] = [];
  const isName = (t: Token | undefined) => t?.kind === "word" || t?.kind === "identifier";
  let expect = false;
  let inList = false;
  for (let i = 0; i < tokens.length; i++) {
    const t = tokens[i];
    const keyword = t.kind === "word" ? t.text.toLowerCase() : null;
    if (keyword && Object.hasOwn(TABLE_AFTER, keyword)) {
      expect = true;
      inList = TABLE_AFTER[keyword].list;
    } else if (keyword && Object.hasOwn(END_OF_LIST, keyword)) {
      expect = inList = false;
    } else if (t.text === "," && inList) {
      expect = true;
    } else if (expect && isName(t)) {
      let ref: TableRef = { database: null, table: unquote(t) };
      if (tokens[i + 1]?.text === "." && isName(tokens[i + 2])) {
        ref = { database: ref.table, table: unquote(tokens[i + 2]) };
        i += 2;
      }
      if (!tables.some((r) => r.database === ref.database && r.table === ref.table)) tables.push(ref);
      expect = false;
    } else if (expect) {
      // A subquery or anything else in place of a name.
      expect = false;
    }
  }
  return tables;
}

/** Quotes a column name for insertion when it is not a plain identifier (`unit-price`, `order date`). */
export function quoteColumn(engine: Engine, name: string): string {
  if (/^[A-Za-z_][A-Za-z0-9_$]*$/.test(name)) return name;
  const quote = engine === "sqlite" ? '"' : "`";
  return quote + name.replaceAll(quote, quote + quote) + quote;
}

/** Columns of every table in a database, as the backend's `list_columns` returns them. */
export type ColumnLoader = (database: string) => Promise<readonly (readonly [string, readonly string[]])[]>;

/**
 * Completes unqualified column names (`WHERE i|`) from the tables of the statement at the cursor, reading each table
 * from the database it is qualified with, else from `defaultDatabase`. Qualified names (`p.|`) are left to
 * lang-sql's schema completion, which already resolves tables and aliases.
 */
export function columnCompletionSource(engine: Engine, defaultDatabase: string | null, loadColumns: ColumnLoader): CompletionSource {
  const columnsOf = async (database: string) => {
    try {
      return new Map((await loadColumns(database)).map(([table, columns]) => [table.toLowerCase(), { table, columns }]));
    } catch {
      // An unknown or unreachable database simply has nothing to suggest.
      return new Map<string, { table: string; columns: readonly string[] }>();
    }
  };
  return async (context) => {
    const word = context.matchBefore(/\w*/);
    if (!word || (word.from === word.to && !context.explicit)) return null;
    const doc = context.state.doc.toString();
    if (word.from > 0 && /[.`"[]/.test(doc[word.from - 1])) return null;
    const statement = statementAt(doc, context.pos, engine);
    if (!statement) return null;
    const text = doc.slice(statement.from, Math.max(statement.to, context.pos));
    const offset = context.pos - statement.from;
    if (tokenize(text).some((t) => (t.kind === "string" || t.kind === "comment") && t.from < offset && (offset < t.to || t.open))) return null;
    const loaded = new Map<string, ReturnType<typeof columnsOf>>();
    const options = new Map<string, Completion>();
    for (const ref of statementTables(text)) {
      const database = ref.database ?? defaultDatabase;
      if (database === null) continue;
      if (!loaded.has(database)) loaded.set(database, columnsOf(database));
      const table = (await loaded.get(database)!).get(ref.table.toLowerCase());
      if (!table) continue;
      const label = ref.database ? `${ref.database}.${table.table}` : table.table;
      for (const column of table.columns) {
        const existing = options.get(column);
        options.set(column, existing
          ? { ...existing, detail: `${existing.detail}, ${label}` }
          : { label: column, apply: quoteColumn(engine, column), type: "property", detail: label, boost: 1 });
      }
    }
    if (context.aborted || options.size === 0) return null;
    return { from: word.from, options: [...options.values()], validFor: /^\w*$/ };
  };
}
