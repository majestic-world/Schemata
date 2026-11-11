use crate::model::derive_identity;
use crate::{
    AppError, CellValue, ColumnInfo, ColumnMeta, CreateDatabase, DatabaseInfo, Driver, Engine, ErrorKind,
    ForeignKeyInfo, IndexInfo, ObjectInfo, ObjectKind, Result, RowIdentity, ScriptResult, StatementError,
    StatementOutcome, TableInfo, Value, split_statements,
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

fn collect<T>(stmt: &mut rusqlite::Statement<'_>, params: impl rusqlite::Params, f: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>) -> Result<Vec<T>> {
    stmt.query_map(params, f).map_err(map_err)?.collect::<rusqlite::Result<Vec<T>>>().map_err(map_err)
}

fn read_table_info(c: &Connection, db: &str, name: &str) -> Result<TableInfo> {
    let schema = quote_ident(db);
    let (kind, create_sql): (String, Option<String>) = c
        .query_row(
            &format!("SELECT type, sql FROM {schema}.sqlite_schema WHERE name = ?1 AND type IN ('table', 'view')"),
            [name],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AppError::validation(format!("Table {name} not found")),
            e => map_err(e),
        })?;
    let create_sql = create_sql.unwrap_or_default();
    let kind = if kind == "view" { ObjectKind::View } else { ObjectKind::Table };
    let upper_sql = create_sql.to_ascii_uppercase();

    let mut stmt = c
        .prepare("SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?1, ?2) ORDER BY cid")
        .map_err(map_err)?;
    let raw = collect(&mut stmt, [name, db], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, i64>(4)?))
    })?;
    let pk_count = raw.iter().filter(|c| c.4 > 0).count();
    let columns: Vec<ColumnInfo> = raw
        .into_iter()
        .map(|(name, data_type, notnull, default, pk)| ColumnInfo {
            auto_increment: pk > 0 && pk_count == 1 && data_type.eq_ignore_ascii_case("INTEGER") && upper_sql.contains("AUTOINCREMENT"),
            nullable: !notnull && pk == 0,
            primary_key: pk > 0,
            name,
            data_type,
            default,
            comment: None,
        })
        .collect();

    let mut stmt = c
        .prepare("SELECT name, \"unique\" FROM pragma_index_list(?1, ?2) WHERE origin <> 'pk' ORDER BY name")
        .map_err(map_err)?;
    let index_rows = collect(&mut stmt, [name, db], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)))?;
    let mut indexes = Vec::new();
    for (index, unique) in index_rows {
        let mut stmt = c.prepare("SELECT name FROM pragma_index_info(?1, ?2) ORDER BY seqno").map_err(map_err)?;
        let cols = collect(&mut stmt, [index.as_str(), db], |r| r.get::<_, Option<String>>(0))?;
        indexes.push(IndexInfo { name: index, columns: cols.into_iter().flatten().collect(), unique, primary: false });
    }

    let mut stmt = c
        .prepare("SELECT id, \"table\", \"from\", \"to\", on_update, on_delete FROM pragma_foreign_key_list(?1, ?2) ORDER BY id, seq")
        .map_err(map_err)?;
    let fk_rows = collect(&mut stmt, [name, db], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?))
    })?;
    let mut foreign_keys: Vec<(i64, ForeignKeyInfo)> = Vec::new();
    for (id, table, from, to, on_update, on_delete) in fk_rows {
        if foreign_keys.last().is_none_or(|(last, _)| *last != id) {
            foreign_keys.push((id, ForeignKeyInfo {
                name: format!("fk_{name}_{id}"),
                columns: Vec::new(),
                ref_table: table,
                ref_columns: Vec::new(),
                on_update,
                on_delete,
            }));
        }
        let fk = &mut foreign_keys.last_mut().unwrap().1;
        fk.columns.push(from);
        fk.ref_columns.extend(to);
    }

    let rows = match kind {
        ObjectKind::Table => Some(
            c.query_row(&format!("SELECT count(*) FROM {schema}.{}", quote_ident(name)), [], |r| r.get(0)).map_err(map_err)?,
        ),
        ObjectKind::View => None,
    };
    let row_identity = match kind {
        ObjectKind::Table => derive_identity(&columns, &indexes, !upper_sql.contains("WITHOUT ROWID")),
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
        rows,
        created: None,
        modified: None,
        comment: None,
        create_sql,
        row_identity,
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

    async fn table_info(&self, db: &str, name: &str) -> Result<TableInfo> {
        let (db, name) = (db.to_owned(), name.to_owned());
        self.with_conn(move |c| read_table_info(c, &db, &name)).await
    }

    async fn create_database(&self, _spec: &CreateDatabase) -> Result<()> {
        Err(AppError::validation("SQLite databases are files: attach a database file instead"))
    }

    async fn drop_database(&self, name: &str) -> Result<()> {
        if name == "main" {
            return Err(AppError::validation("The main database of a SQLite connection cannot be detached"));
        }
        let sql = format!("DETACH DATABASE {}", quote_ident(name));
        self.with_conn(move |c| c.execute_batch(&sql).map_err(map_err)).await
    }

    async fn attach_database(&self, path: &str, alias: &str) -> Result<()> {
        let sql = format!("ATTACH DATABASE ?1 AS {}", quote_ident(alias));
        let path = path.to_owned();
        self.with_conn(move |c| c.execute(&sql, [path]).map(|_| ()).map_err(map_err)).await
    }
}
