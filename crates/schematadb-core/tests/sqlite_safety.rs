//! SQLite data-safety guarantees: query-tab sessions, scripts, imports and edits never lose or mangle data.
mod common;

use common::TestDb;
use schematadb_core::{
    CancellationToken, CellValue, ColumnValue, ErrorKind, ImportOptions, PageRequest, RowChange, ScriptRunReport, StatementOutcome,
    StreamItem, Value,
};
use std::time::Duration;

fn set(column: &str, value: &str) -> ColumnValue {
    ColumnValue { column: column.into(), value: CellValue::Text(value.into()) }
}

/// Rows of `sql` run on the app connection, or on query-tab session `session`.
async fn query(t: &TestDb, session: Option<&str>, sql: &str) -> Vec<Vec<CellValue>> {
    let result = match session {
        Some(session) => t.driver.execute_in_session(session, Some(&t.db), sql, 100, None).await,
        None => t.driver.execute_script(Some(&t.db), sql, 100, None).await,
    }
    .unwrap();
    assert!(result.error.is_none(), "{:?}", result.error);
    match result.outcomes.into_iter().last() {
        Some(StatementOutcome::Rows { rows, .. }) => rows,
        other => panic!("Unexpected result: {other:?}"),
    }
}

fn texts(rows: Vec<Vec<CellValue>>) -> Vec<String> {
    rows.into_iter()
        .map(|row| match &row[0] {
            CellValue::Text(s) => s.clone(),
            other => panic!("{other:?}"),
        })
        .collect()
}

async fn run_file(t: &TestDb, statements: &[&str]) -> schematadb_core::Result<ScriptRunReport> {
    let (tx, rx) = tokio::sync::mpsc::channel(statements.len().max(1));
    for statement in statements {
        tx.send(Ok((*statement).to_owned())).await.unwrap();
    }
    drop(tx);
    t.driver.execute_statements(Some(&t.db), rx, true, CancellationToken::new()).await
}

async fn rename_item(t: &TestDb, id: i64, name: &str) -> Result<u64, schematadb_core::ChangeError> {
    let identity = t.driver.table_info(&t.db, "item").await.unwrap().row_identity;
    let change = RowChange::Update { key: vec![CellValue::Int(id)], values: vec![set("name", name)], expected: vec![] };
    t.driver.apply_changes(&t.db, "item", &identity, &[change]).await
}

/// A query tab's open transaction lives on its own connection: app operations neither join it (and get committed or
/// rolled back with it) nor succeed silently while it holds the lock; closing the tab rolls it back.
#[tokio::test(flavor = "multi_thread")]
async fn query_tab_transactions_do_not_capture_app_operations() {
    let t = TestDb::sqlite().await;
    t.exec("CREATE TABLE item (id INTEGER PRIMARY KEY, name TEXT); INSERT INTO item VALUES (1, 'a'), (2, 'b');").await;
    query(&t, Some("Q"), "BEGIN; UPDATE item SET name = 'tab' WHERE id = 1; SELECT 1").await;
    assert_eq!(texts(query(&t, Some("Q"), "SELECT name FROM item ORDER BY id").await), ["tab", "b"]);
    assert_eq!(texts(query(&t, None, "SELECT name FROM item ORDER BY id").await), ["a", "b"], "uncommitted work stays in the tab");

    let error = rename_item(&t, 2, "grid").await.unwrap_err().error;
    assert!(error.message.contains("open transaction in a query tab"), "{error:?}");
    let error = t.driver.truncate_table(&t.db, "item").await.unwrap_err();
    assert!(error.message.contains("open transaction in a query tab"), "{error:?}");

    t.driver.close_editor_session("Q").await.unwrap();
    rename_item(&t, 2, "grid").await.unwrap();
    assert_eq!(texts(query(&t, None, "SELECT name FROM item ORDER BY id").await), ["a", "grid"], "closing the tab rolled back");
}

/// Sessions see the databases attached to the connection and their own temporary state; a session lost with a
/// replaced driver fails its next run once.
#[tokio::test(flavor = "multi_thread")]
async fn query_tab_sessions_keep_their_state_and_report_loss() {
    let t = TestDb::sqlite().await;
    let dir = tempfile::tempdir().unwrap();
    let other = dir.path().join("other.db").to_string_lossy().into_owned();
    query(&t, Some("Q"), "SELECT 1").await;
    t.driver.attach_database(&other, "other").await.unwrap();
    t.exec("CREATE TABLE other.remote (v TEXT); INSERT INTO other.remote VALUES ('far');").await;
    assert_eq!(texts(query(&t, Some("Q"), "SELECT v FROM other.remote").await), ["far"]);

    query(&t, Some("Q"), "CREATE TEMP TABLE scratch (v TEXT); INSERT INTO scratch VALUES ('s1'), ('s2'), ('s3'); SELECT 1").await;
    let more = t.driver.fetch_more_in_session("Q", Some(&t.db), "SELECT v FROM scratch ORDER BY v", 1, 10).await.unwrap();
    assert_eq!(more.rows, vec![vec![CellValue::Text("s2".into())], vec![CellValue::Text("s3".into())]]);
    assert!(t.driver.execute_in_session("R", Some(&t.db), "SELECT * FROM scratch", 10, None).await.unwrap().error.is_some());

    assert!(t.driver.live_editor_sessions().contains(&"Q".to_owned()));
    t.driver.adopt_lost_editor_sessions(vec!["Q".into()]);
    let error = t.driver.execute_in_session("Q", Some(&t.db), "SELECT 1", 10, None).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Connection);
    query(&t, Some("Q"), "SELECT 1").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cancels_a_query_tab_run() {
    let t = TestDb::sqlite().await;
    let driver = t.driver.clone();
    let slow = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c";
    let run = tokio::spawn(async move { driver.execute_in_session("Q", Some("main"), slow, 10, Some("exec-q")).await });
    tokio::time::sleep(Duration::from_millis(500)).await;
    t.driver.cancel("exec-q").await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), run).await.expect("cancel interrupts the session").unwrap().unwrap();
    assert_eq!(result.error.unwrap().error.kind, ErrorKind::Cancelled);
}

/// Run SQL File never ends a transaction it did not start, and reports the one it leaves open before rolling it back.
#[tokio::test(flavor = "multi_thread")]
async fn sql_files_respect_outside_transactions_and_report_their_own() {
    let t = TestDb::sqlite().await;
    t.exec("CREATE TABLE log (v INT); BEGIN; INSERT INTO log VALUES (1);").await;
    let error = run_file(&t, &["INSERT INTO log VALUES (2)"]).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Validation);
    t.exec("COMMIT").await;
    assert_eq!(t.driver.count_rows(&t.db, "log", &[]).await.unwrap(), 1, "the outside transaction was not rolled back");

    let report = run_file(&t, &["BEGIN", "INSERT INTO log VALUES (3)"]).await.unwrap();
    assert_eq!(report.executed, 2);
    assert!(report.open_transaction_rolled_back);
    assert_eq!(t.driver.count_rows(&t.db, "log", &[]).await.unwrap(), 1);

    let report = run_file(&t, &["BEGIN", "INSERT INTO log VALUES (4)", "COMMIT"]).await.unwrap();
    assert!(!report.open_transaction_rolled_back);
    assert_eq!(t.driver.count_rows(&t.db, "log", &[]).await.unwrap(), 2);
}

/// Only SQLite's own `sqlite_` tables are internal; `SQLiteLog` and `sqlitex` are user tables.
#[tokio::test(flavor = "multi_thread")]
async fn tables_named_like_sqlite_are_listed_and_dumped() {
    let t = TestDb::sqlite().await;
    t.exec("CREATE TABLE SQLiteLog (msg TEXT); CREATE TABLE sqlitex (v INT); CREATE TABLE counter (id INTEGER PRIMARY KEY AUTOINCREMENT);")
        .await;
    let objects: Vec<String> = t.driver.list_objects(&t.db).await.unwrap().into_iter().map(|o| o.name).collect();
    assert_eq!(objects, ["SQLiteLog", "counter", "sqlitex"]);
    let dumped: Vec<String> = t.driver.dump_script(&t.db).await.unwrap().tables.into_iter().map(|(name, _)| name).collect();
    assert_eq!(dumped, ["SQLiteLog", "sqlitex", "counter"]);
    let columns: Vec<String> = t.driver.list_columns(&t.db).await.unwrap().into_iter().map(|(table, _)| table).collect();
    assert!(columns.contains(&"SQLiteLog".to_owned()) && columns.contains(&"sqlitex".to_owned()), "{columns:?}");
}

/// A row whose conflict clause rolls back the whole transaction cannot be skipped: the rows before it are gone.
#[tokio::test(flavor = "multi_thread")]
async fn skip_errors_import_aborts_when_a_row_rolls_back_the_transaction() {
    let t = TestDb::sqlite().await;
    t.exec("CREATE TABLE u (id INTEGER UNIQUE ON CONFLICT ROLLBACK); INSERT INTO u VALUES (1);").await;
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    tx.send(Ok(vec![(1, vec![Value::Int(2)]), (2, vec![Value::Int(1)]), (3, vec![Value::Int(3)])])).await.unwrap();
    drop(tx);
    let options = ImportOptions { skip_errors: true, ..Default::default() };
    let error = t.driver.import_rows(&t.db, "u", &["id".into()], rx, options).await.unwrap_err();
    assert!(error.message.contains("no rows were imported"), "{error:?}");
    assert_eq!(texts(query(&t, None, "SELECT group_concat(id) FROM u").await), ["1"]);
}

/// Expression defaults come back parenthesized, so the designer's rebuild (and copies) can emit them again.
#[tokio::test(flavor = "multi_thread")]
async fn tables_with_expression_defaults_can_be_rebuilt() {
    let t = TestDb::sqlite().await;
    t.exec("CREATE TABLE ev (id INTEGER PRIMARY KEY, name TEXT, at TEXT DEFAULT (datetime('now')), n INT DEFAULT (1 + 2)); INSERT INTO ev (id, name) VALUES (1, 'a');").await;
    let original = t.driver.table_design(&t.db, "ev").await.unwrap();
    let mut design = original.clone();
    design.columns.iter_mut().find(|c| c.name == "name").unwrap().data_type = "VARCHAR(20)".into();
    let ddl = t.driver.plan_table(&t.db, Some(&original), &design).await.unwrap();
    assert!(ddl.iter().any(|s| s.contains("INSERT INTO")), "a type change rebuilds the table: {ddl:?}");
    t.driver.apply_ddl(&t.db, &ddl).await.unwrap_or_else(|e| panic!("{e}\n{}", ddl.join(";\n")));
    t.exec("INSERT INTO ev (id, name) VALUES (2, 'b')").await;
    assert_eq!(query(&t, None, "SELECT at IS NOT NULL, n FROM ev WHERE id = 2").await, vec![vec![CellValue::Int(1), CellValue::Int(3)]]);
}

/// Rebuilds leave foreign-key enforcement as the connection had it.
#[tokio::test(flavor = "multi_thread")]
async fn rebuilds_keep_the_foreign_key_setting() {
    let t = TestDb::sqlite().await;
    t.exec("CREATE TABLE doc (id INTEGER PRIMARY KEY, body TEXT); PRAGMA foreign_keys = OFF;").await;
    let original = t.driver.table_design(&t.db, "doc").await.unwrap();
    let mut design = original.clone();
    design.columns.iter_mut().find(|c| c.name == "body").unwrap().data_type = "VARCHAR(50)".into();
    let ddl = t.driver.plan_table(&t.db, Some(&original), &design).await.unwrap();
    t.driver.apply_ddl(&t.db, &ddl).await.unwrap();
    assert_eq!(query(&t, None, "PRAGMA foreign_keys").await, vec![vec![CellValue::Int(0)]]);
}

/// Grid edits arrive as text; columns without affinity keep canonical numbers numeric and other text as typed.
#[tokio::test(flavor = "multi_thread")]
async fn edits_keep_numbers_numeric_in_untyped_columns() {
    let t = TestDb::sqlite().await;
    t.exec("CREATE TABLE loose (a, b BLOB, c, d TEXT); INSERT INTO loose VALUES (42, 1, 'x', 'y');").await;
    let identity = t.driver.table_info(&t.db, "loose").await.unwrap().row_identity;
    let page = t.driver.fetch_page(&t.db, "loose", &PageRequest { offset: 0, limit: 10, sort: None, filters: vec![] }).await.unwrap();
    let change = RowChange::Update {
        key: vec![page.rows[0][0].clone()],
        values: vec![set("a", "43"), set("b", "1.5"), set("c", "007"), set("d", "5")],
        expected: vec![],
    };
    t.driver.apply_changes(&t.db, "loose", &identity, &[change]).await.unwrap();
    assert_eq!(
        texts(
            query(
                &t,
                None,
                "SELECT typeof(a) || ' ' || a || ', ' || typeof(b) || ', ' || typeof(c) || ' ' || c || ', ' || typeof(d) FROM loose"
            )
            .await
        ),
        ["integer 43, real, text 007, text"]
    );
}

/// TEXT that is not valid UTF-8 is read as its bytes, never as a lossy (and then written back) replacement.
#[tokio::test(flavor = "multi_thread")]
async fn invalid_utf8_text_is_read_as_bytes() {
    let t = TestDb::sqlite().await;
    t.exec("CREATE TABLE names (v TEXT); INSERT INTO names VALUES (CAST(X'4A6F73E9' AS TEXT));").await;
    let mut stream = t.driver.stream_query(Some(&t.db), "SELECT v FROM names", 10).await.unwrap();
    assert!(matches!(stream.recv().await, Some(Ok(StreamItem::Columns(_)))));
    let Some(Ok(StreamItem::Rows(rows))) = stream.recv().await else { panic!("expected rows") };
    assert_eq!(rows, vec![vec![Value::Bytes(vec![0x4A, 0x6F, 0x73, 0xE9])]]);
}

/// "Load more" runs the statement again, so only read-only statements may be continued.
#[tokio::test(flavor = "multi_thread")]
async fn load_more_refuses_statements_with_side_effects() {
    let t = TestDb::sqlite().await;
    t.exec("CREATE TABLE n (i INT); INSERT INTO n VALUES (1), (2), (3);").await;
    let error = t.driver.fetch_more(Some(&t.db), "DELETE FROM n RETURNING i", 1, 10).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Validation);
    let error = t.driver.fetch_more_in_session("Q", Some(&t.db), "DELETE FROM n RETURNING i", 1, 10).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Validation);
    assert_eq!(t.driver.count_rows(&t.db, "n", &[]).await.unwrap(), 3);
}

/// A row whose TEXT is not valid UTF-8 loads as bytes; it must still be editable by rowid without matching another row.
#[tokio::test(flavor = "multi_thread")]
async fn rows_with_invalid_utf8_text_stay_editable() {
    let t = TestDb::sqlite().await;
    t.exec("CREATE TABLE names (v TEXT, n INT); INSERT INTO names VALUES (CAST(X'4A6F73E9' AS TEXT), 1), ('ok', 2);").await;
    let identity = t.driver.table_info(&t.db, "names").await.unwrap().row_identity;
    let req = PageRequest { offset: 0, limit: 10, sort: None, filters: vec![] };
    let page = t.driver.fetch_page(&t.db, "names", &req).await.unwrap();
    let row = &page.rows[0];
    let expected =
        vec![ColumnValue { column: "v".into(), value: row[1].clone() }, ColumnValue { column: "n".into(), value: row[2].clone() }];
    let change = RowChange::Update {
        key: vec![row[0].clone()],
        values: vec![ColumnValue { column: "n".into(), value: CellValue::Int(10) }],
        expected,
    };
    t.driver.apply_changes(&t.db, "names", &identity, &[change]).await.unwrap();
    assert_eq!(t.driver.count_rows(&t.db, "names", &[]).await.unwrap(), 2);
    let page = t.driver.fetch_page(&t.db, "names", &req).await.unwrap();
    assert_eq!(page.rows.iter().map(|r| r[2].clone()).collect::<Vec<_>>(), vec![CellValue::Int(10), CellValue::Int(2)]);
}
