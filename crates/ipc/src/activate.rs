//! Focus handoff between Craft apps.
//!
//! The platform calls live behind [`ActivationBackend`]. An app adapter
//! implements it with the real winit and egui calls, so this crate depends on
//! neither. The decision logic here is plain Rust and runs against a fake
//! backend in tests.
//!
//! | Platform | Sender | Receiver |
//! |---|---|---|
//! | Linux | mint a token from the focused window | set the pending token, then `Focus` |
//! | Windows | none | `Focus` (winit's foreground path) |
//! | macOS | yield activation to the target bundle | `activate`, then `Focus` |
//!
//! Wayland cannot confirm an activation synchronously, so the answer there is
//! [`Activated::Requested`]. When the platform refuses, the receiver asks for
//! user attention and answers [`Activated::No`].

use crate::server::Activation;

/// The answer to `app.activate`. Same three values as the wire reply.
pub type Activated = Activation;

/// What a focus request did, as far as the adapter can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusResult {
    /// The window has focus now.
    Focused,
    /// The request was sent. The compositor or window manager decides later.
    Pending,
    /// The platform refused or the window is gone.
    Refused,
}

/// Platform calls an app adapter provides.
///
/// Every method is called on the thread that owns the UI. The adapter maps
/// them to winit and egui:
///
/// - `mint_token`: `request_token_for_focused_window`, then the token from
///   `WindowEvent::ActivationTokenDone`. Return the last token or `None`.
/// - `set_pending_token`: `set_activation_token_env(ActivationToken::from_raw(..))`.
/// - `focus`: `ViewportCommand::Focus`.
/// - `request_user_attention`: `ViewportCommand::RequestUserAttention`.
/// - `yield_activation_to` and `activate_application`: the `objc2-app-kit`
///   `NSApplication` and `NSRunningApplication` calls, macOS only.
pub trait ActivationBackend {
    /// A fresh activation token, or `None` when the platform cannot mint one.
    fn mint_token(&self) -> Option<String>;
    /// Store `token` so the next `focus` call applies it.
    fn set_pending_token(&self, token: &str);
    /// Ask the window to take focus.
    fn focus(&self) -> FocusResult;
    /// Flash the taskbar or dock entry. Used when focus is refused.
    fn request_user_attention(&self);
    /// macOS sender. Hand activation to the app with `bundle_id`. Returns false on failure.
    fn yield_activation_to(&self, bundle_id: &str) -> bool;
    /// macOS receiver. Activate this app. Returns false on failure.
    fn activate_application(&self) -> bool;
}

/// The platform whose rules apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Windows,
    MacOs,
    /// Anything else. No handoff is possible.
    Other,
}

impl Platform {
    pub const fn current() -> Self {
        if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Other
        }
    }
}

/// Sender side. A token to put in `app.activate`, or `None`.
///
/// Only Linux has tokens. Windows and macOS return `None`.
pub fn mint_token(backend: &dyn ActivationBackend) -> Option<String> {
    mint_token_on(Platform::current(), backend)
}

pub fn mint_token_on(platform: Platform, backend: &dyn ActivationBackend) -> Option<String> {
    match platform {
        Platform::Linux => backend.mint_token().filter(|token| !token.is_empty()),
        Platform::Windows | Platform::MacOs | Platform::Other => None,
    }
}

/// Sender side, before `app.activate`. macOS yields activation to `bundle_id`.
/// Other platforms do nothing and return true. A false return means the yield failed.
pub fn yield_to(backend: &dyn ActivationBackend, bundle_id: &str) -> bool {
    yield_to_on(Platform::current(), backend, bundle_id)
}

pub fn yield_to_on(platform: Platform, backend: &dyn ActivationBackend, bundle_id: &str) -> bool {
    match platform {
        Platform::MacOs => backend.yield_activation_to(bundle_id),
        Platform::Linux | Platform::Windows | Platform::Other => true,
    }
}

/// Receiver side. Apply `app.activate`. `token` is the sender's token or `None`.
pub fn apply_activation(backend: &dyn ActivationBackend, token: Option<&str>) -> Activated {
    apply_activation_on(Platform::current(), backend, token)
}

pub fn apply_activation_on(platform: Platform, backend: &dyn ActivationBackend, token: Option<&str>) -> Activated {
    match platform {
        Platform::Linux => {
            let token = token.filter(|text| !text.is_empty());
            if let Some(token) = token {
                backend.set_pending_token(token);
            }
            let result = backend.focus();
            // Without a token Wayland ignores focus. Pending would claim success we cannot back.
            let result = match (result, token) {
                (FocusResult::Pending, None) => FocusResult::Refused,
                (other, _) => other,
            };
            settle(backend, result)
        }
        Platform::Windows => settle(backend, backend.focus()),
        Platform::MacOs => {
            if !backend.activate_application() {
                backend.request_user_attention();
                return Activated::No;
            }
            settle(backend, backend.focus())
        }
        Platform::Other => {
            backend.request_user_attention();
            Activated::No
        }
    }
}

fn settle(backend: &dyn ActivationBackend, result: FocusResult) -> Activated {
    match result {
        FocusResult::Focused => Activated::Yes,
        FocusResult::Pending => Activated::Requested,
        FocusResult::Refused => {
            backend.request_user_attention();
            Activated::No
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[derive(Default)]
    struct Fake {
        minted: Option<String>,
        focus: Option<FocusResult>,
        yield_ok: bool,
        activate_ok: bool,
        calls: RefCell<Vec<String>>,
    }

    impl Fake {
        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
        fn log(&self, text: impl Into<String>) {
            self.calls.borrow_mut().push(text.into());
        }
    }

    impl ActivationBackend for Fake {
        fn mint_token(&self) -> Option<String> {
            self.log("mint");
            self.minted.clone()
        }
        fn set_pending_token(&self, token: &str) {
            self.log(format!("pending:{token}"));
        }
        fn focus(&self) -> FocusResult {
            self.log("focus");
            self.focus.unwrap_or(FocusResult::Refused)
        }
        fn request_user_attention(&self) {
            self.log("attention");
        }
        fn yield_activation_to(&self, bundle_id: &str) -> bool {
            self.log(format!("yield:{bundle_id}"));
            self.yield_ok
        }
        fn activate_application(&self) -> bool {
            self.log("activate");
            self.activate_ok
        }
    }

    #[test]
    fn linux_sets_the_token_before_focus_and_reports_requested() {
        let fake = Fake { focus: Some(FocusResult::Pending), ..Fake::default() };
        let got = apply_activation_on(Platform::Linux, &fake, Some("tok"));
        assert_eq!(got, Activated::Requested);
        assert_eq!(fake.calls(), ["pending:tok", "focus"]);
    }

    #[test]
    fn linux_without_a_token_cannot_claim_requested() {
        let fake = Fake { focus: Some(FocusResult::Pending), ..Fake::default() };
        let got = apply_activation_on(Platform::Linux, &fake, None);
        assert_eq!(got, Activated::No);
        assert_eq!(fake.calls(), ["focus", "attention"]);
    }

    #[test]
    fn linux_empty_token_counts_as_none() {
        let fake = Fake { focus: Some(FocusResult::Pending), ..Fake::default() };
        let got = apply_activation_on(Platform::Linux, &fake, Some(""));
        assert_eq!(got, Activated::No);
        assert!(!fake.calls().iter().any(|call| call.starts_with("pending")));
    }

    #[test]
    fn linux_focused_window_is_yes_without_attention() {
        let fake = Fake { focus: Some(FocusResult::Focused), ..Fake::default() };
        assert_eq!(apply_activation_on(Platform::Linux, &fake, None), Activated::Yes);
        assert_eq!(fake.calls(), ["focus"]);
    }

    #[test]
    fn refusal_requests_attention_on_every_platform() {
        for platform in [Platform::Linux, Platform::Windows, Platform::MacOs] {
            let fake = Fake { focus: Some(FocusResult::Refused), activate_ok: true, ..Fake::default() };
            assert_eq!(apply_activation_on(platform, &fake, Some("t")), Activated::No, "{platform:?}");
            assert_eq!(fake.calls().last().map(String::as_str), Some("attention"), "{platform:?}");
        }
    }

    #[test]
    fn windows_ignores_the_token() {
        let fake = Fake { focus: Some(FocusResult::Pending), ..Fake::default() };
        assert_eq!(apply_activation_on(Platform::Windows, &fake, Some("t")), Activated::Requested);
        assert_eq!(fake.calls(), ["focus"]);
    }

    #[test]
    fn macos_activates_then_focuses() {
        let fake = Fake { focus: Some(FocusResult::Focused), activate_ok: true, ..Fake::default() };
        assert_eq!(apply_activation_on(Platform::MacOs, &fake, None), Activated::Yes);
        assert_eq!(fake.calls(), ["activate", "focus"]);
    }

    #[test]
    fn macos_activation_failure_is_no_with_attention() {
        let fake = Fake { focus: Some(FocusResult::Focused), activate_ok: false, ..Fake::default() };
        assert_eq!(apply_activation_on(Platform::MacOs, &fake, None), Activated::No);
        assert_eq!(fake.calls(), ["activate", "attention"]);
    }

    #[test]
    fn other_platforms_only_ask_for_attention() {
        let fake = Fake::default();
        assert_eq!(apply_activation_on(Platform::Other, &fake, Some("t")), Activated::No);
        assert_eq!(fake.calls(), ["attention"]);
    }

    #[test]
    fn only_linux_mints_tokens() {
        let fake = Fake { minted: Some("abc".into()), ..Fake::default() };
        assert_eq!(mint_token_on(Platform::Linux, &fake).as_deref(), Some("abc"));
        assert_eq!(mint_token_on(Platform::Windows, &fake), None);
        assert_eq!(mint_token_on(Platform::MacOs, &fake), None);
        let empty = Fake { minted: Some(String::new()), ..Fake::default() };
        assert_eq!(mint_token_on(Platform::Linux, &empty), None);
    }

    #[test]
    fn only_macos_yields() {
        let fake = Fake { yield_ok: false, ..Fake::default() };
        assert!(yield_to_on(Platform::Linux, &fake, "com.x"));
        assert!(fake.calls().is_empty());
        assert!(!yield_to_on(Platform::MacOs, &fake, "com.x"));
        assert_eq!(fake.calls(), ["yield:com.x"]);
    }

    #[test]
    fn current_platform_matches_the_build() {
        assert_eq!(Platform::current() == Platform::Linux, cfg!(target_os = "linux"));
    }
}
