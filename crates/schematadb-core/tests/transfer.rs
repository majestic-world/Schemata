mod common;

use common::TestDb;
use schematadb_core::{
    CancellationToken, CellValue, ColumnMapping, CsvImport, CsvOptions, CsvReadOptions, Engine, ErrorKind, ExportFormat, ExportSource,
    PageRequest, ProgressFn, SortSpec, SqlExportOptions, TextEncoding, export, import_csv, preview_csv, run_sql_file, scan_sql_file,
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
    CsvOptions { delimiter: ',', quote: '"', header: true, encoding: TextEncoding::Utf8, null_text: "\\N".into() }
}

fn read_opts() -> CsvReadOptions {
    CsvReadOptions { delimiter: ',', quote: '"', has_header: true, encoding: TextEncoding::Utf8 }
}

/// Reads what [`csv_opts`] writes: `\N` is NULL, an empty field the empty string.
fn import_spec(options: CsvReadOptions, mapping: Vec<ColumnMapping>) -> CsvImport {
    CsvImport { options, mapping, empty_as_null: false, null_text: "\\N".into(), skip_errors: false }
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

    let spec = import_spec(read_opts(), mapping(&["id", "name", "note", "amount"]));
    let imported = import_csv(t.driver.as_ref(), &t.db, "dst", &file, &spec, no_progress(), CancellationToken::new()).await.unwrap();
    assert_eq!((imported.inserted, imported.failed.len()), (3, 0));
    // The NULL marker keeps NULL (row 1) and the empty string (row 3) apart.
    let (a, b) = (rows(&t, "src").await, rows(&t, "dst").await);
    assert_eq!((&b[0][2], &b[2][2]), (&CellValue::Null, &CellValue::Text(String::new())));
    assert_eq!(a, b);
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
    let strict =
        CsvImport { options: options.clone(), mapping: map.clone(), empty_as_null: true, null_text: String::new(), skip_errors: false };
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
    let report =
        run_sql_file(t.driver.as_ref(), &t.db, &file, TextEncoding::Utf8, true, no_progress(), CancellationToken::new()).await.unwrap();
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
    export(
        t.driver.as_ref(),
        &t.db,
        &ExportSource::Table { table: "table".into() },
        &format,
        &file,
        no_progress(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    let report =
        run_sql_file(t.driver.as_ref(), &t.db, &file, TextEncoding::Utf8, true, no_progress(), CancellationToken::new()).await.unwrap();
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
            run_sql_file(t.driver.as_ref(), &t.db, &file, TextEncoding::Utf8, stop_on_error, no_progress(), CancellationToken::new())
                .await
                .unwrap_err();
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
    let restored =
        run_sql_file(t.driver.as_ref(), &t.db, &file, TextEncoding::Utf8, true, no_progress(), CancellationToken::new()).await.unwrap();
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
    let restored =
        run_sql_file(t.driver.as_ref(), &t.db, &file, TextEncoding::Utf8, true, no_progress(), CancellationToken::new()).await.unwrap();
    assert!(restored.failed.is_empty(), "{:?}", restored.failed);
    assert_eq!(rows(&t, "generated_export").await, original);
}
scenario!(sql_export_recomputes_generated_columns);

async fn latin1_sql_file_is_refused_before_anything_runs(t: TestDb) {
    t.exec("CREATE TABLE keep (id INT PRIMARY KEY, note VARCHAR(20)); INSERT INTO keep VALUES (1, 'original');").await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("latin1.sql");
    // Windows-1252 text: the DROP comes first, the first non-UTF-8 byte (é = 0xE9) only on the third line.
    std::fs::write(
        &file,
        b"DROP TABLE keep;\nCREATE TABLE keep (id INT PRIMARY KEY, note VARCHAR(20));\nINSERT INTO keep VALUES (2, 'caf\xE9');\n",
    )
    .unwrap();
    let error =
        run_sql_file(t.driver.as_ref(), &t.db, &file, TextEncoding::Utf8, true, no_progress(), CancellationToken::new()).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Validation);
    assert!(error.message.contains("line 3") && error.message.contains("Windows-1252"), "{}", error.message);
    assert_eq!(rows(&t, "keep").await, vec![vec![CellValue::Int(1), CellValue::Text("original".into())]], "nothing may have run");

    let report = run_sql_file(t.driver.as_ref(), &t.db, &file, TextEncoding::Windows1252, true, no_progress(), CancellationToken::new())
        .await
        .unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(rows(&t, "keep").await, vec![vec![CellValue::Int(2), CellValue::Text("café".into())]]);
}
scenario!(latin1_sql_file_is_refused_before_anything_runs);

#[tokio::test]
async fn scanning_a_sql_file_lists_the_databases_it_switches_to() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("dump.sql");
    std::fs::write(
        &file,
        "-- MySQL dump\nCREATE DATABASE /*!32312 IF NOT EXISTS*/ `shop` /*!40100 DEFAULT CHARACTER SET utf8mb4 */;\n\nUSE `shop`;\n\
         CREATE TABLE t (note TEXT DEFAULT 'USE other');\nINSERT INTO t VALUES ('CREATE DATABASE x;');\ncreate schema if not exists archive;\nuse shop;\n",
    )
    .unwrap();
    let scan = scan_sql_file(&file, TextEncoding::Utf8, Engine::MySql).await.unwrap();
    assert_eq!(scan.databases, vec!["shop", "archive"]);

    std::fs::write(&file, b"USE `caf\xE9`;\n").unwrap();
    let error = scan_sql_file(&file, TextEncoding::Utf8, Engine::MySql).await.unwrap_err();
    assert!(error.message.contains("Windows-1252"), "{}", error.message);
    assert_eq!(scan_sql_file(&file, TextEncoding::Windows1252, Engine::MySql).await.unwrap().databases, vec!["café"]);
}

async fn csv_rows_with_the_wrong_number_of_fields_fail(t: TestDb) {
    seed(&t).await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ragged.csv");
    // Line 3 is short and line 4 long: neither may be shifted into other columns or padded with NULL.
    std::fs::write(&file, "id,name,note\n1,Ana,x\n2,Bob\n3,\"Smith, John\",NYC,20\n4,Dee,z\n").unwrap();
    let strict = import_spec(read_opts(), mapping(&["id", "name", "note"]));
    let report = import_csv(t.driver.as_ref(), &t.db, "dst", &file, &strict, no_progress(), CancellationToken::new()).await.unwrap();
    assert_eq!((report.inserted, report.failed_count), (0, 1));
    assert_eq!(report.failed[0].line, 3);
    assert!(report.failed[0].message.contains("Expected 3 fields but found 2"), "{}", report.failed[0].message);
    assert!(rows(&t, "dst").await.is_empty(), "the import must be rolled back");

    let lenient = CsvImport { skip_errors: true, ..strict };
    let report = import_csv(t.driver.as_ref(), &t.db, "dst", &file, &lenient, no_progress(), CancellationToken::new()).await.unwrap();
    assert_eq!((report.inserted, report.failed_count), (2, 2));
    assert_eq!(report.failed.iter().map(|f| f.line).collect::<Vec<_>>(), vec![3, 4]);
    let ids: Vec<CellValue> = rows(&t, "dst").await.into_iter().map(|r| r[0].clone()).collect();
    assert_eq!(ids, vec![CellValue::Int(1), CellValue::Int(4)]);

    // Without a header row, the first row sets the width.
    std::fs::write(&file, "1,Ana,x\n2,Bob\n").unwrap();
    let headless = import_spec(CsvReadOptions { has_header: false, ..read_opts() }, mapping(&["id", "name", "note"]));
    t.exec("DELETE FROM dst").await;
    let report = import_csv(t.driver.as_ref(), &t.db, "dst", &file, &headless, no_progress(), CancellationToken::new()).await.unwrap();
    assert_eq!((report.inserted, report.failed[0].line), (0, 2));
}
scenario!(csv_rows_with_the_wrong_number_of_fields_fail);

async fn csv_round_trip_keeps_binary_and_bit_values(t: TestDb) {
    let server = t.engine != Engine::Sqlite;
    let bits = if server { ", flags BIT(8), flag BIT(1)" } else { "" };
    for table in ["bin_src", "bin_dst"] {
        t.exec(&format!("CREATE TABLE {table} (id INT PRIMARY KEY, data BLOB, note VARCHAR(10){bits})")).await;
    }
    let values = if server {
        "(1, X'00FF0A', '', 5, 0), (2, NULL, NULL, NULL, 1), (3, X'', '0x41', 255, 1)"
    } else {
        "(1, X'00FF0A', ''), (2, NULL, NULL), (3, X'', '0x41')"
    };
    t.exec(&format!("INSERT INTO bin_src VALUES {values}")).await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("bin.csv");
    let source = ExportSource::Table { table: "bin_src".into() };
    export(t.driver.as_ref(), &t.db, &source, &ExportFormat::Csv(csv_opts()), &file, no_progress(), CancellationToken::new())
        .await
        .unwrap();
    let columns: &[&str] = if server { &["id", "data", "note", "flags", "flag"] } else { &["id", "data", "note"] };
    let spec = import_spec(read_opts(), mapping(columns));
    let report = import_csv(t.driver.as_ref(), &t.db, "bin_dst", &file, &spec, no_progress(), CancellationToken::new()).await.unwrap();
    assert_eq!((report.inserted, report.failed_count), (3, 0), "{:?}", report.failed);
    // Bytes come back as bytes, text that only looks like hex stays text, BIT values keep their numbers.
    assert_eq!(rows(&t, "bin_src").await, rows(&t, "bin_dst").await);
}
scenario!(csv_round_trip_keeps_binary_and_bit_values);

#[tokio::test]
async fn csv_that_is_not_utf8_is_refused_rather_than_altered() {
    let t = TestDb::sqlite().await;
    seed(&t).await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("excel.csv");
    std::fs::write(&file, b"id,name\n1,Jos\xE9\n").unwrap();
    let error = preview_csv(&file, &read_opts(), 10).unwrap_err();
    assert!(error.message.contains("not valid UTF-8") && error.message.contains("Windows-1252"), "{}", error.message);
    let spec = import_spec(read_opts(), mapping(&["id", "name"]));
    let error = import_csv(t.driver.as_ref(), &t.db, "dst", &file, &spec, no_progress(), CancellationToken::new()).await.unwrap_err();
    assert!(error.message.contains("Windows-1252"), "{}", error.message);
    assert!(rows(&t, "dst").await.is_empty());

    let latin = import_spec(CsvReadOptions { encoding: TextEncoding::Windows1252, ..read_opts() }, mapping(&["id", "name"]));
    import_csv(t.driver.as_ref(), &t.db, "dst", &file, &latin, no_progress(), CancellationToken::new()).await.unwrap();
    assert_eq!(rows(&t, "dst").await[0][1], CellValue::Text("José".into()));
}

#[tokio::test]
async fn windows_1252_export_streams_large_text_and_refuses_unrepresentable_characters() {
    let t = TestDb::sqlite().await;
    t.exec("CREATE TABLE latin (id INTEGER PRIMARY KEY, note TEXT)").await;
    // Far beyond the CSV writer's buffer, so writes split multi-byte characters.
    let long = "ação é ç ".repeat(3000);
    t.exec(&format!("INSERT INTO latin VALUES (1, '{long}'), (2, 'œ€…')")).await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("latin.csv");
    let options = CsvOptions { encoding: TextEncoding::Windows1252, ..csv_opts() };
    let source = ExportSource::Query { sql: "SELECT id, note FROM latin ORDER BY id".into() };
    export(t.driver.as_ref(), &t.db, &source, &ExportFormat::Csv(options.clone()), &file, no_progress(), CancellationToken::new())
        .await
        .unwrap();
    let bytes = std::fs::read(&file).unwrap();
    let (text, _, malformed) = encoding_rs::WINDOWS_1252.decode(&bytes);
    assert!(!malformed);
    assert_eq!(text.replace("\r\n", "\n"), format!("id,note\n1,{long}\n2,œ€…\n"));

    t.exec("INSERT INTO latin VALUES (3, 'smile 😀')").await;
    let error = export(t.driver.as_ref(), &t.db, &source, &ExportFormat::Csv(options), &file, no_progress(), CancellationToken::new())
        .await
        .unwrap_err();
    assert!(error.message.contains("😀") && error.message.contains("Windows-1252"), "{}", error.message);
    assert_eq!(std::fs::read(&file).unwrap(), bytes, "the previous export stays in place");
}

async fn exporting_a_query_that_writes_is_refused(t: TestDb) {
    seed(&t).await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("insert.csv");
    let source = ExportSource::Query { sql: "INSERT INTO src (id, name) VALUES (9, 'again')".into() };
    let error = export(t.driver.as_ref(), &t.db, &source, &ExportFormat::Csv(csv_opts()), &file, no_progress(), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Validation);
    assert_eq!(t.driver.count_rows(&t.db, "src", &[]).await.unwrap(), 3, "the statement must not run again");
    assert!(!file.exists());
}
scenario!(exporting_a_query_that_writes_is_refused);
