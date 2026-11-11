use crate::{AppError, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Storage for connection passwords, keyed by connection id.
pub trait SecretStore: Send + Sync {
    fn get(&self, id: &str) -> Result<Option<String>>;
    fn set(&self, id: &str, secret: &str) -> Result<()>;
    fn delete(&self, id: &str) -> Result<()>;
}

const SERVICE: &str = "dev.schematadb.app";
/// Service used while the app was called ProjectCat; passwords found there move to [`SERVICE`] on first read.
const LEGACY_SERVICE: &str = "dev.projectcat.app";

/// OS credential store: Windows Credential Manager, macOS Keychain or Linux Secret Service.
#[derive(Default)]
pub struct KeyringSecrets;

fn keyring_err(e: keyring::Error) -> AppError {
    AppError::io(format!("Credential store unavailable: {e}"))
}

impl SecretStore for KeyringSecrets {
    fn get(&self, id: &str) -> Result<Option<String>> {
        match keyring::Entry::new(SERVICE, id).map_err(keyring_err)?.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::Error::NoEntry) => {
                let legacy = keyring::Entry::new(LEGACY_SERVICE, id).map_err(keyring_err)?;
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
        keyring::Entry::new(SERVICE, id).map_err(keyring_err)?.set_password(secret).map_err(keyring_err)
    }
    fn delete(&self, id: &str) -> Result<()> {
        for service in [SERVICE, LEGACY_SERVICE] {
            match keyring::Entry::new(service, id).map_err(keyring_err)?.delete_credential() {
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
