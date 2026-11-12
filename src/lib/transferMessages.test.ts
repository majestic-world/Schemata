import { describe, expect, it } from "vitest";
import { emptyFirstConfirmation, restoreConfirmation, scriptSummary } from "./transferMessages";

describe("SQL file run summary", () => {
  it("reports a transaction the script left open as rolled back, not as success", () => {
    const summary = scriptSummary({ executed: 3, failed: [], openTransactionRolledBack: true }, true);
    expect(summary.ok).toBe(false);
    expect(summary.text).toContain("rolled back");
    expect(scriptSummary({ executed: 3, failed: [], openTransactionRolledBack: false }, true)).toEqual({ ok: true, text: "Finished successfully." });
  });

  it("keeps the rollback notice next to statement errors", () => {
    const failed = [{ index: 1, statement: "INSERT", message: "duplicate" }];
    const summary = scriptSummary({ executed: 1, failed, openTransactionRolledBack: true }, false);
    expect(summary.text).toMatch(/^Finished with errors\. .*rolled back/);
  });
});

describe("restore confirmation", () => {
  it("asks to type the database name before replacing an existing database's tables", () => {
    const request = restoreConfirmation("shop", true, { databases: [] });
    expect(request?.requireText).toBe("shop");
    expect(request?.danger).toBe(true);
  });

  it("names the databases a file switches to, even when restoring into a new database", () => {
    const request = restoreConfirmation("shop_copy", false, { databases: ["shop", "archive"] });
    expect(request?.message).toContain("shop, archive");
    expect(request?.requireText).toBe("shop_copy");
  });

  it("needs no confirmation for a new database the file does not leave", () => {
    expect(restoreConfirmation("fresh", false, { databases: ["fresh"] })).toBeNull();
  });
});

describe("delete existing rows confirmation", () => {
  it("names the table and every table referencing it with its ON DELETE action", () => {
    const request = emptyFirstConfirmation("customer", [
      { table: "orders", name: "fk_orders_customer", onDelete: "CASCADE" },
      { table: "invoices", name: "fk_invoices_customer", onDelete: "SET NULL" },
    ]);
    expect(request.message).toContain("customer");
    expect(request.message).toContain("orders (ON DELETE CASCADE), invoices (ON DELETE SET NULL)");
    expect(request.danger).toBe(true);
  });
});
