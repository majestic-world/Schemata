mod common;

use common::TestDb;
use schematadb_core::{
    CancellationToken, CellValue, ConnectParams, Driver, DumpOptions, Engine, PageRequest, ProgressFn, SortSpec, connect, dump_database,
    run_sql_file,
};
use std::sync::Arc;

fn no_progress() -> ProgressFn {
    Arc::new(|_| {})
}

async fn rows(d: &dyn Driver, db: &str, table: &str, order: &str) -> Vec<Vec<CellValue>> {
    let req = PageRequest { offset: 0, limit: 1000, sort: Some(SortSpec { column: order.into(), desc: false }), filters: vec![] };
    let page = d.fetch_page(db, table, &req).await.unwrap();
    page.rows.into_iter().map(|r| r.into_iter().zip(&page.columns).filter(|(_, c)| !c.hidden).map(|(v, _)| v).collect()).collect()
}

async fn seed(t: &TestDb) {
    let sqlite = t.engine == Engine::Sqlite;
    let auto = if sqlite { "INTEGER PRIMARY KEY AUTOINCREMENT" } else { "INT AUTO_INCREMENT PRIMARY KEY" };
    t.exec(&format!(
        "CREATE TABLE customer (id {auto}, name VARCHAR(40) NOT NULL, notes TEXT);
         CREATE TABLE orders (id {auto}, customer_id INT NOT NULL, total DECIMAL(10,2), photo BLOB,
             CONSTRAINT fk_orders_customer FOREIGN KEY (customer_id) REFERENCES customer (id));
         CREATE INDEX ix_orders_total ON orders (total);
         CREATE TABLE audit (msg VARCHAR(80));
         INSERT INTO customer (name, notes) VALUES ('Ana', 'it''s; quoted'), ('Bob', NULL), ('Çá', 'line\nbreak');
         INSERT INTO orders (customer_id, total, photo) VALUES (1, 10.50, X'00FF'), (1, 99.99, NULL), (3, 0.01, X'');
         CREATE VIEW big_orders AS SELECT id, total FROM orders WHERE total > 50;
         CREATE VIEW big_order_ids AS SELECT id FROM big_orders;"
    ))
    .await;
    if sqlite {
        t.exec("CREATE TRIGGER orders_audit AFTER INSERT ON orders BEGIN INSERT INTO audit VALUES ('order ' || NEW.id); END;").await;
    } else {
        t.exec("DELIMITER ;;\nCREATE TRIGGER orders_audit AFTER INSERT ON orders FOR EACH ROW BEGIN INSERT INTO audit VALUES (CONCAT('order ', NEW.id)); END;;\nDELIMITER ;").await;
    }
}

/// A fresh, empty target: another database on the same server, or a new SQLite file.
async fn empty_target(t: &TestDb, dir: &tempfile::TempDir) -> (Arc<dyn Driver>, String) {
    match t.server {
        None => {
            let file = dir.path().join("restored.db").to_string_lossy().into_owned();
            (connect(&ConnectParams::sqlite(file, true)).await.unwrap(), "main".into())
        }
        Some(server) => {
            let name = format!("{}_restore", t.db);
            t.driver.execute_script(None, &format!("CREATE DATABASE `{name}`"), 1, None).await.unwrap();
            (connect(&server.params(Some(&name))).await.unwrap(), name)
        }
    }
}

async fn dump_restores_schema_and_data(t: TestDb) {
    seed(&t).await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("dump.sql");
    let report = dump_database(
        t.driver.as_ref(),
        &t.db,
        &file,
        &DumpOptions { structure: true, data: true },
        no_progress(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(report.rows, 6);

    let (target, db) = empty_target(&t, &dir).await;
    let run = run_sql_file(target.as_ref(), &db, &file, true, no_progress(), CancellationToken::new()).await.unwrap();
    assert!(run.failed.is_empty(), "{:?}", run.failed);

    let names = |o: Vec<schematadb_core::ObjectInfo>| o.into_iter().map(|o| (o.name, o.kind)).collect::<Vec<_>>();
    assert_eq!(names(target.list_objects(&db).await.unwrap()), names(t.driver.list_objects(&t.db).await.unwrap()));
    for table in ["customer", "orders"] {
        let (a, b) = (t.driver.table_info(&t.db, table).await.unwrap(), target.table_info(&db, table).await.unwrap());
        assert_eq!(a.columns, b.columns, "{table} columns");
        assert_eq!(a.indexes, b.indexes, "{table} indexes");
        assert_eq!(a.foreign_keys, b.foreign_keys, "{table} foreign keys");
        assert_eq!(rows(t.driver.as_ref(), &t.db, table, "id").await, rows(target.as_ref(), &db, table, "id").await, "{table} rows");
    }
    assert_eq!(rows(target.as_ref(), &db, "big_order_ids", "id").await, vec![vec![CellValue::Int(2)]]);
    // The trigger came back, and auto-increment continues after the restored rows.
    target.execute_script(Some(&db), "INSERT INTO orders (customer_id, total) VALUES (2, 1.00)", 1, None).await.unwrap();
    assert_eq!(rows(target.as_ref(), &db, "audit", "msg").await, vec![vec![CellValue::Text("order 4".into())]]);

    if t.server.is_some() {
        t.driver.execute_script(None, &format!("DROP DATABASE `{db}`"), 1, None).await.unwrap();
    }
}
scenario!(dump_restores_schema_and_data);

async fn structure_only_dump_has_no_rows(t: TestDb) {
    seed(&t).await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("schema.sql");
    dump_database(t.driver.as_ref(), &t.db, &file, &DumpOptions { structure: true, data: false }, no_progress(), CancellationToken::new())
        .await
        .unwrap();
    let (target, db) = empty_target(&t, &dir).await;
    let run = run_sql_file(target.as_ref(), &db, &file, true, no_progress(), CancellationToken::new()).await.unwrap();
    assert!(run.failed.is_empty(), "{:?}", run.failed);
    assert!(rows(target.as_ref(), &db, "customer", "id").await.is_empty());
    assert_eq!(target.table_info(&db, "orders").await.unwrap().foreign_keys.len(), 1);
    if t.server.is_some() {
        t.driver.execute_script(None, &format!("DROP DATABASE `{db}`"), 1, None).await.unwrap();
    }
}
scenario!(structure_only_dump_has_no_rows);

#[tokio::test]
async fn sqlite_backup_of_attached_database_reads_its_own_rows() {
    let t = TestDb::sqlite().await;
    let dir = tempfile::tempdir().unwrap();
    let other = dir.path().join("other.db").to_string_lossy().into_owned();
    t.driver.attach_database(&other, "other").await.unwrap();
    t.exec("CREATE TABLE main.item (id INTEGER PRIMARY KEY, v TEXT); INSERT INTO main.item VALUES (1, 'main');").await;
    t.exec("CREATE TABLE other.item (id INTEGER PRIMARY KEY, v TEXT); INSERT INTO other.item VALUES (1, 'other'), (2, 'other');").await;

    let file = dir.path().join("other.sql");
    let report = dump_database(
        t.driver.as_ref(),
        "other",
        &file,
        &DumpOptions { structure: true, data: true },
        no_progress(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(report.rows, 2);
    let (target, db) = empty_target(&t, &dir).await;
    run_sql_file(target.as_ref(), &db, &file, true, no_progress(), CancellationToken::new()).await.unwrap();
    assert_eq!(
        rows(target.as_ref(), &db, "item", "id").await,
        vec![vec![CellValue::Int(1), CellValue::Text("other".into())], vec![CellValue::Int(2), CellValue::Text("other".into())],]
    );
}
