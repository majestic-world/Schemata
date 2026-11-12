use crate::design::sqlite::{FK_OFF, FK_ON};
use crate::model::derive_identity;
use crate::transfer::{
    DumpObject, DumpScript, ImportBatch, ImportOptions, ImportReport, MAX_REPORTED_FAILURES, RowFailure, RowStream, ScriptRunReport, StreamItem,
};
use crate::{
    AppError, CellValue, ChangeError, ColumnInfo, ColumnMeta, CreateDatabase, DatabaseInfo, Driver, Engine, ErrorKind, Filter,
    ForeignKeyInfo, IncomingForeignKey, IndexInfo, IndexKind, ObjectInfo, ObjectKind, Page, PageRequest, ROWID_COLUMN, Result, RowChange,
    RowIdentity, ScriptResult, StatementError, StatementOutcome, TableDesign, TableInfo, Value, split_statements,
};
use async_trait::async_trait;
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OpenFlags, OptionalExtension, types::ValueRef};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// How long a statement waits for a lock another connection holds before failing with [`LOCKED`].
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const LOCKED: &str = "The database is locked by another connection; try again once its transaction ends";
const LOCKED_BY_QUERY_TAB: &str =
    "The database is locked by an open transaction in a query tab; commit or roll it back there, then try again";

pub struct SqliteDriver {
    /// The app's own connection: grid saves, DDL, imports and object actions. Query tabs never run on it (unless the
    /// database cannot be opened twice), so their open transactions cannot capture or roll back app operations.
    conn: Arc<Mutex<Connection>>,
    /// What the main database was opened with; `None` when a second connection would not see the same database
    /// (in-memory or temporary), so query-tab sessions share the main connection.
    path: Option<String>,
    /// Databases attached through [`Driver::attach_database`] (alias, file), attached again on every session connection.
    attached: Arc<Mutex<Vec<(String, String)>>>,
    sessions: Mutex<HashMap<String, Arc<EditorSession>>>,
    /// Sessions of a replaced driver whose next run must fail once (see [`Driver::adopt_lost_editor_sessions`]).
    lost_sessions: Mutex<HashSet<String>>,
    executions: Executions,
    version: String,
}

/// A query tab's own connection to the same database, opened on first use.
struct EditorSession {
    path: String,
    conn: Mutex<Option<Connection>>,
    /// Whether the connection was inside a transaction when its last run ended; names the cause of lock timeouts.
    in_transaction: AtomicBool,
}

impl EditorSession {
    /// Runs `f` on the session's connection, opening it on first use. Databases attached to the app connection since
    /// the last run are attached here too (SQLite refuses ATTACH inside a transaction, so only between transactions).
    fn run<T>(&self, attached: &Mutex<Vec<(String, String)>>, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let mut guard = self.conn.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.is_none() {
            *guard = Some(open_connection(&self.path, OpenFlags::empty())?);
        }
        let conn = guard.as_ref().expect("opened above");
        if conn.is_autocommit() {
            let attached = attached.lock().unwrap_or_else(PoisonError::into_inner).clone();
            attach_missing(conn, &attached)?;
        }
        let result = f(conn);
        self.in_transaction.store(!conn.is_autocommit(), Ordering::Relaxed);
        result
    }
}

/// Runs registered under an execution id, so [`Driver::cancel`] reaches the connection running them.
type Executions = Arc<Mutex<HashMap<String, Execution>>>;

struct Execution {
    interrupt: rusqlite::InterruptHandle,
    /// Checked between statements: interrupting only stops a statement that is running.
    cancelled: Arc<AtomicBool>,
}

pub(crate) fn map_err(e: rusqlite::Error) -> AppError {
    match &e {
        rusqlite::Error::SqliteFailure(f, msg) => {
            let message = msg.clone().unwrap_or_else(|| e.to_string());
            match f.code {
                rusqlite::ErrorCode::OperationInterrupted => AppError::cancelled(),
                rusqlite::ErrorCode::CannotOpen | rusqlite::ErrorCode::NotADatabase => AppError::new(ErrorKind::Connection, message),
                rusqlite::ErrorCode::DatabaseBusy => AppError::sql(LOCKED),
                _ => AppError::sql(message),
            }
        }
        _ => AppError::sql(e.to_string()),
    }
}

pub(crate) fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// TEXT that is not valid UTF-8 is returned as its bytes: a lossy decode would be written back (or dumped) mangled.
pub(crate) fn read_value(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Int(i),
        ValueRef::Real(f) => Value::Float(f),
        ValueRef::Text(t) => match std::str::from_utf8(t) {
            Ok(text) => Value::Text(text.to_owned()),
            Err(_) => Value::Bytes(t.to_vec()),
        },
        ValueRef::Blob(b) => Value::Bytes(b.to_vec()),
    }
}

/// Opens `path` read-write with lock waits and foreign-key enforcement, the settings every connection shares.
fn open_connection(path: &str, extra: OpenFlags) -> Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI | extra;
    let conn = Connection::open_with_flags(path, flags).map_err(|e| AppError::connection(e.to_string()))?;
    conn.busy_timeout(BUSY_TIMEOUT).map_err(map_err)?;
    conn.execute_batch(FK_ON).map_err(map_err)?;
    Ok(conn)
}

fn attach_missing(c: &Connection, attached: &[(String, String)]) -> Result<()> {
    let mut stmt = c.prepare("SELECT name FROM pragma_database_list").map_err(map_err)?;
    let present = collect(&mut stmt, [], |r| r.get::<_, String>(0))?;
    for (alias, file) in attached {
        if !present.iter().any(|name| name.eq_ignore_ascii_case(alias)) {
            c.execute(&format!("ATTACH DATABASE ?1 AS {}", quote_ident(alias)), [file]).map_err(map_err)?;
        }
    }
    Ok(())
}

/// Runs a script statement by statement under `execution_id`, stopping at the first error.
fn run_script(c: &Connection, executions: &Executions, execution_id: Option<&str>, sql: &str, row_limit: usize) -> ScriptResult {
    let cancelled = Arc::new(AtomicBool::new(false));
    if let Some(id) = execution_id {
        let execution = Execution { interrupt: c.get_interrupt_handle(), cancelled: cancelled.clone() };
        executions.lock().unwrap_or_else(PoisonError::into_inner).insert(id.to_owned(), execution);
    }
    let mut result = ScriptResult { outcomes: Vec::new(), error: None };
    for (index, statement) in split_statements(sql, Engine::Sqlite).into_iter().enumerate() {
        let outcome = if cancelled.load(Ordering::Relaxed) { Err(AppError::cancelled()) } else { run_statement(c, &statement, row_limit) };
        match outcome {
            Ok(outcome) => result.outcomes.push(outcome),
            Err(error) => {
                result.error = Some(StatementError { index, statement, error });
                break;
            }
        }
    }
    if let Some(id) = execution_id {
        executions.lock().unwrap_or_else(PoisonError::into_inner).remove(id);
    }
    result
}

/// Rows `offset..offset+limit` of a read-only statement; anything else would repeat its side effects.
fn read_more(c: &Connection, statement: &str, offset: u64, limit: u64) -> Result<Page> {
    if !crate::is_read_only_query(statement) {
        return Err(AppError::validation("Only read-only queries can load more rows; run the statement again to see more"));
    }
    let mut stmt = c.prepare(&crate::sql::more_rows(statement)).map_err(map_err)?;
    let columns: Vec<ColumnMeta> = stmt
        .columns()
        .map(|col| ColumnMeta { name: col.name().to_owned(), type_name: col.decl_type().unwrap_or("").to_owned(), hidden: false })
        .collect();
    let n = columns.len();
    let rows =
        collect(&mut stmt, [limit as i64, offset as i64], |r| Ok((0..n).map(|i| read_value(r.get_ref_unwrap(i)).to_cell()).collect()))?;
    Ok(Page { columns, rows })
}

/// Streams the rows of `sql` into `tx`: the column list first, then batches of `batch` rows.
fn stream_rows(c: &Connection, sql: &str, batch: usize, tx: &tokio::sync::mpsc::Sender<Result<StreamItem>>) -> Result<()> {
    let mut stmt = c.prepare(sql).map_err(map_err)?;
    let columns: Vec<ColumnMeta> = stmt
        .columns()
        .map(|c| ColumnMeta { name: c.name().to_owned(), type_name: c.decl_type().unwrap_or("").to_owned(), hidden: false })
        .collect();
    let n = columns.len();
    if tx.blocking_send(Ok(StreamItem::Columns(columns))).is_err() {
        return Ok(());
    }
    let mut rows = stmt.query([]).map_err(map_err)?;
    let mut buf = Vec::with_capacity(batch);
    while let Some(r) = rows.next().map_err(map_err)? {
        buf.push((0..n).map(|i| read_value(r.get_ref_unwrap(i))).collect());
        if buf.len() == batch && tx.blocking_send(Ok(StreamItem::Rows(std::mem::take(&mut buf)))).is_err() {
            return Ok(());
        }
    }
    if !buf.is_empty() {
        let _ = tx.blocking_send(Ok(StreamItem::Rows(buf)));
    }
    Ok(())
}

impl SqliteDriver {
    pub async fn open(path: &str, create: bool) -> Result<Self> {
        let path = path.to_owned();
        tokio::task::spawn_blocking(move || {
            if !create && !Path::new(&path).exists() {
                return Err(AppError::connection(format!("Database file not found: {path}")));
            }
            let conn = open_connection(&path, if create { OpenFlags::SQLITE_OPEN_CREATE } else { OpenFlags::empty() })?;
            // Validates that the file really is a SQLite database.
            conn.query_row("SELECT count(*) FROM sqlite_schema", [], |_| Ok(()))
                .map_err(|e| AppError::connection(format!("Not a SQLite database: {e}")))?;
            let file: String =
                conn.query_row("SELECT file FROM pragma_database_list WHERE name = 'main'", [], |r| r.get(0)).map_err(map_err)?;
            Ok(SqliteDriver {
                conn: Arc::new(Mutex::new(conn)),
                path: (!file.is_empty()).then_some(path),
                attached: Arc::default(),
                sessions: Mutex::default(),
                lost_sessions: Mutex::default(),
                executions: Arc::default(),
                version: rusqlite::version().to_owned(),
            })
        })
        .await
        .map_err(|e| AppError::io(e.to_string()))?
    }

    /// Runs `f` on the main connection in the blocking pool.
    pub(crate) async fn with_conn<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = conn.lock().unwrap_or_else(PoisonError::into_inner);
            f(&mut guard)
        })
        .await
        .map_err(|e| AppError::io(e.to_string()))?
        .map_err(|e| self.explain_lock(e, None))
    }

    /// The connection of query-tab session `key`; `None` when sessions share the main connection (see `path`).
    fn session(&self, key: &str) -> Result<Option<Arc<EditorSession>>> {
        if self.lost_sessions.lock().unwrap_or_else(PoisonError::into_inner).remove(key) {
            return Err(AppError::connection(
                "The connection was reopened, so this query tab's session was closed and any open transaction rolled back. Run again to start a new session.",
            ));
        }
        let Some(path) = &self.path else { return Ok(None) };
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        let session = sessions.entry(key.to_owned()).or_insert_with(|| {
            Arc::new(EditorSession { path: path.clone(), conn: Mutex::new(None), in_transaction: AtomicBool::new(false) })
        });
        Ok(Some(session.clone()))
    }

    /// Runs `f` on session `session`'s connection in the blocking pool.
    async fn on_session<T, F>(&self, session: Arc<EditorSession>, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    {
        let attached = self.attached.clone();
        tokio::task::spawn_blocking(move || session.run(&attached, f)).await.map_err(|e| AppError::io(e.to_string()))?
    }

    /// A lock timeout names its likely cause when a query tab other than `except` holds an open transaction.
    fn explain_lock(&self, error: AppError, except: Option<&str>) -> AppError {
        if error.message != LOCKED {
            return error;
        }
        let sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        if sessions.iter().any(|(key, session)| Some(key.as_str()) != except && session.in_transaction.load(Ordering::Relaxed)) {
            AppError::sql(LOCKED_BY_QUERY_TAB)
        } else {
            error
        }
    }
}

fn collect<T>(
    stmt: &mut rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
    f: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>> {
    stmt.query_map(params, f).map_err(map_err)?.collect::<rusqlite::Result<Vec<T>>>().map_err(map_err)
}

pub(crate) fn to_sql(v: &Value) -> SqlValue {
    match v {
        Value::Null => SqlValue::Null,
        Value::Int(i) => SqlValue::Integer(*i),
        Value::UInt(u) => i64::try_from(*u).map(SqlValue::Integer).unwrap_or_else(|_| SqlValue::Text(u.to_string())),
        Value::Float(f) => SqlValue::Real(*f),
        Value::Decimal(s) | Value::Text(s) | Value::DateTime(s) | Value::Json(s) => SqlValue::Text(s.clone()),
        Value::Bytes(b) => SqlValue::Blob(b.clone()),
    }
}

/// Whether `db.name` is a `WITHOUT ROWID` table, as SQLite itself reports it (the CREATE text may mention the words
/// in a default value or comment).
fn without_rowid(c: &Connection, db: &str, name: &str) -> Result<bool> {
    c.query_row("SELECT wr FROM pragma_table_list(?1) WHERE schema = ?2", [name, db], |r| r.get::<_, bool>(0))
        .optional()
        .map(|wr| wr.unwrap_or(false))
        .map_err(map_err)
}

/// `pragma table_xinfo` reports `DEFAULT (expr)` without its parentheses; they are put back around anything that is
/// not a plain literal, so the default is valid in a `DEFAULT` clause again (rebuilds and copies emit it as is).
fn default_clause(raw: String) -> String {
    let text = raw.trim();
    let upper = text.to_ascii_uppercase();
    let keyword = ["NULL", "TRUE", "FALSE", "CURRENT_TIME", "CURRENT_DATE", "CURRENT_TIMESTAMP"].contains(&upper.as_str());
    // A bare identifier is SQLite's legacy spelling of a string default (`DEFAULT abc` stores 'abc').
    let identifier = text.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_')
        && text.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_');
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    let number = unsigned.starts_with(|ch: char| ch.is_ascii_digit() || ch == '.')
        && (unsigned.parse::<f64>().is_ok()
            || unsigned.get(..2).is_some_and(|p| p.eq_ignore_ascii_case("0x"))
                && unsigned.len() > 2
                && unsigned[2..].chars().all(|ch| ch.is_ascii_hexdigit()));
    let blob = text.get(..2).is_some_and(|p| p.eq_ignore_ascii_case("x'")) && single_quoted(&text[1..]);
    if keyword || identifier || number || single_quoted(text) || blob { raw } else { format!("({text})") }
}

/// Whether `text` is exactly one `'…'` string literal (`''` escapes a quote).
fn single_quoted(text: &str) -> bool {
    let Some(inner) = text.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')) else { return false };
    !inner.replace("''", "").contains('\'')
}

/// Whether a declared type gives a column no type affinity, so SQLite stores values exactly as bound
/// (https://www.sqlite.org/datatype3.html#determination_of_column_affinity; `ANY` in STRICT tables).
fn without_affinity(data_type: &str) -> bool {
    let t = data_type.trim().to_ascii_uppercase();
    !t.contains("INT") && !["CHAR", "CLOB", "TEXT"].iter().any(|k| t.contains(k)) && (t.is_empty() || t.contains("BLOB") || t == "ANY")
}

/// The number a grid text value spells canonically (`43`, `-1.5`; not `007` or `1e3`), so a column without affinity
/// keeps storing numbers as numbers when they are edited as text.
fn canonical_number(text: &str) -> Option<CellValue> {
    if let Ok(i) = text.parse::<i64>() {
        return (i.to_string() == text).then_some(CellValue::Int(i));
    }
    let f = text.parse::<f64>().ok().filter(|f| f.is_finite())?;
    (f.to_string() == text).then_some(CellValue::Float(f))
}

/// The grid sends edits as text. Columns without affinity would store that text as is, turning an edited 42 into
/// '43' (which no longer compares equal to 43), so canonical numbers are written as numbers there.
fn keep_numbers_numeric(changes: &mut [RowChange], columns: &[ColumnInfo]) {
    for change in changes {
        let (RowChange::Update { values, .. } | RowChange::Insert { values }) = change else { continue };
        for value in values {
            let untyped = columns.iter().any(|c| c.name == value.column && without_affinity(&c.data_type));
            if untyped
                && let CellValue::Text(text) = &value.value
                && let Some(number) = canonical_number(text)
            {
                value.value = number;
            }
        }
    }
}

fn read_table_info(c: &Connection, db: &str, name: &str) -> Result<TableInfo> {
    let mut info = read_structure(c, db, name)?;
    if info.kind == ObjectKind::Table {
        let sql = format!("SELECT count(*) FROM {}.{}", quote_ident(db), quote_ident(name));
        info.rows = Some(c.query_row(&sql, [], |r| r.get(0)).map_err(map_err)?);
    }
    Ok(info)
}

/// SQLite exposes the implicit rowid through any unshadowed spelling.
fn rowid_alias(columns: &[ColumnInfo]) -> Option<&'static str> {
    ["rowid", "_rowid_", "oid"].into_iter().find(|alias| !columns.iter().any(|c| c.name.eq_ignore_ascii_case(alias)))
}

fn read_page(c: &Connection, db: &str, table: &str, req: &PageRequest) -> Result<Page> {
    let info = read_structure(c, db, table)?;
    let rowid = info.row_identity == RowIdentity::RowId;
    let select_list = if rowid {
        format!("{} AS {}, *", quote_ident(rowid_alias(&info.columns).unwrap()), quote_ident(ROWID_COLUMN))
    } else {
        "*".to_owned()
    };
    let from = format!("{}.{}", quote_ident(db), quote_ident(table));
    let (sql, binds) = crate::sql::select_page(&select_list, &from, req, quote_ident)?;
    let mut columns: Vec<ColumnMeta> = Vec::new();
    if rowid {
        columns.push(ColumnMeta { name: ROWID_COLUMN.to_owned(), type_name: "INTEGER".into(), hidden: true });
    }
    columns.extend(info.columns.iter().map(|col| ColumnMeta { name: col.name.clone(), type_name: col.data_type.clone(), hidden: false }));
    let mut stmt = c.prepare(&sql).map_err(map_err)?;
    let n = columns.len();
    let params: Vec<SqlValue> = binds.iter().map(to_sql).collect();
    let rows = collect(&mut stmt, rusqlite::params_from_iter(params), |r| {
        Ok((0..n).map(|i| read_value(r.get_ref_unwrap(i)).to_cell()).collect::<Vec<_>>())
    })?;
    Ok(Page { columns, rows })
}

/// Qualifies `CREATE TRIGGER name` / `CREATE VIEW name` with a schema (stored SQL is unqualified).
fn qualify_create(sql: &str, db: &str) -> String {
    if db == "main" {
        return sql.to_owned();
    }
    let upper = sql.to_ascii_uppercase();
    let Some(kw) = ["TRIGGER", "VIEW"].iter().filter_map(|k| upper.find(k).map(|p| p + k.len())).min() else {
        return sql.to_owned();
    };
    let mut pos = kw + sql[kw..].len() - sql[kw..].trim_start().len();
    if upper[pos..].starts_with("IF NOT EXISTS") {
        pos += "IF NOT EXISTS".len();
        pos += sql[pos..].len() - sql[pos..].trim_start().len();
    }
    format!("{}{}.{}", &sql[..pos], quote_ident(db), &sql[pos..])
}

fn dependents(c: &Connection, db: &str, table: &str) -> Result<crate::design::sqlite::Dependents> {
    let schema = quote_ident(db);
    let mut stmt = c
        .prepare(&format!("SELECT sql FROM {schema}.sqlite_schema WHERE type = 'trigger' AND tbl_name = ?1 AND sql IS NOT NULL"))
        .map_err(map_err)?;
    let triggers = collect(&mut stmt, [table], |r| r.get::<_, String>(0))?.into_iter().map(|s| qualify_create(&s, db)).collect();
    let mut stmt = c.prepare(&format!("SELECT name, sql FROM {schema}.sqlite_schema WHERE type = 'view'")).map_err(map_err)?;
    let needle = table.to_ascii_lowercase();
    let views = collect(&mut stmt, [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .into_iter()
        .filter(|(_, sql)| sql.to_ascii_lowercase().contains(&needle))
        .map(|(n, sql)| (n, qualify_create(&sql, db)))
        .collect();
    Ok(crate::design::sqlite::Dependents { triggers, views })
}

/// Refuse rebuilds whose schema cannot be represented by TableDesign instead of silently weakening it.
fn validate_rebuild(c: &Connection, db: &str, info: &TableInfo) -> Result<()> {
    let upper = info.create_sql.to_ascii_uppercase();
    let words: Vec<&str> = upper.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_').collect();
    if ["CHECK", "COLLATE", "STRICT", "GENERATED", "WITHOUT", "DEFERRABLE", "CONFLICT", "VIRTUAL", "DESC"]
        .iter()
        .any(|word| words.contains(word))
        || info.foreign_keys.iter().any(|fk| fk.columns.len() != fk.ref_columns.len())
    {
        return Err(AppError::validation("This SQLite table has constraints or storage attributes the designer cannot preserve; use SQL"));
    }
    let mut stmt = c.prepare("SELECT name, partial FROM pragma_index_list(?1, ?2)").map_err(map_err)?;
    let indexes = collect(&mut stmt, [info.name.as_str(), db], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)))?;
    for (name, partial) in indexes {
        let unsupported: bool = c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_index_xinfo(?1, ?2) WHERE key = 1 AND (cid < 0 OR \"desc\" <> 0 OR coll <> 'BINARY'))",
                [name.as_str(), db],
                |r| r.get(0),
            )
            .map_err(map_err)?;
        if partial || unsupported {
            return Err(AppError::validation(
                "This SQLite table has partial, expression, ordered or collated indexes the designer cannot preserve; use SQL",
            ));
        }
    }
    Ok(())
}

/// Whether the connection enforces foreign keys (a user may have turned it off with `PRAGMA foreign_keys`).
fn foreign_keys_enabled(c: &Connection) -> Result<bool> {
    c.query_row("PRAGMA foreign_keys", [], |r| r.get(0)).map_err(map_err)
}

fn run_ddl(c: &mut Connection, statements: &[String]) -> Result<()> {
    let toggles_fk = statements.first().map(String::as_str) == Some(FK_OFF);
    let enforced = foreign_keys_enabled(c)?;
    if toggles_fk {
        c.execute_batch(FK_OFF).map_err(map_err)?;
    }
    let result = (|| {
        let tx = c.transaction().map_err(map_err)?;
        for st in statements.iter().filter(|s| s.as_str() != FK_OFF && s.as_str() != FK_ON) {
            if st.starts_with("PRAGMA") && st.ends_with("foreign_key_check") {
                let mut check = tx.prepare(st).map_err(map_err)?;
                if check.query([]).map_err(map_err)?.next().map_err(map_err)?.is_some() {
                    return Err(AppError::sql("The change would violate foreign key constraints"));
                }
            } else {
                tx.execute_batch(st).map_err(map_err)?;
            }
        }
        tx.commit().map_err(map_err)
    })();
    if toggles_fk && enforced {
        c.execute_batch(FK_ON).map_err(map_err)?;
    }
    result
}

/// Runs streamed statements one by one. A failed or cancelled script must not leave the session inside its
/// transaction or with changed FK enforcement, so both are restored before returning. A transaction open before the
/// script is not the script's to end, so the script is refused instead.
fn run_statements(
    c: &mut Connection,
    statements: &mut tokio::sync::mpsc::Receiver<Result<String>>,
    stop_on_error: bool,
    cancel: &CancellationToken,
) -> Result<ScriptRunReport> {
    if !c.is_autocommit() {
        return Err(AppError::validation("The connection is inside a transaction; commit or roll it back before running a SQL file"));
    }
    let enforced = foreign_keys_enabled(c)?;
    let mut report = ScriptRunReport::default();
    let token = cancel.clone();
    c.progress_handler(1000, Some(move || token.is_cancelled())).map_err(map_err)?;
    let mut input_error = None;
    let mut index = 0;
    while let Some(st) = tokio::runtime::Handle::current().block_on(async {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => None,
            statement = statements.recv() => statement,
        }
    }) {
        if cancel.is_cancelled() {
            break;
        }
        let st = match st {
            Ok(st) => st,
            Err(error) => {
                input_error = Some(error);
                break;
            }
        };
        let result = (|| -> Result<()> {
            let mut stmt = c.prepare(&st).map_err(map_err)?;
            if stmt.column_count() > 0 {
                let mut rows = stmt.query([]).map_err(map_err)?;
                while rows.next().map_err(map_err)?.is_some() {}
            } else {
                stmt.execute([]).map_err(map_err)?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => report.executed += 1,
            Err(e) => {
                // Every later statement would wait for the same lock in vain.
                let locked = e.message == LOCKED;
                report.failed.push(crate::transfer::failure(index, &st, e.message));
                if stop_on_error || locked {
                    break;
                }
            }
        }
        index += 1;
    }
    let reset = c.progress_handler(0, None::<fn() -> bool>).map_err(map_err);
    report.open_transaction_rolled_back = !c.is_autocommit();
    let rollback = if report.open_transaction_rolled_back { c.execute_batch("ROLLBACK").map_err(map_err) } else { Ok(()) };
    let foreign_keys = c.execute_batch(if enforced { FK_ON } else { FK_OFF }).map_err(map_err);
    reset?;
    rollback?;
    foreign_keys?;
    // The reader may stop at cancellation without sending another statement: check after the channel closes too.
    if cancel.is_cancelled() {
        Err(AppError::cancelled())
    } else if let Some(error) = input_error {
        Err(error)
    } else {
        Ok(report)
    }
}

fn read_structure(c: &Connection, db: &str, name: &str) -> Result<TableInfo> {
    let schema = quote_ident(db);
    let (kind, create_sql): (String, Option<String>) = c
        .query_row(&format!("SELECT type, sql FROM {schema}.sqlite_schema WHERE name = ?1 AND type IN ('table', 'view')"), [name], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AppError::validation(format!("Table {name} not found")),
            e => map_err(e),
        })?;
    let create_sql = create_sql.unwrap_or_default();
    let kind = if kind == "view" { ObjectKind::View } else { ObjectKind::Table };
    let upper_sql = create_sql.to_ascii_uppercase();

    let mut stmt = c
        .prepare("SELECT name, type, \"notnull\", dflt_value, pk, hidden FROM pragma_table_xinfo(?1, ?2) WHERE hidden <> 1 ORDER BY cid")
        .map_err(map_err)?;
    let raw = collect(&mut stmt, [name, db], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, bool>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, i64>(5)?,
        ))
    })?;
    let pk_count = raw.iter().filter(|c| c.4 > 0).count();
    let mut primary_key: Vec<(i64, String)> = raw.iter().filter(|c| c.4 > 0).map(|c| (c.4, c.0.clone())).collect();
    primary_key.sort();
    let primary_key: Vec<String> = primary_key.into_iter().map(|(_, name)| name).collect();
    let without_rowid = kind == ObjectKind::Table && without_rowid(c, db, name)?;
    let columns: Vec<ColumnInfo> = raw
        .into_iter()
        .map(|(name, data_type, notnull, default, pk, hidden)| {
            let rowid_alias = pk > 0 && pk_count == 1 && !without_rowid && data_type.eq_ignore_ascii_case("INTEGER");
            ColumnInfo {
                auto_increment: rowid_alias && upper_sql.contains("AUTOINCREMENT"),
                // Rowid tables accept NULL in other primary key columns unless they are declared NOT NULL.
                nullable: !notnull && !rowid_alias && !(pk > 0 && without_rowid),
                primary_key: pk > 0,
                generated: hidden == 2 || hidden == 3,
                name,
                data_type,
                default: default.map(default_clause),
                comment: None,
            }
        })
        .collect();

    let mut stmt =
        c.prepare("SELECT name, \"unique\", partial FROM pragma_index_list(?1, ?2) WHERE origin <> 'pk' ORDER BY name").map_err(map_err)?;
    let index_rows = collect(&mut stmt, [name, db], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?, r.get::<_, bool>(2)?)))?;
    let mut indexes = Vec::new();
    let mut identity_indexes = Vec::new();
    for (index, unique, partial) in index_rows {
        let mut stmt = c.prepare("SELECT name FROM pragma_index_info(?1, ?2) ORDER BY seqno").map_err(map_err)?;
        let cols = collect(&mut stmt, [index.as_str(), db], |r| r.get::<_, Option<String>>(0))?;
        let identity_usable = !partial && cols.iter().all(Option::is_some);
        indexes.push(IndexInfo {
            name: index,
            columns: cols.into_iter().flatten().collect(),
            kind: if unique { IndexKind::Unique } else { IndexKind::Normal },
            method: None,
            comment: None,
            primary: false,
        });
        if identity_usable {
            identity_indexes.push(indexes.last().unwrap().clone());
        }
    }

    let mut stmt = c
        .prepare("SELECT id, \"table\", \"from\", \"to\", on_update, on_delete FROM pragma_foreign_key_list(?1, ?2) ORDER BY id, seq")
        .map_err(map_err)?;
    let fk_rows = collect(&mut stmt, [name, db], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
        ))
    })?;
    let mut foreign_keys: Vec<(i64, ForeignKeyInfo)> = Vec::new();
    for (id, table, from, to, on_update, on_delete) in fk_rows {
        if foreign_keys.last().is_none_or(|(last, _)| *last != id) {
            foreign_keys.push((
                id,
                ForeignKeyInfo {
                    name: format!("fk_{name}_{id}"),
                    columns: Vec::new(),
                    ref_table: table,
                    ref_schema: None,
                    ref_columns: Vec::new(),
                    on_update,
                    on_delete,
                },
            ));
        }
        let fk = &mut foreign_keys.last_mut().unwrap().1;
        fk.columns.push(from);
        fk.ref_columns.extend(to);
    }
    // `REFERENCES parent` without columns targets the parent's primary key.
    for (_, fk) in &mut foreign_keys {
        if fk.ref_columns.is_empty() {
            let mut stmt = c.prepare("SELECT name FROM pragma_table_info(?1, ?2) WHERE pk > 0 ORDER BY pk").map_err(map_err)?;
            fk.ref_columns = collect(&mut stmt, [fk.ref_table.as_str(), db], |r| r.get::<_, String>(0))?;
        }
    }

    let row_identity = match kind {
        ObjectKind::Table => {
            // The rowid is usable when the table has one and the grid can fetch it under an unshadowed name.
            let rowid_usable =
                !without_rowid && rowid_alias(&columns).is_some() && !columns.iter().any(|c| c.name.eq_ignore_ascii_case(ROWID_COLUMN));
            // SQLite cannot limit an UPDATE/DELETE to one row, so rows are never matched by value.
            match derive_identity(&columns, &identity_indexes, rowid_usable) {
                RowIdentity::AllColumns { .. } => RowIdentity::None,
                identity => identity,
            }
        }
        ObjectKind::View => RowIdentity::None,
    };
    Ok(TableInfo {
        name: name.to_owned(),
        kind,
        columns,
        indexes,
        foreign_keys: foreign_keys.into_iter().map(|(_, fk)| fk).collect(),
        engine: None,
        charset: None,
        collation: None,
        created: None,
        rows: None,
        modified: None,
        comment: None,
        create_sql,
        row_identity,
        primary_key,
    })
}

fn run_statement(conn: &Connection, sql: &str, row_limit: usize) -> Result<StatementOutcome> {
    let started = Instant::now();
    let mut stmt = conn.prepare(sql).map_err(map_err)?;
    if stmt.column_count() == 0 {
        let affected = stmt.execute([]).map_err(map_err)? as u64;
        let is_insert = sql.trim_start().get(..6).is_some_and(|s| s.eq_ignore_ascii_case("insert"));
        return Ok(StatementOutcome::Affected {
            statement: sql.to_owned(),
            rows_affected: affected,
            last_insert_id: is_insert.then(|| conn.last_insert_rowid() as u64),
            duration_ms: started.elapsed().as_millis() as u64,
        });
    }
    let columns: Vec<ColumnMeta> = stmt
        .columns()
        .map(|c| ColumnMeta { name: c.name().to_owned(), type_name: c.decl_type().unwrap_or("").to_owned(), hidden: false })
        .collect();
    let n = columns.len();
    let mut rows = Vec::new();
    let mut has_more = false;
    let mut cursor = stmt.query([]).map_err(map_err)?;
    while let Some(row) = cursor.next().map_err(map_err)? {
        if rows.len() == row_limit {
            has_more = true;
            break;
        }
        rows.push((0..n).map(|i| read_value(row.get_ref_unwrap(i)).to_cell()).collect::<Vec<CellValue>>());
    }
    Ok(StatementOutcome::Rows { statement: sql.to_owned(), columns, rows, has_more, duration_ms: started.elapsed().as_millis() as u64 })
}

#[async_trait]
impl Driver for SqliteDriver {
    fn engine(&self) -> Engine {
        Engine::Sqlite
    }

    fn server_version(&self) -> String {
        format!("SQLite {}", self.version)
    }

    async fn ping(&self) -> Result<()> {
        self.with_conn(|c| c.query_row("SELECT 1", [], |_| Ok(())).map_err(map_err)).await
    }

    async fn list_databases(&self) -> Result<Vec<DatabaseInfo>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare("PRAGMA database_list").map_err(map_err)?;
            let names =
                stmt.query_map([], |r| r.get::<_, String>(1)).map_err(map_err)?.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)?;
            Ok(names.into_iter().filter(|n| n != "temp").map(|name| DatabaseInfo { name }).collect())
        })
        .await
    }

    async fn list_objects(&self, db: &str) -> Result<Vec<ObjectInfo>> {
        let db = db.to_owned();
        self.with_conn(move |c| {
            let schema = quote_ident(&db);
            let sql = format!(
                "SELECT name, type FROM {schema}.sqlite_schema \
                 WHERE type IN ('table', 'view') AND substr(name, 1, 7) <> 'sqlite_' ORDER BY type, name"
            );
            let mut stmt = c.prepare(&sql).map_err(map_err)?;
            let entries = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(map_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(map_err)?;
            entries
                .into_iter()
                .map(|(name, kind)| {
                    let kind = if kind == "view" { ObjectKind::View } else { ObjectKind::Table };
                    let rows = match kind {
                        ObjectKind::Table => Some(
                            c.query_row(&format!("SELECT count(*) FROM {schema}.{}", quote_ident(&name)), [], |r| r.get(0))
                                .map_err(map_err)?,
                        ),
                        ObjectKind::View => None,
                    };
                    Ok(ObjectInfo { name, kind, rows, created: None, modified: None, comment: None })
                })
                .collect()
        })
        .await
    }

    async fn execute_script(&self, _db: Option<&str>, sql: &str, row_limit: usize, execution_id: Option<&str>) -> Result<ScriptResult> {
        let (executions, id, sql) = (self.executions.clone(), execution_id.map(str::to_owned), sql.to_owned());
        let mut result = self.with_conn(move |c| Ok(run_script(c, &executions, id.as_deref(), &sql, row_limit))).await?;
        if let Some(error) = &mut result.error {
            error.error = self.explain_lock(error.error.clone(), None);
        }
        Ok(result)
    }

    async fn execute_in_session(
        &self,
        session: &str,
        db: Option<&str>,
        sql: &str,
        row_limit: usize,
        execution_id: Option<&str>,
    ) -> Result<ScriptResult> {
        let Some(slot) = self.session(session)? else { return self.execute_script(db, sql, row_limit, execution_id).await };
        let (executions, id, sql) = (self.executions.clone(), execution_id.map(str::to_owned), sql.to_owned());
        let mut result = self.on_session(slot, move |c| Ok(run_script(c, &executions, id.as_deref(), &sql, row_limit))).await?;
        if let Some(error) = &mut result.error {
            error.error = self.explain_lock(error.error.clone(), Some(session));
        }
        Ok(result)
    }

    async fn fetch_more_in_session(&self, session: &str, db: Option<&str>, statement: &str, offset: u64, limit: u64) -> Result<Page> {
        let Some(slot) = self.session(session)? else { return self.fetch_more(db, statement, offset, limit).await };
        let statement = statement.to_owned();
        self.on_session(slot, move |c| read_more(c, &statement, offset, limit)).await.map_err(|e| self.explain_lock(e, Some(session)))
    }

    /// Rolls back the session's open transaction and closes its connection. A run still in progress keeps the
    /// connection until it finishes; dropping it then closes it, which rolls back as well.
    async fn close_editor_session(&self, session: &str) -> Result<()> {
        self.lost_sessions.lock().unwrap_or_else(PoisonError::into_inner).remove(session);
        let Some(slot) = self.sessions.lock().unwrap_or_else(PoisonError::into_inner).remove(session) else { return Ok(()) };
        let conn = match slot.conn.try_lock() {
            Ok(mut guard) => guard.take(),
            Err(_) => None,
        };
        if let Some(conn) = conn {
            tokio::task::spawn_blocking(move || {
                if !conn.is_autocommit() {
                    conn.execute_batch("ROLLBACK").map_err(map_err)?;
                }
                conn.close().map_err(|(_, e)| map_err(e))
            })
            .await
            .map_err(|e| AppError::io(e.to_string()))??;
        }
        Ok(())
    }

    fn live_editor_sessions(&self) -> Vec<String> {
        let sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        // A session busy with a run certainly has a connection.
        sessions
            .iter()
            .filter(|(_, session)| match session.conn.try_lock() {
                Ok(conn) => conn.is_some(),
                Err(_) => true,
            })
            .map(|(key, _)| key.clone())
            .collect()
    }

    fn adopt_lost_editor_sessions(&self, keys: Vec<String>) {
        self.lost_sessions.lock().unwrap_or_else(PoisonError::into_inner).extend(keys);
    }

    async fn stream_query(&self, _db: Option<&str>, sql: &str, batch: usize) -> Result<RowStream> {
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let (conn, sql) = (self.conn.clone(), sql.to_owned());
        tokio::task::spawn_blocking(move || {
            let guard = conn.lock().unwrap_or_else(PoisonError::into_inner);
            if let Err(e) = stream_rows(&guard, &sql, batch, &tx) {
                let _ = tx.blocking_send(Err(e));
            }
        });
        Ok(rx)
    }

    async fn stream_query_in_session(&self, session: &str, db: Option<&str>, sql: &str, batch: usize) -> Result<RowStream> {
        let Some(slot) = self.session(session)? else { return self.stream_query(db, sql, batch).await };
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let (attached, sql) = (self.attached.clone(), sql.to_owned());
        tokio::task::spawn_blocking(move || {
            if let Err(e) = slot.run(&attached, |c| stream_rows(c, &sql, batch, &tx)) {
                let _ = tx.blocking_send(Err(e));
            }
        });
        Ok(rx)
    }

    async fn import_rows(
        &self,
        db: &str,
        table: &str,
        columns: &[String],
        mut rows: tokio::sync::mpsc::Receiver<Result<ImportBatch>>,
        options: ImportOptions,
    ) -> Result<ImportReport> {
        let ImportOptions { skip_errors, empty_first, utc_timestamps: _ } = options;
        let marks = vec!["?"; columns.len()].join(", ");
        let cols = columns.iter().map(|c| quote_ident(c)).collect::<Vec<_>>().join(", ");
        let sql = format!("INSERT INTO {}.{} ({cols}) VALUES ({marks})", quote_ident(db), quote_ident(table));
        let conn = self.conn.clone();
        let (db, table, columns) = (db.to_owned(), table.to_owned(), columns.to_vec());
        tokio::task::spawn_blocking(move || {
            let mut guard = conn.lock().unwrap_or_else(PoisonError::into_inner);
            let info = read_structure(&guard, &db, &table)?;
            info.validate_writable_columns(columns.iter().map(String::as_str))?;
            let tx = guard.transaction().map_err(map_err)?;
            if empty_first {
                tx.execute_batch(&format!("DELETE FROM {}.{}", quote_ident(&db), quote_ident(&table))).map_err(map_err)?;
            }
            let mut report = ImportReport::default();
            {
                let mut stmt = tx.prepare(&sql).map_err(map_err)?;
                while let Some(batch) = rows.blocking_recv() {
                    for (line, values) in batch? {
                        let params: Vec<SqlValue> = values.iter().map(to_sql).collect();
                        match stmt.execute(rusqlite::params_from_iter(params)) {
                            Ok(_) => report.inserted += 1,
                            Err(e) => {
                                let message = map_err(e).message;
                                // `ON CONFLICT ROLLBACK` or `RAISE(ROLLBACK)` ended the whole transaction, taking the rows
                                // before it along: continuing would commit only the rest.
                                if skip_errors && tx.is_autocommit() {
                                    return Err(AppError::sql(format!(
                                        "Line {line}: {message}. The table rolled back the whole import, so no rows were imported."
                                    )));
                                }
                                report.failed_count += 1;
                                if report.failed.len() < MAX_REPORTED_FAILURES {
                                    report.failed.push(RowFailure { line, message });
                                }
                                if !skip_errors {
                                    // Dropping the transaction rolls everything back.
                                    report.inserted = 0;
                                    return Ok(report);
                                }
                            }
                        }
                    }
                }
            }
            tx.commit().map_err(map_err)?;
            Ok(report)
        })
        .await
        .map_err(|e| AppError::io(e.to_string()))?
        .map_err(|e| self.explain_lock(e, None))
    }

    async fn execute_statements(
        &self,
        db: Option<&str>,
        mut statements: tokio::sync::mpsc::Receiver<Result<String>>,
        stop_on_error: bool,
        cancel: CancellationToken,
    ) -> Result<ScriptRunReport> {
        let attached = db.filter(|d| *d != "main").map(str::to_owned);
        let mut report = self
            .with_conn(move |c| match attached {
                None => run_statements(c, &mut statements, stop_on_error, &cancel),
                // Statements are unqualified, so an attached database is scripted through its own connection.
                Some(alias) => {
                    let file: String = c
                        .query_row("SELECT file FROM pragma_database_list WHERE name = ?1", [&alias], |r| r.get(0))
                        .map_err(|_| AppError::validation(format!("Database {alias} is not attached")))?;
                    if file.is_empty() {
                        return Err(AppError::validation(format!("Database {alias} has no file to run the script against")));
                    }
                    let mut other = open_connection(&file, OpenFlags::empty())?;
                    run_statements(&mut other, &mut statements, stop_on_error, &cancel)
                }
            })
            .await?;
        for failure in &mut report.failed {
            if failure.message == LOCKED {
                failure.message = self.explain_lock(AppError::sql(LOCKED), None).message;
            }
        }
        Ok(report)
    }

    async fn dump_script(&self, db: &str) -> Result<DumpScript> {
        let db = db.to_owned();
        self.with_conn(move |c| {
            let schema = quote_ident(&db);
            let mut stmt = c
                .prepare(&format!(
                    "SELECT type, name, tbl_name, sql FROM {schema}.sqlite_schema WHERE sql IS NOT NULL AND substr(name, 1, 7) <> 'sqlite_' ORDER BY rowid"
                ))
                .map_err(map_err)?;
            let entries = collect(&mut stmt, [], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
            })?;
            let mut tables = Vec::new();
            let mut objects = Vec::new();
            for (kind, name, owner, sql) in entries {
                match kind.as_str() {
                    "table" => tables.push((
                        name.clone(),
                        format!(
                            "DROP TABLE IF EXISTS {};
{sql};",
                            quote_ident(&name)
                        ),
                    )),
                    "view" => objects.push(DumpObject { sql: format!("DROP VIEW IF EXISTS {};\n{sql};", quote_ident(&name)), table: None }),
                    // Indexes and triggers belong to the table they are defined on.
                    _ => objects.push(DumpObject { sql: format!("{sql};"), table: Some(owner) }),
                }
            }
            let has_sequence: bool = c
                .query_row(&format!("SELECT count(*) FROM {schema}.sqlite_schema WHERE name = 'sqlite_sequence'"), [], |r| {
                    r.get::<_, i64>(0)
                })
                .map_err(map_err)?
                > 0;
            let mut data_tail = Vec::new();
            if has_sequence {
                let mut stmt = c.prepare(&format!("SELECT name, seq FROM {schema}.sqlite_sequence")).map_err(map_err)?;
                let seqs = collect(&mut stmt, [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
                // Per table, so restoring some tables never resets the counters of the others.
                for (name, seq) in seqs {
                    let literal = name.replace('\'', "''");
                    data_tail.push(DumpObject {
                        sql: format!("DELETE FROM sqlite_sequence WHERE name = '{literal}';\nINSERT INTO sqlite_sequence (name, seq) VALUES ('{literal}', {seq});"),
                        table: Some(name),
                    });
                }
            }
            Ok(DumpScript {
                prologue: vec!["PRAGMA foreign_keys = OFF;".into(), "BEGIN TRANSACTION;".into()],
                tables,
                data_tail,
                objects,
                epilogue: vec!["COMMIT;".into(), "PRAGMA foreign_keys = ON;".into()],
            })
        })
        .await
    }

    async fn fetch_more(&self, _db: Option<&str>, statement: &str, offset: u64, limit: u64) -> Result<Page> {
        let statement = statement.to_owned();
        self.with_conn(move |c| read_more(c, &statement, offset, limit)).await
    }

    async fn list_columns(&self, db: &str) -> Result<Vec<(String, Vec<String>)>> {
        let db = db.to_owned();
        self.with_conn(move |c| {
            let sql = format!(
                "SELECT m.name, p.name FROM {}.sqlite_schema m JOIN pragma_table_xinfo(m.name, ?1) p \
                 WHERE m.type IN ('table', 'view') AND substr(m.name, 1, 7) <> 'sqlite_' AND p.hidden <> 1 ORDER BY m.name, p.cid",
                quote_ident(&db)
            );
            let mut stmt = c.prepare(&sql).map_err(map_err)?;
            let pairs = collect(&mut stmt, [db.as_str()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            let mut out: Vec<(String, Vec<String>)> = Vec::new();
            for (table, column) in pairs {
                match out.last_mut() {
                    Some((t, cols)) if *t == table => cols.push(column),
                    _ => out.push((table, vec![column])),
                }
            }
            Ok(out)
        })
        .await
    }

    async fn cancel(&self, execution_id: &str) -> Result<()> {
        if let Some(execution) = self.executions.lock().unwrap_or_else(PoisonError::into_inner).get(execution_id) {
            execution.cancelled.store(true, Ordering::Relaxed);
            execution.interrupt.interrupt();
        }
        Ok(())
    }

    async fn table_info(&self, db: &str, name: &str) -> Result<TableInfo> {
        let (db, name) = (db.to_owned(), name.to_owned());
        self.with_conn(move |c| read_table_info(c, &db, &name)).await
    }

    async fn fetch_page(&self, db: &str, table: &str, req: &PageRequest) -> Result<Page> {
        let (db, table, req) = (db.to_owned(), table.to_owned(), req.clone());
        self.with_conn(move |c| read_page(c, &db, &table, &req)).await
    }

    async fn apply_changes(&self, db: &str, table: &str, identity: &RowIdentity, changes: &[RowChange]) -> Result<u64, ChangeError> {
        let (db, table, identity, mut changes) = (db.to_owned(), table.to_owned(), identity.clone(), changes.to_vec());
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = conn.lock().unwrap_or_else(PoisonError::into_inner);
            let info = read_structure(&guard, &db, &table)?;
            crate::sql::check_identity(&identity, &info.row_identity)?;
            if info.row_identity == RowIdentity::None && !changes.is_empty() {
                return Err(AppError::validation("This table has no safely addressable row identity and is read-only").into());
            }
            info.validate_changes(&changes)?;
            keep_numbers_numeric(&mut changes, &info.columns);
            let rowid = info.row_identity == RowIdentity::RowId;
            let target = crate::sql::ChangeTarget {
                table_ref: format!("{}.{}", quote_ident(&db), quote_ident(&table)),
                identity: &info.row_identity,
                quote: quote_ident,
                dialect: crate::sql::ChangeDialect { empty_insert: "DEFAULT VALUES", null_safe_eq: "IS", single_row: "" },
                byte_exact: Vec::new(),
                // The implicit key must use the same unshadowed alias as the page projection.
                rowid: rowid.then(|| quote_ident(rowid_alias(&info.columns).unwrap())),
                // Rowids are reused after deletes and renumbered by VACUUM: the loaded values must still match.
                expected_columns: if rowid {
                    info.columns.iter().filter(|c| crate::model::matchable(&c.data_type)).map(|c| c.name.clone()).collect()
                } else {
                    Vec::new()
                },
                complete_match: true,
            };
            let statements = crate::sql::change_statements(&changes, &target)?;
            let tx = guard.transaction().map_err(map_err)?;
            for (index, st) in statements.iter().enumerate() {
                let params: Vec<SqlValue> = st.binds.iter().map(to_sql).collect();
                let affected = tx
                    .execute(&st.sql, rusqlite::params_from_iter(params))
                    .map_err(|e| ChangeError { index: Some(index), error: map_err(e) })?;
                if st.keyed && affected > 1 {
                    return Err(crate::sql::several(index, affected as u64));
                }
                if st.keyed && affected == 0 {
                    return Err(crate::sql::vanished(index));
                }
            }
            tx.commit().map_err(map_err)?;
            Ok(statements.len() as u64)
        })
        .await
        .map_err(|e| ChangeError::from(AppError::io(e.to_string())))?
        .map_err(|e| ChangeError { error: self.explain_lock(e.error, None), ..e })
    }

    async fn count_rows(&self, db: &str, table: &str, filters: &[Filter]) -> Result<u64> {
        let from = format!("{}.{}", quote_ident(db), quote_ident(table));
        let (sql, binds) = crate::sql::count(&from, filters, quote_ident)?;
        self.with_conn(move |c| {
            let params: Vec<SqlValue> = binds.iter().map(to_sql).collect();
            c.query_row(&sql, rusqlite::params_from_iter(params), |r| r.get::<_, i64>(0)).map(|n| n as u64).map_err(map_err)
        })
        .await
    }

    async fn plan_table(&self, db: &str, original: Option<&TableDesign>, design: &TableDesign) -> Result<Vec<String>> {
        let Some(original) = original else {
            return crate::design::sqlite::create(db, design);
        };
        let (db, original, design) = (db.to_owned(), original.clone(), design.clone());
        self.with_conn(move |c| {
            let deps = dependents(c, &db, &original.name)?;
            let info = read_structure(c, &db, &original.name)?;
            if info.columns.iter().any(|c| c.generated) {
                return Err(AppError::validation("Tables with generated columns cannot be altered by the table designer"));
            }
            if TableDesign::from_info(&info) != original {
                return Err(AppError::validation(
                    "The table changed since the designer loaded it. Reopen the designer (or revert) to load its current structure, then make your changes again.",
                ));
            }
            // A rowid not carried by an INTEGER PRIMARY KEY column must survive a rebuild under a name neither design shadows.
            let has_alias = info.primary_key.len() == 1
                && info.columns.iter().any(|col| col.primary_key && col.data_type.eq_ignore_ascii_case("INTEGER"));
            let rowid = (!without_rowid(c, &db, &original.name)? && !has_alias)
                .then(|| {
                    ["rowid", "_rowid_", "oid"].into_iter().find(|alias| {
                        !original.columns.iter().chain(&design.columns).any(|col| col.name.eq_ignore_ascii_case(alias))
                    })
                })
                .flatten();
            let mut statements = crate::design::sqlite::alter(&db, &original, &design, deps, rowid)?;
            if statements.first().map(String::as_str) == Some(FK_OFF) {
                validate_rebuild(c, &db, &info)?;
                if info.columns.iter().any(|c| c.auto_increment) && design.columns.iter().any(|c| c.auto_increment) {
                    let schema = quote_ident(&db);
                    let original_name = original.name.replace('\'', "''");
                    let name = design.name.replace('\'', "''");
                    // Deterministic, so planning the same change twice yields the same statements.
                    let saved = quote_ident("_schematadb_sequence");
                    statements.insert(1, format!(
                        "CREATE TEMP TABLE {saved} AS SELECT seq FROM {schema}.sqlite_sequence WHERE name = '{original_name}'"
                    ));
                    statements.insert(statements.len() - 2, format!(
                        "UPDATE {schema}.sqlite_sequence SET seq = max(seq, coalesce((SELECT max(seq) FROM temp.{saved}), seq)) WHERE name = '{name}'"
                    ));
                    statements.insert(statements.len() - 2, format!(
                        "INSERT INTO {schema}.sqlite_sequence (name, seq) SELECT '{name}', seq FROM temp.{saved} WHERE NOT EXISTS \
                         (SELECT 1 FROM {schema}.sqlite_sequence WHERE name = '{name}')"
                    ));
                    statements.insert(statements.len() - 2, format!("DROP TABLE temp.{saved}"));
                }
            }
            Ok(statements)
        })
        .await
    }

    async fn apply_ddl(&self, _db: &str, statements: &[String]) -> Result<()> {
        let statements = statements.to_vec();
        self.with_conn(move |c| run_ddl(c, &statements)).await
    }

    async fn incoming_foreign_keys(&self, db: &str, table: &str) -> Result<Vec<IncomingForeignKey>> {
        let (db, table) = (db.to_owned(), table.to_owned());
        self.with_conn(move |c| {
            let sql = format!(
                "SELECT DISTINCT m.name, f.id, f.on_delete FROM {}.sqlite_schema m JOIN pragma_foreign_key_list(m.name, ?1) f \
                 WHERE m.type = 'table' AND f.\"table\" = ?2 COLLATE NOCASE AND m.name <> ?2 COLLATE NOCASE ORDER BY m.name, f.id",
                quote_ident(&db)
            );
            let mut stmt = c.prepare(&sql).map_err(map_err)?;
            let rows = collect(&mut stmt, [db.as_str(), table.as_str()], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?))
            })?;
            Ok(rows
                .into_iter()
                .map(|(child, id, on_delete)| IncomingForeignKey { name: format!("fk_{child}_{id}"), table: child, on_delete })
                .collect())
        })
        .await
    }

    async fn drop_object(&self, db: &str, name: &str, kind: ObjectKind) -> Result<()> {
        let what = if kind == ObjectKind::View { "VIEW" } else { "TABLE" };
        let sql = format!("DROP {what} {}.{}", quote_ident(db), quote_ident(name));
        self.with_conn(move |c| c.execute_batch(&sql).map_err(map_err)).await
    }

    async fn empty_table(&self, db: &str, name: &str) -> Result<()> {
        let sql = format!("DELETE FROM {}.{}", quote_ident(db), quote_ident(name));
        self.with_conn(move |c| c.execute_batch(&sql).map_err(map_err)).await
    }

    /// SQLite has no TRUNCATE: deletes every row and resets the AUTOINCREMENT counter, in one transaction.
    async fn truncate_table(&self, db: &str, name: &str) -> Result<()> {
        let schema = quote_ident(db);
        let delete = format!("DELETE FROM {schema}.{}", quote_ident(name));
        let has_sequence = format!("SELECT count(*) FROM {schema}.sqlite_schema WHERE name = 'sqlite_sequence'");
        let reset = format!("DELETE FROM {schema}.sqlite_sequence WHERE name = ?1");
        let name = name.to_owned();
        self.with_conn(move |c| {
            let tx = c.transaction().map_err(map_err)?;
            tx.execute_batch(&delete).map_err(map_err)?;
            if tx.query_row(&has_sequence, [], |r| r.get::<_, i64>(0)).map_err(map_err)? > 0 {
                tx.execute(&reset, [name]).map_err(map_err)?;
            }
            tx.commit().map_err(map_err)
        })
        .await
    }

    async fn rename_table(&self, db: &str, from: &str, to: &str) -> Result<()> {
        let sql = format!("ALTER TABLE {}.{} RENAME TO {}", quote_ident(db), quote_ident(from), quote_ident(to));
        self.with_conn(move |c| c.execute_batch(&sql).map_err(map_err)).await
    }

    async fn create_database(&self, _spec: &CreateDatabase) -> Result<()> {
        Err(AppError::validation("SQLite databases are files: attach a database file instead"))
    }

    async fn drop_database(&self, name: &str) -> Result<()> {
        if name == "main" {
            return Err(AppError::validation("The main database of a SQLite connection cannot be detached"));
        }
        let sql = format!("DETACH DATABASE {}", quote_ident(name));
        self.with_conn(move |c| c.execute_batch(&sql).map_err(map_err)).await?;
        self.attached.lock().unwrap_or_else(PoisonError::into_inner).retain(|(alias, _)| !alias.eq_ignore_ascii_case(name));
        Ok(())
    }

    /// Query-tab sessions opened later, or between their transactions, attach the file under the same alias.
    async fn attach_database(&self, path: &str, alias: &str) -> Result<()> {
        let sql = format!("ATTACH DATABASE ?1 AS {}", quote_ident(alias));
        let (path, name) = (path.to_owned(), alias.to_owned());
        let file: String = self
            .with_conn(move |c| {
                c.execute(&sql, [path]).map_err(map_err)?;
                c.query_row("SELECT file FROM pragma_database_list WHERE name = ?1", [name], |r| r.get(0)).map_err(map_err)
            })
            .await?;
        if !file.is_empty() {
            self.attached.lock().unwrap_or_else(PoisonError::into_inner).push((alias.to_owned(), file));
        }
        Ok(())
    }
}
