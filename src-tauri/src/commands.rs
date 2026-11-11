//! Thin command layer: maps UI calls to core services. No business logic here.

use projectcat_core::{
    AppError, ChangeError, HistoryEntry, QueryHistory, StatementOutcome, ObjectKind, RowChange, TableDesign, CharsetInfo, ConnectParams, ConnectionConfig, CreateDatabase, DatabaseInfo, Filter, ObjectInfo, Page,
    PageRequest, SaveOutcome, ScriptResult, Sessions, TableInfo, connect,
};
use projectcat_core::{
    CancellationToken, CsvImport, CsvPreview, CsvReadOptions, ExportFormat, ExportReport, ExportSource, ImportReport, Progress,
    ProgressFn, ScriptRunReport,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};

type CmdResult<T> = Result<T, AppError>;

#[tauri::command]
pub fn list_connections(sessions: State<'_, Sessions>) -> Vec<ConnectionConfig> {
    sessions.store().list()
}

#[tauri::command]
pub fn save_connection(sessions: State<'_, Sessions>, config: ConnectionConfig, password: Option<String>) -> CmdResult<SaveOutcome> {
    sessions.store().save(&config, password.as_deref())
}

#[tauri::command]
pub fn duplicate_connection(sessions: State<'_, Sessions>, id: String) -> CmdResult<ConnectionConfig> {
    sessions.store().duplicate(&id)
}

#[tauri::command]
pub async fn delete_connection(sessions: State<'_, Sessions>, id: String) -> CmdResult<()> {
    sessions.close(&id).await;
    sessions.store().delete(&id)
}

/// Opens a throwaway session and returns the server version. Without a typed password,
/// the one stored for `saved_id` is used.
#[tauri::command]
pub async fn test_connection(sessions: State<'_, Sessions>, mut params: ConnectParams, saved_id: Option<String>) -> CmdResult<String> {
    if let (None, Some(id)) = (&params.password, saved_id) {
        params.password = sessions.store().password(&id)?;
    }
    Ok(connect(&params).await?.server_version())
}

#[tauri::command]
pub async fn open_connection(sessions: State<'_, Sessions>, id: String, password: Option<String>) -> CmdResult<String> {
    sessions.open(&id, password).await
}

#[tauri::command]
pub async fn close_connection(sessions: State<'_, Sessions>, id: String) -> CmdResult<()> {
    sessions.close(&id).await;
    Ok(())
}

#[tauri::command]
pub async fn open_connection_ids(sessions: State<'_, Sessions>) -> CmdResult<Vec<String>> {
    Ok(sessions.open_ids().await)
}

#[tauri::command]
pub async fn list_databases(sessions: State<'_, Sessions>, id: String) -> CmdResult<Vec<DatabaseInfo>> {
    sessions.with_driver(&id, |d| async move { d.list_databases().await }).await
}

#[tauri::command]
pub async fn list_objects(sessions: State<'_, Sessions>, id: String, db: String) -> CmdResult<Vec<ObjectInfo>> {
    sessions.with_driver(&id, |d| { let db = db.clone(); async move { d.list_objects(&db).await } }).await
}

/// Runs a script and records it in the query history.
#[tauri::command]
pub async fn execute_script(
    sessions: State<'_, Sessions>,
    history: State<'_, QueryHistory>,
    id: String,
    db: Option<String>,
    sql: String,
    row_limit: usize,
    execution_id: Option<String>,
) -> CmdResult<ScriptResult> {
    let driver = sessions.driver(&id).await?;
    let result = driver.execute_script(db.as_deref(), &sql, row_limit, execution_id.as_deref()).await?;
    let duration = result
        .outcomes
        .iter()
        .map(|o| match o {
            StatementOutcome::Rows { duration_ms, .. } | StatementOutcome::Affected { duration_ms, .. } => *duration_ms,
        })
        .sum();
    history.record(&id, sql.trim(), result.error.is_none(), duration)?;
    Ok(result)
}

#[tauri::command]
pub async fn cancel_execution(sessions: State<'_, Sessions>, id: String, execution_id: String) -> CmdResult<()> {
    sessions.driver(&id).await?.cancel(&execution_id).await
}

#[tauri::command]
pub async fn table_info(sessions: State<'_, Sessions>, id: String, db: String, name: String) -> CmdResult<TableInfo> {
    sessions.with_driver(&id, |d| { let (db, name) = (db.clone(), name.clone()); async move { d.table_info(&db, &name).await } }).await
}

#[tauri::command]
pub async fn create_database(sessions: State<'_, Sessions>, id: String, spec: CreateDatabase) -> CmdResult<()> {
    sessions.driver(&id).await?.create_database(&spec).await
}

#[tauri::command]
pub async fn drop_database(sessions: State<'_, Sessions>, id: String, name: String) -> CmdResult<()> {
    sessions.driver(&id).await?.drop_database(&name).await
}

#[tauri::command]
pub async fn list_charsets(sessions: State<'_, Sessions>, id: String) -> CmdResult<Vec<CharsetInfo>> {
    sessions.with_driver(&id, |d| async move { d.list_charsets().await }).await
}

#[tauri::command]
pub async fn attach_database(sessions: State<'_, Sessions>, id: String, path: String, alias: String) -> CmdResult<()> {
    sessions.driver(&id).await?.attach_database(&path, &alias).await
}

#[tauri::command]
pub async fn fetch_page(sessions: State<'_, Sessions>, id: String, db: String, table: String, req: PageRequest) -> CmdResult<Page> {
    sessions
        .with_driver(&id, |d| { let (db, table, req) = (db.clone(), table.clone(), req.clone()); async move { d.fetch_page(&db, &table, &req).await } })
        .await
}

#[tauri::command]
pub async fn count_rows(sessions: State<'_, Sessions>, id: String, db: String, table: String, filters: Vec<Filter>) -> CmdResult<u64> {
    sessions
        .with_driver(&id, |d| { let (db, table, filters) = (db.clone(), table.clone(), filters.clone()); async move { d.count_rows(&db, &table, &filters).await } })
        .await
}

#[tauri::command]
pub async fn apply_changes(sessions: State<'_, Sessions>, id: String, db: String, table: String, changes: Vec<RowChange>) -> Result<u64, ChangeError> {
    let driver = sessions.driver(&id).await?;
    driver.apply_changes(&db, &table, &changes).await
}

#[tauri::command]
pub async fn table_design(sessions: State<'_, Sessions>, id: String, db: String, table: String) -> CmdResult<TableDesign> {
    sessions.with_driver(&id, |d| { let (db, table) = (db.clone(), table.clone()); async move { d.table_design(&db, &table).await } }).await
}

#[tauri::command]
pub async fn plan_table(sessions: State<'_, Sessions>, id: String, db: String, original: Option<TableDesign>, design: TableDesign) -> CmdResult<Vec<String>> {
    sessions.driver(&id).await?.plan_table(&db, original.as_ref(), &design).await
}

#[tauri::command]
pub async fn apply_ddl(sessions: State<'_, Sessions>, id: String, db: String, statements: Vec<String>) -> CmdResult<()> {
    sessions.driver(&id).await?.apply_ddl(&db, &statements).await
}

#[tauri::command]
pub async fn drop_object(sessions: State<'_, Sessions>, id: String, db: String, name: String, kind: ObjectKind) -> CmdResult<()> {
    sessions.driver(&id).await?.drop_object(&db, &name, kind).await
}

#[tauri::command]
pub async fn truncate_table(sessions: State<'_, Sessions>, id: String, db: String, name: String) -> CmdResult<()> {
    sessions.driver(&id).await?.truncate_table(&db, &name).await
}

#[tauri::command]
pub async fn rename_table(sessions: State<'_, Sessions>, id: String, db: String, from: String, to: String) -> CmdResult<()> {
    sessions.driver(&id).await?.rename_table(&db, &from, &to).await
}

#[tauri::command]
pub async fn fetch_more(sessions: State<'_, Sessions>, id: String, db: Option<String>, statement: String, offset: u64, limit: u64) -> CmdResult<Page> {
    sessions.driver(&id).await?.fetch_more(db.as_deref(), &statement, offset, limit).await
}

#[tauri::command]
pub async fn list_columns(sessions: State<'_, Sessions>, id: String, db: String) -> CmdResult<Vec<(String, Vec<String>)>> {
    sessions.with_driver(&id, |d| { let db = db.clone(); async move { d.list_columns(&db).await } }).await
}

#[tauri::command]
pub fn list_history(history: State<'_, QueryHistory>, id: String) -> Vec<HistoryEntry> {
    history.list(&id)
}

#[tauri::command]
pub async fn read_text_file(path: String) -> CmdResult<String> {
    Ok(tokio::fs::read_to_string(&path).await?)
}

#[tauri::command]
pub async fn write_text_file(path: String, contents: String) -> CmdResult<()> {
    Ok(tokio::fs::write(&path, contents).await?)
}

/// Cancellation tokens of running long operations, keyed by operation id.
#[derive(Default)]
pub struct Operations(Mutex<HashMap<String, CancellationToken>>);

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationProgress {
    operation_id: String,
    #[serde(flatten)]
    progress: Progress,
}

/// Registers an operation and returns its progress reporter (as `operation-progress` events) and cancel token.
fn begin(app: &AppHandle, ops: &Operations, operation_id: &str) -> (ProgressFn, CancellationToken) {
    let token = CancellationToken::new();
    ops.0.lock().unwrap().insert(operation_id.to_owned(), token.clone());
    let (app, id) = (app.clone(), operation_id.to_owned());
    let progress: ProgressFn = Arc::new(move |progress| {
        let _ = app.emit("operation-progress", OperationProgress { operation_id: id.clone(), progress });
    });
    (progress, token)
}

fn end(ops: &Operations, operation_id: &str) {
    ops.0.lock().unwrap().remove(operation_id);
}

#[tauri::command]
pub fn cancel_operation(ops: State<'_, Operations>, operation_id: String) {
    if let Some(token) = ops.0.lock().unwrap().get(&operation_id) {
        token.cancel();
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn export_data(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    ops: State<'_, Operations>,
    id: String,
    db: String,
    source: ExportSource,
    format: ExportFormat,
    path: PathBuf,
    operation_id: String,
) -> CmdResult<ExportReport> {
    let driver = sessions.driver(&id).await?;
    let (progress, cancel) = begin(&app, &ops, &operation_id);
    let result = projectcat_core::export(driver.as_ref(), &db, &source, &format, &path, progress, cancel).await;
    end(&ops, &operation_id);
    if result.is_err() {
        let _ = std::fs::remove_file(&path);
    }
    result
}

#[tauri::command]
pub async fn preview_csv(path: PathBuf, options: CsvReadOptions) -> CmdResult<CsvPreview> {
    tokio::task::spawn_blocking(move || projectcat_core::preview_csv(&path, &options, 20))
        .await
        .map_err(|e| AppError::io(e.to_string()))?
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn import_csv(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    ops: State<'_, Operations>,
    id: String,
    db: String,
    table: String,
    path: PathBuf,
    spec: CsvImport,
    operation_id: String,
) -> CmdResult<ImportReport> {
    let driver = sessions.driver(&id).await?;
    let (progress, cancel) = begin(&app, &ops, &operation_id);
    let result = projectcat_core::import_csv(driver.as_ref(), &db, &table, &path, &spec, progress, cancel).await;
    end(&ops, &operation_id);
    result
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn run_sql_file(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    ops: State<'_, Operations>,
    id: String,
    db: String,
    path: PathBuf,
    stop_on_error: bool,
    operation_id: String,
) -> CmdResult<ScriptRunReport> {
    let driver = sessions.driver(&id).await?;
    let (progress, cancel) = begin(&app, &ops, &operation_id);
    let result = projectcat_core::run_sql_file(driver.as_ref(), &db, &path, stop_on_error, progress, cancel).await;
    end(&ops, &operation_id);
    result
}
