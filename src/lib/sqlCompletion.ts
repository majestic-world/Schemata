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
      i = until(c === "[" ? "]" : c, i + 1);
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

const unquote = (t: Token) => (t.kind === "identifier" ? t.text.slice(1, t.open ? undefined : -1) : t.text);

/** Keywords that end a FROM/UPDATE table list. */
const END_OF_LIST: Record<string, true> = {
  where: true, group: true, having: true, order: true, limit: true, offset: true, union: true, intersect: true, except: true,
  set: true, on: true, using: true, values: true, select: true, window: true, for: true, returning: true,
};
/** Keywords followed by a table name; FROM and UPDATE take a comma-separated list. */
const TABLE_AFTER: Record<string, { list: boolean }> = { from: { list: true }, update: { list: true }, join: { list: false }, into: { list: false } };

/** Names of the tables a statement reads or writes (FROM lists, JOINs, UPDATE, INSERT INTO), in order. */
export function statementTables(sql: string): string[] {
  const tokens = tokenize(sql).filter((t) => t.kind !== "comment");
  const tables: string[] = [];
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
    } else if (expect && (t.kind === "word" || t.kind === "identifier")) {
      // `schema.table`: the last part names the table.
      let name = unquote(t);
      while (tokens[i + 1]?.text === "." && (tokens[i + 2]?.kind === "word" || tokens[i + 2]?.kind === "identifier")) {
        name = unquote(tokens[i + 2]);
        i += 2;
      }
      if (!tables.includes(name)) tables.push(name);
      expect = false;
    } else if (expect) {
      // A subquery or anything else in place of a name.
      expect = false;
    }
  }
  return tables;
}

/**
 * Completes unqualified column names (`WHERE i|`) from the tables of the statement at the cursor. Qualified names
 * (`p.|`) are left to lang-sql's schema completion, which already resolves tables and aliases.
 */
export function columnCompletionSource(engine: Engine, schema: Readonly<Record<string, readonly string[]>>): CompletionSource {
  const byName = new Map(Object.entries(schema).map(([table, columns]) => [table.toLowerCase(), { table, columns }]));
  return (context) => {
    const word = context.matchBefore(/\w*/);
    if (!word || (word.from === word.to && !context.explicit)) return null;
    const doc = context.state.doc.toString();
    if (word.from > 0 && /[.`"[]/.test(doc[word.from - 1])) return null;
    const statement = statementAt(doc, context.pos, engine);
    if (!statement) return null;
    const text = doc.slice(statement.from, Math.max(statement.to, context.pos));
    const offset = context.pos - statement.from;
    const tokens = tokenize(text);
    if (tokens.some((t) => (t.kind === "string" || t.kind === "comment") && t.from < offset && (offset < t.to || t.open))) return null;
    const options = new Map<string, Completion>();
    for (const name of statementTables(text)) {
      const table = byName.get(name.toLowerCase());
      if (!table) continue;
      for (const column of table.columns) {
        const existing = options.get(column);
        options.set(column, existing ? { ...existing, detail: `${existing.detail}, ${table.table}` } : { label: column, type: "property", detail: table.table, boost: 1 });
      }
    }
    return options.size ? { from: word.from, options: [...options.values()], validFor: /^\w*$/ } : null;
  };
}
