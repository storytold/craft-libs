//! A cancel flag shared between the caller and a running exchange.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A flag a caller (a Cancel button, a signal handler) sets to cancel running work. Clones share
/// the flag. Clients check it between exchanges (`SoapClient`, `discover`); an exchange already
/// on the wire runs to its timeout.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// A token that is not cancelled yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the flag for every clone. Cannot be undone; work already on the wire is not interrupted.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether [`Self::cancel`] has been called on this token or any clone of it.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_the_flag() {
        let a = CancelToken::new();
        let b = a.clone();
        assert!(!b.is_cancelled());
        a.cancel();
        assert!(b.is_cancelled());
    }
}
