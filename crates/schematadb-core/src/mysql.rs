use crate::model::derive_identity;
use crate::transfer::{
    DumpObject, DumpScript, ImportBatch, ImportOptions, ImportReport, MAX_REPORTED_FAILURES, RowFailure, RowStream, ScriptRunReport, StreamItem,
};
use crate::{
    AppError, CellValue, ChangeError, CharsetInfo, ColumnInfo, ColumnMeta, ConnectParams, CreateDatabase, DatabaseInfo, Driver, Engine,
    ErrorKind, Filter, ForeignKeyInfo, IncomingForeignKey, IndexInfo, IndexKind, IndexMethod, ObjectInfo, ObjectKind, Page, PageRequest,
    Result, RowChange, RowIdentity, ScriptResult, StatementError, StatementOutcome, TableDesign, TableInfo, Value, split_statements,
};
use async_trait::async_trait;
use futures::TryStreamExt;
use sqlx::MySql;
use sqlx::mysql::MySqlArguments;
use sqlx::mysql::{MySqlConnectOptions, MySqlPool, MySqlPoolOptions, MySqlRow, MySqlSslMode};
use sqlx::pool::PoolConnection;
use sqlx::query::Query;
use sqlx::{AssertSqlSafe, Column, Connection, Either, Row, Statement, TypeInfo, ValueRef};
use sqlx_mysql::types::MySqlTime;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

struct RunningQuery {
    thread: u64,
    cancelled: bool,
    cancel_complete: Option<oneshot::Receiver<()>>,
}

/// A SQL editor tab's dedicated server connection, opened lazily outside the pool. The async mutex serializes runs.
type EditorSession = Arc<tokio::sync::Mutex<Option<sqlx::MySqlConnection>>>;

/// Session `sql_mode` of pool connections (grid edits, DDL, imports, metadata): the server's mode plus
/// `STRICT_ALL_TABLES`, so invalid values fail instead of being stored converted, and without
/// `NO_BACKSLASH_ESCAPES`, since [`crate::design::mysql::literal`] escapes backslashes. Editor sessions and Run SQL
/// File connections keep the server's mode.
const STRICT_SESSION: &str = "SET SESSION sql_mode = CONCAT_WS(',', 'STRICT_ALL_TABLES', \
     NULLIF(TRIM(BOTH ',' FROM REPLACE(CONCAT(',', @@SESSION.sql_mode, ','), ',NO_BACKSLASH_ESCAPES,', ',')), ''))";

/// `sql_mode` a dump is restored under, as mysqldump writes it: zero ids stay zero and zero dates load.
const DUMP_SQL_MODE: &str = "NO_AUTO_VALUE_ON_ZERO";

/// How a server reports `information_schema.COLUMNS.COLUMN_DEFAULT`.
#[derive(Clone, Copy, PartialEq)]
enum DefaultStyle {
    /// MariaDB 10.2.7+: SQL expressions (quoted literals, `NULL` for none).
    Expressions,
    /// MySQL 8.0.13+: unquoted literals, expressions flagged `DEFAULT_GENERATED` in `EXTRA`.
    FlaggedLiterals,
    /// Older MySQL and MariaDB: unquoted literals, and `CURRENT_TIMESTAMP` is the only expression.
    Literals,
}

impl DefaultStyle {
    fn of(engine: Engine, version: &str) -> Self {
        let number: Vec<u32> = version.split(|c: char| !c.is_ascii_digit()).take(3).map(|p| p.parse().unwrap_or(0)).collect();
        let at_least = |min: [u32; 3]| number.as_slice() >= min.as_slice();
        match engine {
            Engine::MariaDb if at_least([10, 2, 7]) => DefaultStyle::Expressions,
            Engine::MySql if at_least([8, 0, 13]) => DefaultStyle::FlaggedLiterals,
            _ => DefaultStyle::Literals,
        }
    }
}

pub struct MySqlDriver {
    pool: MySqlPool,
    connect_options: MySqlConnectOptions,
    engine: Engine,
    version: String,
    defaults: DefaultStyle,
    uncached_statistics: bool,
    /// Server thread id per running execution, used by `KILL QUERY`.
    running: Mutex<HashMap<String, RunningQuery>>,
    /// Editor sessions by key; dropping the driver closes them, and the server rolls back their transactions.
    editor_sessions: Mutex<HashMap<String, EditorSession>>,
    /// Editor sessions whose server session was lost with a replaced driver; the next run of each reports it once.
    lost_editor_sessions: Mutex<HashSet<String>>,
}

pub(crate) fn quote_ident(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

/// Deadlocks (1213) and lock wait timeouts (1205) may roll back the whole transaction, so later statements would
/// autocommit: treat them as fatal instead of retrying row by row.
pub(crate) fn aborts_transaction(e: &sqlx::Error) -> bool {
    match e {
        sqlx::Error::Database(db) => {
            matches!(db.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>().map(|d| d.number()), Some(1205 | 1213))
        }
        _ => false,
    }
}

pub(crate) fn map_err(e: sqlx::Error) -> AppError {
    match e {
        sqlx::Error::Database(db) => {
            let number = db.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>().map(|d| d.number());
            let kind = match number {
                Some(1317) => ErrorKind::Cancelled,
                Some(1044 | 1045 | 1698) => ErrorKind::Authentication,
                Some(2002 | 2003 | 2006 | 2013) => ErrorKind::Connection,
                _ => ErrorKind::Sql,
            };
            let message = if kind == ErrorKind::Cancelled { "Query cancelled".to_owned() } else { db.message().to_owned() };
            AppError { kind, message, sql_state: db.code().map(|c| c.into_owned()) }
        }
        sqlx::Error::Io(e) => AppError::connection(format!("Connection lost: {e}")),
        sqlx::Error::Tls(e) => AppError::connection(format!("TLS error: {e}")),
        sqlx::Error::PoolTimedOut => AppError {
            kind: ErrorKind::Connection,
            message: "Could not reach the server (timed out waiting for a connection)".to_owned(),
            sql_state: Some(crate::sessions::POOL_TIMEOUT_SQL_STATE.to_owned()),
        },
        sqlx::Error::PoolClosed => AppError::connection("Could not reach the server"),
        sqlx::Error::Protocol(m) => AppError::connection(m),
        other => AppError::sql(other.to_string()),
    }
}

/// Decodes one column of a row into a full-fidelity value, for both text and binary protocol results.
pub(crate) fn read_value(row: &MySqlRow, i: usize) -> Result<Value> {
    let raw = row.try_get_raw(i).map_err(map_err)?;
    if raw.is_null() {
        return Ok(Value::Null);
    }
    let type_name = raw.type_info().name().to_owned();
    let dec = |e: sqlx::Error| AppError::sql(format!("Cannot decode column {i} ({type_name}): {e}"));
    Ok(match type_name.as_str() {
        "BOOLEAN" | "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" | "YEAR" => {
            Value::Int(row.try_get_unchecked::<i64, _>(i).map_err(dec)?)
        }
        t if t.ends_with("UNSIGNED") => Value::UInt(row.try_get_unchecked::<u64, _>(i).map_err(dec)?),
        "FLOAT" => {
            let f = row.try_get_unchecked::<f32, _>(i).map_err(dec)?;
            Value::Float(f.to_string().parse().unwrap_or(f as f64))
        }
        "DOUBLE" => Value::Float(row.try_get_unchecked::<f64, _>(i).map_err(dec)?),
        "DECIMAL" => Value::Decimal(row.try_get_unchecked::<String, _>(i).map_err(dec)?),
        "DATE" => match row.try_get_unchecked::<chrono::NaiveDate, _>(i) {
            Ok(d) => Value::DateTime(d.format("%Y-%m-%d").to_string()),
            Err(_) => Value::DateTime(invalid_date(row, i, false).map_err(dec)?),
        },
        "DATETIME" | "TIMESTAMP" => match row.try_get_unchecked::<chrono::NaiveDateTime, _>(i) {
            Ok(d) => Value::DateTime(d.format("%Y-%m-%d %H:%M:%S%.f").to_string()),
            Err(_) => Value::DateTime(invalid_date(row, i, true).map_err(dec)?),
        },
        "TIME" => Value::DateTime(row.try_get_unchecked::<MySqlTime, _>(i).map_err(dec)?.to_string()),
        "BIT" => {
            let bytes = row.try_get_unchecked::<Vec<u8>, _>(i).map_err(dec)?;
            Value::UInt(bytes.iter().fold(0u64, |acc, b| (acc << 8) | *b as u64))
        }
        "JSON" => Value::Json(row.try_get_unchecked::<String, _>(i).map_err(dec)?),
        // Text with a `_bin` collation carries the BINARY flag too; only the `binary` collation means bytes.
        "BINARY" | "VARBINARY" | "TINYBLOB" | "BLOB" | "MEDIUMBLOB" | "LONGBLOB"
            if <String as sqlx::Type<MySql>>::compatible(raw.type_info().as_ref()) =>
        {
            Value::Text(row.try_get_unchecked::<String, _>(i).map_err(dec)?)
        }
        "BINARY" | "VARBINARY" | "TINYBLOB" | "BLOB" | "MEDIUMBLOB" | "LONGBLOB" | "GEOMETRY" => {
            Value::Bytes(row.try_get_unchecked::<Vec<u8>, _>(i).map_err(dec)?)
        }
        _ => match row.try_get_unchecked::<String, _>(i) {
            Ok(s) => Value::Text(s),
            Err(_) => Value::Bytes(row.try_get_unchecked::<Vec<u8>, _>(i).map_err(dec)?),
        },
    })
}

/// Dates chrono cannot represent, which MySQL stores as given: zero dates (`0000-00-00`) and dates with a zero
/// month or day (`2024-05-00`). Text results carry them as text; binary (prepared statement) results as
/// `length, year (2 bytes LE), month, day[, hour, minute, second[, microseconds (4 bytes LE)]]`.
fn invalid_date(row: &MySqlRow, i: usize, with_time: bool) -> std::result::Result<String, sqlx::Error> {
    let b = row.try_get_unchecked::<Vec<u8>, _>(i)?;
    if b.first().is_some_and(u8::is_ascii_digit) {
        return Ok(String::from_utf8_lossy(&b).into_owned());
    }
    let byte = |n: usize| b.get(n).copied().unwrap_or(0);
    let len = byte(0);
    let date = format!("{:04}-{:02}-{:02}", u16::from_le_bytes([byte(1), byte(2)]), byte(3), byte(4));
    if !with_time {
        return Ok(date);
    }
    let (hour, minute, second) = if len >= 7 { (byte(5), byte(6), byte(7)) } else { (0, 0, 0) };
    let micros = if len >= 11 { u32::from_le_bytes([byte(8), byte(9), byte(10), byte(11)]) } else { 0 };
    // Fractions as chrono's `%.f` writes them: none, milliseconds or microseconds.
    let fraction = match micros {
        0 => String::new(),
        m if m % 1000 == 0 => format!(".{:03}", m / 1000),
        m => format!(".{m:06}"),
    };
    Ok(format!("{date} {hour:02}:{minute:02}:{second:02}{fraction}"))
}

pub(crate) fn column_meta(row: &MySqlRow) -> Vec<ColumnMeta> {
    row.columns()
        .iter()
        .map(|c| ColumnMeta { name: c.name().to_owned(), type_name: c.type_info().name().to_owned(), hidden: false })
        .collect()
}

/// Binds a value as a prepared-statement parameter.
pub(crate) fn bind(q: Query<'_, MySql, MySqlArguments>, v: Value) -> Query<'_, MySql, MySqlArguments> {
    match v {
        Value::Null => q.bind(None::<String>),
        Value::Int(i) => q.bind(i),
        Value::UInt(u) => q.bind(u),
        Value::Float(f) => q.bind(f),
        Value::Decimal(s) | Value::Text(s) | Value::DateTime(s) | Value::Json(s) => q.bind(s),
        Value::Bytes(b) => q.bind(b),
    }
}

/// Nullable text of a column, decoded leniently (information_schema types vary across servers).
fn text(row: &MySqlRow, i: usize) -> Result<Option<String>> {
    Ok(read_value(row, i)?.as_text())
}

fn int(row: &MySqlRow, i: usize) -> Result<Option<i64>> {
    Ok(match read_value(row, i)? {
        Value::Int(v) => Some(v),
        Value::UInt(v) => Some(v as i64),
        other => other.as_text().and_then(|t| t.parse().ok()),
    })
}

/// Charset and collation names are interpolated into DDL, so only plain identifiers are accepted.
fn plain_name(value: &str) -> Result<&str> {
    if !value.is_empty() && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Ok(value)
    } else {
        Err(AppError::validation(format!("Invalid name: {value}")))
    }
}

fn is_numeric_type(data_type: &str) -> bool {
    matches!(
        data_type,
        "tinyint"
            | "smallint"
            | "mediumint"
            | "int"
            | "integer"
            | "bigint"
            | "decimal"
            | "numeric"
            | "float"
            | "double"
            | "real"
            | "bit"
            | "year"
            | "bool"
            | "boolean"
    )
}

/// `CURRENT_TIMESTAMP`, `CURRENT_TIMESTAMP(3)`, `now()`: the only expression defaults before MySQL 8.0.13.
fn is_current_timestamp(expression: &str) -> bool {
    let upper = expression.trim().to_ascii_uppercase();
    let rest = upper.strip_prefix("CURRENT_TIMESTAMP").or_else(|| upper.strip_prefix("NOW"));
    rest.is_some_and(|r| {
        r.is_empty() || r.strip_prefix('(').and_then(|r| r.strip_suffix(')')).is_some_and(|p| p.chars().all(|c| c.is_ascii_digit()))
    })
}

/// MySQL reports expression defaults with quotes and backslashes escaped (`concat(_utf8mb4\'a\')`). An expression
/// with a quote that is not escaped is reported as written.
fn unescape_expression(raw: &str) -> String {
    let mut backslashes = 0;
    for c in raw.chars() {
        if c == '\'' && backslashes % 2 == 0 {
            return raw.to_owned();
        }
        backslashes = if c == '\\' { backslashes + 1 } else { 0 };
    }
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        out.extend(if c == '\\' { chars.next() } else { Some(c) });
    }
    out
}

/// Turns `information_schema.COLUMNS.COLUMN_DEFAULT` into a SQL expression (see [`DefaultStyle`]).
fn default_expression(style: DefaultStyle, raw: Option<String>, data_type: &str, extra: &str) -> Option<String> {
    let raw = raw?;
    match style {
        DefaultStyle::Expressions => return (!raw.eq_ignore_ascii_case("NULL")).then_some(raw),
        DefaultStyle::FlaggedLiterals if extra.to_ascii_uppercase().contains("DEFAULT_GENERATED") => {
            let expression = unescape_expression(&raw);
            return Some(if is_current_timestamp(&expression) { expression } else { format!("({expression})") });
        }
        DefaultStyle::Literals if matches!(data_type, "timestamp" | "datetime") && is_current_timestamp(&raw) => return Some(raw),
        _ => {}
    }
    let binary = matches!(data_type, "binary" | "varbinary" | "tinyblob" | "blob" | "mediumblob" | "longblob");
    // MySQL 8 reports binary literals in hex (`0x6162`), which is valid SQL as is.
    let hex = raw.strip_prefix("0x").is_some_and(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit()));
    if is_numeric_type(data_type) || (binary && hex) { Some(raw) } else { Some(crate::design::mysql::literal(&raw)) }
}

/// Column names of a statement, by preparing it. Used when a result set is empty and has no row to read them
/// from; `None` when the statement cannot be prepared.
async fn statement_columns(conn: &mut sqlx::MySqlConnection, sql: &str) -> Option<Vec<ColumnMeta>> {
    let statement = sqlx::Executor::prepare(conn, sqlx::SqlSafeStr::into_sql_str(AssertSqlSafe(sql))).await.ok()?;
    Some(
        statement
            .columns()
            .iter()
            .map(|c| ColumnMeta { name: c.name().to_owned(), type_name: c.type_info().name().to_owned(), hidden: false })
            .collect(),
    )
}

/// Removes the `DEFINER=user@host` clause so a dumped view, routine or event can be created by whoever restores it.
fn strip_definer(sql: &str) -> String {
    let Some(start) = sql.find("DEFINER=") else { return sql.to_owned() };
    let mut rest = &sql[start + "DEFINER=".len()..];
    // `user`@`host`, each part quoted (doubling its quote) or bare, or a bare `CURRENT_USER`.
    loop {
        let end = match rest.chars().next() {
            Some(quote @ ('`' | '\'' | '"')) => {
                let mut chars = rest.char_indices().skip(1).peekable();
                let mut end = rest.len();
                while let Some((i, c)) = chars.next() {
                    if c == quote {
                        if chars.peek().is_some_and(|&(_, next)| next == quote) {
                            chars.next();
                        } else {
                            end = i + 1;
                            break;
                        }
                    }
                }
                end
            }
            _ => rest.find(|c: char| c.is_whitespace() || c == '@').unwrap_or(rest.len()),
        };
        rest = &rest[end..];
        match rest.strip_prefix('@') {
            Some(host) => rest = host,
            None => break,
        }
    }
    format!("{}{}", &sql[..start], rest.trim_start())
}

/// Statements whose results are row sets even when empty.
fn returns_rows(sql: &str) -> bool {
    let first = sql
        .trim_start_matches(|c: char| c.is_whitespace() || c == '(')
        .split(|c: char| !c.is_alphanumeric())
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    matches!(
        first.as_str(),
        "SELECT"
            | "SHOW"
            | "DESCRIBE"
            | "DESC"
            | "EXPLAIN"
            | "WITH"
            | "VALUES"
            | "TABLE"
            | "CHECKSUM"
            | "ANALYZE"
            | "CHECK"
            | "OPTIMIZE"
            | "REPAIR"
    )
}

impl MySqlDriver {
    pub async fn connect(params: &ConnectParams) -> Result<Self> {
        let mut opts = MySqlConnectOptions::new()
            .host(params.host.as_deref().unwrap_or("localhost"))
            .port(params.port.unwrap_or(3306))
            .username(params.user.as_deref().unwrap_or("root"))
            .ssl_mode(MySqlSslMode::Preferred)
            .charset("utf8mb4")
            .pipes_as_concat(false)
            .no_engine_substitution(false)
            .timezone(None);
        if let Some(pw) = params.password.as_deref() {
            opts = opts.password(pw);
        }
        if let Some(db) = params.database.as_deref().filter(|d| !d.is_empty()) {
            opts = opts.database(db);
        }
        let pool = MySqlPoolOptions::new()
            .max_connections(6)
            .min_connections(1)
            .acquire_timeout(Duration::from_secs(15))
            .after_connect(|conn, _| Box::pin(async move { sqlx::raw_sql(STRICT_SESSION).execute(conn).await.map(|_| ()) }))
            .connect_with(opts.clone())
            .await
            .map_err(map_err)?;
        let version: String = sqlx::query_scalar("SELECT VERSION()").fetch_one(&pool).await.map_err(map_err)?;
        let engine = if version.to_ascii_lowercase().contains("mariadb") { Engine::MariaDb } else { Engine::MySql };
        let uncached_statistics = engine == Engine::MySql
            && version.split('.').next().and_then(|major| major.parse::<u32>().ok()).is_some_and(|major| major >= 8);
        Ok(Self {
            pool,
            connect_options: opts,
            engine,
            defaults: DefaultStyle::of(engine, &version),
            version,
            uncached_statistics,
            running: Mutex::new(HashMap::new()),
            editor_sessions: Mutex::new(HashMap::new()),
            lost_editor_sessions: Mutex::new(HashSet::new()),
        })
    }

    /// Read current engine statistics rather than MySQL 8's day-old INFORMATION_SCHEMA cache.
    async fn metadata_connection(&self) -> Result<PoolConnection<MySql>> {
        let mut conn = self.pool.acquire().await.map_err(map_err)?;
        if self.uncached_statistics {
            sqlx::query("SET SESSION information_schema_stats_expiry = 0").execute(&mut *conn).await.map_err(map_err)?;
        }
        Ok(conn)
    }

    /// Takes the connection of the editor session `key`, opening it if needed. The returned guard serializes runs
    /// of that session; the caller puts the connection back unless it failed at the connection level, so the next
    /// run opens a fresh one (the failed statement is not retried).
    async fn checkout_editor_session(
        &self,
        key: &str,
    ) -> Result<(tokio::sync::OwnedMutexGuard<Option<sqlx::MySqlConnection>>, sqlx::MySqlConnection)> {
        // The session was lost with the driver this one replaced (reconnection, close and reopen).
        if self.lost_editor_sessions.lock().unwrap().remove(key) {
            return Err(lost_editor_session());
        }
        let slot = self.editor_sessions.lock().unwrap().entry(key.to_owned()).or_default().clone();
        let mut guard = slot.lock_owned().await;
        let conn = match guard.take() {
            Some(mut conn) => {
                // The server may have closed it meanwhile (wait_timeout, KILL, restart). Reconnecting silently would hide
                // the rolled-back transaction, so this run fails and the next one opens a fresh session.
                if conn.ping().await.is_err() {
                    return Err(lost_editor_session());
                }
                conn
            }
            None => sqlx::MySqlConnection::connect_with(&self.connect_options).await.map_err(map_err)?,
        };
        Ok((guard, conn))
    }

    async fn run_script(
        &self,
        conn: &mut sqlx::MySqlConnection,
        db: Option<&str>,
        sql: &str,
        row_limit: usize,
        execution_id: Option<&str>,
    ) -> Result<ScriptResult> {
        let statements = split_statements(sql, self.engine);
        use_database(conn, db).await?;
        if let Some(id) = execution_id {
            let thread: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()").fetch_one(&mut *conn).await.map_err(map_err)?;
            self.running.lock().unwrap().insert(id.to_owned(), RunningQuery { thread, cancelled: false, cancel_complete: None });
        }
        let mut outcomes = Vec::new();
        let mut error = None;
        let cancelled = || execution_id.is_some_and(|id| self.running.lock().unwrap().get(id).is_some_and(|query| query.cancelled));
        for (index, statement) in statements.into_iter().enumerate() {
            let result = if cancelled() { Err(AppError::cancelled()) } else { Self::run_statement(conn, &statement, row_limit).await };
            // MySQL may return a successful SLEEP result when KILL QUERY interrupts it.
            let result = if cancelled() { Err(AppError::cancelled()) } else { result };
            match result {
                Ok(o) => outcomes.extend(o),
                Err(e) => {
                    error = Some(StatementError { index, statement, error: e });
                    break;
                }
            }
        }
        if let Some(id) = execution_id {
            let running = self.running.lock().unwrap().remove(id);
            if let Some(complete) = running.and_then(|query| query.cancel_complete) {
                // Do not reuse this server thread before its pending KILL has finished.
                let _ = complete.await;
            }
        }
        Ok(ScriptResult { outcomes, error })
    }

    async fn more_rows(conn: &mut sqlx::MySqlConnection, db: Option<&str>, statement: &str, offset: u64, limit: u64) -> Result<Page> {
        use_database(conn, db).await?;
        let rows = sqlx::query(AssertSqlSafe(crate::sql::more_rows(statement)))
            .bind(limit as i64)
            .bind(offset as i64)
            .fetch_all(&mut *conn)
            .await
            .map_err(map_err)?;
        let columns = rows.first().map(column_meta).unwrap_or_default();
        let rows = rows
            .iter()
            .map(|r| (0..columns.len()).map(|i| read_value(r, i).map(|v| v.to_cell())).collect::<Result<Vec<_>>>())
            .collect::<Result<Vec<_>>>()?;
        Ok(Page { columns, rows })
    }

    /// Cancellation must not queue behind the very queries occupying every pooled connection.
    async fn kill_query(&self, thread: u64) -> Result<()> {
        let mut conn = sqlx::MySqlConnection::connect_with(&self.connect_options).await.map_err(map_err)?;
        sqlx::raw_sql(AssertSqlSafe(format!("KILL QUERY {thread}"))).execute(&mut conn).await.map_err(map_err)?;
        Ok(())
    }

    async fn require_transactional(&self, db: &str, table: &str) -> Result<()> {
        let supported: Option<String> = sqlx::query_scalar(
            "SELECT e.TRANSACTIONS FROM information_schema.TABLES t \
             JOIN information_schema.ENGINES e ON e.ENGINE = t.ENGINE WHERE t.TABLE_SCHEMA = ? AND t.TABLE_NAME = ?",
        )
        .bind(db)
        .bind(table)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;
        if supported.as_deref() != Some("YES") {
            return Err(AppError::validation("Atomic edits and imports require a transactional storage engine (for example InnoDB)"));
        }
        Ok(())
    }

    /// Runs one statement; a `CALL` may return several result sets, one outcome each, every set with its own columns.
    async fn run_statement(conn: &mut sqlx::MySqlConnection, sql: &str, row_limit: usize) -> Result<Vec<StatementOutcome>> {
        let started = Instant::now();
        let mut stream = sqlx::raw_sql(AssertSqlSafe(sql)).fetch_many(&mut *conn);
        let mut outcomes = Vec::new();
        let mut columns: Option<Vec<ColumnMeta>> = None;
        let mut rows = Vec::new();
        let mut has_more = false;
        let mut affected = 0u64;
        let mut last_insert_id = None;
        let mut end_result_set = |columns: Vec<ColumnMeta>, rows: &mut Vec<Vec<CellValue>>, has_more: &mut bool| {
            outcomes.push(StatementOutcome::Rows {
                statement: sql.to_owned(),
                columns,
                rows: std::mem::take(rows),
                has_more: std::mem::take(has_more),
                duration_ms: started.elapsed().as_millis() as u64,
            })
        };
        while let Some(item) = stream.try_next().await.map_err(map_err)? {
            match item {
                Either::Right(row) => {
                    let cols = columns.get_or_insert_with(|| column_meta(&row));
                    if rows.len() < row_limit {
                        rows.push((0..cols.len()).map(|i| read_value(&row, i).map(|v| v.to_cell())).collect::<Result<Vec<CellValue>>>()?);
                    } else {
                        has_more = true;
                    }
                }
                Either::Left(done) => match columns.take() {
                    Some(cols) => end_result_set(cols, &mut rows, &mut has_more),
                    None => {
                        affected += done.rows_affected();
                        if done.last_insert_id() > 0 {
                            last_insert_id = Some(done.last_insert_id());
                        }
                    }
                },
            }
        }
        drop(stream);
        if let Some(cols) = columns.take() {
            end_result_set(cols, &mut rows, &mut has_more);
        }
        if !outcomes.is_empty() {
            return Ok(outcomes);
        }
        let duration_ms = started.elapsed().as_millis() as u64;
        Ok(vec![if returns_rows(sql) {
            // Empty result sets carry no rows to read column names from; prepare the statement instead.
            let columns = statement_columns(conn, sql).await.unwrap_or_default();
            StatementOutcome::Rows { statement: sql.to_owned(), columns, rows: Vec::new(), has_more: false, duration_ms }
        } else {
            StatementOutcome::Affected { statement: sql.to_owned(), rows_affected: affected, last_insert_id, duration_ms }
        }])
    }

    /// [`Driver::execute_statements`] on `conn`, which the caller closes afterwards.
    async fn run_statements(
        &self,
        conn: &mut sqlx::MySqlConnection,
        db: Option<&str>,
        mut statements: tokio::sync::mpsc::Receiver<Result<String>>,
        stop_on_error: bool,
        cancel: &CancellationToken,
    ) -> Result<ScriptRunReport> {
        use_database(conn, db).await?;
        let thread: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()").fetch_one(&mut *conn).await.map_err(map_err)?;
        let mut report = ScriptRunReport::default();
        let mut index = 0;
        let mut input_error = None;
        loop {
            let next = tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                next = statements.recv() => next,
            };
            let st = match next {
                None => break,
                Some(Ok(st)) => st,
                Some(Err(error)) => {
                    input_error = Some(error);
                    break;
                }
            };
            let result = {
                let query = sqlx::raw_sql(AssertSqlSafe(st.as_str())).execute(&mut *conn);
                tokio::pin!(query);
                tokio::select! {
                    biased;
                    result = &mut query => result.map_err(map_err).map(|_| ()),
                    _ = cancel.cancelled() => {
                        // Poll the query while establishing the control connection too: cancellation can race
                        // its first socket write. Drain the interrupted response before session cleanup.
                        let (_, killed) = tokio::join!(&mut query, self.kill_query(thread));
                        killed.and(Err(AppError::cancelled()))
                    }
                }
            };
            match result {
                Ok(()) => report.executed += 1,
                Err(error) => {
                    report.failed.push(crate::transfer::failure(index, &st, error.message));
                    if stop_on_error || cancel.is_cancelled() {
                        break;
                    }
                }
            }
            index += 1;
        }
        // Undo what the script left uncommitted, including on reader failures and cancellation.
        report.open_transaction_rolled_back = roll_back_open_transaction(conn).await?;
        if cancel.is_cancelled() {
            Err(AppError::cancelled())
        } else if let Some(error) = input_error {
            Err(error)
        } else {
            Ok(report)
        }
    }

    /// `SHOW CREATE <kind>` of an object of `db`: `(sql_mode, CREATE statement without DEFINER, time_zone)`.
    async fn show_create(&self, db: &str, kind: &str, name: &str) -> Result<(String, String, Option<String>)> {
        let row = sqlx::raw_sql(AssertSqlSafe(format!("SHOW CREATE {kind} {}.{}", quote_ident(db), quote_ident(name))))
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?;
        // Events report their time zone before the statement.
        let (create, time_zone) = if kind == "EVENT" { (text(&row, 3)?, text(&row, 2)?) } else { (text(&row, 2)?, None) };
        // Without the privilege to read the body, the server reports none: the backup would silently lack it.
        let create = create.ok_or_else(|| {
            AppError::validation(format!(
                "Cannot read the definition of {} {name}: the user lacks the privilege to see it",
                kind.to_lowercase()
            ))
        })?;
        Ok((text(&row, 1)?.unwrap_or_default(), strip_definer(&create), time_zone))
    }

    /// Stored procedures, functions (and MariaDB packages) of `db`, each created under its own `sql_mode`.
    async fn dump_routines(&self, db: &str) -> Result<Vec<String>> {
        let routines = sqlx::query(
            "SELECT ROUTINE_TYPE, ROUTINE_NAME FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA = ? ORDER BY ROUTINE_TYPE, ROUTINE_NAME",
        )
        .bind(db)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        let mut out = Vec::new();
        for r in &routines {
            let (kind, name) = (text(r, 0)?.unwrap_or_default().to_ascii_uppercase(), text(r, 1)?.unwrap_or_default());
            if !matches!(kind.as_str(), "PROCEDURE" | "FUNCTION" | "PACKAGE" | "PACKAGE BODY") {
                return Err(AppError::validation(format!("Cannot back up routine {name} of unknown type {kind}")));
            }
            let (sql_mode, create, _) = self.show_create(db, &kind, &name).await?;
            out.push(format!(
                "DROP {kind} IF EXISTS {};\nSET SESSION sql_mode = {};\nDELIMITER ;;\n{create};;\nDELIMITER ;\nSET SESSION sql_mode = {};",
                quote_ident(&name),
                crate::design::mysql::literal(&sql_mode),
                crate::design::mysql::literal(DUMP_SQL_MODE),
            ));
        }
        Ok(out)
    }

    /// Events of `db`, each created under its own `sql_mode` and time zone (which their schedules are read in).
    async fn dump_events(&self, db: &str) -> Result<Vec<String>> {
        let names: Vec<String> = sqlx::query("SELECT EVENT_NAME FROM information_schema.EVENTS WHERE EVENT_SCHEMA = ? ORDER BY EVENT_NAME")
            .bind(db)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?
            .iter()
            .map(|r| text(r, 0).map(Option::unwrap_or_default))
            .collect::<Result<_>>()?;
        let mut out = Vec::new();
        for name in names {
            let (sql_mode, create, time_zone) = self.show_create(db, "EVENT", &name).await?;
            out.push(format!(
                "DROP EVENT IF EXISTS {};\nSET SESSION sql_mode = {};\nSET time_zone = {};\nDELIMITER ;;\n{create};;\nDELIMITER ;\nSET SESSION sql_mode = {};\nSET time_zone = '+00:00';",
                quote_ident(&name),
                crate::design::mysql::literal(&sql_mode),
                crate::design::mysql::literal(time_zone.as_deref().unwrap_or("SYSTEM")),
                crate::design::mysql::literal(DUMP_SQL_MODE),
            ));
        }
        Ok(out)
    }

    /// Indexes the designer cannot model, by name: prefix lengths (`SUB_PART`), descending or functional key parts,
    /// invisible (MySQL) or ignored (MariaDB). Re-created from a design they would change, so the designer leaves
    /// them alone. `PRIMARY` is listed when the primary key is such an index.
    async fn unmodelled_indexes(&self, db: &str, table: &str) -> Result<Vec<String>> {
        // Every column: their set differs between servers and versions.
        let rows = sqlx::query("SELECT * FROM information_schema.STATISTICS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?")
            .bind(db)
            .bind(table)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;
        let mut out: Vec<String> = Vec::new();
        for r in &rows {
            let field = |name: &str| match r.columns().iter().position(|c| c.name().eq_ignore_ascii_case(name)) {
                Some(i) => text(r, i),
                None => Ok(None),
            };
            let index = field("INDEX_NAME")?.unwrap_or_default();
            // Some servers report a fixed prefix length on spatial key parts.
            let spatial = field("INDEX_TYPE")?.is_some_and(|t| t.eq_ignore_ascii_case("SPATIAL"));
            let unmodelled = (field("SUB_PART")?.is_some() && !spatial)
                || field("COLLATION")?.as_deref() == Some("D")
                || field("COLUMN_NAME")?.is_none()
                || field("EXPRESSION")?.is_some()
                || field("IS_VISIBLE")?.is_some_and(|v| v.eq_ignore_ascii_case("NO"))
                || field("IGNORED")?.is_some_and(|v| v.eq_ignore_ascii_case("YES"));
            if unmodelled && !out.contains(&index) {
                out.push(index);
            }
        }
        Ok(out)
    }
}

/// BIT values typed as text (`6`, `b'110'`, `0x06`) as integers: bound as text, MySQL would store the bytes of the
/// characters (`'6'` is 54). Other text is refused.
fn bit_values<'a>(info: &TableInfo, changes: &'a [RowChange]) -> Result<std::borrow::Cow<'a, [RowChange]>, ChangeError> {
    let bits: Vec<&str> =
        info.columns.iter().filter(|c| c.data_type.to_ascii_lowercase().starts_with("bit")).map(|c| c.name.as_str()).collect();
    let typed = |v: &crate::ColumnValue| bits.contains(&v.column.as_str()) && matches!(v.value, CellValue::Text(_));
    let values = |c: &RowChange| match c {
        RowChange::Insert { values } | RowChange::Update { values, .. } => values.iter().any(typed),
        RowChange::Delete { .. } => false,
    };
    if !changes.iter().any(values) {
        return Ok(std::borrow::Cow::Borrowed(changes));
    }
    let mut converted = changes.to_vec();
    for (index, change) in converted.iter_mut().enumerate() {
        let (RowChange::Insert { values } | RowChange::Update { values, .. }) = change else { continue };
        for v in values.iter_mut().filter(|v| bits.contains(&v.column.as_str())) {
            let CellValue::Text(text) = &v.value else { continue };
            let number = parse_bit(text).ok_or_else(|| ChangeError {
                index: Some(index),
                error: AppError::validation(format!("{} is a BIT column: enter a number such as 6, b'110' or 0x06", v.column)),
            })?;
            v.value = Value::UInt(number).to_cell();
        }
    }
    Ok(std::borrow::Cow::Owned(converted))
}

/// `6`, `b'110'`/`0b110` or `x'06'`/`0x06`.
fn parse_bit(text: &str) -> Option<u64> {
    let t = text.trim();
    let quoted = |prefix: [char; 2]| t.strip_prefix(prefix).and_then(|r| r.strip_prefix('\'')).and_then(|r| r.strip_suffix('\''));
    let digits = |d: &str, radix| if d.is_empty() || d.starts_with('+') { None } else { u64::from_str_radix(d, radix).ok() };
    if let Some(b) = t.strip_prefix("0b").or_else(|| quoted(['b', 'B'])) {
        return digits(b, 2);
    }
    if let Some(h) = t.strip_prefix("0x").or_else(|| quoted(['x', 'X'])) {
        return digits(h, 16);
    }
    digits(t, 10)
}

fn lost_editor_session() -> AppError {
    AppError::connection("The server closed this editor session; any open transaction was rolled back. Run again to reconnect.")
}

async fn use_database(conn: &mut sqlx::MySqlConnection, db: Option<&str>) -> Result<()> {
    if let Some(db) = db {
        sqlx::raw_sql(AssertSqlSafe(format!("USE {}", quote_ident(db)))).execute(&mut *conn).await.map_err(map_err)?;
    }
    Ok(())
}

/// Running a statement again to load more of its rows must not repeat side effects.
fn require_read_only(statement: &str) -> Result<()> {
    if crate::sql::is_read_only_query(statement) {
        Ok(())
    } else {
        Err(AppError::validation(
            "Only read-only statements (such as SELECT) can load more rows: this one would run again and repeat its effects",
        ))
    }
}

/// Rolls back the transaction open on `conn`, if any, and tells whether there was one. Every server reply flags an
/// open transaction; `begin_with` fails with `BeginFailed` when a no-op statement's reply does not.
async fn roll_back_open_transaction(conn: &mut sqlx::MySqlConnection) -> Result<bool> {
    match conn.begin_with("DO 0").await {
        Ok(tx) => {
            tx.rollback().await.map_err(map_err)?;
            Ok(true)
        }
        Err(sqlx::Error::BeginFailed) => Ok(false),
        Err(e) => Err(map_err(e)),
    }
}

/// Sends the rows of `sql` to `tx` in batches of `batch`, the column list first. Stops when the receiver is gone.
async fn stream_rows(
    conn: &mut sqlx::MySqlConnection,
    sql: &str,
    batch: usize,
    tx: &tokio::sync::mpsc::Sender<Result<StreamItem>>,
) -> Result<()> {
    let mut stream = sqlx::raw_sql(AssertSqlSafe(sql)).fetch(&mut *conn);
    let Some(first) = stream.try_next().await.map_err(map_err)? else {
        drop(stream);
        let columns = statement_columns(conn, sql).await.unwrap_or_default();
        let _ = tx.send(Ok(StreamItem::Columns(columns))).await;
        return Ok(());
    };
    let columns = column_meta(&first);
    let n = columns.len();
    if tx.send(Ok(StreamItem::Columns(columns))).await.is_err() {
        return Ok(());
    }
    let read = |r: &MySqlRow| (0..n).map(|i| read_value(r, i)).collect::<Result<Vec<_>>>();
    let mut buf = vec![read(&first)?];
    while let Some(row) = stream.try_next().await.map_err(map_err)? {
        buf.push(read(&row)?);
        if buf.len() == batch && tx.send(Ok(StreamItem::Rows(std::mem::take(&mut buf)))).await.is_err() {
            return Ok(());
        }
    }
    if !buf.is_empty() {
        let _ = tx.send(Ok(StreamItem::Rows(buf))).await;
    }
    Ok(())
}

#[async_trait]
impl Driver for MySqlDriver {
    fn engine(&self) -> Engine {
        self.engine
    }

    fn server_version(&self) -> String {
        let name = if self.engine == Engine::MariaDb { "MariaDB" } else { "MySQL" };
        format!("{name} {}", self.version.split('-').next().unwrap_or(&self.version))
    }

    async fn ping(&self) -> Result<()> {
        sqlx::query("SELECT 1").execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    async fn list_databases(&self) -> Result<Vec<DatabaseInfo>> {
        let names: Vec<String> = sqlx::query_scalar("SELECT SCHEMA_NAME FROM information_schema.SCHEMATA ORDER BY SCHEMA_NAME")
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(names.into_iter().map(|name| DatabaseInfo { name }).collect())
    }

    async fn list_objects(&self, db: &str) -> Result<Vec<ObjectInfo>> {
        let rows = {
            let mut conn = self.metadata_connection().await?;
            sqlx::query(
                "SELECT TABLE_NAME, TABLE_TYPE, CAST(TABLE_ROWS AS SIGNED), \
                        DATE_FORMAT(CREATE_TIME, '%Y-%m-%d %H:%i:%s'), DATE_FORMAT(UPDATE_TIME, '%Y-%m-%d %H:%i:%s'), TABLE_COMMENT \
                 FROM information_schema.TABLES WHERE TABLE_SCHEMA = ? \
                 ORDER BY TABLE_TYPE LIKE '%VIEW', TABLE_NAME",
            )
            .bind(db)
            .fetch_all(&mut *conn)
            .await
            .map_err(map_err)?
        };
        rows.iter()
            .map(|r| {
                let kind_raw: String = r.try_get(1).map_err(map_err)?;
                let kind = if kind_raw.ends_with("VIEW") { ObjectKind::View } else { ObjectKind::Table };
                let comment: Option<String> = r.try_get(5).map_err(map_err)?;
                Ok(ObjectInfo {
                    name: r.try_get(0).map_err(map_err)?,
                    kind,
                    rows: if kind == ObjectKind::Table { r.try_get(2).map_err(map_err)? } else { None },
                    created: r.try_get(3).map_err(map_err)?,
                    modified: r.try_get(4).map_err(map_err)?,
                    comment: comment.filter(|c| !c.is_empty() && kind == ObjectKind::Table),
                })
            })
            .collect()
    }

    async fn execute_script(&self, db: Option<&str>, sql: &str, row_limit: usize, execution_id: Option<&str>) -> Result<ScriptResult> {
        let mut conn = self.pool.acquire().await.map_err(map_err)?;
        self.run_script(&mut conn, db, sql, row_limit, execution_id).await
    }

    async fn execute_in_session(
        &self,
        session: &str,
        db: Option<&str>,
        sql: &str,
        row_limit: usize,
        execution_id: Option<&str>,
    ) -> Result<ScriptResult> {
        let (mut guard, mut conn) = self.checkout_editor_session(session).await?;
        let result = self.run_script(&mut conn, db, sql, row_limit, execution_id).await;
        let lost = match &result {
            Ok(r) => r.error.as_ref().is_some_and(|e| e.error.kind == ErrorKind::Connection),
            Err(e) => e.kind == ErrorKind::Connection,
        };
        if !lost {
            *guard = Some(conn);
        }
        result
    }

    async fn stream_query(&self, db: Option<&str>, sql: &str, batch: usize) -> Result<RowStream> {
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let mut conn = self.pool.acquire().await.map_err(map_err)?;
        use_database(&mut conn, db).await?;
        let sql = sql.to_owned();
        tokio::spawn(async move {
            if let Err(e) = stream_rows(&mut conn, &sql, batch, &tx).await {
                let _ = tx.send(Err(e)).await;
            }
        });
        Ok(rx)
    }

    async fn stream_query_in_session(&self, session: &str, db: Option<&str>, sql: &str, batch: usize) -> Result<RowStream> {
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let (mut guard, mut conn) = self.checkout_editor_session(session).await?;
        if let Err(e) = use_database(&mut conn, db).await {
            if e.kind != ErrorKind::Connection {
                *guard = Some(conn);
            }
            return Err(e);
        }
        let sql = sql.to_owned();
        // The guard moves into the task: other runs of the session wait until the stream ends.
        tokio::spawn(async move {
            let result = stream_rows(&mut conn, &sql, batch, &tx).await;
            if !result.as_ref().is_err_and(|e| e.kind == ErrorKind::Connection) {
                *guard = Some(conn);
            }
            if let Err(e) = result {
                let _ = tx.send(Err(e)).await;
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
        let ImportOptions { skip_errors, empty_first, utc_timestamps } = options;
        self.require_transactional(db, table).await?;
        self.table_info(db, table).await?.validate_writable_columns(columns.iter().map(String::as_str))?;
        let cols = columns.iter().map(|c| quote_ident(c)).collect::<Vec<_>>().join(", ");
        let one = format!("({})", vec!["?"; columns.len()].join(", "));
        let prefix = format!("INSERT INTO {}.{} ({cols}) VALUES ", quote_ident(db), quote_ident(table));
        // MySQL allows at most 65,535 placeholders per statement.
        let per_statement = (60_000 / columns.len().max(1)).max(1);
        let mut conn = self.pool.acquire().await.map_err(map_err)?;
        if utc_timestamps {
            // TIMESTAMP values are converted from the session time zone; this one must not go back to the pool.
            conn.close_on_drop();
            sqlx::raw_sql("SET time_zone = '+00:00'").execute(&mut *conn).await.map_err(map_err)?;
        }
        let mut tx = Connection::begin(&mut *conn).await.map_err(map_err)?;
        if empty_first {
            sqlx::raw_sql(AssertSqlSafe(format!("DELETE FROM {}.{}", quote_ident(db), quote_ident(table))))
                .execute(&mut *tx)
                .await
                .map_err(map_err)?;
        }
        let mut report = ImportReport::default();
        while let Some(batch) = rows.recv().await {
            let batch = batch?;
            for chunk in batch.chunks(per_statement) {
                let sql = format!("{prefix}{}", vec![one.as_str(); chunk.len()].join(", "));
                let mut q = sqlx::query(AssertSqlSafe(sql));
                for (_, values) in chunk {
                    for v in values.iter().cloned() {
                        q = bind(q, v);
                    }
                }
                match q.execute(&mut *tx).await {
                    Ok(_) => {
                        report.inserted += chunk.len() as u64;
                        continue;
                    }
                    Err(e) if aborts_transaction(&e) => {
                        let _ = tx.rollback().await;
                        return Err(map_err(e));
                    }
                    Err(_) => {}
                }
                // Find the failing rows one by one (a failed statement leaves the transaction usable).
                for (line, values) in chunk {
                    let mut q = sqlx::query(AssertSqlSafe(format!("{prefix}{one}")));
                    for v in values.iter().cloned() {
                        q = bind(q, v);
                    }
                    match q.execute(&mut *tx).await {
                        Ok(_) => report.inserted += 1,
                        Err(e) if aborts_transaction(&e) => {
                            let _ = tx.rollback().await;
                            return Err(map_err(e));
                        }
                        Err(e) => {
                            report.failed_count += 1;
                            if report.failed.len() < MAX_REPORTED_FAILURES {
                                report.failed.push(RowFailure { line: *line, message: map_err(e).message });
                            }
                            if !skip_errors {
                                tx.rollback().await.map_err(map_err)?;
                                report.inserted = 0;
                                return Ok(report);
                            }
                        }
                    }
                }
            }
        }
        tx.commit().await.map_err(map_err)?;
        Ok(report)
    }

    async fn execute_statements(
        &self,
        db: Option<&str>,
        statements: tokio::sync::mpsc::Receiver<Result<String>>,
        stop_on_error: bool,
        cancel: CancellationToken,
    ) -> Result<ScriptRunReport> {
        // A dedicated connection, closed afterwards: scripts change session state (SET NAMES, sql_mode, time_zone,
        // UNIQUE_CHECKS, autocommit, USE) that must not reach the pool.
        let mut conn = sqlx::MySqlConnection::connect_with(&self.connect_options).await.map_err(map_err)?;
        let result = self.run_statements(&mut conn, db, statements, stop_on_error, &cancel).await;
        let _ = conn.close().await;
        result
    }

    async fn dump_script(&self, db: &str) -> Result<DumpScript> {
        let objects = self.list_objects(db).await?;
        let qdb = quote_ident(db);
        let mut tables = Vec::new();
        let mut views = Vec::new();
        for o in &objects {
            let row = sqlx::raw_sql(AssertSqlSafe(format!("SHOW CREATE TABLE {qdb}.{}", quote_ident(&o.name))))
                .fetch_one(&self.pool)
                .await
                .map_err(map_err)?;
            let create = text(&row, 1)?.unwrap_or_default();
            match o.kind {
                ObjectKind::Table => tables.push((
                    o.name.clone(),
                    format!(
                        "DROP TABLE IF EXISTS {};
{create};",
                        quote_ident(&o.name)
                    ),
                )),
                ObjectKind::View => {
                    // Portable views: no DEFINER, no references to the source database name.
                    let create = strip_definer(&create).replace(&format!("{qdb}."), "");
                    views.push((
                        o.name.clone(),
                        format!(
                            "DROP VIEW IF EXISTS {};
{create};",
                            quote_ident(&o.name)
                        ),
                    ));
                }
            }
        }
        // Routines first: a view may call a stored function, while routine bodies are only checked when they run.
        let unowned = |sql: String| DumpObject { sql, table: None };
        let mut objects_sql: Vec<DumpObject> = self.dump_routines(db).await?.into_iter().map(unowned).collect();
        objects_sql.extend(crate::transfer::order_views(views, quote_ident).into_iter().map(|(_, sql)| unowned(sql)));
        let triggers = sqlx::query(
            "SELECT TRIGGER_NAME, ACTION_TIMING, EVENT_MANIPULATION, EVENT_OBJECT_TABLE, ACTION_STATEMENT \
             FROM information_schema.TRIGGERS WHERE TRIGGER_SCHEMA = ? ORDER BY EVENT_OBJECT_TABLE, ACTION_ORDER",
        )
        .bind(db)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        for r in &triggers {
            let (name, timing, event, table, body) = (
                text(r, 0)?.unwrap_or_default(),
                text(r, 1)?.unwrap_or_default(),
                text(r, 2)?.unwrap_or_default(),
                text(r, 3)?.unwrap_or_default(),
                text(r, 4)?.unwrap_or_default(),
            );
            let sql = format!(
                "DROP TRIGGER IF EXISTS {0};
DELIMITER ;;
CREATE TRIGGER {0} {timing} {event} ON {1} FOR EACH ROW {body};;
DELIMITER ;",
                quote_ident(&name),
                quote_ident(&table)
            );
            objects_sql.push(DumpObject { sql, table: Some(table) });
        }
        objects_sql.extend(self.dump_events(db).await?.into_iter().map(unowned));
        Ok(DumpScript {
            // As mysqldump: values are written in UTC and restored as given (zero ids and dates included).
            prologue: vec![
                "SET NAMES utf8mb4;".into(),
                format!("SET SESSION sql_mode = {};", crate::design::mysql::literal(DUMP_SQL_MODE)),
                "SET time_zone = '+00:00';".into(),
                "SET FOREIGN_KEY_CHECKS = 0;".into(),
            ],
            tables,
            data_tail: Vec::new(),
            objects: objects_sql,
            epilogue: vec!["SET FOREIGN_KEY_CHECKS = 1;".into()],
        })
    }

    async fn fetch_more(&self, db: Option<&str>, statement: &str, offset: u64, limit: u64) -> Result<Page> {
        require_read_only(statement)?;
        let mut conn = self.pool.acquire().await.map_err(map_err)?;
        Self::more_rows(&mut conn, db, statement, offset, limit).await
    }

    async fn fetch_more_in_session(&self, session: &str, db: Option<&str>, statement: &str, offset: u64, limit: u64) -> Result<Page> {
        require_read_only(statement)?;
        let (mut guard, mut conn) = self.checkout_editor_session(session).await?;
        let result = Self::more_rows(&mut conn, db, statement, offset, limit).await;
        if !result.as_ref().is_err_and(|e| e.kind == ErrorKind::Connection) {
            *guard = Some(conn);
        }
        result
    }

    async fn close_editor_session(&self, session: &str) -> Result<()> {
        self.lost_editor_sessions.lock().unwrap().remove(session);
        let Some(slot) = self.editor_sessions.lock().unwrap().remove(session) else { return Ok(()) };
        // A run still in progress holds the lock; its connection is dropped when that run finishes.
        let conn = match slot.try_lock() {
            Ok(mut guard) => guard.take(),
            Err(_) => None,
        };
        if let Some(conn) = conn {
            let _ = conn.close().await;
        }
        Ok(())
    }

    fn live_editor_sessions(&self) -> Vec<String> {
        let sessions = self.editor_sessions.lock().unwrap();
        // A session busy with a run holds a connection too.
        let live = sessions.iter().filter(|(_, slot)| slot.try_lock().map_or(true, |conn| conn.is_some())).map(|(key, _)| key.clone());
        // Sessions this driver took over and has not reported yet are lost with it as well.
        live.chain(self.lost_editor_sessions.lock().unwrap().iter().cloned()).collect()
    }

    fn adopt_lost_editor_sessions(&self, keys: Vec<String>) {
        self.lost_editor_sessions.lock().unwrap().extend(keys);
    }

    async fn list_columns(&self, db: &str) -> Result<Vec<(String, Vec<String>)>> {
        let rows = sqlx::query(
            "SELECT TABLE_NAME, COLUMN_NAME FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = ? ORDER BY TABLE_NAME, ORDINAL_POSITION",
        )
        .bind(db)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        let mut out: Vec<(String, Vec<String>)> = Vec::new();
        for r in &rows {
            let (table, column) = (text(r, 0)?.unwrap_or_default(), text(r, 1)?.unwrap_or_default());
            match out.last_mut() {
                Some((t, cols)) if *t == table => cols.push(column),
                _ => out.push((table, vec![column])),
            }
        }
        Ok(out)
    }

    async fn cancel(&self, execution_id: &str) -> Result<()> {
        let cancellation = self.running.lock().unwrap().get_mut(execution_id).and_then(|query| {
            if query.cancelled {
                return None;
            }
            let (done, complete) = oneshot::channel();
            query.cancelled = true;
            query.cancel_complete = Some(complete);
            Some((query.thread, done))
        });
        if let Some((thread, done)) = cancellation {
            let result = self.kill_query(thread).await;
            let _ = done.send(());
            result?;
        }
        Ok(())
    }

    async fn table_info(&self, db: &str, name: &str) -> Result<TableInfo> {
        let table = {
            let mut conn = self.metadata_connection().await?;
            sqlx::query(
                "SELECT TABLE_TYPE, ENGINE, TABLE_COLLATION, CAST(TABLE_ROWS AS SIGNED), \
                        DATE_FORMAT(CREATE_TIME, '%Y-%m-%d %H:%i:%s'), DATE_FORMAT(UPDATE_TIME, '%Y-%m-%d %H:%i:%s'), TABLE_COMMENT \
                 FROM information_schema.TABLES WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?",
            )
            .bind(db)
            .bind(name)
            .fetch_optional(&mut *conn)
            .await
            .map_err(map_err)?
            .ok_or_else(|| AppError::validation(format!("Table {name} not found")))?
        };
        let kind = if text(&table, 0)?.unwrap_or_default().ends_with("VIEW") { ObjectKind::View } else { ObjectKind::Table };
        let collation = text(&table, 2)?;
        let charset = match &collation {
            Some(col) => sqlx::query("SELECT CHARACTER_SET_NAME FROM information_schema.COLLATIONS WHERE COLLATION_NAME = ?")
                .bind(col)
                .fetch_optional(&self.pool)
                .await
                .map_err(map_err)?
                .map(|r| text(&r, 0))
                .transpose()?
                .flatten(),
            None => None,
        };

        let index_rows = sqlx::query(
            "SELECT INDEX_NAME, NON_UNIQUE, COLUMN_NAME, INDEX_TYPE, INDEX_COMMENT FROM information_schema.STATISTICS \
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY INDEX_NAME = 'PRIMARY' DESC, INDEX_NAME, SEQ_IN_INDEX",
        )
        .bind(db)
        .bind(name)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        let mut all_indexes: Vec<IndexInfo> = Vec::new();
        // MySQL 8 functional key parts have a NULL column name: such indexes cannot identify rows.
        let mut expression_indexes: Vec<String> = Vec::new();
        for r in &index_rows {
            let index = text(r, 0)?.unwrap_or_default();
            let column = text(r, 2)?;
            if column.is_none() && !expression_indexes.contains(&index) {
                expression_indexes.push(index.clone());
            }
            if all_indexes.last().is_none_or(|i| i.name != index) {
                let index_type = text(r, 3)?.unwrap_or_default().to_ascii_uppercase();
                let (kind, method) = match index_type.as_str() {
                    "FULLTEXT" => (IndexKind::Fulltext, None),
                    "SPATIAL" | "RTREE" => (IndexKind::Spatial, None),
                    other => (
                        if int(r, 1)? == Some(0) { IndexKind::Unique } else { IndexKind::Normal },
                        match other {
                            "BTREE" => Some(IndexMethod::Btree),
                            "HASH" => Some(IndexMethod::Hash),
                            _ => None,
                        },
                    ),
                };
                all_indexes.push(IndexInfo {
                    primary: index == "PRIMARY",
                    kind,
                    method,
                    comment: text(r, 4)?.filter(|c| !c.is_empty()),
                    name: index,
                    columns: Vec::new(),
                });
            }
            all_indexes.last_mut().unwrap().columns.extend(column);
        }
        let pk_columns: Vec<String> = all_indexes.iter().find(|i| i.primary).map(|i| i.columns.clone()).unwrap_or_default();
        let indexes: Vec<IndexInfo> = all_indexes.into_iter().filter(|i| !i.primary).collect();

        let column_rows = sqlx::query(
            "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_DEFAULT, EXTRA, COLUMN_COMMENT, DATA_TYPE \
             FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
        )
        .bind(db)
        .bind(name)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        let columns = column_rows
            .iter()
            .map(|r| {
                let name = text(r, 0)?.unwrap_or_default();
                let extra = text(r, 4)?.unwrap_or_default();
                let data_type = text(r, 6)?.unwrap_or_default().to_ascii_lowercase();
                Ok(ColumnInfo {
                    primary_key: pk_columns.contains(&name),
                    data_type: text(r, 1)?.unwrap_or_default(),
                    nullable: text(r, 2)?.as_deref() == Some("YES"),
                    default: default_expression(self.defaults, text(r, 3)?, &data_type, &extra),
                    auto_increment: extra.to_ascii_lowercase().contains("auto_increment"),
                    generated: extra.to_ascii_uppercase().contains("VIRTUAL GENERATED")
                        || extra.to_ascii_uppercase().contains("STORED GENERATED"),
                    comment: text(r, 5)?.filter(|c| !c.is_empty()),
                    name,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let fk_rows = sqlx::query(
            "SELECT k.CONSTRAINT_NAME, k.COLUMN_NAME, k.REFERENCED_TABLE_NAME, k.REFERENCED_COLUMN_NAME, r.UPDATE_RULE, r.DELETE_RULE, \
                    k.REFERENCED_TABLE_SCHEMA \
             FROM information_schema.KEY_COLUMN_USAGE k \
             JOIN information_schema.REFERENTIAL_CONSTRAINTS r \
               ON r.CONSTRAINT_SCHEMA = k.CONSTRAINT_SCHEMA AND r.CONSTRAINT_NAME = k.CONSTRAINT_NAME AND r.TABLE_NAME = k.TABLE_NAME \
             WHERE k.TABLE_SCHEMA = ? AND k.TABLE_NAME = ? AND k.REFERENCED_TABLE_NAME IS NOT NULL \
             ORDER BY k.CONSTRAINT_NAME, k.ORDINAL_POSITION",
        )
        .bind(db)
        .bind(name)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        let mut foreign_keys: Vec<ForeignKeyInfo> = Vec::new();
        for r in &fk_rows {
            let fk_name = text(r, 0)?.unwrap_or_default();
            if foreign_keys.last().is_none_or(|f| f.name != fk_name) {
                foreign_keys.push(ForeignKeyInfo {
                    name: fk_name,
                    columns: Vec::new(),
                    ref_table: text(r, 2)?.unwrap_or_default(),
                    ref_schema: text(r, 6)?.filter(|schema| schema != db),
                    ref_columns: Vec::new(),
                    on_update: text(r, 4)?.unwrap_or_default(),
                    on_delete: text(r, 5)?.unwrap_or_default(),
                });
            }
            let fk = foreign_keys.last_mut().unwrap();
            fk.columns.extend(text(r, 1)?);
            fk.ref_columns.extend(text(r, 3)?);
        }

        let create = sqlx::raw_sql(AssertSqlSafe(format!("SHOW CREATE TABLE {}.{}", quote_ident(db), quote_ident(name))))
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?;
        let create_sql = text(&create, 1)?.unwrap_or_default();
        let row_identity = match kind {
            ObjectKind::Table => {
                let identity_indexes: Vec<IndexInfo> = indexes.iter().filter(|i| !expression_indexes.contains(&i.name)).cloned().collect();
                derive_identity(&columns, &identity_indexes, false)
            }
            ObjectKind::View => RowIdentity::None,
        };
        Ok(TableInfo {
            name: name.to_owned(),
            kind,
            rows: if kind == ObjectKind::Table { int(&table, 3)? } else { None },
            engine: text(&table, 1)?,
            charset,
            collation,
            created: text(&table, 4)?,
            modified: text(&table, 5)?,
            comment: if kind == ObjectKind::Table { text(&table, 6)?.filter(|c| !c.is_empty()) } else { None },
            columns,
            indexes,
            foreign_keys,
            create_sql,
            row_identity,
            primary_key: pk_columns,
        })
    }

    async fn fetch_page(&self, db: &str, table: &str, req: &PageRequest) -> Result<Page> {
        let col_rows = sqlx::query(
            "SELECT COLUMN_NAME, COLUMN_TYPE FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
        )
        .bind(db)
        .bind(table)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        if col_rows.is_empty() {
            return Err(AppError::validation(format!("Table {table} not found")));
        }
        let columns = col_rows
            .iter()
            .map(|r| Ok(ColumnMeta { name: text(r, 0)?.unwrap_or_default(), type_name: text(r, 1)?.unwrap_or_default(), hidden: false }))
            .collect::<Result<Vec<_>>>()?;
        let from = format!("{}.{}", quote_ident(db), quote_ident(table));
        let select_list = columns.iter().map(|c| quote_ident(&c.name)).collect::<Vec<_>>().join(", ");
        let (sql, binds) = crate::sql::select_page(&select_list, &from, req, quote_ident)?;
        let mut q = sqlx::query(AssertSqlSafe(sql));
        for v in binds {
            q = bind(q, v);
        }
        let rows = q.fetch_all(&self.pool).await.map_err(map_err)?;
        let rows = rows
            .iter()
            .map(|r| (0..columns.len()).map(|i| read_value(r, i).map(|v| v.to_cell())).collect::<Result<Vec<_>>>())
            .collect::<Result<Vec<_>>>()?;
        Ok(Page { columns, rows })
    }

    async fn apply_changes(&self, db: &str, table: &str, identity: &RowIdentity, changes: &[RowChange]) -> Result<u64, ChangeError> {
        let info = self.table_info(db, table).await?;
        crate::sql::check_identity(identity, &info.row_identity)?;
        self.require_transactional(db, table).await?;
        info.validate_changes(changes)?;
        let changes = bit_values(&info, changes)?;
        // Matching by value must be byte-exact: case-insensitive or PAD SPACE collations would match other rows.
        let byte_exact: Vec<String> =
            info.columns.iter().filter(|c| crate::model::is_text_type(&c.data_type)).map(|c| c.name.clone()).collect();
        let target = crate::sql::ChangeTarget {
            table_ref: format!("{}.{}", quote_ident(db), quote_ident(table)),
            identity: &info.row_identity,
            quote: quote_ident,
            dialect: crate::sql::ChangeDialect { empty_insert: "() VALUES ()", null_safe_eq: "<=>", single_row: "LIMIT 1" },
            byte_exact,
            rowid: None,
            expected_columns: Vec::new(),
            complete_match: info.row_identity.columns().len() == info.columns.len(),
        };
        let statements = crate::sql::change_statements(&changes, &target)?;
        let mut tx = self.pool.begin().await.map_err(map_err)?;
        for (index, st) in statements.iter().enumerate() {
            let at = |e: sqlx::Error| ChangeError { index: Some(index), error: map_err(e) };
            if let Some(sql) = &st.ambiguity_sql {
                let mut q = sqlx::query(AssertSqlSafe(sql.clone()));
                for v in st.key_binds.iter().cloned() {
                    q = bind(q, v);
                }
                let row = q.fetch_one(&mut *tx).await.map_err(at)?;
                if int(&row, 0).map_err(|error| ChangeError { index: Some(index), error })?.unwrap_or(0) > 1 {
                    return Err(crate::sql::ambiguous(index));
                }
            }
            let mut q = sqlx::query(AssertSqlSafe(st.sql.clone()));
            for v in st.binds.iter().cloned() {
                q = bind(q, v);
            }
            let affected = q.execute(&mut *tx).await.map_err(at)?.rows_affected();
            if st.keyed && affected > 1 {
                return Err(crate::sql::several(index, affected));
            }
            if st.keyed && affected == 0 {
                // MySQL counts changed rows: a no-op update reports 0 although the row exists.
                let exists = match &st.exists_sql {
                    Some(sql) => {
                        let mut q = sqlx::query(AssertSqlSafe(sql.clone()));
                        for v in st.key_binds.iter().cloned() {
                            q = bind(q, v);
                        }
                        q.fetch_optional(&mut *tx).await.map_err(at)?.is_some()
                    }
                    None => false,
                };
                if !exists {
                    return Err(crate::sql::vanished(index));
                }
            }
        }
        tx.commit().await.map_err(map_err)?;
        Ok(statements.len() as u64)
    }

    async fn count_rows(&self, db: &str, table: &str, filters: &[Filter]) -> Result<u64> {
        let from = format!("{}.{}", quote_ident(db), quote_ident(table));
        let (sql, binds) = crate::sql::count(&from, filters, quote_ident)?;
        let mut q = sqlx::query(AssertSqlSafe(sql));
        for v in binds {
            q = bind(q, v);
        }
        let row = q.fetch_one(&self.pool).await.map_err(map_err)?;
        Ok(int(&row, 0)?.unwrap_or(0) as u64)
    }

    async fn table_design(&self, db: &str, table: &str) -> Result<TableDesign> {
        let info = self.table_info(db, table).await?;
        if info.columns.iter().any(|c| c.generated) {
            return Err(AppError::validation("Tables with generated columns cannot be edited in the table designer; use SQL"));
        }
        let mut design = TableDesign::from_info(&info);
        // Left out of the design, they are never dropped or re-created (`table_info` still lists them).
        let unmodelled = self.unmodelled_indexes(db, table).await?;
        design.indexes.retain(|index| !unmodelled.contains(&index.name));
        Ok(design)
    }

    async fn plan_table(&self, db: &str, original: Option<&TableDesign>, design: &TableDesign) -> Result<Vec<String>> {
        let Some(original) = original else {
            return crate::design::mysql::create(design);
        };
        let current = self.table_design(db, &original.name).await?;
        if current != *original {
            return Err(AppError::validation(
                "The table changed since the designer loaded it. Reopen the designer (or revert) to load its current structure, then make your changes again.",
            ));
        }
        let statements = crate::design::mysql::alter(original, design)?;
        if statements.is_empty() {
            return Ok(statements);
        }
        let info = self.table_info(db, &original.name).await?;
        if info.columns.iter().any(|c| c.generated) {
            return Err(AppError::validation("Tables with generated columns cannot be altered by the table designer"));
        }
        if statements.iter().any(|sql| sql.contains("DROP PRIMARY KEY"))
            && self.unmodelled_indexes(db, &original.name).await?.iter().any(|index| index == "PRIMARY")
        {
            return Err(AppError::validation(
                "The primary key has prefix lengths or descending parts the designer cannot preserve; change it with SQL",
            ));
        }
        // Columns a CHANGE COLUMN redefines: attributes the design does not model would be lost on them only.
        let redefined: Vec<&str> = original
            .columns
            .iter()
            .map(|c| c.name.as_str())
            .filter(|name| statements.iter().any(|sql| sql.contains(&format!("CHANGE COLUMN {} ", quote_ident(name)))))
            .collect();
        if redefined.is_empty() {
            return Ok(statements);
        }
        let columns = sqlx::query(
            "SELECT COLUMN_NAME, EXTRA, COLLATION_NAME FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?",
        )
        .bind(db)
        .bind(&original.name)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        for column in &columns {
            let name = text(column, 0)?.unwrap_or_default();
            if !redefined.contains(&name.as_str()) {
                continue;
            }
            let extra =
                text(column, 1)?.unwrap_or_default().to_ascii_uppercase().replace("AUTO_INCREMENT", "").replace("DEFAULT_GENERATED", "");
            let collation = text(column, 2)?;
            if !extra.trim().is_empty() || (collation.is_some() && collation != info.collation) {
                return Err(AppError::validation(format!(
                    "Column attributes of {name} (such as ON UPDATE or a column collation) cannot be preserved by the designer; use SQL"
                )));
            }
        }
        // Attributes SHOW CREATE TABLE writes on the column's own line, including MariaDB column-level CHECK constraints.
        for name in redefined {
            let prefix = format!("{} ", quote_ident(name));
            let Some(line) = info.create_sql.lines().find(|line| line.trim_start().starts_with(&prefix)) else { continue };
            let upper = line.to_ascii_uppercase();
            if [" COLUMN_FORMAT ", " STORAGE ", " SRID ", " INVISIBLE", " CHECK ("].iter().any(|attribute| upper.contains(attribute)) {
                return Err(AppError::validation(format!(
                    "Column {name} has attributes (such as a CHECK constraint or storage options) the designer cannot preserve; use SQL"
                )));
            }
        }
        Ok(statements)
    }

    async fn apply_ddl(&self, db: &str, statements: &[String]) -> Result<()> {
        let mut conn = self.pool.acquire().await.map_err(map_err)?;
        // Planned statements name tables without the database, as shown in the SQL preview.
        sqlx::raw_sql(AssertSqlSafe(format!("USE {}", quote_ident(db)))).execute(&mut *conn).await.map_err(map_err)?;
        for st in statements {
            sqlx::raw_sql(AssertSqlSafe(st.clone())).execute(&mut *conn).await.map_err(map_err)?;
        }
        Ok(())
    }

    async fn incoming_foreign_keys(&self, db: &str, table: &str) -> Result<Vec<IncomingForeignKey>> {
        let rows = sqlx::query(
            "SELECT CONSTRAINT_SCHEMA, TABLE_NAME, CONSTRAINT_NAME, DELETE_RULE FROM information_schema.REFERENTIAL_CONSTRAINTS \
             WHERE UNIQUE_CONSTRAINT_SCHEMA = ? AND REFERENCED_TABLE_NAME = ? \
             ORDER BY CONSTRAINT_SCHEMA, TABLE_NAME, CONSTRAINT_NAME",
        )
        .bind(db)
        .bind(table)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        let mut out = Vec::new();
        for r in &rows {
            let (schema, child) = (text(r, 0)?.unwrap_or_default(), text(r, 1)?.unwrap_or_default());
            if schema == db && child == table {
                continue;
            }
            out.push(IncomingForeignKey {
                table: if schema == db { child } else { format!("{schema}.{child}") },
                name: text(r, 2)?.unwrap_or_default(),
                on_delete: text(r, 3)?.unwrap_or_default(),
            });
        }
        Ok(out)
    }

    async fn drop_object(&self, db: &str, name: &str, kind: ObjectKind) -> Result<()> {
        let what = if kind == ObjectKind::View { "VIEW" } else { "TABLE" };
        let sql = format!("DROP {what} {}.{}", quote_ident(db), quote_ident(name));
        sqlx::raw_sql(AssertSqlSafe(sql)).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    async fn empty_table(&self, db: &str, name: &str) -> Result<()> {
        let sql = format!("DELETE FROM {}.{}", quote_ident(db), quote_ident(name));
        sqlx::raw_sql(AssertSqlSafe(sql)).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    async fn truncate_table(&self, db: &str, name: &str) -> Result<()> {
        let sql = format!("TRUNCATE TABLE {}.{}", quote_ident(db), quote_ident(name));
        sqlx::raw_sql(AssertSqlSafe(sql)).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    async fn rename_table(&self, db: &str, from: &str, to: &str) -> Result<()> {
        let sql = format!("RENAME TABLE {0}.{1} TO {0}.{2}", quote_ident(db), quote_ident(from), quote_ident(to));
        sqlx::raw_sql(AssertSqlSafe(sql)).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    async fn create_database(&self, spec: &CreateDatabase) -> Result<()> {
        let mut sql = format!("CREATE DATABASE {}", quote_ident(&spec.name));
        if let Some(cs) = spec.charset.as_deref().filter(|s| !s.is_empty()) {
            sql += &format!(" CHARACTER SET {}", plain_name(cs)?);
        }
        if let Some(co) = spec.collation.as_deref().filter(|s| !s.is_empty()) {
            sql += &format!(" COLLATE {}", plain_name(co)?);
        }
        sqlx::raw_sql(AssertSqlSafe(sql)).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    async fn drop_database(&self, name: &str) -> Result<()> {
        sqlx::raw_sql(AssertSqlSafe(format!("DROP DATABASE {}", quote_ident(name)))).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    async fn list_charsets(&self) -> Result<Vec<CharsetInfo>> {
        let rows = sqlx::query(
            "SELECT CHARACTER_SET_NAME, COLLATION_NAME, IS_DEFAULT FROM information_schema.COLLATIONS \
             WHERE CHARACTER_SET_NAME IS NOT NULL ORDER BY CHARACTER_SET_NAME, COLLATION_NAME",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        let mut out: Vec<CharsetInfo> = Vec::new();
        for r in &rows {
            let (Some(cs), Some(col)) = (text(r, 0)?, text(r, 1)?) else { continue };
            let is_default = text(r, 2)?.is_some_and(|d| d.eq_ignore_ascii_case("yes"));
            if out.last().is_none_or(|c| c.name != cs) {
                out.push(CharsetInfo { name: cs, default_collation: String::new(), collations: Vec::new() });
            }
            let entry = out.last_mut().unwrap();
            if is_default {
                entry.default_collation = col.clone();
            }
            entry.collations.push(col);
        }
        for c in &mut out {
            if c.default_collation.is_empty() {
                c.default_collation = c.collations[0].clone();
            }
        }
        Ok(out)
    }
}
