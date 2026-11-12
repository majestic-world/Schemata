use crate::model::derive_identity;
use crate::transfer::{DumpScript, ImportBatch, ImportReport, MAX_REPORTED_FAILURES, RowFailure, RowStream, ScriptRunReport, StreamItem};
use crate::{
    AppError, CellValue, ChangeError, CharsetInfo, ColumnInfo, ColumnMeta, ConnectParams, CreateDatabase, DatabaseInfo, Driver, Engine,
    ErrorKind, Filter, ForeignKeyInfo, IndexInfo, IndexKind, IndexMethod, ObjectInfo, ObjectKind, Page, PageRequest, Result, RowChange,
    RowIdentity, ScriptResult, StatementError, StatementOutcome, TableDesign, TableInfo, Value, split_statements,
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
use std::collections::HashMap;
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

pub struct MySqlDriver {
    pool: MySqlPool,
    connect_options: MySqlConnectOptions,
    engine: Engine,
    version: String,
    uncached_statistics: bool,
    /// Server thread id per running execution, used by `KILL QUERY`.
    running: Mutex<HashMap<String, RunningQuery>>,
    /// Editor sessions by key; dropping the driver closes them, and the server rolls back their transactions.
    editor_sessions: Mutex<HashMap<String, EditorSession>>,
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
        sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed => AppError::connection("Could not reach the server"),
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
            Err(_) => Value::DateTime(zero_date(row, i, "0000-00-00")),
        },
        "DATETIME" | "TIMESTAMP" => match row.try_get_unchecked::<chrono::NaiveDateTime, _>(i) {
            Ok(d) => Value::DateTime(d.format("%Y-%m-%d %H:%M:%S%.f").to_string()),
            Err(_) => Value::DateTime(zero_date(row, i, "0000-00-00 00:00:00")),
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

/// Zero dates ('0000-00-00') are valid in MySQL but not representable in chrono.
fn zero_date(row: &MySqlRow, i: usize, fallback: &str) -> String {
    match row.try_get_unchecked::<Vec<u8>, _>(i) {
        Ok(b) if !b.is_empty() && b.iter().all(|c| c.is_ascii()) => String::from_utf8_lossy(&b).into_owned(),
        _ => fallback.to_owned(),
    }
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

/// Turns `information_schema.COLUMNS.COLUMN_DEFAULT` into a SQL expression. MariaDB already reports
/// expressions; MySQL reports literals unquoted and flags expressions with `DEFAULT_GENERATED`.
fn default_expression(engine: Engine, raw: Option<String>, data_type: &str, extra: &str) -> Option<String> {
    let raw = raw?;
    if engine == Engine::MariaDb {
        return (!raw.eq_ignore_ascii_case("NULL")).then_some(raw);
    }
    if extra.to_ascii_uppercase().contains("DEFAULT_GENERATED") {
        let upper = raw.to_ascii_uppercase();
        return Some(if upper.starts_with("CURRENT_TIMESTAMP") || upper.starts_with("NOW") { raw } else { format!("({raw})") });
    }
    if is_numeric_type(data_type) { Some(raw) } else { Some(crate::design::mysql::literal(&raw)) }
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

/// Removes `DEFINER=user@host` so a dumped view can be created by whoever restores it.
fn strip_definer(sql: &str) -> String {
    let Some(start) = sql.find("DEFINER=") else { return sql.to_owned() };
    let rest = &sql[start..];
    let end = rest.find(" SQL SECURITY").or_else(|| rest.find(" VIEW")).unwrap_or(rest.len());
    format!("{}{}", &sql[..start], rest[end..].trim_start())
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
            version,
            uncached_statistics,
            running: Mutex::new(HashMap::new()),
            editor_sessions: Mutex::new(HashMap::new()),
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
        let slot = self.editor_sessions.lock().unwrap().entry(key.to_owned()).or_default().clone();
        let mut guard = slot.lock_owned().await;
        let conn = match guard.take() {
            Some(mut conn) => {
                // The server may have closed it meanwhile (wait_timeout, KILL, restart). Reconnecting silently would hide
                // the rolled-back transaction, so this run fails and the next one opens a fresh session.
                if conn.ping().await.is_err() {
                    return Err(AppError::connection(
                        "The server closed this editor session; any open transaction was rolled back. Run again to reconnect.",
                    ));
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
        if let Some(db) = db {
            sqlx::raw_sql(AssertSqlSafe(format!("USE {}", quote_ident(db)))).execute(&mut *conn).await.map_err(map_err)?;
        }
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
                Ok(o) => outcomes.push(o),
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
        if let Some(db) = db {
            sqlx::raw_sql(AssertSqlSafe(format!("USE {}", quote_ident(db)))).execute(&mut *conn).await.map_err(map_err)?;
        }
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

    async fn run_statement(conn: &mut sqlx::MySqlConnection, sql: &str, row_limit: usize) -> Result<StatementOutcome> {
        let started = Instant::now();
        let mut stream = sqlx::raw_sql(AssertSqlSafe(sql)).fetch_many(&mut *conn);
        let mut columns: Option<Vec<ColumnMeta>> = None;
        let mut rows = Vec::new();
        let mut has_more = false;
        let mut affected = 0u64;
        let mut last_insert_id = None;
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
                Either::Left(done) => {
                    affected += done.rows_affected();
                    if done.last_insert_id() > 0 {
                        last_insert_id = Some(done.last_insert_id());
                    }
                }
            }
        }
        drop(stream);
        let duration_ms = started.elapsed().as_millis() as u64;
        if columns.is_none() && returns_rows(sql) {
            // Empty result sets carry no rows to read column names from; prepare the statement instead.
            columns = statement_columns(conn, sql).await;
        }
        if columns.is_some() || returns_rows(sql) {
            Ok(StatementOutcome::Rows { statement: sql.to_owned(), columns: columns.unwrap_or_default(), rows, has_more, duration_ms })
        } else {
            Ok(StatementOutcome::Affected { statement: sql.to_owned(), rows_affected: affected, last_insert_id, duration_ms })
        }
    }
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
        if let Some(db) = db {
            sqlx::raw_sql(AssertSqlSafe(format!("USE {}", quote_ident(db)))).execute(&mut *conn).await.map_err(map_err)?;
        }
        let sql = sql.to_owned();
        tokio::spawn(async move {
            let result: Result<()> = async {
                let mut stream = sqlx::raw_sql(AssertSqlSafe(sql.clone())).fetch(&mut *conn);
                let Some(first) = stream.try_next().await.map_err(map_err)? else {
                    drop(stream);
                    let columns = statement_columns(&mut conn, &sql).await.unwrap_or_default();
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
            .await;
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
        skip_errors: bool,
        empty_first: bool,
    ) -> Result<ImportReport> {
        self.require_transactional(db, table).await?;
        self.table_info(db, table).await?.validate_writable_columns(columns.iter().map(String::as_str))?;
        let cols = columns.iter().map(|c| quote_ident(c)).collect::<Vec<_>>().join(", ");
        let one = format!("({})", vec!["?"; columns.len()].join(", "));
        let prefix = format!("INSERT INTO {}.{} ({cols}) VALUES ", quote_ident(db), quote_ident(table));
        // MySQL allows at most 65,535 placeholders per statement.
        let per_statement = (60_000 / columns.len().max(1)).max(1);
        let mut tx = self.pool.begin().await.map_err(map_err)?;
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
        mut statements: tokio::sync::mpsc::Receiver<Result<String>>,
        stop_on_error: bool,
        cancel: CancellationToken,
    ) -> Result<ScriptRunReport> {
        let mut conn = self.pool.acquire().await.map_err(map_err)?;
        if let Some(db) = db {
            sqlx::raw_sql(AssertSqlSafe(format!("USE {}", quote_ident(db)))).execute(&mut *conn).await.map_err(map_err)?;
        }
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
        // Never return a dirty script session to the pool, including on reader/decode failures.
        sqlx::raw_sql("ROLLBACK").execute(&mut *conn).await.map_err(map_err)?;
        sqlx::raw_sql("SET FOREIGN_KEY_CHECKS = 1").execute(&mut *conn).await.map_err(map_err)?;
        if cancel.is_cancelled() {
            Err(AppError::cancelled())
        } else if let Some(error) = input_error {
            Err(error)
        } else {
            Ok(report)
        }
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
        let mut objects_sql: Vec<String> = crate::transfer::order_views(views, quote_ident).into_iter().map(|(_, sql)| sql).collect();
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
            objects_sql.push(format!(
                "DROP TRIGGER IF EXISTS {0};
DELIMITER ;;
CREATE TRIGGER {0} {timing} {event} ON {1} FOR EACH ROW {body};;
DELIMITER ;",
                quote_ident(&name),
                quote_ident(&table)
            ));
        }
        Ok(DumpScript {
            prologue: vec!["SET NAMES utf8mb4;".into(), "SET FOREIGN_KEY_CHECKS = 0;".into()],
            tables,
            data_tail: Vec::new(),
            objects: objects_sql,
            epilogue: vec!["SET FOREIGN_KEY_CHECKS = 1;".into()],
        })
    }

    async fn fetch_more(&self, db: Option<&str>, statement: &str, offset: u64, limit: u64) -> Result<Page> {
        let mut conn = self.pool.acquire().await.map_err(map_err)?;
        Self::more_rows(&mut conn, db, statement, offset, limit).await
    }

    async fn fetch_more_in_session(&self, session: &str, db: Option<&str>, statement: &str, offset: u64, limit: u64) -> Result<Page> {
        let (mut guard, mut conn) = self.checkout_editor_session(session).await?;
        let result = Self::more_rows(&mut conn, db, statement, offset, limit).await;
        if !result.as_ref().is_err_and(|e| e.kind == ErrorKind::Connection) {
            *guard = Some(conn);
        }
        result
    }

    async fn close_editor_session(&self, session: &str) -> Result<()> {
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
                    default: default_expression(self.engine, text(r, 3)?, &data_type, &extra),
                    auto_increment: extra.to_ascii_lowercase().contains("auto_increment"),
                    generated: extra.to_ascii_uppercase().contains("VIRTUAL GENERATED")
                        || extra.to_ascii_uppercase().contains("STORED GENERATED"),
                    comment: text(r, 5)?.filter(|c| !c.is_empty()),
                    name,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let fk_rows = sqlx::query(
            "SELECT k.CONSTRAINT_NAME, k.COLUMN_NAME, k.REFERENCED_TABLE_NAME, k.REFERENCED_COLUMN_NAME, r.UPDATE_RULE, r.DELETE_RULE \
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
                let identity_indexes: Vec<IndexInfo> =
                    indexes.iter().filter(|i| !expression_indexes.contains(&i.name)).cloned().collect();
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

    async fn apply_changes(&self, db: &str, table: &str, changes: &[RowChange]) -> Result<u64, ChangeError> {
        let info = self.table_info(db, table).await?;
        self.require_transactional(db, table).await?;
        info.validate_changes(changes)?;
        let table_ref = format!("{}.{}", quote_ident(db), quote_ident(table));
        let dialect = crate::sql::ChangeDialect { empty_insert: "() VALUES ()", null_safe_eq: "<=>", single_row: "LIMIT 1" };
        // Matching by value must be byte-exact: case-insensitive or PAD SPACE collations would match other rows.
        let byte_exact: Vec<String> =
            info.columns.iter().filter(|c| crate::model::is_text_type(&c.data_type)).map(|c| c.name.clone()).collect();
        let statements =
            crate::sql::change_statements_byte_exact(changes, &table_ref, &info.row_identity, quote_ident, &dialect, &byte_exact)?;
        let mut tx = self.pool.begin().await.map_err(map_err)?;
        for (index, st) in statements.iter().enumerate() {
            let at = |e: sqlx::Error| ChangeError { index: Some(index), error: map_err(e) };
            let mut q = sqlx::query(AssertSqlSafe(st.sql.clone()));
            for v in st.binds.iter().cloned() {
                q = bind(q, v);
            }
            let affected = q.execute(&mut *tx).await.map_err(at)?.rows_affected();
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

    async fn plan_table(&self, db: &str, original: Option<&TableDesign>, design: &TableDesign) -> Result<Vec<String>> {
        let Some(original) = original else {
            return crate::design::mysql::create(design);
        };
        let statements = crate::design::mysql::alter(original, design)?;
        if statements.is_empty() {
            return Ok(statements);
        }
        let info = self.table_info(db, &original.name).await?;
        if info.columns.iter().any(|c| c.generated) {
            return Err(AppError::validation("Tables with generated columns cannot be altered by the table designer"));
        }
        if statements.iter().any(|sql| sql.contains("CHANGE COLUMN")) {
            let columns =
                sqlx::query("SELECT EXTRA, COLLATION_NAME FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?")
                    .bind(db)
                    .bind(&original.name)
                    .fetch_all(&self.pool)
                    .await
                    .map_err(map_err)?;
            for column in &columns {
                let extra = text(column, 0)?
                    .unwrap_or_default()
                    .to_ascii_uppercase()
                    .replace("AUTO_INCREMENT", "")
                    .replace("DEFAULT_GENERATED", "");
                let collation = text(column, 1)?;
                if !extra.trim().is_empty() || (collation.is_some() && collation != info.collation) {
                    return Err(AppError::validation(
                        "Column attributes (such as ON UPDATE or a column collation) cannot be preserved by the designer; use SQL",
                    ));
                }
            }
            let upper = info.create_sql.to_ascii_uppercase();
            if ["COLUMN_FORMAT", " STORAGE ", " SRID ", " INVISIBLE"].iter().any(|attribute| upper.contains(attribute)) {
                return Err(AppError::validation("This table contains column storage attributes the designer cannot preserve; use SQL"));
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

    async fn drop_object(&self, db: &str, name: &str, kind: ObjectKind) -> Result<()> {
        let what = if kind == ObjectKind::View { "VIEW" } else { "TABLE" };
        let sql = format!("DROP {what} {}.{}", quote_ident(db), quote_ident(name));
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
