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

fn page_request() -> PageRequest {
    PageRequest { offset: 0, limit: 100, sort: None, filters: vec![] }
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

async fn generated_columns_are_visible_but_not_writable(t: TestDb) {
    t.exec(
        "CREATE TABLE computed (id INT PRIMARY KEY, base INT, doubled INT GENERATED ALWAYS AS (base * 2) STORED, tail INT); \
            INSERT INTO computed (id, base, tail) VALUES (1, 7, 90)",
    )
    .await;
    let info = t.driver.table_info(&t.db, "computed").await.unwrap();
    assert!(info.columns[2].generated);
    assert!(!info.columns[3].generated);
    let page = t.driver.fetch_page(&t.db, "computed", &page_request()).await.unwrap();
    assert_eq!(page.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id", "base", "doubled", "tail"]);
    assert_eq!(page.rows[0], vec![int(1), int(7), int(14), int(90)]);
    t.driver.apply_changes(&t.db, "computed", &[RowChange::Update { key: vec![int(1)], values: vec![set("base", int(8))] }]).await.unwrap();
    let error = t
        .driver
        .apply_changes(
            &t.db,
            "computed",
            &[
                RowChange::Update { key: vec![int(1)], values: vec![set("tail", int(91))] },
                RowChange::Update { key: vec![int(1)], values: vec![set("doubled", int(0))] },
            ],
        )
        .await
        .unwrap_err();
    assert_eq!(error.error.kind, ErrorKind::Validation);
    assert_eq!(error.index, Some(1));
    let page = t.driver.fetch_page(&t.db, "computed", &page_request()).await.unwrap();
    assert_eq!(page.rows[0], vec![int(1), int(8), int(16), int(90)]);
    assert!(t.driver.table_design(&t.db, "computed").await.is_err());
}
scenario!(generated_columns_are_visible_but_not_writable);

async fn sqlite_partial_and_expression_indexes_never_identify_rows(t: TestDb) {
    if t.engine != Engine::Sqlite {
        return;
    }
    t.exec(
        "CREATE TABLE indexed (a INT NOT NULL, b INT, enabled INT); \
            CREATE UNIQUE INDEX partial_key ON indexed (a) WHERE enabled = 1; \
            CREATE UNIQUE INDEX expression_key ON indexed (a, abs(b)); \
            INSERT INTO indexed VALUES (1, 2, 0), (1, 3, 0)",
    )
    .await;
    assert_eq!(t.driver.table_info(&t.db, "indexed").await.unwrap().row_identity, schematadb_core::RowIdentity::RowId);
    let page = t.driver.fetch_page(&t.db, "indexed", &page_request()).await.unwrap();
    t.driver
        .apply_changes(&t.db, "indexed", &[RowChange::Update { key: vec![page.rows[0][0].clone()], values: vec![set("enabled", int(1))] }])
        .await
        .unwrap();
    let page = t.driver.fetch_page(&t.db, "indexed", &page_request()).await.unwrap();
    assert_eq!(page.rows.iter().filter(|r| r[3] == int(1)).count(), 1);
}
scenario!(sqlite_partial_and_expression_indexes_never_identify_rows);

async fn sqlite_uses_unshadowed_rowid_or_is_read_only(t: TestDb) {
    if t.engine != Engine::Sqlite {
        return;
    }
    t.exec(
        "CREATE TABLE shadowed (rowid INT, _rowid_ INT, payload INT); \
            INSERT INTO shadowed VALUES (10, 20, 1), (10, 20, 2)",
    )
    .await;
    let page = t.driver.fetch_page(&t.db, "shadowed", &page_request()).await.unwrap();
    assert_ne!(page.rows[0][0], page.rows[1][0]);
    t.driver.apply_changes(&t.db, "shadowed", &[RowChange::Delete { key: vec![page.rows[0][0].clone()] }]).await.unwrap();
    assert_eq!(t.driver.fetch_page(&t.db, "shadowed", &page_request()).await.unwrap().rows.len(), 1);
    t.exec("CREATE TABLE fully_shadowed (rowid INT, _rowid_ INT, oid INT); INSERT INTO fully_shadowed VALUES (1, 1, 1)").await;
    assert_eq!(t.driver.table_info(&t.db, "fully_shadowed").await.unwrap().row_identity, schematadb_core::RowIdentity::None);
    assert!(t.driver.apply_changes(&t.db, "fully_shadowed", &[RowChange::Delete { key: vec![int(1)] }]).await.is_err());
}
scenario!(sqlite_uses_unshadowed_rowid_or_is_read_only);

async fn non_transactional_changes_are_rejected_before_writing(t: TestDb) {
    if t.engine == Engine::Sqlite {
        return;
    }
    t.exec("CREATE TABLE non_atomic (id INT PRIMARY KEY, amount INT) ENGINE=MyISAM; INSERT INTO non_atomic VALUES (1, 10)").await;
    let error = t
        .driver
        .apply_changes(
            &t.db,
            "non_atomic",
            &[
                RowChange::Update { key: vec![int(1)], values: vec![set("amount", int(20))] },
                RowChange::Insert { values: vec![set("id", int(1))] },
            ],
        )
        .await
        .unwrap_err();
    assert_eq!(error.error.kind, ErrorKind::Validation);
    assert_eq!(rows(&t, "non_atomic").await[0], vec![int(1), int(10)]);
}
scenario!(non_transactional_changes_are_rejected_before_writing);

async fn literal_rowid_alias_primary_key_addresses_its_own_column(t: TestDb) {
    t.exec("CREATE TABLE literal_key (__rowid__ INT PRIMARY KEY, payload INT); INSERT INTO literal_key VALUES (10, 1), (20, 2)").await;
    t.driver
        .apply_changes(&t.db, "literal_key", &[RowChange::Update { key: vec![int(20)], values: vec![set("payload", int(7))] }])
        .await
        .unwrap();
    let page = t.driver.fetch_page(&t.db, "literal_key", &page_request()).await.unwrap();
    let row = page.rows.iter().find(|row| row[0] == int(20)).unwrap();
    assert_eq!(row[1], int(7));
    assert_eq!(page.rows.iter().find(|row| row[0] == int(10)).unwrap()[1], int(1));
}
scenario!(literal_rowid_alias_primary_key_addresses_its_own_column);

async fn sqlite_reserved_rowid_alias_without_key_is_read_only(t: TestDb) {
    if t.engine != Engine::Sqlite {
        return;
    }
    t.exec("CREATE TABLE reserved_alias (__rowid__ INT, payload INT); INSERT INTO reserved_alias VALUES (10, 1)").await;
    assert_eq!(t.driver.table_info(&t.db, "reserved_alias").await.unwrap().row_identity, schematadb_core::RowIdentity::None);
    let page = t.driver.fetch_page(&t.db, "reserved_alias", &page_request()).await.unwrap();
    assert_eq!(page.columns.iter().filter(|c| c.name == "__rowid__").count(), 1);
    assert!(!page.columns[0].hidden);
    assert_eq!(page.rows[0], vec![int(10), int(1)]);
    assert!(
        t.driver
            .apply_changes(&t.db, "reserved_alias", &[RowChange::Update { key: vec![int(10)], values: vec![set("payload", int(7))] }])
            .await
            .is_err()
    );
}
scenario!(sqlite_reserved_rowid_alias_without_key_is_read_only);
