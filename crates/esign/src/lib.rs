//! What an e-sign service has to offer the Craft apps, and nothing about who
//! provides it.
//!
//! An envelope is a ceremony: a document snapshot, a set of recipients, field
//! placements, and a status. The legal audit trail, identity checks and
//! invitation mail belong to the signing service. This crate only names the
//! contracts an app needs to create, send, watch and finish that ceremony.
//!
//! Nothing in this crate opens a socket. Each trait is implemented by a host
//! (CollabCraft, a desktop adapter, a test double) that brings its own
//! transport. No type here names a vendor. No type here knows about PDF
//! structure: a document is opaque bytes plus a content hash.
//!
//! This crate sits beside `craft-cloud`. It reuses [`craft_cloud::Identity`]
//! and the idea of a contact for autocomplete, but it does not extend
//! [`craft_cloud::Provider`] or [`craft_cloud::Capability`]. Signing is not
//! file access, and a signer role is not a share permission.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod error;
mod types;

pub use error::{Error, Result};
pub use types::{
    CompletedDocument, Envelope, EnvelopeDraft, EnvelopeEvent, EnvelopeStatus, EsignCapability, FieldPlacement, Recipient, RecipientStatus,
};

/// Creating and driving envelopes.
///
/// Methods that a minimal service may omit are defaulted to
/// [`Error::Unsupported`], the same pattern [`craft_cloud::Conversations`]
/// uses for optional cloud features.
pub trait Envelopes {
    /// Create a draft from a document snapshot.
    ///
    /// `document` is the exact byte sequence the ceremony seals. The
    /// implementation hashes it and stores `content_sha256` on the envelope.
    /// `idempotency_key` makes a retried create return the same envelope.
    fn create(&self, draft: &EnvelopeDraft, document: &[u8], idempotency_key: &str) -> Result<Envelope>;

    /// Send invitations and move the envelope to [`EnvelopeStatus::Sent`].
    fn send(&self, envelope: &str) -> Result<Envelope>;

    /// Current status, including per-recipient progress.
    fn status(&self, envelope: &str) -> Result<Envelope>;

    /// The finished document and provider evidence.
    ///
    /// Fails unless status is [`EnvelopeStatus::Completed`].
    fn completed(&self, envelope: &str) -> Result<CompletedDocument>;

    /// Cancel an in-flight envelope.
    fn void(&self, _envelope: &str, _reason: &str) -> Result<()> {
        Err(Error::Unsupported("void"))
    }

    /// Nudge one recipient who has not signed yet.
    fn remind(&self, _envelope: &str, _recipient: &str) -> Result<()> {
        Err(Error::Unsupported("reminders"))
    }

    /// A URL the recipient opens to sign inside the provider's UI.
    fn signing_url(&self, _envelope: &str, _recipient: &str, _return_to: &str) -> Result<String> {
        Err(Error::Unsupported("embedded signing"))
    }
}

/// Turning provider webhooks into [`EnvelopeEvent`]s.
///
/// Server-side only. A desktop host that implements [`Envelopes`] never needs
/// a webhook secret and need not implement this beyond refusing.
pub trait EventIntake {
    /// Verify authenticity and parse zero or more events from one HTTP body.
    fn verify_event(&self, headers: &[(String, String)], body: &[u8]) -> Result<Vec<EnvelopeEvent>>;
}

/// Everything one signing service can do.
///
/// A service implements the capabilities it has and reports the rest as
/// absent, so a caller can hide void on a stub that only supports the happy
/// path rather than offering a control that fails on use.
pub trait SigningService: Envelopes + EventIntake {
    /// A stable identifier, such as `stub` or `documenso`. Used in
    /// configuration and logs, never shown to a user.
    fn id(&self) -> &str;

    /// What this service actually supports.
    fn capabilities(&self) -> Vec<EsignCapability>;

    fn supports(&self, capability: EsignCapability) -> bool {
        self.capabilities().contains(&capability)
    }

    /// The party that holds legal and compliance responsibility for the
    /// ceremony. Shown to senders before they send (upstream #2315).
    fn responsible_party(&self) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_cloud::Identity;

    struct RefuseAll;

    impl Envelopes for RefuseAll {
        fn create(&self, _: &EnvelopeDraft, _: &[u8], _: &str) -> Result<Envelope> {
            Err(Error::Unsupported("create"))
        }

        fn send(&self, _: &str) -> Result<Envelope> {
            Err(Error::Unsupported("send"))
        }

        fn status(&self, _: &str) -> Result<Envelope> {
            Err(Error::Unsupported("status"))
        }

        fn completed(&self, _: &str) -> Result<CompletedDocument> {
            Err(Error::Unsupported("completed"))
        }
    }

    impl EventIntake for RefuseAll {
        fn verify_event(&self, _: &[(String, String)], _: &[u8]) -> Result<Vec<EnvelopeEvent>> {
            Err(Error::Unsupported("webhooks"))
        }
    }

    impl SigningService for RefuseAll {
        fn id(&self) -> &str {
            "refuse"
        }

        fn capabilities(&self) -> Vec<EsignCapability> {
            Vec::new()
        }

        fn responsible_party(&self) -> String {
            "nobody".into()
        }
    }

    #[test]
    fn defaulted_methods_are_unsupported() {
        let s = RefuseAll;
        assert!(matches!(s.void("e", "x"), Err(Error::Unsupported("void"))));
        assert!(matches!(s.remind("e", "r"), Err(Error::Unsupported("reminders"))));
        assert!(matches!(s.signing_url("e", "r", "https://example"), Err(Error::Unsupported("embedded signing"))));
        assert!(!s.supports(EsignCapability::Envelopes));
        assert_eq!(s.responsible_party(), "nobody");
    }

    #[test]
    fn draft_round_trips_json() {
        let draft = EnvelopeDraft {
            title: "NDA".into(),
            message: Some("please sign".into()),
            sender: Identity { id: "u1".into(), display: "Ada".into(), email: Some("ada@example.com".into()), avatar: None },
            document_id: Some("doc-1".into()),
            document_etag: Some("\"abc\"".into()),
            recipients: vec![Recipient {
                id: "r1".into(),
                name: "Bob".into(),
                email: "bob@example.com".into(),
                role: "Signer".into(),
                required: true,
                status: RecipientStatus::Pending,
            }],
            placements: vec![FieldPlacement {
                role: "Signer".into(),
                page: 0,
                x: 0.1,
                y: 0.2,
                w: 0.3,
                h: 0.05,
                kind: "signature".into(),
                required: true,
            }],
            expires: Some("2026-12-31".into()),
        };
        let json = serde_json::to_string(&draft).expect("serialize");
        let back: EnvelopeDraft = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, draft);
    }

    #[test]
    fn cloud_errors_wrap_and_report_transient() {
        let err = Error::from(craft_cloud::Error::Unreachable("down".into()));
        assert!(err.is_transient());
        let err = Error::Declined("no".into());
        assert!(!err.is_transient());
    }
}
