//! The one place that touches the `craft_host.call` import.
//!
//! On wasm32 this is the real import. On other targets a test installs a fake host with
//! [`crate::testing::with_host`]. Without one, [`call`] returns `None` and the typed helpers report
//! [`crate::HostError::NoHost`]. There is no default answer, so a native run never pretends to be a host.

/// Call op `op` with `input`. The host writes up to `out.len()` bytes into `out`. Returns the packed
/// `status | count << 32` value, or `None` when there is no host.
pub fn call(op: u32, input: &[u8], out: &mut [u8]) -> Option<i64> {
    imp::call(op, input, out)
}

#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code)]
mod imp {
    #[link(wasm_import_module = "craft_host")]
    unsafe extern "C" {
        #[link_name = "call"]
        fn craft_host_call(op: i32, in_ptr: *const u8, in_len: usize, out_ptr: *mut u8, out_cap: usize) -> i64;
    }

    pub fn call(op: u32, input: &[u8], out: &mut [u8]) -> Option<i64> {
        // SAFETY: both pointers and lengths come from live slices. The host reads `input` and writes
        // at most `out.len()` bytes into `out`. ABI.md section 4: it never calls back into the guest.
        Some(unsafe { craft_host_call(op as i32, input.as_ptr(), input.len(), out.as_mut_ptr(), out.len()) })
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::cell::RefCell;

    /// A fake host: `(op number, request bytes, output buffer) -> packed return`.
    pub type FakeHost = Box<dyn FnMut(u32, &[u8], &mut [u8]) -> i64>;

    thread_local! {
        static HOST: RefCell<Option<FakeHost>> = const { RefCell::new(None) };
    }

    pub fn call(op: u32, input: &[u8], out: &mut [u8]) -> Option<i64> {
        HOST.with(|slot| {
            // A fake host that calls back into the SDK would borrow twice. That is a test bug and reads as "no host".
            let mut slot = slot.try_borrow_mut().ok()?;
            let host = slot.as_mut()?;
            Some(host(op, input, out))
        })
    }

    /// Swap the fake host and return the old one.
    pub fn replace(host: Option<FakeHost>) -> Option<FakeHost> {
        HOST.with(|slot| std::mem::replace(&mut *slot.borrow_mut(), host))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use imp::{FakeHost, replace};
