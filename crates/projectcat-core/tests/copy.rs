mod common;

use common::TestDb;
use projectcat_core::{
    CancellationToken, CellValue, ConnectParams, CopyTableTarget, Driver, Engine, ErrorKind, PageRequest, ProgressFn, SortSpec, connect,
    copy_table,
};
use std::sync::Arc;

fn no_progress() -> ProgressFn {
    Arc::new(|_| {})
}

async fn rows(d: &dyn Driver, db: &str, table: &str) -> Vec<Vec<CellValue>> {
    let req = PageRequest { offset: 0, limit: 1000, sort: Some(SortSpec { column: "id".into(), desc: false }), filters: vec![] };
    let page = d.fetch_page(db, table, &req).await.unwrap();
    page.rows.into_iter().map(|r| r.into_iter().zip(&page.columns).filter(|(_, c)| !c.hidden).map(|(v, _)| v).collect()).collect()
}

async fn seed(t: &TestDb) {
    let auto = if t.engine == Engine::Sqlite { "INTEGER PRIMARY KEY AUTOINCREMENT" } else { "INT AUTO_INCREMENT PRIMARY KEY" };
    t.exec(&format!(
        "CREATE TABLE customer (id {auto}, name VARCHAR(40) NOT NULL, notes TEXT);
         CREATE TABLE orders (id {auto}, customer_id INT NOT NULL, total DECIMAL(10,2) DEFAULT 0, photo BLOB,
             CONSTRAINT fk_orders_customer FOREIGN KEY (customer_id) REFERENCES customer (id));
         CREATE INDEX ix_orders_total ON orders (total);
         INSERT INTO customer (name, notes) VALUES ('Ana', 'it''s'), ('Bob', NULL);
         INSERT INTO orders (customer_id, total, photo) VALUES (1, 10.50, X'00FF'), (1, 99.99, NULL), (2, 0.01, X'');"
    ))
    .await;
}

fn to(db: &str, name: &str, data: bool) -> CopyTableTarget {
    CopyTableTarget { db: db.into(), name: name.into(), data }
}

/// Another database on the same server, or another SQLite file (a separate connection either way).
async fn other_database(t: &TestDb, dir: &tempfile::TempDir) -> (Arc<dyn Driver>, String) {
    match t.server {
        None => {
            let file = dir.path().join("target.db").to_string_lossy().into_owned();
            (connect(&ConnectParams::sqlite(file, true)).await.unwrap(), "main".into())
        }
        Some(server) => {
            let name = format!("{}_copy", t.db);
            t.driver.execute_script(None, &format!("CREATE DATABASE `{name}`"), 1, None).await.unwrap();
            (connect(&server.params(Some(&name))).await.unwrap(), name)
        }
    }
}

async fn drop_other(t: &TestDb, db: &str) {
    if t.server.is_some() {
        t.driver.execute_script(None, &format!("DROP DATABASE `{db}`"), 1, None).await.unwrap();
    }
}

async fn copies_structure_and_rows_into_another_database(t: TestDb) {
    seed(&t).await;
    let dir = tempfile::tempdir().unwrap();
    let (target, db) = other_database(&t, &dir).await;
    let src = t.driver.as_ref();

    // The referenced table is missing from the target, so the foreign key is left out.
    let report =
        copy_table(src, &t.db, "orders", target.as_ref(), &to(&db, "orders", true), no_progress(), CancellationToken::new()).await.unwrap();
    assert_eq!(report.rows, 3);
    let (a, b) = (src.table_info(&t.db, "orders").await.unwrap(), target.table_info(&db, "orders").await.unwrap());
    assert_eq!(a.columns, b.columns);
    assert_eq!(a.indexes, b.indexes);
    assert!(b.foreign_keys.is_empty());
    assert_eq!(rows(src, &t.db, "orders").await, rows(target.as_ref(), &db, "orders").await);

    // Once it exists there, a copy keeps the foreign key.
    copy_table(src, &t.db, "customer", target.as_ref(), &to(&db, "customer", true), no_progress(), CancellationToken::new()).await.unwrap();
    copy_table(src, &t.db, "orders", target.as_ref(), &to(&db, "orders2", false), no_progress(), CancellationToken::new()).await.unwrap();
    assert_eq!(target.table_info(&db, "orders2").await.unwrap().foreign_keys.len(), 1);
    assert!(rows(target.as_ref(), &db, "orders2").await.is_empty());

    // Auto-increment continues after the copied rows.
    target.execute_script(Some(&db), "INSERT INTO customer (name) VALUES ('Cid')", 1, None).await.unwrap();
    assert_eq!(rows(target.as_ref(), &db, "customer").await[2][0], CellValue::Int(3));
    drop_other(&t, &db).await;
}
scenario!(copies_structure_and_rows_into_another_database);

async fn copies_within_the_same_database_under_a_new_name(t: TestDb) {
    seed(&t).await;
    let d = t.driver.as_ref();
    copy_table(d, &t.db, "orders", d, &to(&t.db, "orders_copy", true), no_progress(), CancellationToken::new()).await.unwrap();
    assert_eq!(rows(d, &t.db, "orders").await, rows(d, &t.db, "orders_copy").await);
    let copy = d.table_info(&t.db, "orders_copy").await.unwrap();
    assert_eq!(copy.foreign_keys.len(), 1);
    assert_eq!(copy.foreign_keys[0].ref_table, "customer");
    // The original keeps its index and foreign key.
    let original = d.table_info(&t.db, "orders").await.unwrap();
    assert_eq!((original.indexes.len(), original.foreign_keys.len()), (copy.indexes.len(), 1));

    let err =
        copy_table(d, &t.db, "orders", d, &to(&t.db, "ORDERS_copy", true), no_progress(), CancellationToken::new()).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
}
scenario!(copies_within_the_same_database_under_a_new_name);

async fn cancelled_copy_leaves_no_table(t: TestDb) {
    seed(&t).await;
    let d = t.driver.as_ref();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let err = copy_table(d, &t.db, "orders", d, &to(&t.db, "orders_copy", true), no_progress(), cancel).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::Cancelled);
    assert!(!d.list_objects(&t.db).await.unwrap().iter().any(|o| o.name == "orders_copy"));
}
scenario!(cancelled_copy_leaves_no_table);

/// MySQL/MariaDB tables copied into a SQLite file keep their columns and rows.
#[tokio::test]
async fn copies_from_mysql_into_sqlite() {
    let t = TestDb::mysql().await;
    seed(&t).await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("target.db").to_string_lossy().into_owned();
    let target = connect(&ConnectParams::sqlite(file, true)).await.unwrap();
    for table in ["customer", "orders"] {
        copy_table(t.driver.as_ref(), &t.db, table, target.as_ref(), &to("main", table, true), no_progress(), CancellationToken::new())
            .await
            .unwrap();
    }
    assert_eq!(rows(target.as_ref(), "main", "customer").await, rows(t.driver.as_ref(), &t.db, "customer").await);
    assert_eq!(rows(target.as_ref(), "main", "orders").await.len(), 3);
    assert_eq!(target.table_info("main", "orders").await.unwrap().foreign_keys.len(), 1);
}
