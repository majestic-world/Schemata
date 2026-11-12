mod common;

use common::TestDb;
use schematadb_core::{CellValue, StatementOutcome};

async fn run(t: &TestDb, session: &str, sql: &str) -> Vec<StatementOutcome> {
    let result = t.driver.execute_in_session(session, Some(&t.db), sql, 100, None).await.unwrap();
    assert!(result.error.is_none(), "{:?}", result.error);
    result.outcomes
}

async fn count(t: &TestDb, session: &str, sql: &str) -> CellValue {
    match run(t, session, sql).await.pop() {
        Some(StatementOutcome::Rows { rows, .. }) => rows[0][0].clone(),
        other => panic!("Unexpected result: {other:?}"),
    }
}

/// Each editor session keeps its own transaction and session state across runs.
async fn transactions_span_runs_of_one_session(t: TestDb) {
    run(&t, "S", "CREATE TABLE e (id INT PRIMARY KEY) ENGINE = InnoDB").await;
    run(&t, "S", "BEGIN; INSERT INTO e VALUES (1); SET @marker = 7; CREATE TEMPORARY TABLE scratch (v INT); INSERT INTO scratch VALUES (1), (2), (3)").await;

    assert_eq!(count(&t, "S", "SELECT COUNT(*) FROM e").await, CellValue::Int(1));
    assert_eq!(count(&t, "T", "SELECT COUNT(*) FROM e").await, CellValue::Int(0));
    assert_eq!(count(&t, "S", "SELECT @marker").await, CellValue::Int(7));
    // "Load more" continues on the session that owns the temporary table.
    let more = t.driver.fetch_more_in_session("S", Some(&t.db), "SELECT v FROM scratch ORDER BY v", 1, 10).await.unwrap();
    assert_eq!(more.rows, vec![vec![CellValue::Int(2)], vec![CellValue::Int(3)]]);

    run(&t, "S", "COMMIT").await;
    assert_eq!(count(&t, "T", "SELECT COUNT(*) FROM e").await, CellValue::Int(1));

    // Closing a session with an open transaction rolls it back.
    run(&t, "S", "BEGIN; INSERT INTO e VALUES (2)").await;
    t.driver.close_editor_session("S").await.unwrap();
    // A locking read waits for the closed session's rollback to release the row lock.
    run(&t, "T", "SET SESSION innodb_lock_wait_timeout = 10").await;
    assert_eq!(count(&t, "T", "SELECT COUNT(*) FROM e FOR UPDATE").await, CellValue::Int(1));
    // The key opens a fresh session afterwards.
    assert_eq!(count(&t, "S", "SELECT @marker IS NULL").await, CellValue::Int(1));
}

/// A session the server closed fails its next run with a connection error instead of silently reconnecting (which
/// would hide a rolled-back transaction); the run after that opens a fresh session.
async fn server_closed_session_is_reported_once(t: TestDb) {
    run(&t, "S", "SET @marker = 7").await;
    let CellValue::Int(thread) = count(&t, "S", "SELECT CONNECTION_ID()").await else { panic!("connection id") };
    run(&t, "T", &format!("KILL {thread}")).await;
    let error = t.driver.execute_in_session("S", Some(&t.db), "SELECT 1", 100, None).await.unwrap_err();
    assert_eq!(error.kind, schematadb_core::ErrorKind::Connection, "{error:?}");
    assert_eq!(count(&t, "S", "SELECT @marker IS NULL").await, CellValue::Int(1));
}

#[tokio::test(flavor = "multi_thread")]
async fn mysql_transactions_span_runs_of_one_session() {
    transactions_span_runs_of_one_session(TestDb::mysql().await).await
}

#[tokio::test(flavor = "multi_thread")]
async fn mariadb_transactions_span_runs_of_one_session() {
    transactions_span_runs_of_one_session(TestDb::mariadb().await).await
}

#[tokio::test(flavor = "multi_thread")]
async fn mysql_server_closed_session_is_reported_once() {
    server_closed_session_is_reported_once(TestDb::mysql().await).await
}

#[tokio::test(flavor = "multi_thread")]
async fn mariadb_server_closed_session_is_reported_once() {
    server_closed_session_is_reported_once(TestDb::mariadb().await).await
}
