//! The Craft App Suite's shared account store: the suite's keychain.
//!
//! Somebody signs in once and every Craft App acts as them. Before this, each application kept its
//! own session file. Taken across the suite that meant a sign-in per application, a copy of a
//! bearer token on disk per application, and a separate place to revoke per application.
//!
//! The store has two halves, kept deliberately apart:
//!
//! * The list of accounts is a small JSON file in one suite-level folder ([`paths`]). It names who
//!   has signed in, at which deployment, and which account is current. It holds no credential.
//! * The token is in the operating system's keychain ([`store`]). Windows Credential Manager, the
//!   macOS Keychain, or the Secret Service.
//!
//! Nothing here is mandatory. An application asks [`Store::shared`] what the suite has to offer and
//! gets one of three answers: somebody is signed in, nobody is signed in, or there is an account on
//! file whose token could not be read. The second and third leave the application exactly as it was
//! before this crate existed, offering its own sign-in. A machine with no keychain, a locked
//! keychain, a missing folder and an empty suite all land on "nobody is signed in". None of them is
//! an error that stops an application starting.
//!
//! [`Store::discard`] clears the keychain entry and then the row, for a session this machine
//! should stop holding. [`Store::forget`] clears only the keychain entry, leaving the row for a
//! user interface to still show (and, say, offer "sign in again" on). Neither talks to the
//! deployment: ending the session there, so every machine the person left one on drops it too, is
//! a distinct operation that belongs to the provider adapter that knows how to ask the deployment
//! for that, per RFC-0025's "Cross-app lifecycle guarantees" (point 4: forgetting is local,
//! revoking is at the provider). A consumer that wants both calls its own revoke, then
//! [`Store::discard`].
//!
//! An application upgrading from its own session file calls [`migrate::adopt`] once at startup.
//!
//! ```no_run
//! # fn main() -> Result<(), craft_suite_account::Error> {
//! use craft_suite_account::{Shared, Store};
//!
//! let store = Store::open();
//! match store.shared() {
//!     Shared::Session { account, token } => {
//!         // Use `token` as a bearer against `account.service`.
//!         let _ = (account, token);
//!     }
//!     // Offer this application's own sign-in, exactly as before.
//!     Shared::None => {}
//!     Shared::Unreadable { why, .. } => eprintln!("the shared sign-in could not be read: {why}"),
//! }
//! # Ok(()) }
//! ```

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod accounts;
mod error;
pub mod migrate;
pub mod paths;
mod secret;
pub mod store;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

pub use accounts::{Account, Accounts};
pub use error::Error;
pub use migrate::Adopted;
pub use secret::Secret;
pub use store::{KEYCHAIN_SERVICE, Keychain, Memory, TokenStore};

/// What the suite store has to offer.
#[derive(Debug)]
pub enum Shared {
    /// Somebody is signed in and the token came back.
    Session { account: Account, token: Secret },
    /// Nobody is signed in here. Offer a sign-in.
    None,
    /// An account is on file and its token could not be read: a locked keychain, a keychain this
    /// process may not reach, or an entry somebody deleted by hand.
    ///
    /// Offer a sign-in. Say why the shared one did not come through. Quietly showing a signed-out
    /// window would leave somebody wondering where their account went.
    Unreadable { account: Account, why: String },
}

/// A sign-in about to be recorded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewAccount {
    /// The deployment's base URL.
    pub service: String,
    /// The deployment's identifier for the account: the OIDC subject.
    pub user: String,
    pub display: String,
    pub email: Option<String>,
    /// Whether cloud storage is linked. Recorded for a user interface; the deployment is the
    /// authority and is asked again on each start.
    pub linked: bool,
}

/// The suite's account store.
pub struct Store<S: TokenStore = Keychain> {
    /// The suite folder. `None` when this process keeps nothing.
    dir: Option<PathBuf>,
    tokens: S,
}

impl Store<Keychain> {
    /// The real store: the suite folder for this platform and the platform's keychain.
    pub fn open() -> Store<Keychain> {
        Store::at(paths::resolve(&paths::Probe::default()).dir, Keychain)
    }
}

impl<S: TokenStore> Store<S> {
    /// A store over a given folder and token store. `None` keeps nothing.
    pub fn at(dir: Option<PathBuf>, tokens: S) -> Store<S> {
        Store { dir, tokens }
    }

    /// The suite folder, when there is one.
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// The accounts file, when there is one.
    pub fn path(&self) -> Option<PathBuf> {
        self.dir.as_deref().map(paths::file)
    }

    /// Whether this store can hold a session. It needs a folder. The machine needs a keychain.
    ///
    /// Checked before a sign-in and before a migration. Neither then half-completes.
    pub fn usable(&self) -> Result<(), Error> {
        let _ = self.dir.as_deref().ok_or(Error::Isolated)?;
        self.tokens.available().map_err(Error::Keychain)
    }

    /// Whether a readable token is stored for this account handle.
    pub fn has_token(&self, id: &str) -> Result<bool, Error> {
        Ok(self.tokens.get(id).map_err(Error::Keychain)?.is_some())
    }

    /// Every account on file. An isolated store has none.
    pub fn accounts(&self) -> Result<Accounts, Error> {
        match self.path() {
            Some(path) => accounts::read(&path),
            None => Ok(Accounts::default()),
        }
    }

    /// What the suite has to offer. Never fails: absence of every kind is [`Shared::None`].
    ///
    /// This is the call an application makes at startup. The answer decides whether it offers its
    /// own sign-in.
    pub fn shared(&self) -> Shared {
        let list = match self.accounts() {
            Ok(list) => list,
            Err(Error::Isolated) => return Shared::None,
            Err(e) => {
                log::warn!("the suite account list could not be read: {e}");
                return Shared::None;
            }
        };
        let Some(account) = list.current().cloned() else { return Shared::None };
        match self.tokens.get(&account.id) {
            Ok(Some(token)) => Shared::Session { account, token },
            // On file with no entry behind it. Somebody deleted the entry, or a sign-out got half
            // way. Either way nobody is signed in. The next sign-in tidies the row up.
            Ok(None) => Shared::None,
            Err(why) => Shared::Unreadable { account, why },
        }
    }

    /// Records a sign-in: the token into the keychain, then the account onto the list.
    ///
    /// The token goes first. A failed list write then leaves the previous state intact and one
    /// unreferenced keychain entry. The next sign-in for the same account overwrites that entry,
    /// because the handle is a digest of the deployment and the subject. Nothing fresh is minted
    /// each time. The other order would leave an account on the list with no token behind it.
    pub fn sign_in(&self, new: &NewAccount, token: &Secret, now: i64) -> Result<Account, Error> {
        self.usable()?;
        let path = self.path().ok_or(Error::Isolated)?;
        let service = accounts::normalise(&new.service);
        let id = accounts::handle(&service, &new.user);
        self.tokens.put(&id, token).map_err(Error::Keychain)?;
        let mut list = accounts::read(&path)?;
        let existing = list.get(&id);
        let added = existing.map_or(now, |a| a.added);
        // A fresh generation, same as a removal would leave behind: a write made against the
        // generation this account held a moment ago is now stale. See [`Account::generation`].
        let generation = existing.map_or(0, |a| a.generation + 1);
        let account = Account {
            id,
            service,
            user: new.user.clone(),
            display: new.display.clone(),
            email: new.email.clone(),
            added,
            last_used: now,
            generation,
        };
        list.put(account.clone());
        accounts::write(&path, &list)?;
        Ok(account)
    }

    /// Writes a refreshed token back, honouring the account's generation.
    ///
    /// `expected_generation` is the generation the caller last read the account at. A sign-in or a
    /// removal bumps the generation (see [`Account::generation`]); a refresh that was already in
    /// flight when one of those landed is then refused here rather than silently overwriting
    /// something newer, per RFC-0025's "Cross-app lifecycle guarantees" (point 2: removal wins over
    /// refresh). An account that is gone outright, because it was removed, is [`Error::Unknown`]
    /// rather than [`Error::StaleGeneration`]; see the TODO on [`Accounts::remove`] for why that
    /// distinction is not sharper yet.
    pub fn update_token(&self, id: &str, expected_generation: u32, token: &Secret, now: i64) -> Result<Account, Error> {
        self.usable()?;
        let path = self.path().ok_or(Error::Isolated)?;
        let mut list = accounts::read(&path)?;
        let current = list.get(id).ok_or_else(|| Error::Unknown(id.to_owned()))?.generation;
        if current != expected_generation {
            return Err(Error::StaleGeneration { id: id.to_owned(), expected: expected_generation, current });
        }
        self.tokens.put(id, token).map_err(Error::Keychain)?;
        let account = list.accounts.iter_mut().find(|a| a.id == id).ok_or_else(|| Error::Unknown(id.to_owned()))?;
        account.generation += 1;
        account.last_used = now;
        let updated = account.clone();
        accounts::write(&path, &list)?;
        Ok(updated)
    }

    /// Makes an account the current one.
    pub fn set_active(&self, id: &str) -> Result<(), Error> {
        let path = self.path().ok_or(Error::Isolated)?;
        let mut list = accounts::read(&path)?;
        if list.get(id).is_none() {
            return Err(Error::Unknown(id.to_owned()));
        }
        list.active = Some(id.to_owned());
        accounts::write(&path, &list)
    }

    /// Records that an application used the account, for a list that can show the most recent one.
    pub fn touch(&self, id: &str, now: i64) -> Result<(), Error> {
        let path = self.path().ok_or(Error::Isolated)?;
        let mut list = accounts::read(&path)?;
        let Some(account) = list.accounts.iter_mut().find(|a| a.id == id) else {
            return Err(Error::Unknown(id.to_owned()));
        };
        account.last_used = now;
        accounts::write(&path, &list)
    }

    /// Clears an account here: the keychain entry, then the row.
    ///
    /// This is the local half of a sign-out only. Ending the session at the deployment, so every
    /// machine the person left one on drops it too, is a distinct call on the provider adapter; a
    /// consumer that wants both makes that call first, while the token can still be read, and then
    /// calls this. See the module documentation and RFC-0025's "Cross-app lifecycle guarantees".
    pub fn discard(&self, id: &str) -> Result<(), Error> {
        let path = self.path().ok_or(Error::Isolated)?;
        self.forget(id)?;
        let mut list = accounts::read(&path)?;
        list.remove(id);
        accounts::write(&path, &list)
    }

    /// Clears the keychain entry for an account, leaving the list alone.
    pub fn forget(&self, id: &str) -> Result<(), Error> {
        self.tokens.remove(id).map_err(Error::Keychain)
    }
}

impl<S: TokenStore + std::fmt::Debug> std::fmt::Debug for Store<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").field("dir", &self.dir).field("tokens", &self.tokens).finish()
    }
}
