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

export type ColumnInfo = {
  name: string;
  dataType: string;
  nullable: boolean;
  default: string | null;
  autoIncrement: boolean;
  primaryKey: boolean;
  comment: string | null;
};
export type IndexInfo = { name: string; columns: string[]; unique: boolean; primary: boolean };
export type ForeignKeyInfo = {
  name: string;
  columns: string[];
  refTable: string;
  refColumns: string[];
  onUpdate: string;
  onDelete: string;
};
export type RowIdentity =
  | { kind: "primaryKey"; columns: string[] }
  | { kind: "uniqueKey"; columns: string[] }
  | { kind: "rowId" }
  | { kind: "none" };
export type TableInfo = {
  name: string;
  kind: ObjectKind;
  columns: ColumnInfo[];
  indexes: IndexInfo[];
  foreignKeys: ForeignKeyInfo[];
  engine: string | null;
  charset: string | null;
  collation: string | null;
  rows: number | null;
  created: string | null;
  modified: string | null;
  comment: string | null;
  createSql: string;
  rowIdentity: RowIdentity;
};
export type CreateDatabase = { name: string; charset: string | null; collation: string | null };
export type CharsetInfo = { name: string; defaultCollation: string; collations: string[] };

export type SortSpec = { column: string; desc: boolean };
export type FilterOp = "eq" | "notEq" | "lt" | "lte" | "gt" | "gte" | "contains" | "startsWith" | "isNull" | "isNotNull";
export type Filter = { column: string; op: FilterOp; value: CellValue | null };
export type PageRequest = { offset: number; limit: number; sort: SortSpec | null; filters: Filter[] };
export type Page = { columns: ColumnMeta[]; rows: CellValue[][] };

export type ColumnValue = { column: string; value: CellValue };
export type RowChange =
  | { kind: "update"; key: CellValue[]; values: ColumnValue[] }
  | { kind: "insert"; values: ColumnValue[] }
  | { kind: "delete"; key: CellValue[] };
export type ChangeError = { index: number | null; error: AppError };

export type ColumnDesign = {
  originalName: string | null;
  name: string;
  dataType: string;
  nullable: boolean;
  default: string | null;
  autoIncrement: boolean;
  primaryKey: boolean;
  comment: string | null;
};
export type IndexDesign = { name: string; columns: string[]; unique: boolean };
export type ForeignKeyDesign = {
  name: string;
  columns: string[];
  refTable: string;
  refColumns: string[];
  onUpdate: string;
  onDelete: string;
};
export type TableOptions = { engine: string | null; charset: string | null; collation: string | null; comment: string | null };
export type TableDesign = {
  name: string;
  columns: ColumnDesign[];
  indexes: IndexDesign[];
  foreignKeys: ForeignKeyDesign[];
  options: TableOptions;
};

export type HistoryEntry = { connectionId: string; sql: string; ok: boolean; durationMs: number; at: number };
