//! Export, import and SQL-file execution. Rows stream in batches; nothing holds a whole table in memory.

use crate::{AppError, ColumnMeta, Driver, Engine, Result, Splitter, Value};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
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

/// Output written to a uniquely named temporary file next to the destination and moved into place only on
/// success, so a failed or cancelled export never destroys an existing file. Dropping it removes the file.
struct PendingFile {
    tmp: tempfile::NamedTempFile,
    path: std::path::PathBuf,
}

impl PendingFile {
    fn create(path: &Path) -> Result<(PendingFile, std::io::BufWriter<std::fs::File>)> {
        let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let prefix = format!(".{}.", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        let tmp = tempfile::Builder::new().prefix(&prefix).suffix(".partial").tempfile_in(dir)?;
        let file = tmp.reopen()?;
        Ok((PendingFile { tmp, path: path.to_owned() }, std::io::BufWriter::new(file)))
    }

    fn commit(self) -> Result<()> {
        self.tmp.persist(&self.path).map_err(|e| AppError::io(e.to_string()))?;
        Ok(())
    }
}

/// Reads a SQL text file. It must be UTF-8; a leading byte-order mark is dropped so it never reaches the server.
pub async fn read_sql_text(path: &Path) -> Result<String> {
    let bytes = tokio::fs::read(path).await?;
    let text = String::from_utf8(bytes).map_err(|_| {
        AppError::validation(format!("{} is not a UTF-8 text file; convert it to UTF-8 to open it", path.display()))
    })?;
    Ok(match text.strip_prefix('\u{feff}') {
        Some(rest) => rest.to_owned(),
        None => text,
    })
}

/// Writes `contents` to a temporary file next to `path`, flushes it to disk and then replaces `path`, so a
/// failed write leaves any existing file untouched.
pub async fn write_text_atomic(path: &Path, contents: &str) -> Result<()> {
    let path = path.to_owned();
    let contents = contents.to_owned();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let (pending, mut file) = PendingFile::create(&path)?;
        file.write_all(contents.as_bytes())?;
        file.into_inner().map_err(|e| AppError::io(e.to_string()))?.sync_all()?;
        pending.commit()
    })
    .await
    .map_err(|e| AppError::io(e.to_string()))?
}

/// Rewrites the table-name token of a `CREATE [TEMP|TEMPORARY] TABLE [IF NOT EXISTS] [schema.]name ...`
/// statement to `quoted` (an already-quoted identifier). Returns `None` when the statement has another shape.
pub(crate) fn rename_create_table(create_sql: &str, quoted: &str) -> Option<String> {
    let bytes = create_sql.as_bytes();
    let mut pos = 0usize;
    let skip_ws = |mut p: usize| {
        while p < bytes.len() && bytes[p].is_ascii_whitespace() {
            p += 1;
        }
        p
    };
    // Reads one identifier (quoted or bare) at `p` and returns its end.
    let ident_end = |p: usize| -> Option<usize> {
        let close = match *bytes.get(p)? {
            b'`' => b'`',
            b'"' => b'"',
            b'[' => b']',
            c if c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80 => {
                let mut e = p;
                while e < bytes.len() && (bytes[e].is_ascii_alphanumeric() || bytes[e] == b'_' || bytes[e] == b'$' || bytes[e] >= 0x80) {
                    e += 1;
                }
                return Some(e);
            }
            _ => return None,
        };
        let mut e = p + 1;
        loop {
            if *bytes.get(e)? == close {
                // A doubled closing quote is an escaped quote (not for brackets).
                if close != b']' && bytes.get(e + 1) == Some(&close) {
                    e += 2;
                    continue;
                }
                return Some(e + 1);
            }
            e += 1;
        }
    };
    let keyword = |pos: &mut usize, word: &str| -> bool {
        let p = skip_ws(*pos);
        let end = p + word.len();
        let matches = create_sql.get(p..end).is_some_and(|w| w.eq_ignore_ascii_case(word))
            && bytes.get(end).is_none_or(|c| !(c.is_ascii_alphanumeric() || *c == b'_'));
        if matches {
            *pos = end;
        }
        matches
    };
    if !keyword(&mut pos, "CREATE") {
        return None;
    }
    let _ = keyword(&mut pos, "TEMPORARY") || keyword(&mut pos, "TEMP");
    if !keyword(&mut pos, "TABLE") {
        return None;
    }
    let before_if = pos;
    if !(keyword(&mut pos, "IF") && keyword(&mut pos, "NOT") && keyword(&mut pos, "EXISTS")) {
        pos = before_if;
    }
    let start = skip_ws(pos);
    let mut end = ident_end(start)?;
    // `schema.name`: replace both, the export targets the destination's current database.
    let after = skip_ws(end);
    if bytes.get(after) == Some(&b'.') {
        end = ident_end(skip_ws(after + 1))?;
    }
    Some(format!("{}{}{}", &create_sql[..start], quoted, &create_sql[end..]))
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
        Value::Bytes(b) => format!("X'{}'", hex(b)),
    }
}

pub(crate) fn quote_for(engine: Engine) -> fn(&str) -> String {
    match engine {
        Engine::Sqlite => crate::sqlite::quote_ident,
        _ => crate::mysql::quote_ident,
    }
}

/// Generated values stay visible in query results but must be recomputed when INSERTs are restored.
fn hide_generated(columns: &mut [ColumnMeta], info: &crate::TableInfo) {
    for generated in info.columns.iter().filter(|c| c.generated) {
        if let Some(column) = columns.iter_mut().find(|c| c.name == generated.name) {
            column.hidden = true;
        }
    }
}

/// Writes `INSERT` statements for a batch of rows.
pub(crate) fn insert_statements(
    out: &mut impl Write,
    engine: Engine,
    table: &str,
    columns: &[ColumnMeta],
    rows: &[Vec<Value>],
) -> Result<()> {
    let q = quote_for(engine);
    let visible: Vec<usize> = (0..columns.len()).filter(|i| !columns[*i].hidden).collect();
    if visible.is_empty() {
        for _ in rows {
            match engine {
                Engine::Sqlite => writeln!(out, "INSERT INTO {} DEFAULT VALUES;", q(table))?,
                Engine::MySql | Engine::MariaDb => writeln!(out, "INSERT INTO {} () VALUES ();", q(table))?,
            }
        }
        return Ok(());
    }
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
        ExportSource::Table { table } => {
            (format!("SELECT * FROM {}.{}", q(db), q(table)), Some(driver.count_rows(db, table, &[]).await?), table.clone())
        }
        ExportSource::Query { sql } => (sql.trim().trim_end_matches(';').to_owned(), None, "query_result".to_owned()),
    };
    // SQLite's stream owns the connection until it is drained; metadata must be read first.
    let table_info = match (source, format) {
        (ExportSource::Table { table }, ExportFormat::Sql(_)) => Some(driver.table_info(db, table).await?),
        _ => None,
    };
    let mut stream = driver.stream_query(Some(db), &sql, BATCH).await?;
    let mut columns = match stream.recv().await {
        Some(Ok(StreamItem::Columns(c))) => c,
        Some(Err(e)) => return Err(e),
        _ => return Err(AppError::sql("The query did not return a result set")),
    };
    if let Some(info) = &table_info {
        hide_generated(&mut columns, info);
    }
    let (pending, file) = PendingFile::create(path)?;
    let mut done = 0u64;
    progress(Progress { done, total });

    // Each format: a header, a writer per batch and a footer.
    enum Sink<W: Write> {
        Csv(Box<csv::Writer<Encoded<W>>>, String),
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
            Sink::Csv(Box::new(w), o.null_text.clone())
        }
        ExportFormat::Json => {
            let mut f = file;
            f.write_all(b"[")?;
            Sink::Json(f, true)
        }
        ExportFormat::Sql(o) => {
            let mut f = file;
            let name = o.table_name.clone().filter(|n| !n.is_empty()).unwrap_or(default_name);
            writeln!(f, "-- Exported by Schemata")?;
            if o.include_create
                && let ExportSource::Table { table } = source
                && let Some(info) = &table_info
            {
                let renamed = if name != *table {
                    Some(rename_create_table(&info.create_sql, &q(&name)).ok_or_else(|| {
                        AppError::validation("Cannot rename the table in its CREATE statement; export it under its own name")
                    })?)
                } else {
                    None
                };
                let create = renamed.as_deref().unwrap_or(&info.create_sql);
                writeln!(f, "{};\n", create.trim_end_matches(';'))?;
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
        Sink::Csv(w, _) => {
            drop(w.into_inner().map_err(|e| AppError::io(e.to_string()))?.out.into_inner().map_err(|e| AppError::io(e.to_string()))?)
        }
        Sink::Json(mut f, _) => {
            f.write_all(b"\n]\n")?;
            drop(f.into_inner().map_err(|e| AppError::io(e.to_string()))?);
        }
        Sink::Sql(f, _) => drop(f.into_inner().map_err(|e| AppError::io(e.to_string()))?),
    }
    pending.commit()?;
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
    let report = driver.import_rows(db, table, &columns, rx, spec.skip_errors, false).await;
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
    let statement =
        if statement.len() > 300 { format!("{}…", &statement[..statement.floor_char_boundary(300)]) } else { statement.to_owned() };
    StatementFailure { index, statement, message }
}

/// Runs every statement of a `.sql` file on one session, outside a transaction (dumps manage their own).
/// The file is read and split incrementally; progress is reported in bytes read.
pub async fn run_sql_file(
    driver: &dyn Driver,
    db: &str,
    path: &Path,
    stop_on_error: bool,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<ScriptRunReport> {
    let file = std::fs::File::open(path)?;
    let total = file.metadata()?.len();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<String>>(64);
    let engine = driver.engine();
    let cancel_reader = cancel.clone();
    let reader = tokio::task::spawn_blocking(move || {
        let run = || -> Result<()> {
            let mut input = std::io::BufReader::new(file);
            let mut splitter = Splitter::new(engine);
            let mut line = Vec::new();
            let mut read = 0u64;
            let mut first = true;
            loop {
                line.clear();
                let n = input.read_until(b'\n', &mut line)?;
                if n == 0 || cancel_reader.is_cancelled() {
                    break;
                }
                read += n as u64;
                let bytes = if first { line.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&line) } else { &line };
                first = false;
                let text = std::str::from_utf8(bytes).map_err(|e| AppError::validation(format!("The SQL file is not valid UTF-8: {e}")))?;
                for statement in splitter.feed(text) {
                    if tx.blocking_send(Ok(statement)).is_err() {
                        return Ok(());
                    }
                }
                progress(Progress { done: read, total: Some(total) });
            }
            if !cancel_reader.is_cancelled() {
                for statement in splitter.finish() {
                    if tx.blocking_send(Ok(statement)).is_err() {
                        break;
                    }
                }
            }
            Ok(())
        };
        if let Err(error) = run() {
            let _ = tx.blocking_send(Err(error));
        }
    });
    let report = driver.execute_statements(Some(db), rx, stop_on_error, cancel).await;
    reader.await.map_err(|e| AppError::io(e.to_string()))?;
    report
}

// ---------------------------------------------------------------- copy table

/// Where a copied table goes: possibly another database, connection or engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyTableTarget {
    pub db: String,
    pub name: String,
    /// Copy the rows too, not only the structure.
    pub data: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyTableReport {
    pub rows: u64,
}

/// Structure of `design` recreated as `name` in the target database. Self-references follow the copy;
/// foreign keys to tables missing from the target, table options of another engine, and index features SQLite
/// lacks (FULLTEXT/SPATIAL indexes, index methods and comments) are dropped.
/// Index and foreign key names are scoped to the database (SQLite indexes, MySQL foreign keys), so a copy
/// under a new name renames them after it (`ix_orders_total` → `ix_orders_copy_total`).
pub(crate) fn copy_design(
    mut design: crate::TableDesign,
    name: &str,
    source_engine: Engine,
    target_engine: Engine,
    target_tables: &[String],
) -> crate::TableDesign {
    let source_name = std::mem::replace(&mut design.name, name.to_owned());
    let rename = |object: &mut String| {
        if source_name != name {
            *object = if object.contains(&source_name) { object.replacen(&source_name, name, 1) } else { format!("{name}_{object}") };
        }
    };
    design.foreign_keys.retain(|fk| fk.ref_table == source_name || target_tables.contains(&fk.ref_table));
    for fk in &mut design.foreign_keys {
        if fk.ref_table == source_name {
            fk.ref_table = name.to_owned();
        }
        rename(&mut fk.name);
    }
    for index in &mut design.indexes {
        rename(&mut index.name);
    }
    if source_engine != target_engine {
        design.options = crate::TableOptions { comment: design.options.comment.take(), ..Default::default() };
    }
    if target_engine == Engine::Sqlite {
        // SQLite has neither FULLTEXT/SPATIAL indexes nor index methods and comments.
        design.indexes.retain(|i| matches!(i.kind, crate::IndexKind::Normal | crate::IndexKind::Unique));
        for index in &mut design.indexes {
            index.method = None;
            index.comment = None;
        }
    }
    for c in &mut design.columns {
        c.original_name = None;
        // SQLite only auto-increments an INTEGER PRIMARY KEY.
        if target_engine == Engine::Sqlite && c.auto_increment {
            c.data_type = "INTEGER".into();
        }
    }
    design
}

/// Copies a table's structure (and optionally its rows) into another database, possibly on another
/// connection or engine. The new table is dropped again when the data copy fails or is cancelled.
pub async fn copy_table(
    source: &dyn Driver,
    source_db: &str,
    table: &str,
    target: &dyn Driver,
    to: &CopyTableTarget,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<CopyTableReport> {
    if to.name.trim().is_empty() {
        return Err(AppError::validation("Enter a name for the copy"));
    }
    let target_tables: Vec<String> = target.list_objects(&to.db).await?.into_iter().map(|o| o.name).collect();
    if target_tables.iter().any(|t| t.eq_ignore_ascii_case(&to.name)) {
        return Err(AppError::validation(format!("A table named {} already exists in {}", to.name, to.db)));
    }
    let design = copy_design(source.table_design(source_db, table).await?, &to.name, source.engine(), target.engine(), &target_tables);
    let ddl = target.plan_table(&to.db, None, &design).await?;
    target.apply_ddl(&to.db, &ddl).await?;
    if !to.data {
        return Ok(CopyTableReport { rows: 0 });
    }
    match copy_rows(source, source_db, table, target, to, progress, cancel).await {
        Ok(rows) => Ok(CopyTableReport { rows }),
        Err(e) => {
            let _ = target.drop_object(&to.db, &to.name, crate::ObjectKind::Table).await;
            Err(e)
        }
    }
}

async fn copy_rows(
    source: &dyn Driver,
    source_db: &str,
    table: &str,
    target: &dyn Driver,
    to: &CopyTableTarget,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<u64> {
    let total = Some(source.count_rows(source_db, table, &[]).await?);
    let q = quote_for(source.engine());
    let mut stream = source.stream_query(Some(source_db), &format!("SELECT * FROM {}.{}", q(source_db), q(table)), BATCH).await?;
    let columns = match stream.recv().await {
        Some(Ok(StreamItem::Columns(c))) => c,
        Some(Err(e)) => return Err(e),
        _ => return Err(AppError::sql("The table did not return a result set")),
    };
    if source.engine() == Engine::Sqlite && std::ptr::addr_eq(source, target) {
        // A SQLite stream holds the driver's only connection. Drain it to a temporary database first;
        // columns without affinity retain integers, floats, text (including NULs) and BLOBs exactly.
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("rows.sqlite");
        let staging = crate::connect(&crate::ConnectParams::sqlite(path.to_string_lossy().into_owned(), true)).await?;
        let names = columns.iter().filter(|c| !c.hidden).map(|c| q(&c.name)).collect::<Vec<_>>().join(", ");
        let created = staging.execute_script(Some("main"), &format!("CREATE TABLE staged_rows ({names})"), 1, None).await?;
        if let Some(error) = created.error {
            return Err(error.error);
        }
        let staged_target = CopyTableTarget { db: "main".into(), name: "staged_rows".into(), data: true };
        import_stream(stream, &columns, staging.as_ref(), &staged_target, Arc::new(|_| {}), cancel.clone(), total).await?;
        let mut staged_stream = staging.stream_query(Some("main"), "SELECT * FROM staged_rows", BATCH).await?;
        let staged_columns = match staged_stream.recv().await {
            Some(Ok(StreamItem::Columns(c))) => c,
            Some(Err(e)) => return Err(e),
            _ => return Err(AppError::sql("The staged table did not return a result set")),
        };
        return import_stream(staged_stream, &staged_columns, target, to, progress, cancel, total).await;
    }
    import_stream(stream, &columns, target, to, progress, cancel, total).await
}

async fn import_stream(
    mut stream: RowStream,
    columns: &[ColumnMeta],
    target: &dyn Driver,
    to: &CopyTableTarget,
    progress: ProgressFn,
    cancel: CancellationToken,
    total: Option<u64>,
) -> Result<u64> {
    let visible: Vec<usize> = (0..columns.len()).filter(|i| !columns[*i].hidden).collect();
    let names: Vec<String> = visible.iter().map(|i| columns[*i].name.clone()).collect();
    progress(Progress { done: 0, total });
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<ImportBatch>>(4);
    let feed = async move {
        let mut done = 0u64;
        loop {
            let item = tokio::select! {
                biased;
                _ = cancel.cancelled() => Some(Err(AppError::cancelled())),
                item = stream.recv() => item,
            };
            let rows = match item {
                None => break,
                Some(Ok(StreamItem::Columns(_))) => continue,
                Some(Ok(StreamItem::Rows(rows))) => rows,
                Some(Err(e)) => {
                    let _ = tx.send(Err(e)).await;
                    break;
                }
            };
            let batch: ImportBatch = rows
                .into_iter()
                .enumerate()
                .map(|(i, mut r)| (done + i as u64 + 1, visible.iter().map(|c| std::mem::replace(&mut r[*c], Value::Null)).collect()))
                .collect();
            done += batch.len() as u64;
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    let _ = tx.send(Err(AppError::cancelled())).await;
                    break;
                }
                sent = tx.send(Ok(batch)) => {
                    if sent.is_err() {
                        break;
                    }
                }
            }
            progress(Progress { done, total });
        }
    };
    let (report, ()) = tokio::join!(target.import_rows(&to.db, &to.name, &names, rx, false, false), feed);
    let report = report?;
    if let Some(f) = report.failed.first() {
        return Err(AppError::sql(format!("Row {} could not be copied: {}", f.line, f.message)));
    }
    Ok(report.inserted)
}

// ---------------------------------------------------------------- backup

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpOptions {
    pub structure: bool,
    pub data: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpReport {
    pub tables: u64,
    pub rows: u64,
}

/// Engine-specific parts of a dump, each item a complete SQL chunk (statement terminators included).
pub struct DumpScript {
    /// Session setup, written first.
    pub prologue: Vec<String>,
    /// Tables in a creation-safe order with their `DROP ...; CREATE ...;` text.
    pub tables: Vec<(String, String)>,
    /// Statements that depend on the data being present (e.g. SQLite's autoincrement counters).
    pub data_tail: Vec<String>,
    /// Indexes, views and triggers, in dependency order.
    pub objects: Vec<String>,
    pub epilogue: Vec<String>,
}

/// Writes a `.sql` dump of `db` that the engine's own CLI (or [`run_sql_file`]) can restore.
pub async fn dump_database(
    driver: &dyn Driver,
    db: &str,
    path: &Path,
    options: &DumpOptions,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<DumpReport> {
    if !options.structure && !options.data {
        return Err(AppError::validation("Choose structure, data or both"));
    }
    let engine = driver.engine();
    let script = driver.dump_script(db).await?;
    let mut total = 0u64;
    if options.data {
        for (table, _) in &script.tables {
            total += driver.count_rows(db, table, &[]).await?;
        }
    }
    let (pending, mut out) = PendingFile::create(path)?;
    writeln!(out, "-- Schemata dump of {db} ({})\n", driver.server_version())?;
    for s in &script.prologue {
        writeln!(out, "{s}")?;
    }
    let mut done = 0u64;
    progress(Progress { done, total: Some(total) });
    let q = quote_for(engine);
    for (table, create) in &script.tables {
        writeln!(out)?;
        if options.structure {
            writeln!(out, "{create}")?;
        }
        if !options.data {
            continue;
        }
        let info = driver.table_info(db, table).await?;
        let mut stream = driver.stream_query(Some(db), &format!("SELECT * FROM {}.{}", q(db), q(table)), BATCH).await?;
        let mut columns = Vec::new();
        loop {
            let item = tokio::select! {
                _ = cancel.cancelled() => return Err(AppError::cancelled()),
                item = stream.recv() => item,
            };
            match item {
                None => break,
                Some(Err(e)) => return Err(e),
                Some(Ok(StreamItem::Columns(mut c))) => {
                    hide_generated(&mut c, &info);
                    columns = c;
                }
                Some(Ok(StreamItem::Rows(rows))) => {
                    insert_statements(&mut out, engine, table, &columns, &rows)?;
                    done += rows.len() as u64;
                    progress(Progress { done, total: Some(total) });
                }
            }
        }
    }
    if options.data {
        for s in &script.data_tail {
            writeln!(out, "{s}")?;
        }
    }
    if options.structure {
        writeln!(out)?;
        for s in &script.objects {
            writeln!(out, "{s}")?;
        }
    }
    writeln!(out)?;
    for s in &script.epilogue {
        writeln!(out, "{s}")?;
    }
    drop(out.into_inner().map_err(|e| AppError::io(e.to_string()))?);
    if cancel.is_cancelled() {
        return Err(AppError::cancelled());
    }
    pending.commit()?;
    Ok(DumpReport { tables: script.tables.len() as u64, rows: done })
}

/// Orders views so that each comes after the views its definition mentions.
pub(crate) fn order_views(mut views: Vec<(String, String)>, mention: impl Fn(&str) -> String) -> Vec<(String, String)> {
    let mut ordered = Vec::with_capacity(views.len());
    while !views.is_empty() {
        let ready = views
            .iter()
            .position(|(name, sql)| !views.iter().any(|(other, _)| other != name && sql.contains(&mention(other))))
            .unwrap_or(0);
        ordered.push(views.remove(ready));
    }
    ordered
}
