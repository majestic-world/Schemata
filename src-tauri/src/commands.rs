//! Thin command layer: maps UI calls to core services. No business logic here.

use projectcat_core::{
    AppError, ChangeError, RowChange, CharsetInfo, ConnectParams, ConnectionConfig, CreateDatabase, DatabaseInfo, Filter, ObjectInfo, Page,
    PageRequest, SaveOutcome, ScriptResult, Sessions, TableInfo, connect,
};
use tauri::State;

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

#[tauri::command]
pub async fn execute_script(
    sessions: State<'_, Sessions>,
    id: String,
    db: Option<String>,
    sql: String,
    row_limit: usize,
    execution_id: Option<String>,
) -> CmdResult<ScriptResult> {
    let driver = sessions.driver(&id).await?;
    driver.execute_script(db.as_deref(), &sql, row_limit, execution_id.as_deref()).await
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
