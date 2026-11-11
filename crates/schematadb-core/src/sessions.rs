use crate::{AppError, ConnectParams, ConnectionConfig, ConnectionStore, Driver, Engine, ErrorKind, Result, connect};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::Mutex;

struct Session {
    driver: Arc<dyn Driver>,
    params: ConnectParams,
}

/// Open sessions keyed by saved-connection id, with transparent reconnection.
pub struct Sessions {
    store: ConnectionStore,
    open: Mutex<HashMap<String, Session>>,
}

pub fn params_for(config: &ConnectionConfig, password: Option<String>) -> ConnectParams {
    ConnectParams {
        engine: config.engine,
        host: config.host.clone(),
        port: config.port,
        user: config.user.clone(),
        password,
        database: config.database.clone(),
        file_path: config.file_path.clone(),
        create_file: false,
    }
}

impl Sessions {
    pub fn new(store: ConnectionStore) -> Self {
        Self { store, open: Mutex::new(HashMap::new()) }
    }

    pub fn store(&self) -> &ConnectionStore {
        &self.store
    }

    /// Opens a session. Without a supplied password, the stored one is used; MySQL/MariaDB
    /// connections that do not save their password fail with `Authentication` so the UI can prompt.
    pub async fn open(&self, id: &str, password: Option<String>) -> Result<String> {
        let config = self.store.get(id)?;
        let password = match password {
            Some(p) => Some(p),
            None => self.store.password(id)?,
        };
        if config.engine != Engine::Sqlite && password.is_none() && !config.save_password {
            return Err(AppError::new(ErrorKind::Authentication, "Password required"));
        }
        let params = params_for(&config, password);
        let driver = connect(&params).await?;
        let version = driver.server_version();
        self.open.lock().await.insert(id.to_owned(), Session { driver, params });
        Ok(version)
    }

    pub async fn close(&self, id: &str) {
        self.open.lock().await.remove(id);
    }

    pub async fn open_ids(&self) -> Vec<String> {
        self.open.lock().await.keys().cloned().collect()
    }

    pub async fn driver(&self, id: &str) -> Result<Arc<dyn Driver>> {
        self.open.lock().await.get(id).map(|s| s.driver.clone()).ok_or_else(|| AppError::connection("Connection is not open"))
    }

    async fn reconnect(&self, id: &str) -> Result<Arc<dyn Driver>> {
        let params = {
            let open = self.open.lock().await;
            open.get(id).map(|s| s.params.clone()).ok_or_else(|| AppError::connection("Connection is not open"))?
        };
        let driver = connect(&params).await?;
        self.open.lock().await.insert(id.to_owned(), Session { driver: driver.clone(), params });
        Ok(driver)
    }

    /// Runs `op` against the session; on a connection-level failure, reconnects once and retries.
    pub async fn with_driver<T, F, Fut>(&self, id: &str, op: F) -> Result<T>
    where
        F: Fn(Arc<dyn Driver>) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let driver = self.driver(id).await?;
        match op(driver).await {
            Err(e) if e.kind == ErrorKind::Connection => op(self.reconnect(id).await?).await,
            other => other,
        }
    }
}
