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

/// A resolved key and where it came from. Private, so the key never leaks through it.
enum Resolved {
    Saved(String),
    FromEnvironment(String),
    Missing,
}

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

    /// The key to use now: the saved one, else the environment's.
    pub fn current(&self) -> Option<String> {
        match self.resolve() {
            Resolved::Saved(key) | Resolved::FromEnvironment(key) => Some(key),
            Resolved::Missing => None,
        }
    }

    /// Where the key [`current`](Self::current) would return comes from. Reads the
    /// store, so callers that show it repeatedly should keep the answer.
    pub fn status(&self) -> KeyStatus {
        match self.resolve() {
            Resolved::Saved(_) => KeyStatus::Saved,
            Resolved::FromEnvironment(_) => KeyStatus::FromEnvironment,
            Resolved::Missing => KeyStatus::Missing,
        }
    }

    /// The one place the precedence lives: the saved key wins over the environment's.
    /// Blank values count as absent, and a store that cannot be read counts as holding
    /// nothing. The store is read exactly once.
    fn resolve(&self) -> Resolved {
        if let Some(key) = self.saved() {
            Resolved::Saved(key)
        } else if let Some(key) = non_blank((self.environment)()) {
            Resolved::FromEnvironment(key)
        } else {
            Resolved::Missing
        }
    }

    /// Keeps `key` (trimmed) for every later request. A blank `key` changes nothing.
    pub fn save(&self, key: &str) -> Result<(), String> {
        match non_blank(Some(key.to_string())) {
            Some(key) => self.store.save(&key),
            None => Ok(()),
        }
    }

    /// Like [`save`](Self::save), but hands back what was saved before, so a change that
    /// fails later can be undone with [`PreviousKey::restore`].
    pub fn replace(&self, key: &str) -> Result<PreviousKey, String> {
        let Some(key) = non_blank(Some(key.to_string())) else {
            return Ok(PreviousKey::Unchanged);
        };
        let previous = match self.store.read()? {
            Some(saved) => PreviousKey::Saved(saved),
            None => PreviousKey::Absent,
        };
        self.store.save(&key)?;
        Ok(previous)
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

/// What the store held before [`OpenAiKey::replace`]. Its `Debug` never shows the key.
#[must_use = "a key change that is not kept must be restored"]
pub enum PreviousKey {
    /// Nothing was replaced (the new key was blank).
    Unchanged,
    /// No key was saved.
    Absent,
    /// This key was saved.
    Saved(String),
}

impl PreviousKey {
    /// Puts the store back the way it was before the replacement.
    pub fn restore(self, key: &OpenAiKey) -> Result<(), String> {
        match self {
            PreviousKey::Unchanged => Ok(()),
            PreviousKey::Absent => key.store.remove(),
            PreviousKey::Saved(saved) => key.store.save(&saved),
        }
    }
}

impl fmt::Debug for PreviousKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PreviousKey::Unchanged => "Unchanged",
            PreviousKey::Absent => "Absent",
            PreviousKey::Saved(_) => "Saved(<redacted>)",
        })
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

    /// Counts reads, to show each answer comes from a single look at the store.
    #[derive(Default)]
    struct Counting {
        inner: MemorySecretStore,
        reads: std::sync::atomic::AtomicUsize,
    }
    impl SecretStore for Counting {
        fn read(&self) -> Result<Option<String>, String> {
            self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.read()
        }
        fn save(&self, secret: &str) -> Result<(), String> {
            self.inner.save(secret)
        }
        fn remove(&self) -> Result<(), String> {
            self.inner.remove()
        }
    }

    #[test]
    fn each_answer_reads_the_store_once() {
        let store = Arc::new(Counting::default());
        let key = key_with(store.clone(), Some("sk-env"));
        key.save("sk-saved").unwrap();

        assert_eq!(key.status(), KeyStatus::Saved);
        assert_eq!(key.current().as_deref(), Some("sk-saved"));
        key.remove().unwrap();
        assert_eq!(key.status(), KeyStatus::FromEnvironment);

        assert_eq!(store.reads.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn a_replaced_key_can_be_restored() {
        let store = Arc::new(MemorySecretStore::holding("sk-old"));
        let key = key_with(store.clone(), None);

        let previous = key.replace(" sk-new ").unwrap();
        assert_eq!(store.read().unwrap().as_deref(), Some("sk-new"));
        previous.restore(&key).unwrap();

        assert_eq!(store.read().unwrap().as_deref(), Some("sk-old"));
    }

    #[test]
    fn restoring_a_first_key_removes_it() {
        let store = Arc::new(MemorySecretStore::default());
        let key = key_with(store.clone(), None);

        key.replace("sk-new").unwrap().restore(&key).unwrap();

        assert_eq!(store.read().unwrap(), None);
    }

    #[test]
    fn replacing_with_a_blank_key_changes_and_restores_nothing() {
        let store = Arc::new(MemorySecretStore::holding("sk-old"));
        let key = key_with(store.clone(), None);

        let previous = key.replace("  ").unwrap();
        store.save("sk-other").unwrap();
        previous.restore(&key).unwrap();

        assert_eq!(store.read().unwrap().as_deref(), Some("sk-other"));
    }

    #[test]
    fn replacing_fails_when_the_store_does() {
        let key = key_with(Arc::new(Broken), None);

        assert_eq!(
            key.replace("sk-new").map(|_| ()),
            Err("keychain locked".to_string())
        );
    }

    #[test]
    fn the_previous_key_is_never_shown() {
        let shown = format!("{:?}", PreviousKey::Saved("sk-old".into()));

        assert_eq!(shown, "Saved(<redacted>)");
        assert_eq!(format!("{:?}", PreviousKey::Absent), "Absent");
        assert_eq!(format!("{:?}", PreviousKey::Unchanged), "Unchanged");
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
