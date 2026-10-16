//! Small synthetic cases for document-local ordinary stream expansion limits.
use super::*;
use crate::Pdf;
use crate::filter::MAX_DECODED_STREAM;
use crate::object::stream::ImageDecodeParams;
use alloc::collections::BTreeMap;
use alloc::format;

fn run_length(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let mut run = 1;
        while run < 128 && at + run < data.len() && data[at + run] == data[at] {
            run += 1;
        }
        if run >= 3 {
            out.extend([u8::try_from(257 - run).unwrap(), data[at]]);
            at += run;
        } else {
            let start = at;
            at += run;
            while at < data.len() && at - start < 128 {
                if at + 2 < data.len() && data[at] == data[at + 1] && data[at] == data[at + 2] {
                    break;
                }
                at += 1;
            }
            out.push(u8::try_from(at - start - 1).unwrap());
            out.extend_from_slice(&data[start..at]);
        }
    }
    out.push(128);
    out
}

fn stream(data: &[u8], extra: &str) -> Vec<u8> {
    let encoded = run_length(data);
    let mut out = format!("<< /Length {} /Filter /RunLengthDecode {extra} >>\nstream\n", encoded.len()).into_bytes();
    out.extend_from_slice(&encoded);
    out.extend_from_slice(b"\nendstream");
    out
}

fn object(out: &mut Vec<u8>, offsets: &mut BTreeMap<u32, usize>, n: u32, bytes: &[u8]) {
    offsets.insert(n, out.len());
    out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
    out.extend_from_slice(bytes);
    out.extend_from_slice(b"\nendobj\n");
}

fn base() -> (Vec<u8>, BTreeMap<u32, usize>) {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = BTreeMap::new();
    object(&mut out, &mut offsets, 1, b"<< /Type /Catalog /Pages 2 0 R >>");
    object(&mut out, &mut offsets, 2, b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>");
    object(&mut out, &mut offsets, 3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>");
    (out, offsets)
}

fn table(out: &mut Vec<u8>, offsets: &BTreeMap<u32, usize>, extra: &str) -> usize {
    let start = out.len();
    let size = offsets.keys().max().copied().unwrap_or(0) + 1;
    out.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
    for n in 0..size {
        match offsets.get(&n) {
            Some(at) => out.extend_from_slice(format!("{at:010} 00000 n \n").as_bytes()),
            None => out.extend_from_slice(b"0000000000 65535 f \n"),
        }
    }
    out.extend_from_slice(format!("trailer\n<< /Root 1 0 R /Size {size} {extra} >>\nstartxref\n{start}\n%%EOF\n").as_bytes());
    start
}

fn ordinary_stream_pdf() -> Vec<u8> {
    let (mut out, mut offsets) = base();
    object(&mut out, &mut offsets, 4, &stream(&vec![b'A'; 512], ""));
    table(&mut out, &offsets, "");
    out
}

fn object_stream_pdf(repair: bool) -> (Vec<u8>, usize, BTreeMap<u32, usize>) {
    let (mut out, mut offsets) = base();
    let header = b"40 0 41 1024 ";
    let mut payload = header.to_vec();
    payload.extend_from_slice(b"<< /Marker 123 >>");
    payload.resize(header.len() + 1024, b' ');
    payload.extend_from_slice(b"<< /Marker 456 >>");
    object(&mut out, &mut offsets, 10, &stream(&payload, &format!("/Type /ObjStm /N 2 /First {}", header.len())));
    if repair {
        // An invalid normal reference forces lazy repair to rediscover object 41 in its stream.
        offsets.insert(41, 0);
        let start = table(&mut out, &offsets, "");
        return (out, start, offsets);
    }
    let start = out.len();
    offsets.insert(11, start);
    let mut rows = Vec::new();
    for n in 0..42 {
        let (kind, a, b): (u8, u32, u16) = match n {
            40 => (2, 10, 0),
            41 => (2, 10, 1),
            n => offsets.get(&n).map_or((0, 0, 65535), |at| (1, u32::try_from(*at).unwrap(), 0)),
        };
        rows.push(kind);
        rows.extend_from_slice(&a.to_be_bytes());
        rows.extend_from_slice(&b.to_be_bytes());
    }
    object(&mut out, &mut offsets, 11, &stream(&rows, "/Type /XRef /Size 42 /Root 1 0 R /W [1 4 2]"));
    out.extend_from_slice(format!("startxref\n{start}\n%%EOF\n").as_bytes());
    (out, start, offsets)
}

fn marker(pdf: &Pdf, n: i32) -> Option<i32> {
    pdf.xref().get::<Dict<'_>>(ObjectIdentifier::new(n, 0))?.get::<i32>(b"Marker")
}

#[test]
fn decode_limits_default_and_lowering_are_bounded() {
    assert_eq!(DecodeLimits::default().max_decoded_stream(), MAX_DECODED_STREAM);
    let limits = DecodeLimits::default().with_max_decoded_stream(32);
    assert_eq!(limits.with_max_decoded_stream(usize::MAX), limits);
    assert_eq!(limits.with_max_decoded_stream(0).max_decoded_stream(), 0);
}

#[test]
fn ordinary_stream_expansion_is_document_local_and_keeps_caller_budget() {
    let bytes = ordinary_stream_pdf();
    let low = Pdf::new_with_limits(bytes.clone(), DecodeLimits::default().with_max_decoded_stream(32)).unwrap();
    let normal = Pdf::new(bytes).unwrap();
    let low_stream = low.xref().get::<Stream<'_>>(ObjectIdentifier::new(4, 0)).unwrap();
    let normal_stream = normal.xref().get::<Stream<'_>>(ObjectIdentifier::new(4, 0)).unwrap();
    assert!(low_stream.raw_data().len() < 32, "the compressed input itself fits");
    assert_eq!(low_stream.decoded().unwrap().as_ref(), &[b'A'; 32]);
    assert_eq!(normal_stream.decoded().unwrap().as_ref(), &[b'A'; 512]);
    assert_eq!(low_stream.decoded_within(7).unwrap().len(), 7);
    assert_eq!(low_stream.decoded_raw_within(low_stream.raw_data(), 7).unwrap().len(), 7);
    // A second document or a clone of its xref cannot mutate the first document's limit.
    assert_eq!(low.decode_limits().max_decoded_stream(), 32);
    assert_eq!(normal.decode_limits(), DecodeLimits::default());
    assert_eq!(low.xref().clone().decode_limits(), low.decode_limits());
}

#[test]
fn final_known_size_images_keep_their_distinct_pixel_budget() {
    let limits = DecodeLimits::default().with_max_decoded_stream(32);
    let params = ImageDecodeParams { width: 64, height: 1, bpc: Some(8), num_components: Some(1), ..Default::default() };
    assert_eq!(crate::filter::decode_limit(&params, false, limits), 32);
    assert_eq!(crate::filter::decode_limit(&params, true, limits), 128);
    assert_eq!(crate::filter::decode_limit(&ImageDecodeParams::default(), true, limits), 32);
}

#[test]
fn object_stream_limit_applies_after_successful_root_xref() {
    let (bytes, _, _) = object_stream_pdf(false);
    // 512 permits the full 294-byte xref stream but cuts the 1052-byte object stream.
    let limits = DecodeLimits::default().with_max_decoded_stream(512);
    assert!(root_xref(bytes.clone().into(), b"", limits).is_ok());
    let low = Pdf::new_with_limits(bytes.clone(), limits).unwrap();
    let normal = Pdf::new(bytes).unwrap();
    assert_eq!(marker(&normal, 41), Some(456));
    assert_eq!(marker(&low, 40), Some(123));
    assert_eq!(marker(&low, 41), None);
}

#[test]
fn root_prev_and_hybrid_xref_streams_use_the_limit_before_xref_exists() {
    let (bytes, stream_at, offsets) = object_stream_pdf(false);
    let limits = DecodeLimits::default().with_max_decoded_stream(32);
    for via in ["root", "Prev", "XRefStm"] {
        let mut input = bytes.clone();
        if via != "root" {
            table(&mut input, &offsets, &format!("/{via} {stream_at}"));
        }
        assert!(root_xref(input.clone().into(), b"", DecodeLimits::default()).is_ok(), "{via}");
        assert!(root_xref(input.clone().into(), b"", limits).is_err(), "{via}: root decode escaped limit");
        // Best-effort open can rebuild ordinary offsets, but must retain the same limit.
        let repaired = Pdf::new_with_limits(input, limits).unwrap();
        assert_eq!(repaired.decode_limits(), limits);
        assert_eq!(marker(&repaired, 41), None, "{via}: fallback escaped limit");
    }
}

#[test]
fn lazy_xref_repair_keeps_the_document_limit() {
    let (bytes, _, _) = object_stream_pdf(true);
    let limits = DecodeLimits::default().with_max_decoded_stream(512);
    let low = Pdf::new_with_limits(bytes.clone(), limits).unwrap();
    let normal = Pdf::new(bytes).unwrap();
    assert_eq!(marker(&normal, 41), Some(456));
    assert_eq!(marker(&low, 41), None);
    low.xref().repair();
    assert_eq!(marker(&low, 41), None);
    assert_eq!(low.decode_limits(), limits);
}
