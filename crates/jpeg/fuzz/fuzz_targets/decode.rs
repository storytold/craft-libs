#![no_main]

use craft_jpeg::{Limits, Samples, decode, frame};
use libfuzzer_sys::fuzz_target;

fn fuzz_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_pixels = 1 << 20;
    limits.max_alloc = 64 << 20;
    limits
}

fuzz_target!(|data: &[u8]| {
    let limits = fuzz_limits();
    let _ = frame(data);
    // The input length picks the Adobe transform, so every color path is reached without
    // changing the stream.
    let adobe = [None, Some(0), Some(1), Some(2)][data.len() % 4];
    if let Ok(img) = decode(data, &limits, adobe) {
        let pixels = u64::from(img.width) * u64::from(img.height);
        let (len, width) = match &img.samples {
            Samples::U8(v) => (v.len() as u64, 1),
            Samples::U16(v) => (v.len() as u64, 2),
        };
        assert!(pixels <= limits.max_pixels);
        assert!(len * width <= limits.max_alloc);
        assert_eq!(len, pixels * u64::from(img.components));
    }
});
