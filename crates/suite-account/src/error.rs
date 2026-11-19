//! What can go wrong.

/// A failure in the shared account store.
///
/// None of these is "nobody is signed in". That is an ordinary answer. It comes back as
/// [`crate::Shared::None`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// This process reads and writes nothing: `ARTCRAFT_SUITE_DIR` is set and empty, or the system
    /// has no per-user configuration folder.
    Isolated,
    Io(String),
    /// The accounts file is in a format this build does not know, or is not an accounts file.
    Malformed(String),
    /// The operating system's keychain refused.
    Keychain(String),
    /// No account on file has that handle.
    Unknown(String),
    /// A write named a generation the account has already moved past: another sign-in or removal
    /// landed first. See RFC-0025's "Cross-app lifecycle guarantees".
    StaleGeneration {
        id: String,
        expected: u32,
        current: u32,
    },
    /// This handle has reached the highest generation `u32` can hold. Nothing can bump it
    /// further. The write that would have is refused. Wrapping back to a small generation would
    /// let a stale caller pass the check again. Reaching this needs about four billion sign-ins
    /// or removals of the same handle.
    GenerationExhausted(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Isolated => f.write_str("this process keeps no shared account"),
            Error::Io(why) | Error::Malformed(why) => f.write_str(why),
            Error::Keychain(why) => write!(f, "the keychain refused: {why}"),
            Error::Unknown(id) => write!(f, "no account {id} is signed in"),
            Error::StaleGeneration { id, expected, current } => {
                write!(
                    f,
                    "account {id} moved from generation {expected} to {current}; this write was made against a generation that is no longer current"
                )
            }
            Error::GenerationExhausted(id) => write!(f, "account {id} has reached the highest generation this build can record"),
        }
    }
}

impl std::error::Error for Error {}
