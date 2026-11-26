//! Locks that keep the tests of one process out of a section at the same time.
//!
//! `cargo test` runs a binary's tests on parallel threads. A test that must not overlap another
//! one takes a [`TestLock`] for its whole run:
//!
//! ```
//! use craft_testkit::serial::TestLock;
//!
//! static FILES: TestLock = TestLock::new();
//!
//! let _files = FILES.hold();
//! // The rest of the test runs while no other thread holds `FILES`.
//! ```
//!
//! The thread that holds a lock can take it again. Each [`TestLock::hold`] returns a guard, and the
//! lock is free again when the last of that thread's guards is dropped, in any order. A test that
//! panics while it holds a lock releases the lock as its guards unwind, so the next test is not
//! blocked. A lock works inside one process only: under nextest each test is its own process, and
//! the lock is never contended. Waiting threads take the lock in no particular order.
//!
//! [`gpu_lock`] is the one lock of the process for tests that build a GPU harness or a wgpu device.
//! Several graphics drivers crash when one process drives wgpu devices from more than one thread
//! at once. Every such test in a process must take this lock. Take it on the test's first line (`let _gpu = gpu_lock();`) so it is released last.
//! [`hold_gpu_lock`] takes the same lock and keeps it until the calling thread ends.

use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, ThreadId};

/// Which thread holds a [`TestLock`], and how many of its guards are alive.
#[derive(Debug)]
struct State {
    owner: Option<ThreadId>,
    depth: usize,
    /// Threads blocked in [`TestLock::hold`], so a test can tell that a thread is waiting.
    #[cfg(test)]
    waiting: usize,
}

/// A lock for tests that one thread holds at a time and that the holding thread can take again.
///
/// [`TestLock::new`] is `const`, so a lock can be a `static`. A panic while the lock is held does
/// not poison it.
#[derive(Debug)]
pub struct TestLock {
    state: Mutex<State>,
    free: Condvar,
}

impl TestLock {
    /// A lock that no thread holds.
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(State {
                owner: None,
                depth: 0,
                #[cfg(test)]
                waiting: 0,
            }),
            free: Condvar::new(),
        }
    }

    /// Waits until no other thread holds the lock, then holds it until the returned guard and every
    /// other guard of this thread are dropped. A thread that already holds the lock gets another
    /// guard at once.
    ///
    /// A thread that keeps `usize::MAX` guards alive at once (for example by forgetting them) blocks
    /// in its next call instead of letting the count wrap.
    pub fn hold(&self) -> TestLockGuard<'_> {
        let me = thread::current().id();
        let mut state = self.lock_state();
        loop {
            match state.owner {
                None => {
                    state.owner = Some(me);
                    state.depth = 1;
                    break;
                }
                Some(owner) if owner == me && state.depth < usize::MAX => {
                    state.depth += 1;
                    break;
                }
                Some(_) => {
                    #[cfg(test)]
                    {
                        state.waiting += 1;
                    }
                    state = self.free.wait(state).unwrap_or_else(PoisonError::into_inner);
                    #[cfg(test)]
                    {
                        state.waiting -= 1;
                    }
                }
            }
        }
        drop(state);
        TestLockGuard { lock: self, not_send: PhantomData }
    }

    fn lock_state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for TestLock {
    fn default() -> Self {
        Self::new()
    }
}

/// One hold of a [`TestLock`]. The lock is released when the last guard of the holding thread is
/// dropped. A guard is not `Send`: it is dropped on the thread that took it.
#[derive(Debug)]
#[must_use = "the lock is released when the guard is dropped"]
pub struct TestLockGuard<'a> {
    lock: &'a TestLock,
    not_send: PhantomData<Rc<()>>,
}

impl Drop for TestLockGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.lock.lock_state();
        state.depth = state.depth.saturating_sub(1);
        if state.depth == 0 {
            state.owner = None;
            drop(state);
            self.lock.free.notify_one();
        }
    }
}

static GPU_LOCK: TestLock = TestLock::new();

thread_local! {
    static HELD_GPU: RefCell<Option<TestLockGuard<'static>>> = const { RefCell::new(None) };
}

/// Holds the process's GPU lock for as long as the guard lives. Take it before the test creates a
/// GPU harness or device, and keep it for the whole test, harness included.
pub fn gpu_lock() -> TestLockGuard<'static> {
    GPU_LOCK.hold()
}

/// Holds the process's GPU lock until the calling thread ends. Calling it again on the same thread
/// does nothing more. Called on a long-lived thread, such as a harness's main thread, it keeps
/// every other thread out of [`gpu_lock`] until that thread ends.
///
/// The lock is released by the thread's local-variable destructors, so it is not released when the
/// process exits or aborts while the thread runs. While the thread's locals are being destroyed,
/// the call returns without taking the lock.
pub fn hold_gpu_lock() {
    let _ = HELD_GPU.try_with(|held| {
        if let Ok(mut held) = held.try_borrow_mut()
            && held.is_none()
        {
            *held = Some(gpu_lock());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    fn owner_and_depth(lock: &TestLock) -> (Option<ThreadId>, usize) {
        let state = lock.lock_state();
        (state.owner, state.depth)
    }

    /// Yields until `ready` is true of the lock's state.
    fn until(lock: &TestLock, ready: impl Fn(&State) -> bool) {
        while !ready(&lock.lock_state()) {
            thread::yield_now();
        }
    }

    #[test]
    fn a_panicked_holder_does_not_block_the_next_test() {
        let holder = thread::spawn(|| {
            let _gpu = gpu_lock();
            panic!("a GPU test failed");
        });
        let id = holder.thread().id();
        let _ = holder.join();
        assert_ne!(owner_and_depth(&GPU_LOCK).0, Some(id));
        drop(gpu_lock());
    }

    #[test]
    fn poisoned_state_recovers() {
        let lock = &TestLock::new();
        let poisoned = thread::scope(|s| {
            s.spawn(|| {
                let _state = lock.state.lock();
                panic!("poison the state");
            })
            .join()
        });
        assert!(poisoned.is_err());
        assert!(lock.state.is_poisoned());
        let outer = lock.hold();
        thread::scope(|s| {
            let (tx, rx) = mpsc::channel();
            s.spawn(move || {
                let _held = lock.hold();
                tx.send(()).unwrap();
            });
            until(lock, |state| state.waiting == 1);
            drop(outer);
            rx.recv().unwrap();
        });
        assert_eq!(owner_and_depth(lock), (None, 0));
    }

    #[test]
    fn holders_take_turns() {
        let lock = &TestLock::new();
        let inside = &AtomicUsize::new(0);
        let max = &AtomicUsize::new(0);
        let first = lock.hold();
        thread::scope(|s| {
            let (entered_tx, entered_rx) = mpsc::channel();
            let releases: Vec<_> = (0..8)
                .map(|i| {
                    let (release_tx, release_rx) = mpsc::channel::<()>();
                    let entered_tx = entered_tx.clone();
                    s.spawn(move || {
                        let _held = lock.hold();
                        let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                        max.fetch_max(now, Ordering::SeqCst);
                        entered_tx.send(i).unwrap();
                        release_rx.recv().unwrap();
                        inside.fetch_sub(1, Ordering::SeqCst);
                    });
                    release_tx
                })
                .collect();
            until(lock, |state| state.waiting + inside.load(Ordering::SeqCst) == 8);
            drop(first);
            for _ in 0..8 {
                let i: usize = entered_rx.recv().unwrap();
                releases[i].send(()).unwrap();
            }
        });
        assert_eq!(max.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn static_lock_is_const_and_reentrant() {
        static LOCK: TestLock = TestLock::new();
        let outer = LOCK.hold();
        let inner = LOCK.hold();
        assert_eq!(owner_and_depth(&LOCK), (Some(thread::current().id()), 2));
        drop(inner);
        drop(outer);
        assert_eq!(owner_and_depth(&LOCK), (None, 0));
    }

    #[test]
    fn nested_guards_drop_in_either_order() {
        for outer_first in [true, false] {
            let lock = &TestLock::new();
            let outer = lock.hold();
            let inner = lock.hold();
            thread::scope(|s| {
                let (tx, rx) = mpsc::channel();
                s.spawn(move || {
                    let _held = lock.hold();
                    tx.send(()).unwrap();
                });
                until(lock, |state| state.waiting == 1);
                let (first, second) = if outer_first { (outer, inner) } else { (inner, outer) };
                drop(first);
                assert_eq!(owner_and_depth(lock), (Some(thread::current().id()), 1), "outer_first: {outer_first}");
                drop(second);
                rx.recv().unwrap();
            });
            assert_eq!(owner_and_depth(lock), (None, 0));
        }
    }

    #[test]
    fn gpu_thread_hold_is_idempotent_and_releases_at_exit() {
        let holder = thread::spawn(|| {
            let early = gpu_lock();
            hold_gpu_lock();
            hold_gpu_lock();
            let with_early = owner_and_depth(&GPU_LOCK);
            drop(early);
            let held = owner_and_depth(&GPU_LOCK);
            let late = gpu_lock();
            let with_late = owner_and_depth(&GPU_LOCK);
            drop(late);
            let held_again = owner_and_depth(&GPU_LOCK);
            [with_early, held, with_late, held_again]
        });
        let id = Some(holder.thread().id());
        let depths = holder.join().unwrap();
        assert_eq!(depths, [(id, 2), (id, 1), (id, 2), (id, 1)]);
        assert_ne!(owner_and_depth(&GPU_LOCK).0, id);
        drop(gpu_lock());
    }

    #[test]
    fn different_locks_are_independent() {
        let a = TestLock::new();
        let b = TestLock::new();
        let _a = a.hold();
        assert_eq!(owner_and_depth(&b), (None, 0));
        thread::scope(|s| s.spawn(|| drop(b.hold())).join()).unwrap();
        assert_eq!(owner_and_depth(&b), (None, 0));
    }

    trait AmbiguousIfSend<A> {
        fn some_item() {}
    }
    impl<T: ?Sized> AmbiguousIfSend<()> for T {}
    impl<T: ?Sized + Send> AmbiguousIfSend<u8> for T {}

    #[test]
    fn guard_is_not_send() {
        // Compiles only while `TestLockGuard` is not `Send`: for a `Send` guard both impls apply and
        // the call is ambiguous.
        let _ = <TestLockGuard<'static> as AmbiguousIfSend<_>>::some_item;
    }
}
