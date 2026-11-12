mod common;

use common::TestDb;
use schematadb_core::{
    AppError, ConnectParams, ConnectionConfig, ConnectionStore, Engine, ErrorKind, MemorySecrets, ObjectKind, Result, SecretStore,
    Sessions, connect,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn store_persists_connections_without_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("connections.json");
    let secrets = MemorySecrets::default();

    let mut config = ConnectionConfig::new("Prod MySQL", Engine::MySql);
    config.host = Some("db.example.com".into());
    config.port = Some(3306);
    config.user = Some("app".into());
    config.color = Some("#e04f55".into());
    config.save_password = true;

    {
        let store = ConnectionStore::open(&file, Box::new(secrets.clone())).unwrap();
        store.save(&config, Some("s3cret-pw")).unwrap();
    }

    let raw = std::fs::read_to_string(&file).unwrap();
    assert!(raw.contains("db.example.com"));
    assert!(!raw.contains("s3cret-pw"), "password leaked into config file");

    let reopened = ConnectionStore::open(&file, Box::new(secrets.clone())).unwrap();
    assert_eq!(reopened.list(), vec![config.clone()]);
    assert_eq!(reopened.password(&config.id).unwrap().as_deref(), Some("s3cret-pw"));

    reopened.delete(&config.id).unwrap();
    assert!(reopened.list().is_empty());
    assert_eq!(reopened.password(&config.id).unwrap(), None);
}

#[tokio::test]
async fn unsaved_password_is_not_stored_anywhere() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConnectionStore::open(&dir.path().join("c.json"), Box::new(MemorySecrets::default())).unwrap();
    let mut config = ConnectionConfig::new("Ask me", Engine::MariaDb);
    config.save_password = false;
    store.save(&config, Some("typed-once")).unwrap();
    assert_eq!(store.password(&config.id).unwrap(), None);
}

#[tokio::test]
async fn store_reports_unavailable_secret_service() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConnectionStore::open(&dir.path().join("c.json"), Box::new(MemorySecrets::unavailable())).unwrap();
    let mut config = ConnectionConfig::new("No keyring", Engine::MySql);
    config.save_password = true;
    let saved = store.save(&config, Some("pw")).unwrap();
    assert!(!saved.password_saved);
    assert!(!store.list()[0].save_password, "config must reflect that the password was not saved");
}

#[derive(Clone, Default)]
struct DeleteFailingSecrets {
    stored: MemorySecrets,
    reads: Arc<AtomicUsize>,
}

impl SecretStore for DeleteFailingSecrets {
    fn get(&self, id: &str) -> Result<Option<String>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.stored.get(id)
    }

    fn set(&self, id: &str, secret: &str) -> Result<()> {
        self.stored.set(id, secret)
    }

    fn delete(&self, _id: &str) -> Result<()> {
        Err(AppError::io("Credential deletion denied"))
    }
}

#[tokio::test]
async fn disabling_password_ignores_retained_credentials_and_reports_failed_deletion() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("connections.json");
    let secrets = DeleteFailingSecrets::default();
    let store = ConnectionStore::open(&path, Box::new(secrets.clone())).unwrap();
    let mut config = ConnectionConfig::new("Previously saved", Engine::MySql);
    config.save_password = true;
    store.save(&config, Some("retained-secret")).unwrap();
    config.save_password = false;
    let saved = store.save(&config, None).unwrap();
    assert!(!saved.config.save_password);
    assert!(!saved.password_saved);
    let warning = saved.password_warning.expect("the retained credential must not be reported as removed");
    assert!(warning.contains("could not be removed"));
    assert_eq!(secrets.stored.get(&config.id).unwrap().as_deref(), Some("retained-secret"));
    assert_eq!(store.password(&config.id).unwrap(), None);
    assert_eq!(secrets.reads.load(Ordering::SeqCst), 0);
    drop(store);
    let reopened = ConnectionStore::open(&path, Box::new(secrets.clone())).unwrap();
    assert!(!reopened.get(&config.id).unwrap().save_password);
    assert_eq!(reopened.password(&config.id).unwrap(), None);
    let sessions = Sessions::new(reopened);
    let error = sessions.open(&config.id, None).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Authentication, "disabled passwords must prompt rather than connecting with the old secret");
    assert!(sessions.open_ids().await.is_empty());
    assert_eq!(secrets.reads.load(Ordering::SeqCst), 0, "disabled passwords must never even read the credential store");
}

#[tokio::test]
async fn deleting_a_connection_reports_a_credential_that_could_not_be_removed() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("connections.json");
    let secrets = DeleteFailingSecrets::default();
    let store = ConnectionStore::open(&path, Box::new(secrets.clone())).unwrap();
    let mut config = ConnectionConfig::new("Saved password", Engine::MySql);
    config.save_password = true;
    store.save(&config, Some("retained-secret")).unwrap();
    let without_password = ConnectionConfig::new("No password", Engine::MySql);
    store.save(&without_password, None).unwrap();

    let error = store.delete(&config.id).unwrap_err();
    assert!(error.message.contains("could not be removed"));
    store.delete(&without_password.id).expect("a connection without a saved password has nothing to report");
    assert!(store.list().is_empty());
    let reopened = ConnectionStore::open(&path, Box::new(secrets.clone())).unwrap();
    assert!(reopened.list().is_empty(), "the connection itself must still be deleted");
}

#[test]
fn connection_snapshots_replace_atomically_without_overwriting_an_unrelated_temporary_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("connections.json");
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, b"unrelated").unwrap();
    let store = ConnectionStore::open(&path, Box::new(MemorySecrets::default())).unwrap();
    let mut config = ConnectionConfig::new("Original", Engine::Sqlite);
    store.save(&config, None).unwrap();
    config.name = "Updated".into();
    store.save(&config, None).unwrap();
    assert_eq!(std::fs::read(&temporary).unwrap(), b"unrelated");
    let reopened = ConnectionStore::open(&path, Box::new(MemorySecrets::default())).unwrap();
    assert_eq!(reopened.get(&config.id).unwrap().name, "Updated");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[tokio::test]
async fn sqlite_connect_creates_new_file_only_when_asked() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.db").to_string_lossy().into_owned();
    let err = match connect(&ConnectParams::sqlite(missing.clone(), false)).await {
        Ok(_) => panic!("opening a missing file without create must fail"),
        Err(e) => e,
    };
    assert_eq!(err.kind, ErrorKind::Connection);
    assert!(!std::path::Path::new(&missing).exists());

    connect(&ConnectParams::sqlite(missing.clone(), true)).await.unwrap();
    assert!(std::path::Path::new(&missing).exists());
}

#[tokio::test]
async fn sqlite_lists_main_database_and_objects_with_exact_counts() {
    let t = TestDb::sqlite().await;
    t.exec(
        "CREATE TABLE product (id INTEGER PRIMARY KEY, name TEXT);
         INSERT INTO product (name) VALUES ('a'), ('b'), ('c');
         CREATE TABLE culture (id TEXT);
         CREATE VIEW cheap AS SELECT * FROM product;",
    )
    .await;

    let dbs = t.driver.list_databases().await.unwrap();
    assert_eq!(dbs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), vec!["main"]);

    let objects = t.driver.list_objects(&t.db).await.unwrap();
    let summary: Vec<_> = objects.iter().map(|o| (o.name.as_str(), o.kind, o.rows)).collect();
    assert_eq!(
        summary,
        vec![("culture", ObjectKind::Table, Some(0)), ("product", ObjectKind::Table, Some(3)), ("cheap", ObjectKind::View, None),]
    );
}
