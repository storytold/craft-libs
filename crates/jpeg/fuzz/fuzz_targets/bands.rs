#![no_main]

use craft_jpeg::{Bands, Limits};
use libfuzzer_sys::fuzz_target;

/// Limits that keep each input's work small: 1 MP and 64 MiB.
fn fuzz_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_pixels = 1 << 20;
    limits.max_alloc = 64 << 20;
    limits
}

fuzz_target!(|data: &[u8]| {
    let limits = fuzz_limits();
    // The input length picks the Adobe transform, as in the `decode` target.
    let adobe = [None, Some(0), Some(1), Some(2)][data.len() % 4];
    let Ok(mut bands) = Bands::new(data, &limits, adobe) else { return };
    assert!(bands.reader_bytes() <= limits.max_alloc);
    assert!(u64::from(bands.width()) * u64::from(bands.height()) <= limits.max_pixels);
    let mut reader = bands.reader();
    let mut in_order = Vec::new();
    loop {
        match reader.next_band() {
            Ok(Some(band)) => in_order.extend_from_slice(band.samples),
            Ok(None) => break,
            // Damage is reported by `Reader::damaged`, never as an error.
            Err(e) => panic!("a read after Bands::new accepted the stream: {e}"),
        }
    }
    assert_eq!(in_order.len(), bands.width() as usize * bands.height() as usize * usize::from(bands.components()));
    // Reading from every seek point, last first, gives the same samples.
    bands.find_seek_points();
    let points: Vec<usize> = bands.seek_points().collect();
    let mut sought = vec![0u8; in_order.len()];
    let mut reader = bands.reader();
    for (k, &p) in points.iter().enumerate().rev() {
        reader.seek(p).expect("a seek to an advertised seek point");
        for _ in p..points.get(k + 1).copied().unwrap_or(bands.bands()) {
            let band = reader.next_band().expect("a read after a seek").expect("a band after a seek");
            let at = band.index * bands.band_bytes();
            sought[at..at + band.samples.len()].copy_from_slice(band.samples);
        }
    }
    assert!(sought == in_order, "seeking changes the samples");
});
