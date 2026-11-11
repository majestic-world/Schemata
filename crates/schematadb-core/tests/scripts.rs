mod common;

use common::TestDb;
use schematadb_core::{CancellationToken, CellValue, Engine, ErrorKind, QueryHistory, Splitter, StatementOutcome, split_statements};
use std::time::{Duration, Instant};

#[test]
fn splits_on_semicolons_outside_quotes_and_comments() {
    let sql = "SELECT 'a;b', \"c;d\", `e;f` FROM t; -- trailing; comment\nUPDATE t SET x = 1 /* inline ; */ WHERE y = 'it''s'; ;\n# hash; comment only\n";
    assert_eq!(
        split_statements(sql, Engine::MySql),
        vec!["SELECT 'a;b', \"c;d\", `e;f` FROM t", "-- trailing; comment\nUPDATE t SET x = 1 /* inline ; */ WHERE y = 'it''s'"]
    );
}

#[test]
fn honors_mysql_delimiter_blocks() {
    let sql = "DELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT 1; SELECT 2; END$$\nDELIMITER ;\nCALL p();";
    assert_eq!(split_statements(sql, Engine::MariaDb), vec!["CREATE PROCEDURE p() BEGIN SELECT 1; SELECT 2; END", "CALL p()"]);
}

#[test]
fn mysql_backslash_escapes_do_not_end_strings() {
    assert_eq!(split_statements(r"SELECT 'a\';b'; SELECT 2", Engine::MySql), vec![r"SELECT 'a\';b'", "SELECT 2"]);
}

#[test]
fn mysql_splitter_accepts_multibyte_text_at_the_directive_boundary() {
    for engine in [Engine::MySql, Engine::MariaDb] {
        assert_eq!(split_statements("SELECT 'aé'; SELECT '🦀';", engine), vec!["SELECT 'aé'", "SELECT '🦀'"]);
    }
}

#[test]
fn keeps_sqlite_trigger_bodies_whole() {
    let sql = "CREATE TRIGGER tr AFTER INSERT ON t BEGIN INSERT INTO log VALUES (CASE WHEN 1 THEN 'x' END); DELETE FROM q; END; SELECT 1";
    assert_eq!(
        split_statements(sql, Engine::Sqlite),
        vec!["CREATE TRIGGER tr AFTER INSERT ON t BEGIN INSERT INTO log VALUES (CASE WHEN 1 THEN 'x' END); DELETE FROM q; END", "SELECT 1"]
    );
}

async fn runs_statements_and_stops_at_first_error(t: TestDb) {
    let script = "CREATE TABLE s (id INT PRIMARY KEY, v VARCHAR(5));
                  INSERT INTO s VALUES (1, 'a'), (2, 'b'), (3, 'c');
                  SELECT id, v FROM s ORDER BY id;
                  SELECT v FROM s WHERE id > 100;
                  INSERT INTO s VALUES (1, 'dup');
                  DELETE FROM s;";
    let r = t.driver.execute_script(Some(&t.db), script, 2, None).await.unwrap();
    assert_eq!(r.outcomes.len(), 4);
    match &r.outcomes[1] {
        StatementOutcome::Affected { rows_affected, .. } => assert_eq!(*rows_affected, 3),
        other => panic!("{other:?}"),
    }
    match &r.outcomes[2] {
        StatementOutcome::Rows { columns, rows, has_more, .. } => {
            assert_eq!(columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["id", "v"]);
            assert_eq!(rows.len(), 2);
            assert!(*has_more);
        }
        other => panic!("{other:?}"),
    }
    match &r.outcomes[3] {
        StatementOutcome::Rows { columns, rows, .. } => {
            assert!(rows.is_empty());
            assert_eq!(columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["v"], "empty results keep their columns");
        }
        other => panic!("{other:?}"),
    }
    let err = r.error.expect("duplicate key must fail");
    assert_eq!(err.index, 4);
    assert!(err.statement.contains("dup"));
    assert_eq!(err.error.kind, ErrorKind::Sql);
    // The DELETE after the failure did not run.
    let after = t.driver.execute_script(Some(&t.db), "SELECT COUNT(*) FROM s", 10, None).await.unwrap();
    let StatementOutcome::Rows { rows, .. } = &after.outcomes[0] else { panic!() };
    assert_eq!(rows[0][0], CellValue::Int(3));
}
scenario!(runs_statements_and_stops_at_first_error);

async fn loads_more_rows_of_a_select(t: TestDb) {
    t.exec("CREATE TABLE n (i INT); INSERT INTO n VALUES (1), (2), (3), (4), (5);").await;
    let more = t.driver.fetch_more(Some(&t.db), "SELECT i FROM n ORDER BY i", 2, 2).await.unwrap();
    assert_eq!(more.rows, vec![vec![CellValue::Int(3)], vec![CellValue::Int(4)]]);
}
scenario!(loads_more_rows_of_a_select);

async fn cancels_a_running_query(t: TestDb) {
    let slow = match t.engine {
        Engine::Sqlite => "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c",
        _ => "SELECT SLEEP(30)",
    };
    let driver = t.driver.clone();
    let db = t.db.clone();
    let started = Instant::now();
    let run = tokio::spawn(async move { driver.execute_script(Some(&db), slow, 10, Some("exec-1")).await });
    tokio::time::sleep(Duration::from_millis(700)).await;
    t.driver.cancel("exec-1").await.unwrap();
    let result = run.await.unwrap().unwrap();
    assert!(started.elapsed() < Duration::from_secs(10), "cancel took {:?}", started.elapsed());
    match (t.engine, result.error) {
        (_, Some(e)) => assert_eq!(e.error.kind, ErrorKind::Cancelled),
        // MySQL's SLEEP returns 1 instead of failing when interrupted.
        (Engine::MySql | Engine::MariaDb, None) => match &result.outcomes[0] {
            StatementOutcome::Rows { rows, .. } => assert_eq!(rows[0][0], CellValue::Int(1)),
            other => panic!("{other:?}"),
        },
        (Engine::Sqlite, None) => panic!("SQLite query was not interrupted"),
    }
}
scenario!(cancels_a_running_query);

async fn lists_columns_for_autocomplete(t: TestDb) {
    t.exec("CREATE TABLE a (x INT, y INT); CREATE TABLE b (z INT);").await;
    let mut cols = t.driver.list_columns(&t.db).await.unwrap();
    cols.sort();
    assert_eq!(cols, vec![("a".to_string(), vec!["x".to_string(), "y".to_string()]), ("b".to_string(), vec!["z".to_string()])]);
}
scenario!(lists_columns_for_autocomplete);

#[test]
fn history_persists_per_connection_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.json");
    {
        let h = QueryHistory::open(&path).unwrap();
        h.record("c1", "SELECT 1", true, 3).unwrap();
        h.record("c2", "SELECT 2", true, 4).unwrap();
        h.record("c1", "SELECT broken", false, 1).unwrap();
    }
    let h = QueryHistory::open(&path).unwrap();
    let c1: Vec<_> = h.list("c1").into_iter().map(|e| (e.sql, e.ok)).collect();
    assert_eq!(c1, vec![("SELECT broken".to_string(), false), ("SELECT 1".to_string(), true)]);
    assert_eq!(h.list("c2").len(), 1);
}

#[test]
fn incremental_splitting_matches_whole_script() {
    let sql = "INSERT INTO t VALUES ('multi\nline; text');\n/* block\n comment; */ SELECT 1;\nDELIMITER //\nCREATE PROCEDURE p() BEGIN\n  SELECT 2;\nEND//\nDELIMITER ;\nSELECT 3";
    let mut splitter = Splitter::new(Engine::MySql);
    let mut streamed = Vec::new();
    for line in sql.split_inclusive('\n') {
        streamed.extend(splitter.feed(line));
    }
    streamed.extend(splitter.finish());
    assert_eq!(streamed, split_statements(sql, Engine::MySql));
    assert_eq!(streamed.len(), 4);
    assert_eq!(streamed[0], "INSERT INTO t VALUES ('multi\nline; text')");
}

#[tokio::test]
async fn sqlite_script_cancellation_rolls_back_its_transaction() {
    let t = TestDb::sqlite().await;
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let cancel = CancellationToken::new();
    for st in ["PRAGMA foreign_keys = OFF", "BEGIN TRANSACTION", "CREATE TABLE half (a INT)"] {
        tx.send(Ok(st.to_owned())).await.unwrap();
    }
    let driver = t.driver.clone();
    let token = cancel.clone();
    let run = tokio::spawn(async move { driver.execute_statements(Some("main"), rx, true, token).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    cancel.cancel();
    let _ = tx.send(Ok("CREATE TABLE never (a INT)".into())).await;
    drop(tx);
    let err = run.await.unwrap().unwrap_err();
    assert_eq!(err.kind, ErrorKind::Cancelled);
    // The half-applied transaction was rolled back and the session is usable, with FK enforcement back on.
    assert!(t.driver.list_objects("main").await.unwrap().is_empty());
    let r = t.driver.execute_script(Some("main"), "BEGIN; PRAGMA foreign_keys; COMMIT;", 10, None).await.unwrap();
    assert!(r.error.is_none(), "{:?}", r.error);
    let StatementOutcome::Rows { rows, .. } = &r.outcomes[1] else { panic!() };
    assert_eq!(rows[0][0], CellValue::Int(1));
}

#[tokio::test]
async fn sqlite_scripts_can_target_an_attached_database() {
    let t = TestDb::sqlite().await;
    let dir = tempfile::tempdir().unwrap();
    let other = dir.path().join("other.db").to_string_lossy().into_owned();
    t.driver.attach_database(&other, "other").await.unwrap();
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    tx.send(Ok("CREATE TABLE only_here (a INT)".to_owned())).await.unwrap();
    drop(tx);
    let report = t.driver.execute_statements(Some("other"), rx, true, CancellationToken::new()).await.unwrap();
    assert_eq!(report.executed, 1);
    assert!(t.driver.list_objects("main").await.unwrap().is_empty());
    assert_eq!(t.driver.list_objects("other").await.unwrap()[0].name, "only_here");
}

#[tokio::test]
async fn cancellation_is_reported_when_the_script_stops_early() {
    let t = TestDb::sqlite().await;
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    let cancel = CancellationToken::new();
    tx.send(Ok("CREATE TABLE first (a INT)".to_owned())).await.unwrap();
    cancel.cancel();
    drop(tx);
    // The reader stopped without sending more; the consumer must still report cancellation.
    let err = t.driver.execute_statements(Some("main"), rx, true, cancel).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::Cancelled);
}

async fn statement_reader_errors_roll_back_and_restore_foreign_keys(t: TestDb) {
    t.exec("CREATE TABLE reader_error (id INT PRIMARY KEY)").await;
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let disable_fk = if t.engine == Engine::Sqlite { "PRAGMA foreign_keys = OFF" } else { "SET FOREIGN_KEY_CHECKS = 0" };
    for statement in [disable_fk, "BEGIN", "INSERT INTO reader_error VALUES (1)"] {
        tx.send(Ok(statement.to_owned())).await.unwrap();
    }
    tx.send(Err(schematadb_core::AppError::io("The SQL file could not be read"))).await.unwrap();
    drop(tx);
    let error = t.driver.execute_statements(Some(&t.db), rx, false, CancellationToken::new()).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Io);
    assert_eq!(t.driver.count_rows(&t.db, "reader_error", &[]).await.unwrap(), 0);
    let inspect_fk = if t.engine == Engine::Sqlite { "PRAGMA foreign_keys" } else { "SELECT @@FOREIGN_KEY_CHECKS" };
    let report = t.driver.execute_script(Some(&t.db), inspect_fk, 1, None).await.unwrap();
    assert!(report.error.is_none(), "{:?}", report.error);
    let StatementOutcome::Rows { rows, .. } = &report.outcomes[0] else { panic!() };
    assert_eq!(rows[0][0], CellValue::Int(1));
}
scenario!(statement_reader_errors_roll_back_and_restore_foreign_keys);

async fn statement_consumer_cancels_an_active_query(t: TestDb) {
    t.exec("CREATE TABLE cancelled_file (id INT PRIMARY KEY)").await;
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    for statement in ["BEGIN", "INSERT INTO cancelled_file VALUES (1)"] {
        tx.send(Ok(statement.to_owned())).await.unwrap();
    }
    let query = if t.engine == Engine::Sqlite {
        "WITH RECURSIVE seq(n) AS (VALUES(1) UNION ALL SELECT n + 1 FROM seq WHERE n < 1000000000) SELECT sum(n) FROM seq"
    } else {
        "SELECT SLEEP(10)"
    };
    tx.send(Ok(query.to_owned())).await.unwrap();
    drop(tx);
    let driver = t.driver.clone();
    let db = t.db.clone();
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let run = tokio::spawn(async move { driver.execute_statements(Some(&db), rx, true, token).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    cancel.cancel();
    let error = tokio::time::timeout(Duration::from_secs(3), run)
        .await
        .expect("cancellation must interrupt the active statement")
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Cancelled);
    assert_eq!(t.driver.count_rows(&t.db, "cancelled_file", &[]).await.unwrap(), 0);
    t.exec("BEGIN; INSERT INTO cancelled_file VALUES (2); ROLLBACK;").await;
}
scenario!(statement_consumer_cancels_an_active_query);
