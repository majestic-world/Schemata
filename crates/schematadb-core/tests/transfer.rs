mod common;

use common::TestDb;
use schematadb_core::{
    CancellationToken, CellValue, ColumnMapping, CsvImport, CsvOptions, CsvReadOptions, Engine, ErrorKind, ExportFormat, ExportSource,
    PageRequest, ProgressFn, SortSpec, SqlExportOptions, TextEncoding, export, import_csv, preview_csv, run_sql_file,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn no_progress() -> ProgressFn {
    Arc::new(|_| {})
}

async fn rows(t: &TestDb, table: &str) -> Vec<Vec<CellValue>> {
    let req = PageRequest { offset: 0, limit: 1000, sort: Some(SortSpec { column: "id".into(), desc: false }), filters: vec![] };
    t.driver.fetch_page(&t.db, table, &req).await.unwrap().rows
}

async fn seed(t: &TestDb) {
    let ddl = |name: &str| format!("CREATE TABLE {name} (id INT PRIMARY KEY, name VARCHAR(40), note VARCHAR(40), amount DECIMAL(12,2))");
    t.exec(&format!(
        "{}; {}; INSERT INTO src VALUES (1, 'Ana', NULL, 10.50), (2, 'Bob \"B\", Jr.', 'multi\nline', 0.00), (3, 'Çédille', '', -3.25);",
        ddl("src"),
        ddl("dst")
    ))
    .await;
}

fn csv_opts() -> CsvOptions {
    CsvOptions { delimiter: ',', quote: '"', header: true, encoding: TextEncoding::Utf8, null_text: String::new() }
}

fn read_opts() -> CsvReadOptions {
    CsvReadOptions { delimiter: ',', quote: '"', has_header: true, encoding: TextEncoding::Utf8 }
}

fn mapping(names: &[&str]) -> Vec<ColumnMapping> {
    names.iter().enumerate().map(|(i, n)| ColumnMapping { source: i, target: n.to_string() }).collect()
}

async fn csv_round_trip_preserves_values(t: TestDb) {
    seed(&t).await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("src.csv");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let progress: ProgressFn = {
        let seen = seen.clone();
        Arc::new(move |p| seen.lock().unwrap().push(p.done))
    };
    let report = export(
        t.driver.as_ref(),
        &t.db,
        &ExportSource::Table { table: "src".into() },
        &ExportFormat::Csv(csv_opts()),
        &file,
        progress,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(report.rows, 3);
    assert_eq!(*seen.lock().unwrap().last().unwrap(), 3);

    let preview = preview_csv(&file, &read_opts(), 10).unwrap();
    assert_eq!(preview.headers, vec!["id", "name", "note", "amount"]);
    assert_eq!(preview.rows.len(), 3);

    let spec =
        CsvImport { options: read_opts(), mapping: mapping(&["id", "name", "note", "amount"]), empty_as_null: false, skip_errors: false };
    let imported = import_csv(t.driver.as_ref(), &t.db, "dst", &file, &spec, no_progress(), CancellationToken::new()).await.unwrap();
    assert_eq!((imported.inserted, imported.failed.len()), (3, 0));
    let (a, b) = (rows(&t, "src").await, rows(&t, "dst").await);
    // CSV has no NULL: the NULL note comes back as the empty string; everything else is identical.
    assert_eq!(b[0][2], CellValue::Text(String::new()));
    assert_eq!(a[1..], b[1..]);
    assert_eq!(a[0][..2], b[0][..2]);
}
scenario!(csv_round_trip_preserves_values);

async fn csv_import_maps_columns_and_handles_bad_rows(t: TestDb) {
    seed(&t).await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("in.csv");
    std::fs::write(&file, "label;ignored;key\nfirst;x;10\nsecond;y;not-a-number\nthird;z;12\n").unwrap();
    let options = CsvReadOptions { delimiter: ';', ..read_opts() };
    let map = vec![ColumnMapping { source: 2, target: "id".into() }, ColumnMapping { source: 0, target: "name".into() }];

    // Abort mode: the bad row rolls everything back and is reported with its line number.
    let strict = CsvImport { options: options.clone(), mapping: map.clone(), empty_as_null: true, skip_errors: false };
    let report = import_csv(t.driver.as_ref(), &t.db, "dst", &file, &strict, no_progress(), CancellationToken::new()).await.unwrap();
    // SQLite accepts 'not-a-number' in an INT column, so only the server engines fail here.
    if t.engine == Engine::Sqlite {
        assert_eq!(report.inserted, 3);
        return;
    }
    assert_eq!(report.inserted, 0);
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.failed[0].line, 3);
    assert!(rows(&t, "dst").await.is_empty());

    // Skip mode: good rows are kept.
    let lenient = CsvImport { skip_errors: true, ..strict };
    let report = import_csv(t.driver.as_ref(), &t.db, "dst", &file, &lenient, no_progress(), CancellationToken::new()).await.unwrap();
    assert_eq!((report.inserted, report.failed.len()), (2, 1));
    let got: Vec<_> = rows(&t, "dst").await.into_iter().map(|r| (r[0].clone(), r[1].clone())).collect();
    assert_eq!(got, vec![(CellValue::Int(10), CellValue::Text("first".into())), (CellValue::Int(12), CellValue::Text("third".into()))]);
}
scenario!(csv_import_maps_columns_and_handles_bad_rows);

async fn json_export_keeps_types(t: TestDb) {
    seed(&t).await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("q.json");
    let source = ExportSource::Query { sql: "SELECT id, name, note FROM src ORDER BY id".into() };
    export(t.driver.as_ref(), &t.db, &source, &ExportFormat::Json, &file, no_progress(), CancellationToken::new()).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(parsed[0], serde_json::json!({"id": 1, "name": "Ana", "note": null}));
    assert_eq!(parsed[1]["name"], "Bob \"B\", Jr.");
    assert_eq!(parsed.as_array().unwrap().len(), 3);
}
scenario!(json_export_keeps_types);

async fn sql_export_recreates_table_elsewhere(t: TestDb) {
    seed(&t).await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("src.sql");
    let format = ExportFormat::Sql(SqlExportOptions { include_create: true, table_name: None });
    export(t.driver.as_ref(), &t.db, &ExportSource::Table { table: "src".into() }, &format, &file, no_progress(), CancellationToken::new())
        .await
        .unwrap();
    // Load it under the same name after dropping the original.
    let original = rows(&t, "src").await;
    t.exec("DROP TABLE src").await;
    let report = run_sql_file(t.driver.as_ref(), &t.db, &file, true, no_progress(), CancellationToken::new()).await.unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(rows(&t, "src").await, original);
}
scenario!(sql_export_recreates_table_elsewhere);

async fn sql_export_renames_only_the_created_table(t: TestDb) {
    // The source name is a keyword that also occurs earlier in the CREATE statement and as a column name.
    let quote = if t.driver.engine() == Engine::Sqlite { '"' } else { '`' };
    let ident = |name: &str| format!("{quote}{name}{quote}");
    t.exec(&format!(
        "CREATE TABLE {table} (id INT PRIMARY KEY, {table} VARCHAR(20)); INSERT INTO {table} VALUES (1, 'kept'), (2, NULL);",
        table = ident("table")
    ))
    .await;
    let original = rows(&t, "table").await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("renamed.sql");
    let format = ExportFormat::Sql(SqlExportOptions { include_create: true, table_name: Some("table copy".into()) });
    export(t.driver.as_ref(), &t.db, &ExportSource::Table { table: "table".into() }, &format, &file, no_progress(), CancellationToken::new())
        .await
        .unwrap();
    let report = run_sql_file(t.driver.as_ref(), &t.db, &file, true, no_progress(), CancellationToken::new()).await.unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(rows(&t, "table copy").await, original);
    assert_eq!(rows(&t, "table").await, original, "the source table must be untouched");
}
scenario!(sql_export_renames_only_the_created_table);

async fn cancelled_export_reports_cancellation(t: TestDb) {
    seed(&t).await;
    let dir = tempfile::tempdir().unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let err = export(
        t.driver.as_ref(),
        &t.db,
        &ExportSource::Table { table: "src".into() },
        &ExportFormat::Json,
        &dir.path().join("x.json"),
        no_progress(),
        cancel,
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Cancelled);
}
scenario!(cancelled_export_reports_cancellation);

#[tokio::test]
async fn csv_export_honors_delimiter_and_windows_1252() {
    let t = TestDb::sqlite().await;
    seed(&t).await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("w.csv");
    let opts = CsvOptions { delimiter: ';', encoding: TextEncoding::Windows1252, null_text: "NULL".into(), ..csv_opts() };
    export(
        t.driver.as_ref(),
        &t.db,
        &ExportSource::Query { sql: "SELECT id, name, note FROM src WHERE id IN (1, 3) ORDER BY id".into() },
        &ExportFormat::Csv(opts),
        &file,
        no_progress(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    let bytes = std::fs::read(&file).unwrap();
    assert!(bytes.contains(&0xC7), "Ç must be a single windows-1252 byte");
    let (text, _, _) = encoding_rs::WINDOWS_1252.decode(&bytes);
    assert_eq!(text.replace("\r\n", "\n"), "id;name;note\n1;Ana;NULL\n3;Çédille;\n");
}

#[tokio::test]
async fn failed_export_keeps_an_existing_file() {
    let t = TestDb::sqlite().await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("keep.csv");
    std::fs::write(&file, "precious").unwrap();
    let source = ExportSource::Query { sql: "SELECT * FROM missing_table".into() };
    export(t.driver.as_ref(), &t.db, &source, &ExportFormat::Csv(csv_opts()), &file, no_progress(), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "precious");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no partial files are left behind");
}

async fn invalid_utf8_sql_file_rolls_back_its_open_transaction(t: TestDb) {
    t.exec("CREATE TABLE decoded (id INT PRIMARY KEY, note TEXT)").await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("invalid.sql");
    std::fs::write(
        &file,
        b"\xEF\xBB\xBFBEGIN;\nINSERT INTO decoded VALUES (1, 'valid');\nINSERT INTO decoded VALUES (2, '\xFF');\nCOMMIT;\n",
    )
    .unwrap();
    for stop_on_error in [true, false] {
        let error =
            run_sql_file(t.driver.as_ref(), &t.db, &file, stop_on_error, no_progress(), CancellationToken::new()).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::Validation);
        assert_eq!(t.driver.count_rows(&t.db, "decoded", &[]).await.unwrap(), 0);
        t.exec("BEGIN; INSERT INTO decoded VALUES (3, 'session usable'); ROLLBACK;").await;
    }
}
scenario!(invalid_utf8_sql_file_rolls_back_its_open_transaction);

#[tokio::test]
async fn sqlite_sql_export_with_structure_streams_more_than_the_channel_capacity() {
    let t = TestDb::sqlite().await;
    t.exec(
        "CREATE TABLE large_export (id INTEGER PRIMARY KEY, note TEXT);
         WITH RECURSIVE seq(n) AS (VALUES(1) UNION ALL SELECT n + 1 FROM seq WHERE n < 3000)
         INSERT INTO large_export SELECT n, 'row-' || n FROM seq;",
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("large.sql");
    let source = ExportSource::Table { table: "large_export".into() };
    let format = ExportFormat::Sql(SqlExportOptions { include_create: true, table_name: None });
    let report = tokio::time::timeout(
        Duration::from_secs(15),
        export(t.driver.as_ref(), &t.db, &source, &format, &file, no_progress(), CancellationToken::new()),
    )
    .await
    .expect("SQL export must not wait for metadata while its stream owns the connection")
    .unwrap();
    assert_eq!(report.rows, 3000);
    t.exec("DROP TABLE large_export").await;
    let restored = run_sql_file(t.driver.as_ref(), &t.db, &file, true, no_progress(), CancellationToken::new()).await.unwrap();
    assert!(restored.failed.is_empty(), "{:?}", restored.failed);
    assert_eq!(t.driver.count_rows(&t.db, "large_export", &[]).await.unwrap(), 3000);
    let request = PageRequest { offset: 2999, limit: 1, sort: Some(SortSpec { column: "id".into(), desc: false }), filters: vec![] };
    let page = t.driver.fetch_page(&t.db, "large_export", &request).await.unwrap();
    assert_eq!(
        page.rows[0].iter().zip(&page.columns).filter(|(_, c)| !c.hidden).map(|(v, _)| v.clone()).collect::<Vec<_>>(),
        vec![CellValue::Int(3000), CellValue::Text("row-3000".into())]
    );
}

async fn sql_export_recomputes_generated_columns(t: TestDb) {
    t.exec(
        "CREATE TABLE generated_export (
             id INT PRIMARY KEY, base INT,
             doubled INT GENERATED ALWAYS AS (base * 2) STORED,
             tripled INT GENERATED ALWAYS AS (base * 3) VIRTUAL);
         INSERT INTO generated_export (id, base) VALUES (1, 7), (2, NULL);",
    )
    .await;
    let original = rows(&t, "generated_export").await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("generated.sql");
    export(
        t.driver.as_ref(),
        &t.db,
        &ExportSource::Table { table: "generated_export".into() },
        &ExportFormat::Sql(SqlExportOptions { include_create: true, table_name: None }),
        &file,
        no_progress(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    t.exec("DROP TABLE generated_export").await;
    let restored = run_sql_file(t.driver.as_ref(), &t.db, &file, true, no_progress(), CancellationToken::new()).await.unwrap();
    assert!(restored.failed.is_empty(), "{:?}", restored.failed);
    assert_eq!(rows(&t, "generated_export").await, original);
}
scenario!(sql_export_recomputes_generated_columns);
