//! Fuzz the `.asl` style library parser: any input must return `Ok` or `Err`, never panic, and
//! anything that parses must write back and parse to the same number of styles.
#![no_main]

use libfuzzer_sys::fuzz_target;
use craft_psd::asl;

fuzz_target!(|data: &[u8]| {
    if let Ok(f) = asl::parse(data)
        && let Ok(bytes) = asl::write(&f)
    {
        let back = asl::parse(&bytes).expect("a written library parses");
        assert_eq!(back.styles.len(), f.styles.len());
    }
});
