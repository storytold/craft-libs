//! Fuzz the `.atn` actions parser: any input must return `Ok` or `Err`, never panic, and
//! anything that parses must survive writing, reparsing and stable canonical writing.
#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../../tests/it/common/atn_roundtrip.rs"]
mod atn_roundtrip;

fuzz_target!(|data: &[u8]| {
    atn_roundtrip::check(data);
});
