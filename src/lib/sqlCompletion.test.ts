import { describe, expect, it } from "vitest";
import { CompletionContext, type CompletionResult } from "@codemirror/autocomplete";
import { EditorState } from "@codemirror/state";
import { columnCompletionSource, statementTables } from "./sqlCompletion";

const schema = { products: ["id", "name", "price"], Orders: ["id", "product_id", "total"], users: ["email"] };

async function complete(doc: string, explicit = false) {
  const pos = doc.indexOf("|");
  const state = EditorState.create({ doc: doc.replace("|", "") });
  const result = await columnCompletionSource("mySql", schema)(new CompletionContext(state, pos, explicit));
  return (result as CompletionResult | null)?.options.map((o) => o.label) ?? null;
}

describe("statement tables", () => {
  it("reads FROM lists, joins, aliases and qualified names", () => {
    expect(statementTables("SELECT * FROM `shop`.products p, orders AS o JOIN users u ON u.id = o.id WHERE id = 1")).toEqual(["products", "orders", "users"]);
  });

  it("reads UPDATE and INSERT INTO targets, not subqueries", () => {
    expect(statementTables("UPDATE products SET price = 1 WHERE id IN (SELECT product_id FROM orders)")).toEqual(["products", "orders"]);
    expect(statementTables("INSERT INTO products (")).toEqual(["products"]);
  });

  it("ignores names inside strings and comments", () => {
    expect(statementTables("SELECT 'from users' FROM /* from orders */ products")).toEqual(["products"]);
  });
});

describe("column completion", () => {
  it("offers the columns of the statement's tables after WHERE", async () => {
    expect(await complete("SELECT * from products where i|")).toEqual(["id", "name", "price"]);
  });

  it("matches tables case-insensitively and merges columns of several tables", async () => {
    expect(await complete("select * from products join orders on |", true)).toEqual(["id", "name", "price", "product_id", "total"]);
  });

  it("uses only the statement at the cursor", async () => {
    expect(await complete("select * from users;\nselect * from products where n|")).toEqual(["id", "name", "price"]);
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
