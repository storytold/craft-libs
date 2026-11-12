#![no_main]

use libfuzzer_sys::fuzz_target;

// The doc.dat object stream and the model built from it, wrapped in a stored archive entry so
// mutations reach the tagged grammar instead of the archive's checksum.
fuzz_target!(|stream: &[u8]| {
    let _ = craft_affinity::stream::parse(stream);
    let file = craft_affinity::synth::container(&[("doc.dat", stream, craft_affinity::synth::Method::Stored)], None);
    let _ = craft_affinity::read(&file, craft_affinity::Limits { max_entry: 16 << 20, max_total: 64 << 20, max_pixel_bytes: 256 << 20 });
});
