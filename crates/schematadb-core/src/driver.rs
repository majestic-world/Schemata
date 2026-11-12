use crate::transfer::{DumpScript, ImportBatch, ImportOptions, ImportReport, RowStream, ScriptRunReport};
use crate::{
    AppError, ChangeError, CharsetInfo, CreateDatabase, DatabaseInfo, Engine, Filter, IncomingForeignKey, ObjectInfo, ObjectKind, Page,
    PageRequest, Result, RowChange, RowIdentity, ScriptResult, TableDesign, TableInfo,
};
use async_trait::async_trait;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

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
    /// [`Driver::execute_script`] on the SQL editor session `session`: every run with the same key uses the same
    /// server session, so transactions, user variables and temporary tables persist across runs until
    /// [`Driver::close_editor_session`]. Single-connection drivers share their only session.
    async fn execute_in_session(
        &self,
        _session: &str,
        db: Option<&str>,
        sql: &str,
        row_limit: usize,
        execution_id: Option<&str>,
    ) -> Result<ScriptResult> {
        self.execute_script(db, sql, row_limit, execution_id).await
    }
    /// [`Driver::fetch_more`] on the SQL editor session `session`, so continuations see that session's state.
    async fn fetch_more_in_session(&self, _session: &str, db: Option<&str>, statement: &str, offset: u64, limit: u64) -> Result<Page> {
        self.fetch_more(db, statement, offset, limit).await
    }
    /// Ends an editor session; the server rolls back its open transaction. Unknown keys are ignored.
    async fn close_editor_session(&self, _session: &str) -> Result<()> {
        Ok(())
    }
    /// Editor sessions that hold a server session (transaction, variables, temporary tables) lost with this driver.
    fn live_editor_sessions(&self) -> Vec<String> {
        Vec::new()
    }
    /// Marks editor sessions of a replaced driver as lost: the next run of each fails once with a connection error
    /// instead of silently opening a fresh session.
    fn adopt_lost_editor_sessions(&self, _keys: Vec<String>) {}
    /// Every table and view with its column names, for autocompletion.
    async fn list_columns(&self, db: &str) -> Result<Vec<(String, Vec<String>)>>;
    /// Streams the rows of a statement in batches; the first item is the column list.
    async fn stream_query(&self, db: Option<&str>, sql: &str, batch: usize) -> Result<RowStream>;
    /// [`Driver::stream_query`] on the session `session` (see [`Driver::execute_in_session`]), so a series of reads
    /// shares that session's transaction snapshot and settings.
    async fn stream_query_in_session(&self, _session: &str, db: Option<&str>, sql: &str, batch: usize) -> Result<RowStream> {
        self.stream_query(db, sql, batch).await
    }
    /// Inserts rows from `rows` in one transaction. Without `options.skip_errors`, the first failing row rolls back
    /// everything and is reported; an `Err` item on the channel (e.g. cancellation) rolls back and is returned.
    async fn import_rows(
        &self,
        db: &str,
        table: &str,
        columns: &[String],
        rows: tokio::sync::mpsc::Receiver<Result<ImportBatch>>,
        options: ImportOptions,
    ) -> Result<ImportReport>;
    /// Runs streamed statements on one session. Cancellation interrupts the active statement.
    /// Reader errors and cancellation roll back open transactions and restore foreign-key enforcement.
    async fn execute_statements(
        &self,
        db: Option<&str>,
        statements: tokio::sync::mpsc::Receiver<Result<String>>,
        stop_on_error: bool,
        cancel: CancellationToken,
    ) -> Result<ScriptRunReport>;
    /// Schema parts of a database dump (see [`crate::dump_database`]).
    async fn dump_script(&self, db: &str) -> Result<DumpScript>;
    /// Cancels the statement currently running under `execution_id`, if any.
    async fn cancel(&self, execution_id: &str) -> Result<()>;
    async fn table_info(&self, db: &str, name: &str) -> Result<TableInfo>;
    /// One page of a table or view. On SQLite tables addressed by rowid, a hidden rowid column comes first.
    async fn fetch_page(&self, db: &str, table: &str, req: &PageRequest) -> Result<Page>;
    async fn count_rows(&self, db: &str, table: &str, filters: &[Filter]) -> Result<u64>;
    async fn table_design(&self, db: &str, table: &str) -> Result<TableDesign> {
        let info = self.table_info(db, table).await?;
        if info.columns.iter().any(|c| c.generated) {
            return Err(AppError::validation("Tables with generated columns cannot be edited in the table designer; use SQL"));
        }
        Ok(TableDesign::from_info(&info))
    }
    /// DDL that creates `design` (no `original`) or alters `original` into it. Empty when nothing changes. Refused when
    /// `original` no longer matches the table as it is now (it changed since the designer loaded it).
    async fn plan_table(&self, db: &str, original: Option<&TableDesign>, design: &TableDesign) -> Result<Vec<String>>;
    /// Runs DDL produced by [`Driver::plan_table`]; a failure leaves the table as it was.
    async fn apply_ddl(&self, db: &str, statements: &[String]) -> Result<()>;
    async fn drop_object(&self, db: &str, name: &str, kind: ObjectKind) -> Result<()>;
    /// Deletes every row, keeping the structure.
    async fn truncate_table(&self, db: &str, name: &str) -> Result<()>;
    async fn rename_table(&self, db: &str, from: &str, to: &str) -> Result<()>;
    /// Applies grid changes in one transaction; returns the number of applied changes. `identity` is the row identity
    /// the change keys were taken from; the changes are refused when the table's identity is no longer the same.
    async fn apply_changes(&self, db: &str, table: &str, identity: &RowIdentity, changes: &[RowChange]) -> Result<u64, ChangeError>;
    /// Foreign keys of other tables that reference `table`: deleting its rows or dropping it triggers their actions.
    async fn incoming_foreign_keys(&self, db: &str, table: &str) -> Result<Vec<IncomingForeignKey>>;
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
