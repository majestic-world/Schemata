mod commands;
mod window_chrome;

use projectcat_core::{ConnectionStore, KeyringSecrets, QueryHistory, Sessions};
use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            let store = ConnectionStore::open(&dir.join("connections.json"), Box::new(KeyringSecrets))?;
            app.manage(Sessions::new(store));
            app.manage(commands::Operations::default());
            if let Some(window) = app.get_webview_window("main") {
                window_chrome::match_theme(&window);
            }
            app.manage(QueryHistory::open(&app.path().app_data_dir()?.join("history.json"))?);
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running ProjectCat");
}
