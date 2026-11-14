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
//! ## Concurrent mutations and partial-failure recovery
//!
//! Several apps run at once against one account list and one keychain (RFC-0025's "Cross-app
//! lifecycle guarantees"). Every mutation of the accounts file ([`Store::sign_in`],
//! [`Store::update_token`], [`Store::discard`], [`Store::set_active`], [`Store::touch`]) therefore
//! reads, changes and writes it under one OS-level file lock ([`lock::with_lock`]), never an
//! unlocked read-modify-write. A pure read ([`Store::accounts`], [`Store::shared`]) does not take
//! the lock; [`accounts::write`] already replaces the file through a temporary file and a rename,
//! so a reader never sees a half-written file, only one write older than the latest.
//!
//! The lock does not wrap an unconditional keychain call. A keychain write or removal happens
//! either entirely before the lock is taken or, for [`Store::discard`], [`Store::sign_in`]'s
//! re-check and [`Store::update_token`]'s write, inside the same lock hold as the list change it
//! belongs with. Each of those three is conditional on the list mutation it sits beside actually
//! going ahead, never a call made regardless. A locked section is never held across a call into
//! the keychain that another `Store` in this process could itself be waiting on; that would
//! deadlock two `Store`s over one folder in a single process the same way two applications would
//! be.
//!
//! Each operation's ordering is chosen so a crash, or another process winning a race, leaves a
//! state the next reader detects and repairs rather than a silent mismatch (guarantee 1):
//!
//! * [`Store::sign_in`] writes the keychain token first, the list second. A crash in between
//!   leaves one keychain entry nothing on the list points at yet. Harmless: the account's handle
//!   is a digest of the deployment and the subject, so the next sign-in for the same account
//!   overwrites that same entry rather than minting a new one. A concurrent [`Store::discard`]
//!   for the same handle landing in that same window is not harmless the same way: it would
//!   delete the entry this call just wrote and then, moments later, this call would still commit
//!   a row claiming a session that no longer has a token behind it. The lock closure therefore
//!   re-checks the keychain just before committing the row. It writes the token again when
//!   something else has already changed it. The row this call reports `Ok` for then always has
//!   its token in place the moment it lands.
//! * [`Store::discard`] removes the keychain entry and writes the list's tombstone under the same
//!   lock hold. A crash between them leaves a row on the list with no entry behind it; `shared()`
//!   already reads that as nobody signed in, and the next sign-in for that account tidies the row
//!   up.
//! * [`Store::update_token`] checks the account's generation once, unlocked, purely to reject an
//!   already-stale caller cheaply before it ever takes the lock. The keychain write itself
//!   happens only under the lock, in the generation re-check's own success arm, immediately
//!   before the row commits. A sign-in, a discard, or a discard followed by a re-add can land in
//!   the window the unlocked check left open. The re-check exists to catch exactly that, whoever
//!   won it. Either way, this call answers [`Error::StaleGeneration`] (or [`Error::Unknown`], for
//!   a handle never on this list at all; see "Cross-app lifecycle guarantees" point 2 and
//!   [`accounts::Accounts::locate`]) having never touched the keychain. There is never a stale
//!   token left in it to clean up or weigh against the winner's own. A crash between the write
//!   and the commit, both now inside one lock hold, leaves the list one generation behind a
//!   keychain token that is already the
//!   newer one; the next successful `update_token` against that same, still-current generation
//!   simply writes through it again.
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
mod lock;
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
    ///
    /// Checked against isolation first, the same as every call that reaches the keychain. An
    /// isolated store answers [`Error::Isolated`] here too, before any keychain read.
    pub fn has_token(&self, id: &str) -> Result<bool, Error> {
        let _ = self.dir.as_deref().ok_or(Error::Isolated)?;
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

    /// Records a sign-in: the token into the keychain, then the account onto the list, the latter
    /// as one locked read-change-write (see the module docs, "Concurrent mutations and
    /// partial-failure recovery").
    ///
    /// The token goes first. A failed list write then leaves the previous state intact and one
    /// unreferenced keychain entry. The next sign-in for the same account overwrites that entry,
    /// because the handle is a digest of the deployment and the subject. Nothing fresh is minted
    /// each time. The other order would leave an account on the list with no token behind it.
    ///
    /// A [`Store::discard`] for this same handle can still land between that first write and the
    /// lock below, taking the entry this call just placed with it. The lock closure re-checks the
    /// keychain right before it commits the row. It writes the token again when something else
    /// has already changed it. An `Ok` from this call then never reports a session with no token
    /// behind it.
    pub fn sign_in(&self, new: &NewAccount, token: &Secret, now: i64) -> Result<Account, Error> {
        self.usable()?;
        let path = self.path().ok_or(Error::Isolated)?;
        let service = accounts::normalise(&new.service);
        let id = accounts::handle(&service, &new.user);
        self.tokens.put(&id, token).map_err(Error::Keychain)?;
        lock::with_lock(&path, || {
            let mut list = accounts::read(&path)?;
            let existing = list.get(&id).cloned();
            let added = existing.as_ref().map_or(now, |a| a.added);
            // A fresh generation, continuing past whatever this handle last reached, whether that
            // was an active row (a sign-in replacing it) or a tombstone (a removal, per RFC-0025
            // lifecycle guarantee 2: generations are monotonic across removal and re-add, never
            // restarting at 0). Either way a write made against the generation this handle held a
            // moment ago is now stale. See [`Account::generation`].
            let generation = match &existing {
                Some(account) => account.generation.checked_add(1).ok_or_else(|| Error::GenerationExhausted(id.clone()))?,
                None => match list.tombstone_generation(&id) {
                    Some(generation) => generation.checked_add(1).ok_or_else(|| Error::GenerationExhausted(id.clone()))?,
                    None => 0,
                },
            };
            let account = Account {
                id: id.clone(),
                service: service.clone(),
                user: new.user.clone(),
                display: new.display.clone(),
                email: new.email.clone(),
                added,
                last_used: now,
                linked: new.linked,
                generation,
            };
            // A discard for this handle could have run entirely in the window between the
            // unlocked write above and this lock: it would have removed that entry on its way to
            // the tombstone this call just read past. Putting the token again here, still inside
            // the lock, closes that window: the row committed just below is never left pointing
            // at a keychain entry something else already took. Skipped when nothing changed it,
            // so the common, non-racing sign-in never pays for a second keychain write.
            if self.tokens.get(&id).map_err(Error::Keychain)?.as_ref() != Some(token) {
                self.tokens.put(&id, token).map_err(Error::Keychain)?;
            }
            list.put(account.clone());
            accounts::write(&path, &list)?;
            Ok(account)
        })
    }

    /// Writes a refreshed token back, honouring the account's generation.
    ///
    /// Updates this account's row in place. It never changes which account is active. A refresh
    /// runs in the background for whichever account needs one, including one the person did not
    /// select. It must never steal the selection away from the account they actually picked.
    /// [`Store::sign_in`] and [`Store::set_active`] are the only calls that change `active`.
    ///
    /// `expected_generation` is the generation the caller last read the account at. A sign-in or a
    /// removal bumps the generation (see [`Account::generation`]); a refresh that was already in
    /// flight when one of those landed is then refused rather than silently overwriting something
    /// newer, per RFC-0025's "Cross-app lifecycle guarantees" (point 2: removal wins over refresh).
    /// An account that was removed answers [`Error::StaleGeneration`] holding the tombstone's
    /// generation, never [`Error::Unknown`] and never a silent recreation; only a handle that has
    /// never been on this list at all is `Unknown`. See [`accounts::Accounts::locate`].
    ///
    /// Checked twice. The keychain is written only once, inside the lock, after the second check
    /// passes.
    ///
    /// The first check, unlocked, is a fast rejection only. A caller already stale when it calls
    /// at all never even takes the lock. It writes nothing. The second check, under the accounts
    /// lock, is what the keychain write is actually conditioned on: `self.tokens.put` runs only
    /// in that check's own success arm, right before the row commits, both inside the one lock
    /// hold. A sign-in or a discard for the same handle can land in the window between the two
    /// checks. The second check exists to catch exactly that. By the time it runs, this call
    /// still has not touched the keychain at all. Every losing path, a newer sign-in, a discard,
    /// a discard followed by a re-add, answers [`Error::StaleGeneration`] (or [`Error::Unknown`])
    /// having written nothing. No stale token is ever left in the keychain for a cleanup step to
    /// weigh against a winner's. See the module docs ("Concurrent mutations and partial-failure
    /// recovery").
    pub fn update_token(&self, id: &str, expected_generation: u32, token: &Secret, now: i64) -> Result<Account, Error> {
        self.usable()?;
        let path = self.path().ok_or(Error::Isolated)?;

        match accounts::read(&path)?.locate(id) {
            accounts::Located::Active(account) if account.generation == expected_generation => {}
            accounts::Located::Active(account) => {
                return Err(Error::StaleGeneration { id: id.to_owned(), expected: expected_generation, current: account.generation });
            }
            accounts::Located::Tombstoned(current) => {
                return Err(Error::StaleGeneration { id: id.to_owned(), expected: expected_generation, current });
            }
            accounts::Located::Unknown => return Err(Error::Unknown(id.to_owned())),
        }

        lock::with_lock(&path, || {
            let mut list = accounts::read(&path)?;
            match list.locate(id) {
                accounts::Located::Active(account) if account.generation == expected_generation => {
                    let mut updated = account.clone();
                    updated.generation = updated.generation.checked_add(1).ok_or_else(|| Error::GenerationExhausted(id.to_owned()))?;
                    updated.last_used = now;
                    // The write and the commit are one atomic step from here on, with respect to
                    // every other mutator. sign_in and discard both take this same lock for
                    // their own keychain and list changes. Nothing can land between this put and
                    // this write.
                    self.tokens.put(id, token).map_err(Error::Keychain)?;
                    // Not `list.put`: a refresh is maintenance, never a reason to make this the
                    // account a person selected. See [`accounts::Accounts::refresh_in_place`].
                    list.refresh_in_place(id, updated.clone());
                    accounts::write(&path, &list)?;
                    Ok(updated)
                }
                accounts::Located::Active(account) => {
                    Err(Error::StaleGeneration { id: id.to_owned(), expected: expected_generation, current: account.generation })
                }
                accounts::Located::Tombstoned(current) => Err(Error::StaleGeneration { id: id.to_owned(), expected: expected_generation, current }),
                accounts::Located::Unknown => Err(Error::Unknown(id.to_owned())),
            }
        })
    }

    /// Makes an account the current one.
    pub fn set_active(&self, id: &str) -> Result<(), Error> {
        let path = self.path().ok_or(Error::Isolated)?;
        lock::with_lock(&path, || {
            let mut list = accounts::read(&path)?;
            if list.get(id).is_none() {
                return Err(Error::Unknown(id.to_owned()));
            }
            list.active = Some(id.to_owned());
            accounts::write(&path, &list)
        })
    }

    /// Records that an application used the account, for a list that can show the most recent one.
    pub fn touch(&self, id: &str, now: i64) -> Result<(), Error> {
        let path = self.path().ok_or(Error::Isolated)?;
        lock::with_lock(&path, || {
            let mut list = accounts::read(&path)?;
            let Some(account) = list.accounts.iter_mut().find(|a| a.id == id) else {
                return Err(Error::Unknown(id.to_owned()));
            };
            account.last_used = now;
            accounts::write(&path, &list)
        })
    }

    /// Clears an account here: the keychain entry and the row's tombstone, under one lock hold.
    ///
    /// This is the local half of a sign-out only. Ending the session at the deployment, so every
    /// machine the person left one on drops it too, is a distinct call on the provider adapter; a
    /// consumer that wants both makes that call first, while the token can still be read, and then
    /// calls this. See the module documentation and RFC-0025's "Cross-app lifecycle guarantees".
    ///
    /// The keychain removal happens inside the same lock hold as the list write, not before it: a
    /// refresh racing this discard (see [`Store::update_token`]'s second check) must never be able
    /// to land its own keychain write back in between this call's keychain removal and its list
    /// write and so put the entry right back.
    pub fn discard(&self, id: &str) -> Result<(), Error> {
        let path = self.path().ok_or(Error::Isolated)?;
        lock::with_lock(&path, || {
            let mut list = accounts::read(&path)?;
            if let Some(account) = list.get(id).cloned() {
                self.tokens.remove(id).map_err(Error::Keychain)?;
                list.remove_with_tombstone(id, account.generation)?;
                accounts::write(&path, &list)?;
            }
            Ok(())
        })
    }

    /// Clears the keychain entry for an account, leaving the list alone.
    ///
    /// Checked against isolation first. `Store::at(None, _)` keeps nothing; without this check an
    /// isolated store could still reach into the keychain and delete another application's
    /// shared credential on this same machine.
    pub fn forget(&self, id: &str) -> Result<(), Error> {
        let _ = self.dir.as_deref().ok_or(Error::Isolated)?;
        self.tokens.remove(id).map_err(Error::Keychain)
    }
}

impl<S: TokenStore + std::fmt::Debug> std::fmt::Debug for Store<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").field("dir", &self.dir).field("tokens", &self.tokens).finish()
    }
}
