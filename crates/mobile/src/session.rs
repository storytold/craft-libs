//! The one running app, for the C and JNI layers.
//!
//! Every call from a shell comes from the thread that owns the view (`UIKit`'s main thread; the
//! render thread on Android), so the host lives in a thread-local: no handle crosses the
//! boundary. Calls before a start or after a stop do nothing.

use std::cell::RefCell;

use crate::host::HostOps;
use crate::platform::Request;

thread_local! {
    static HOST: RefCell<Option<Box<dyn HostOps>>> = const { RefCell::new(None) };
}

pub(crate) fn install(host: Box<dyn HostOps>) {
    // Drop the old app (and its surface) before the new one takes over.
    stop();
    HOST.with(|h| *h.borrow_mut() = Some(host));
}

pub(crate) fn stop() {
    let old = HOST.with(|h| h.borrow_mut().take());
    drop(old);
}

pub(crate) fn with<R>(f: impl FnOnce(&mut dyn HostOps) -> R) -> Option<R> {
    HOST.with(|h| h.borrow_mut().as_mut().map(|host| f(host.as_mut())))
}

/// Run a frame if one is due, then hand back what the shell must do. The requests are returned,
/// not delivered here, so the shell's handlers can call back in without finding the host borrowed.
pub(crate) fn frame() -> (bool, Vec<Request>) {
    with(|h| {
        let drew = h.frame();
        (drew, h.take_requests())
    })
    .unwrap_or_default()
}
