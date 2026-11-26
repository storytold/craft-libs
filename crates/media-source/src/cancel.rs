//! Cooperative cancellation of media requests.
//!
//! A request worker installs a flag with [`with_cancel`]. Readers poll [`cancelled`] at
//! their existing interruption points and can return [`crate::MediaError::Cancelled`].
//! Use [`Context`] to carry the same flag to another worker.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

thread_local! {
    static CURRENT: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

/// Run `f` with this cancellation flag, restoring the preceding state when the scope ends.
pub fn with_cancel<R>(flag: &Arc<AtomicBool>, f: impl FnOnce() -> R) -> R {
    Context(Some(Arc::clone(flag))).run(f)
}

/// Whether the flag installed on this thread has been set. An empty context is never cancelled.
pub fn cancelled() -> bool {
    CURRENT.with_borrow(|flag| flag.as_ref().is_some_and(|flag| flag.load(Ordering::Relaxed)))
}

/// A captured cancellation scope that can be carried to another thread.
///
/// It holds the flag itself, so cancellation after capture is visible to the worker.
/// A default context has no flag.
#[derive(Clone, Default)]
pub struct Context(Option<Arc<AtomicBool>>);

impl Context {
    /// Capture this thread's current cancellation scope.
    pub fn current() -> Self {
        Self(CURRENT.with_borrow(Clone::clone))
    }

    /// Run `f` with this context, even when empty. Restore the previous context on return or unwind.
    pub fn run<R>(self, f: impl FnOnce() -> R) -> R {
        struct Restore(Option<Arc<AtomicBool>>);

        impl Drop for Restore {
            fn drop(&mut self) {
                CURRENT.with_borrow_mut(|flag| *flag = self.0.take());
            }
        }

        let _restore = Restore(CURRENT.with_borrow_mut(|flag| std::mem::replace(flag, self.0)));
        f()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_flag_is_scoped_and_nested_contexts_can_mask_it() {
        let outer = Arc::new(AtomicBool::new(false));
        let inner = Arc::new(AtomicBool::new(false));
        assert!(!cancelled());
        with_cancel(&outer, || {
            assert!(!cancelled());
            outer.store(true, Ordering::Relaxed);
            assert!(cancelled());
            with_cancel(&inner, || assert!(!cancelled()));
            assert!(cancelled());
            Context::default().run(|| assert!(!cancelled()));
            assert!(cancelled());
        });
        assert!(!cancelled());
    }

    #[test]
    fn unwind_restores_the_enclosing_flag() {
        let outer = Arc::new(AtomicBool::new(false));
        let inner = Arc::new(AtomicBool::new(true));
        with_cancel(&outer, || {
            let result = std::panic::catch_unwind(|| with_cancel(&inner, || panic!("request failed")));
            assert!(result.is_err());
            assert!(!cancelled());
            outer.store(true, Ordering::Relaxed);
            assert!(cancelled());
        });
        assert!(!cancelled());
    }

    #[test]
    fn captured_flag_is_live_on_the_worker_and_restores_its_context() {
        let request = Arc::new(AtomicBool::new(false));
        let context = with_cancel(&request, Context::current);
        request.store(true, Ordering::Relaxed);

        std::thread::spawn(move || {
            let worker = Arc::new(AtomicBool::new(false));
            with_cancel(&worker, || {
                assert!(context.clone().run(cancelled));
                assert!(!cancelled());
                let result = std::panic::catch_unwind(|| context.run(|| panic!("delegate failed")));
                assert!(result.is_err());
                assert!(!cancelled());
                worker.store(true, Ordering::Relaxed);
                assert!(cancelled());
            });
            assert!(!cancelled());
        })
        .join()
        .unwrap();
        assert!(!cancelled());
    }
}
