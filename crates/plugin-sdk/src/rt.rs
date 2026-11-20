//! The runtime behind the generated exports.
//!
//! The functions without a target gate are plain safe Rust over slices, and the unit tests run
//! them natively. The wasm32 module turns the raw `i32` pointers of the ABI into those slices.
//! That is the only code in the SDK that reads guest memory by address.

use craft_plugin_abi::{ProcessDescriptor, Status};
use serde_json::Value;

use crate::entry::{Band, ControlFn, DataFn};
use craft_plugin_abi::descriptor::Kind;

/// `craft_process` return codes. 0 is success. Any other value makes the host discard the band.
pub mod process_status {
    pub const OK: i32 = 0;
    /// The handler returned an error.
    pub const HANDLER_ERROR: i32 = 1;
    /// The descriptor or the params did not decode.
    pub const BAD_DESCRIPTOR: i32 = 2;
    /// A pointer range is outside memory or two ranges overlap.
    pub const BAD_POINTER: i32 = 3;
    /// `entry` is not a data plane contribution.
    pub const NO_SUCH_ENTRY: i32 = 4;
}

fn status(s: Status) -> i64 {
    i64::from(s.code())
}

/// Run a control plane entry on request bytes. `Err` carries the negative `craft_invoke` status.
pub fn invoke_bytes(table: &[Option<ControlFn>], entry: i32, req: &[u8], blob: &[u8]) -> Result<Option<Vec<u8>>, i64> {
    let Some(adapter) = usize::try_from(entry).ok().and_then(|i| table.get(i)).copied().flatten() else {
        crate::host::error(&format!("craft_invoke: entry {entry} is not a control plane contribution"));
        return Err(status(Status::Unsupported));
    };
    let request: Value = if req.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(req).map_err(|e| {
            crate::host::error(&format!("craft_invoke: request is not JSON: {e}"));
            status(Status::BadArg)
        })?
    };
    match adapter(&request, blob) {
        Ok(None) => Ok(None),
        Ok(Some(v)) => serde_json::to_vec(&v).map(Some).map_err(|_| status(Status::BadArg)),
        Err(message) => {
            crate::host::error(&format!("craft_invoke: {message}"));
            Err(status(Status::BadArg))
        }
    }
}

/// The same call on parsed JSON, for native tests.
pub fn invoke_value(table: &[Option<ControlFn>], entry: usize, req: &Value, blob: &[u8]) -> Result<Option<Value>, String> {
    match table.get(entry).copied().flatten() {
        Some(adapter) => adapter(req, blob),
        None => Err(format!("entry {entry} is not a control plane contribution")),
    }
}

/// Bytes one band holds: `stride * h` for an image, planar f32 `channels * frames * 4` for audio.
pub fn band_len(desc: &ProcessDescriptor) -> Option<usize> {
    match desc.kind {
        Kind::Image => (desc.stride_bytes as usize).checked_mul(desc.h as usize),
        Kind::Audio => (desc.channels as usize).checked_mul(desc.frames as usize)?.checked_mul(4),
    }
}

/// Run a data plane entry on slices. Returns a `craft_process` code.
pub fn process_slices(table: &[Option<DataFn>], desc: &ProcessDescriptor, input: &[u8], output: &mut [u8], params: &[u8]) -> i32 {
    let Some(adapter) = table.get(desc.entry as usize).copied().flatten() else {
        crate::host::error(&format!("craft_process: entry {} is not a data plane contribution", desc.entry));
        return process_status::NO_SUCH_ENTRY;
    };
    let params: Value = if params.is_empty() {
        Value::Object(serde_json::Map::new())
    } else {
        match serde_json::from_slice(params) {
            Ok(v) => v,
            Err(e) => {
                crate::host::error(&format!("craft_process: params are not JSON: {e}"));
                return process_status::BAD_DESCRIPTOR;
            }
        }
    };
    let mut band = Band { desc, params, input, output };
    match adapter(&mut band) {
        Ok(()) => process_status::OK,
        Err(message) => {
            crate::host::error(&format!("craft_process: {message}"));
            process_status::HANDLER_ERROR
        }
    }
}

/// Native test helper: run one band and return the output bytes.
pub fn process_vec(table: &[Option<DataFn>], entry: usize, desc: &ProcessDescriptor, input: &[u8], params: Value) -> Result<Vec<u8>, String> {
    let len = band_len(desc).ok_or("descriptor sizes overflow")?;
    if input.len() != len {
        return Err(format!("input is {} bytes, the descriptor needs {len}", input.len()));
    }
    let adapter = table.get(entry).copied().flatten().ok_or_else(|| format!("entry {entry} is not a data plane contribution"))?;
    let mut desc = desc.clone();
    desc.entry = u32::try_from(entry).map_err(|_| "entry does not fit u32")?;
    let mut output = vec![0u8; len];
    let mut band = Band { desc: &desc, params, input, output: &mut output };
    adapter(&mut band)?;
    Ok(output)
}

#[cfg(target_arch = "wasm32")]
pub use wasm::{alloc, free, invoke, process};

#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code)]
mod wasm {
    use std::alloc::Layout;
    use std::sync::Once;

    use craft_plugin_abi::descriptor::DESCRIPTOR_V1_SIZE;
    use craft_plugin_abi::ops::pack_invoke_return;

    use super::{ControlFn, DataFn, ProcessDescriptor, Status, band_len, invoke_bytes, process_slices, process_status, status};

    const PAGE: usize = 65_536;

    fn memory_bytes() -> usize {
        core::arch::wasm32::memory_size::<0>() * PAGE
    }

    /// The byte range `ptr..ptr+len` if it lies inside linear memory. Address 0 is refused for a
    /// non-empty range, because a Rust slice may not start at a null pointer.
    fn range(ptr: u32, len: usize) -> Option<std::ops::Range<usize>> {
        if ptr == 0 && len != 0 {
            return None;
        }
        let start = ptr as usize;
        let end = start.checked_add(len)?;
        (end <= memory_bytes()).then_some(start..end)
    }

    fn overlaps(a: &std::ops::Range<usize>, b: &std::ops::Range<usize>) -> bool {
        a.start < b.end && b.start < a.end
    }

    fn slice<'a>(ptr: u32, len: usize) -> Option<&'a [u8]> {
        if len == 0 {
            return Some(&[]);
        }
        let r = range(ptr, len)?;
        // SAFETY: the range is inside linear memory. The host owns these bytes for the duration of
        // the call and does not write them while guest code runs (ABI.md sections 3 and 7).
        Some(unsafe { std::slice::from_raw_parts(r.start as *const u8, len) })
    }

    fn slice_mut<'a>(ptr: u32, len: usize) -> Option<&'a mut [u8]> {
        if len == 0 {
            return Some(&mut []);
        }
        let r = range(ptr, len)?;
        // SAFETY: as `slice`, and the caller checked that this range does not overlap any other
        // slice it holds from the same call.
        Some(unsafe { std::slice::from_raw_parts_mut(r.start as *mut u8, len) })
    }

    fn install_panic_hook() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            // wasm32-unknown-unknown aborts on panic, which the host sees as a trap. The hook is the
            // only chance to say why.
            std::panic::set_hook(Box::new(|info| crate::host::error(&format!("plugin panicked: {info}"))));
        });
    }

    /// `craft_alloc`. 0 on failure. A zero length returns a non-null dangling address.
    pub fn alloc(len: u32, align: u32) -> u32 {
        if len == 0 {
            return align.max(1);
        }
        let Ok(layout) = Layout::from_size_align(len as usize, align as usize) else {
            return 0;
        };
        // SAFETY: the layout has a non-zero size.
        let ptr = unsafe { std::alloc::alloc(layout) };
        ptr as usize as u32
    }

    /// `craft_free`. The host passes the pointer, length and alignment of a block from `craft_alloc`
    /// or of a response this plugin returned (alignment 1).
    pub fn free(ptr: u32, len: u32, align: u32) {
        if len == 0 {
            return;
        }
        let Ok(layout) = Layout::from_size_align(len as usize, align as usize) else {
            return;
        };
        // SAFETY: ABI.md section 3: `craft_free` gets a block that `craft_alloc` returned, with the
        // same length and alignment, or a response buffer this module returned from `craft_invoke`.
        unsafe { std::alloc::dealloc(ptr as usize as *mut u8, layout) }
    }

    /// `craft_invoke`.
    pub fn invoke(table: &[Option<ControlFn>], entry: i32, req_ptr: u32, req_len: u32, blob_ptr: u32, blob_len: u32) -> i64 {
        install_panic_hook();
        let (Some(req), Some(blob)) = (slice(req_ptr, req_len as usize), slice(blob_ptr, blob_len as usize)) else {
            return status(Status::BadArg);
        };
        match invoke_bytes(table, entry, req, blob) {
            Ok(None) => 0,
            Ok(Some(bytes)) if bytes.is_empty() => 0,
            Ok(Some(bytes)) => {
                let len = bytes.len();
                // A boxed slice has capacity == length and alignment 1, so `craft_free(ptr, len, 1)` is exact.
                let leaked: &'static mut [u8] = Box::leak(bytes.into_boxed_slice());
                let ptr = leaked.as_ptr() as usize as u32;
                u32::try_from(len).ok().and_then(|l| pack_invoke_return(ptr, l)).unwrap_or_else(|| status(Status::Limit))
            }
            Err(code) => code,
        }
    }

    /// `craft_process`.
    pub fn process(table: &[Option<DataFn>], desc_ptr: u32, desc_len: u32) -> i32 {
        install_panic_hook();
        let Some(raw) = slice(desc_ptr, desc_len as usize) else {
            return process_status::BAD_POINTER;
        };
        let Ok(desc) = ProcessDescriptor::decode(raw) else {
            return process_status::BAD_DESCRIPTOR;
        };
        let Some(len) = band_len(&desc) else {
            return process_status::BAD_DESCRIPTOR;
        };
        let desc_range = desc_ptr as usize..desc_ptr as usize + DESCRIPTOR_V1_SIZE.min(desc_len as usize);
        let (Some(in_r), Some(out_r), Some(par_r)) =
            (range(desc.in_ptr, len), range(desc.out_ptr, len), range(desc.params_ptr, desc.params_len as usize))
        else {
            return process_status::BAD_POINTER;
        };
        // The output is the one mutable slice, so it must not alias the input, the params, or the descriptor.
        if overlaps(&out_r, &in_r) || overlaps(&out_r, &par_r) || overlaps(&out_r, &desc_range) {
            return process_status::BAD_POINTER;
        }
        let (Some(input), Some(params), Some(output)) =
            (slice(desc.in_ptr, len), slice(desc.params_ptr, desc.params_len as usize), slice_mut(desc.out_ptr, len))
        else {
            return process_status::BAD_POINTER;
        };
        process_slices(table, &desc, input, output, params)
    }
}
