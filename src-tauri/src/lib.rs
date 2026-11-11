mod commands;
mod window_chrome;

use schematadb_core::{ConnectionStore, KeyringSecrets, QueryHistory, Sessions};
use std::path::Path;
use tauri::Manager;
use tauri_plugin_window_state::StateFlags;

/// App identifier used while the app was called ProjectCat.
const LEGACY_IDENTIFIER: &str = "dev.projectcat.app";

/// Copies `path` from the ProjectCat-era app directory (a sibling named after the old identifier) when it does
/// not exist yet, so saved connections and history survive the rename.
fn adopt_legacy_file(path: &Path) -> std::io::Result<()> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else { return Ok(()) };
    let Some(legacy) = dir.parent().map(|root| root.join(LEGACY_IDENTIFIER).join(name)) else { return Ok(()) };
    if path.exists() || !legacy.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    std::fs::copy(legacy, path).map(|_| ())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        // The frontend owns initial visibility; restoring it would reveal an unfinished webview.
        .plugin(tauri_plugin_window_state::Builder::default().with_state_flags(StateFlags::all() & !StateFlags::VISIBLE).build())
        .plugin(tauri_plugin_decorum::init())
        .setup(|app| {
            let connections = app.path().app_config_dir()?.join("connections.json");
            let history = app.path().app_data_dir()?.join("history.json");
            adopt_legacy_file(&connections)?;
            adopt_legacy_file(&history)?;
            let store = ConnectionStore::open(&connections, Box::new(KeyringSecrets))?;
            app.manage(Sessions::new(store));
            app.manage(commands::Operations::default());
            if let Some(window) = app.get_webview_window("main") {
                window_chrome::apply(&window)?;
                // Window-state restores the last size first; always start maximized on top of it so un-maximizing
                // still returns to the remembered size.
                window.maximize()?;
            }
            app.manage(QueryHistory::open(&history)?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_connections,
            commands::save_connection,
            commands::duplicate_connection,
            commands::delete_connection,
            commands::test_connection,
            commands::open_connection,
            commands::close_connection,
            commands::open_connection_ids,
            commands::list_databases,
            commands::list_objects,
            commands::execute_script,
            commands::cancel_execution,
            commands::table_info,
            commands::create_database,
            commands::drop_database,
            commands::list_charsets,
            commands::attach_database,
            commands::fetch_page,
            commands::count_rows,
            commands::apply_changes,
            commands::table_design,
            commands::plan_table,
            commands::apply_ddl,
            commands::drop_object,
            commands::truncate_table,
            commands::rename_table,
            commands::fetch_more,
            commands::list_columns,
            commands::list_history,
            commands::read_text_file,
            commands::write_text_file,
            commands::cancel_operation,
            commands::export_data,
            commands::preview_csv,
            commands::import_csv,
            commands::run_sql_file,
            commands::backup_database,
            commands::copy_table,
            commands::suggest_generators,
            commands::preview_generated,
            commands::generate_data,
        ])
        .run(tauri::generate_context!())
        .expect("error while running SchemataDb");
}
