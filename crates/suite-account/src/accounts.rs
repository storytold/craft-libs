//! The accounts file: who has signed in and at which deployment.
//!
//! No token is in here. Every field is something a user interface shows, or something needed to
//! find the token in the keychain. See [`crate::store`] for the token itself.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::Error;

/// The format this code writes. A reader that does not recognise the number leaves the file alone.
pub const FORMAT: u32 = 1;

/// The largest accounts file that will be read. A suite account list is a few hundred bytes. A
/// megabyte is already far past anything legitimate.
const MAX_BYTES: u64 = 1 << 20;

/// One account, as the suite remembers it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    /// The handle for this account inside the suite. It is also the keychain entry's user name.
    ///
    /// A digest of `service` and `user` (see [`handle`]). Signing in again as the same person at
    /// the same deployment therefore lands on the same keychain entry. No stale entry is left
    /// behind.
    pub id: String,
    /// The deployment this account signs in to, as a base URL with no trailing slash.
    pub service: String,
    /// The deployment's own identifier for this account: the OIDC subject, never an address.
    pub user: String,
    /// What to show a person. Never an identity: two people can share a display name.
    pub display: String,
    pub email: Option<String>,
    /// Unix seconds when this account was first stored here.
    pub added: i64,
    /// Unix seconds when an application last used it.
    pub last_used: i64,
    /// Whether cloud storage is linked, as recorded by [`crate::NewAccount::linked`] at sign-in.
    /// For a user interface only; the deployment is the authority and is asked again on each
    /// start.
    pub linked: bool,
    /// Bumped every time this row's content changes: a sign-in that replaces the entry, or (per
    /// RFC-0025's cross-app lifecycle guarantees) a removal. A caller that read the account before
    /// one of those passes its generation back on a later write; [`crate::Store::update_token`]
    /// refuses a write made against a generation that is no longer current, so a refresh started
    /// before a change completes can never land after it and overwrite something newer.
    pub generation: u32,
}

/// What is left behind when an account is removed: the generation it reached, so a re-add or a
/// stale write can never land before it.
///
/// RFC-0025's "Cross-app lifecycle guarantees" (point 2): generations are monotonic across removal
/// and re-add. Without this, a removed-and-re-added account restarted at generation 0, so a stale
/// refresh from before the removal passed the generation check against the new session and
/// replaced its token; a refresh racing a removal could even put a keychain entry back after the
/// removal deleted it. See [`Located::Tombstoned`] and [`crate::Store::update_token`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tombstone {
    pub id: String,
    /// The generation the account reached, bumped once more by the removal itself. A write
    /// against any generation at or before this one is stale.
    pub generation: u32,
}

/// Where [`Accounts::locate`] found (or didn't find) an id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Located<'a> {
    /// A row is on the list.
    Active(&'a Account),
    /// No row, but a tombstone: the account was removed and holds this generation. A write against
    /// it is stale, never [`crate::Error::Unknown`]; see [`crate::Store::update_token`].
    Tombstoned(u32),
    /// Neither. This handle has never been on this list.
    Unknown,
}

/// The whole file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Accounts {
    pub format: u32,
    pub accounts: Vec<Account>,
    /// The account an application should use when it has no reason to prefer another.
    pub active: Option<String>,
    /// Removed accounts, holding the generation they reached. See [`Tombstone`].
    pub tombstones: Vec<Tombstone>,
}

impl Default for Accounts {
    fn default() -> Accounts {
        Accounts { format: FORMAT, accounts: Vec::new(), active: None, tombstones: Vec::new() }
    }
}

impl Accounts {
    pub fn get(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }

    /// The generation a tombstone for `id` holds, when there is one.
    pub fn tombstone_generation(&self, id: &str) -> Option<u32> {
        self.tombstones.iter().find(|t| t.id == id).map(|t| t.generation)
    }

    /// Finds `id` as an active row, a tombstone, or neither. The one lookup
    /// [`crate::Store::update_token`] needs to answer a write with the right error: a stale
    /// generation against a row that is still there, a stale generation against one that was
    /// removed, or a handle that was never on this list at all.
    pub fn locate(&self, id: &str) -> Located<'_> {
        match self.get(id) {
            Some(account) => Located::Active(account),
            None => match self.tombstone_generation(id) {
                Some(generation) => Located::Tombstoned(generation),
                None => Located::Unknown,
            },
        }
    }

    /// The active account, when one is named and still on the list.
    ///
    /// An `active` naming an account that is gone gives the ordinary answer of `None`. A
    /// half-finished sign-out on another machine must not stop an application starting.
    pub fn current(&self) -> Option<&Account> {
        self.active.as_deref().and_then(|id| self.get(id))
    }

    /// Adds or replaces an account. The account then becomes the active one.
    ///
    /// Clears any tombstone for this id: the account is current again, so nothing should still
    /// answer a write against it with "removed".
    pub fn put(&mut self, account: Account) {
        self.tombstones.retain(|t| t.id != account.id);
        self.active = Some(account.id.clone());
        match self.accounts.iter_mut().find(|a| a.id == account.id) {
            Some(existing) => *existing = account,
            None => self.accounts.push(account),
        }
    }

    /// Drops an account and leaves a tombstone holding its generation, bumped once more by the
    /// removal itself (RFC-0025 lifecycle guarantee 2). Whoever is left over, if anybody, becomes
    /// active.
    ///
    /// Refuses with [`Error::GenerationExhausted`] when `generation` is already `u32::MAX`.
    /// Wrapping past it would hand out a small number again. A stale caller's check against that
    /// small number would then pass. Monotonic generations exist to rule that out.
    pub fn remove_with_tombstone(&mut self, id: &str, generation: u32) -> Result<(), Error> {
        let bumped = generation.checked_add(1).ok_or_else(|| Error::GenerationExhausted(id.to_owned()))?;
        self.accounts.retain(|a| a.id != id);
        if self.active.as_deref() == Some(id) {
            self.active = self.accounts.first().map(|a| a.id.clone());
        }
        match self.tombstones.iter_mut().find(|t| t.id == id) {
            Some(existing) => existing.generation = bumped,
            None => self.tombstones.push(Tombstone { id: id.to_owned(), generation: bumped }),
        }
        Ok(())
    }
}

/// The handle for an account at a deployment.
///
/// A digest, with the address and the subject kept out of it. The digest is stable. One account
/// therefore stays on one keychain entry however many times somebody signs in. The handle is also
/// opaque. A credential manager window puts the handle on screen. Nobody's address belongs in that
/// list.
pub fn handle(service: &str, user: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(normalise(service).as_bytes());
    hash.update(b"\n");
    hash.update(user.as_bytes());
    let digest = hash.finalize();
    digest.iter().take(12).map(|b| format!("{b:02x}")).collect()
}

/// A deployment's base URL with any trailing slashes taken off. One deployment is then one account
/// however the URL was typed.
pub fn normalise(service: &str) -> String {
    service.trim().trim_end_matches('/').to_owned()
}

/// Reads the file. A missing file is an empty list. That is what "nobody has signed in" looks like.
pub fn read(path: &Path) -> Result<Accounts, Error> {
    let bytes = match std::fs::metadata(path) {
        Ok(meta) if meta.len() > MAX_BYTES => {
            return Err(Error::Malformed(format!("{} is {} bytes, too large to be an account list", path.display(), meta.len())));
        }
        Ok(_) => std::fs::read(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Accounts::default()),
        Err(e) => return Err(Error::Io(format!("couldn't read {} ({e})", path.display()))),
    };
    let bytes = bytes.map_err(|e| Error::Io(format!("couldn't read {} ({e})", path.display())))?;
    let parsed: Accounts = serde_json::from_slice(&bytes).map_err(|e| Error::Malformed(format!("{} didn't parse ({e})", path.display())))?;
    if parsed.format > FORMAT {
        return Err(Error::Malformed(format!("{} is format {}. This build understands {FORMAT}.", path.display(), parsed.format)));
    }
    Ok(parsed)
}

/// Writes the file through a temporary file and a rename. A crash then never leaves half of one.
///
/// Readable by this user alone where the platform has file modes. The file holds no token. It does
/// say who is signed in. That is nobody else's business.
///
/// Refuses a list that would serialize past [`MAX_BYTES`], before any file is touched. [`read`]
/// rejects a file that large; writing one anyway would succeed here and then make every later
/// read of this list fail, including the reads every other mutation needs before it can repair
/// anything. An oversized `display` or `email` on one account is the realistic way to reach this,
/// not the ordinary handful of accounts a suite ever holds.
pub fn write(path: &Path, accounts: &Accounts) -> Result<(), Error> {
    let dir = path.parent().ok_or_else(|| Error::Io(format!("{} has no parent folder", path.display())))?;
    let json = serde_json::to_vec_pretty(accounts).map_err(|e| Error::Malformed(e.to_string()))?;
    if json.len() as u64 > MAX_BYTES {
        return Err(Error::Malformed(format!(
            "the account list would be {} bytes, too large to write; {} would reject it unread",
            json.len(),
            path.display()
        )));
    }
    std::fs::create_dir_all(dir).map_err(|e| Error::Io(format!("couldn't create {} ({e})", dir.display())))?;
    use std::io::Write;

    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let pid = std::process::id();
    let tmp = dir.join(format!(".{}.{}.{}.tmp", path.file_name().and_then(|n| n.to_str()).unwrap_or("accounts"), stamp, pid));
    #[cfg(unix)]
    {
        use std::fs::OpenOptions;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| Error::Io(format!("couldn't create {} ({e})", tmp.display())))?;
        file.write_all(&json).map_err(|e| Error::Io(format!("couldn't write {} ({e})", tmp.display())))?;
    }
    #[cfg(not(unix))]
    {
        use std::fs::OpenOptions;
        let mut file =
            OpenOptions::new().write(true).create_new(true).open(&tmp).map_err(|e| Error::Io(format!("couldn't create {} ({e})", tmp.display())))?;
        file.write_all(&json).map_err(|e| Error::Io(format!("couldn't write {} ({e})", tmp.display())))?;
        restrict(&tmp)?;
    }
    std::fs::rename(&tmp, path).map_err(|e| Error::Io(format!("couldn't replace {} ({e})", path.display())))
}

#[cfg(not(unix))]
fn restrict(_path: &Path) -> Result<(), Error> {
    // Windows gives a file in `%APPDATA%` the profile's own access rules. Those are this user's.
    Ok(())
}
