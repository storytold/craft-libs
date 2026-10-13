//! What a cloud provider has to offer the Craft apps, and nothing about who
//! provides it.
//!
//! Every capability here is an RFC or an ordinary HTTP API that many systems
//! implement. Files are WebDAV. Contacts are CardDAV. Calendars are CalDAV.
//! Sharing and notification are the only genuinely vendor-shaped parts, and
//! they are small. So there is no reason for an app to know which server it is
//! talking to, and no reason to write the same logic again for the next one.
//!
//! Nothing in this crate opens a socket. Each trait is implemented by a
//! provider crate that brings its own transport, which keeps this testable
//! with no server and keeps the contracts honest about what they need.
//!
//! No type here names a vendor. If a field only makes sense for one product,
//! it belongs in that provider's crate.
//!
//! Nothing here says where an implementation runs. A local adapter may
//! implement these traits against a provider directly, and an application may
//! consume them that way. The contracts do not care.
//!
//! CollabCraft chooses otherwise for its own apps. They reach one service, and
//! that service is the single place a provider is configured, a credential is
//! held and a permission is decided. A desktop binary carrying a provider
//! credential leaks one the moment somebody copies the file, which is what
//! that arrangement exists to prevent. It is a deployment policy, and this
//! crate imposes it on nobody.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod error;
mod types;

pub use error::{Error, Result};
pub use types::{Capability, Contact, Entry, EntryKind, Identity, MailMessage, Notification, Permission, Share, ShareOptions, ShareTarget};

/// Reading and writing documents.
///
/// `list` and `shared_with_me` are separate because they answer different
/// questions. One is a folder's contents, the other is everything another
/// person has given this user access to, which has no single parent folder.
pub trait Files {
    /// One folder's contents. An empty path means the user's root.
    fn list(&self, path: &str) -> Result<Vec<Entry>>;

    /// Everything shared with this user, wherever it lives.
    ///
    /// A provider with no concept of this returns an empty list rather than an
    /// error, because "nobody has shared anything with you" and "this server
    /// cannot tell you" look the same to a file picker.
    fn shared_with_me(&self) -> Result<Vec<Entry>>;

    /// One document's bytes.
    fn read(&self, id: &str) -> Result<Vec<u8>>;

    /// Replace one document's bytes.
    ///
    /// `if_match` carries the etag the caller last saw. A provider that
    /// supports conditional writes uses it to refuse a save that would
    /// overwrite somebody else's, returning [`Error::Conflict`]. Passing
    /// `None` means "write regardless", which is what a Save As does.
    fn write(&self, id: &str, bytes: &[u8], if_match: Option<&str>) -> Result<Entry>;

    /// Create a document and return it.
    fn create(&self, folder: &str, name: &str, bytes: &[u8]) -> Result<Entry>;

    /// One document's metadata, without its bytes.
    fn stat(&self, id: &str) -> Result<Entry>;
}

/// Giving other people access to a document.
///
/// Persisting a share through the provider matters more than it looks. A share
/// recorded only by CollabCraft would be invisible in the provider's own file
/// manager, so the two would disagree about who can open what. The provider
/// stays the authority.
pub trait Sharing {
    /// Share a document.
    ///
    /// A link share has no recipient, so [`ShareTarget::Link`] is how a caller
    /// asks for one. The returned [`Share::url`] is what to hand out.
    fn share(&self, id: &str, with: &ShareTarget, permission: Permission, options: &ShareOptions) -> Result<Share>;

    /// Who currently has access.
    fn shares(&self, id: &str) -> Result<Vec<Share>>;

    /// Withdraw one share.
    fn unshare(&self, share_id: &str) -> Result<()>;
}

/// Looking people up, for a share dialog's autocomplete.
///
/// This is the only thing the share dialog needs, so it is the only thing the
/// contract exposes. A full address book API would be a bigger surface for no
/// gain.
pub trait Contacts {
    /// People matching a partial name or address.
    ///
    /// `limit` exists because this runs on every keystroke. A provider is free
    /// to return fewer.
    fn search(&self, query: &str, limit: usize) -> Result<Vec<Contact>>;
}

/// Telling somebody something happened.
///
/// Separate from [`Mail`] on purpose. An in-app notification is cheap, arrives
/// instantly and is read in context. An email is none of those and is the
/// right choice for somebody who is not currently in the app. A share usually
/// wants both.
pub trait Notifications {
    fn notify(&self, user: &str, notification: &Notification) -> Result<()>;
}

/// Sending mail as the provider.
pub trait Mail {
    fn send(&self, message: &MailMessage) -> Result<()>;
}

/// Who the caller is, according to the provider.
pub trait Accounts {
    /// Resolve the signed-in user.
    fn me(&self) -> Result<Identity>;
}

/// Everything one provider can do.
///
/// A provider implements the capabilities it has and reports the rest as
/// absent, so CollabCraft can hide a share button on a server with no sharing
/// rather than offering a feature that will fail.
pub trait Provider: Files + Sharing + Contacts + Notifications + Mail + Accounts {
    /// A stable identifier, such as `nextcloud`. Used in configuration and
    /// logs, never shown to a user.
    fn id(&self) -> &str;

    /// What this provider actually supports.
    fn capabilities(&self) -> Vec<Capability>;

    fn supports(&self, capability: Capability) -> bool {
        self.capabilities().contains(&capability)
    }
}
