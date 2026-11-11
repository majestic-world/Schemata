use crate::{AppError, Engine, Result, SecretStore};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A saved connection definition. Never contains secrets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionConfig {
    pub id: String,
    pub name: String,
    pub engine: Engine,
    pub color: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub database: Option<String>,
    pub file_path: Option<String>,
    pub save_password: bool,
}

impl ConnectionConfig {
    pub fn new(name: impl Into<String>, engine: Engine) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            engine,
            color: None,
            host: None,
            port: None,
            user: None,
            database: None,
            file_path: None,
            save_password: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveOutcome {
    pub config: ConnectionConfig,
    /// False when a password was supplied but could not be stored in the credential store.
    pub password_saved: bool,
    /// A credential-store failure; disabling password storage still prevents all future reads.
    pub password_warning: Option<String>,
}

/// Saved connections persisted as JSON; passwords live in a [`SecretStore`].
pub struct ConnectionStore {
    path: PathBuf,
    secrets: Box<dyn SecretStore>,
    configs: Mutex<Vec<ConnectionConfig>>,
}

impl ConnectionStore {
    pub fn open(path: &Path, secrets: Box<dyn SecretStore>) -> Result<Self> {
        let configs = match std::fs::read_to_string(path) {
            Ok(raw) => serde_json::from_str(&raw)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self { path: path.to_owned(), secrets, configs: Mutex::new(configs) })
    }

    pub fn list(&self) -> Vec<ConnectionConfig> {
        self.configs.lock().unwrap().clone()
    }

    pub fn get(&self, id: &str) -> Result<ConnectionConfig> {
        self.list().into_iter().find(|c| c.id == id).ok_or_else(|| AppError::validation(format!("Unknown connection {id}")))
    }

    /// Inserts or updates a connection. With `save_password` and no new password, the stored one is kept.
    pub fn save(&self, config: &ConnectionConfig, password: Option<&str>) -> Result<SaveOutcome> {
        let mut config = config.clone();
        let mut password_saved = false;
        let mut password_warning = None;
        if config.save_password {
            match password {
                Some(pw) => match self.secrets.set(&config.id, pw) {
                    Ok(()) => password_saved = true,
                    Err(error) => {
                        config.save_password = false;
                        password_warning = Some(format!("The password could not be saved: {}", error.message));
                    }
                },
                None => match self.secrets.get(&config.id) {
                    Ok(Some(_)) => password_saved = true,
                    Ok(None) => config.save_password = false,
                    Err(error) => {
                        config.save_password = false;
                        password_warning = Some(format!("The saved password could not be read: {}", error.message));
                    }
                },
            }
        } else if let Err(error) = self.secrets.delete(&config.id) {
            password_warning = Some(format!(
                "Password saving is disabled, but an existing credential could not be removed from the credential store: {}",
                error.message
            ));
        }
        {
            let mut configs = self.configs.lock().unwrap();
            match configs.iter_mut().find(|c| c.id == config.id) {
                Some(existing) => *existing = config.clone(),
                None => configs.push(config.clone()),
            }
            self.persist(&configs)?;
        }
        Ok(SaveOutcome { config, password_saved, password_warning })
    }

    /// Copies a connection (and its stored password) under a new id.
    pub fn duplicate(&self, id: &str) -> Result<ConnectionConfig> {
        let source = self.get(id)?;
        let mut copy = source.clone();
        copy.id = uuid::Uuid::new_v4().to_string();
        copy.name = format!("{} (copy)", source.name);
        let password = if source.save_password { self.password(id)? } else { None };
        Ok(self.save(&copy, password.as_deref())?.config)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let mut configs = self.configs.lock().unwrap();
        configs.retain(|c| c.id != id);
        self.persist(&configs)?;
        let _ = self.secrets.delete(id);
        Ok(())
    }

    pub fn password(&self, id: &str) -> Result<Option<String>> {
        if !self.configs.lock().unwrap().iter().any(|config| config.id == id && config.save_password) {
            return Ok(None);
        }
        match self.secrets.get(id) {
            Ok(p) => Ok(p),
            Err(_) => Ok(None),
        }
    }

    fn persist(&self, configs: &[ConnectionConfig]) -> Result<()> {
        persist_json(&self.path, &serde_json::to_vec_pretty(configs)?)
    }
}

/// Writes a complete JSON snapshot without exposing partial contents or reusing a temporary path.
pub(crate) fn persist_json(path: &Path, contents: &[u8]) -> Result<()> {
    let dir = path.parent().filter(|dir| !dir.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut temporary = tempfile::NamedTempFile::new_in(dir)?;
    temporary.write_all(contents)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}
