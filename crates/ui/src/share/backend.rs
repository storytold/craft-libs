//! Where the share dialog's work happens, which is never on the frame.
//!
//! Every call a share needs is blocking, and a blocking call inside `update()`
//! freezes the window for as long as the server takes. So the dialog states
//! what it wants and reads answers off a channel, and something behind this
//! trait does the waiting.
//!
//! The dialog itself opens no socket, which keeps it buildable for the web and
//! testable with no server. The only implementation that does open one is
//! [`CollabCraft`], and it reaches the CollabCraft service rather than a
//! provider: no app holds a provider credential and no app speaks a provider
//! protocol.

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

#[cfg(all(feature = "collabcraft", not(target_arch = "wasm32")))]
pub use collabcraft::CollabCraft;

#[cfg(all(feature = "collabcraft", not(target_arch = "wasm32")))]
mod collabcraft {
    use std::sync::Arc;

    use craft_cloud_client::{Client, ShareRequest};

    use super::{Answer, Destination, Failure, Replier, Request, ShareBackend};

    /// Where each request's client comes from.
    #[derive(Clone)]
    enum Source {
        /// One client for the life of the dialog.
        Fixed(Client),
        /// A client built for each request.
        ///
        /// A desktop application collects its session through a browser,
        /// possibly long after the window opened, and keeps it somewhere only
        /// the application knows. Asking for a client per request means a share
        /// opened after that sign-in finds the session without a restart.
        Made(Arc<dyn Fn() -> Option<Client> + Send + Sync>),
    }

    /// A [`ShareBackend`] that reaches the CollabCraft service.
    ///
    /// This is the only part of the dialog that opens a socket, and it opens it
    /// to CollabCraft. Which provider sits behind that, and what credential
    /// reaches it, are the service's business and are configured in one place.
    #[derive(Clone)]
    pub struct CollabCraft {
        source: Source,
    }

    impl CollabCraft {
        /// A backend on one client, for a host that already holds a session.
        pub fn new(client: Client) -> CollabCraft {
            CollabCraft { source: Source::Fixed(client) }
        }

        /// A backend that asks the host for a client each time, so a session
        /// collected later is picked up without a restart.
        ///
        /// `None` from the factory means there is no service configured, which
        /// leaves the dialog unavailable rather than broken.
        pub fn from_fn(make: impl Fn() -> Option<Client> + Send + Sync + 'static) -> CollabCraft {
            CollabCraft { source: Source::Made(Arc::new(make)) }
        }

        /// A backend from the environment: `CRAFT_CLOUD_URL`, and
        /// `CRAFT_CLOUD_TOKEN` when a session has already been collected.
        ///
        /// Neither is compiled in, which is what lets one build talk to any
        /// deployment. `None` means there is no service to talk to.
        pub fn from_env() -> Option<CollabCraft> {
            let mut client = Client::new(var("CRAFT_CLOUD_URL")?);
            if let Some(token) = var("CRAFT_CLOUD_TOKEN") {
                client = client.with_token(token);
            }
            Some(CollabCraft::new(client))
        }
    }

    fn var(key: &str) -> Option<String> {
        std::env::var(key).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
    }

    fn client(source: &Source) -> Option<Client> {
        match source {
            Source::Fixed(client) => Some(client.clone()),
            Source::Made(make) => make(),
        }
    }

    impl ShareBackend for CollabCraft {
        fn submit(&self, request: Request, reply: Replier) {
            let source = self.source.clone();
            // One thread per request. The dialog debounces its autocomplete,
            // so this is a handful at a time, and a lookup that is still
            // waiting cannot hold up a share the user has already pressed.
            // The client is built on this thread too: reading a saved session
            // off disk is not work for a frame.
            let spare = reply.dup();
            let started = std::thread::Builder::new().name("craft-share".to_owned()).spawn(move || match client(&source) {
                None => reply.refuse("no cloud service is configured"),
                Some(client) => match run(&client, request) {
                    Ok(answer) => reply.answer(answer),
                    Err(reason) => reply.refuse(reason),
                },
            });
            if let Err(e) = started {
                // The moved handle went with the closure that never ran, so the
                // spare is the only way to stop the dialog waiting forever.
                spare.refuse(format!("no thread to run this on: {e}"));
            }
        }
    }

    fn run(client: &Client, request: Request) -> Result<Answer, String> {
        match request {
            Request::Capabilities => {
                let me = client.me().map_err(say)?;
                Ok(Answer::Capabilities { capabilities: me.capabilities, linked: me.provider_linked })
            }
            Request::Lookup { query, limit } => client.contacts(&query, limit).map(Answer::Lookup).map_err(say),
            Request::Access { id } => client.shares(&id).map(Answer::Access).map_err(say),
            Request::Grant(grant) => {
                let request = ShareRequest {
                    id: grant.id,
                    targets: grant.targets,
                    permission: grant.permission,
                    note: grant.note,
                    notify: grant.notify,
                    password: grant.password,
                    expires: None,
                };
                let outcome = client.share(&request).map_err(say)?;
                let failed = outcome.failed.into_iter().map(|f| Failure { target: f.target, code: f.code, detail: f.detail }).collect();
                Ok(Answer::Granted { shared: outcome.shared, failed })
            }
            Request::Withdraw { share_id } => {
                client.unshare(&share_id).map_err(say)?;
                Ok(Answer::Withdrawn { share_id })
            }
            // A contact's avatar URL is the provider's own, and an app reaching
            // it would be an app talking to the provider, which is the thing
            // this whole arrangement exists to prevent. Until CollabCraft
            // relays pictures, the dialog paints initials. A host that has
            // decided it may fetch them wraps this backend and answers this
            // one request itself; nothing else changes.
            Request::Avatar { .. } => Err("this build does not fetch contact pictures".to_owned()),

            // One destination today, described by the account behind it.
            // Everything above the contracts treats this as a list, so a
            // second provider is an entry rather than a change.
            Request::Destinations => {
                let me = client.me().map_err(|e| e.to_string())?;
                Ok(Answer::Destinations(vec![Destination {
                    id: "cloud".to_owned(),
                    name: "Cloud".to_owned(),
                    account: me.email.clone().or(Some(me.display.clone())),
                    linked: me.provider_linked,
                }]))
            }

            Request::Link { .. } => {
                let start = client.link_start().map_err(|e| e.to_string())?;
                // Opening a browser is the host application's business, so the
                // URL is reported rather than launched from here.
                Err(format!("open {} to connect an account", start.login_url))
            }
        }
    }

    /// A client error in words a user can read.
    fn say(e: craft_cloud_client::Error) -> String {
        e.to_string()
    }
}
