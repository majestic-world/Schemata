mod common;

use common::TestDb;
use schematadb_core::{
    CancellationToken, CellValue, ColumnGenerator, Engine, ErrorKind, GenerateSpec, GeneratorKind, ProgressFn, generate_data,
    preview_generated, suggest_generators,
};
use std::sync::Arc;

fn no_progress() -> ProgressFn {
    Arc::new(|_| {})
}

async fn seed(t: &TestDb) {
    let sqlite = t.engine == Engine::Sqlite;
    let auto = if sqlite { "INTEGER PRIMARY KEY AUTOINCREMENT" } else { "INT AUTO_INCREMENT PRIMARY KEY" };
    let status = if sqlite { "VARCHAR(10)" } else { "ENUM('new','paid','shipped')" };
    t.exec(&format!(
        "CREATE TABLE customer (id {auto}, name VARCHAR(40) NOT NULL, email VARCHAR(80) NOT NULL UNIQUE, age INT, active BOOLEAN,
             created_at DATETIME, notes TEXT);
         CREATE TABLE orders (id {auto}, customer_id INT NOT NULL, total DECIMAL(10,2), status {status},
             CONSTRAINT fk_orders_customer FOREIGN KEY (customer_id) REFERENCES customer (id));"
    ))
    .await;
}

async fn scalar(t: &TestDb, sql: &str) -> i64 {
    let r = t.driver.execute_script(Some(&t.db), sql, 10, None).await.unwrap();
    match &r.outcomes[0] {
        schematadb_core::StatementOutcome::Rows { rows, .. } => match &rows[0][0] {
            CellValue::Int(i) => *i,
            CellValue::BigInt(s) | CellValue::Decimal(s) | CellValue::Text(s) => s.parse().unwrap(),
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    }
}

async fn spec(t: &TestDb, table: &str, rows: u64, empty_first: bool) -> GenerateSpec {
    let info = t.driver.table_info(&t.db, table).await.unwrap();
    GenerateSpec { rows, columns: suggest_generators(&info), empty_first }
}

async fn suggests_and_fills_tables_respecting_constraints(t: TestDb) {
    seed(&t).await;
    let info = t.driver.table_info(&t.db, "customer").await.unwrap();
    let kinds: Vec<(String, GeneratorKind)> = suggest_generators(&info).into_iter().map(|g| (g.column, g.kind)).collect();
    let kind = |name: &str| kinds.iter().find(|(c, _)| c == name).unwrap().1;
    assert_eq!(kind("id"), GeneratorKind::Skip);
    assert_eq!(kind("name"), GeneratorKind::FullName);
    assert_eq!(kind("email"), GeneratorKind::Email);
    assert_eq!(kind("active"), GeneratorKind::Boolean);
    assert_eq!(kind("created_at"), GeneratorKind::DateTime);
    assert_eq!(kind("notes"), GeneratorKind::Paragraph);

    // Orders reference customers, so they need customers first.
    let err =
        generate_data(t.driver.as_ref(), &t.db, "orders", &spec(&t, "orders", 5, false).await, no_progress(), CancellationToken::new())
            .await
            .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);

    let report = generate_data(
        t.driver.as_ref(),
        &t.db,
        "customer",
        &spec(&t, "customer", 400, false).await,
        no_progress(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(report.rows, 400);
    assert_eq!(scalar(&t, "SELECT COUNT(DISTINCT email) FROM customer").await, 400);
    assert_eq!(scalar(&t, "SELECT COUNT(*) FROM customer WHERE LENGTH(name) > 40 OR age NOT BETWEEN 18 AND 90").await, 0);

    let orders = spec(&t, "orders", 300, false).await;
    assert_eq!(orders.columns.iter().find(|g| g.column == "customer_id").unwrap().kind, GeneratorKind::ForeignKey);
    generate_data(t.driver.as_ref(), &t.db, "orders", &orders, no_progress(), CancellationToken::new()).await.unwrap();
    assert_eq!(scalar(&t, "SELECT COUNT(*) FROM orders o JOIN customer c ON c.id = o.customer_id").await, 300);
    if t.engine != Engine::Sqlite {
        assert_eq!(scalar(&t, "SELECT COUNT(*) FROM orders WHERE status NOT IN ('new','paid','shipped')").await, 0);
    }
}
scenario!(suggests_and_fills_tables_respecting_constraints);

async fn preview_inserts_nothing_and_empty_first_replaces_rows(t: TestDb) {
    seed(&t).await;
    let columns = vec![
        ColumnGenerator { column: "name".into(), kind: GeneratorKind::FirstName },
        ColumnGenerator { column: "email".into(), kind: GeneratorKind::Email },
    ];
    let page = preview_generated(t.driver.as_ref(), &t.db, "customer", &columns, 5).await.unwrap();
    assert_eq!(page.rows.len(), 5);
    assert_eq!(page.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["name", "email"]);
    assert_eq!(scalar(&t, "SELECT COUNT(*) FROM customer").await, 0);

    let run = |rows, empty_first| GenerateSpec { rows, columns: columns.clone(), empty_first };
    generate_data(t.driver.as_ref(), &t.db, "customer", &run(20, false), no_progress(), CancellationToken::new()).await.unwrap();
    generate_data(t.driver.as_ref(), &t.db, "customer", &run(7, true), no_progress(), CancellationToken::new()).await.unwrap();
    assert_eq!(scalar(&t, "SELECT COUNT(*) FROM customer").await, 7);
}
scenario!(preview_inserts_nothing_and_empty_first_replaces_rows);

async fn failed_replacement_keeps_original_rows(t: TestDb) {
    t.exec("CREATE TABLE replacement (id INT PRIMARY KEY, required INT NOT NULL); INSERT INTO replacement VALUES (99, 42)").await;
    let invalid =
        GenerateSpec { rows: 3, empty_first: true, columns: vec![ColumnGenerator { column: "id".into(), kind: GeneratorKind::Sequence }] };
    assert!(generate_data(t.driver.as_ref(), &t.db, "replacement", &invalid, no_progress(), CancellationToken::new()).await.is_err());
    assert_eq!(scalar(&t, "SELECT required FROM replacement WHERE id = 99").await, 42);
    let mut invalid_plan = invalid.clone();
    invalid_plan.columns[0].column = "missing".into();
    assert!(generate_data(t.driver.as_ref(), &t.db, "replacement", &invalid_plan, no_progress(), CancellationToken::new()).await.is_err());
    assert_eq!(scalar(&t, "SELECT COUNT(*) FROM replacement").await, 1);
    let token = CancellationToken::new();
    token.cancel();
    assert_eq!(
        generate_data(t.driver.as_ref(), &t.db, "replacement", &invalid, no_progress(), token).await.unwrap_err().kind,
        ErrorKind::Cancelled
    );
    assert_eq!(scalar(&t, "SELECT required FROM replacement WHERE id = 99").await, 42);
}
scenario!(failed_replacement_keeps_original_rows);

async fn generated_columns_are_skipped_by_generation(t: TestDb) {
    t.exec("CREATE TABLE generated_data (id INT PRIMARY KEY, base INT NOT NULL, doubled INT GENERATED ALWAYS AS (base * 2) STORED)").await;
    let spec = spec(&t, "generated_data", 5, false).await;
    assert_eq!(spec.columns.iter().find(|c| c.column == "doubled").unwrap().kind, GeneratorKind::Skip);
    generate_data(t.driver.as_ref(), &t.db, "generated_data", &spec, no_progress(), CancellationToken::new()).await.unwrap();
    assert_eq!(scalar(&t, "SELECT COUNT(*) FROM generated_data WHERE doubled = base * 2").await, 5);
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    tx.send(Ok(vec![(1, vec![schematadb_core::Value::Int(1), schematadb_core::Value::Int(2)])])).await.unwrap();
    drop(tx);
    let error = t.driver.import_rows(&t.db, "generated_data", &["id".into(), "doubled".into()], rx, false, true).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Validation);
    assert_eq!(scalar(&t, "SELECT COUNT(*) FROM generated_data WHERE doubled = base * 2").await, 5);
}
scenario!(generated_columns_are_skipped_by_generation);

async fn non_transactional_generation_is_rejected_before_empty_first(t: TestDb) {
    if t.engine == Engine::Sqlite {
        return;
    }
    t.exec("CREATE TABLE legacy_data (id INT PRIMARY KEY) ENGINE=MyISAM; INSERT INTO legacy_data VALUES (99)").await;
    let spec =
        GenerateSpec { rows: 3, empty_first: true, columns: vec![ColumnGenerator { column: "id".into(), kind: GeneratorKind::Sequence }] };
    let error = generate_data(t.driver.as_ref(), &t.db, "legacy_data", &spec, no_progress(), CancellationToken::new()).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Validation);
    assert_eq!(scalar(&t, "SELECT id FROM legacy_data").await, 99);
}
scenario!(non_transactional_generation_is_rejected_before_empty_first);

async fn sequences_reject_overflow(t: TestDb) {
    t.exec("CREATE TABLE big_seq (id BIGINT PRIMARY KEY); INSERT INTO big_seq VALUES (9223372036854775806)").await;
    let columns = vec![ColumnGenerator { column: "id".into(), kind: GeneratorKind::Sequence }];
    let error = preview_generated(t.driver.as_ref(), &t.db, "big_seq", &columns, 5).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Validation);
    let spec = GenerateSpec { rows: 5, empty_first: false, columns };
    let error = generate_data(t.driver.as_ref(), &t.db, "big_seq", &spec, no_progress(), CancellationToken::new()).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Validation);
    assert_eq!(scalar(&t, "SELECT COUNT(*) FROM big_seq").await, 1);
}
scenario!(sequences_reject_overflow);

async fn sqlite_foreign_key_without_target_columns_uses_parent_primary_key(t: TestDb) {
    if t.engine != Engine::Sqlite {
        return;
    }
    t.exec(
        "CREATE TABLE parent (id INTEGER PRIMARY KEY, name TEXT); INSERT INTO parent VALUES (7, 'a'), (8, 'b');
         CREATE TABLE child (id INTEGER PRIMARY KEY, parent_id INTEGER NOT NULL REFERENCES parent);",
    )
    .await;
    let info = t.driver.table_info(&t.db, "child").await.unwrap();
    assert_eq!(info.foreign_keys[0].ref_columns, vec!["id".to_owned()]);
    let spec = spec(&t, "child", 10, false).await;
    assert_eq!(spec.columns.iter().find(|c| c.column == "parent_id").unwrap().kind, GeneratorKind::ForeignKey);
    generate_data(t.driver.as_ref(), &t.db, "child", &spec, no_progress(), CancellationToken::new()).await.unwrap();
    assert_eq!(scalar(&t, "SELECT COUNT(*) FROM child WHERE parent_id IN (7, 8)").await, 10);
}
scenario!(sqlite_foreign_key_without_target_columns_uses_parent_primary_key);
