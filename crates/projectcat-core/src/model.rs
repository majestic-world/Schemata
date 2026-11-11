use crate::value::CellValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Engine {
    MySql,
    MariaDb,
    Sqlite,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseInfo {
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObjectKind {
    Table,
    View,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectInfo {
    pub name: String,
    pub kind: ObjectKind,
    /// Exact on SQLite, an estimate on MySQL/MariaDB, `None` for views.
    pub rows: Option<i64>,
    pub created: Option<String>,
    pub modified: Option<String>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMeta {
    pub name: String,
    pub type_name: String,
    /// Internal columns (e.g. SQLite rowid used for row identity) that the grid must not show.
    pub hidden: bool,
}

/// Outcome of one statement of a script.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StatementOutcome {
    #[serde(rename_all = "camelCase")]
    Rows { statement: String, columns: Vec<ColumnMeta>, rows: Vec<Vec<CellValue>>, has_more: bool, duration_ms: u64 },
    #[serde(rename_all = "camelCase")]
    Affected { statement: String, rows_affected: u64, last_insert_id: Option<u64>, duration_ms: u64 },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatementError {
    pub index: usize,
    pub statement: String,
    pub error: crate::AppError,
}

/// Results of a script; execution stops at the first failing statement.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptResult {
    pub outcomes: Vec<StatementOutcome>,
    pub error: Option<StatementError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    pub name: String,
    /// Full type as declared, e.g. `varchar(100)` or `decimal(12,4) unsigned`.
    pub data_type: String,
    pub nullable: bool,
    /// Default as a SQL expression (string literals quoted), `None` when there is no default.
    pub default: Option<String>,
    pub auto_increment: bool,
    pub primary_key: bool,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    /// True for the primary key index (never returned by `table_info`, which reports it on columns).
    #[serde(default)]
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKeyInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
    pub on_update: String,
    pub on_delete: String,
}

/// How rows of a table are addressed for updates and deletes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RowIdentity {
    PrimaryKey { columns: Vec<String> },
    UniqueKey { columns: Vec<String> },
    /// SQLite implicit rowid.
    RowId,
    /// No reliable identity: the data is read-only.
    None,
}

impl RowIdentity {
    pub fn columns(&self) -> Vec<String> {
        match self {
            RowIdentity::PrimaryKey { columns } | RowIdentity::UniqueKey { columns } => columns.clone(),
            RowIdentity::RowId => vec![ROWID_COLUMN.to_owned()],
            RowIdentity::None => Vec::new(),
        }
    }
}

/// Alias under which SQLite rowids are fetched and addressed.
pub const ROWID_COLUMN: &str = "__rowid__";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    pub name: String,
    pub kind: ObjectKind,
    pub columns: Vec<ColumnInfo>,
    pub indexes: Vec<IndexInfo>,
    pub foreign_keys: Vec<ForeignKeyInfo>,
    pub engine: Option<String>,
    pub charset: Option<String>,
    pub collation: Option<String>,
    pub rows: Option<i64>,
    pub created: Option<String>,
    pub modified: Option<String>,
    pub comment: Option<String>,
    /// `CREATE TABLE` / `CREATE VIEW` statement.
    pub create_sql: String,
    pub row_identity: RowIdentity,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDatabase {
    pub name: String,
    pub charset: Option<String>,
    pub collation: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharsetInfo {
    pub name: String,
    pub default_collation: String,
    pub collations: Vec<String>,
}

/// Picks the row identity from a primary key, else a unique key over non-null columns.
pub(crate) fn derive_identity(columns: &[ColumnInfo], indexes: &[IndexInfo], has_rowid: bool) -> RowIdentity {
    let pk: Vec<String> = columns.iter().filter(|c| c.primary_key).map(|c| c.name.clone()).collect();
    if !pk.is_empty() {
        return RowIdentity::PrimaryKey { columns: pk };
    }
    let not_null = |name: &String| columns.iter().any(|c| &c.name == name && !c.nullable);
    if let Some(idx) = indexes.iter().find(|i| i.unique && !i.columns.is_empty() && i.columns.iter().all(not_null)) {
        return RowIdentity::UniqueKey { columns: idx.columns.clone() };
    }
    if has_rowid { RowIdentity::RowId } else { RowIdentity::None }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SortSpec {
    pub column: String,
    pub desc: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FilterOp {
    Eq,
    NotEq,
    Lt,
    Lte,
    Gt,
    Gte,
    Contains,
    StartsWith,
    IsNull,
    IsNotNull,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    pub column: String,
    pub op: FilterOp,
    pub value: Option<CellValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageRequest {
    pub offset: u64,
    pub limit: u64,
    pub sort: Option<SortSpec>,
    /// Combined with AND.
    pub filters: Vec<Filter>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<CellValue>>,
}
