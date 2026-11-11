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
};
