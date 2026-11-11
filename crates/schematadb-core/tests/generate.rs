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
