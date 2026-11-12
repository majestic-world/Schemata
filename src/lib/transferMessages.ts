import type { IncomingForeignKey, ScriptRunReport, SqlFileScan } from "../api/types";
import type { ConfirmRequest } from "../store/confirm";

/** Text an export writes for NULL by default, and the import reads back as NULL, so NULL and "" stay distinct. */
export const DEFAULT_NULL_TEXT = "\\N";

/** The closing line of a SQL file run: errors, or a transaction the script left open (rolled back), are not success. */
export function scriptSummary(report: ScriptRunReport, stopOnError: boolean): { ok: boolean; text: string } {
  const rolledBack = report.openTransactionRolledBack
    ? "The script left a transaction open: it was rolled back, so its uncommitted changes were discarded."
    : null;
  if (report.failed.length > 0) {
    const errors = stopOnError ? "Stopped at the first error." : "Finished with errors.";
    return { ok: false, text: rolledBack ? `${errors} ${rolledBack}` : errors };
  }
  if (rolledBack) return { ok: false, text: rolledBack };
  return { ok: true, text: "Finished successfully." };
}

/**
 * Confirmation before restoring a backup: restoring into an existing database replaces its tables, and a file that
 * switches databases (`USE`/`CREATE DATABASE`) sends its statements elsewhere. `null` when nothing existing is at risk.
 */
export function restoreConfirmation(database: string, existing: boolean, scan: SqlFileScan): ConfirmRequest | null {
  const elsewhere = scan.databases.filter((d) => d !== database);
  if (!existing && elsewhere.length === 0) return null;
  const parts: string[] = [];
  if (existing) parts.push(`Tables in the backup replace the tables with the same name in ${database}.`);
  if (elsewhere.length > 0) {
    parts.push(
      `The file switches to or creates ${elsewhere.length === 1 ? "the database" : "the databases"} ${elsewhere.join(", ")}: ` +
        `its statements after that run there, not in ${database}.`,
    );
  }
  return { title: `Restore into ${database}`, message: parts.join(" "), confirmLabel: "Restore", danger: true, requireText: database };
}

/** Confirmation before "Delete existing rows first": names the table and every table whose rows the delete reaches. */
export function emptyFirstConfirmation(table: string, incoming: IncomingForeignKey[]): ConfirmRequest {
  const referencing = incoming.map((fk) => `${fk.table} (ON DELETE ${fk.onDelete || "NO ACTION"})`);
  const message =
    `Every existing row of ${table} is deleted before the new rows are generated.` +
    (referencing.length > 0
      ? ` Foreign keys of ${referencing.join(", ")} reference it: their rows are deleted or changed with it, or the delete fails.`
      : "");
  return { title: `Delete all rows of ${table}`, message, confirmLabel: "Delete and generate", danger: true };
}
