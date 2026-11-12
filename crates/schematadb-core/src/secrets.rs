use crate::{AppError, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Storage for connection passwords, keyed by connection id.
pub trait SecretStore: Send + Sync {
    fn get(&self, id: &str) -> Result<Option<String>>;
    fn set(&self, id: &str, secret: &str) -> Result<()>;
    fn delete(&self, id: &str) -> Result<()>;
}

/// Service of the installed release. Only it may adopt passwords from [`LEGACY_SERVICE`]; any other service
/// (the dev build's) starts empty and never reads the release's credentials.
const PRODUCTION_SERVICE: &str = "dev.schematadb.app";
/// Service used while the app was called ProjectCat; passwords found there move to [`PRODUCTION_SERVICE`] on first read.
const LEGACY_SERVICE: &str = "dev.projectcat.app";

/// OS credential store: Windows Credential Manager, macOS Keychain or Linux Secret Service.
pub struct KeyringSecrets {
    service: String,
}

impl KeyringSecrets {
    pub fn new(service: impl Into<String>) -> Self {
        Self { service: service.into() }
    }

    fn legacy_service(&self) -> Option<&'static str> {
        (self.service == PRODUCTION_SERVICE).then_some(LEGACY_SERVICE)
    }

    fn entry(&self, service: &str, id: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(service, id).map_err(keyring_err)
    }
}

fn keyring_err(e: keyring::Error) -> AppError {
    AppError::io(format!("Credential store unavailable: {e}"))
}

impl SecretStore for KeyringSecrets {
    fn get(&self, id: &str) -> Result<Option<String>> {
        match self.entry(&self.service, id)?.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::Error::NoEntry) => {
                let Some(legacy_service) = self.legacy_service() else { return Ok(None) };
                let legacy = self.entry(legacy_service, id)?;
                match legacy.get_password() {
                    Ok(p) => {
                        self.set(id, &p)?;
                        let _ = legacy.delete_credential();
                        Ok(Some(p))
                    }
                    Err(keyring::Error::NoEntry) => Ok(None),
                    Err(e) => Err(keyring_err(e)),
                }
            }
            Err(e) => Err(keyring_err(e)),
        }
    }
    fn set(&self, id: &str, secret: &str) -> Result<()> {
        self.entry(&self.service, id)?.set_password(secret).map_err(keyring_err)
    }
    fn delete(&self, id: &str) -> Result<()> {
        for service in std::iter::once(self.service.as_str()).chain(self.legacy_service()) {
            match self.entry(service, id)?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(e) => return Err(keyring_err(e)),
            }
        }
        Ok(())
    }
}

/// In-process secret store; `unavailable()` simulates a missing OS credential service.
#[derive(Clone, Default)]
pub struct MemorySecrets {
    map: Arc<Mutex<HashMap<String, String>>>,
    unavailable: bool,
}

impl MemorySecrets {
    pub fn unavailable() -> Self {
        Self { unavailable: true, ..Self::default() }
    }
    fn check(&self) -> Result<()> {
        if self.unavailable { Err(AppError::io("Credential store unavailable")) } else { Ok(()) }
    }
}

impl SecretStore for MemorySecrets {
    fn get(&self, id: &str) -> Result<Option<String>> {
        self.check()?;
        Ok(self.map.lock().unwrap().get(id).cloned())
    }
    fn set(&self, id: &str, secret: &str) -> Result<()> {
        self.check()?;
        self.map.lock().unwrap().insert(id.to_owned(), secret.to_owned());
        Ok(())
    }
    fn delete(&self, id: &str) -> Result<()> {
        self.check()?;
        self.map.lock().unwrap().remove(id);
        Ok(())
    }
}
