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
    /// Computed by the engine; never supplied by inserts or updates.
    #[serde(default)]
    pub generated: bool,
    pub primary_key: bool,
    pub comment: Option<String>,
}

/// What an index enforces or serves. SQLite only has `Normal` and `Unique`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IndexKind {
    Normal,
    Unique,
    Fulltext,
    Spatial,
}

/// Storage structure of a MySQL/MariaDB `Normal` or `Unique` index (`USING ...`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IndexMethod {
    Btree,
    Hash,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub kind: IndexKind,
    /// Reported by MySQL/MariaDB for `Normal` and `Unique` indexes; `None` on SQLite.
    pub method: Option<IndexMethod>,
    /// MySQL/MariaDB index comment.
    pub comment: Option<String>,
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
    PrimaryKey {
        columns: Vec<String>,
    },
    UniqueKey {
        columns: Vec<String>,
    },
    /// SQLite implicit rowid.
    RowId,
    /// No key (Navicat-style): a row is matched by the values of every comparable column, and each update or delete
    /// touches at most one row, so only one of several identical rows changes.
    AllColumns {
        columns: Vec<String>,
    },
    /// No usable identity (views, or tables without comparable columns): the data is read-only.
    None,
}

impl RowIdentity {
    pub fn columns(&self) -> Vec<String> {
        match self {
            RowIdentity::PrimaryKey { columns } | RowIdentity::UniqueKey { columns } | RowIdentity::AllColumns { columns } => {
                columns.clone()
            }
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

impl TableInfo {
    pub(crate) fn validate_writable_columns<'a>(&self, columns: impl IntoIterator<Item = &'a str>) -> crate::Result<()> {
        for name in columns {
            let column = self
                .columns
                .iter()
                .find(|c| c.name == name)
                .ok_or_else(|| crate::AppError::validation(format!("Column {name} not found")))?;
            if column.generated {
                return Err(crate::AppError::validation(format!("Column {name} is generated and cannot be written")));
            }
        }
        Ok(())
    }

    pub(crate) fn validate_changes(&self, changes: &[RowChange]) -> Result<(), ChangeError> {
        for (index, change) in changes.iter().enumerate() {
            let values = match change {
                RowChange::Insert { values } | RowChange::Update { values, .. } => values,
                RowChange::Delete { .. } => continue,
            };
            self.validate_writable_columns(values.iter().map(|v| v.column.as_str()))
                .map_err(|error| ChangeError { index: Some(index), error })?;
        }
        Ok(())
    }
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

/// Picks the row identity from a primary key, else a unique key over non-null columns, else the SQLite rowid,
/// else every column whose value can be matched exactly.
pub(crate) fn derive_identity(columns: &[ColumnInfo], indexes: &[IndexInfo], has_rowid: bool) -> RowIdentity {
    let pk: Vec<String> = columns.iter().filter(|c| c.primary_key).map(|c| c.name.clone()).collect();
    if !pk.is_empty() {
        return RowIdentity::PrimaryKey { columns: pk };
    }
    let not_null = |name: &String| columns.iter().any(|c| &c.name == name && !c.nullable);
    if let Some(idx) = indexes.iter().find(|i| i.kind == IndexKind::Unique && !i.columns.is_empty() && i.columns.iter().all(not_null)) {
        return RowIdentity::UniqueKey { columns: idx.columns.clone() };
    }
    if has_rowid {
        return RowIdentity::RowId;
    }
    let comparable: Vec<String> = columns.iter().filter(|c| matchable(&c.data_type)).map(|c| c.name.clone()).collect();
    if comparable.is_empty() { RowIdentity::None } else { RowIdentity::AllColumns { columns: comparable } }
}

/// Whether a column's grid value can find its row by equality: binary cells arrive as previews, FLOAT is stored
/// approximately, and JSON/spatial values do not compare equal to their text.
fn matchable(data_type: &str) -> bool {
    let t = data_type.to_ascii_lowercase();
    const UNMATCHABLE: [&str; 9] = ["blob", "binary", "json", "float", "geometry", "point", "linestring", "polygon", "bit"];
    !UNMATCHABLE.iter().any(|u| t.contains(u))
}

/// Whether a MySQL/MariaDB column type compares under a collation (CHAR/VARCHAR/TEXT family, ENUM, SET).
pub(crate) fn is_text_type(data_type: &str) -> bool {
    let t = data_type.to_ascii_lowercase();
    (t.contains("char") || t.contains("text") || t.starts_with("enum") || t.starts_with("set")) && !t.contains("binary")
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnValue {
    pub column: String,
    pub value: CellValue,
}

/// One pending grid change. `key` lists the row-identity column values in identity order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RowChange {
    Update { key: Vec<CellValue>, values: Vec<ColumnValue> },
    Insert { values: Vec<ColumnValue> },
    Delete { key: Vec<CellValue> },
}

/// Failure of a change set; `index` points at the change that failed, when known.
#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{error}")]
pub struct ChangeError {
    pub index: Option<usize>,
    pub error: crate::AppError,
}

impl From<crate::AppError> for ChangeError {
    fn from(error: crate::AppError) -> Self {
        ChangeError { index: None, error }
    }
}
