//! The data the contracts pass around.
//!
//! Every type here is plain and vendor-neutral. A field that only means
//! something to one product belongs in that provider's crate.

use serde::{Deserialize, Serialize};

/// What a provider can do.
///
/// Reported rather than assumed, so CollabCraft hides a share button on a
/// server without sharing instead of offering a feature that fails on use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Files,
    SharedWithMe,
    Sharing,
    Contacts,
    Notifications,
    Mail,
    /// Conditional writes, so a save can refuse to clobber a newer version.
    Etags,
    /// Public links. Absent on a server that forbids them, and the dialog then
    /// offers no Copy link rather than a button that fails.
    LinkShares,
    /// A key on a public link. This is what most places mean when they talk
    /// about sharing with somebody who has no account.
    LinkPassword,
    /// The server requires that key, so a link cannot be made without one.
    LinkPasswordRequired,
    /// An expiry date on a share.
    ShareExpiry,
    /// The provider can tell a recipient that something was shared with them.
    ///
    /// Distinct from [`Capability::Mail`], which is about sending arbitrary
    /// mail. A provider commonly has this and not that: it will notify the
    /// person a share names without offering any way to send mail in general.
    ShareNotify,
}

/// A file or folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// An opaque handle for reading, writing and sharing this entry.
    ///
    /// Only the provider that issued it may interpret it. A caller passes it
    /// back and never parses it.
    pub id: String,
    /// The stable identity used for a collaboration room.
    ///
    /// Separate from `id` on purpose. A provider's read and write handle is
    /// often a path, and a path changes when somebody renames or moves the
    /// file. A room keyed on that would silently split in two mid-session,
    /// with each half seeing only its own edits. This one survives a rename.
    pub room: String,
    /// What to show the user.
    pub name: String,
    /// Where it sits, for a breadcrumb. Never a handle.
    pub path: String,
    pub kind: EntryKind,
    /// Bytes, absent for a folder.
    pub size: Option<u64>,
    /// RFC 3339. Absent when the provider does not say.
    pub modified: Option<String>,
    /// For matching a document to the app that opens it.
    pub mime: Option<String>,
    /// The version the caller last saw, for a conditional write.
    pub etag: Option<String>,
    /// Whether this user may change it. A read-only share still lists.
    pub writable: bool,
    /// Who shared it, when it came from somebody else.
    pub shared_by: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Folder,
}

/// A person, as a share dialog needs them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    /// What the provider wants back when sharing with this person.
    pub id: String,
    /// How to share with them, decided by the provider and never guessed.
    ///
    /// The distinction is not cosmetic. A contact with an account is a user
    /// share. One known only by address is an email share. Sharing an address
    /// as though it were a user id fails with "not found", because no such
    /// user exists, and nothing in `id` alone says which case this is.
    pub target: ShareTarget,
    /// What to show. A display name where there is one.
    pub name: String,
    pub email: Option<String>,
    /// An avatar, as a URL the client may fetch. Never image bytes: a search
    /// returning ten people would otherwise carry ten pictures nobody asked
    /// for yet.
    pub avatar: Option<String>,
    /// A group rather than a person. Shown differently and shared differently.
    pub is_group: bool,
}

/// Who to share with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum ShareTarget {
    User(String),
    Group(String),
    /// Somebody with no account, reached by address.
    Email(String),
    /// Anybody holding the link.
    Link,
}

/// What a share allows.
///
/// Deliberately coarse. Providers disagree on the fine detail, and a dialog
/// offering a distinction the server cannot keep is worse than one that does
/// not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// Open and read. No edits reach the document.
    View,
    /// Open and edit.
    Edit,
}

/// Everything optional about a share.
///
/// A struct rather than more positional arguments. The list grew past the
/// point where a call site reads correctly, and a provider that cannot honour
/// one of these needs to see which it was given.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareOptions {
    /// Shown to the recipient.
    pub note: Option<String>,
    /// Send mail where the provider can.
    pub notify: bool,
    /// A key the recipient must present. Only meaningful for a link, and
    /// refused by a server that does not allow one.
    pub password: Option<String>,
    /// `YYYY-MM-DD`. After this the share stops working.
    pub expires: Option<String>,
}

/// A share that exists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Share {
    pub id: String,
    pub target: ShareTarget,
    pub permission: Permission,
    /// Present for a link share, absent otherwise.
    pub url: Option<String>,
    pub note: Option<String>,
    /// RFC 3339.
    pub created: Option<String>,
}

/// Something worth telling a user about, in the app.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    /// A short machine-readable kind, such as `document_shared`. Lets a client
    /// decide how to render without parsing prose.
    pub kind: String,
    pub subject: String,
    pub body: Option<String>,
    /// Where to go when it is clicked.
    pub link: Option<String>,
}

/// An email to send.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailMessage {
    pub to: Vec<String>,
    pub subject: String,
    pub body: String,
    /// Whether `body` is HTML. Plain text otherwise.
    pub html: bool,
}

/// The signed-in user, as the provider sees them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// The provider's identifier for this account.
    pub id: String,
    /// What to show. Never an identity: two people can share a display name.
    pub display: String,
    pub email: Option<String>,
    pub avatar: Option<String>,
}
