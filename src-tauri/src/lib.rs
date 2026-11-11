mod commands;

use projectcat_core::{ConnectionStore, KeyringSecrets, Sessions};
use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            let store = ConnectionStore::open(&dir.join("connections.json"), Box::new(KeyringSecrets))?;
            app.manage(Sessions::new(store));
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running ProjectCat");
}
