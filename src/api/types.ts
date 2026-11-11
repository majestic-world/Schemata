export type Engine = "mySql" | "mariaDb" | "sqlite";

export const ENGINE_LABEL: Record<Engine, string> = { mySql: "MySQL", mariaDb: "MariaDB", sqlite: "SQLite" };

export type ConnectionConfig = {
  id: string;
  name: string;
  engine: Engine;
  color: string | null;
  host: string | null;
  port: number | null;
  user: string | null;
  database: string | null;
  filePath: string | null;
  savePassword: boolean;
};

export type ConnectParams = {
  engine: Engine;
  host: string | null;
  port: number | null;
  user: string | null;
  password: string | null;
  database: string | null;
  filePath: string | null;
  createFile: boolean;
};

export type SaveOutcome = { config: ConnectionConfig; passwordSaved: boolean };

export type ErrorKind = "connection" | "authentication" | "sql" | "cancelled" | "io" | "validation";
export type AppError = { kind: ErrorKind; message: string; sqlState: string | null };

export type DatabaseInfo = { name: string };
export type ObjectKind = "table" | "view";
export type ObjectInfo = {
  name: string;
  kind: ObjectKind;
  rows: number | null;
  created: string | null;
  modified: string | null;
  comment: string | null;
};

export type CellValue =
  | { type: "null" }
  | { type: "int"; value: number }
  | { type: "bigInt"; value: string }
  | { type: "float"; value: number }
  | { type: "decimal"; value: string }
  | { type: "text"; value: string }
  | { type: "bytes"; value: { previewHex: string; len: number } }
  | { type: "dateTime"; value: string }
  | { type: "json"; value: string };

export type ColumnMeta = { name: string; typeName: string; hidden: boolean };

export type StatementOutcome =
  | { kind: "rows"; statement: string; columns: ColumnMeta[]; rows: CellValue[][]; hasMore: boolean; durationMs: number }
  | { kind: "affected"; statement: string; rowsAffected: number; lastInsertId: number | null; durationMs: number };

export type StatementError = { index: number; statement: string; error: AppError };
export type ScriptResult = { outcomes: StatementOutcome[]; error: StatementError | null };
