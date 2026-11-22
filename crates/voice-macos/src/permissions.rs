//! macOS privacy permissions: microphone and accessibility.
use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};
use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
use std::process::Command;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
}

/// A page of System Settings › Privacy & Security.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyPane {
    Microphone,
    Accessibility,
    InputMonitoring,
}

impl PrivacyPane {
    pub fn url(self) -> &'static str {
        match self {
            PrivacyPane::Microphone => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
            }
            PrivacyPane::Accessibility => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
            }
            PrivacyPane::InputMonitoring => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent"
            }
        }
    }
}

/// `Some(granted)` once the user decided; `None` while macOS has not asked yet.
fn microphone_decision(status: AVAuthorizationStatus) -> Option<bool> {
    if status == AVAuthorizationStatus::NotDetermined {
        None
    } else {
        Some(status == AVAuthorizationStatus::Authorized)
    }
}

/// Checks and requests the permissions dictation needs.
#[derive(Debug, Clone, Copy, Default)]
pub struct MacPermissions;

impl MacPermissions {
    /// The app may control the computer (needed to paste into other apps).
    pub fn accessibility_granted() -> bool {
        // SAFETY: plain query without arguments.
        unsafe { AXIsProcessTrusted() }
    }

    /// Shows the system prompt asking for accessibility access (if not granted).
    /// Returns whether access is granted right now.
    pub fn prompt_accessibility() -> bool {
        // SAFETY: the key is an immutable framework constant.
        let key = unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) };
        let options = CFDictionary::from_CFType_pairs(&[(key, CFBoolean::true_value())]);
        // SAFETY: `options` is a valid dictionary for the duration of the call.
        unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) }
    }

    /// `Some(true)` granted, `Some(false)` denied or restricted, `None` not asked yet.
    pub fn microphone_granted() -> Option<bool> {
        // SAFETY: framework constant; the media type is one the call accepts.
        let status = unsafe {
            let Some(audio) = AVMediaTypeAudio else {
                return Some(false);
            };
            AVCaptureDevice::authorizationStatusForMediaType(audio)
        };
        microphone_decision(status)
    }

    /// Shows the system microphone prompt if the user has not decided yet.
    /// Returns immediately; the answer is read later with `microphone_granted`.
    pub fn request_microphone() {
        let handler = block2::RcBlock::new(|granted: objc2::runtime::Bool| {
            log::info!("microphone access granted: {}", granted.as_bool());
        });
        // SAFETY: framework constant; the media type is one the call accepts.
        unsafe {
            if let Some(audio) = AVMediaTypeAudio {
                AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &handler);
            }
        }
    }

    /// Opens the given page of System Settings.
    pub fn open_privacy_settings(pane: PrivacyPane) -> std::io::Result<()> {
        Command::new("open").arg(pane.url()).spawn().map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn microphone_decision_follows_authorization_status() {
        let decisions = [
            AVAuthorizationStatus::Authorized,
            AVAuthorizationStatus::Denied,
            AVAuthorizationStatus::Restricted,
            AVAuthorizationStatus::NotDetermined,
        ]
        .map(microphone_decision);

        assert_eq!(decisions, [Some(true), Some(false), Some(false), None]);
    }

    #[test]
    fn privacy_panes_open_their_settings_page() {
        let urls = [
            PrivacyPane::Microphone,
            PrivacyPane::Accessibility,
            PrivacyPane::InputMonitoring,
        ]
        .map(PrivacyPane::url);

        assert_eq!(
            urls,
            [
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone",
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent",
            ]
        );
    }
}
