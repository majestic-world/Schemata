use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use tauri::utils::config::{AppDirectoriesOverride, Config};

/// One profile per canonical executable directory, independent of working directory and build version.
pub(crate) struct InstallationProfile {
    pub directory: PathBuf,
    identifier: String,
}

impl InstallationProfile {
    pub fn current() -> io::Result<Self> {
        let data = dirs::data_dir().ok_or_else(|| io::Error::other("Cannot resolve the user's application data directory"))?;
        #[cfg(target_os = "linux")]
        let executable = match std::env::var_os("APPIMAGE") {
            Some(path) => PathBuf::from(path),
            None => std::env::current_exe()?,
        };
        #[cfg(not(target_os = "linux"))]
        let executable = std::env::current_exe()?;
        Self::for_executable(&executable, &data)
    }

    fn for_executable(executable: &Path, data: &Path) -> io::Result<Self> {
        let parent = executable.parent().ok_or_else(|| io::Error::other("Executable has no installation directory"))?;
        let directory = dunce::canonicalize(parent)?;
        let path = directory.to_str().ok_or_else(|| io::Error::other("Installation directory is not valid Unicode"))?;
        // Windows directory casing and verbatim path prefixes must not create a second profile.
        #[cfg(windows)]
        let path = path.to_lowercase();
        let hash = format!("{:x}", md5::compute(path.as_bytes()));
        Ok(Self {
            directory: data.join("schemata").join(&hash),
            identifier: format!("dev.schematadb.app.p{hash}"),
        })
    }

    pub fn configure(&self, config: &mut Config) {
        config.identifier.clone_from(&self.identifier);
        // Config, data, local data (including WebView2), window state, cache and logs all follow this root.
        config.app.app_directories_override = Some(AppDirectoriesOverride::Root(self.directory.clone()));
    }
}

/// Held by Tauri state until shutdown, before reading persistent files.
pub(crate) struct PersistenceLock {
    _file: File,
}

pub(crate) fn lock_app_directory(directory: &Path) -> io::Result<PersistenceLock> {
    std::fs::create_dir_all(directory)?;
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(directory.join("instance.lock"))?;
    file.try_lock().map_err(|error| {
        io::Error::other(format!(
            "Cannot lock Schemata's configuration directory. Another Schemata instance may already be running; close it before starting this instance. {error}"
        ))
    })?;
    Ok(PersistenceLock { _file: file })
}

#[cfg(test)]
mod tests {
    use super::{InstallationProfile, lock_app_directory};
    use schematadb_core::{ConnectionConfig, ConnectionStore, Engine, MemorySecrets, QueryHistory};
    use std::path::Path;

    fn profile(install: &Path, data: &Path) -> InstallationProfile {
        std::fs::create_dir_all(install).unwrap();
        InstallationProfile::for_executable(&install.join("schematadb.exe"), data).unwrap()
    }

    #[test]
    fn separate_installations_keep_their_own_connections_and_history() {
        let temp = tempfile::tempdir().unwrap();
        let first = profile(&temp.path().join("first"), temp.path());
        let second = profile(&temp.path().join("second"), temp.path());
        let _first_lock = lock_app_directory(&first.directory).unwrap();
        let _second_lock = lock_app_directory(&second.directory).unwrap();
        let config = ConnectionConfig::new("First installation", Engine::Sqlite);
        let store = ConnectionStore::open(&first.directory.join("connections.json"), Box::new(MemorySecrets::default())).unwrap();
        store.save(&config, None).unwrap();
        QueryHistory::open(&first.directory.join("history.json")).unwrap().record(&config.id, "SELECT 1", true, 1).unwrap();

        let other = ConnectionStore::open(&second.directory.join("connections.json"), Box::new(MemorySecrets::default())).unwrap();
        assert!(other.list().is_empty());
        assert!(QueryHistory::open(&second.directory.join("history.json")).unwrap().list(&config.id).is_empty());
        let reopened = profile(&temp.path().join("first"), temp.path());
        let saved = ConnectionStore::open(&reopened.directory.join("connections.json"), Box::new(MemorySecrets::default())).unwrap();
        assert_eq!(saved.list(), vec![config.clone()]);
        assert_eq!(QueryHistory::open(&reopened.directory.join("history.json")).unwrap().list(&config.id)[0].sql, "SELECT 1");
    }

    #[test]
    fn deleting_a_profile_starts_empty_even_when_old_profiles_remain() {
        let temp = tempfile::tempdir().unwrap();
        let profile = profile(&temp.path().join("install"), temp.path());
        let config = ConnectionConfig::new("Do not restore", Engine::Sqlite);
        let legacy = temp.path().join("dev.projectcat.app");
        std::fs::create_dir_all(&legacy).unwrap();
        let legacy_store = ConnectionStore::open(&legacy.join("connections.json"), Box::new(MemorySecrets::default())).unwrap();
        legacy_store.save(&config, None).unwrap();
        let legacy_history = QueryHistory::open(&legacy.join("history.json")).unwrap();
        legacy_history.record(&config.id, "SELECT 99", true, 1).unwrap();
        {
            let _lock = lock_app_directory(&profile.directory).unwrap();
            let store = ConnectionStore::open(&profile.directory.join("connections.json"), Box::new(MemorySecrets::default())).unwrap();
            store.save(&config, None).unwrap();
            QueryHistory::open(&profile.directory.join("history.json")).unwrap().record(&config.id, "SELECT 1", true, 1).unwrap();
        }
        std::fs::remove_dir_all(&profile.directory).unwrap();
        let _lock = lock_app_directory(&profile.directory).unwrap();
        let store = ConnectionStore::open(&profile.directory.join("connections.json"), Box::new(MemorySecrets::default())).unwrap();
        assert!(store.list().is_empty());
        assert!(QueryHistory::open(&profile.directory.join("history.json")).unwrap().list(&config.id).is_empty());
        assert_eq!(legacy_store.list(), vec![config.clone()]);
        assert_eq!(legacy_history.list(&config.id)[0].sql, "SELECT 99");
    }

    #[cfg(windows)]
    #[test]
    fn windows_directory_aliases_share_the_same_profile() {
        let temp = tempfile::tempdir().unwrap();
        let install = temp.path().join("MixedCase");
        let original = profile(&install, temp.path());
        // Resolve the executable's parent, not the caller's working directory or filename.
        let alternate = InstallationProfile::for_executable(&install.join("alternate.exe"), temp.path()).unwrap();
        assert_eq!(original.directory, alternate.directory);
        let verbatim = std::fs::canonicalize(&install).unwrap();
        let aliased = InstallationProfile::for_executable(&verbatim.join("schematadb.exe"), temp.path()).unwrap();
        assert_eq!(original.directory, aliased.directory);
        let uppercase = std::path::PathBuf::from(install.to_str().unwrap().to_uppercase()).join("schematadb.exe");
        let uppercase_profile = InstallationProfile::for_executable(&uppercase, temp.path()).unwrap();
        assert_eq!(original.directory, uppercase_profile.directory);
    }

    #[test]
    fn persistence_lock_rejects_another_instance_until_shutdown() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path();
        let connections = directory.join("connections.json");
        std::fs::write(&connections, b"existing connections").unwrap();
        let first = lock_app_directory(directory).unwrap();
        let error = lock_app_directory(directory).err().expect("a second instance must not acquire the lock");
        assert!(error.to_string().contains("Another Schemata instance"));
        assert_eq!(std::fs::read(&connections).unwrap(), b"existing connections");
        drop(first);
        let _next = lock_app_directory(directory).unwrap();
    }
}
