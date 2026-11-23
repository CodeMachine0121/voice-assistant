//! One secret kept as a generic password in the user's login Keychain.
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

/// Keychain status code for "no such item".
const ITEM_NOT_FOUND: i32 = -25300;

/// Keychain service under which the app keeps its secrets.
pub const KEYCHAIN_SERVICE: &str = "VoiceAssistant";
/// Keychain account of the OpenAI key.
pub const OPENAI_KEY_ACCOUNT: &str = "openai-api-key";

/// A generic-password Keychain item named by service and account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeychainSecret {
    service: String,
    account: String,
}

impl KeychainSecret {
    pub fn new(service: &str, account: &str) -> Self {
        Self {
            service: service.to_string(),
            account: account.to_string(),
        }
    }

    /// Where the OpenAI key is kept.
    pub fn openai_key() -> Self {
        Self::new(KEYCHAIN_SERVICE, OPENAI_KEY_ACCOUNT)
    }

    /// `Ok(None)` when the item does not exist.
    pub fn read(&self) -> Result<Option<String>, String> {
        read_result(get_generic_password(&self.service, &self.account).map_err(|e| e.code()))
    }

    /// Creates the item or replaces its secret.
    pub fn save(&self, secret: &str) -> Result<(), String> {
        set_generic_password(&self.service, &self.account, secret.as_bytes())
            .map_err(|e| format!("cannot save to the Keychain: {e}"))
    }

    /// Deletes the item; an item that does not exist is already removed.
    pub fn remove(&self) -> Result<(), String> {
        removal_result(delete_generic_password(&self.service, &self.account).map_err(|e| e.code()))
    }
}

fn read_result(result: Result<Vec<u8>, i32>) -> Result<Option<String>, String> {
    match result {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| "the Keychain item is not text".to_string()),
        Err(ITEM_NOT_FOUND) => Ok(None),
        Err(code) => Err(format!("cannot read the Keychain (status {code})")),
    }
}

fn removal_result(result: Result<(), i32>) -> Result<(), String> {
    match result {
        Ok(()) | Err(ITEM_NOT_FOUND) => Ok(()),
        Err(code) => Err(format!("cannot remove from the Keychain (status {code})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_secret_is_read_as_text() {
        assert_eq!(
            read_result(Ok(b"sk-test".to_vec())),
            Ok(Some("sk-test".to_string()))
        );
    }

    #[test]
    fn a_missing_item_reads_as_nothing() {
        assert_eq!(read_result(Err(ITEM_NOT_FOUND)), Ok(None));
    }

    #[test]
    fn other_read_failures_are_errors_without_the_secret() {
        assert_eq!(
            read_result(Err(-25293)),
            Err("cannot read the Keychain (status -25293)".to_string())
        );
        assert_eq!(
            read_result(Ok(vec![0xff, 0xfe])),
            Err("the Keychain item is not text".to_string())
        );
    }

    #[test]
    fn removing_a_missing_item_is_fine_but_other_failures_are_not() {
        assert_eq!(removal_result(Ok(())), Ok(()));
        assert_eq!(removal_result(Err(ITEM_NOT_FOUND)), Ok(()));
        assert_eq!(
            removal_result(Err(-25293)),
            Err("cannot remove from the Keychain (status -25293)".to_string())
        );
    }

    #[test]
    fn the_openai_key_lives_under_the_app_service() {
        assert_eq!(
            KeychainSecret::openai_key(),
            KeychainSecret::new("VoiceAssistant", "openai-api-key")
        );
    }

    /// Touches the real login Keychain: `cargo test -p voice-macos -- --ignored`.
    #[test]
    #[ignore = "uses the real Keychain"]
    fn live_keychain_round_trip() {
        let item = KeychainSecret::new("VoiceAssistant.tests", "round-trip");
        item.remove().unwrap();
        assert_eq!(item.read(), Ok(None));
        item.save("sk-one").unwrap();
        item.save("sk-two").unwrap();
        assert_eq!(item.read(), Ok(Some("sk-two".to_string())));
        item.remove().unwrap();
        assert_eq!(item.read(), Ok(None));
    }
}
