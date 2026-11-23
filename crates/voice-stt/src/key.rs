//! The OpenAI key: kept in a secret store, with the usual environment variable as a
//! fallback. The key never appears in debug output, errors or logs.
use std::fmt;
use std::sync::Arc;

/// Environment variable other OpenAI tools read the key from.
pub const OPENAI_KEY_VARIABLE: &str = "OPENAI_API_KEY";

/// Somewhere one secret can be kept, such as the macOS Keychain.
pub trait SecretStore: Send + Sync {
    /// `Ok(None)` = nothing kept.
    fn read(&self) -> Result<Option<String>, String>;
    fn save(&self, secret: &str) -> Result<(), String>;
    /// Removing a secret that is not there is not an error.
    fn remove(&self) -> Result<(), String>;
}

/// Where the key used for the next request comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStatus {
    /// Saved in the secret store (wins over the environment).
    Saved,
    /// Not saved; taken from the environment variable.
    FromEnvironment,
    /// Neither saved nor in the environment.
    Missing,
}

type EnvLookup = dyn Fn() -> Option<String> + Send + Sync;

/// Resolves, saves and removes the OpenAI key.
pub struct OpenAiKey {
    store: Arc<dyn SecretStore>,
    environment: Box<EnvLookup>,
}

impl OpenAiKey {
    /// Falls back to `OPENAI_API_KEY` of this process when the store holds no key.
    pub fn new(store: Arc<dyn SecretStore>) -> Self {
        Self::with_environment(store, || std::env::var(OPENAI_KEY_VARIABLE).ok())
    }

    /// Falls back to whatever `environment` returns when the store holds no key.
    pub fn with_environment(
        store: Arc<dyn SecretStore>,
        environment: impl Fn() -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            store,
            environment: Box::new(environment),
        }
    }

    /// The key to use now: the saved one, else the environment's. Blank values count as
    /// absent, and a store that cannot be read counts as holding nothing.
    pub fn current(&self) -> Option<String> {
        self.saved().or_else(|| non_blank((self.environment)()))
    }

    pub fn status(&self) -> KeyStatus {
        if self.saved().is_some() {
            KeyStatus::Saved
        } else if non_blank((self.environment)()).is_some() {
            KeyStatus::FromEnvironment
        } else {
            KeyStatus::Missing
        }
    }

    /// Keeps `key` (trimmed) for every later request. A blank `key` changes nothing.
    pub fn save(&self, key: &str) -> Result<(), String> {
        match non_blank(Some(key.to_string())) {
            Some(key) => self.store.save(&key),
            None => Ok(()),
        }
    }

    /// Forgets the saved key; the environment's key (if any) is used from then on.
    pub fn remove(&self) -> Result<(), String> {
        self.store.remove()
    }

    fn saved(&self) -> Option<String> {
        match self.store.read() {
            Ok(saved) => non_blank(saved),
            Err(e) => {
                log::warn!("cannot read the saved OpenAI key: {e}");
                None
            }
        }
    }
}

impl fmt::Debug for OpenAiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiKey")
            .field("status", &self.status())
            .finish()
    }
}

fn non_blank(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// A secret kept in memory only: for demo mode and tests.
#[derive(Default)]
pub struct MemorySecretStore(std::sync::Mutex<Option<String>>);

impl MemorySecretStore {
    pub fn holding(secret: &str) -> Self {
        Self(std::sync::Mutex::new(Some(secret.to_string())))
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Option<String>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl SecretStore for MemorySecretStore {
    fn read(&self) -> Result<Option<String>, String> {
        Ok(self.slot().clone())
    }

    fn save(&self, secret: &str) -> Result<(), String> {
        *self.slot() = Some(secret.to_string());
        Ok(())
    }

    fn remove(&self) -> Result<(), String> {
        *self.slot() = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store that cannot be read or written, like a locked Keychain.
    struct Broken;
    impl SecretStore for Broken {
        fn read(&self) -> Result<Option<String>, String> {
            Err("keychain locked".into())
        }
        fn save(&self, _: &str) -> Result<(), String> {
            Err("keychain locked".into())
        }
        fn remove(&self) -> Result<(), String> {
            Err("keychain locked".into())
        }
    }

    fn key_with(store: Arc<dyn SecretStore>, env: Option<&'static str>) -> OpenAiKey {
        OpenAiKey::with_environment(store, move || env.map(String::from))
    }

    #[test]
    fn a_saved_key_is_kept_in_the_store_and_used() {
        let store = Arc::new(MemorySecretStore::default());
        let key = key_with(store.clone(), None);

        key.save("sk-test").unwrap();

        assert_eq!(store.read().unwrap().as_deref(), Some("sk-test"));
        assert_eq!(key.current().as_deref(), Some("sk-test"));
        assert_eq!(key.status(), KeyStatus::Saved);
    }

    #[test]
    fn the_environment_key_is_used_when_none_is_saved() {
        let key = key_with(Arc::new(MemorySecretStore::default()), Some("sk-env"));

        assert_eq!(key.current().as_deref(), Some("sk-env"));
        assert_eq!(key.status(), KeyStatus::FromEnvironment);
    }

    #[test]
    fn the_saved_key_wins_over_the_environment() {
        let key = key_with(
            Arc::new(MemorySecretStore::holding("sk-saved")),
            Some("sk-env"),
        );

        assert_eq!(key.current().as_deref(), Some("sk-saved"));
        assert_eq!(key.status(), KeyStatus::Saved);
    }

    #[test]
    fn removing_the_saved_key_falls_back_to_the_environment() {
        let store = Arc::new(MemorySecretStore::holding("sk-saved"));
        let key = key_with(store.clone(), Some("sk-env"));

        key.remove().unwrap();

        assert_eq!(store.read().unwrap(), None);
        assert_eq!(key.current().as_deref(), Some("sk-env"));
        assert_eq!(key.status(), KeyStatus::FromEnvironment);
    }

    #[test]
    fn removing_the_only_key_leaves_none() {
        let key = key_with(Arc::new(MemorySecretStore::holding("sk-saved")), None);

        key.remove().unwrap();

        assert_eq!(key.current(), None);
        assert_eq!(key.status(), KeyStatus::Missing);
    }

    #[test]
    fn saving_a_blank_key_keeps_the_saved_one() {
        let store = Arc::new(MemorySecretStore::holding("sk-saved"));
        let key = key_with(store.clone(), None);

        key.save("   ").unwrap();

        assert_eq!(store.read().unwrap().as_deref(), Some("sk-saved"));
    }

    #[test]
    fn a_typed_key_is_trimmed() {
        let store = Arc::new(MemorySecretStore::default());

        key_with(store.clone(), None).save("  sk-test \n").unwrap();

        assert_eq!(store.read().unwrap().as_deref(), Some("sk-test"));
    }

    #[test]
    fn blank_keys_count_as_missing() {
        let key = key_with(Arc::new(MemorySecretStore::holding("  ")), Some(" "));

        assert_eq!(key.current(), None);
        assert_eq!(key.status(), KeyStatus::Missing);
    }

    #[test]
    fn an_unreadable_store_falls_back_to_the_environment() {
        let key = key_with(Arc::new(Broken), Some("sk-env"));

        assert_eq!(key.current().as_deref(), Some("sk-env"));
        assert_eq!(key.status(), KeyStatus::FromEnvironment);
    }

    #[test]
    fn store_failures_are_reported() {
        let key = key_with(Arc::new(Broken), None);

        assert_eq!(key.save("sk-test"), Err("keychain locked".to_string()));
        assert_eq!(key.remove(), Err("keychain locked".to_string()));
    }

    #[test]
    fn debug_output_never_shows_the_key() {
        let key = key_with(Arc::new(MemorySecretStore::holding("sk-secret")), None);

        let shown = format!("{key:?}");

        assert!(!shown.contains("sk-secret"), "{shown}");
        assert!(shown.contains("Saved"), "{shown}");
    }

    #[test]
    fn the_openai_api_key_variable_of_this_process_is_the_fallback() {
        // Only this test reads the real variable; the others inject their environment.
        std::env::set_var("OPENAI_API_KEY", "sk-from-process");
        let key = OpenAiKey::new(Arc::new(MemorySecretStore::default()));

        let found = key.current();
        std::env::remove_var("OPENAI_API_KEY");

        assert_eq!(found.as_deref(), Some("sk-from-process"));
    }
}
