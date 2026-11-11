//! Export, import and SQL-file execution. Rows stream in batches; nothing holds a whole table in memory.

use crate::{AppError, ColumnMeta, Driver, Engine, Result, Value, split_statements};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
pub use tokio_util::sync::CancellationToken;

pub const BATCH: usize = 500;
/// Rows per INSERT statement in SQL exports.
const INSERT_ROWS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub done: u64,
    pub total: Option<u64>,
}

pub type ProgressFn = Arc<dyn Fn(Progress) + Send + Sync>;

/// A stream item: the column list always comes first.
pub enum StreamItem {
    Columns(Vec<ColumnMeta>),
    Rows(Vec<Vec<Value>>),
}

pub type RowStream = tokio::sync::mpsc::Receiver<Result<StreamItem>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextEncoding {
    Utf8,
    Utf8Bom,
    Windows1252,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvOptions {
    pub delimiter: char,
    pub quote: char,
    pub header: bool,
    pub encoding: TextEncoding,
    /// Text written for NULL values (CSV has no NULL).
    pub null_text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlExportOptions {
    pub include_create: bool,
    /// Table name used in the INSERT statements; defaults to the source table (or `query_result`).
    pub table_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "format", content = "options", rename_all = "camelCase")]
pub enum ExportFormat {
    Csv(CsvOptions),
    Json,
    Sql(SqlExportOptions),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ExportSource {
    Table { table: String },
    Query { sql: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub rows: u64,
}

fn ascii(c: char, what: &str) -> Result<u8> {
    u8::try_from(c).map_err(|_| AppError::validation(format!("The {what} must be a single ASCII character")))
}

/// Writes UTF-8 text through the selected encoding.
struct Encoded<W: Write> {
    out: W,
    encoding: TextEncoding,
}

impl<W: Write> Encoded<W> {
    fn new(mut out: W, encoding: TextEncoding) -> Result<Self> {
        if encoding == TextEncoding::Utf8Bom {
            out.write_all(b"\xEF\xBB\xBF")?;
        }
        Ok(Self { out, encoding })
    }
}

impl<W: Write> Write for Encoded<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.encoding {
            TextEncoding::Utf8 | TextEncoding::Utf8Bom => self.out.write_all(buf)?,
            TextEncoding::Windows1252 => {
                // csv hands over whole fields/records, so chunks never split a UTF-8 sequence.
                let text = std::str::from_utf8(buf).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                let (bytes, _, _) = encoding_rs::WINDOWS_1252.encode(text);
                self.out.write_all(&bytes)?;
            }
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.out.flush()
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}

fn csv_field(v: &Value, null_text: &str) -> String {
    match v {
        Value::Null => null_text.to_owned(),
        Value::Bytes(b) => format!("0x{}", hex(b)),
        other => other.as_text().unwrap_or_default(),
    }
}

fn json_value(v: &Value) -> serde_json::Value {
    const SAFE: u64 = 9_007_199_254_740_992;
    match v {
        Value::Null => serde_json::Value::Null,
        Value::Int(i) if i.unsigned_abs() <= SAFE => (*i).into(),
        Value::UInt(u) if *u <= SAFE => (*u).into(),
        Value::Float(f) => serde_json::Number::from_f64(*f).map(serde_json::Value::Number).unwrap_or_else(|| f.to_string().into()),
        Value::Json(j) => serde_json::from_str(j).unwrap_or_else(|_| j.clone().into()),
        Value::Bytes(b) => format!("0x{}", hex(b)).into(),
        other => other.as_text().unwrap_or_default().into(),
    }
}

/// SQL literal for INSERT statements in the given dialect.
pub(crate) fn literal(v: &Value, engine: Engine) -> String {
    let quote = |s: &str| match engine {
        Engine::Sqlite => format!("'{}'", s.replace('\'', "''")),
        _ => crate::design::mysql::literal(s),
    };
    match v {
        Value::Null => "NULL".to_owned(),
        Value::Int(i) => i.to_string(),
        Value::UInt(u) => u.to_string(),
        Value::Float(f) if f.is_finite() => format!("{f:?}"),
        Value::Float(f) => quote(&f.to_string()),
        Value::Decimal(d) => d.clone(),
        Value::Text(s) | Value::DateTime(s) | Value::Json(s) => quote(s),
        Value::Bytes(b) if b.is_empty() => "''".to_owned(),
        Value::Bytes(b) => format!("X'{}'", hex(b)),
    }
}

pub(crate) fn quote_for(engine: Engine) -> fn(&str) -> String {
    match engine {
        Engine::Sqlite => crate::sqlite::quote_ident,
        _ => crate::mysql::quote_ident,
    }
}

/// Writes `INSERT` statements for a batch of rows.
pub(crate) fn insert_statements(out: &mut impl Write, engine: Engine, table: &str, columns: &[ColumnMeta], rows: &[Vec<Value>]) -> Result<()> {
    let q = quote_for(engine);
    let visible: Vec<usize> = (0..columns.len()).filter(|i| !columns[*i].hidden).collect();
    let cols = visible.iter().map(|i| q(&columns[*i].name)).collect::<Vec<_>>().join(", ");
    for chunk in rows.chunks(INSERT_ROWS) {
        let values = chunk
            .iter()
            .map(|r| format!("({})", visible.iter().map(|i| literal(&r[*i], engine)).collect::<Vec<_>>().join(", ")))
            .collect::<Vec<_>>()
            .join(",\n  ");
        writeln!(out, "INSERT INTO {} ({cols}) VALUES\n  {values};", q(table))?;
    }
    Ok(())
}

pub async fn export(
    driver: &dyn Driver,
    db: &str,
    source: &ExportSource,
    format: &ExportFormat,
    path: &Path,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<ExportReport> {
    if cancel.is_cancelled() {
        return Err(AppError::cancelled());
    }
    let engine = driver.engine();
    let q = quote_for(engine);
    let (sql, total, default_name) = match source {
        ExportSource::Table { table } => (
            format!("SELECT * FROM {}.{}", q(db), q(table)),
            Some(driver.count_rows(db, table, &[]).await?),
            table.clone(),
        ),
        ExportSource::Query { sql } => (sql.trim().trim_end_matches(';').to_owned(), None, "query_result".to_owned()),
    };
    let mut stream = driver.stream_query(Some(db), &sql, BATCH).await?;
    let columns = match stream.recv().await {
        Some(Ok(StreamItem::Columns(c))) => c,
        Some(Err(e)) => return Err(e),
        _ => return Err(AppError::sql("The query did not return a result set")),
    };
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut done = 0u64;
    progress(Progress { done, total });

    // Each format: a header, a writer per batch and a footer.
    enum Sink<W: Write> {
        Csv(csv::Writer<Encoded<W>>, String),
        Json(W, bool),
        Sql(W, String),
    }
    let names: Vec<String> = columns.iter().filter(|c| !c.hidden).map(|c| c.name.clone()).collect();
    let mut sink = match format {
        ExportFormat::Csv(o) => {
            let mut w = csv::WriterBuilder::new()
                .delimiter(ascii(o.delimiter, "delimiter")?)
                .quote(ascii(o.quote, "quote character")?)
                .from_writer(Encoded::new(file, o.encoding)?);
            if o.header {
                w.write_record(&names).map_err(|e| AppError::io(e.to_string()))?;
            }
            Sink::Csv(w, o.null_text.clone())
        }
        ExportFormat::Json => {
            let mut f = file;
            f.write_all(b"[")?;
            Sink::Json(f, true)
        }
        ExportFormat::Sql(o) => {
            let mut f = file;
            let name = o.table_name.clone().filter(|n| !n.is_empty()).unwrap_or(default_name);
            writeln!(f, "-- Exported by ProjectCat")?;
            if o.include_create {
                if let ExportSource::Table { table } = source {
                    let create = driver.table_info(db, table).await?.create_sql;
                    let create = if name != *table { create.replacen(table.as_str(), &name, 1) } else { create };
                    writeln!(f, "{};\n", create.trim_end_matches(';'))?;
                }
            }
            Sink::Sql(f, name)
        }
    };
    let visible: Vec<usize> = (0..columns.len()).filter(|i| !columns[*i].hidden).collect();

    loop {
        let batch = tokio::select! {
            _ = cancel.cancelled() => return Err(AppError::cancelled()),
            item = stream.recv() => item,
        };
        let rows = match batch {
            None => break,
            Some(Err(e)) => return Err(e),
            Some(Ok(StreamItem::Columns(_))) => continue,
            Some(Ok(StreamItem::Rows(rows))) => rows,
        };
        match &mut sink {
            Sink::Csv(w, null_text) => {
                for r in &rows {
                    w.write_record(visible.iter().map(|i| csv_field(&r[*i], null_text))).map_err(|e| AppError::io(e.to_string()))?;
                }
            }
            Sink::Json(f, first) => {
                for r in &rows {
                    let obj: serde_json::Map<String, serde_json::Value> =
                        visible.iter().map(|i| (columns[*i].name.clone(), json_value(&r[*i]))).collect();
                    f.write_all(if *first { b"\n  " } else { b",\n  " })?;
                    *first = false;
                    serde_json::to_writer(&mut *f, &obj)?;
                }
            }
            Sink::Sql(f, name) => insert_statements(f, engine, name, &columns, &rows)?,
        }
        done += rows.len() as u64;
        progress(Progress { done, total });
    }
    match sink {
        Sink::Csv(mut w, _) => w.flush()?,
        Sink::Json(mut f, _) => {
            f.write_all(b"\n]\n")?;
            f.flush()?;
        }
        Sink::Sql(mut f, _) => f.flush()?,
    }
    Ok(ExportReport { rows: done })
}

// ---------------------------------------------------------------- import

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvReadOptions {
    pub delimiter: char,
    pub quote: char,
    pub has_header: bool,
    pub encoding: TextEncoding,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvPreview {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMapping {
    /// Zero-based CSV column.
    pub source: usize,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvImport {
    pub options: CsvReadOptions,
    pub mapping: Vec<ColumnMapping>,
    /// Empty fields become NULL instead of the empty string.
    pub empty_as_null: bool,
    /// Keep going past rows that fail (they are reported); otherwise the first failure rolls back everything.
    pub skip_errors: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowFailure {
    /// 1-based line in the source file.
    pub line: u64,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub inserted: u64,
    /// At most [`MAX_REPORTED_FAILURES`] entries.
    pub failed: Vec<RowFailure>,
    pub failed_count: u64,
}

pub const MAX_REPORTED_FAILURES: usize = 100;

/// A batch of rows to insert, each tagged with its source line.
pub type ImportBatch = Vec<(u64, Vec<Value>)>;

fn csv_reader(path: &Path, o: &CsvReadOptions) -> Result<csv::Reader<Box<dyn std::io::Read + Send>>> {
    let file = std::fs::File::open(path)?;
    let encoding = match o.encoding {
        TextEncoding::Windows1252 => Some(encoding_rs::WINDOWS_1252),
        _ => Some(encoding_rs::UTF_8),
    };
    // Strips a BOM when present and transcodes to UTF-8.
    let reader: Box<dyn std::io::Read + Send> =
        Box::new(encoding_rs_io::DecodeReaderBytesBuilder::new().encoding(encoding).bom_override(true).build(file));
    Ok(csv::ReaderBuilder::new()
        .delimiter(ascii(o.delimiter, "delimiter")?)
        .quote(ascii(o.quote, "quote character")?)
        .has_headers(o.has_header)
        .flexible(true)
        .from_reader(reader))
}

pub fn preview_csv(path: &Path, options: &CsvReadOptions, limit: usize) -> Result<CsvPreview> {
    let mut reader = csv_reader(path, options)?;
    let rows: Vec<Vec<String>> = reader
        .records()
        .take(limit)
        .map(|r| r.map(|r| r.iter().map(str::to_owned).collect()).map_err(|e| AppError::validation(format!("Invalid CSV: {e}"))))
        .collect::<Result<_>>()?;
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let headers = if options.has_header {
        reader.headers().map_err(|e| AppError::validation(format!("Invalid CSV: {e}")))?.iter().map(str::to_owned).collect()
    } else {
        (1..=width).map(|i| format!("column {i}")).collect()
    };
    Ok(CsvPreview { headers, rows })
}

pub async fn import_csv(
    driver: &dyn Driver,
    db: &str,
    table: &str,
    path: &Path,
    spec: &CsvImport,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<ImportReport> {
    if spec.mapping.is_empty() {
        return Err(AppError::validation("Map at least one column"));
    }
    let total = {
        let mut r = csv_reader(path, &spec.options)?;
        Some(r.records().count() as u64)
    };
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<ImportBatch>>(4);
    let reader = csv_reader(path, &spec.options)?;
    let spec_c = spec.clone();
    let cancel_c = cancel.clone();
    let producer = tokio::task::spawn_blocking(move || {
        let mut reader = reader;
        let mut batch: ImportBatch = Vec::with_capacity(BATCH);
        let mut done = 0u64;
        for record in reader.records() {
            if cancel_c.is_cancelled() {
                let _ = tx.blocking_send(Err(AppError::cancelled()));
                return;
            }
            let record = match record {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.blocking_send(Err(AppError::validation(format!("Invalid CSV: {e}"))));
                    return;
                }
            };
            let line = record.position().map(|p| p.line()).unwrap_or(0);
            let values = spec_c
                .mapping
                .iter()
                .map(|m| match record.get(m.source) {
                    Some("") if spec_c.empty_as_null => Value::Null,
                    Some(s) => Value::Text(s.to_owned()),
                    None => Value::Null,
                })
                .collect();
            batch.push((line, values));
            if batch.len() == BATCH {
                done += BATCH as u64;
                if tx.blocking_send(Ok(std::mem::take(&mut batch))).is_err() {
                    return;
                }
                progress(Progress { done, total });
            }
        }
        done += batch.len() as u64;
        if !batch.is_empty() && tx.blocking_send(Ok(batch)).is_err() {
            return;
        }
        progress(Progress { done, total });
    });
    let columns: Vec<String> = spec.mapping.iter().map(|m| m.target.clone()).collect();
    let report = driver.import_rows(db, table, &columns, rx, spec.skip_errors).await;
    let _ = producer.await;
    report
}

// ---------------------------------------------------------------- SQL files

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatementFailure {
    pub index: usize,
    pub statement: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptRunReport {
    pub executed: u64,
    pub failed: Vec<StatementFailure>,
}

pub(crate) fn failure(index: usize, statement: &str, message: String) -> StatementFailure {
    let statement = if statement.len() > 300 { format!("{}…", &statement[..statement.floor_char_boundary(300)]) } else { statement.to_owned() };
    StatementFailure { index, statement, message }
}

/// Runs every statement of a `.sql` file on one session, outside a transaction (dumps manage their own).
pub async fn run_sql_file(
    driver: &dyn Driver,
    db: &str,
    path: &Path,
    stop_on_error: bool,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<ScriptRunReport> {
    let bytes = tokio::fs::read(path).await?;
    let text = String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes)).into_owned();
    let statements = split_statements(&text, driver.engine());
    driver.execute_statements(Some(db), statements, stop_on_error, progress, cancel).await
}
