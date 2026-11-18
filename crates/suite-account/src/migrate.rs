//! Taking over a per-application session file.
//!
//! An application that signed in before the suite store existed wrote its own session beside its
//! own settings, naming the file however it liked. GridCraft's was
//! `%APPDATA%\GridCraft\legacy-session.json`. Upgrading must not sign anybody out. The application
//! therefore hands that session to the suite store once and deletes the file. The token then lives
//! in the keychain and the whole suite can use it.
//!
//! The application does this, never the launcher. The file belongs to the application. Nothing
//! should reach into another application's folder to delete a file.
//!
//! The file is only deleted once the suite store holds the session. Where the suite store can keep
//! nothing (no keychain on this machine, or this process is isolated) the file stays exactly where
//! it is. Removing it would sign the user out. That is the one thing migration must not do.
//!
//! The earliest of these files held one key: `{"token": "..."}`. It names no deployment and no
//! subject. There is nothing to key an account on. Such a file is left exactly where it is too. A
//! made-up handle would file the token under something nothing ever looks up again. That reads as
//! a silent sign-out. An application that rewrites the file with the deployment and the subject is
//! adopted on the next startup. Adopting one launch late costs nothing.

use std::path::Path;

use serde::Deserialize;
use zeroize::Zeroizing;

use crate::accounts::{self, Account};
use crate::error::Error;
use crate::secret::Secret;
use crate::store::TokenStore;
use crate::{NewAccount, Store};

/// What a legacy session file said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Legacy {
    /// The deployment's base URL, with any trailing slash trimmed. Empty where the file predates
    /// the application recording it.
    pub service: String,
    /// The deployment's own identifier for the account: the OIDC subject. Empty where the file
    /// predates the application recording it.
    pub user: String,
    pub display: String,
    pub email: Option<String>,
    pub token: Secret,
}

impl Legacy {
    /// What the file does not say yet, when it cannot be keyed to an account.
    ///
    /// The account handle is a digest of the deployment and the subject. Both are therefore
    /// needed. A session written before the application recorded them carries a token alone.
    pub fn incomplete(&self) -> Option<&'static str> {
        if self.service.is_empty() {
            return Some("which deployment it belongs to");
        }
        if self.user.is_empty() {
            return Some("whose account it is");
        }
        None
    }
}

/// What happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Adopted {
    /// There was no file. There is nothing to take over.
    Nothing,
    /// The session is in the suite store now. The file is gone.
    Taken(Account),
    /// The file was left exactly where it is. The application goes on reading it. `why` says
    /// which case it was: the suite store can keep nothing here, or the file does not yet say
    /// enough to key an account on.
    Kept { why: String },
}

/// The shape on disk.
///
/// What an application writes today is `token`, `service`, `subject` and `display`. The file
/// predates this crate. The aliases therefore cover the other plausible spellings.
///
/// The token is the only key that has to be there: a file without one carries no session at all.
/// Everything else defaults. That is what lets the earliest `{"token": "..."}` file still parse.
/// [`adopt`] then leaves such a file alone. Keying an account on nothing is the alternative.
#[derive(Deserialize)]
struct Stored {
    #[serde(alias = "session", alias = "session_token", alias = "bearer", alias = "access_token")]
    token: String,
    #[serde(default, alias = "base", alias = "base_url", alias = "url", alias = "service_url")]
    service: String,
    #[serde(default, alias = "id", alias = "sub", alias = "subject", alias = "user_id")]
    user: String,
    #[serde(default, alias = "name", alias = "display_name")]
    display: String,
    #[serde(default)]
    email: Option<String>,
}

/// The largest session file that will be read.
const MAX_BYTES: u64 = 64 << 10;

/// Reads a legacy session file. A missing file is `Ok(None)`.
pub fn read(path: &Path) -> Result<Option<Legacy>, Error> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > MAX_BYTES => {
            return Err(Error::Malformed(format!("{} is {} bytes, too large to be a session", path.display(), meta.len())));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::Io(format!("couldn't read {} ({e})", path.display()))),
    }
    // The raw file bytes and the token `serde` deserializes both hold the token in plain text,
    // the same as the final `Secret` does. Dropping them ordinarily leaves that plain text for
    // the allocator, unzeroed; only `Secret`'s own copy would be covered. Wrapping both in
    // `Zeroizing` closes that gap. Destructuring `Stored` below moves `token` out whole, with no
    // clone and no second plain copy to forget about.
    let bytes = Zeroizing::new(std::fs::read(path).map_err(|e| Error::Io(format!("couldn't read {} ({e})", path.display())))?);
    let Stored { token, service, user, display, email } =
        serde_json::from_slice(&bytes).map_err(|e| Error::Malformed(format!("{} didn't parse ({e})", path.display())))?;
    let token = Zeroizing::new(token);
    if token.trim().is_empty() {
        return Err(Error::Malformed(format!("{} carries no session", path.display())));
    }
    let service = accounts::normalise(&service);
    // The subject is never stood in for. A handle built on anything else would file the token
    // where nothing looks for it. See [`Legacy::incomplete`].
    let user = user.trim().to_owned();
    let display = if display.trim().is_empty() { user.clone() } else { display };
    Ok(Some(Legacy { service, user, display, email, token: Secret::new(token.trim()) }))
}

/// Hands a legacy session to the suite store and removes the file.
///
/// Call this once at startup, before asking the suite store who is signed in. Safe to call when
/// there is no file. Safe to call again after it has run.
pub fn adopt<S: TokenStore>(store: &Store<S>, path: &Path, now: i64) -> Result<Adopted, Error> {
    let Some(legacy) = read(path)? else { return Ok(Adopted::Nothing) };
    if let Some(unsaid) = legacy.incomplete() {
        return Ok(Adopted::Kept { why: format!("{} does not say {unsaid} yet", path.display()) });
    }
    if let Err(why) = store.usable() {
        return Ok(Adopted::Kept { why: why.to_string() });
    }
    let service = accounts::normalise(&legacy.service);
    let id = accounts::handle(&service, &legacy.user);
    let existing = if store.has_token(&id)? { store.accounts()?.get(&id).cloned() } else { None };
    if let Some(account) = existing {
        if file_still_holds(path, &legacy) {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::Io(format!("the suite already holds this session. {} is still there ({e})", path.display()))),
            }
        }
        return Ok(Adopted::Taken(account));
    }
    let new = NewAccount {
        service: legacy.service.clone(),
        user: legacy.user.clone(),
        display: legacy.display.clone(),
        email: legacy.email.clone(),
        linked: false,
    };
    let account = store.sign_in(&new, &legacy.token, now)?;
    // Only now, with the session held somewhere better, does the file go. Something else may
    // have rewritten it in the meantime (another process still running the application's own,
    // now-superseded writer, mid-refresh); deleting that newer content would discard a session
    // nothing else holds a copy of. `file_still_holds` only lets the removal through when the
    // file is still exactly what was just adopted. Leaving it is always safe: the next startup's
    // `adopt` call sees it again and tries once more.
    if file_still_holds(path, &legacy) {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::Io(format!("the session moved to the keychain. {} is still there ({e})", path.display()))),
        }
    }
    Ok(Adopted::Taken(account))
}

/// Whether `path` still parses to exactly `adopted`, right before it would be removed.
///
/// Any other outcome, a changed file, a vanished file, or one that no longer parses, is treated
/// the same: do not delete. A changed file holds something nothing else has adopted yet; a
/// vanished one needs no removal; an unparseable one is not safe to reason about here at all.
fn file_still_holds(path: &Path, adopted: &Legacy) -> bool {
    matches!(read(path), Ok(Some(current)) if current == *adopted)
}
