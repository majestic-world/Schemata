mod common;

use common::TestDb;
use schematadb_core::{ConnectionConfig, ConnectionStore, Engine, MemorySecrets, Sessions, connect};

/// A session whose server connections are all killed keeps working on the next action.
async fn reconnects_after_connection_loss(t: TestDb) {
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
        .with_driver(&config.id, |d| {
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
