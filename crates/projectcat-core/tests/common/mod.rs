#![allow(dead_code)]
use projectcat_core::{ConnectParams, Driver, Engine, connect};
use std::sync::Arc;

/// A fresh, isolated database on one engine, plus the driver connected to it.
pub struct TestDb {
    pub driver: Arc<dyn Driver>,
    pub db: String,
    pub engine: Engine,
    _dir: Option<tempfile::TempDir>,
}

impl TestDb {
    pub async fn sqlite() -> TestDb {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let driver = connect(&ConnectParams::sqlite(path.to_string_lossy().into_owned(), true))
            .await
            .unwrap();
        TestDb { driver, db: "main".into(), engine: Engine::Sqlite, _dir: Some(dir) }
    }

    /// Runs SQL on the test database, panicking on failure.
    pub async fn exec(&self, sql: &str) {
        let result = self.driver.execute_script(Some(&self.db), sql, 1000, None).await.unwrap();
        if let Some(err) = result.error {
            panic!("setup SQL failed: {} ({})", err.error.message, err.statement);
        }
    }
}
