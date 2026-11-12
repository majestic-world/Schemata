use crate::{AppError, ConnectParams, ConnectionConfig, ConnectionStore, Driver, Engine, ErrorKind, Result, SaveOutcome, connect};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::Mutex;

/// SQLSTATE of a timeout waiting for a pooled connection ("timeout expired"): the server is busy or unreachable, but
/// the session's connections are not lost, so it is not reconnected (which would drop its editor sessions).
pub(crate) const POOL_TIMEOUT_SQL_STATE: &str = "HYT00";

struct Session {
    identity: Arc<()>,
    driver: Arc<dyn Driver>,
    params: ConnectParams,
}

#[derive(Default)]
struct SessionState {
    open: HashMap<String, Session>,
    pending: HashMap<String, Arc<()>>,
    /// Editor sessions of closed connections, by connection id: the reopened connection reports each one lost once.
    lost_editor_sessions: HashMap<String, Vec<String>>,
}

/// Open sessions keyed by saved-connection id, with transparent reconnection.
pub struct Sessions {
    store: ConnectionStore,
    state: Mutex<SessionState>,
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
        Self { store, state: Mutex::new(SessionState::default()) }
    }

    pub fn store(&self) -> &ConnectionStore {
        &self.store
    }

    /// Serializes destination changes with session creation and installation.
    pub async fn save_connection(&self, config: &ConnectionConfig, password: Option<&str>) -> Result<SaveOutcome> {
        let state = self.state.lock().await;
        if (state.open.contains_key(&config.id) || state.pending.contains_key(&config.id))
            && !same_target(&self.store.get(&config.id)?, config)
        {
            return Err(AppError::validation("Close the connection before changing its destination."));
        }
        self.store.save(config, password)
    }

    /// Opens a session. Without a supplied password, the stored one is used; MySQL/MariaDB
    /// connections that do not save their password fail with `Authentication` so the UI can prompt.
    pub async fn open(&self, id: &str, password: Option<String>) -> Result<String> {
        let identity = Arc::new(());
        let (config, params) = {
            let mut state = self.state.lock().await;
            let config = self.store.get(id)?;
            let password = match password {
                Some(p) => Some(p),
                None => self.store.password(id)?,
            };
            if config.engine != Engine::Sqlite && password.is_none() && !config.save_password {
                return Err(AppError::new(ErrorKind::Authentication, "Password required"));
            }
            let params = params_for(&config, password);
            state.pending.insert(id.to_owned(), identity.clone());
            (config, params)
        };
        let connected = connect(&params).await;
        let mut state = self.state.lock().await;
        if !state.pending.get(id).is_some_and(|pending| Arc::ptr_eq(pending, &identity)) {
            return Err(AppError::connection("Connection was closed or replaced"));
        }
        state.pending.remove(id);
        let driver = connected?;
        if !same_target(&self.store.get(id)?, &config) {
            return Err(AppError::connection("Connection destination changed while opening"));
        }
        let version = driver.server_version();
        // Editor sessions of the connection this one replaces (or of its last close) are gone with their transactions.
        let mut lost = state.lost_editor_sessions.remove(id).unwrap_or_default();
        if let Some(replaced) = state.open.get(id) {
            lost.extend(replaced.driver.live_editor_sessions());
        }
        driver.adopt_lost_editor_sessions(lost);
        state.open.insert(id.to_owned(), Session { identity, driver, params });
        Ok(version)
    }

    pub async fn close(&self, id: &str) {
        let mut state = self.state.lock().await;
        state.pending.remove(id);
        if let Some(session) = state.open.remove(id) {
            let lost = session.driver.live_editor_sessions();
            state.lost_editor_sessions.entry(id.to_owned()).or_default().extend(lost);
        }
    }

    pub async fn open_ids(&self) -> Vec<String> {
        self.state.lock().await.open.keys().cloned().collect()
    }

    pub async fn driver(&self, id: &str) -> Result<Arc<dyn Driver>> {
        self.state.lock().await.open.get(id).map(|s| s.driver.clone()).ok_or_else(|| AppError::connection("Connection is not open"))
    }

    async fn reconnect(&self, id: &str, identity: &Arc<()>, failed: &Arc<dyn Driver>) -> Result<Arc<dyn Driver>> {
        let params = {
            let state = self.state.lock().await;
            let original = state.open.get(id).filter(|session| Arc::ptr_eq(&session.identity, identity));
            let session = original.ok_or_else(|| AppError::connection("Connection was closed or replaced"))?;
            if !Arc::ptr_eq(&session.driver, failed) {
                return Ok(session.driver.clone());
            }
            session.params.clone()
        };
        let driver = connect(&params).await?;
        let mut state = self.state.lock().await;
        let original = state.open.get_mut(id).filter(|session| Arc::ptr_eq(&session.identity, identity));
        let session = original.ok_or_else(|| AppError::connection("Connection was closed or replaced"))?;
        if Arc::ptr_eq(&session.driver, failed) {
            driver.adopt_lost_editor_sessions(failed.live_editor_sessions());
            session.driver = driver;
        }
        Ok(session.driver.clone())
    }

    /// Runs `op` against the session; on a connection-level failure, reconnects once and retries.
    pub async fn with_driver<T, F, Fut>(&self, id: &str, op: F) -> Result<T>
    where
        F: Fn(Arc<dyn Driver>) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let (driver, identity) = {
            let state = self.state.lock().await;
            let session = state.open.get(id).ok_or_else(|| AppError::connection("Connection is not open"))?;
            (session.driver.clone(), session.identity.clone())
        };
        match op(driver.clone()).await {
            Err(e) if connection_lost(&e) => op(self.reconnect(id, &identity, &driver).await?).await,
            other => other,
        }
    }
}

/// A connection-level failure that a new driver may fix; a pool timeout is not one.
fn connection_lost(e: &AppError) -> bool {
    e.kind == ErrorKind::Connection && e.sql_state.as_deref() != Some(POOL_TIMEOUT_SQL_STATE)
}

fn same_target(left: &ConnectionConfig, right: &ConnectionConfig) -> bool {
    left.engine == right.engine
        && left.host == right.host
        && left.port == right.port
        && left.user == right.user
        && left.database == right.database
        && left.file_path == right.file_path
}
