//! App passwords in the OS keyring. The service is the app's name and the entry user is [`Account::key`], so two apps
//! signed in to one account keep separate passwords. There is no plaintext fallback.

use std::io;

use crate::{Account, Error};

fn keyring_error(e: ::keyring::Error) -> Error {
    use ::keyring::Error as K;
    match e {
        K::NoDefaultStore => unavailable("no keyring store is configured".into()),
        K::NoStorageAccess(d) => unavailable(d.to_string()),
        K::PlatformFailure(d) => unavailable(d.to_string()),
        other => Error::Io(io::Error::other(format!("The OS keyring failed: {other}"))),
    }
}

fn unavailable(detail: String) -> Error {
    Error::Io(io::Error::other(format!("No OS keyring is available ({detail}); the Nextcloud app password can't be stored")))
}

fn entry(app: &str, account: &Account) -> Result<::keyring::Entry, Error> {
    ::keyring::Entry::new(app, &account.key()).map_err(keyring_error)
}

/// Saves `password` for `account` under the service `app`, replacing any earlier one.
pub fn store_password(app: &str, account: &Account, password: &str) -> Result<(), Error> {
    entry(app, account)?.set_password(password).map_err(keyring_error)
}

/// The saved password, or `None` when there is none.
pub fn load_password(app: &str, account: &Account) -> Result<Option<String>, Error> {
    match entry(app, account)?.get_password() {
        Ok(p) => Ok(Some(p)),
        Err(::keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(keyring_error(e)),
    }
}

/// Removes the saved password; succeeds when there is none.
pub fn delete_password(app: &str, account: &Account) -> Result<(), Error> {
    match entry(app, account)?.delete_credential() {
        Ok(()) | Err(::keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(keyring_error(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyring_errors_are_actionable() {
        let msg = keyring_error(::keyring::Error::NoDefaultStore).to_string();
        assert!(msg.contains("No OS keyring"), "{msg}");
        assert!(msg.contains("can't be stored"), "{msg}");
    }
}
