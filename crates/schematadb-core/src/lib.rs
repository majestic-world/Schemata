//! SchemataDb core: database drivers and services, independent of the UI shell.

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
mod sqlite;
mod store;
mod transfer;
mod value;

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
pub use store::{ConnectionConfig, ConnectionStore, SaveOutcome};
pub use transfer::{
    BATCH, CancellationToken, ColumnMapping, CopyTableReport, CopyTableTarget, CsvImport, CsvOptions, CsvPreview, CsvReadOptions,
    DumpOptions, DumpReport, DumpScript, ExportFormat, ExportReport, ExportSource, ImportBatch, ImportReport, MAX_REPORTED_FAILURES,
    Progress, ProgressFn, RowFailure, RowStream, ScriptRunReport, SqlExportOptions, StatementFailure, StreamItem, TextEncoding, copy_table,
    dump_database, export, import_csv, preview_csv, run_sql_file,
};
pub use value::{CellValue, Value};
