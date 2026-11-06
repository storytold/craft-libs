//! The `craft_host.call` import body, shared by the wasmi, wasmtime and browser engines.
//!
//! The engine hands over guest memory and the five raw `i32` arguments. This code bounds-checks
//! the ranges, runs the [`HostFn`] straight on guest memory, and packs the return value. An engine
//! that calls it has the same host call semantics as every other engine.

use craft_plugin_abi::ops::pack_call_return;
use craft_plugin_abi::{Op, Status};

use super::{HostError, HostFn, Trap, checked_range};

/// The five `i32` arguments of `craft_host.call`, read as unsigned.
pub(crate) struct RawCall {
    pub(crate) op: u32,
    pub(crate) in_ptr: u32,
    pub(crate) in_len: u32,
    pub(crate) out_ptr: u32,
    pub(crate) out_cap: u32,
}

/// A host call whose op is known and whose ranges lie inside guest memory.
pub(crate) struct Decoded {
    pub(crate) op: Op,
    pub(crate) input: std::ops::Range<usize>,
    pub(crate) output: std::ops::Range<usize>,
}

/// Decode one host call against a memory of `mem_len` bytes. `Err` carries the packed answer the
/// guest sees for an unknown op (`E_UNSUPPORTED`) or a range outside memory (`E_BAD_ARG`).
pub(crate) fn decode_call(call: &RawCall, mem_len: usize) -> Result<Decoded, i64> {
    let Some(op) = Op::from_number(call.op) else {
        return Err(pack_call_return(Status::Unsupported, 0));
    };
    let (Ok(input), Ok(output)) =
        (checked_range(call.in_ptr, call.in_len as usize, mem_len), checked_range(call.out_ptr, call.out_cap as usize, mem_len))
    else {
        return Err(pack_call_return(Status::BadArg, 0));
    };
    Ok(Decoded { op, input, output })
}

/// Turn a dispatcher result into the packed return value, or the trap that stops the guest.
/// `out_cap` is the size of the guest supplied output buffer.
pub(crate) fn pack_outcome(result: super::HostResult, out_cap: usize) -> Result<i64, Trap> {
    match result {
        Ok(n) if n <= out_cap => Ok(pack_call_return(Status::Ok, n as u32)),
        Ok(n) => Err(Trap::Other(format!("host dispatcher wrote {n} bytes into a {out_cap} byte buffer"))),
        Err(HostError::TooSmall { required }) => match u32::try_from(required) {
            Ok(r) => Ok(pack_call_return(Status::TooSmall, r)),
            Err(_) => Ok(pack_call_return(Status::Limit, 0)),
        },
        Err(HostError::Status(Status::Ok)) => Err(Trap::Other("host dispatcher returned Status::Ok as an error".into())),
        Err(HostError::Status(s)) => Ok(pack_call_return(s, 0)),
        Err(HostError::Trap(t)) => Err(t),
    }
}

/// Decode one host call, run the dispatcher on guest memory, pack the return value.
pub(crate) fn run_host_call(dispatch: &dyn HostFn, mem: &mut [u8], call: &RawCall) -> Result<i64, Trap> {
    let d = match decode_call(call, mem.len()) {
        Ok(d) => d,
        Err(packed) => return Ok(packed),
    };
    let out_cap = d.output.len();
    let result = dispatch_on(dispatch, d.op, mem, d.input, d.output)?;
    pack_outcome(result, out_cap)
}

/// Run the dispatcher with the request and the output buffer borrowed from the same memory.
/// The request is copied only when the guest made the two ranges overlap.
fn dispatch_on(
    dispatch: &dyn HostFn,
    op: Op,
    mem: &mut [u8],
    input: std::ops::Range<usize>,
    output: std::ops::Range<usize>,
) -> Result<super::HostResult, Trap> {
    let bad = || Trap::Other("host call ranges left guest memory after a bounds check".into());
    if input.is_empty() {
        let out = mem.get_mut(output).ok_or_else(bad)?;
        return Ok(dispatch(op, &[], out));
    }
    if output.is_empty() {
        let inp = mem.get(input).ok_or_else(bad)?;
        return Ok(dispatch(op, inp, &mut []));
    }
    if input.end <= output.start {
        let (low, high) = mem.split_at_mut(output.start);
        let out = high.get_mut(..output.len()).ok_or_else(bad)?;
        let inp = low.get(input).ok_or_else(bad)?;
        return Ok(dispatch(op, inp, out));
    }
    if output.end <= input.start {
        let (low, high) = mem.split_at_mut(input.start);
        let inp = high.get(..input.len()).ok_or_else(bad)?;
        let out = low.get_mut(output).ok_or_else(bad)?;
        return Ok(dispatch(op, inp, out));
    }
    let copy = mem.get(input).ok_or_else(bad)?.to_vec();
    let out = mem.get_mut(output).ok_or_else(bad)?;
    Ok(dispatch(op, &copy, out))
}
