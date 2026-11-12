use crate::store::{load_json_list, persist_json};
use crate::{Result, ScriptResult, StatementOutcome};
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
        Ok(Self { path: path.to_owned(), entries: Mutex::new(load_json_list(path, "history")?) })
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
        persist_json(&self.path, &serde_json::to_vec(&*entries)?)?;
        Ok(())
    }

    /// Records an executed script with the summed time of its statements.
    pub fn record_script(&self, connection_id: &str, sql: &str, result: &ScriptResult) -> Result<()> {
        let duration = result
            .outcomes
            .iter()
            .map(|o| match o {
                StatementOutcome::Rows { duration_ms, .. } | StatementOutcome::Affected { duration_ms, .. } => *duration_ms,
            })
            .sum();
        self.record(connection_id, sql.trim(), result.error.is_none(), duration)
    }

    /// Newest first.
    pub fn list(&self, connection_id: &str) -> Vec<HistoryEntry> {
        self.entries.lock().unwrap().iter().rev().filter(|e| e.connection_id == connection_id).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::QueryHistory;

    #[test]
    fn malformed_history_is_preserved_and_does_not_prevent_new_queries() {
        for damaged in [b"{not valid JSON".as_slice(), b"\xff\xfe".as_slice()] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("history.json");
            std::fs::write(&path, damaged).unwrap();
            let history = QueryHistory::open(&path).unwrap();
            assert!(history.list("connection").is_empty());
            assert_eq!(std::fs::read(&path).unwrap(), damaged);
            let backups: Vec<_> = std::fs::read_dir(directory.path())
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|entry| entry.file_name().unwrap().to_string_lossy().starts_with("history-corrupt-"))
                .collect();
            assert_eq!(backups.len(), 1);
            assert_eq!(std::fs::read(&backups[0]).unwrap(), damaged);
            history.record("connection", "SELECT 1", true, 1).unwrap();
            let reopened = QueryHistory::open(&path).unwrap();
            assert_eq!(reopened.list("connection")[0].sql, "SELECT 1");
            assert_eq!(std::fs::read(&backups[0]).unwrap(), damaged);
        }
    }
}
