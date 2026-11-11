use crate::{
    AppError, CellValue, ColumnMeta, DatabaseInfo, Driver, Engine, ErrorKind, ObjectInfo, ObjectKind, Result,
    ScriptResult, StatementError, StatementOutcome, Value, split_statements,
};
use async_trait::async_trait;
use rusqlite::{Connection, OpenFlags, types::ValueRef};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub struct SqliteDriver {
    conn: Arc<Mutex<Connection>>,
    interrupt: rusqlite::InterruptHandle,
    version: String,
}

pub(crate) fn map_err(e: rusqlite::Error) -> AppError {
    match &e {
        rusqlite::Error::SqliteFailure(f, msg) => {
            let message = msg.clone().unwrap_or_else(|| e.to_string());
            match f.code {
                rusqlite::ErrorCode::OperationInterrupted => AppError::cancelled(),
                rusqlite::ErrorCode::CannotOpen | rusqlite::ErrorCode::NotADatabase => {
                    AppError::new(ErrorKind::Connection, message)
                }
                _ => AppError::sql(message),
            }
        }
        _ => AppError::sql(e.to_string()),
    }
}

pub(crate) fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

pub(crate) fn read_value(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Int(i),
        ValueRef::Real(f) => Value::Float(f),
        ValueRef::Text(t) => Value::Text(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Value::Bytes(b.to_vec()),
    }
}

impl SqliteDriver {
    pub async fn open(path: &str, create: bool) -> Result<Self> {
        let path = path.to_owned();
        tokio::task::spawn_blocking(move || {
            if !create && !Path::new(&path).exists() {
                return Err(AppError::connection(format!("Database file not found: {path}")));
            }
            let mut flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI;
            if create {
                flags |= OpenFlags::SQLITE_OPEN_CREATE;
            }
            let conn = Connection::open_with_flags(&path, flags).map_err(|e| AppError::connection(e.to_string()))?;
            // Validates that the file really is a SQLite database.
            conn.query_row("SELECT count(*) FROM sqlite_schema", [], |_| Ok(()))
                .map_err(|e| AppError::connection(format!("Not a SQLite database: {e}")))?;
            conn.execute_batch("PRAGMA foreign_keys = ON;").map_err(map_err)?;
            Ok(SqliteDriver {
                interrupt: conn.get_interrupt_handle(),
                conn: Arc::new(Mutex::new(conn)),
                version: rusqlite::version().to_owned(),
            })
        })
        .await
        .map_err(|e| AppError::io(e.to_string()))?
    }

    /// Runs `f` on the connection in the blocking pool.
    pub(crate) async fn with_conn<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = conn.lock().unwrap_or_else(|p| p.into_inner());
            f(&mut guard)
        })
        .await
        .map_err(|e| AppError::io(e.to_string()))?
    }
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
    Ok(StatementOutcome::Rows {
        statement: sql.to_owned(),
        columns,
        rows,
        has_more,
        duration_ms: started.elapsed().as_millis() as u64,
    })
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
            let names = stmt
                .query_map([], |r| r.get::<_, String>(1))
                .map_err(map_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(map_err)?;
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
                 WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY type, name"
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

    async fn execute_script(&self, _db: Option<&str>, sql: &str, row_limit: usize, _execution_id: Option<&str>) -> Result<ScriptResult> {
        let statements = split_statements(sql, Engine::Sqlite);
        self.with_conn(move |c| {
            let mut outcomes = Vec::new();
            for (index, statement) in statements.into_iter().enumerate() {
                match run_statement(c, &statement, row_limit) {
                    Ok(outcome) => outcomes.push(outcome),
                    Err(error) => {
                        return Ok(ScriptResult { outcomes, error: Some(StatementError { index, statement, error }) });
                    }
                }
            }
            Ok(ScriptResult { outcomes, error: None })
        })
        .await
    }

    async fn cancel(&self, _execution_id: &str) -> Result<()> {
        self.interrupt.interrupt();
        Ok(())
    }
}
