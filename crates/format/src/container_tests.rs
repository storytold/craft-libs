use super::*;
use std::io::Cursor;

const LIMITS: Limits = Limits {
    max_entries: 8,
    max_expanded: || 1 << 20,
    part_limit: |name| if name.ends_with(".big") { 1 << 20 } else { 1024 },
    level: |_, _| 1,
    check_size: |length| if length <= 1 << 20 { Ok(()) } else { Err("too large".into()) },
    cancelled: None,
};

/// Parts and blobs written are read back as they were; a blob is stored once, named by its hash,
/// and checked against it.
#[test]
fn parts_and_blobs_read_back() {
    let mut writer = Writer::new(LIMITS);
    writer.part("document.json", br#"{"a":1}"#, true).unwrap();
    let first = writer.blob(b"payload", "bin", false).unwrap();
    let again = writer.blob(b"payload", "bin", true).unwrap();
    assert_eq!(first, again, "the same payload is one blob");
    assert_eq!(first, Value::String(format!("blobs/{}.bin", digest(b"payload"))));
    let bytes = writer.finish().unwrap();
    assert!(is_container(&bytes));
    let mut reader = Reader::new(Cursor::new(bytes), LIMITS).unwrap();
    assert_eq!(reader.part_names().len(), 2);
    assert!(reader.has_part("document.json") && !reader.has_part("other.json"));
    assert_eq!(&*reader.document(64).unwrap(), br#"{"a":1}"#);
    let reference = serde_json::json!({"blob": first});
    assert_eq!(reader.blob(&reference, "blob", "bin").unwrap(), b"payload");
    assert!(reader.blob(&reference, "blob", "png").is_err(), "another extension");
    assert_eq!(reader.hydrated, 7);
    reader.hydrated = (LIMITS.max_expanded)();
    assert!(reader.blob(&reference, "blob", "bin").unwrap_err().to_string().contains("resolved payloads too large"));
}

/// A part past its limit, or more entries than the limit, are refused as they are written.
#[test]
fn budgets_are_kept_when_writing() {
    let mut writer = Writer::new(LIMITS);
    assert_eq!(writer.part("small.json", &[0; 1025], false), Err(FormatError::Invalid("part too large".into())));
    assert!(writer.part("large.big", &[0; 1025], true).is_ok());
    for i in 0..7 {
        writer.part(&format!("{i}.json"), b"x", false).unwrap();
    }
    assert_eq!(writer.part("one-more.json", b"x", false), Err(FormatError::Invalid("package too large".into())));
    assert!(!writer.room(1, 0, 1 << 20), "no room for an entry past the limit");
}

/// A thumbnail is kept only for the file it shows.
#[test]
fn a_thumbnail_is_keyed_to_its_file() {
    let picture = Thumbnail { format: "app-thumbnail/1", app: "1.0", size: [2, 1], max_bytes: 1024 };
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.extend_from_slice(&2u32.to_be_bytes());
    png.extend_from_slice(&1u32.to_be_bytes());
    assert!(picture.valid(&png));
    let mut writer = Writer::new(LIMITS);
    writer.part("document.json", b"{}", false).unwrap();
    picture.write(&mut writer, &png, b"{}", 8 << 20).unwrap();
    let mut reader = Reader::new(Cursor::new(writer.finish().unwrap()), LIMITS).unwrap();
    assert_eq!(picture.read(&mut reader, 64).unwrap(), png);
    let mut writer = Writer::new(LIMITS);
    writer.part("document.json", b"{\"changed\":1}", false).unwrap();
    picture.write(&mut writer, &png, b"{}", 8 << 20).unwrap();
    let mut reader = Reader::new(Cursor::new(writer.finish().unwrap()), LIMITS).unwrap();
    assert!(picture.read(&mut reader, 64).is_err(), "stale");
}

/// A refused part is not remembered as written: once the budget is raised it is written.
#[test]
fn a_refused_part_can_be_written_later() {
    static BUDGET: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(4);
    let limits = Limits { max_expanded: || BUDGET.load(std::sync::atomic::Ordering::SeqCst), ..LIMITS };
    let mut writer = Writer::new(limits);
    assert_eq!(writer.part("a.json", b"12345678", false), Err(FormatError::Invalid("package too large".into())));
    assert!(writer.blob(b"12345678", "bin", false).is_err());
    BUDGET.store(1 << 20, std::sync::atomic::Ordering::SeqCst);
    writer.part("a.json", b"12345678", false).unwrap();
    writer.blob(b"12345678", "bin", false).unwrap();
    let reader = Reader::new(Cursor::new(writer.finish().unwrap()), limits).unwrap();
    assert_eq!(reader.part_names().len(), 2, "both parts were written on the retry");
}

/// A cancelled save writes no more parts, whether the part is small or written in pieces.
#[test]
fn cancellation_stops_the_save() {
    let cancelled = Limits { cancelled: Some(|| true), ..LIMITS };
    let mut writer = Writer::new(cancelled);
    assert_eq!(writer.part("a.json", b"a", false), Err(FormatError::Cancelled));
    // cancelled after the first piece of a long part: checks before the part, then per piece
    static CHECKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let midway = Limits {
        cancelled: Some(|| CHECKS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 2),
        max_expanded: || 1 << 22,
        part_limit: |_| 1 << 22,
        ..LIMITS
    };
    let mut writer = Writer::new(midway);
    assert_eq!(writer.part("big.big", &vec![0; PIECE + 1], true), Err(FormatError::Cancelled));
    assert_eq!(CHECKS.load(std::sync::atomic::Ordering::SeqCst), 3);
    let going = Limits { cancelled: Some(|| false), max_expanded: || 1 << 22, part_limit: |_| 1 << 22, ..LIMITS };
    let mut writer = Writer::new(going);
    writer.part("big.big", &vec![7; PIECE + 1], true).unwrap();
    let mut reader = Reader::new(Cursor::new(writer.finish().unwrap()), going).unwrap();
    assert_eq!(reader.limited_part("big.big", 1 << 22).unwrap().len(), PIECE + 1);
}

/// Room counts what is already written, and the reader keeps every part's configured limit.
#[test]
fn room_counts_written_parts_and_reads_keep_part_limits() {
    let mut writer = Writer::new(LIMITS);
    writer.part("large.big", &[0; 500 * 1024], false).unwrap();
    assert!(!writer.room(1, 200 * 1024, 600 * 1024));
    assert!(writer.room(1, 1024, 1 << 20));
    // what is written already is past the budget: not even an empty part fits
    assert!(!writer.room(1, 0, 400 * 1024));
    assert_eq!(writer.space(1, 400 * 1024), 0);
    let wide = Limits { part_limit: |_| 1 << 20, ..LIMITS };
    let mut writer = Writer::new(wide);
    writer.part("document.json", &[b' '; 2048], false).unwrap();
    let bytes = writer.finish().unwrap();
    assert!(Reader::new(Cursor::new(bytes.clone()), LIMITS).is_err(), "declared past its configured limit");
    let mut reader = Reader::new(Cursor::new(bytes), wide).unwrap();
    assert!(reader.limited_part("document.json", 4096).is_ok());
}

/// A thumbnail past a cache part's own limit is left out, never failing the save.
#[test]
fn a_thumbnail_past_its_part_limit_is_left_out() {
    let picture = Thumbnail { format: "app-thumbnail/1", app: "1.0", size: [2, 1], max_bytes: 2048 };
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.extend_from_slice(&2u32.to_be_bytes());
    png.extend_from_slice(&1u32.to_be_bytes());
    png.resize(1500, 0);
    let mut writer = Writer::new(LIMITS);
    picture.write(&mut writer, &png, b"{}", 8 << 20).unwrap();
    let reader = Reader::new(Cursor::new(writer.finish().unwrap()), LIMITS).unwrap();
    assert!(!reader.has_part("cache/thumbnail.json") && !reader.has_part("cache/thumbnail.png"));
}

/// A source that grows after its length was approved is refused.
#[test]
fn a_source_that_grows_while_read_is_refused() {
    struct Growing(Cursor<Vec<u8>>, bool);
    impl Read for Growing {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if !self.1 {
                self.1 = true;
                let end = self.0.get_ref().len();
                self.0.get_mut().extend_from_slice(&[0; 16]);
                self.0.set_position(self.0.position().min(end as u64));
            }
            self.0.read(buf)
        }
    }
    impl Seek for Growing {
        fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
            self.0.seek(to)
        }
    }
    let mut writer = Writer::new(LIMITS);
    writer.part("document.json", b"{}", false).unwrap();
    let bytes = writer.finish().unwrap();
    assert!(Reader::new(Growing(Cursor::new(bytes), false), LIMITS).err().is_some_and(|e| e.to_string().contains("changed")));
}

/// Compressed parts use the configured Deflate level.
#[test]
fn compressed_parts_use_the_configured_level() {
    // words in a varied order, so the levels compress them differently
    let words = ["alpha ", "beta ", "gamma ", "delta ", "epsilon ", "zeta ", "eta ", "theta "];
    let mut seed = 7u32;
    let text: Vec<u8> = (0..150)
        .flat_map(|_| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            words[(seed >> 16) as usize % words.len()].bytes()
        })
        .take(1000)
        .collect();
    let size = |level: fn(&str, usize) -> i64| {
        let mut writer = Writer::new(Limits { level, ..LIMITS });
        writer.part("document.json", &text, true).unwrap();
        writer.finish().unwrap().len()
    };
    assert_ne!(size(|_, _| 1), size(|_, _| 9), "the level is used");
    assert_eq!(size(|_, _| 42), size(|_, _| 9), "past 9 is clamped");
    assert_eq!(size(|_, _| -3), size(|_, _| 1), "below 1 is clamped");
}

/// An archive naming one part twice is refused.
#[test]
fn a_repeated_part_name_is_refused() {
    let mut writer = Writer::new(LIMITS);
    writer.part("document.json", b"{}", false).unwrap();
    writer.part("documenX.json", b"[]", false).unwrap();
    let mut bytes = writer.finish().unwrap();
    let (from, to) = (b"documenX.json", b"document.json");
    let mut at = 0;
    while let Some(i) = bytes[at..].windows(from.len()).position(|w| w == from) {
        bytes[at + i..at + i + to.len()].copy_from_slice(to);
        at += i + to.len();
    }
    assert!(Reader::new(Cursor::new(bytes), LIMITS).is_err());
}

/// Cached documents and blobs keep the current part limits.
#[test]
fn cached_parts_keep_the_current_part_limits() {
    let wide = Limits { part_limit: |_| 1 << 20, ..LIMITS };
    let mut writer = Writer::new(wide);
    writer.part("document.json", &[b' '; 512], false).unwrap();
    let blob = writer.blob(&[7; 512], "bin", false).unwrap();
    let bytes = writer.finish().unwrap();
    let mut reader = Reader::new(Cursor::new(bytes), wide).unwrap();
    let reference = serde_json::json!({"blob": blob});
    assert!(reader.document(4096).is_ok());
    assert!(reader.blob(&reference, "blob", "bin").is_ok());
    reader.limits = Limits { part_limit: |_| 128, ..wide };
    assert!(reader.document(4096).is_err());
    assert!(reader.limited_blob(&reference, "blob", "bin", 4096).is_err());
}

/// A raised expanded budget applies to the open reader, and what was read still counts.
#[test]
fn a_raised_expanded_budget_applies_at_once() {
    static BUDGET: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(600);
    let limits = Limits { max_expanded: || BUDGET.load(std::sync::atomic::Ordering::Relaxed), part_limit: |_| 1 << 20, ..LIMITS };
    let wide = Limits { max_expanded: || 1 << 20, ..limits };
    let mut writer = Writer::new(wide);
    writer.part("document.json", &[b' '; 256], false).unwrap();
    writer.part("other.big", &[1; 512], false).unwrap();
    let bytes = writer.finish().unwrap();
    BUDGET.store(1 << 20, std::sync::atomic::Ordering::Relaxed);
    let mut reader = Reader::new(Cursor::new(bytes), limits).unwrap();
    BUDGET.store(600, std::sync::atomic::Ordering::Relaxed);
    assert!(reader.limited_part("document.json", 4096).is_ok());
    assert!(reader.limited_part("other.big", 4096).is_err(), "256 read + 512 is past 600");
    BUDGET.store(1024, std::sync::atomic::Ordering::Relaxed);
    assert!(reader.limited_part("other.big", 4096).is_ok(), "768 fits the raised 1024");
    // a lowered budget does not forget what was read: 768 + 256 is past 1000 after a dip to 100
    BUDGET.store(100, std::sync::atomic::Ordering::Relaxed);
    assert!(reader.limited_part("document.json", 4096).is_err());
    BUDGET.store(1000, std::sync::atomic::Ordering::Relaxed);
    assert!(reader.limited_part("document.json", 4096).is_err(), "1024 is past 1000");
    BUDGET.store(1024, std::sync::atomic::Ordering::Relaxed);
    assert!(reader.limited_part("document.json", 4096).is_ok());
}

/// An entry whose local header can't be read is refused at open, not skipped.
#[test]
fn an_unreadable_entry_is_refused() {
    let mut writer = Writer::new(LIMITS);
    writer.part("document.json", b"{}", false).unwrap();
    writer.part("other.json", b"[]", false).unwrap();
    let mut bytes = writer.finish().unwrap();
    let second = bytes.windows(4).enumerate().filter(|(_, w)| *w == b"PK\x03\x04").map(|(i, _)| i).nth(1).unwrap();
    bytes[second + 2] = 0;
    assert!(Reader::new(Cursor::new(bytes), LIMITS).is_err());
}
