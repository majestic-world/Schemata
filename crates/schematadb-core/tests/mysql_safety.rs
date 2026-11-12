mod common;

use common::TestDb;
use schematadb_core::{
    CancellationToken, CellValue, ColumnValue, DumpOptions, Engine, ErrorKind, ImportOptions, PageRequest, ProgressFn, RowChange, SortSpec,
    StatementOutcome, TextEncoding, Value, dump_database, is_read_only_query, run_sql_file,
};
use std::sync::Arc;

fn no_progress() -> ProgressFn {
    Arc::new(|_| {})
}

/// Rows of the last outcome of `sql`, run on a pool connection.
async fn query(t: &TestDb, sql: &str) -> Vec<Vec<CellValue>> {
    let result = t.driver.execute_script(Some(&t.db), sql, 1000, None).await.unwrap();
    assert!(result.error.is_none(), "{:?}", result.error);
    match result.outcomes.into_iter().last() {
        Some(StatementOutcome::Rows { rows, .. }) => rows,
        other => panic!("Unexpected result: {other:?}"),
    }
}

async fn scalar(t: &TestDb, sql: &str) -> CellValue {
    query(t, sql).await.remove(0).remove(0)
}

fn text(value: &str) -> CellValue {
    CellValue::Text(value.into())
}

async fn update(t: &TestDb, table: &str, id: i64, column: &str, value: CellValue) -> Result<u64, schematadb_core::ChangeError> {
    let identity = t.driver.table_info(&t.db, table).await.unwrap().row_identity;
    let change =
        RowChange::Update { key: vec![CellValue::Int(id)], values: vec![ColumnValue { column: column.into(), value }], expected: vec![] };
    t.driver.apply_changes(&t.db, table, &identity, &[change]).await
}

async fn run_file(t: &TestDb, sql: &str, stop_on_error: bool) -> schematadb_core::ScriptRunReport {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("script.sql");
    std::fs::write(&file, sql).unwrap();
    run_sql_file(t.driver.as_ref(), &t.db, &file, TextEncoding::Utf8, stop_on_error, no_progress(), CancellationToken::new()).await.unwrap()
}

/// Session settings of a script that stops at an error never reach the pooled connections used by the grid, imports
/// and DDL. Six concurrent sleepers occupy every pooled connection, including any the script could have used.
async fn run_sql_file_does_not_leak_session_state(t: TestDb) {
    if t.engine == Engine::Sqlite {
        return;
    }
    t.exec("CREATE TABLE leak (id INT PRIMARY KEY) ENGINE = InnoDB").await;
    let report = run_file(
        &t,
        "SET time_zone = '+05:00';\nSET NAMES latin1;\nSET UNIQUE_CHECKS = 0;\nSET SESSION sql_mode = '';\n\
         START TRANSACTION;\nINSERT INTO leak VALUES (1);\nSELECT * FROM missing_table;\n",
        true,
    )
    .await;
    assert_eq!(report.failed.len(), 1, "{:?}", report.failed);
    assert!(report.open_transaction_rolled_back);
    let checks = (0..6).map(|_| {
        query(&t, "SELECT @@SESSION.time_zone, @@SESSION.character_set_client, @@SESSION.unique_checks, @@SESSION.sql_mode, SLEEP(0.5)")
    });
    for rows in futures::future::join_all(checks).await {
        let row = &rows[0];
        assert_ne!(row[0], text("+05:00"));
        assert_eq!(row[1], text("utf8mb4"));
        assert_eq!(row[2], CellValue::Int(1));
        let CellValue::Text(mode) = &row[3] else { panic!("sql_mode {:?}", row[3]) };
        assert!(mode.split(',').any(|m| m == "STRICT_ALL_TABLES"), "{mode}");
    }
    assert_eq!(scalar(&t, "SELECT COUNT(*) FROM leak").await, CellValue::Int(0));
}
scenario!(run_sql_file_does_not_leak_session_state);

/// A script that leaves its transaction open is rolled back and says so; a committed one is not flagged.
async fn open_transaction_is_reported(t: TestDb) {
    if t.engine == Engine::Sqlite {
        return;
    }
    t.exec("CREATE TABLE tx (id INT PRIMARY KEY) ENGINE = InnoDB").await;
    let open = run_file(&t, "START TRANSACTION;\nINSERT INTO tx VALUES (1);\n", true).await;
    assert!(open.failed.is_empty(), "{:?}", open.failed);
    assert!(open.open_transaction_rolled_back);
    assert_eq!(scalar(&t, "SELECT COUNT(*) FROM tx").await, CellValue::Int(0));

    let committed = run_file(&t, "START TRANSACTION;\nINSERT INTO tx VALUES (2);\nCOMMIT;\n", true).await;
    assert!(!committed.open_transaction_rolled_back);
    assert_eq!(scalar(&t, "SELECT COUNT(*) FROM tx").await, CellValue::Int(1));
}
scenario!(open_transaction_is_reported);

/// Pooled writes run in strict mode, whatever the server default: a typo is refused instead of stored as 12.
async fn grid_edits_are_strict(t: TestDb) {
    if t.engine == Engine::Sqlite {
        return;
    }
    t.exec("CREATE TABLE strict_edit (id INT PRIMARY KEY, n INT) ENGINE = InnoDB; INSERT INTO strict_edit VALUES (1, 5)").await;
    let CellValue::Text(mode) = scalar(&t, "SELECT @@SESSION.sql_mode").await else { panic!("sql_mode") };
    assert!(mode.split(',').any(|m| m == "STRICT_ALL_TABLES"), "{mode}");
    assert!(!mode.split(',').any(|m| m == "NO_BACKSLASH_ESCAPES"), "{mode}");
    assert!(update(&t, "strict_edit", 1, "n", text("12abc")).await.is_err());
    assert_eq!(scalar(&t, "SELECT n FROM strict_edit").await, CellValue::Int(5));
}
scenario!(grid_edits_are_strict);

/// Text typed into a BIT cell is the number it spells, not the bytes of its characters ('6' would store 54).
async fn bit_edits_store_numbers(t: TestDb) {
    if t.engine == Engine::Sqlite {
        return;
    }
    t.exec("CREATE TABLE bits (id INT PRIMARY KEY, f BIT(8)) ENGINE = InnoDB; INSERT INTO bits VALUES (1, 5), (2, 5), (3, 5)").await;
    update(&t, "bits", 1, "f", text("6")).await.unwrap();
    update(&t, "bits", 2, "f", text("b'110'")).await.unwrap();
    update(&t, "bits", 3, "f", text("0x06")).await.unwrap();
    let error = update(&t, "bits", 1, "f", text("abc")).await.unwrap_err();
    assert_eq!(error.error.kind, ErrorKind::Validation, "{error:?}");
    assert_eq!(query(&t, "SELECT f + 0 FROM bits ORDER BY id").await, vec![vec![CellValue::Int(6)]; 3]);
}
scenario!(bit_edits_store_numbers);

/// DECIMAL keys compare as decimals: as strings MySQL compares them in floating point, where both rows are equal.
async fn decimal_keys_match_exactly(t: TestDb) {
    if t.engine == Engine::Sqlite {
        return;
    }
    t.exec(
        "CREATE TABLE dec_key (k DECIMAL(30,20) PRIMARY KEY, v INT) ENGINE = InnoDB; \
         INSERT INTO dec_key VALUES (0.10000000000000000001, 1), (0.10000000000000000002, 2)",
    )
    .await;
    let identity = t.driver.table_info(&t.db, "dec_key").await.unwrap().row_identity;
    let change = RowChange::Update {
        key: vec![CellValue::Decimal("0.10000000000000000002".into())],
        values: vec![ColumnValue { column: "v".into(), value: CellValue::Int(20) }],
        expected: vec![],
    };
    t.driver.apply_changes(&t.db, "dec_key", &identity, &[change]).await.unwrap();
    assert_eq!(query(&t, "SELECT v FROM dec_key ORDER BY k").await, vec![vec![CellValue::Int(1)], vec![CellValue::Int(20)]]);
}
scenario!(decimal_keys_match_exactly);

/// Dates with a zero month or day are read as stored, through both protocols.
async fn partial_zero_dates_are_read_as_stored(t: TestDb) {
    if t.engine == Engine::Sqlite {
        return;
    }
    let setup = t
        .driver
        .execute_in_session(
            "zero",
            Some(&t.db),
            "SET SESSION sql_mode = ''; CREATE TABLE zero_dates (id INT PRIMARY KEY, d DATE, dt DATETIME(6)); \
             INSERT INTO zero_dates VALUES (1, '2024-05-00', '2024-00-07 10:20:30.250000')",
            10,
            None,
        )
        .await
        .unwrap();
    assert!(setup.error.is_none(), "{:?}", setup.error);
    let expected = vec![CellValue::Int(1), CellValue::DateTime("2024-05-00".into()), CellValue::DateTime("2024-00-07 10:20:30.250".into())];
    let req = PageRequest { offset: 0, limit: 10, sort: Some(SortSpec { column: "id".into(), desc: false }), filters: vec![] };
    assert_eq!(t.driver.fetch_page(&t.db, "zero_dates", &req).await.unwrap().rows, vec![expected.clone()]);
    let text_protocol = t.driver.execute_in_session("zero", Some(&t.db), "SELECT id, d, dt FROM zero_dates", 10, None).await.unwrap();
    let Some(StatementOutcome::Rows { rows, .. }) = text_protocol.outcomes.last() else { panic!("rows") };
    assert_eq!(rows[0][1], expected[1]);
    assert_eq!(rows[0][2], CellValue::DateTime("2024-00-07 10:20:30.250000".into()));
}
scenario!(partial_zero_dates_are_read_as_stored);

/// Each result set of a CALL is its own outcome, decoded with its own columns.
async fn call_returns_every_result_set(t: TestDb) {
    if t.engine == Engine::Sqlite {
        return;
    }
    t.exec("DELIMITER ;;\nCREATE PROCEDURE two_sets() BEGIN SELECT 1 AS a; SELECT 'x' AS b, 2 AS c; END;;\nDELIMITER ;").await;
    let result = t.driver.execute_in_session("call", Some(&t.db), "CALL two_sets()", 10, None).await.unwrap();
    assert!(result.error.is_none(), "{:?}", result.error);
    let sets: Vec<(Vec<String>, Vec<Vec<CellValue>>)> = result
        .outcomes
        .into_iter()
        .map(|o| match o {
            StatementOutcome::Rows { columns, rows, .. } => (columns.into_iter().map(|c| c.name).collect(), rows),
            other => panic!("Unexpected outcome {other:?}"),
        })
        .collect();
    assert_eq!(
        sets,
        vec![
            (vec!["a".to_owned()], vec![vec![CellValue::Int(1)]]),
            (vec!["b".to_owned(), "c".to_owned()], vec![vec![text("x"), CellValue::Int(2)]]),
        ]
    );
}
scenario!(call_returns_every_result_set);

/// "Load more" runs the statement again, so it is refused for anything that is not read-only.
async fn load_more_refuses_writes(t: TestDb) {
    if t.engine == Engine::Sqlite {
        return;
    }
    t.exec("CREATE TABLE more_rows (id INT PRIMARY KEY) ENGINE = InnoDB").await;
    for statement in ["INSERT INTO more_rows VALUES (1)", "SELECT GET_LOCK('schematadb_more', 0)"] {
        let error = t.driver.fetch_more_in_session("more", Some(&t.db), statement, 0, 10).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::Validation, "{statement}: {error:?}");
        let error = t.driver.fetch_more(Some(&t.db), statement, 0, 10).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::Validation, "{statement}: {error:?}");
    }
    assert_eq!(scalar(&t, "SELECT IS_FREE_LOCK('schematadb_more')").await, CellValue::Int(1));
    assert_eq!(scalar(&t, "SELECT COUNT(*) FROM more_rows").await, CellValue::Int(0));
}
scenario!(load_more_refuses_writes);

#[test]
fn read_only_check_catches_hidden_writes() {
    for sql in ["SELECT 1", "/* c */ (SELECT a FROM t) UNION SELECT 2", "SHOW TABLES", "EXPLAIN SELECT * FROM t", "DESCRIBE t"] {
        assert!(is_read_only_query(sql), "{sql}");
    }
    for sql in [
        "EXPLAIN ANALYZE DELETE FROM t",
        "DESCRIBE UPDATE t SET a = 1",
        "SELECT GET_LOCK('x', 1)",
        "SELECT @n := @n + 1 FROM t",
        "SELECT NEXTVAL(s)",
        "SELECT * FROM t FOR UPDATE",
        "CALL p()",
        "INSERT INTO t VALUES (1)",
    ] {
        assert!(!is_read_only_query(sql), "{sql}");
    }
}

/// Imports of UTC TIMESTAMP values store the instant they name, and the session time zone does not leak.
async fn utc_imports_keep_instants(t: TestDb) {
    if t.engine == Engine::Sqlite {
        return;
    }
    t.exec("CREATE TABLE utc_import (id INT PRIMARY KEY, at TIMESTAMP NULL) ENGINE = InnoDB").await;
    let pool_zone = scalar(&t, "SELECT @@SESSION.time_zone").await;
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    tx.send(Ok(vec![(1, vec![Value::Int(1), Value::DateTime("2024-01-01 00:00:00".into())])])).await.unwrap();
    drop(tx);
    let options = ImportOptions { utc_timestamps: true, ..Default::default() };
    let report = t.driver.import_rows(&t.db, "utc_import", &["id".into(), "at".into()], rx, options).await.unwrap();
    assert_eq!(report.inserted, 1);
    assert_eq!(scalar(&t, "SELECT UNIX_TIMESTAMP(at) FROM utc_import").await, CellValue::Int(1_704_067_200));
    let zones = (0..6).map(|_| query(&t, "SELECT @@SESSION.time_zone, SLEEP(0.2)"));
    for rows in futures::future::join_all(zones).await {
        assert_eq!(rows[0][0], pool_zone);
    }
}
scenario!(utc_imports_keep_instants);

/// Backups include routines and events (without DEFINER) and pin the restore session; restoring them works,
/// including a view that calls a stored function.
async fn dump_includes_routines(t: TestDb) {
    if t.engine == Engine::Sqlite {
        return;
    }
    t.exec(
        "CREATE TABLE amounts (id INT PRIMARY KEY, v INT) ENGINE = InnoDB; INSERT INTO amounts VALUES (1, 20);\n\
         DELIMITER ;;\n\
         CREATE FUNCTION doubled(x INT) RETURNS INT DETERMINISTIC RETURN x * 2;;\n\
         CREATE PROCEDURE add_amount(IN n INT) BEGIN INSERT INTO amounts VALUES (n, n); SELECT COUNT(*) FROM amounts; END;;\n\
         CREATE EVENT nightly ON SCHEDULE EVERY 1 DAY STARTS '2030-01-01 00:00:00' DISABLE DO DELETE FROM amounts WHERE v < 0;;\n\
         DELIMITER ;\n\
         CREATE VIEW doubled_amounts AS SELECT id, doubled(v) AS v2 FROM amounts;",
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("dump.sql");
    dump_database(t.driver.as_ref(), &t.db, &file, &DumpOptions { structure: true, data: true }, no_progress(), CancellationToken::new())
        .await
        .unwrap();
    let dump = std::fs::read_to_string(&file).unwrap();
    for expected in ["CREATE PROCEDURE `add_amount`", "CREATE FUNCTION `doubled`", "CREATE EVENT `nightly`", "SET time_zone = '+00:00';"] {
        assert!(dump.contains(expected), "missing {expected}:\n{dump}");
    }
    assert!(dump.contains("SET SESSION sql_mode = 'NO_AUTO_VALUE_ON_ZERO';"), "{dump}");
    assert!(!dump.contains("DEFINER="), "{dump}");

    let server = t.server.unwrap();
    let target = format!("{}_restore", t.db);
    t.exec(&format!("CREATE DATABASE `{target}`")).await;
    let restored = schematadb_core::connect(&server.params(Some(&target))).await.unwrap();
    let run =
        run_sql_file(restored.as_ref(), &target, &file, TextEncoding::Utf8, true, no_progress(), CancellationToken::new()).await.unwrap();
    let call = restored.execute_script(Some(&target), "CALL add_amount(2); SELECT v2 FROM doubled_amounts WHERE id = 1", 10, None).await;
    t.exec(&format!("DROP DATABASE `{target}`")).await;
    assert!(run.failed.is_empty(), "{:?}", run.failed);
    let call = call.unwrap();
    assert!(call.error.is_none(), "{:?}", call.error);
    let Some(StatementOutcome::Rows { rows, .. }) = call.outcomes.last() else { panic!("rows") };
    assert_eq!(rows, &vec![vec![CellValue::Int(40)]]);
}
scenario!(dump_includes_routines);

/// Indexes the designer cannot model (prefix, descending, functional, invisible) are left out of the design, so an
/// unrelated change neither drops them nor is blocked by them.
async fn unmodelled_indexes_survive_designer_changes(t: TestDb) {
    if t.engine == Engine::Sqlite {
        return;
    }
    let extra = if t.engine == Engine::MySql {
        ", KEY k_upper ((UPPER(note))), KEY k_hidden (note) INVISIBLE"
    } else {
        ", KEY k_hidden (note) IGNORED"
    };
    t.exec(&format!(
        "CREATE TABLE indexed (id INT PRIMARY KEY, email VARCHAR(200), note VARCHAR(20), \
         UNIQUE KEY k_prefix (email(20)), KEY k_desc (note DESC), KEY k_plain (note){extra}) ENGINE = InnoDB"
    ))
    .await;
    let original = t.driver.table_design(&t.db, "indexed").await.unwrap();
    assert_eq!(original.indexes.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), vec!["k_plain"]);
    let info = t.driver.table_info(&t.db, "indexed").await.unwrap();
    assert!(info.indexes.iter().any(|i| i.name == "k_prefix"));

    let mut changed = original.clone();
    changed.columns[2].comment = Some("changed".into());
    changed.indexes[0].comment = Some("plain".into());
    let ddl = t.driver.plan_table(&t.db, Some(&original), &changed).await.unwrap();
    t.driver.apply_ddl(&t.db, &ddl).await.unwrap();
    let create = t.driver.table_info(&t.db, "indexed").await.unwrap().create_sql;
    assert!(create.contains("`email`(20)"), "{create}");
    assert!(create.contains("`note` DESC"), "{create}");
    assert!(create.contains("k_hidden"), "{create}");
}
scenario!(unmodelled_indexes_survive_designer_changes);

/// Only the columns a CHANGE COLUMN redefines are checked for attributes the designer cannot preserve.
async fn designer_guard_checks_redefined_columns_only(t: TestDb) {
    if t.engine == Engine::Sqlite {
        return;
    }
    t.exec(
        "CREATE TABLE guarded (id INT PRIMARY KEY, updated TIMESTAMP NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP, \
         label VARCHAR(10)) ENGINE = InnoDB",
    )
    .await;
    let original = t.driver.table_design(&t.db, "guarded").await.unwrap();
    let mut changed = original.clone();
    changed.columns[2].comment = Some("label".into());
    let ddl = t.driver.plan_table(&t.db, Some(&original), &changed).await.unwrap();
    t.driver.apply_ddl(&t.db, &ddl).await.unwrap();
    assert!(t.driver.table_info(&t.db, "guarded").await.unwrap().create_sql.to_ascii_uppercase().contains("ON UPDATE"));

    let original = t.driver.table_design(&t.db, "guarded").await.unwrap();
    let mut changed = original.clone();
    changed.columns[1].comment = Some("touched".into());
    assert!(t.driver.plan_table(&t.db, Some(&original), &changed).await.is_err());
}
scenario!(designer_guard_checks_redefined_columns_only);

/// MariaDB keeps column-level CHECK constraints on the column, so redefining it would drop them.
async fn mariadb_column_checks_are_not_dropped(t: TestDb) {
    if t.engine != Engine::MariaDb {
        return;
    }
    t.exec("CREATE TABLE checked (id INT PRIMARY KEY, age INT CHECK (age > 0)) ENGINE = InnoDB").await;
    let original = t.driver.table_design(&t.db, "checked").await.unwrap();
    let mut changed = original.clone();
    changed.columns[1].comment = Some("years".into());
    let error = t.driver.plan_table(&t.db, Some(&original), &changed).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Validation, "{error:?}");
}
scenario!(mariadb_column_checks_are_not_dropped);

/// Expression defaults with charset introducers and binary defaults are read back as valid SQL, so redefining
/// their columns keeps them.
async fn defaults_survive_column_redefinition(t: TestDb) {
    if t.engine == Engine::Sqlite {
        return;
    }
    let expression = if t.engine == Engine::MySql { "(CONCAT('a', 'b'))" } else { "CONCAT('a', 'b')" };
    t.exec(&format!(
        "CREATE TABLE defaults (id INT PRIMARY KEY, joined VARCHAR(10) DEFAULT {expression}, raw BINARY(2) DEFAULT 'ab') ENGINE = InnoDB"
    ))
    .await;
    let original = t.driver.table_design(&t.db, "defaults").await.unwrap();
    let mut changed = original.clone();
    changed.columns[1].name = "joined2".into();
    changed.columns[2].name = "raw2".into();
    let ddl = t.driver.plan_table(&t.db, Some(&original), &changed).await.unwrap();
    t.driver.apply_ddl(&t.db, &ddl).await.unwrap();
    t.exec("INSERT INTO defaults (id) VALUES (1)").await;
    assert_eq!(query(&t, "SELECT joined2, HEX(raw2) FROM defaults").await, vec![vec![text("ab"), text("6162")]]);
}
scenario!(defaults_survive_column_redefinition);
