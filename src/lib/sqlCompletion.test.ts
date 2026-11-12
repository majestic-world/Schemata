import { describe, expect, it } from "vitest";
import { CompletionContext, type CompletionResult } from "@codemirror/autocomplete";
import { EditorState } from "@codemirror/state";
import type { Engine } from "../api/types";
import { columnCompletionSource, statementTables } from "./sqlCompletion";

const databases: Record<string, [string, string[]][]> = {
  shop: [["products", ["id", "name", "price"]], ["Orders", ["id", "product_id", "total"]], ["odd`name", ["odd_col"]], ["items", ["unit-price", "qty"]]],
  archive: [["products", ["archived_at"]]],
};

async function complete(doc: string, { explicit = false, engine = "mySql" as Engine } = {}) {
  const pos = doc.indexOf("|");
  const state = EditorState.create({ doc: doc.replace("|", "") });
  const source = columnCompletionSource(engine, "shop", async (database) => {
    const tables = databases[database];
    if (!tables) throw new Error(`Unknown database ${database}`);
    return tables;
  });
  return (await source(new CompletionContext(state, pos, explicit))) as CompletionResult | null;
}
const labels = (result: CompletionResult | null) => result?.options.map((o) => o.label) ?? null;

describe("statement tables", () => {
  it("reads FROM lists, joins, aliases and database-qualified names", () => {
    expect(statementTables("SELECT * FROM `shop`.products p, orders AS o JOIN users u ON u.id = o.id WHERE id = 1")).toEqual([
      { database: "shop", table: "products" },
      { database: null, table: "orders" },
      { database: null, table: "users" },
    ]);
  });

  it("reads UPDATE and INSERT INTO targets, not subqueries", () => {
    expect(statementTables("UPDATE products SET price = 1 WHERE id IN (SELECT product_id FROM orders)").map((t) => t.table)).toEqual(["products", "orders"]);
    expect(statementTables("INSERT INTO products (").map((t) => t.table)).toEqual(["products"]);
  });

  it("decodes doubled delimiters in quoted names", () => {
    expect(statementTables('SELECT * FROM `odd``name`, "a""b", [c]]d]').map((t) => t.table)).toEqual(["odd`name", 'a"b', "c]d"]);
  });

  it("ignores names inside strings and comments", () => {
    expect(statementTables("SELECT 'from users' FROM /* from orders */ products").map((t) => t.table)).toEqual(["products"]);
  });
});

describe("column completion", () => {
  it("offers the columns of the statement's tables after WHERE", async () => {
    expect(labels(await complete("SELECT * from products where i|"))).toEqual(["id", "name", "price"]);
  });

  it("matches tables case-insensitively and merges columns of several tables", async () => {
    expect(labels(await complete("select * from products join orders on |", { explicit: true }))).toEqual(["id", "name", "price", "product_id", "total"]);
  });

  it("reads a database-qualified table from that database", async () => {
    expect(labels(await complete("select * from archive.products where a|"))).toEqual(["archived_at"]);
    expect(labels(await complete("select * from missing.products where a|"))).toBeNull();
  });

  it("finds tables whose quoted names contain an escaped delimiter", async () => {
    expect(labels(await complete("select * from `odd``name` where o|"))).toEqual(["odd_col"]);
  });

  it("quotes inserted names that are not plain identifiers, per dialect", async () => {
    const apply = async (engine: Engine) => (await complete("select * from items where u|", { engine }))?.options.find((o) => o.label === "unit-price")?.apply;
    expect(await apply("mySql")).toBe("`unit-price`");
    expect(await apply("sqlite")).toBe('"unit-price"');
    expect((await complete("select * from items where q|"))?.options.find((o) => o.label === "qty")?.apply).toBe("qty");
  });

  it("uses only the statement at the cursor", async () => {
    expect(labels(await complete("select * from items;\nselect * from products where n|"))).toEqual(["id", "name", "price"]);
  });

  it("stays out of strings, comments and qualified names", async () => {
    expect(await complete("select * from products where name = 'n|")).toBeNull();
    expect(await complete("select * from products -- n|")).toBeNull();
    expect(await complete("select * from products p where p.n|")).toBeNull();
  });

  it("offers nothing for unknown tables or before typing", async () => {
    expect(await complete("select * from missing where i|")).toBeNull();
    expect(await complete("select * from products where |")).toBeNull();
  });
});
