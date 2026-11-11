use crate::model::derive_identity;
use crate::{
    AppError, CellValue, ChangeError, RowChange, TableDesign, ColumnInfo, ColumnMeta, CreateDatabase, DatabaseInfo, Driver, Engine, ErrorKind, Filter,
    ForeignKeyInfo, IndexInfo, ObjectInfo, ObjectKind, Page, PageRequest, ROWID_COLUMN, Result, RowIdentity,
    ScriptResult, StatementError, StatementOutcome, TableInfo, Value, split_statements,
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

pub(crate) fn to_sql(v: &Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as S;
    match v {
        Value::Null => S::Null,
        Value::Int(i) => S::Integer(*i),
        Value::UInt(u) => i64::try_from(*u).map(S::Integer).unwrap_or_else(|_| S::Text(u.to_string())),
        Value::Float(f) => S::Real(*f),
        Value::Decimal(s) | Value::Text(s) | Value::DateTime(s) | Value::Json(s) => S::Text(s.clone()),
        Value::Bytes(b) => S::Blob(b.clone()),
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

fn read_page(c: &Connection, db: &str, table: &str, req: &PageRequest) -> Result<Page> {
    let info = read_structure(c, db, table)?;
    let rowid = info.row_identity == RowIdentity::RowId;
    let select_list = if rowid { format!("rowid AS {}, *", quote_ident(ROWID_COLUMN)) } else { "*".to_owned() };
    let from = format!("{}.{}", quote_ident(db), quote_ident(table));
    let (sql, binds) = crate::sql::select_page(&select_list, &from, req, quote_ident)?;
    let mut columns: Vec<ColumnMeta> = Vec::new();
    if rowid {
        columns.push(ColumnMeta { name: ROWID_COLUMN.to_owned(), type_name: "INTEGER".into(), hidden: true });
    }
    columns.extend(info.columns.iter().map(|col| ColumnMeta { name: col.name.clone(), type_name: col.data_type.clone(), hidden: false }));
    let mut stmt = c.prepare(&sql).map_err(map_err)?;
    let n = columns.len();
    let params: Vec<rusqlite::types::Value> = binds.iter().map(to_sql).collect();
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

fn run_ddl(c: &mut Connection, statements: &[String]) -> Result<()> {
    use crate::design::sqlite::{FK_OFF, FK_ON};
    let toggles_fk = statements.first().map(String::as_str) == Some(FK_OFF);
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
    if toggles_fk {
        c.execute_batch(FK_ON).map_err(map_err)?;
    }
    result
}

fn read_structure(c: &Connection, db: &str, name: &str) -> Result<TableInfo> {
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
        created: None,
        rows: None,
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

    async fn fetch_more(&self, _db: Option<&str>, statement: &str, offset: u64, limit: u64) -> Result<Page> {
        let sql = crate::sql::more_rows(statement);
        self.with_conn(move |c| {
            let mut stmt = c.prepare(&sql).map_err(map_err)?;
            let columns: Vec<ColumnMeta> = stmt
                .columns()
                .map(|col| ColumnMeta { name: col.name().to_owned(), type_name: col.decl_type().unwrap_or("").to_owned(), hidden: false })
                .collect();
            let n = columns.len();
            let rows = collect(&mut stmt, [limit as i64, offset as i64], |r| Ok((0..n).map(|i| read_value(r.get_ref_unwrap(i)).to_cell()).collect()))?;
            Ok(Page { columns, rows })
        })
        .await
    }

    async fn list_columns(&self, db: &str) -> Result<Vec<(String, Vec<String>)>> {
        let db = db.to_owned();
        self.with_conn(move |c| {
            let sql = format!(
                "SELECT m.name, p.name FROM {}.sqlite_schema m JOIN pragma_table_info(m.name, ?1) p \
                 WHERE m.type IN ('table', 'view') AND m.name NOT LIKE 'sqlite_%' ORDER BY m.name, p.cid",
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

    async fn cancel(&self, _execution_id: &str) -> Result<()> {
        self.interrupt.interrupt();
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

    async fn apply_changes(&self, db: &str, table: &str, changes: &[RowChange]) -> Result<u64, ChangeError> {
        let (db, table, changes) = (db.to_owned(), table.to_owned(), changes.to_vec());
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = conn.lock().unwrap_or_else(|p| p.into_inner());
            let info = read_structure(&guard, &db, &table)?;
            let table_ref = format!("{}.{}", quote_ident(&db), quote_ident(&table));
            let statements = crate::sql::change_statements(&changes, &table_ref, &info.row_identity, quote_ident, "DEFAULT VALUES")?;
            let tx = guard.transaction().map_err(map_err)?;
            for (index, st) in statements.iter().enumerate() {
                let params: Vec<rusqlite::types::Value> = st.binds.iter().map(to_sql).collect();
                let affected = tx
                    .execute(&st.sql, rusqlite::params_from_iter(params))
                    .map_err(|e| ChangeError { index: Some(index), error: map_err(e) })?;
                if st.keyed && affected == 0 {
                    return Err(crate::sql::vanished(index));
                }
            }
            tx.commit().map_err(map_err)?;
            Ok(statements.len() as u64)
        })
        .await
        .map_err(|e| ChangeError::from(AppError::io(e.to_string())))?
    }

    async fn count_rows(&self, db: &str, table: &str, filters: &[Filter]) -> Result<u64> {
        let from = format!("{}.{}", quote_ident(db), quote_ident(table));
        let (sql, binds) = crate::sql::count(&from, filters, quote_ident)?;
        self.with_conn(move |c| {
            let params: Vec<rusqlite::types::Value> = binds.iter().map(to_sql).collect();
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
            crate::design::sqlite::alter(&db, &original, &design, deps)
        })
        .await
    }

    async fn apply_ddl(&self, _db: &str, statements: &[String]) -> Result<()> {
        let statements = statements.to_vec();
        self.with_conn(move |c| run_ddl(c, &statements)).await
    }

    async fn drop_object(&self, db: &str, name: &str, kind: ObjectKind) -> Result<()> {
        let what = if kind == ObjectKind::View { "VIEW" } else { "TABLE" };
        let sql = format!("DROP {what} {}.{}", quote_ident(db), quote_ident(name));
        self.with_conn(move |c| c.execute_batch(&sql).map_err(map_err)).await
    }

    async fn truncate_table(&self, db: &str, name: &str) -> Result<()> {
        let sql = format!("DELETE FROM {}.{}", quote_ident(db), quote_ident(name));
        self.with_conn(move |c| c.execute_batch(&sql).map_err(map_err)).await
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
        self.with_conn(move |c| c.execute_batch(&sql).map_err(map_err)).await
    }

    async fn attach_database(&self, path: &str, alias: &str) -> Result<()> {
        let sql = format!("ATTACH DATABASE ?1 AS {}", quote_ident(alias));
        let path = path.to_owned();
        self.with_conn(move |c| c.execute(&sql, [path]).map(|_| ()).map_err(map_err)).await
    }
}
