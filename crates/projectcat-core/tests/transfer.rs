mod common;

use common::TestDb;
use projectcat_core::{
    CancellationToken, CellValue, ColumnMapping, CsvImport, CsvOptions, CsvReadOptions, Engine, ErrorKind, ExportFormat,
    ExportSource, PageRequest, ProgressFn, SortSpec, SqlExportOptions, TextEncoding, export, import_csv, preview_csv, run_sql_file,
};
use std::sync::{Arc, Mutex};

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
    let report = export(t.driver.as_ref(), &t.db, &ExportSource::Table { table: "src".into() }, &ExportFormat::Csv(csv_opts()), &file, progress, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(report.rows, 3);
    assert_eq!(*seen.lock().unwrap().last().unwrap(), 3);

    let preview = preview_csv(&file, &read_opts(), 10).unwrap();
    assert_eq!(preview.headers, vec!["id", "name", "note", "amount"]);
    assert_eq!(preview.rows.len(), 3);

    let spec = CsvImport { options: read_opts(), mapping: mapping(&["id", "name", "note", "amount"]), empty_as_null: false, skip_errors: false };
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
    export(t.driver.as_ref(), &t.db, &ExportSource::Table { table: "src".into() }, &format, &file, no_progress(), CancellationToken::new()).await.unwrap();
    // Load it under the same name after dropping the original.
    let original = rows(&t, "src").await;
    t.exec("DROP TABLE src").await;
    let report = run_sql_file(t.driver.as_ref(), &t.db, &file, true, no_progress(), CancellationToken::new()).await.unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(rows(&t, "src").await, original);
}
scenario!(sql_export_recreates_table_elsewhere);

async fn cancelled_export_reports_cancellation(t: TestDb) {
    seed(&t).await;
    let dir = tempfile::tempdir().unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let err = export(t.driver.as_ref(), &t.db, &ExportSource::Table { table: "src".into() }, &ExportFormat::Json, &dir.path().join("x.json"), no_progress(), cancel)
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
    export(t.driver.as_ref(), &t.db, &ExportSource::Query { sql: "SELECT id, name, note FROM src WHERE id IN (1, 3) ORDER BY id".into() }, &ExportFormat::Csv(opts), &file, no_progress(), CancellationToken::new())
        .await
        .unwrap();
    let bytes = std::fs::read(&file).unwrap();
    assert!(bytes.contains(&0xC7), "Ç must be a single windows-1252 byte");
    let (text, _, _) = encoding_rs::WINDOWS_1252.decode(&bytes);
    assert_eq!(text.replace("\r\n", "\n"), "id;name;note\n1;Ana;NULL\n3;Çédille;\n");
}
