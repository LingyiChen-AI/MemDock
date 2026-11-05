//! GitHub token storage. The token lives only in the system keychain.

use std::sync::Mutex;

pub trait TokenStore: Send + Sync {
    fn get(&self) -> Result<Option<String>, String>;
    fn set(&self, t: &str) -> Result<(), String>;
    fn clear(&self) -> Result<(), String>;
}

/// System keychain (service `MemDock`, account `github`).
pub struct KeyringStore;

impl KeyringStore {
    fn entry() -> Result<keyring::Entry, String> {
        keyring::Entry::new("MemDock", "github").map_err(|e| format!("keychain: {e}"))
    }
}

impl TokenStore for KeyringStore {
    fn get(&self) -> Result<Option<String>, String> {
        match Self::entry()?.get_password() {
            Ok(t) => Ok(Some(t)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("keychain: {e}")),
        }
    }

    fn set(&self, t: &str) -> Result<(), String> {
        Self::entry()?
            .set_password(t)
            .map_err(|e| format!("keychain: {e}"))
    }

    fn clear(&self) -> Result<(), String> {
        match Self::entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("keychain: {e}")),
        }
    }
}

/// In-memory store for tests.
#[derive(Default)]
pub struct MemoryTokenStore(Mutex<Option<String>>);

impl MemoryTokenStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl TokenStore for MemoryTokenStore {
    fn get(&self) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn set(&self, t: &str) -> Result<(), String> {
        *self.0.lock().unwrap() = Some(t.to_string());
        Ok(())
    }
    fn clear(&self) -> Result<(), String> {
        *self.0.lock().unwrap() = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_token_store_roundtrip() {
        let s = MemoryTokenStore::new();
        assert_eq!(s.get().unwrap(), None);
        s.set("tok").unwrap();
        assert_eq!(s.get().unwrap().as_deref(), Some("tok"));
        s.clear().unwrap();
        assert_eq!(s.get().unwrap(), None);
        s.clear().unwrap();
    }
}
