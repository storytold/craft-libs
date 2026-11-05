//! The data the e-sign contracts pass around.
//!
//! Every type here is plain and vendor-neutral. A field that only means
//! something to one product belongs in that provider's crate. Nothing here
//! knows about PDF structure; a document is opaque bytes plus a content hash.

use craft_cloud::Identity;
use serde::{Deserialize, Serialize};

/// What a signing service can do.
///
/// Reported rather than assumed, so a caller hides void or remind on a service
/// that only supports create/send/status/completed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EsignCapability {
    /// Create, send, poll status, and fetch the completed document.
    Envelopes,
    /// Cancel an in-flight envelope.
    Void,
    /// Nudge a recipient who has not signed yet.
    Reminders,
    /// Return a URL the recipient opens to sign in the provider's UI.
    EmbeddedSigning,
    /// Accept provider webhooks and turn them into [`EnvelopeEvent`]s.
    Webhooks,
}

/// Where an envelope is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvelopeStatus {
    /// Created but not yet sent to recipients.
    Draft,
    /// At least one recipient still owes a signature.
    Sent,
    /// Every required recipient has signed.
    Completed,
    /// The sender cancelled it.
    Voided,
    /// A recipient declined.
    Declined,
    /// The deadline passed without completion.
    Expired,
}

/// What one recipient still owes, or has done.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecipientStatus {
    Pending,
    Viewed,
    Signed,
    Declined,
}

/// Who must sign, as the dialog and the service agree.
///
/// Built from a [`craft_cloud::Contact`] by an explicit conversion at the host
/// boundary. Never a [`craft_cloud::ShareTarget`]: signing is not access.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recipient {
    /// Opaque handle the service wants back.
    pub id: String,
    /// What to show. A display name where there is one.
    pub name: String,
    /// Delivery address. Required for invitation mail.
    pub email: String,
    /// Signer role label, such as "Client" or "Countersign". Colours and
    /// field placement key off this string within one envelope.
    pub role: String,
    /// Whether the envelope cannot complete without this person.
    pub required: bool,
    pub status: RecipientStatus,
}

/// One signature (or initials / date) placement on the document.
///
/// Coordinates are normalised page space: origin top-left, `x`/`y`/`w`/`h` in
/// `[0.0, 1.0]` of the page. The document implementation maps these onto its
/// own field model; this crate never sees PDF objects.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FieldPlacement {
    /// Recipient role this field belongs to.
    pub role: String,
    /// Zero-based page index.
    pub page: u32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Machine-readable kind: `signature`, `initials`, `date`, `text`.
    pub kind: String,
    pub required: bool,
}

impl PartialEq for FieldPlacement {
    fn eq(&self, other: &Self) -> bool {
        self.role == other.role
            && self.page == other.page
            && self.kind == other.kind
            && self.required == other.required
            && self.x.to_bits() == other.x.to_bits()
            && self.y.to_bits() == other.y.to_bits()
            && self.w.to_bits() == other.w.to_bits()
            && self.h.to_bits() == other.h.to_bits()
    }
}

impl Eq for FieldPlacement {}

/// What the caller wants to send, before the service assigns an id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvelopeDraft {
    /// Human title for the ceremony.
    pub title: String,
    /// Optional note shown to recipients.
    pub message: Option<String>,
    /// Who is asking for signatures. Bound to the active account at send time.
    pub sender: Identity,
    /// Cloud document identity at the moment of the snapshot, when known.
    pub document_id: Option<String>,
    pub document_etag: Option<String>,
    pub recipients: Vec<Recipient>,
    pub placements: Vec<FieldPlacement>,
    /// `YYYY-MM-DD`. After this the envelope expires if still open.
    pub expires: Option<String>,
}

/// An envelope the service knows about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub id: String,
    pub title: String,
    pub message: Option<String>,
    pub sender: Identity,
    pub document_id: Option<String>,
    pub document_etag: Option<String>,
    /// SHA-256 of the document bytes at create time, lowercase hex.
    pub content_sha256: String,
    pub recipients: Vec<Recipient>,
    pub placements: Vec<FieldPlacement>,
    pub status: EnvelopeStatus,
    pub expires: Option<String>,
    /// RFC 3339.
    pub created: Option<String>,
    /// RFC 3339 when status became [`EnvelopeStatus::Completed`].
    pub completed_at: Option<String>,
}

/// The finished document plus whatever evidence the provider returns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletedDocument {
    pub envelope_id: String,
    /// Signed document bytes. Opaque to this crate.
    pub bytes: Vec<u8>,
    /// Provider evidence (certificate of completion, audit export, …).
    pub evidence: Vec<u8>,
}

/// Something that happened to an envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvelopeEvent {
    pub envelope_id: String,
    /// Machine-readable kind: `sent`, `viewed`, `signed`, `declined`, `voided`, `completed`.
    pub kind: String,
    /// Recipient id when the event is about one person.
    pub recipient_id: Option<String>,
    /// RFC 3339.
    pub at: Option<String>,
    pub detail: Option<String>,
}
