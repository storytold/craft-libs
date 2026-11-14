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
    /// Fails unless status is [`EnvelopeStatus::Completed`]. The returned
    /// [`CompletedDocument`] carries both the original document's content
    /// digest and the signed artifact's digest, computed honestly rather
    /// than copied from one another: the implementation must not assume the
    /// provider's sealing left the bytes unchanged.
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
    use std::sync::Mutex;

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

    /// A non-cryptographic stand-in for a content digest. Good enough to
    /// prove two byte strings differ; this crate takes no hashing
    /// dependency, since hashing the document is an adapter's job.
    fn fake_digest(bytes: &[u8]) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    /// An honest in-process double for [`Envelopes`]. It keeps the exact
    /// bytes `create` was given, and on `completed` returns a "signed"
    /// artifact built by appending a stub provider seal to those bytes, so
    /// `original_content_sha256` and `signed_content_sha256` are computed
    /// from genuinely different byte strings, the way a real provider's
    /// sealing would produce them, rather than one copied from the other.
    struct Double {
        document: Mutex<Option<Vec<u8>>>,
    }

    impl Double {
        fn new() -> Double {
            Double { document: Mutex::new(None) }
        }
    }

    impl Envelopes for Double {
        fn create(&self, draft: &EnvelopeDraft, document: &[u8], _idempotency_key: &str) -> Result<Envelope> {
            *self.document.lock().expect("lock") = Some(document.to_vec());
            Ok(Envelope {
                id: "env-double".into(),
                title: draft.title.clone(),
                message: draft.message.clone(),
                sender: draft.sender.clone(),
                document_id: draft.document_id.clone(),
                document_etag: draft.document_etag.clone(),
                content_sha256: fake_digest(document),
                recipients: draft.recipients.clone(),
                placements: draft.placements.clone(),
                status: EnvelopeStatus::Draft,
                expires: draft.expires.clone(),
                created: None,
                completed_at: None,
            })
        }

        fn send(&self, envelope: &str) -> Result<Envelope> {
            self.status(envelope)
        }

        fn status(&self, envelope: &str) -> Result<Envelope> {
            let document = self.document.lock().expect("lock").clone().unwrap_or_default();
            Ok(Envelope {
                id: envelope.into(),
                title: "Double".into(),
                message: None,
                sender: Identity { id: "u1".into(), display: "Ada".into(), email: None, avatar: None },
                document_id: None,
                document_etag: None,
                content_sha256: fake_digest(&document),
                recipients: Vec::new(),
                placements: Vec::new(),
                status: EnvelopeStatus::Completed,
                expires: None,
                created: None,
                completed_at: None,
            })
        }

        fn completed(&self, envelope: &str) -> Result<CompletedDocument> {
            let original = self.document.lock().expect("lock").clone().unwrap_or_default();
            let mut signed = original.clone();
            signed.extend_from_slice(b"\0provider-seal");
            Ok(CompletedDocument {
                envelope_id: envelope.into(),
                bytes: signed.clone(),
                original_content_sha256: fake_digest(&original),
                signed_content_sha256: fake_digest(&signed),
                evidence: b"certificate-of-completion".to_vec(),
            })
        }
    }

    /// RFC-0021's evidence guarantee: `CompletedDocument` carries the
    /// original document's digest and the signed artifact's digest as two
    /// distinct, honestly computed fields, and they differ when the
    /// provider's sealing changes the bytes.
    #[test]
    fn completed_document_keeps_original_and_signed_digests_distinct() {
        let double = Double::new();
        let draft = EnvelopeDraft {
            title: "Contract".into(),
            message: None,
            sender: Identity { id: "u1".into(), display: "Ada".into(), email: None, avatar: None },
            document_id: Some("doc-1".into()),
            document_etag: None,
            recipients: Vec::new(),
            placements: Vec::new(),
            expires: None,
        };
        let original_bytes = b"the original document bytes";
        let envelope = double.create(&draft, original_bytes, "idem-1").expect("create");
        let completed = double.completed(&envelope.id).expect("completed");

        assert_eq!(completed.original_content_sha256, fake_digest(original_bytes));
        assert_ne!(completed.bytes, original_bytes, "the signed artifact's bytes differ from the original");
        assert_eq!(completed.signed_content_sha256, fake_digest(&completed.bytes));
        assert_ne!(
            completed.original_content_sha256, completed.signed_content_sha256,
            "sealing changed the bytes, so the two digests must not be conflated"
        );

        let json = serde_json::to_string(&completed).expect("serialize");
        let back: CompletedDocument = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, completed);
    }
}
