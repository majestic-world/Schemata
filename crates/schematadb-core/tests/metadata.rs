mod common;

use common::TestDb;
use schematadb_core::{CreateDatabase, Engine, ErrorKind, IndexKind, ObjectKind, RowIdentity};

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
    assert_eq!(summary, vec![("culture", ObjectKind::Table), ("product", ObjectKind::Table), ("cheap", ObjectKind::View)]);
    assert!(objects[0].rows.is_some(), "tables report a row count");
    assert_eq!(objects[2].rows, None, "views have no row count");
    if t.engine == Engine::Sqlite {
        assert_eq!(objects[1].rows, Some(3));
    }
}
scenario!(lists_tables_then_views);

async fn refreshes_row_counts_after_inserts(t: TestDb) {
    // MyISAM reports exact engine statistics, so the assertion does not depend on InnoDB sampling.
    let engine = if t.engine == Engine::Sqlite { "" } else { " ENGINE=MyISAM" };
    t.exec(&format!("CREATE TABLE counter (id INTEGER PRIMARY KEY){engine};")).await;
    let before = t.driver.list_objects(&t.db).await.unwrap();
    assert_eq!(before.iter().find(|object| object.name == "counter").unwrap().rows, Some(0));

    t.exec("INSERT INTO counter (id) VALUES (1), (2), (3);").await;

    let after = t.driver.list_objects(&t.db).await.unwrap();
    assert_eq!(after.iter().find(|object| object.name == "counter").unwrap().rows, Some(3));
}
scenario!(refreshes_row_counts_after_inserts);

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

async fn describes_table_structure(t: TestDb) {
    let auto = if t.engine == Engine::Sqlite { "INTEGER PRIMARY KEY AUTOINCREMENT" } else { "INT AUTO_INCREMENT PRIMARY KEY" };
    t.exec(&format!(
        "CREATE TABLE category (id INT PRIMARY KEY, name VARCHAR(30) NOT NULL);
         CREATE TABLE product (
            id {auto},
            sku VARCHAR(20) NOT NULL,
            name VARCHAR(100) DEFAULT 'unnamed',
            category_id INT,
            CONSTRAINT uq_sku UNIQUE (sku),
            CONSTRAINT fk_cat FOREIGN KEY (category_id) REFERENCES category (id) ON DELETE CASCADE
         );
         CREATE INDEX ix_name ON product (name);
         CREATE VIEW v_product AS SELECT id, name FROM product;"
    ))
    .await;

    let info = t.driver.table_info(&t.db, "product").await.unwrap();
    assert_eq!(info.kind, ObjectKind::Table);
    let cols: Vec<_> = info.columns.iter().map(|c| (c.name.as_str(), c.nullable, c.primary_key, c.auto_increment)).collect();
    assert_eq!(
        cols,
        vec![("id", false, true, true), ("sku", false, false, false), ("name", true, false, false), ("category_id", true, false, false),]
    );
    assert!(info.columns[1].data_type.to_lowercase().starts_with("varchar"), "{}", info.columns[1].data_type);
    assert_eq!(info.columns[2].default.as_deref(), Some("'unnamed'"));
    assert_eq!(info.row_identity, RowIdentity::PrimaryKey { columns: vec!["id".into()] });

    let unique = info.indexes.iter().find(|i| i.columns == vec!["sku".to_string()]).expect("unique index on sku");
    assert_eq!(unique.kind, IndexKind::Unique);
    let plain = info.indexes.iter().find(|i| i.name == "ix_name").expect("ix_name");
    assert_eq!(plain.kind, IndexKind::Normal);
    assert_eq!(plain.columns, vec!["name".to_string()]);
    assert!(!info.indexes.iter().any(|i| i.primary), "the primary key is reported on columns, not as an index");

    assert_eq!(info.foreign_keys.len(), 1);
    let fk = &info.foreign_keys[0];
    assert_eq!(
        (fk.columns.clone(), fk.ref_table.as_str(), fk.ref_columns.clone()),
        (vec!["category_id".to_string()], "category", vec!["id".to_string()])
    );
    assert_eq!(fk.on_delete, "CASCADE");

    // The CREATE statement recreates the same structure.
    let create = info.create_sql.clone();
    t.exec("DROP VIEW v_product; DROP TABLE product;").await;
    t.exec(&create).await;
    let again = t.driver.table_info(&t.db, "product").await.unwrap();
    assert_eq!(again.columns, info.columns);

    t.exec("CREATE VIEW v_product AS SELECT id, name FROM product;").await;
    let view = t.driver.table_info(&t.db, "v_product").await.unwrap();
    assert_eq!(view.kind, ObjectKind::View);
    assert_eq!(view.row_identity, RowIdentity::None);
    assert!(view.create_sql.to_lowercase().contains("select"), "{}", view.create_sql);
}
scenario!(describes_table_structure);

async fn row_identity_falls_back_to_unique_key(t: TestDb) {
    t.exec(
        "CREATE TABLE coded (code VARCHAR(10) NOT NULL, label VARCHAR(10), CONSTRAINT uq_code UNIQUE (code));
         CREATE TABLE loose (a INT, b INT);",
    )
    .await;
    let coded = t.driver.table_info(&t.db, "coded").await.unwrap();
    assert_eq!(coded.row_identity, RowIdentity::UniqueKey { columns: vec!["code".into()] });
    let loose = t.driver.table_info(&t.db, "loose").await.unwrap();
    let expected = if t.engine == Engine::Sqlite { RowIdentity::RowId } else { RowIdentity::AllColumns { columns: vec!["a".into(), "b".into()] } };
    assert_eq!(loose.row_identity, expected);
}
scenario!(row_identity_falls_back_to_unique_key);

async fn creates_and_drops_databases(t: TestDb) {
    if t.engine == Engine::Sqlite {
        let err = t.driver.create_database(&CreateDatabase { name: "x".into(), charset: None, collation: None }).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("other.db").to_string_lossy().into_owned();
        t.driver.attach_database(&file, "other").await.unwrap();
        assert!(t.driver.list_databases().await.unwrap().iter().any(|d| d.name == "other"));
        t.driver.drop_database("other").await.unwrap();
        assert!(!t.driver.list_databases().await.unwrap().iter().any(|d| d.name == "other"));
        return;
    }
    let name = format!("{}_new", t.db);
    t.driver
        .create_database(&CreateDatabase {
            name: name.clone(),
            charset: Some("latin1".into()),
            collation: Some("latin1_swedish_ci".into()),
        })
        .await
        .unwrap();
    let charsets = t.driver.list_charsets().await.unwrap();
    assert!(charsets.iter().any(|c| c.name == "utf8mb4" && c.collations.iter().any(|x| x == &c.default_collation)));
    let r = t
        .driver
        .execute_script(
            None,
            &format!("SELECT DEFAULT_COLLATION_NAME FROM information_schema.SCHEMATA WHERE SCHEMA_NAME = '{name}'"),
            1,
            None,
        )
        .await
        .unwrap();
    let schematadb_core::StatementOutcome::Rows { rows, .. } = &r.outcomes[0] else { panic!() };
    assert_eq!(rows[0][0], schematadb_core::CellValue::Text("latin1_swedish_ci".into()));
    t.driver.drop_database(&name).await.unwrap();
    assert!(!t.driver.list_databases().await.unwrap().iter().any(|d| d.name == name));
}
scenario!(creates_and_drops_databases);
