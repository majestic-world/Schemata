//! Schemata core: database drivers and services, independent of the UI shell.

mod copy;
mod design;
mod driver;
mod error;
mod generate;
mod history;
mod model;
mod mysql;
mod secrets;
mod sessions;
mod splitter;
mod sql;
mod sql_tokens;
mod sqlite;
mod store;
mod transfer;
mod value;

pub use copy::{CopyTableReport, CopyTableTarget, copy_table};
pub use design::{ColumnDesign, ForeignKeyDesign, IndexDesign, TableDesign, TableOptions};
pub use driver::{ConnectParams, Driver, connect};
pub use error::{AppError, ErrorKind, Result};
pub use generate::{
    ColumnGenerator, GenerateReport, GenerateSpec, GeneratorKind, MAX_GENERATED_ROWS, generate_data, preview_generated, suggest_generators,
};
pub use history::{HistoryEntry, QueryHistory};
pub use model::*;
pub use secrets::{KeyringSecrets, MemorySecrets, SecretStore};
pub use sessions::{Sessions, params_for};
pub use splitter::{Splitter, split_statements};
pub use sql::is_read_only_query;
pub use store::{ConnectionConfig, ConnectionStore, SaveOutcome};
pub use transfer::{
    BATCH, CancellationToken, ColumnMapping, CsvImport, CsvOptions, CsvPreview, CsvReadOptions, DumpOptions, DumpReport, DumpScript,
    ExportFormat, ExportReport, ExportSource, ImportBatch, ImportOptions, ImportReport, MAX_REPORTED_FAILURES, Progress, ProgressFn,
    RowFailure, RowStream, ScriptRunReport, SqlExportOptions, SqlFileScan, StatementFailure, StreamItem, TextEncoding, dump_database,
    export, import_csv, preview_csv, read_sql_text, run_sql_file, scan_sql_file, write_text_atomic,
};
pub use value::{CellValue, Value};
