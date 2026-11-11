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
