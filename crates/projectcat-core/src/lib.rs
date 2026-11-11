//! ProjectCat core: database drivers and services, independent of the UI shell.

mod driver;
mod error;
mod model;
mod secrets;
mod sessions;
mod splitter;
mod sqlite;
mod store;
mod value;

pub use driver::{ConnectParams, Driver, connect};
pub use error::{AppError, ErrorKind, Result};
pub use model::*;
pub use secrets::{KeyringSecrets, MemorySecrets, SecretStore};
pub use sessions::{Sessions, params_for};
pub use splitter::split_statements;
pub use store::{ConnectionConfig, ConnectionStore, SaveOutcome};
pub use value::{CellValue, Value};
