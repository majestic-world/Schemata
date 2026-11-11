import { invoke } from "@tauri-apps/api/core";
import type {
  AppError,
  ConnectParams,
  ConnectionConfig,
  DatabaseInfo,
  ObjectInfo,
  SaveOutcome,
  ScriptResult,
  TableInfo,
  CreateDatabase,
  CharsetInfo,
  Filter,
  Page,
  PageRequest,
  RowChange,
  ChangeError,
  ObjectKind,
  TableDesign,
  HistoryEntry,
  ExportSource,
  ExportFormat,
  CsvReadOptions,
  CsvPreview,
  CsvImport,
  ImportReport,
  ScriptRunReport,
  CopyTableTarget,
  ColumnGenerator,
  GenerateSpec,
} from "./types";

export function isAppError(e: unknown): e is AppError {
  return typeof e === "object" && e !== null && "kind" in e && "message" in e;
}

export function toAppError(e: unknown): AppError {
  if (isAppError(e)) return e;
  return { kind: "io", message: e instanceof Error ? e.message : String(e), sqlState: null };
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw toAppError(e);
  }
}

export const api = {
  listConnections: () => call<ConnectionConfig[]>("list_connections"),
  saveConnection: (config: ConnectionConfig, password: string | null) =>
    call<SaveOutcome>("save_connection", { config, password }),
  duplicateConnection: (id: string) => call<ConnectionConfig>("duplicate_connection", { id }),
  deleteConnection: (id: string) => call<void>("delete_connection", { id }),
  testConnection: (params: ConnectParams, savedId: string | null) => call<string>("test_connection", { params, savedId }),
  openConnection: (id: string, password: string | null) => call<string>("open_connection", { id, password }),
  closeConnection: (id: string) => call<void>("close_connection", { id }),
  openConnectionIds: () => call<string[]>("open_connection_ids"),
  listDatabases: (id: string) => call<DatabaseInfo[]>("list_databases", { id }),
  listObjects: (id: string, db: string) => call<ObjectInfo[]>("list_objects", { id, db }),
  executeScript: (id: string, db: string | null, sql: string, rowLimit: number, executionId: string | null) =>
    call<ScriptResult>("execute_script", { id, db, sql, rowLimit, executionId }),
  cancelExecution: (id: string, executionId: string) => call<void>("cancel_execution", { id, executionId }),
  tableInfo: (id: string, db: string, name: string) => call<TableInfo>("table_info", { id, db, name }),
  createDatabase: (id: string, spec: CreateDatabase) => call<void>("create_database", { id, spec }),
  dropDatabase: (id: string, name: string) => call<void>("drop_database", { id, name }),
  listCharsets: (id: string) => call<CharsetInfo[]>("list_charsets", { id }),
  attachDatabase: (id: string, path: string, alias: string) => call<void>("attach_database", { id, path, alias }),
  fetchPage: (id: string, db: string, table: string, req: PageRequest) => call<Page>("fetch_page", { id, db, table, req }),
  countRows: (id: string, db: string, table: string, filters: Filter[]) => call<number>("count_rows", { id, db, table, filters }),
  tableDesign: (id: string, db: string, table: string) => call<TableDesign>("table_design", { id, db, table }),
  planTable: (id: string, db: string, original: TableDesign | null, design: TableDesign) =>
    call<string[]>("plan_table", { id, db, original, design }),
  applyDdl: (id: string, db: string, statements: string[]) => call<void>("apply_ddl", { id, db, statements }),
  dropObject: (id: string, db: string, name: string, kind: ObjectKind) => call<void>("drop_object", { id, db, name, kind }),
  truncateTable: (id: string, db: string, name: string) => call<void>("truncate_table", { id, db, name }),
  renameTable: (id: string, db: string, from: string, to: string) => call<void>("rename_table", { id, db, from, to }),
  fetchMore: (id: string, db: string | null, statement: string, offset: number, limit: number) =>
    call<Page>("fetch_more", { id, db, statement, offset, limit }),
  listColumns: (id: string, db: string) => call<[string, string[]][]>("list_columns", { id, db }),
  listHistory: (id: string) => call<HistoryEntry[]>("list_history", { id }),
  readTextFile: (path: string) => call<string>("read_text_file", { path }),
  writeTextFile: (path: string, contents: string) => call<void>("write_text_file", { path, contents }),
  exportData: (id: string, db: string, source: ExportSource, format: ExportFormat, path: string, operationId: string) =>
    call<{ rows: number }>("export_data", { id, db, source, format, path, operationId }),
  previewCsv: (path: string, options: CsvReadOptions) => call<CsvPreview>("preview_csv", { path, options }),
  importCsv: (id: string, db: string, table: string, path: string, spec: CsvImport, operationId: string) =>
    call<ImportReport>("import_csv", { id, db, table, path, spec, operationId }),
  runSqlFile: (id: string, db: string, path: string, stopOnError: boolean, operationId: string) =>
    call<ScriptRunReport>("run_sql_file", { id, db, path, stopOnError, operationId }),
  backupDatabase: (id: string, db: string, path: string, options: { structure: boolean; data: boolean }, operationId: string) =>
    call<{ tables: number; rows: number }>("backup_database", { id, db, path, options, operationId }),
  copyTable: (sourceId: string, sourceDb: string, table: string, targetId: string, target: CopyTableTarget, operationId: string) =>
    call<{ rows: number }>("copy_table", { sourceId, sourceDb, table, targetId, target, operationId }),
  suggestGenerators: (id: string, db: string, table: string) => call<ColumnGenerator[]>("suggest_generators", { id, db, table }),
  previewGenerated: (id: string, db: string, table: string, columns: ColumnGenerator[]) =>
    call<Page>("preview_generated", { id, db, table, columns }),
  generateData: (id: string, db: string, table: string, spec: GenerateSpec, operationId: string) =>
    call<{ rows: number }>("generate_data", { id, db, table, spec, operationId }),
  cancelOperation: (operationId: string) => call<void>("cancel_operation", { operationId }),
  /** Rejects with a `ChangeError` (not an `AppError`) when a change fails. */
  applyChanges: async (id: string, db: string, table: string, changes: RowChange[]): Promise<number> => {
    try {
      return await invoke<number>("apply_changes", { id, db, table, changes });
    } catch (e) {
      const err = e as Partial<ChangeError>;
      throw err && typeof err === "object" && "error" in err ? (err as ChangeError) : ({ index: null, error: toAppError(e) } satisfies ChangeError);
    }
  },
};
