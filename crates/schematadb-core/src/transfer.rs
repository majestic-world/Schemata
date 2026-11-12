//! Export, import and SQL-file execution. Rows stream in batches; nothing holds a whole table in memory.

use crate::{AppError, ColumnMeta, Driver, Engine, Result, Splitter, TableInfo, Value};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
pub use tokio_util::sync::CancellationToken;

pub const BATCH: usize = 500;
/// Rows per INSERT statement in SQL exports.
const INSERT_ROWS: usize = 100;
/// Size cap of one INSERT statement in SQL exports (a single larger row still gets its own statement), well below
/// the servers' default `max_allowed_packet`.
const INSERT_BYTES: usize = 1 << 20;

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

/// Writes UTF-8 text through the selected encoding. Windows-1252 output is encoded as it streams: a UTF-8 sequence
/// split across writes is completed by the next write, and a character Windows-1252 lacks fails the export.
struct Encoded<W: Write> {
    out: W,
    encoding: TextEncoding,
    encoder: encoding_rs::Encoder,
    /// Start of a UTF-8 sequence the next write completes.
    pending: Vec<u8>,
    buffer: Vec<u8>,
}

impl<W: Write> Encoded<W> {
    fn new(mut out: W, encoding: TextEncoding) -> Result<Self> {
        if encoding == TextEncoding::Utf8Bom {
            out.write_all(b"\xEF\xBB\xBF")?;
        }
        let buffer = if encoding == TextEncoding::Windows1252 { vec![0; 8192] } else { Vec::new() };
        Ok(Self { out, encoding, encoder: encoding_rs::WINDOWS_1252.new_encoder(), pending: Vec::new(), buffer })
    }

    /// The underlying writer; fails when the text ended inside a UTF-8 sequence.
    fn finish(self) -> std::io::Result<W> {
        if !self.pending.is_empty() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "The exported text ended inside a UTF-8 sequence"));
        }
        Ok(self.out)
    }

    fn encode(&mut self, mut text: &str) -> std::io::Result<()> {
        loop {
            let (result, read, written) = self.encoder.encode_from_utf8_without_replacement(text, &mut self.buffer, false);
            self.out.write_all(&self.buffer[..written])?;
            text = &text[read..];
            match result {
                encoding_rs::EncoderResult::InputEmpty => return Ok(()),
                encoding_rs::EncoderResult::OutputFull => {}
                encoding_rs::EncoderResult::Unmappable(c) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("The character “{c}” (U+{:04X}) cannot be written in Windows-1252; export as UTF-8 instead", c as u32),
                    ));
                }
            }
        }
    }
}

/// Length of the UTF-8 sequence that `lead` starts.
fn utf8_sequence_len(lead: u8) -> usize {
    match lead {
        0xF0.. => 4,
        0xE0.. => 3,
        0xC0.. => 2,
        _ => 1,
    }
}

impl<W: Write> Write for Encoded<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let invalid = |e| std::io::Error::new(std::io::ErrorKind::InvalidData, e);
        match self.encoding {
            TextEncoding::Utf8 | TextEncoding::Utf8Bom => self.out.write_all(buf)?,
            TextEncoding::Windows1252 => {
                let mut input = buf;
                if let Some(&lead) = self.pending.first() {
                    let take = (utf8_sequence_len(lead) - self.pending.len()).min(input.len());
                    self.pending.extend_from_slice(&input[..take]);
                    input = &input[take..];
                    if self.pending.len() < utf8_sequence_len(lead) {
                        return Ok(buf.len());
                    }
                    let pending = std::mem::take(&mut self.pending);
                    self.encode(std::str::from_utf8(&pending).map_err(invalid)?)?;
                }
                let (text, rest) = match std::str::from_utf8(input) {
                    Ok(text) => (text, &[][..]),
                    // The write ends inside a sequence: keep its start for the next write.
                    Err(e) if e.error_len().is_none() => {
                        let (valid, rest) = input.split_at(e.valid_up_to());
                        (std::str::from_utf8(valid).map_err(invalid)?, rest)
                    }
                    Err(e) => return Err(invalid(e)),
                };
                self.encode(text)?;
                self.pending.extend_from_slice(rest);
            }
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.out.flush()
    }
}

/// Output written to a uniquely named temporary file next to the destination and moved into place only on
/// success, so a failed or cancelled export never destroys an existing file. Dropping it removes the file. It holds
/// no open handle, so a per-table dump can keep one for every table until all are written.
struct PendingFile {
    tmp: tempfile::TempPath,
    path: std::path::PathBuf,
}

impl PendingFile {
    fn create(path: &Path) -> Result<(PendingFile, std::io::BufWriter<std::fs::File>)> {
        let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let prefix = format!(".{}.", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        let tmp = tempfile::Builder::new().prefix(&prefix).suffix(".partial").tempfile_in(dir)?;
        let file = tmp.reopen()?;
        Ok((PendingFile { tmp: tmp.into_temp_path(), path: path.to_owned() }, std::io::BufWriter::new(file)))
    }

    fn commit(self) -> Result<()> {
        self.tmp.persist(&self.path).map_err(|e| AppError::io(e.to_string()))?;
        Ok(())
    }
}

/// Reads a SQL text file. It must be UTF-8; a leading byte-order mark is dropped so it never reaches the server.
pub async fn read_sql_text(path: &Path) -> Result<String> {
    let bytes = tokio::fs::read(path).await?;
    let text = String::from_utf8(bytes)
        .map_err(|_| AppError::validation(format!("{} is not a UTF-8 text file; convert it to UTF-8 to open it", path.display())))?;
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
pub(crate) fn hide_generated(columns: &mut [ColumnMeta], info: &TableInfo) {
    for generated in info.columns.iter().filter(|c| c.generated) {
        if let Some(column) = columns.iter_mut().find(|c| c.name == generated.name) {
            column.hidden = true;
        }
    }
}

/// Reads every row of a table. MySQL/MariaDB name each column, because `SELECT *` leaves out INVISIBLE columns.
pub(crate) fn select_all(engine: Engine, db: &str, info: &TableInfo) -> String {
    let q = quote_for(engine);
    match engine {
        Engine::Sqlite => format!("SELECT * FROM {}.{}", q(db), q(&info.name)),
        Engine::MySql | Engine::MariaDb => {
            let columns = info.columns.iter().map(|c| q(&c.name)).collect::<Vec<_>>().join(", ");
            format!("SELECT {columns} FROM {}.{}", q(db), q(&info.name))
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
    let prefix =
        format!("INSERT INTO {} ({}) VALUES\n  ", q(table), visible.iter().map(|i| q(&columns[*i].name)).collect::<Vec<_>>().join(", "));
    let mut statement = String::new();
    let mut count = 0;
    for r in rows {
        let tuple = format!("({})", visible.iter().map(|i| literal(&r[*i], engine)).collect::<Vec<_>>().join(", "));
        if count > 0 && (count == INSERT_ROWS || statement.len() + tuple.len() + 4 > INSERT_BYTES) {
            writeln!(out, "{statement};")?;
            statement.clear();
            count = 0;
        }
        statement.push_str(if count == 0 { &prefix } else { ",\n  " });
        statement.push_str(&tuple);
        count += 1;
    }
    if count > 0 {
        writeln!(out, "{statement};")?;
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
    let (sql, table_info, total, default_name) = match source {
        ExportSource::Table { table } => {
            // SQLite's stream owns the connection until it is drained; metadata must be read first.
            let info = driver.table_info(db, table).await?;
            (select_all(engine, db, &info), Some(info), Some(driver.count_rows(db, table, &[]).await?), table.clone())
        }
        ExportSource::Query { sql } => {
            // Exporting runs the statement again, on another connection: only a statement without side effects may.
            if !crate::is_read_only_query(sql) {
                return Err(AppError::validation(
                    "Only the results of read-only queries (SELECT, WITH, SHOW, …) can be exported, because exporting runs the query again",
                ));
            }
            (sql.trim().trim_end_matches(';').to_owned(), None, None, "query_result".to_owned())
        }
    };
    let mut stream = driver.stream_query(Some(db), &sql, BATCH).await?;
    let mut columns = match stream.recv().await {
        Some(Ok(StreamItem::Columns(c))) => c,
        Some(Err(e)) => return Err(e),
        _ => return Err(AppError::sql("The query did not return a result set")),
    };
    // CSV and JSON keep generated values; SQL INSERTs must leave them for the server to recompute.
    if let (Some(info), ExportFormat::Sql(_)) = (&table_info, format) {
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
            let encoded = w.into_inner().map_err(|e| AppError::io(e.error().to_string()))?;
            drop(encoded.finish()?.into_inner().map_err(|e| AppError::io(e.to_string()))?)
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
    /// Fields equal to this text become NULL, matching [`CsvOptions::null_text`] of an export (`\N` by default),
    /// so NULL and the empty string stay distinct. Empty: no text means NULL.
    pub null_text: String,
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

/// How [`crate::Driver::import_rows`] writes its rows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportOptions {
    /// Report failing rows and keep going instead of rolling everything back at the first one.
    pub skip_errors: bool,
    /// Delete every existing row first, in the same transaction.
    pub empty_first: bool,
    /// TIMESTAMP values are given in UTC (`+00:00`) rather than in the session time zone (MySQL/MariaDB).
    pub utc_timestamps: bool,
}

fn csv_reader(path: &Path, o: &CsvReadOptions) -> Result<csv::Reader<Box<dyn std::io::Read + Send>>> {
    let file = std::fs::File::open(path)?;
    let mut decoder = encoding_rs_io::DecodeReaderBytesBuilder::new();
    match o.encoding {
        // A UTF-8 byte-order mark overrides the selection; other bytes are transcoded.
        TextEncoding::Windows1252 => decoder.encoding(Some(encoding_rs::WINDOWS_1252)).bom_override(true),
        // Bytes pass through unchanged (minus a BOM), so the CSV reader rejects invalid UTF-8 instead of it being
        // replaced by U+FFFD.
        TextEncoding::Utf8 | TextEncoding::Utf8Bom => decoder.encoding(None).utf8_passthru(true).strip_bom(true),
    };
    let reader: Box<dyn std::io::Read + Send> = Box::new(decoder.build(file));
    Ok(csv::ReaderBuilder::new()
        .delimiter(ascii(o.delimiter, "delimiter")?)
        .quote(ascii(o.quote, "quote character")?)
        .has_headers(o.has_header)
        .flexible(true)
        .from_reader(reader))
}

fn csv_error(e: &csv::Error) -> AppError {
    match e.kind() {
        csv::ErrorKind::Utf8 { pos, .. } => AppError::validation(format!(
            "The file is not valid UTF-8{}. If it was saved by Excel or another Windows program, choose the Windows-1252 encoding.",
            pos.as_ref().map(|p| format!(" (line {})", p.line())).unwrap_or_default()
        )),
        _ => AppError::validation(format!("Invalid CSV: {e}")),
    }
}

pub fn preview_csv(path: &Path, options: &CsvReadOptions, limit: usize) -> Result<CsvPreview> {
    let mut reader = csv_reader(path, options)?;
    let headers: Option<Vec<String>> =
        if options.has_header { Some(reader.headers().map_err(|e| csv_error(&e))?.iter().map(str::to_owned).collect()) } else { None };
    let rows: Vec<Vec<String>> = reader
        .records()
        .take(limit)
        .map(|r| r.map(|r| r.iter().map(str::to_owned).collect()).map_err(|e| csv_error(&e)))
        .collect::<Result<_>>()?;
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let headers = headers.unwrap_or_else(|| (1..=width).map(|i| format!("column {i}")).collect());
    Ok(CsvPreview { headers, rows })
}

/// How a CSV field is read for its target column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    Text,
    /// Binary columns take the `0x…` hex an export writes for bytes.
    Binary,
    /// MySQL/MariaDB BIT columns take numbers (and `0x…` hex) as integers, not as the bytes of their text.
    Bit,
}

fn field_kind(engine: Engine, data_type: &str) -> FieldKind {
    let base = data_type.trim().split(['(', ' ']).next().unwrap_or("").to_ascii_lowercase();
    match engine {
        Engine::Sqlite if base.contains("blob") => FieldKind::Binary,
        Engine::Sqlite => FieldKind::Text,
        Engine::MySql | Engine::MariaDb => match base.as_str() {
            "bit" => FieldKind::Bit,
            "binary" | "varbinary" | "tinyblob" | "blob" | "mediumblob" | "longblob" => FieldKind::Binary,
            _ => FieldKind::Text,
        },
    }
}

/// Bytes written as `0x` followed by an even number of hex digits.
fn hex_bytes(s: &str) -> Option<Vec<u8>> {
    let digits = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
    if digits.len() % 2 != 0 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..digits.len()).step_by(2).map(|i| u8::from_str_radix(&digits[i..i + 2], 16).ok()).collect()
}

fn csv_value(field: &str, kind: FieldKind) -> Value {
    match kind {
        FieldKind::Text => Value::Text(field.to_owned()),
        FieldKind::Binary => hex_bytes(field).map_or_else(|| Value::Text(field.to_owned()), Value::Bytes),
        FieldKind::Bit => match field.trim().parse::<u64>() {
            Ok(n) => Value::UInt(n),
            Err(_) => match hex_bytes(field.trim()).filter(|b| b.len() <= 8) {
                Some(bytes) => Value::UInt(bytes.iter().fold(0, |n, b| (n << 8) | u64::from(*b))),
                None => Value::Text(field.to_owned()),
            },
        },
    }
}

/// Rows the CSV reader rejected itself (wrong number of fields), merged into the driver's report.
#[derive(Default)]
struct Rejected {
    /// Without `skip_errors`: the row that stopped the import.
    fatal: Option<RowFailure>,
    failed: Vec<RowFailure>,
    count: u64,
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
    let info = driver.table_info(db, table).await?;
    let engine = driver.engine();
    let kinds: Vec<FieldKind> = spec
        .mapping
        .iter()
        .map(|m| info.columns.iter().find(|c| c.name == m.target).map_or(FieldKind::Text, |c| field_kind(engine, &c.data_type)))
        .collect();
    let total = {
        let mut r = csv_reader(path, &spec.options)?;
        Some(r.records().count() as u64)
    };
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<ImportBatch>>(4);
    let reader = csv_reader(path, &spec.options)?;
    let spec_c = spec.clone();
    let cancel_c = cancel.clone();
    let rejected = Arc::new(Mutex::new(Rejected::default()));
    let rejected_c = rejected.clone();
    let producer = tokio::task::spawn_blocking(move || {
        let mut reader = reader;
        let mut batch: ImportBatch = Vec::with_capacity(BATCH);
        let mut done = 0u64;
        // Every row must have as many fields as the header (or, without one, the first row): a shorter or longer
        // row would put its values under the wrong columns.
        let mut width = None;
        if spec_c.options.has_header {
            match reader.headers() {
                Ok(headers) => width = Some(headers.len()),
                Err(e) => {
                    let _ = tx.blocking_send(Err(csv_error(&e)));
                    return;
                }
            }
        }
        for record in reader.records() {
            if cancel_c.is_cancelled() {
                let _ = tx.blocking_send(Err(AppError::cancelled()));
                return;
            }
            let record = match record {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.blocking_send(Err(csv_error(&e)));
                    return;
                }
            };
            let line = record.position().map(|p| p.line()).unwrap_or(0);
            let expected = *width.get_or_insert(record.len());
            if record.len() != expected {
                let failure = RowFailure { line, message: format!("Expected {expected} fields but found {}", record.len()) };
                if !spec_c.skip_errors {
                    // Rows before it go first, so a failure among them is still the one reported.
                    let message = failure.message.clone();
                    rejected_c.lock().unwrap().fatal = Some(failure);
                    if !batch.is_empty() && tx.blocking_send(Ok(std::mem::take(&mut batch))).is_err() {
                        return;
                    }
                    let _ = tx.blocking_send(Err(AppError::validation(message)));
                    return;
                }
                let mut rejected = rejected_c.lock().unwrap();
                rejected.count += 1;
                if rejected.failed.len() < MAX_REPORTED_FAILURES {
                    rejected.failed.push(failure);
                }
                continue;
            }
            let values = spec_c
                .mapping
                .iter()
                .zip(&kinds)
                .map(|(m, kind)| match record.get(m.source) {
                    Some("") if spec_c.empty_as_null => Value::Null,
                    Some(s) if !spec_c.null_text.is_empty() && s == spec_c.null_text => Value::Null,
                    Some(s) => csv_value(s, *kind),
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
    let options = ImportOptions { skip_errors: spec.skip_errors, ..ImportOptions::default() };
    let report = driver.import_rows(db, table, &columns, rx, options).await;
    producer.await.map_err(|e| AppError::io(e.to_string()))?;
    let rejected = std::mem::take(&mut *rejected.lock().unwrap());
    match report {
        // The import was rolled back at the row the reader rejected.
        Err(e) => match rejected.fatal {
            Some(failure) if e.message == failure.message => Ok(ImportReport { inserted: 0, failed: vec![failure], failed_count: 1 }),
            _ => Err(e),
        },
        Ok(mut report) => {
            if rejected.count > 0 {
                report.failed_count += rejected.count;
                report.failed.extend(rejected.failed);
                report.failed.sort_by_key(|f| f.line);
                report.failed.truncate(MAX_REPORTED_FAILURES);
            }
            Ok(report)
        }
    }
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
    /// The script left a transaction open; it was rolled back, discarding its uncommitted writes.
    pub open_transaction_rolled_back: bool,
}

pub(crate) fn failure(index: usize, statement: &str, message: String) -> StatementFailure {
    let statement =
        if statement.len() > 300 { format!("{}…", &statement[..statement.floor_char_boundary(300)]) } else { statement.to_owned() };
    StatementFailure { index, statement, message }
}

/// A `.sql` file read line by line in the selected encoding. A UTF-8 byte-order mark is dropped, and wins over a
/// Windows-1252 selection because it proves the file is UTF-8.
struct SqlFileLines {
    input: std::io::BufReader<std::fs::File>,
    windows_1252: bool,
    line: Vec<u8>,
    number: u64,
    /// Bytes read so far.
    read: u64,
    /// File size.
    total: u64,
}

impl SqlFileLines {
    fn open(path: &Path, encoding: TextEncoding) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        let total = file.metadata()?.len();
        let mut input = std::io::BufReader::new(file);
        let bom = input.fill_buf()?.starts_with(b"\xEF\xBB\xBF");
        if bom {
            input.consume(3);
        }
        let windows_1252 = encoding == TextEncoding::Windows1252 && !bom;
        Ok(Self { input, windows_1252, line: Vec::new(), number: 0, read: if bom { 3 } else { 0 }, total })
    }

    fn next_line(&mut self) -> Result<Option<Cow<'_, str>>> {
        self.line.clear();
        let n = self.input.read_until(b'\n', &mut self.line)?;
        if n == 0 {
            return Ok(None);
        }
        self.number += 1;
        self.read += n as u64;
        if self.windows_1252 {
            return Ok(Some(encoding_rs::WINDOWS_1252.decode_without_bom_handling(&self.line).0));
        }
        let number = self.number;
        match std::str::from_utf8(&self.line) {
            Ok(text) => Ok(Some(Cow::Borrowed(text))),
            Err(_) => Err(AppError::validation(format!(
                "The SQL file is not valid UTF-8 (line {number}); nothing was run. If it was saved by a Windows program, choose the Windows-1252 encoding."
            ))),
        }
    }

    /// Decodes the whole file without using it, so a decoding error is found before anything runs.
    fn check(path: &Path, encoding: TextEncoding, cancel: &CancellationToken) -> Result<()> {
        let mut lines = Self::open(path, encoding)?;
        if lines.windows_1252 {
            // Every byte is a Windows-1252 character.
            return Ok(());
        }
        while lines.next_line()?.is_some() {
            if cancel.is_cancelled() {
                return Err(AppError::cancelled());
            }
        }
        Ok(())
    }
}

/// Runs every statement of a `.sql` file on one session, outside a transaction (dumps manage their own).
/// The whole file is decoded first, so a decoding error never leaves the statements before it applied; it is then
/// read and split incrementally, and progress is reported in bytes read.
pub async fn run_sql_file(
    driver: &dyn Driver,
    db: &str,
    path: &Path,
    encoding: TextEncoding,
    stop_on_error: bool,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<ScriptRunReport> {
    {
        let (path, cancel) = (path.to_owned(), cancel.clone());
        tokio::task::spawn_blocking(move || SqlFileLines::check(&path, encoding, &cancel))
            .await
            .map_err(|e| AppError::io(e.to_string()))??;
    }
    let mut lines = SqlFileLines::open(path, encoding)?;
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<String>>(64);
    let engine = driver.engine();
    let cancel_reader = cancel.clone();
    let reader = tokio::task::spawn_blocking(move || {
        let mut run = || -> Result<()> {
            let mut splitter = Splitter::new(engine);
            while !cancel_reader.is_cancelled() {
                let statements = match lines.next_line()? {
                    Some(text) => splitter.feed(&text),
                    None => break,
                };
                for statement in statements {
                    if tx.blocking_send(Ok(statement)).is_err() {
                        return Ok(());
                    }
                }
                progress(Progress { done: lines.read, total: Some(lines.total) });
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

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlFileScan {
    /// Databases the file switches to (`USE`) or creates (`CREATE DATABASE`/`CREATE SCHEMA`), in order of first
    /// appearance. Its statements after a `USE` run in that database, not in the one it is restored into.
    pub databases: Vec<String>,
}

/// The database a `USE` or `CREATE DATABASE`/`CREATE SCHEMA` statement names.
fn named_database(statement: &str, engine: Engine) -> Option<String> {
    let mut tokens = crate::sql_tokens::Tokens::new(statement, engine);
    let first = tokens.next()?;
    let mut name = if first.is_keyword(statement, "USE") {
        tokens.next()?
    } else if first.is_keyword(statement, "CREATE") {
        let kind = tokens.next()?;
        if !(kind.is_keyword(statement, "DATABASE") || kind.is_keyword(statement, "SCHEMA")) {
            return None;
        }
        tokens.next()?
    } else {
        return None;
    };
    if name.is_keyword(statement, "IF") {
        // IF NOT EXISTS
        name = tokens.nth(2)?;
    }
    name.name(statement).map(Cow::into_owned)
}

/// Checks that a `.sql` file decodes in `encoding` and lists the databases it switches to or creates, so a restore can
/// warn before the file's statements go somewhere other than the selected database.
pub async fn scan_sql_file(path: &Path, encoding: TextEncoding, engine: Engine) -> Result<SqlFileScan> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || -> Result<SqlFileScan> {
        let mut lines = SqlFileLines::open(&path, encoding)?;
        let mut splitter = Splitter::new(engine);
        let mut scan = SqlFileScan::default();
        let mut note = |statement: &str| {
            if let Some(name) = named_database(statement, engine).filter(|n| !scan.databases.contains(n)) {
                scan.databases.push(name);
            }
        };
        while let Some(text) = lines.next_line()? {
            splitter.feed(&text).iter().for_each(|s| note(s));
        }
        splitter.finish().iter().for_each(|s| note(s));
        Ok(scan)
    })
    .await
    .map_err(|e| AppError::io(e.to_string()))?
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

/// A complete SQL chunk of a dump. `table` names the table it belongs to (its indexes, triggers and autoincrement
/// counter), so a per-table dump writes it into that table's file.
pub struct DumpObject {
    pub sql: String,
    pub table: Option<String>,
}

/// Engine-specific parts of a dump, each item a complete SQL chunk (statement terminators included).
pub struct DumpScript {
    /// Session setup, written first.
    pub prologue: Vec<String>,
    /// Tables in a creation-safe order with their `DROP ...; CREATE ...;` text.
    pub tables: Vec<(String, String)>,
    /// Statements that depend on the data being present (e.g. SQLite's autoincrement counters).
    pub data_tail: Vec<DumpObject>,
    /// Indexes, views and triggers, in dependency order.
    pub objects: Vec<DumpObject>,
    pub epilogue: Vec<String>,
}

/// Opens the MySQL/MariaDB session `key` for a series of reads: TIMESTAMPs come back in UTC and, with `snapshot`, every
/// read sees the same consistent snapshot of InnoDB tables (as `mysqldump --single-transaction`). The caller closes it
/// with [`Driver::close_editor_session`].
pub(crate) async fn open_read_session(driver: &dyn Driver, key: &str, db: &str, snapshot: bool) -> Result<()> {
    let mut sql = "SET time_zone = '+00:00'".to_owned();
    if snapshot {
        sql += ";\nSET SESSION TRANSACTION ISOLATION LEVEL REPEATABLE READ;\nSTART TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY";
    }
    let result = driver.execute_in_session(key, Some(db), &sql, 1, None).await?;
    match result.error {
        Some(error) => Err(error.error),
        None => Ok(()),
    }
}

/// Where a dump goes: one file, or a folder that gets one file per table.
#[derive(Clone, Copy)]
enum DumpTarget<'a> {
    File(&'a Path),
    Folder(&'a Path),
}

/// Writes a `.sql` dump of `db` that the engine's own CLI (or [`run_sql_file`]) can restore. On MySQL/MariaDB every
/// table is read in one session, inside one consistent snapshot and with TIMESTAMPs in UTC (the dump's prologue sets
/// `time_zone = '+00:00'` to match).
pub async fn dump_database(
    driver: &dyn Driver,
    db: &str,
    path: &Path,
    options: &DumpOptions,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<DumpReport> {
    dump(driver, db, DumpTarget::File(path), options, progress, cancel).await
}

/// Writes one restorable `.sql` file per table of `db` into `dir`, named after the table:
/// its `DROP`/`CREATE`, rows, indexes and triggers. Views, routines and events are left out. Tables are read as in
/// [`dump_database`], and files replace same-named ones only after every table is written, so a failed or cancelled
/// read leaves the folder as it was; a failure while moving the files into place can leave earlier ones replaced.
pub async fn dump_tables(
    driver: &dyn Driver,
    db: &str,
    dir: &Path,
    options: &DumpOptions,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<DumpReport> {
    dump(driver, db, DumpTarget::Folder(dir), options, progress, cancel).await
}

async fn dump(
    driver: &dyn Driver,
    db: &str,
    target: DumpTarget<'_>,
    options: &DumpOptions,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<DumpReport> {
    if !options.structure && !options.data {
        return Err(AppError::validation("Choose structure, data or both"));
    }
    if driver.engine() == Engine::Sqlite || !options.data {
        return write_dump(driver, db, target, options, None, progress, cancel).await;
    }
    let session = format!("schemata-dump-{}", uuid::Uuid::new_v4());
    let dumped = async {
        open_read_session(driver, &session, db, true).await?;
        write_dump(driver, db, target, options, Some(&session), progress, cancel).await
    }
    .await;
    // Ends the snapshot's transaction, also after an error or a cancel.
    let _ = driver.close_editor_session(&session).await;
    dumped
}

async fn write_dump(
    driver: &dyn Driver,
    db: &str,
    target: DumpTarget<'_>,
    options: &DumpOptions,
    session: Option<&str>,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<DumpReport> {
    let engine = driver.engine();
    let script = driver.dump_script(db).await?;
    let mut total = 0u64;
    if options.data {
        for (table, _) in &script.tables {
            total += driver.count_rows(db, table, &[]).await?;
        }
    }
    let mut rows = RowWriter { driver, db, session, cancel: &cancel, progress: &progress, done: 0, total };
    progress(Progress { done: 0, total: Some(total) });
    let open = |path: &Path, title: &str| -> Result<(PendingFile, std::io::BufWriter<std::fs::File>)> {
        let (pending, mut out) = PendingFile::create(path)?;
        writeln!(out, "-- Schemata dump of {title} ({})\n", driver.server_version())?;
        for s in &script.prologue {
            writeln!(out, "{s}")?;
        }
        Ok((pending, out))
    };
    let close = |mut out: std::io::BufWriter<std::fs::File>| -> Result<()> {
        writeln!(out)?;
        for s in &script.epilogue {
            writeln!(out, "{s}")?;
        }
        drop(out.into_inner().map_err(|e| AppError::io(e.to_string()))?);
        Ok(())
    };
    let files = match target {
        DumpTarget::File(path) => {
            let (pending, mut out) = open(path, db)?;
            write_tables(&mut out, &script.tables, &script, options, &mut rows, |_| true).await?;
            close(out)?;
            vec![pending]
        }
        DumpTarget::Folder(dir) => {
            let names = table_file_names(script.tables.iter().map(|(table, _)| table.as_str()));
            let mut files = Vec::with_capacity(script.tables.len());
            for (entry, name) in script.tables.iter().zip(names) {
                let table = &entry.0;
                // SQLite matches identifiers case-insensitively: a trigger keeps its table's name as the CREATE wrote it.
                let owned = |o: &DumpObject| {
                    o.table.as_deref().is_some_and(|t| if engine == Engine::Sqlite { t.eq_ignore_ascii_case(table) } else { t == table })
                };
                let (pending, mut out) = open(&dir.join(name), &format!("{db}.{table}"))?;
                write_tables(&mut out, std::slice::from_ref(entry), &script, options, &mut rows, owned).await?;
                close(out)?;
                files.push(pending);
            }
            files
        }
    };
    if cancel.is_cancelled() {
        return Err(AppError::cancelled());
    }
    for file in files {
        file.commit()?;
    }
    Ok(DumpReport { tables: script.tables.len() as u64, rows: rows.done })
}

/// Writes `tables` (structure and rows, as `options` asks), then the dump's data tail and objects `keep` accepts.
async fn write_tables(
    out: &mut std::io::BufWriter<std::fs::File>,
    tables: &[(String, String)],
    script: &DumpScript,
    options: &DumpOptions,
    rows: &mut RowWriter<'_>,
    keep: impl Fn(&DumpObject) -> bool,
) -> Result<()> {
    for (table, create) in tables {
        writeln!(out)?;
        if options.structure {
            writeln!(out, "{create}")?;
        }
        if options.data {
            rows.write(out, table).await?;
        }
    }
    if options.data {
        for s in script.data_tail.iter().filter(|o| keep(o)) {
            writeln!(out, "{}", s.sql)?;
        }
    }
    if options.structure {
        writeln!(out)?;
        for s in script.objects.iter().filter(|o| keep(o)) {
            writeln!(out, "{}", s.sql)?;
        }
    }
    Ok(())
}

/// Streams tables' rows into a dump as INSERT statements, counting them for progress.
struct RowWriter<'a> {
    driver: &'a dyn Driver,
    db: &'a str,
    session: Option<&'a str>,
    cancel: &'a CancellationToken,
    progress: &'a ProgressFn,
    done: u64,
    total: u64,
}

impl RowWriter<'_> {
    async fn write(&mut self, out: &mut impl Write, table: &str) -> Result<()> {
        let engine = self.driver.engine();
        let info = self.driver.table_info(self.db, table).await?;
        let sql = select_all(engine, self.db, &info);
        let mut stream = match self.session {
            Some(session) => self.driver.stream_query_in_session(session, Some(self.db), &sql, BATCH).await?,
            None => self.driver.stream_query(Some(self.db), &sql, BATCH).await?,
        };
        let mut columns = Vec::new();
        loop {
            let item = tokio::select! {
                _ = self.cancel.cancelled() => return Err(AppError::cancelled()),
                item = stream.recv() => item,
            };
            match item {
                None => return Ok(()),
                Some(Err(e)) => return Err(e),
                Some(Ok(StreamItem::Columns(mut c))) => {
                    hide_generated(&mut c, &info);
                    columns = c;
                }
                Some(Ok(StreamItem::Rows(rows))) => {
                    insert_statements(out, engine, table, &columns, &rows)?;
                    self.done += rows.len() as u64;
                    (self.progress)(Progress { done: self.done, total: Some(self.total) });
                }
            }
        }
    }
}

/// Windows device names, which no file may use as its name (with or without an extension).
const RESERVED_FILE_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4",
    "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// `<table>.sql` file names valid on every OS: characters Windows forbids become `_`, trailing dots and spaces and
/// device names (`CON`, `NUL`, …) get a `_` suffix, and names equal ignoring case (distinct tables on a
/// case-sensitive server, one file on Windows and macOS) are numbered `name (2).sql`.
fn table_file_names<'a>(tables: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut used = std::collections::HashSet::new();
    tables
        .map(|table| {
            let mut stem: String = table.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
            let base = stem.split('.').next().unwrap_or_default().to_ascii_uppercase();
            if stem.is_empty() || stem.ends_with(['.', ' ']) || RESERVED_FILE_NAMES.contains(&base.trim_end()) {
                stem.push('_');
            }
            let mut name = format!("{stem}.sql");
            let mut n = 2;
            while !used.insert(name.to_lowercase()) {
                name = format!("{stem} ({n}).sql");
                n += 1;
            }
            name
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::table_file_names;

    #[test]
    fn table_file_names_are_valid_and_unique_on_every_os() {
        let names = table_file_names(["orders", "Orders", "a/b:c?", "con", "nul.log", "dots..", "", "orders"].into_iter());
        assert_eq!(names, ["orders.sql", "Orders (2).sql", "a_b_c_.sql", "con_.sql", "nul.log_.sql", "dots.._.sql", "_.sql", "orders (3).sql"]);
    }
}
