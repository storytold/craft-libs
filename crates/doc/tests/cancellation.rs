//! Cancellation is observed inside reads, with no timing or worker-thread scheduling assumptions.

mod common;

use std::cell::Cell;

use common::*;
use craft_doc::{Error, Limits, info, info_with_cancel, outline, outline_with_cancel, renditions, renditions_with_cancel};

type Read = fn(&[u8], &Limits, &dyn Fn() -> bool) -> Result<(), Error>;

const READS: [Read; 3] = [
    |bytes, limits, cancel| info_with_cancel(bytes, limits, cancel).map(|_| ()),
    |bytes, limits, cancel| renditions_with_cancel(bytes, limits, cancel).map(|_| ()),
    |bytes, limits, cancel| outline_with_cancel(bytes, limits, cancel).map(|_| ()),
];

#[test]
fn cancellation_interrupts_detection_before_an_unrecognized_result() {
    let json = format!(r#"{{"savedBy":"{}","unrelated":0}}"#, "\\n".repeat(1 << 20));
    // Optional filename and comment fields are consumed before the decoded prefix.
    let mut header = vec![0x1f, 0x8b, 8, 24, 0, 0, 0, 0, 0, 3];
    header.extend(std::iter::repeat_n(b'x', 60_000));
    header.push(0);
    header.extend(std::iter::repeat_n(b'x', 60_000));
    header.push(0);
    header.extend(&gzip(b"unrelated")[10..]);
    let entries: Vec<_> = (0..200).map(|_| store("other", b"")).collect();
    let archive = zip(&entries);
    // The truncated bundle forces detection to walk local headers instead of the directory.
    let local = &archive[..archive.len() / 4];
    for (case, bytes) in [json.as_bytes(), header.as_slice(), archive.as_slice(), local].into_iter().enumerate() {
        for read in READS {
            let threshold = if case == 1 { 3 } else { 10 };
            let polls = Cell::new(0);
            let cancel = || {
                polls.set(polls.get() + 1);
                polls.get() == threshold
            };
            assert_eq!(read(bytes, &Limits::default(), &cancel), Err(Error::Cancelled), "detection case {case}, polls {}", polls.get());
            assert_eq!(polls.get(), threshold);
        }
    }
}

#[test]
fn reads_honor_an_initial_request_even_for_unrecognized_input() {
    for read in READS {
        assert_eq!(read(b"not a document", &Limits::default(), &|| true), Err(Error::Cancelled));
    }
}

#[test]
fn uncanceled_reads_preserve_results_and_errors() {
    let vc = vectorcraft_json(4, "");
    for file in [vc.as_bytes().to_vec(), gzip(vc.as_bytes()), pcraft(&png(1, 1, 0), &png(2, 2, 0)), ecproj(2).into_bytes(), vec![1]] {
        let limits = Limits::default();
        assert_eq!(info_with_cancel(&file, &limits, &|| false), info(&file, &limits));
        assert_eq!(renditions_with_cancel(&file, &limits, &|| false), renditions(&file, &limits));
        assert_eq!(outline_with_cancel(&file, &limits, &|| false), outline(&file, &limits));
    }
}

#[test]
fn cancellation_after_work_starts_interrupts_plain_and_gzip_json() {
    let json = vectorcraft_json(4, &format!(r#""padding":"{}""#, "x".repeat(2 << 20)));
    for file in [json.as_bytes().to_vec(), gzip(json.as_bytes())] {
        for read in READS {
            let polls = Cell::new(0);
            let cancel = || {
                polls.set(polls.get() + 1);
                polls.get() >= 20
            };
            assert_eq!(read(&file, &Limits::default(), &cancel), Err(Error::Cancelled));
            assert_eq!(polls.get(), 20, "the request is latched instead of being polled again");
        }
    }
}

#[test]
fn cancellation_interrupts_zip_manifest_reads() {
    let json = pcraft_manifest(1, "[]", &format!(r#","padding":"{}""#, "x".repeat(2 << 20)));
    for entry in [store("manifest.json", json.as_bytes()), deflated("manifest.json", json.as_bytes())] {
        let file = zip(&[entry]);
        for read in READS {
            let polls = Cell::new(0);
            let cancel = || {
                polls.set(polls.get() + 1);
                polls.get() >= 20
            };
            assert_eq!(read(&file, &Limits::default(), &cancel), Err(Error::Cancelled));
        }
    }
}

#[test]
fn cancellation_interrupts_the_effectcraft_header_parse() {
    let original = ecproj(2);
    let prefix = original.trim_end().strip_suffix('}').unwrap();
    let json = format!(r#"{prefix},"padding":"{}"}}"#, "x".repeat(2 << 20));
    // Allow the initial callback, input-size check and complete depth scan. Cancellation then
    // arrives after the header's own reader has consumed some input.
    let threshold = json.len().div_ceil(64 * 1024) + 6;
    let polls = Cell::new(0);
    let cancel = || {
        polls.set(polls.get() + 1);
        polls.get() >= threshold
    };
    assert_eq!(info_with_cancel(json.as_bytes(), &Limits::default(), &cancel), Err(Error::Cancelled));
}

#[test]
fn controlled_reads_keep_limit_and_malformed_input_errors() {
    let json = vectorcraft_json(4, r#""preview":{"mime":"image/png","data":"AQID"}"#);
    let files = [
        json.as_bytes().to_vec(),
        gzip(json.as_bytes()),
        json.as_bytes()[..json.len() - 1].to_vec(),
        vectorcraft_json(99, "").into_bytes(),
        pcraft(&png(1, 1, 0), &png(2, 2, 0)),
    ];
    for file in files {
        let mut bounds = vec![Limits::default(); 6];
        bounds[1].max_json = 16;
        bounds[2].max_entry = 0;
        bounds[3].max_total = 8;
        bounds[4].max_depth = 1;
        bounds[5].max_items = 0;
        for limits in bounds {
            assert_eq!(info_with_cancel(&file, &limits, &|| false), info(&file, &limits));
            assert_eq!(renditions_with_cancel(&file, &limits, &|| false), renditions(&file, &limits));
            assert_eq!(outline_with_cancel(&file, &limits, &|| false), outline(&file, &limits));
        }
    }
}

#[test]
fn controlled_full_composites_preserve_provenance_and_selected_reads() {
    let full = png(4, 3, 30);
    let manifest = pcraft_manifest(1, "[]", r#", "full_composite":"composite/full.png", "composite":"composite/preview.png""#);
    let file = zip(&[store("manifest.json", manifest.as_bytes()), store("composite/full.png", &full)]);
    let limits = Limits::default();
    let selected = craft_doc::rendition(&file, craft_doc::RenditionKind::Composite, &limits).unwrap().unwrap();
    let controlled = renditions_with_cancel(&file, &limits, &|| false).unwrap();
    assert_eq!(controlled, vec![selected]);
    assert_eq!(controlled[0].bytes, full);
    assert!(controlled[0].is_full_composite);
    assert_eq!(renditions_with_cancel(&file, &limits, &|| true), Err(Error::Cancelled));

    let missing_full = zip(&[store("manifest.json", manifest.as_bytes()), store("composite/preview.png", &full)]);
    assert!(matches!(renditions_with_cancel(&missing_full, &limits, &|| false), Err(Error::Malformed(_))));
}
