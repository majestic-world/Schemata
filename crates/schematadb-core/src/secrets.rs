use crate::{AppError, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Storage for connection passwords, keyed by connection id.
pub trait SecretStore: Send + Sync {
    fn get(&self, id: &str) -> Result<Option<String>>;
    fn set(&self, id: &str, secret: &str) -> Result<()>;
    fn delete(&self, id: &str) -> Result<()>;
}

/// OS credential store: Windows Credential Manager, macOS Keychain or Linux Secret Service.
pub struct KeyringSecrets {
    service: String,
}

impl KeyringSecrets {
    pub fn new(service: impl Into<String>) -> Self {
        Self { service: service.into() }
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
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keyring_err(e)),
        }
    }
    fn set(&self, id: &str, secret: &str) -> Result<()> {
        self.entry(&self.service, id)?.set_password(secret).map_err(keyring_err)
    }
    fn delete(&self, id: &str) -> Result<()> {
        match self.entry(&self.service, id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keyring_err(e)),
        }
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
