mod common;

use common::TestDb;
use schematadb_core::{
    CancellationToken, CellValue, ConnectParams, Driver, DumpOptions, Engine, ObjectKind, PageRequest, ProgressFn, SortSpec, TextEncoding,
    connect, dump_database, dump_tables, run_sql_file,
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
    let run = run_sql_file(target.as_ref(), &db, &file, TextEncoding::Utf8, true, no_progress(), CancellationToken::new()).await.unwrap();
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
    let run = run_sql_file(target.as_ref(), &db, &file, TextEncoding::Utf8, true, no_progress(), CancellationToken::new()).await.unwrap();
    assert!(run.failed.is_empty(), "{:?}", run.failed);
    assert!(rows(target.as_ref(), &db, "customer", "id").await.is_empty());
    assert_eq!(target.table_info(&db, "orders").await.unwrap().foreign_keys.len(), 1);
    if t.server.is_some() {
        t.driver.execute_script(None, &format!("DROP DATABASE `{db}`"), 1, None).await.unwrap();
    }
}
scenario!(structure_only_dump_has_no_rows);

/// One file per table: each restores on its own, in any order, with its rows, indexes, foreign keys, trigger and
/// autoincrement counter; views stay out.
async fn per_table_dump_restores_each_table(t: TestDb) {
    seed(&t).await;
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("tables");
    std::fs::create_dir(&out).unwrap();
    let report =
        dump_tables(t.driver.as_ref(), &t.db, &out, &DumpOptions { structure: true, data: true }, no_progress(), CancellationToken::new())
            .await
            .unwrap();
    assert_eq!((report.tables, report.rows), (3, 6));
    let mut files: Vec<String> = std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    files.sort();
    assert_eq!(files, ["audit.sql", "customer.sql", "orders.sql"]);

    let (target, db) = empty_target(&t, &dir).await;
    for file in ["orders.sql", "customer.sql", "audit.sql"] {
        let run = run_sql_file(target.as_ref(), &db, &out.join(file), TextEncoding::Utf8, true, no_progress(), CancellationToken::new())
            .await
            .unwrap();
        assert!(run.failed.is_empty(), "{file}: {:?}", run.failed);
    }
    let tables = |o: Vec<schematadb_core::ObjectInfo>| o.into_iter().map(|o| (o.name, o.kind)).collect::<Vec<_>>();
    let source_tables = tables(t.driver.list_objects(&t.db).await.unwrap()).into_iter().filter(|(_, kind)| *kind == ObjectKind::Table).collect::<Vec<_>>();
    assert_eq!(tables(target.list_objects(&db).await.unwrap()), source_tables, "tables only, no views");
    for table in ["customer", "orders"] {
        let (a, b) = (t.driver.table_info(&t.db, table).await.unwrap(), target.table_info(&db, table).await.unwrap());
        assert_eq!(a.columns, b.columns, "{table} columns");
        assert_eq!(a.indexes, b.indexes, "{table} indexes");
        assert_eq!(a.foreign_keys, b.foreign_keys, "{table} foreign keys");
        assert_eq!(rows(t.driver.as_ref(), &t.db, table, "id").await, rows(target.as_ref(), &db, table, "id").await, "{table} rows");
    }
    target.execute_script(Some(&db), "INSERT INTO orders (customer_id, total) VALUES (2, 1.00)", 1, None).await.unwrap();
    assert_eq!(rows(target.as_ref(), &db, "audit", "msg").await, vec![vec![CellValue::Text("order 4".into())]]);

    if t.server.is_some() {
        t.driver.execute_script(None, &format!("DROP DATABASE `{db}`"), 1, None).await.unwrap();
    }
}
scenario!(per_table_dump_restores_each_table);

/// A cancelled per-table dump replaces no file and leaves no temporary file behind.
#[tokio::test]
async fn cancelled_per_table_dump_leaves_the_folder_as_it_was() {
    let t = TestDb::sqlite().await;
    seed(&t).await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("customer.sql"), "old").unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let err = dump_tables(t.driver.as_ref(), &t.db, dir.path(), &DumpOptions { structure: true, data: true }, no_progress(), cancel)
        .await
        .unwrap_err();
    assert_eq!(err.kind, schematadb_core::ErrorKind::Cancelled);
    let files: Vec<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(files, ["customer.sql"]);
    assert_eq!(std::fs::read_to_string(dir.path().join("customer.sql")).unwrap(), "old");
}

/// SQLite: an index or trigger that names its table in another case still goes into that table's file.
#[tokio::test]
async fn sqlite_per_table_dump_keeps_objects_naming_the_table_in_another_case() {
    let t = TestDb::sqlite().await;
    t.exec(
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, total INT);
         CREATE TABLE audit (msg TEXT);
         CREATE INDEX ix_orders_total ON ORDERS (total);
         CREATE TRIGGER orders_audit AFTER INSERT ON Orders BEGIN INSERT INTO audit VALUES ('order'); END;",
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    dump_tables(t.driver.as_ref(), &t.db, dir.path(), &DumpOptions { structure: true, data: false }, no_progress(), CancellationToken::new())
        .await
        .unwrap();
    let orders = std::fs::read_to_string(dir.path().join("orders.sql")).unwrap();
    assert!(orders.contains("CREATE INDEX ix_orders_total") && orders.contains("CREATE TRIGGER orders_audit"), "{orders}");
    let audit = std::fs::read_to_string(dir.path().join("audit.sql")).unwrap();
    assert!(!audit.contains("CREATE INDEX") && !audit.contains("CREATE TRIGGER"), "{audit}");
}

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
    run_sql_file(target.as_ref(), &db, &file, TextEncoding::Utf8, true, no_progress(), CancellationToken::new()).await.unwrap();
    assert_eq!(
        rows(target.as_ref(), &db, "item", "id").await,
        vec![vec![CellValue::Int(1), CellValue::Text("other".into())], vec![CellValue::Int(2), CellValue::Text("other".into())],]
    );
}

async fn backup_restores_generated_columns_over_existing_views(t: TestDb) {
    t.exec(
        "CREATE TABLE generated_backup (
             id INT PRIMARY KEY, base INT,
             doubled INT GENERATED ALWAYS AS (base * 2) STORED,
             tripled INT GENERATED ALWAYS AS (base * 3) VIRTUAL);
         INSERT INTO generated_backup (id, base) VALUES (1, 7), (2, NULL);
         CREATE VIEW generated_view AS SELECT id, doubled, tripled FROM generated_backup;
         CREATE VIEW dependent_view AS SELECT * FROM generated_view;",
    )
    .await;
    let expected =
        vec![vec![CellValue::Int(1), CellValue::Int(14), CellValue::Int(21)], vec![CellValue::Int(2), CellValue::Null, CellValue::Null]];
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("generated.sql");
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
    assert_eq!(report.rows, 2);
    t.exec("UPDATE generated_backup SET base = 99 WHERE id = 1").await;
    let restored =
        run_sql_file(t.driver.as_ref(), &t.db, &file, TextEncoding::Utf8, true, no_progress(), CancellationToken::new()).await.unwrap();
    assert!(restored.failed.is_empty(), "{:?}", restored.failed);
    assert_eq!(rows(t.driver.as_ref(), &t.db, "dependent_view", "id").await, expected);
    t.exec("UPDATE generated_backup SET base = 5 WHERE id = 1").await;
    assert_eq!(
        rows(t.driver.as_ref(), &t.db, "generated_view", "id").await[0],
        vec![CellValue::Int(1), CellValue::Int(10), CellValue::Int(15)]
    );
}
scenario!(backup_restores_generated_columns_over_existing_views);

async fn dump_caps_each_insert_statement_by_size(t: TestDb) {
    let text = if t.engine == Engine::Sqlite { "TEXT" } else { "LONGTEXT" };
    t.exec(&format!("CREATE TABLE wide (id INT PRIMARY KEY, body {text})")).await;
    // 30 rows of 100 KB: one 100-row INSERT would be about 3 MB.
    for id in 1..=30 {
        t.exec(&format!("INSERT INTO wide VALUES ({id}, '{}')", "x".repeat(100_000))).await;
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("wide.sql");
    dump_database(t.driver.as_ref(), &t.db, &file, &DumpOptions { structure: true, data: true }, no_progress(), CancellationToken::new())
        .await
        .unwrap();
    let dump = std::fs::read_to_string(&file).unwrap();
    let inserts: Vec<&str> = dump.split("INSERT INTO").skip(1).map(|s| s.split(";\n").next().unwrap()).collect();
    assert!(inserts.len() >= 3, "{} INSERT statements", inserts.len());
    assert!(
        inserts.iter().all(|s| s.len() <= 1 << 20),
        "an INSERT exceeds 1 MB: {:?}",
        inserts.iter().map(|s| s.len()).collect::<Vec<_>>()
    );

    let (target, db) = empty_target(&t, &dir).await;
    let run = run_sql_file(target.as_ref(), &db, &file, TextEncoding::Utf8, true, no_progress(), CancellationToken::new()).await.unwrap();
    assert!(run.failed.is_empty(), "{:?}", run.failed);
    assert_eq!(target.count_rows(&db, "wide", &[]).await.unwrap(), 30);
    if t.server.is_some() {
        t.driver.execute_script(None, &format!("DROP DATABASE `{db}`"), 1, None).await.unwrap();
    }
}
scenario!(dump_caps_each_insert_statement_by_size);

/// Dumps read every table in one snapshot on one session, which is closed afterwards: no transaction stays open
/// that would hold metadata locks against later DDL.
async fn dump_leaves_no_open_session_behind(t: TestDb) {
    seed(&t).await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("dump.sql");
    dump_database(t.driver.as_ref(), &t.db, &file, &DumpOptions { structure: true, data: true }, no_progress(), CancellationToken::new())
        .await
        .unwrap();
    let ddl = tokio::time::timeout(std::time::Duration::from_secs(20), t.exec("ALTER TABLE customer ADD COLUMN extra INT"));
    ddl.await.expect("DDL must not wait for a transaction left open by the dump");
}
scenario!(dump_leaves_no_open_session_behind);

/// MySQL/MariaDB: rows committed while the dump runs never show up in tables it reads later (one snapshot).
async fn server_dump_reads_one_consistent_snapshot(t: TestDb) {
    t.exec("CREATE TABLE a_first (id INT PRIMARY KEY); INSERT INTO a_first VALUES (1); CREATE TABLE z_last (id INT PRIMARY KEY); INSERT INTO z_last VALUES (1);")
        .await;
    let (server, db) = (t.server.unwrap(), t.db.clone());
    let wrote = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // After the first table's rows, another connection commits a row into the table dumped last.
    let progress: ProgressFn = Arc::new(move |p| {
        if p.done >= 1 && !wrote.swap(true, std::sync::atomic::Ordering::SeqCst) {
            let db = db.clone();
            std::thread::spawn(move || {
                tokio::runtime::Runtime::new().unwrap().block_on(async move {
                    let writer = connect(&server.params(Some(&db))).await.unwrap();
                    let result = writer.execute_script(Some(&db), "INSERT INTO z_last VALUES (2)", 1, None).await.unwrap();
                    assert!(result.error.is_none(), "{:?}", result.error);
                })
            })
            .join()
            .unwrap();
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("snapshot.sql");
    let report =
        dump_database(t.driver.as_ref(), &t.db, &file, &DumpOptions { structure: false, data: true }, progress, CancellationToken::new())
            .await
            .unwrap();
    assert_eq!(report.rows, 2, "z_last is read as it was when the dump started");
    assert_eq!(t.driver.count_rows(&t.db, "z_last", &[]).await.unwrap(), 2);
    let dump = std::fs::read_to_string(&file).unwrap();
    assert!(dump.contains("INSERT INTO `z_last` (`id`) VALUES\n  (1);"), "{dump}");
}

#[tokio::test(flavor = "multi_thread")]
async fn mysql_dump_reads_one_consistent_snapshot() {
    server_dump_reads_one_consistent_snapshot(TestDb::mysql().await).await
}

#[tokio::test(flavor = "multi_thread")]
async fn mariadb_dump_reads_one_consistent_snapshot() {
    server_dump_reads_one_consistent_snapshot(TestDb::mariadb().await).await
}
