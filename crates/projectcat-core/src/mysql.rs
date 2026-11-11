use crate::model::derive_identity;
use crate::{
    AppError, CellValue, ChangeError, RowChange, TableDesign, CharsetInfo, ColumnInfo, ColumnMeta, ConnectParams, CreateDatabase, DatabaseInfo, Driver,
    Engine, ErrorKind, Filter, ForeignKeyInfo, IndexInfo, ObjectInfo, ObjectKind, Page, PageRequest, Result, RowIdentity,
    ScriptResult, StatementError, StatementOutcome, TableInfo, Value, split_statements,
};
use sqlx::MySql;
use sqlx::mysql::MySqlArguments;
use sqlx::query::Query;
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
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint" | "decimal" | "numeric" | "float" | "double" | "real" | "bit" | "year" | "bool" | "boolean"
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
    if is_numeric_type(data_type) { Some(raw) } else { Some(format!("'{}'", raw.replace('\'', "''"))) }
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
        drop(stream);
        let duration_ms = started.elapsed().as_millis() as u64;
        if columns.is_none() && returns_rows(sql) {
            // Empty result sets carry no rows to read column names from; prepare the statement instead.
            columns = sqlx::Executor::prepare(&mut *conn, sqlx::SqlSafeStr::into_sql_str(AssertSqlSafe(sql)))
                .await
                .ok()
                .map(|st| {
                    use sqlx::Statement;
                    st.columns()
                        .iter()
                        .map(|c| ColumnMeta { name: c.name().to_owned(), type_name: c.type_info().name().to_owned(), hidden: false })
                        .collect()
                });
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

    async fn fetch_more(&self, db: Option<&str>, statement: &str, offset: u64, limit: u64) -> Result<Page> {
        let mut conn = self.pool.acquire().await.map_err(map_err)?;
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

    async fn list_columns(&self, db: &str) -> Result<Vec<(String, Vec<String>)>> {
        let rows = sqlx::query("SELECT TABLE_NAME, COLUMN_NAME FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = ? ORDER BY TABLE_NAME, ORDINAL_POSITION")
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
        let thread = self.running.lock().unwrap().get(execution_id).copied();
        if let Some(thread) = thread {
            sqlx::raw_sql(AssertSqlSafe(format!("KILL QUERY {thread}"))).execute(&self.pool).await.map_err(map_err)?;
        }
        Ok(())
    }

    async fn table_info(&self, db: &str, name: &str) -> Result<TableInfo> {
        let table = sqlx::query(
            "SELECT TABLE_TYPE, ENGINE, TABLE_COLLATION, CAST(TABLE_ROWS AS SIGNED), \
                    DATE_FORMAT(CREATE_TIME, '%Y-%m-%d %H:%i:%s'), DATE_FORMAT(UPDATE_TIME, '%Y-%m-%d %H:%i:%s'), TABLE_COMMENT \
             FROM information_schema.TABLES WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?",
        )
        .bind(db)
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?
        .ok_or_else(|| AppError::validation(format!("Table {name} not found")))?;
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
            "SELECT INDEX_NAME, NON_UNIQUE, COLUMN_NAME FROM information_schema.STATISTICS \
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY INDEX_NAME = 'PRIMARY' DESC, INDEX_NAME, SEQ_IN_INDEX",
        )
        .bind(db)
        .bind(name)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        let mut all_indexes: Vec<IndexInfo> = Vec::new();
        for r in &index_rows {
            let index = text(r, 0)?.unwrap_or_default();
            let column = text(r, 2)?;
            if all_indexes.last().is_none_or(|i| i.name != index) {
                all_indexes.push(IndexInfo { primary: index == "PRIMARY", unique: int(r, 1)? == Some(0), name: index, columns: Vec::new() });
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
            ObjectKind::Table => derive_identity(&columns, &indexes, false),
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
        let table_ref = format!("{}.{}", quote_ident(db), quote_ident(table));
        let statements = crate::sql::change_statements(changes, &table_ref, &info.row_identity, quote_ident, "() VALUES ()")?;
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
        match original {
            None => crate::design::mysql::create(db, design),
            Some(original) => crate::design::mysql::alter(db, original, design),
        }
    }

    async fn apply_ddl(&self, _db: &str, statements: &[String]) -> Result<()> {
        let mut conn = self.pool.acquire().await.map_err(map_err)?;
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
