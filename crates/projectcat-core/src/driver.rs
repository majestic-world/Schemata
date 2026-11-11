use crate::{
    AppError, ChangeError, ObjectKind, RowChange, TableDesign, CharsetInfo, CreateDatabase, DatabaseInfo, Engine, Filter, ObjectInfo, Page, PageRequest, Result, ScriptResult,
    TableInfo,
};
use async_trait::async_trait;
use std::sync::Arc;

/// Everything needed to open a session. Built from a saved connection plus its password.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectParams {
    pub engine: Engine,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub database: Option<String>,
    pub file_path: Option<String>,
    /// SQLite only: create the database file when it does not exist.
    #[serde(default)]
    pub create_file: bool,
}

impl ConnectParams {
    pub fn sqlite(path: impl Into<String>, create_file: bool) -> Self {
        Self {
            engine: Engine::Sqlite,
            host: None,
            port: None,
            user: None,
            password: None,
            database: None,
            file_path: Some(path.into()),
            create_file,
        }
    }
}

/// A live session to one server or SQLite file. Implementations hide dialect differences.
#[async_trait]
pub trait Driver: Send + Sync {
    fn engine(&self) -> Engine;
    fn server_version(&self) -> String;
    async fn ping(&self) -> Result<()>;
    async fn list_databases(&self) -> Result<Vec<DatabaseInfo>>;
    /// Tables first, then views, each sorted by name.
    async fn list_objects(&self, db: &str) -> Result<Vec<ObjectInfo>>;
    /// Runs a script statement by statement, stopping at the first error. `execution_id` makes it cancelable.
    async fn execute_script(&self, db: Option<&str>, sql: &str, row_limit: usize, execution_id: Option<&str>) -> Result<ScriptResult>;
    /// Rows `offset..offset+limit` of a row-returning statement (for "load more").
    async fn fetch_more(&self, db: Option<&str>, statement: &str, offset: u64, limit: u64) -> Result<Page>;
    /// Every table and view with its column names, for autocompletion.
    async fn list_columns(&self, db: &str) -> Result<Vec<(String, Vec<String>)>>;
    /// Cancels the statement currently running under `execution_id`, if any.
    async fn cancel(&self, execution_id: &str) -> Result<()>;
    async fn table_info(&self, db: &str, name: &str) -> Result<TableInfo>;
    /// One page of a table or view. On SQLite tables addressed by rowid, a hidden rowid column comes first.
    async fn fetch_page(&self, db: &str, table: &str, req: &PageRequest) -> Result<Page>;
    async fn count_rows(&self, db: &str, table: &str, filters: &[Filter]) -> Result<u64>;
    async fn table_design(&self, db: &str, table: &str) -> Result<TableDesign> {
        Ok(TableDesign::from_info(&self.table_info(db, table).await?))
    }
    /// DDL that creates `design` (no `original`) or alters `original` into it. Empty when nothing changes.
    async fn plan_table(&self, db: &str, original: Option<&TableDesign>, design: &TableDesign) -> Result<Vec<String>>;
    /// Runs DDL produced by [`Driver::plan_table`]; a failure leaves the table as it was.
    async fn apply_ddl(&self, db: &str, statements: &[String]) -> Result<()>;
    async fn drop_object(&self, db: &str, name: &str, kind: ObjectKind) -> Result<()>;
    /// Deletes every row, keeping the structure.
    async fn truncate_table(&self, db: &str, name: &str) -> Result<()>;
    async fn rename_table(&self, db: &str, from: &str, to: &str) -> Result<()>;
    /// Applies grid changes in one transaction; returns the number of applied changes.
    async fn apply_changes(&self, db: &str, table: &str, changes: &[RowChange]) -> Result<u64, ChangeError>;
    async fn create_database(&self, spec: &CreateDatabase) -> Result<()>;
    /// Drops a database (detaches it on SQLite).
    async fn drop_database(&self, name: &str) -> Result<()>;
    /// Character sets and their collations; empty when the engine has none to choose from.
    async fn list_charsets(&self) -> Result<Vec<CharsetInfo>> {
        Ok(Vec::new())
    }
    /// SQLite only: attaches another database file under `alias`.
    async fn attach_database(&self, _path: &str, _alias: &str) -> Result<()> {
        Err(AppError::validation("Attaching database files is only supported on SQLite"))
    }
}

pub async fn connect(params: &ConnectParams) -> Result<Arc<dyn Driver>> {
    match params.engine {
        Engine::Sqlite => {
            let path = params.file_path.as_deref().ok_or_else(|| AppError::validation("A database file is required"))?;
            Ok(Arc::new(crate::sqlite::SqliteDriver::open(path, params.create_file).await?))
        }
        Engine::MySql | Engine::MariaDb => Ok(Arc::new(crate::mysql::MySqlDriver::connect(params).await?)),
    }
}
