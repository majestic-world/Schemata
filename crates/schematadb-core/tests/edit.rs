mod common;

use common::TestDb;
use schematadb_core::{CellValue, ColumnValue, Engine, ErrorKind, PageRequest, RowChange, SortSpec};

fn text(s: &str) -> CellValue {
    CellValue::Text(s.into())
}
fn int(i: i64) -> CellValue {
    CellValue::Int(i)
}
fn set(column: &str, value: CellValue) -> ColumnValue {
    ColumnValue { column: column.into(), value }
}

async fn rows(t: &TestDb, table: &str) -> Vec<Vec<CellValue>> {
    let req = PageRequest { offset: 0, limit: 100, sort: Some(SortSpec { column: "id".into(), desc: false }), filters: vec![] };
    t.driver.fetch_page(&t.db, table, &req).await.unwrap().rows
}

async fn seed(t: &TestDb) {
    t.exec("CREATE TABLE item (id INT PRIMARY KEY, name VARCHAR(20) NOT NULL, qty INT); INSERT INTO item VALUES (1, 'a', 10), (2, 'b', 20), (3, 'c', 30);").await;
}

async fn applies_change_set_in_one_go(t: TestDb) {
    seed(&t).await;
    let changes = vec![
        RowChange::Update { key: vec![int(1)], values: vec![set("name", text("A")), set("qty", CellValue::Null)] },
        RowChange::Insert { values: vec![set("id", int(4)), set("name", text("d")), set("qty", int(40))] },
        RowChange::Delete { key: vec![int(2)] },
    ];
    let applied = t.driver.apply_changes(&t.db, "item", &changes).await.unwrap();
    assert_eq!(applied, 3);
    assert_eq!(
        rows(&t, "item").await,
        vec![vec![int(1), text("A"), CellValue::Null], vec![int(3), text("c"), int(30)], vec![int(4), text("d"), int(40)]]
    );
}
scenario!(applies_change_set_in_one_go);

async fn failure_rolls_back_and_points_at_change(t: TestDb) {
    seed(&t).await;
    let changes = vec![
        RowChange::Update { key: vec![int(1)], values: vec![set("name", text("changed"))] },
        RowChange::Insert { values: vec![set("id", int(2)), set("name", text("dup"))] },
    ];
    let err = t.driver.apply_changes(&t.db, "item", &changes).await.unwrap_err();
    assert_eq!(err.index, Some(1));
    assert_eq!(err.error.kind, ErrorKind::Sql);
    assert_eq!(rows(&t, "item").await[0][1], text("a"), "earlier changes must be rolled back");
}
scenario!(failure_rolls_back_and_points_at_change);

async fn vanished_row_is_reported(t: TestDb) {
    seed(&t).await;
    let changes = vec![RowChange::Delete { key: vec![int(99)] }];
    let err = t.driver.apply_changes(&t.db, "item", &changes).await.unwrap_err();
    assert_eq!(err.index, Some(0));
    // Writing the value a row already has is not a vanished row.
    let same = vec![RowChange::Update { key: vec![int(1)], values: vec![set("name", text("a"))] }];
    t.driver.apply_changes(&t.db, "item", &same).await.unwrap();
}
scenario!(vanished_row_is_reported);

async fn tables_without_key_are_edited_by_value(t: TestDb) {
    t.exec("CREATE TABLE loose (a INT, b VARCHAR(5)); INSERT INTO loose VALUES (1, 'x'), (1, 'x'), (2, NULL);").await;
    let all = PageRequest { offset: 0, limit: 10, sort: None, filters: vec![] };
    let page = t.driver.fetch_page(&t.db, "loose", &all).await.unwrap();
    if t.engine == Engine::Sqlite {
        // rowid identity: the hidden first column is the key.
        let rowid = page.rows[0][0].clone();
        let changes = vec![RowChange::Update { key: vec![rowid], values: vec![set("b", text("y"))] }];
        t.driver.apply_changes(&t.db, "loose", &changes).await.unwrap();
        let page = t.driver.fetch_page(&t.db, "loose", &all).await.unwrap();
        assert_eq!(page.rows[0][2], text("y"));
        return;
    }
    // Like Navicat: the key is every column's value; NULL matches NULL and only one of identical rows changes.
    let changes = vec![
        RowChange::Update { key: vec![int(1), text("x")], values: vec![set("b", text("y"))] },
        RowChange::Update { key: vec![int(2), CellValue::Null], values: vec![set("b", text("z"))] },
    ];
    t.driver.apply_changes(&t.db, "loose", &changes).await.unwrap();
    let mut values: Vec<Vec<CellValue>> = t.driver.fetch_page(&t.db, "loose", &all).await.unwrap().rows;
    values.sort_by_key(|r| format!("{r:?}"));
    assert_eq!(values, vec![vec![int(1), text("x")], vec![int(1), text("y")], vec![int(2), text("z")]]);

    t.driver.apply_changes(&t.db, "loose", &[RowChange::Delete { key: vec![int(1), text("x")] }]).await.unwrap();
    assert_eq!(t.driver.count_rows(&t.db, "loose", &[]).await.unwrap(), 2);
    let err = t.driver.apply_changes(&t.db, "loose", &[RowChange::Delete { key: vec![int(1), text("x")] }]).await.unwrap_err();
    assert_eq!(err.index, Some(0), "a row matched by value that no longer exists is reported");

    // Columns that cannot be matched exactly (here FLOAT and BLOB) leave nothing to match by: read-only.
    t.exec("CREATE TABLE opaque (f FLOAT, data BLOB)").await;
    let err = t.driver.apply_changes(&t.db, "opaque", &[RowChange::Delete { key: vec![] }]).await.unwrap_err();
    assert_eq!(err.error.kind, ErrorKind::Validation);
}
scenario!(tables_without_key_are_edited_by_value);

async fn written_values_round_trip(t: TestDb) {
    let sqlite = t.engine == Engine::Sqlite;
    if sqlite {
        t.exec("CREATE TABLE w (id INTEGER PRIMARY KEY, big INTEGER, amount TEXT, js TEXT, dt TEXT, s TEXT)").await;
    } else {
        t.exec("CREATE TABLE w (id BIGINT UNSIGNED PRIMARY KEY, big BIGINT, amount DECIMAL(30,10), js JSON, dt DATETIME, s VARCHAR(10))")
            .await;
    }
    let id = if sqlite { int(1) } else { CellValue::BigInt("18446744073709551615".into()) };
    let values = vec![
        set("id", id.clone()),
        set("big", CellValue::BigInt("-9223372036854775808".into())),
        set("amount", text("12345678901234567890.0123456789")),
        set("js", text("{\"k\": [1, 2]}")),
        set("dt", text("2024-02-29 23:59:59")),
        set("s", text("")),
    ];
    t.driver.apply_changes(&t.db, "w", &[RowChange::Insert { values }]).await.unwrap();
    let row = &rows(&t, "w").await[0];
    assert_eq!(row[0], id);
    assert_eq!(row[1], CellValue::BigInt("-9223372036854775808".into()));
    let amount = match &row[2] {
        CellValue::Decimal(s) | CellValue::Text(s) => s.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(amount, "12345678901234567890.0123456789");
    let js = match &row[3] {
        CellValue::Json(s) | CellValue::Text(s) => s.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(serde_json::from_str::<serde_json::Value>(&js).unwrap(), serde_json::json!({"k": [1, 2]}));
    let dt = match &row[4] {
        CellValue::DateTime(s) | CellValue::Text(s) => s.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(dt, "2024-02-29 23:59:59");
    assert_eq!(row[5], text(""));

    // Keyed by the unsigned maximum: updating to NULL works and stays distinct from ''.
    t.driver.apply_changes(&t.db, "w", &[RowChange::Update { key: vec![id], values: vec![set("s", CellValue::Null)] }]).await.unwrap();
    assert_eq!(rows(&t, "w").await[0][5], CellValue::Null);
}
scenario!(written_values_round_trip);
