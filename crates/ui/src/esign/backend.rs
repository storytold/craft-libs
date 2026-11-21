//! Where the e-sign dialog's work happens, which is never on the frame.
//!
//! Same shape as [`crate::share::ShareBackend`]: the dialog states what it
//! wants, something behind this trait does the waiting, and answers arrive on
//! a channel keyed by sequence number.

use craft_cloud::Contact;
use craft_esign::{CompletedDocument, Envelope, EnvelopeDraft, EsignCapability};

/// One thing the dialog needs done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// What the signing service can actually do, and who is legally responsible.
    Capabilities,
    /// People matching what has been typed so far (cloud contacts).
    Lookup { query: String, limit: usize },
    /// Create a draft from a document snapshot, then optionally send it.
    Create { draft: Box<EnvelopeDraft>, document: Vec<u8>, idempotency_key: String, send: bool },
    /// Current status of one envelope.
    Status { envelope: String },
    /// Finished document bytes.
    Completed { envelope: String },
    /// Cancel an in-flight envelope.
    Void { envelope: String, reason: String },
    /// Nudge one recipient.
    Remind { envelope: String, recipient: String },
}

/// What came back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Capabilities {
        capabilities: Vec<EsignCapability>,
        responsible_party: String,
        /// Whether a cloud account is attached so invitations can leave.
        linked: bool,
    },
    Lookup(Vec<Contact>),
    Created(Envelope),
    Status(Envelope),
    Completed(CompletedDocument),
    Voided {
        envelope: String,
    },
    Reminded {
        envelope: String,
        recipient: String,
    },
}

/// Where one answer goes.
pub struct Replier {
    seq: u64,
    tx: std::sync::mpsc::Sender<(u64, Result<Answer, String>)>,
}

impl Replier {
    pub(crate) fn new(seq: u64, tx: std::sync::mpsc::Sender<(u64, Result<Answer, String>)>) -> Replier {
        Replier { seq, tx }
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn dup(&self) -> Replier {
        Replier { seq: self.seq, tx: self.tx.clone() }
    }

    pub fn answer(self, answer: Answer) {
        self.deliver(Ok(answer));
    }

    pub fn refuse(self, reason: impl Into<String>) {
        self.deliver(Err(reason.into()));
    }

    fn deliver(self, result: Result<Answer, String>) {
        let _ = self.tx.send((self.seq, result));
    }
}

impl std::fmt::Debug for Replier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Replier").field("seq", &self.seq).finish()
    }
}

/// Somewhere a [`Request`] can run without blocking a frame.
pub trait EsignBackend {
    fn submit(&self, request: Request, reply: Replier);
}
