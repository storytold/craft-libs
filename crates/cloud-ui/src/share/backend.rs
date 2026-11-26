//! Where the share dialog's work happens, which is never on the frame.
//!
//! Every call a share needs is blocking, and a blocking call inside `update()`
//! freezes the window for as long as the server takes. So the dialog states
//! what it wants and reads answers off a channel, and something behind this
//! trait does the waiting.
//!
//! The dialog itself opens no socket, which keeps it buildable for the web
//! and testable with no server. A host supplies the half that does, and this
//! crate never learns what it reaches.

use craft_cloud::{Capability, Contact, Permission, Share, ShareTarget};

/// One thing the dialog needs done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// What the provider behind the service can actually do.
    Capabilities,
    /// People matching what has been typed so far.
    Lookup { query: String, limit: usize },
    /// Who already has access to one document.
    Access { id: String },
    /// Give several people access at once.
    Grant(Grant),
    /// Take one existing share away.
    Withdraw { share_id: String },
    /// Where this document could be saved, so it can be shared.
    ///
    /// Asked for only when there is nothing to share yet. A list rather than a
    /// single answer, because more than one provider may be configured and the
    /// user picks.
    Destinations,
    /// Link an account for a destination that is configured but not connected.
    Link { destination: String },
    /// One contact's picture, by the URL that contact carried.
    ///
    /// A separate request because a search answering with ten people must not
    /// cost ten pictures before anybody has looked at them. The dialog asks
    /// for a picture when it first paints the row that needs it, once per URL.
    Avatar { url: String },
}

/// A share to create, as the dialog has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    /// A key the recipient must present. Only meaningful for a link share, and
    /// refused by a server that does not allow one.
    pub password: Option<String>,
    /// The document, as an opaque handle the app was given.
    pub id: String,
    pub targets: Vec<ShareTarget>,
    pub permission: Permission,
    pub note: Option<String>,
    pub notify: bool,
}

/// What came back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Capabilities {
        capabilities: Vec<Capability>,
        /// Whether this user has a cloud account attached at all.
        linked: bool,
    },
    Lookup(Vec<Contact>),
    Access(Vec<Share>),
    /// Partial success is the ordinary case, so both halves come back.
    Granted {
        shared: Vec<Share>,
        failed: Vec<Failure>,
    },
    Withdrawn {
        share_id: String,
    },
    Destinations(Vec<Destination>),
    /// An account was linked, so the destinations are worth asking for again.
    Linked,
    Avatar {
        url: String,
        /// Encoded image bytes, in whatever format the server sent. Decoding
        /// is egui's image loaders' job, which the application installs.
        bytes: Vec<u8>,
    },
}

/// Somewhere a document could be saved.
///
/// `account` is what makes this choosable. Two entries reading "Cloud" tell
/// nobody which of their accounts a document is about to land in, which is the
/// one thing somebody needs to know before pressing it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Destination {
    /// Opaque to the dialog, handed back with the save.
    pub id: String,
    /// What to show, such as the service's own name.
    pub name: String,
    /// Which account, when one is linked.
    pub account: Option<String>,
    /// False when the destination is configured but has no account yet. The
    /// dialog offers a sign-in rather than a save.
    pub linked: bool,
}

/// One recipient a share did not reach.
///
/// Named rather than counted. Sharing with five people where one address is
/// wrong shares with four and says which one failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub target: ShareTarget,
    pub code: String,
    pub detail: String,
}

/// Where one answer goes.
///
/// Carries the sequence number of the request it answers. Autocomplete fires
/// per keystroke, and a service is free to answer the third query before the
/// second, so the dialog keeps the newest sequence it asked for and drops
/// anything older. Without that the list flickers between a stale set of
/// matches and a current one.
pub struct Replier {
    seq: u64,
    tx: std::sync::mpsc::Sender<(u64, Result<Answer, String>)>,
}

impl Replier {
    pub(crate) fn new(seq: u64, tx: std::sync::mpsc::Sender<(u64, Result<Answer, String>)>) -> Replier {
        Replier { seq, tx }
    }

    /// Which request this answers.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Another handle to the same answer slot, for a backend that has to give
    /// up after moving the first one into a worker.
    pub fn dup(&self) -> Replier {
        Replier { seq: self.seq, tx: self.tx.clone() }
    }

    pub fn answer(self, answer: Answer) {
        self.deliver(Ok(answer));
    }

    /// Say why it could not be done, in words a user can read.
    pub fn refuse(self, reason: impl Into<String>) {
        self.deliver(Err(reason.into()));
    }

    /// A dropped receiver means the dialog closed, which is not a failure.
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
///
/// `submit` must return immediately. The answer arrives through `reply`
/// whenever it is ready, or never, which the dialog survives.
pub trait ShareBackend {
    fn submit(&self, request: Request, reply: Replier);
}
