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
export type IndexKind = "normal" | "unique" | "fulltext" | "spatial";
export type IndexMethod = "btree" | "hash";
export type IndexInfo = { name: string; columns: string[]; kind: IndexKind; method: IndexMethod | null; comment: string | null; primary: boolean };
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
export type IndexDesign = { name: string; columns: string[]; kind: IndexKind; method: IndexMethod | null; comment: string | null };
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

export type TextEncoding = "utf8" | "utf8Bom" | "windows1252";
export type CsvOptions = { delimiter: string; quote: string; header: boolean; encoding: TextEncoding; nullText: string };
export type SqlExportOptions = { includeCreate: boolean; tableName: string | null };
export type ExportFormat = { format: "csv"; options: CsvOptions } | { format: "json" } | { format: "sql"; options: SqlExportOptions };
export type ExportSource = { kind: "table"; table: string } | { kind: "query"; sql: string };
export type CsvReadOptions = { delimiter: string; quote: string; hasHeader: boolean; encoding: TextEncoding };
export type CsvPreview = { headers: string[]; rows: string[][] };
export type ColumnMapping = { source: number; target: string };
export type CsvImport = { options: CsvReadOptions; mapping: ColumnMapping[]; emptyAsNull: boolean; skipErrors: boolean };
export type ImportReport = { inserted: number; failed: { line: number; message: string }[]; failedCount: number };
export type ScriptRunReport = { executed: number; failed: { index: number; statement: string; message: string }[] };
export type OperationProgress = { operationId: string; done: number; total: number | null };
export type CopyTableTarget = { db: string; name: string; data: boolean };
export type GeneratorKind =
  | "skip" | "null" | "sequence" | "integer" | "decimal" | "boolean"
  | "firstName" | "lastName" | "fullName" | "email" | "username" | "phone"
  | "company" | "city" | "state" | "country" | "address" | "zipCode"
  | "url" | "uuid" | "ipAddress" | "word" | "title" | "sentence" | "paragraph" | "slug" | "code" | "color" | "hash"
  | "date" | "dateTime" | "time" | "year" | "enumValue" | "json" | "bytes" | "foreignKey";
export type ColumnGenerator = { column: string; kind: GeneratorKind };
export type GenerateSpec = { rows: number; columns: ColumnGenerator[]; emptyFirst: boolean };
