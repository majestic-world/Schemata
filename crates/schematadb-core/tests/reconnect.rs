mod common;

use common::TestDb;
use futures::poll;
use schematadb_core::{
    AppError, ConnectParams, ConnectionConfig, ConnectionStore, Driver, Engine, ErrorKind, MemorySecrets, Sessions, connect,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use tokio::runtime::{Builder, Runtime};
use tokio::task::JoinHandle;

/// `t`'s server as an open saved connection, with its id; the directory holds the connection store.
async fn server_sessions(t: &TestDb) -> (Sessions, String, tempfile::TempDir) {
    let server = t.server.expect("server engine");
    let dir = tempfile::tempdir().unwrap();
    let store = ConnectionStore::open(&dir.path().join("c.json"), Box::new(MemorySecrets::default())).unwrap();
    let mut config = ConnectionConfig::new("srv", t.engine);
    config.host = Some("127.0.0.1".into());
    config.port = Some(server.port);
    config.user = Some("root".into());
    config.database = Some(t.db.clone());
    config.save_password = true;
    store.save(&config, Some(common::ROOT_PASSWORD)).unwrap();
    let sessions = Sessions::new(store);
    sessions.open(&config.id, None).await.unwrap();
    (sessions, config.id, dir)
}

/// A session whose server connections are all killed keeps working on the next action.
async fn reconnects_after_connection_loss(t: TestDb) {
    let server = t.server.expect("server engine");
    let (sessions, id, _dir) = server_sessions(&t).await;

    let admin = connect(&server.params(None)).await.unwrap();
    let ids = admin
        .execute_script(None, &format!("SELECT ID FROM information_schema.PROCESSLIST WHERE DB = '{}'", t.db), 100, None)
        .await
        .unwrap();
    let schematadb_core::StatementOutcome::Rows { rows, .. } = &ids.outcomes[0] else { panic!("expected rows") };
    assert!(!rows.is_empty());
    for row in rows {
        let schematadb_core::CellValue::Int(id) = row[0] else { panic!("id") };
        let _ = admin.execute_script(None, &format!("KILL {id}"), 1, None).await;
    }

    let objects = sessions
        .with_driver(&id, |d| {
            let db = t.db.clone();
            async move { d.list_objects(&db).await }
        })
        .await;
    assert!(objects.is_ok(), "{objects:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn mysql_reconnects() {
    reconnects_after_connection_loss(TestDb::mysql().await).await
}

#[tokio::test(flavor = "multi_thread")]
async fn mariadb_reconnects() {
    let t = TestDb::mariadb().await;
    assert_eq!(t.engine, Engine::MariaDb);
    reconnects_after_connection_loss(t).await
}

/// Runs `sql` in the editor session `S`.
async fn run_in_editor(driver: &Arc<dyn Driver>, db: &str, sql: &str) -> Result<(), AppError> {
    let result = driver.execute_in_session("S", Some(db), sql, 10, None).await?;
    assert!(result.error.is_none(), "{:?}", result.error);
    Ok(())
}

enum Replacement {
    Reconnect,
    CloseAndReopen,
}

/// When the driver is replaced, an editor session that held a transaction reports once that it was lost, instead of
/// silently starting over (a later COMMIT would commit nothing).
async fn replaced_driver_reports_lost_editor_sessions(t: TestDb, replacement: Replacement) {
    let (sessions, id, _dir) = server_sessions(&t).await;
    let driver = sessions.driver(&id).await.unwrap();
    run_in_editor(&driver, &t.db, "CREATE TABLE lost (id INT PRIMARY KEY) ENGINE = InnoDB; BEGIN; INSERT INTO lost VALUES (1)")
        .await
        .unwrap();
    match replacement {
        Replacement::Reconnect => {
            let attempts = AtomicUsize::new(0);
            sessions
                .with_driver(&id, |d| {
                    let first = attempts.fetch_add(1, Ordering::SeqCst) == 0;
                    async move { if first { Err(AppError::connection("Connection lost")) } else { d.ping().await } }
                })
                .await
                .unwrap();
        }
        Replacement::CloseAndReopen => {
            sessions.close(&id).await;
            sessions.open(&id, None).await.unwrap();
        }
    }
    let replaced = sessions.driver(&id).await.unwrap();
    assert!(!Arc::ptr_eq(&driver, &replaced));
    drop(driver);
    let error = run_in_editor(&replaced, &t.db, "COMMIT").await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Connection, "{error:?}");
    assert!(error.message.contains("rolled back"), "{error:?}");
    // Reported once: the next run opens a fresh session, and the insert was never committed.
    let count = replaced.execute_in_session("S", Some(&t.db), "SELECT COUNT(*) FROM lost", 10, None).await.unwrap();
    let schematadb_core::StatementOutcome::Rows { rows, .. } = &count.outcomes[0] else { panic!("expected rows") };
    assert_eq!(rows, &[vec![schematadb_core::CellValue::Int(0)]]);
    run_in_editor(&replaced, &t.db, "SELECT 1").await.unwrap();
    // A session that never ran is not affected.
    replaced.execute_in_session("fresh", Some(&t.db), "SELECT 1", 10, None).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn mysql_reconnect_reports_lost_editor_sessions() {
    replaced_driver_reports_lost_editor_sessions(TestDb::mysql().await, Replacement::Reconnect).await
}

#[tokio::test(flavor = "multi_thread")]
async fn mariadb_reconnect_reports_lost_editor_sessions() {
    replaced_driver_reports_lost_editor_sessions(TestDb::mariadb().await, Replacement::Reconnect).await
}

#[tokio::test(flavor = "multi_thread")]
async fn mysql_reopen_reports_lost_editor_sessions() {
    replaced_driver_reports_lost_editor_sessions(TestDb::mysql().await, Replacement::CloseAndReopen).await
}

#[tokio::test(flavor = "multi_thread")]
async fn mariadb_reopen_reports_lost_editor_sessions() {
    replaced_driver_reports_lost_editor_sessions(TestDb::mariadb().await, Replacement::CloseAndReopen).await
}

/// Waiting too long for a pooled connection (SQLSTATE HYT00) does not replace the driver and its editor sessions.
#[tokio::test(flavor = "multi_thread")]
async fn pool_timeout_does_not_reconnect() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("timeout.sqlite").to_string_lossy().into_owned();
    drop(connect(&ConnectParams::sqlite(file.clone(), true)).await.unwrap());
    let store = ConnectionStore::open(&directory.path().join("connections.json"), Box::new(MemorySecrets::default())).unwrap();
    let mut config = ConnectionConfig::new("SQLite", Engine::Sqlite);
    config.file_path = Some(file);
    store.save(&config, None).unwrap();
    let sessions = Sessions::new(store);
    sessions.open(&config.id, None).await.unwrap();
    let before = sessions.driver(&config.id).await.unwrap();
    let attempts = AtomicUsize::new(0);
    let error = sessions
        .with_driver(&config.id, |_| {
            attempts.fetch_add(1, Ordering::SeqCst);
            async { Err::<(), _>(AppError { kind: ErrorKind::Connection, message: "Timed out".into(), sql_state: Some("HYT00".into()) }) }
        })
        .await
        .unwrap_err();
    assert_eq!(error.sql_state.as_deref(), Some("HYT00"));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert!(Arc::ptr_eq(&before, &sessions.driver(&config.id).await.unwrap()));
}

fn serialized_blocking_runtime() -> Runtime {
    Builder::new_current_thread().enable_all().max_blocking_threads(1).build().unwrap()
}

/// Holds the only blocking worker so the real SQLite connect cannot complete before the test acts.
fn occupy_blocking_worker() -> (mpsc::Sender<()>, JoinHandle<()>) {
    let (release, released) = mpsc::channel();
    let (started, ready) = mpsc::channel();
    let worker = tokio::task::spawn_blocking(move || {
        started.send(()).unwrap();
        let _ = released.recv();
    });
    ready.recv().unwrap();
    (release, worker)
}

enum ConcurrentAction {
    Close,
    Delete,
    Reopen,
}

async fn reconnect_does_not_reinstall_obsolete_session(action: ConcurrentAction) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("database.sqlite").to_string_lossy().into_owned();
    let created = connect(&ConnectParams::sqlite(file.clone(), true)).await.unwrap();
    created.execute_script(Some("main"), "CREATE TABLE retained (id INTEGER); INSERT INTO retained VALUES (42)", 10, None).await.unwrap();
    drop(created);
    let store = ConnectionStore::open(&directory.path().join("connections.json"), Box::new(MemorySecrets::default())).unwrap();
    let mut config = ConnectionConfig::new("SQLite", Engine::Sqlite);
    config.file_path = Some(file.clone());
    store.save(&config, None).unwrap();
    let sessions = Sessions::new(store);
    sessions.open(&config.id, None).await.unwrap();
    let attempts = Arc::new(AtomicUsize::new(0));
    let (release, worker) = occupy_blocking_worker();
    let mut reconnecting = Box::pin(sessions.with_driver(&config.id, |driver| {
        let attempts = attempts.clone();
        async move {
            if attempts.fetch_add(1, Ordering::SeqCst) == 0 { Err(AppError::connection("Connection lost")) } else { driver.ping().await }
        }
    }));
    assert!(poll!(reconnecting.as_mut()).is_pending(), "reconnect must be waiting for its actual SQLite connection");
    sessions.close(&config.id).await;
    if matches!(action, ConcurrentAction::Delete) {
        sessions.store().delete(&config.id).unwrap();
    }
    release.send(()).unwrap();
    worker.await.unwrap();
    let replacement = if matches!(action, ConcurrentAction::Reopen) {
        sessions.open(&config.id, None).await.unwrap();
        Some(sessions.driver(&config.id).await.unwrap())
    } else {
        None
    };
    let error = reconnecting.await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Connection);
    assert_eq!(attempts.load(Ordering::SeqCst), 1, "a cancelled reconnect must not retry the operation");
    if let Some(replacement) = replacement {
        assert!(Arc::ptr_eq(&sessions.driver(&config.id).await.unwrap(), &replacement), "the newer session must not be replaced");
        replacement.ping().await.unwrap();
    } else {
        assert!(sessions.open_ids().await.is_empty());
        assert!(sessions.driver(&config.id).await.is_err());
    }
    let retained = connect(&ConnectParams::sqlite(file, false)).await.unwrap();
    let data = retained.execute_script(Some("main"), "SELECT id FROM retained", 10, None).await.unwrap();
    let schematadb_core::StatementOutcome::Rows { rows, .. } = &data.outcomes[0] else { panic!("expected retained rows") };
    assert_eq!(rows, &[vec![schematadb_core::CellValue::Int(42)]]);
}

#[test]
fn reconnect_in_flight_cannot_resurrect_a_closed_session() {
    serialized_blocking_runtime().block_on(reconnect_does_not_reinstall_obsolete_session(ConcurrentAction::Close));
}

#[test]
fn reconnect_in_flight_cannot_resurrect_a_deleted_connection() {
    serialized_blocking_runtime().block_on(reconnect_does_not_reinstall_obsolete_session(ConcurrentAction::Delete));
}

#[test]
fn reconnect_in_flight_cannot_replace_a_newer_session() {
    serialized_blocking_runtime().block_on(reconnect_does_not_reinstall_obsolete_session(ConcurrentAction::Reopen));
}

#[test]
fn pending_open_rejects_destination_changes_and_cannot_survive_close() {
    serialized_blocking_runtime().block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("database.sqlite").to_string_lossy().into_owned();
        drop(connect(&ConnectParams::sqlite(file.clone(), true)).await.unwrap());
        let store = ConnectionStore::open(&directory.path().join("connections.json"), Box::new(MemorySecrets::default())).unwrap();
        let mut config = ConnectionConfig::new("SQLite", Engine::Sqlite);
        config.file_path = Some(file);
        store.save(&config, None).unwrap();
        let sessions = Sessions::new(store);
        let (release, worker) = occupy_blocking_worker();
        let mut opening = Box::pin(sessions.open(&config.id, None));
        assert!(poll!(opening.as_mut()).is_pending());
        let mut changed = config.clone();
        changed.file_path = Some(directory.path().join("different.sqlite").to_string_lossy().into_owned());
        let error = sessions.save_connection(&changed, None).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::Validation);
        assert_eq!(sessions.store().get(&config.id).unwrap().file_path, config.file_path);
        let mut renamed = config.clone();
        renamed.name = "Renamed while opening".into();
        sessions.save_connection(&renamed, None).await.unwrap();
        sessions.close(&config.id).await;
        release.send(()).unwrap();
        worker.await.unwrap();
        let error = opening.await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::Connection);
        assert!(sessions.open_ids().await.is_empty());
        assert!(sessions.driver(&config.id).await.is_err());
    });
}

#[test]
fn pending_open_does_not_install_an_outdated_destination() {
    serialized_blocking_runtime().block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original.sqlite").to_string_lossy().into_owned();
        let replacement = directory.path().join("replacement.sqlite").to_string_lossy().into_owned();
        drop(connect(&ConnectParams::sqlite(original.clone(), true)).await.unwrap());
        let target = connect(&ConnectParams::sqlite(replacement.clone(), true)).await.unwrap();
        target
            .execute_script(Some("main"), "CREATE TABLE destination (id INTEGER); INSERT INTO destination VALUES (7)", 10, None)
            .await
            .unwrap();
        drop(target);
        let store = ConnectionStore::open(&directory.path().join("connections.json"), Box::new(MemorySecrets::default())).unwrap();
        let mut config = ConnectionConfig::new("SQLite", Engine::Sqlite);
        config.file_path = Some(original);
        store.save(&config, None).unwrap();
        let sessions = Sessions::new(store);
        let (release, worker) = occupy_blocking_worker();
        let mut opening = Box::pin(sessions.open(&config.id, None));
        assert!(poll!(opening.as_mut()).is_pending());
        let mut changed = config.clone();
        changed.file_path = Some(replacement);
        sessions.store().save(&changed, None).unwrap();
        release.send(()).unwrap();
        worker.await.unwrap();
        assert_eq!(opening.await.unwrap_err().kind, ErrorKind::Connection);
        assert!(sessions.open_ids().await.is_empty());
        sessions.open(&config.id, None).await.unwrap();
        let data =
            sessions.driver(&config.id).await.unwrap().execute_script(Some("main"), "SELECT id FROM destination", 10, None).await.unwrap();
        let schematadb_core::StatementOutcome::Rows { rows, .. } = &data.outcomes[0] else { panic!("expected destination rows") };
        assert_eq!(rows, &[vec![schematadb_core::CellValue::Int(7)]]);
    });
}
