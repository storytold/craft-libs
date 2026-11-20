//! Run plugin code in a plain `cargo test` by installing a fake host.
//!
//! ```ignore
//! let reply = |op: u32, input: &[u8], out: &mut [u8]| testing::reply(out, &json!({ "cancelled": false }));
//! testing::with_host(reply, || assert_eq!(host::cancelled(), Ok(false)));
//! ```

use craft_plugin_abi::Status;
use craft_plugin_abi::ops::pack_call_return;
use serde_json::Value;

use crate::backend::{self, FakeHost};

struct Restore(Option<FakeHost>);

impl Drop for Restore {
    fn drop(&mut self) {
        backend::replace(self.0.take());
    }
}

/// Install `host` for the current thread while `f` runs. The old host comes back afterward, also on panic.
pub fn with_host<R>(host: impl FnMut(u32, &[u8], &mut [u8]) -> i64 + 'static, f: impl FnOnce() -> R) -> R {
    let _restore = Restore(backend::replace(Some(Box::new(host))));
    f()
}

/// Write `value` as JSON into `out` the way the real host does: `OK` and the byte count, or
/// `E_TOO_SMALL` and the bytes required when `out` is too short.
pub fn reply(out: &mut [u8], value: &Value) -> i64 {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let count = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    match out.get_mut(..bytes.len()) {
        Some(dst) => {
            dst.copy_from_slice(&bytes);
            pack_call_return(Status::Ok, count)
        }
        None => pack_call_return(Status::TooSmall, count),
    }
}

/// A packed return with no body.
pub fn status(status: Status) -> i64 {
    pack_call_return(status, 0)
}
