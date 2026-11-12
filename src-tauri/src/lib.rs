mod commands;
mod window_chrome;

use schematadb_core::{ConnectionStore, KeyringSecrets, QueryHistory, Sessions};
use std::fs::{File, OpenOptions};
use std::path::Path;
use tauri::Manager;
use tauri_plugin_window_state::StateFlags;
#[cfg(windows)]
use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2Settings3, ICoreWebView2Settings4};
#[cfg(windows)]
use windows_core::Interface;

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

/// Held by Tauri state until shutdown, before reading or migrating any persistent files.
struct PersistenceLock {
    _file: File,
}

fn lock_app_directory(directory: &Path) -> std::io::Result<PersistenceLock> {
    std::fs::create_dir_all(directory)?;
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(directory.join("instance.lock"))?;
    file.try_lock().map_err(|error| {
        std::io::Error::other(format!(
            "Cannot lock Schemata's configuration directory. Another Schemata instance may already be running; close it before starting this instance. {error}"
        ))
    })?;
    Ok(PersistenceLock { _file: file })
}

/// Debug builds (`pnpm tauri dev`, `cargo run`) are the test version. They get their own identifier, which Tauri
/// turns into separate config/data directories, WebView2 profile and window state, and which also names their
/// credential-store service. The installed release never sees a test connection and vice versa.
const DEV_BUILD: bool = cfg!(debug_assertions);

fn runtime_identifier(base: &str, dev_build: bool) -> String {
    if dev_build { format!("{base}.dev") } else { base.to_owned() }
}

pub fn run() {
    let mut context = tauri::generate_context!();
    context.config_mut().identifier = runtime_identifier(&context.config().identifier, DEV_BUILD);
    tauri::Builder::default()
        // Registered first: a second launch (keyed on the runtime identifier, so dev and release stay apart) exits
        // and brings the running window forward. The instance lock in setup still guards the data directory.
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
            if !DEV_BUILD {
                adopt_legacy_file(&connections)?;
                adopt_legacy_file(&history)?;
            }
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::double_click_interval,
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

#[cfg(test)]
mod tests {
    use super::{lock_app_directory, runtime_identifier};

    #[test]
    fn dev_and_release_builds_never_share_an_identifier() {
        assert_eq!(runtime_identifier("dev.schematadb.app", false), "dev.schematadb.app");
        assert_eq!(runtime_identifier("dev.schematadb.app", true), "dev.schematadb.app.dev");
    }

    #[test]
    fn persistence_lock_rejects_another_instance_until_shutdown() {
        let directory = std::env::temp_dir().join(format!("schemata-instance-lock-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let connections = directory.join("connections.json");
        std::fs::write(&connections, b"existing connections").unwrap();
        let first = lock_app_directory(&directory).unwrap();
        let error = lock_app_directory(&directory).err().expect("a second instance must not acquire the lock");
        assert!(error.to_string().contains("Another Schemata instance"));
        assert_eq!(std::fs::read(&connections).unwrap(), b"existing connections");
        drop(first);
        let next = lock_app_directory(&directory).unwrap();
        drop(next);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
