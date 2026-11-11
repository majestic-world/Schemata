//! Thin command layer: maps UI calls to core services. No business logic here.

use projectcat_core::{
    AppError, ConnectParams, ConnectionConfig, DatabaseInfo, ObjectInfo, SaveOutcome, ScriptResult, Sessions, connect,
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
