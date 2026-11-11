import { invoke } from "@tauri-apps/api/core";
import type {
  AppError,
  ConnectParams,
  ConnectionConfig,
  DatabaseInfo,
  ObjectInfo,
  SaveOutcome,
  ScriptResult,
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
};
