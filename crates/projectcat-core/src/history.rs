use crate::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Entries kept per connection.
const PER_CONNECTION: usize = 200;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub connection_id: String,
    pub sql: String,
    pub ok: bool,
    pub duration_ms: u64,
    /// Unix time in milliseconds.
    pub at: u64,
}

/// Executed queries per connection, persisted as JSON.
pub struct QueryHistory {
    path: PathBuf,
    entries: Mutex<Vec<HistoryEntry>>,
}

impl QueryHistory {
    pub fn open(path: &Path) -> Result<Self> {
        let entries = match std::fs::read_to_string(path) {
            Ok(raw) => serde_json::from_str(&raw)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self { path: path.to_owned(), entries: Mutex::new(entries) })
    }

    pub fn record(&self, connection_id: &str, sql: &str, ok: bool, duration_ms: u64) -> Result<()> {
        let at = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let mut entries = self.entries.lock().unwrap();
        entries.push(HistoryEntry { connection_id: connection_id.to_owned(), sql: sql.to_owned(), ok, duration_ms, at });
        let count = entries.iter().filter(|e| e.connection_id == connection_id).count();
        if count > PER_CONNECTION {
            let mut excess = count - PER_CONNECTION;
            entries.retain(|e| {
                if excess > 0 && e.connection_id == connection_id {
                    excess -= 1;
                    false
                } else {
                    true
                }
            });
        }
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&self.path, serde_json::to_vec(&*entries)?)?;
        Ok(())
    }

    /// Newest first.
    pub fn list(&self, connection_id: &str) -> Vec<HistoryEntry> {
        self.entries.lock().unwrap().iter().rev().filter(|e| e.connection_id == connection_id).cloned().collect()
    }
}
