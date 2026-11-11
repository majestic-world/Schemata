mod common;

use common::TestDb;
use projectcat_core::{Engine, ObjectKind};

async fn lists_tables_then_views(t: TestDb) {
    t.exec(
        "CREATE TABLE product (id INTEGER PRIMARY KEY, name VARCHAR(50));
         INSERT INTO product (id, name) VALUES (1, 'a'), (2, 'b'), (3, 'c');
         CREATE TABLE culture (code VARCHAR(10));
         CREATE VIEW cheap AS SELECT * FROM product;",
    )
    .await;

    let dbs = t.driver.list_databases().await.unwrap();
    assert!(dbs.iter().any(|d| d.name == t.db), "{:?} missing {}", dbs, t.db);

    let objects = t.driver.list_objects(&t.db).await.unwrap();
    let summary: Vec<_> = objects.iter().map(|o| (o.name.as_str(), o.kind)).collect();
    assert_eq!(
        summary,
        vec![("culture", ObjectKind::Table), ("product", ObjectKind::Table), ("cheap", ObjectKind::View)]
    );
    assert!(objects[0].rows.is_some(), "tables report a row count");
    assert_eq!(objects[2].rows, None, "views have no row count");
    if t.engine == Engine::Sqlite {
        assert_eq!(objects[1].rows, Some(3));
    }
}
scenario!(lists_tables_then_views);

async fn detects_server_flavor(t: TestDb) {
    assert_eq!(t.driver.engine(), t.engine);
    let version = t.driver.server_version();
    let expected = match t.engine {
        Engine::MySql => "MySQL ",
        Engine::MariaDb => "MariaDB ",
        Engine::Sqlite => "SQLite ",
    };
    assert!(version.starts_with(expected), "{version}");
}
scenario!(detects_server_flavor);
