mod commands;
mod persistence;
mod window_chrome;

use persistence::{InstallationProfile, lock_app_directory};
use schematadb_core::{ConnectionStore, KeyringSecrets, QueryHistory, SavedQueries, Sessions};
use tauri::Manager;
use tauri_plugin_window_state::StateFlags;
#[cfg(windows)]
use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2Settings3, ICoreWebView2Settings4};
#[cfg(windows)]
use windows_core::Interface;

pub fn run() {
    let mut context = tauri::generate_context!();
    let profile = InstallationProfile::current().expect("Failed to resolve Schemata's installation profile");
    profile.configure(context.config_mut());
    tauri::Builder::default()
        // Each installation has its own instance identity and data-directory lock.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        // The frontend owns initial visibility; restoring it would reveal an unfinished webview.
        .plugin(tauri_plugin_window_state::Builder::default().with_state_flags(StateFlags::all() & !StateFlags::VISIBLE).build())
        .plugin(tauri_plugin_decorum::init())
        .setup(|app| {
            let config_directory = app.path().app_config_dir()?;
            app.manage(lock_app_directory(&config_directory)?);
            let connections = config_directory.join("connections.json");
            let history = app.path().app_data_dir()?.join("history.json");
            let saved_queries = app.path().app_data_dir()?.join("queries.json");
            // No migrations or fallback reads: a missing profile always starts empty.
            let store = ConnectionStore::open(&connections, Box::new(KeyringSecrets::new(app.config().identifier.clone())))?;
            app.manage(Sessions::new(store));
            app.manage(commands::Operations::default());
            if let Some(window) = app.get_webview_window("main") {
                #[cfg(windows)]
                window.with_webview(|webview| {
                    // SAFETY: Tauri runs this callback on the UI thread with a live WebView2 controller.
                    // Browser-owned menus and shortcuts are disabled; DOM contextmenu and key events still reach the
                    // page. Browser shortcuts (Ctrl+N new window, Ctrl+F find, Ctrl+P print, zoom…) would otherwise
                    // swallow the app's own, so Ctrl+N never reached the frontend. Autofill is off because its
                    // "information saved" bubble popped up over the toolbar after typing in the search box.
                    unsafe {
                        webview.controller().CoreWebView2().and_then(|core| core.Settings()).and_then(|settings| {
                            settings.SetAreDefaultContextMenusEnabled(false)?;
                            settings.cast::<ICoreWebView2Settings3>()?.SetAreBrowserAcceleratorKeysEnabled(false)?;
                            // Runtimes without ICoreWebView2Settings4 keep autofill; Wry skips it there too.
                            if let Ok(autofill) = settings.cast::<ICoreWebView2Settings4>() {
                                autofill.SetIsGeneralAutofillEnabled(false)?;
                                autofill.SetIsPasswordAutosaveEnabled(false)?;
                            }
                            Ok(())
                        })
                    }
                    .expect("Failed to disable the WebView2 default menus and shortcuts");
                })?;
                window_chrome::apply(&window)?;
                // Window-state restores the last size first; always start maximized on top of it so un-maximizing
                // still returns to the remembered size.
                window.maximize()?;
            }
            app.manage(QueryHistory::open(&history)?);
            app.manage(SavedQueries::open(&saved_queries)?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::double_click_interval,
            commands::set_window_theme,
            commands::open_app_data_directory,
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
            commands::apply_table_design,
            commands::incoming_foreign_keys,
            commands::drop_object,
            commands::empty_table,
            commands::truncate_table,
            commands::rename_table,
            commands::fetch_more,
            commands::close_editor_session,
            commands::list_columns,
            commands::list_history,
            commands::list_saved_queries,
            commands::save_query,
            commands::delete_saved_query,
            commands::read_text_file,
            commands::write_text_file,
            commands::cancel_operation,
            commands::export_data,
            commands::preview_csv,
            commands::import_csv,
            commands::run_sql_file,
            commands::scan_sql_file,
            commands::backup_database,
            commands::copy_table,
            commands::suggest_generators,
            commands::preview_generated,
            commands::generate_data,
        ])
        .run(context)
        .expect("error while running Schemata");
}

