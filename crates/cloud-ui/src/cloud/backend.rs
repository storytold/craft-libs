//! Where Open and Save do their waiting. It is never on the frame.
//!
//! Every call a cloud provider needs is blocking. A blocking read of a twenty
//! megabyte document inside `update()` freezes the window for seconds. So this
//! module states a [`Job`] and reads an [`Event`]. Something behind
//! [`CloudBackend`] does the waiting.
//!
//! Nothing here opens a socket. That keeps the pages buildable for the web and
//! testable with no service running. A host supplies the half that does. This
//! crate never learns what it reaches.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;

use craft_cloud::{Capability, Conversation, Entry};

/// Work for a host's worker. Each one answers with one or more [`Event`].
pub enum Job {
    /// Who is signed in, and what the provider behind the service can do.
    Account,
    /// Begin signing in to the service through the browser or device flow.
    SignIn,
    /// Begin linking a cloud account. The answer carries a URL to open.
    LinkStart,
    /// Whether the user has finished in the browser yet.
    LinkPoll(String),
    /// One folder's contents. An empty path is the user's root.
    List(String),
    /// Everything other people have shared with this user.
    SharedWithMe,
    /// A document's bytes. `into` names the open document a reload replaces.
    Read { entry: Entry, into: Option<u64> },
    /// Save back over `entry`, refusing to overwrite a newer version.
    Write { uid: u64, entry: Entry, bytes: Vec<u8> },
    /// Save under a new name in `folder`.
    Create { uid: u64, folder: String, name: String, bytes: Vec<u8> },
    /// End the session and forget the saved token.
    SignOut,
    /// Where to go to work together on a document.
    Conversation(String),
}

/// What a worker reports back.
pub enum Event {
    /// Where to go to work together, or why there is nowhere.
    Conversation(Result<Conversation, Fault>),
    Account(Result<Account, Fault>),
    /// A link or a sign-in needs the user's browser. `handle` is polled until
    /// they are done, when the service gave one.
    Browser {
        url: String,
        handle: Option<String>,
    },
    /// `Ok(true)` means the account is linked now.
    Linked(Result<bool, Fault>),
    Listed {
        path: String,
        entries: Result<Vec<Entry>, Fault>,
    },
    Shared(Result<Vec<Entry>, Fault>),
    Read {
        entry: Entry,
        into: Option<u64>,
        bytes: Result<Vec<u8>, Fault>,
    },
    Saved {
        uid: u64,
        entry: Result<Entry, Fault>,
    },
}

/// Where a worker's answers go.
///
/// Handed to [`CloudBackend::submit`] with the job it belongs to. A worker is
/// free to keep it and send later, or to send several times. That is how a
/// sign-in reports the URL it wants a browser pointed at and then reports that
/// the user finished there. A send after the window closed is not a failure.
pub struct Post {
    tx: Sender<Event>,
    /// The window's account generation as of the moment the job this answers
    /// was issued, beside a handle to what it is now. A sign-out or a new
    /// sign-in bumps the live value; a reply that arrives after is for a
    /// session that is gone, and must not land on the one that replaced it.
    generation: Arc<AtomicU64>,
    issued: u64,
}

impl Post {
    pub(crate) fn new(tx: Sender<Event>, generation: Arc<AtomicU64>) -> Post {
        let issued = generation.load(Ordering::Relaxed);
        Post { tx, generation, issued }
    }

    /// Another handle to the same place, for a worker that answers more than
    /// once or hands the job on. Carries the same generation: every answer to
    /// one job was issued at the same moment, however many of them there are.
    pub fn dup(&self) -> Post {
        Post { tx: self.tx.clone(), generation: self.generation.clone(), issued: self.issued }
    }

    /// Delivers the event, unless the window has signed out or signed in
    /// again since the job this answers was issued. Silently dropping it
    /// there, rather than delivering it, is what keeps a late reply from
    /// restoring the previous session's account, files or busy state on top
    /// of whatever replaced it.
    pub fn send(&self, event: Event) {
        if self.generation.load(Ordering::Relaxed) != self.issued {
            return;
        }
        let _ = self.tx.send(event);
    }
}

impl std::fmt::Debug for Post {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Post").finish()
    }
}

/// Somewhere a [`Job`] can run without blocking a frame.
///
/// `submit` must return immediately. Answers arrive through `post` whenever
/// they are ready, or never. The pages survive either.
pub trait CloudBackend {
    fn submit(&self, job: Job, post: Post);
}

/// What the service said about the signed-in user.
///
/// Capabilities are reported by the service and never assumed here. A provider
/// without incoming shares hides that side of the Open page. An empty list
/// where a feature should be reads to a user as a broken window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Account {
    pub display: String,
    pub capabilities: Vec<Capability>,
    /// Until this is true, Open and Save have nowhere to go.
    pub provider_linked: bool,
}

impl Account {
    pub fn can(&self, capability: Capability) -> bool {
        self.capabilities.contains(&capability)
    }
}

/// A call that did not succeed, in the shapes the pages act on differently.
///
/// One case is worth telling apart from every other refusal. A conditional
/// write the server refused means somebody else's work is in the stored copy.
/// That one must never be retried and must never be forced. That is why
/// [`Fault::Conflict`] has a shape of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    /// No cloud account is linked yet. This is a state with an obvious next
    /// step. It is never drawn as an error.
    NotLinked,
    /// Somebody else saved while this user was editing. The save was refused
    /// and their work is intact.
    Conflict,
    /// The session expired. The user signs in again.
    Unauthenticated,
    Unreachable(String),
    Refused(String),
}

impl Fault {
    /// One sentence for a person to read. `noun` is what the application calls
    /// one of its documents, from [`crate::cloud::DocumentKind::noun`].
    pub fn text(&self, noun: &str) -> String {
        match self {
            Fault::NotLinked => "No cloud account is linked yet.".to_owned(),
            Fault::Conflict => format!("Somebody else saved this {noun} while you were editing it."),
            Fault::Unauthenticated => "Your cloud session has expired. Sign in again.".to_owned(),
            Fault::Unreachable(detail) => format!("The cloud service could not be reached. {detail}"),
            Fault::Refused(detail) => detail.clone(),
        }
    }

    /// A short machine-readable name, for a control channel.
    pub fn code(&self) -> &'static str {
        match self {
            Fault::NotLinked => "not_linked",
            Fault::Conflict => "conflict",
            Fault::Unauthenticated => "unauthenticated",
            Fault::Unreachable(_) => "unreachable",
            Fault::Refused(_) => "refused",
        }
    }
}

/// The contract's own error, in the shapes the pages act on.
///
/// For a host that reaches a provider through [`craft_cloud`] directly. A host
/// that reaches a service through its own client crate maps that crate's error
/// instead, since only it knows which code that service answers a refused
/// conditional write with.
///
/// [`craft_cloud::Error::Conflict`] is the one case worth telling apart. It
/// means somebody else's work is in the stored copy. The save must never be
/// retried and never be forced.
impl From<craft_cloud::Error> for Fault {
    fn from(error: craft_cloud::Error) -> Fault {
        use craft_cloud::Error;
        match error {
            Error::Unreachable(detail) => Fault::Unreachable(detail),
            Error::Unauthenticated => Fault::Unauthenticated,
            Error::Conflict(_) => Fault::Conflict,
            other => Fault::Refused(other.to_string()),
        }
    }
}
