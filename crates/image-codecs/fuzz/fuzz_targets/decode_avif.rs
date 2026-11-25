#![no_main]

use libfuzzer_sys::fuzz_target;
use craft_image_codecs::{decode_as_with, probe_size, DecodeOptions, Format, Limits};

// The AVIF box reader and craft-video's AV1 decoder parse untrusted files: run with `-timeout=10`
// so a hang is reported, not just a panic. The codec turns an AV1 decoder panic into an error, so
// only the box reader's own panics would reach the fuzzer. Both orientation paths are covered.
fuzz_target!(|data: &[u8]| {
    let keep_orientation = data.first().is_some_and(|b| b & 1 == 1);
    let limits = Limits { max_width: 4096, max_height: 4096, max_pixels: 1 << 22, max_alloc: 256 << 20 };
    let _ = probe_size(data);
    let _ = decode_as_with(Format::Avif, data, &DecodeOptions { limits, keep_orientation });
});
