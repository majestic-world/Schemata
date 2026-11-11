mod common;

use common::TestDb;
use projectcat_core::{ConnectParams, ConnectionConfig, ConnectionStore, Engine, ErrorKind, MemorySecrets, ObjectKind, connect};

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
