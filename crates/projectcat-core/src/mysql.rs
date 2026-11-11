use crate::{
    AppError, CellValue, ColumnMeta, ConnectParams, DatabaseInfo, Driver, Engine, ErrorKind, ObjectInfo, ObjectKind,
    Result, ScriptResult, StatementError, StatementOutcome, Value, split_statements,
};
use async_trait::async_trait;
use futures::TryStreamExt;
use sqlx::mysql::{MySqlConnectOptions, MySqlPool, MySqlPoolOptions, MySqlRow, MySqlSslMode};
use sqlx::{AssertSqlSafe, Column, Either, Row, TypeInfo, ValueRef};
use sqlx_mysql::types::MySqlTime;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct MySqlDriver {
    pool: MySqlPool,
    engine: Engine,
    version: String,
    /// Server thread id per running execution, used by `KILL QUERY`.
    running: Mutex<HashMap<String, u64>>,
}

pub(crate) fn quote_ident(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
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

/// Statements whose results are row sets even when empty.
fn returns_rows(sql: &str) -> bool {
    let first = sql
        .trim_start_matches(|c: char| c.is_whitespace() || c == '(')
        .split(|c: char| !c.is_alphanumeric())
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    matches!(first.as_str(), "SELECT" | "SHOW" | "DESCRIBE" | "DESC" | "EXPLAIN" | "WITH" | "VALUES" | "TABLE" | "CHECKSUM" | "ANALYZE" | "CHECK" | "OPTIMIZE" | "REPAIR")
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
            .connect_with(opts)
            .await
            .map_err(map_err)?;
        let version: String = sqlx::query_scalar("SELECT VERSION()").fetch_one(&pool).await.map_err(map_err)?;
        let engine = if version.to_ascii_lowercase().contains("mariadb") { Engine::MariaDb } else { Engine::MySql };
        Ok(Self { pool, engine, version, running: Mutex::new(HashMap::new()) })
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
        let duration_ms = started.elapsed().as_millis() as u64;
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
        let rows = sqlx::query(
            "SELECT TABLE_NAME, TABLE_TYPE, CAST(TABLE_ROWS AS SIGNED), \
                    DATE_FORMAT(CREATE_TIME, '%Y-%m-%d %H:%i:%s'), DATE_FORMAT(UPDATE_TIME, '%Y-%m-%d %H:%i:%s'), TABLE_COMMENT \
             FROM information_schema.TABLES WHERE TABLE_SCHEMA = ? \
             ORDER BY TABLE_TYPE LIKE '%VIEW', TABLE_NAME",
        )
        .bind(db)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
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
        let statements = split_statements(sql, self.engine);
        let mut conn = self.pool.acquire().await.map_err(map_err)?;
        if let Some(db) = db {
            sqlx::raw_sql(AssertSqlSafe(format!("USE {}", quote_ident(db)))).execute(&mut *conn).await.map_err(map_err)?;
        }
        if let Some(id) = execution_id {
            let thread: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()").fetch_one(&mut *conn).await.map_err(map_err)?;
            self.running.lock().unwrap().insert(id.to_owned(), thread);
        }
        let mut outcomes = Vec::new();
        let mut error = None;
        for (index, statement) in statements.into_iter().enumerate() {
            match Self::run_statement(&mut conn, &statement, row_limit).await {
                Ok(o) => outcomes.push(o),
                Err(e) => {
                    error = Some(StatementError { index, statement, error: e });
                    break;
                }
            }
        }
        if let Some(id) = execution_id {
            self.running.lock().unwrap().remove(id);
        }
        Ok(ScriptResult { outcomes, error })
    }

    async fn cancel(&self, execution_id: &str) -> Result<()> {
        let thread = self.running.lock().unwrap().get(execution_id).copied();
        if let Some(thread) = thread {
            sqlx::raw_sql(AssertSqlSafe(format!("KILL QUERY {thread}"))).execute(&self.pool).await.map_err(map_err)?;
        }
        Ok(())
    }
}
