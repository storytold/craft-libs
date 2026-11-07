use super::*;
use alloc::{vec, vec::Vec};
use core::sync::atomic::Ordering;

#[test]
fn a_stream_without_decode_params_keeps_its_open_control() {
    use crate::reader::{Reader, ReaderContext, ReaderExt};
    use core::sync::atomic::{AtomicBool, AtomicUsize};
    let armed = alloc::sync::Arc::new(AtomicBool::new(false));
    let polls = alloc::sync::Arc::new(AtomicUsize::new(0));
    let flag = armed.clone();
    let counter = polls.clone();
    let cancel = crate::Cancellation::new(move || flag.load(Ordering::Relaxed) && counter.fetch_add(1, Ordering::Relaxed) + 1 == 4);
    let control = crate::cancel::Control::new(&cancel);
    let ctx = ReaderContext::dummy_with_control(crate::DecodeLimits::default(), Some(control));
    let mut raw = [129, 0].repeat(30_000);
    raw.push(128);
    let mut bytes = alloc::format!("<< /Length {} /Filter /RunLengthDecode >> stream\n", raw.len()).into_bytes();
    bytes.extend(raw);
    bytes.extend_from_slice(b"\nendstream");
    let stream = Reader::new(&bytes).read_with_context::<crate::object::Stream<'_>>(&ctx).unwrap();
    armed.store(true, Ordering::Relaxed);
    assert!(stream.decoded().is_err());
    assert_eq!(polls.load(Ordering::Relaxed), 4);
}

#[test]
fn ascii_decoders_stop_during_long_scans() {
    let mut ascii85 = vec![b' '; 2 << 20];
    ascii85.extend_from_slice(b"z~>");
    assert_eq!(ascii_85::decode(&ascii85, usize::MAX), Some(vec![0; 4]));
    let (control, polls) = crate::cancel::stop_at(3);
    assert!(ascii_85::decode_controlled(&ascii85, usize::MAX, Some(&control)).is_none());
    assert_eq!(polls.load(Ordering::Relaxed), 3);

    let asciihex = vec![b'0'; 2 << 20];
    let (control, polls) = crate::cancel::stop_at(3);
    assert!(ascii_hex::decode_into_controlled::<Vec<u8>>(&asciihex, Some(&control)).is_none());
    assert_eq!(polls.load(Ordering::Relaxed), 3);
}

#[test]
fn run_length_stops_before_finishing_its_expansion() {
    let mut input = [129, 0].repeat(30_000);
    input.push(128);
    assert_eq!(run_length::decode(&input, usize::MAX).map(|v| v.len()), Some(128 * 30_000));
    let (control, polls) = crate::cancel::stop_at(4);
    assert!(run_length::decode_controlled(&input, usize::MAX, Some(&control)).is_none());
    assert_eq!(polls.load(Ordering::Relaxed), 4);
}

#[test]
fn flate_stops_inside_stream_decoding() {
    // Valid zlib data containing forty stored blocks of zeroes, including its Adler32.
    let mut input = vec![0x78, 0x01];
    for i in 0..40 {
        input.extend_from_slice(&[u8::from(i == 39), 0xff, 0xff, 0, 0]);
        input.extend(core::iter::repeat_n(0, 65535));
    }
    let length = 40 * 65535;
    input.extend_from_slice(&(((length % 65521) as u32) << 16 | 1).to_be_bytes());
    assert_eq!(lzw_flate::flate::decode(&input, &Dict::empty(), usize::MAX).map(|v| v.len()), Some(length));
    let (control, polls) = crate::cancel::stop_at(3);
    assert!(lzw_flate::flate::decode_controlled(&input, &Dict::empty(), usize::MAX, Some(&control)).is_none());
    assert_eq!(polls.load(Ordering::Relaxed), 3);
}

#[test]
fn lzw_without_output_still_observes_cancellation() {
    // Eight consecutive nine-bit clear-table codes, repeated without producing a pixel.
    let input = [0x80, 0x40, 0x20, 0x10, 8, 4, 2, 1, 0].repeat(65536);
    let (control, polls) = crate::cancel::stop_at(4);
    assert!(lzw_flate::lzw::decode_controlled(&input, &Dict::empty(), usize::MAX, Some(&control)).is_none());
    assert_eq!(polls.load(Ordering::Relaxed), 4);
}
