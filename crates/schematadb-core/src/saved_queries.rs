use crate::store::{load_json_list, persist_json};
use crate::{AppError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// A named SQL script kept by Schemata for one connection and, optionally, the database it runs in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedQuery {
    pub id: String,
    pub connection_id: String,
    pub database: Option<String>,
    pub name: String,
    pub sql: String,
    /// Unix time in milliseconds of the last save.
    pub updated_at: u64,
}

/// What the editor sends to create (`id: None`) or overwrite a saved query.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveQueryRequest {
    pub id: Option<String>,
    pub connection_id: String,
    pub database: Option<String>,
    pub name: String,
    pub sql: String,
}

/// Saved queries of every connection, persisted as one JSON file in the app data directory.
pub struct SavedQueries {
    path: PathBuf,
    entries: Mutex<Vec<SavedQuery>>,
}

impl SavedQueries {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self { path: path.to_owned(), entries: Mutex::new(load_json_list(path, "queries")?) })
    }

    /// The connection's saved queries, by name.
    pub fn list(&self, connection_id: &str) -> Vec<SavedQuery> {
        let mut list: Vec<_> = self.entries.lock().unwrap().iter().filter(|q| q.connection_id == connection_id).cloned().collect();
        list.sort_by_key(|q| q.name.to_lowercase());
        list
    }

    /// Creates or overwrites a query. Names are unique (ignoring case) per connection and database.
    pub fn save(&self, request: SaveQueryRequest) -> Result<SavedQuery> {
        let name = request.name.trim();
        if name.is_empty() {
            return Err(AppError::validation("Enter a name for the query."));
        }
        let mut entries = self.entries.lock().unwrap();
        if let Some(id) = &request.id
            && !entries.iter().any(|q| &q.id == id)
        {
            return Err(AppError::validation("This saved query no longer exists. Save it again under a name."));
        }
        let taken = entries.iter().any(|q| {
            Some(&q.id) != request.id.as_ref()
                && q.connection_id == request.connection_id
                && q.database == request.database
                && q.name.to_lowercase() == name.to_lowercase()
        });
        if taken {
            return Err(AppError::validation(format!("A saved query named \"{name}\" already exists here.")));
        }
        let saved = SavedQuery {
            id: request.id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            connection_id: request.connection_id,
            database: request.database,
            name: name.to_owned(),
            sql: request.sql,
            updated_at: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
        };
        let mut next = entries.clone();
        match next.iter_mut().find(|q| q.id == saved.id) {
            Some(existing) => *existing = saved.clone(),
            None => next.push(saved.clone()),
        }
        // Memory changes only after the file holds the new list, so a failed write never shows unsaved state as saved.
        persist_json(&self.path, &serde_json::to_vec_pretty(&next)?)?;
        *entries = next;
        Ok(saved)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let mut entries = self.entries.lock().unwrap();
        let next: Vec<_> = entries.iter().filter(|q| q.id != id).cloned().collect();
        persist_json(&self.path, &serde_json::to_vec_pretty(&next)?)?;
        *entries = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{SaveQueryRequest, SavedQueries};

    fn request(id: Option<&str>, database: Option<&str>, name: &str, sql: &str) -> SaveQueryRequest {
        SaveQueryRequest {
            id: id.map(str::to_owned),
            connection_id: "c".into(),
            database: database.map(str::to_owned),
            name: name.into(),
            sql: sql.into(),
        }
    }

    #[test]
    fn saved_queries_survive_reopening_and_overwrite_by_id() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("queries.json");
        let queries = SavedQueries::open(&path).unwrap();
        let first = queries.save(request(None, Some("shop"), " Top products ", "SELECT 1")).unwrap();
        assert_eq!(first.name, "Top products");
        queries.save(request(Some(&first.id), Some("shop"), "Top products", "SELECT 2")).unwrap();

        let reopened = SavedQueries::open(&path).unwrap();
        let list = reopened.list("c");
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].id.as_str(), list[0].sql.as_str()), (first.id.as_str(), "SELECT 2"));
        assert!(reopened.list("other").is_empty());

        reopened.delete(&first.id).unwrap();
        assert!(SavedQueries::open(&path).unwrap().list("c").is_empty());
    }

    #[test]
    fn names_are_unique_per_database_ignoring_case() {
        let directory = tempfile::tempdir().unwrap();
        let queries = SavedQueries::open(&directory.path().join("queries.json")).unwrap();
        let saved = queries.save(request(None, Some("shop"), "Report", "SELECT 1")).unwrap();
        assert!(queries.save(request(None, Some("shop"), "report", "SELECT 2")).is_err());
        assert!(queries.save(request(None, Some("archive"), "report", "SELECT 2")).is_ok());
        assert!(queries.save(request(None, None, "report", "SELECT 2")).is_ok());
        assert!(queries.save(request(None, Some("shop"), "  ", "SELECT 2")).is_err());
        // Renaming a query to its own name in another case is not a conflict.
        assert!(queries.save(request(Some(&saved.id), Some("shop"), "REPORT", "SELECT 1")).is_ok());
        assert!(queries.save(request(Some("gone"), Some("shop"), "Gone", "SELECT 1")).is_err());
        assert_eq!(queries.list("c").len(), 3);
    }

    #[test]
    fn a_failed_write_keeps_the_previous_list() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("queries.json");
        let queries = SavedQueries::open(&path).unwrap();
        // A directory in place of the file makes persisting fail.
        std::fs::create_dir(&path).unwrap();
        assert!(queries.save(request(None, None, "Report", "SELECT 1")).is_err());
        assert!(queries.list("c").is_empty());
    }
}
