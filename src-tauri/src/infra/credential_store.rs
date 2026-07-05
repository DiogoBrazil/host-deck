use crate::error::{AppError, AppResult};

pub fn password_ref(connection_id: &str) -> String {
    format!("ssh-password:{connection_id}")
}

pub fn passphrase_ref(connection_id: &str) -> String {
    format!("key-passphrase:{connection_id}")
}

/// Abstraction over the OS credential store, with a mock implementation for tests.
pub trait CredentialStore: Send + Sync {
    fn set(&self, entry_ref: &str, secret: &str) -> AppResult<()>;
    fn get(&self, entry_ref: &str) -> AppResult<Option<String>>;
    fn delete(&self, entry_ref: &str) -> AppResult<()>;
}

/// OS keyring implementation.
///
/// Uses Secret Service on Linux, Keychain on macOS, and Credential Manager on Windows.
pub struct SystemKeyring {
    service: String,
}

impl SystemKeyring {
    pub fn new() -> Self {
        Self {
            service: "com.hostdeck.app".into(),
        }
    }

    fn entry(&self, entry_ref: &str) -> AppResult<keyring::Entry> {
        keyring::Entry::new(&self.service, entry_ref).map_err(map_keyring_error)
    }
}

fn map_keyring_error(e: keyring::Error) -> AppError {
    match e {
        keyring::Error::NoStorageAccess(inner) | keyring::Error::PlatformFailure(inner) => {
            AppError::CredentialStoreUnavailable(inner.to_string())
        }
        other => AppError::CredentialStore(other.to_string()),
    }
}

impl CredentialStore for SystemKeyring {
    fn set(&self, entry_ref: &str, secret: &str) -> AppResult<()> {
        self.entry(entry_ref)?
            .set_password(secret)
            .map_err(map_keyring_error)
    }

    fn get(&self, entry_ref: &str) -> AppResult<Option<String>> {
        match self.entry(entry_ref)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(map_keyring_error(e)),
        }
    }

    fn delete(&self, entry_ref: &str) -> AppResult<()> {
        match self.entry(entry_ref)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(map_keyring_error(e)),
        }
    }
}

#[cfg(test)]
mod system_tests {
    use super::*;

    /// Runs against the real OS keyring:
    /// `cargo test real_keyring -- --ignored`
    #[test]
    #[ignore]
    fn real_keyring_roundtrip() {
        let store = SystemKeyring::new();
        let entry = "test-entry:hostdeck-selftest";

        store.set(entry, "segredo-de-teste").unwrap();
        assert_eq!(
            store.get(entry).unwrap().as_deref(),
            Some("segredo-de-teste")
        );

        store.set(entry, "segredo-atualizado").unwrap();
        assert_eq!(
            store.get(entry).unwrap().as_deref(),
            Some("segredo-atualizado")
        );

        store.delete(entry).unwrap();
        assert_eq!(store.get(entry).unwrap(), None);

        store.delete(entry).unwrap();
    }
}

#[cfg(test)]
pub mod mock {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    pub struct MockStore(pub Mutex<HashMap<String, String>>);

    impl CredentialStore for MockStore {
        fn set(&self, entry_ref: &str, secret: &str) -> AppResult<()> {
            self.0
                .lock()
                .unwrap()
                .insert(entry_ref.into(), secret.into());
            Ok(())
        }

        fn get(&self, entry_ref: &str) -> AppResult<Option<String>> {
            Ok(self.0.lock().unwrap().get(entry_ref).cloned())
        }

        fn delete(&self, entry_ref: &str) -> AppResult<()> {
            self.0.lock().unwrap().remove(entry_ref);
            Ok(())
        }
    }
}
