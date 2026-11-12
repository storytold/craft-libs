//! Count actual offset equality checks, independent of machine speed, while comparing every byte.

use std::cell::Cell;

use super::{Ifd0, Order, TAG_X_RESOLUTION, TYPE_SHORT, exif_with_resolution, put_u16, put_u32, put_u64, remove_entries};

std::thread_local! {
    // Each test thread owns its measurement; other parallel tests cannot change the budget.
    static COMPARISONS: Cell<usize> = const { Cell::new(0) };
}

#[derive(Clone, Copy)]
struct CountedOffset(usize);

impl From<usize> for CountedOffset {
    /// Wrap a real entry offset without counting construction as search work.
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl PartialEq for CountedOffset {
    /// Observe each equality actually requested by compaction, preserving its result.
    fn eq(&self, other: &Self) -> bool {
        COMPARISONS.with(|count| count.set(count.get() + 1));
        self.0 == other.0
    }
}

#[derive(Clone, Copy, Debug)]
enum RemovalPattern {
    None,
    All,
    Alternating,
    Prefix,
    Suffix,
}

impl RemovalPattern {
    /// Include exhaustion and long runs before the first removal, not just adjacent matches.
    fn removes(self, index: usize, count: usize) -> bool {
        match self {
            Self::None => false,
            Self::All => true,
            Self::Alternating => index.is_multiple_of(2),
            Self::Prefix => index < count / 2,
            Self::Suffix => index >= count / 2,
        }
    }
}

/// Build a parsed EXIF/TIFF block with malformed duplicate XResolution fields. The expected
/// block is assembled from retained raw entries, leaving their shared payload, a valid empty
/// next IFD and trailing sentinel at the original absolute offsets.
fn fixture(order: Order, big: bool, count: usize, pattern: RemovalPattern) -> (Vec<u8>, Vec<u8>, Ifd0, Vec<usize>) {
    let (header, count_bytes, entry_bytes, next_bytes, alignment) = if big { (16, 8, 20, 8, 8) } else { (8, 2, 12, 4, 2) };
    let first = header + count_bytes;
    let payload_at = first + count * entry_bytes + next_bytes;
    let payload = b"unchanged out-of-line payload\0";
    let next_at = (payload_at + payload.len()).next_multiple_of(alignment);
    let mut header_bytes = match order {
        Order::Little => b"II".to_vec(),
        Order::Big => b"MM".to_vec(),
    };
    header_bytes.extend(put_u16(order, if big { 43 } else { 42 }));
    if big {
        header_bytes.extend(put_u16(order, 8));
        header_bytes.extend(put_u16(order, 0));
        header_bytes.extend(put_u64(order, header as u64));
    } else {
        header_bytes.extend(put_u32(order, header as u32));
    }
    let mut entries = Vec::new();
    let mut kept = Vec::new();
    for index in 0..count {
        let malformed = pattern.removes(index, count);
        let tag = if malformed { TAG_X_RESOLUTION } else { 0x8000 + index as u16 };
        let mut entry = put_u16(order, tag).to_vec();
        entry.extend(put_u16(order, if malformed { TYPE_SHORT } else { 2 }));
        let values = if malformed { 1 } else { payload.len() as u64 };
        if big {
            entry.extend(put_u64(order, values));
        } else {
            entry.extend(put_u32(order, values as u32));
        }
        if malformed {
            entry.extend(put_u16(order, 300));
        } else if big {
            entry.extend(put_u64(order, payload_at as u64));
        } else {
            entry.extend(put_u32(order, payload_at as u32));
        }
        entry.resize(entry_bytes, 0);
        entries.extend_from_slice(&entry);
        if !malformed {
            kept.extend_from_slice(&entry);
        }
    }
    let next = if big { put_u64(order, next_at as u64).to_vec() } else { put_u32(order, next_at as u32).to_vec() };
    let encoded_count = |value: usize| {
        if big { put_u64(order, value as u64).to_vec() } else { put_u16(order, value as u16).to_vec() }
    };
    let mut source = header_bytes.clone();
    source.extend(encoded_count(count));
    source.extend(entries);
    source.extend_from_slice(&next);
    source.extend_from_slice(payload);
    source.resize(next_at, 0);
    source.extend(vec![0; count_bytes + next_bytes]);
    source.extend_from_slice(b"trailing sentinel");
    let mut expected = header_bytes;
    expected.extend(encoded_count(kept.len() / entry_bytes));
    expected.extend(kept);
    expected.extend(next);
    expected.resize(payload_at, 0);
    expected.extend_from_slice(&source[payload_at..]);
    let ifd = Ifd0::parse(&source).expect("bounded synthetic IFD must parse");
    let removed = ifd.fields.iter().map(|field| field.entry).collect();
    (source, expected, ifd, removed)
}

/// Thousands of malformed entries must stay within a generous linear comparison budget,
/// while retaining exact bytes and agreeing with the public rewrite route. A per-entry
/// membership scan exceeds this budget even though its cleaned output is identical.
#[test]
fn large_cleanup_has_linear_comparison_budget_and_identical_bytes() {
    for count in [4096, 8192] {
        for big in [false, true] {
            for order in [Order::Little, Order::Big] {
                for pattern in
                    [RemovalPattern::None, RemovalPattern::All, RemovalPattern::Alternating, RemovalPattern::Prefix, RemovalPattern::Suffix]
                {
                    let (source, expected, ifd, removed) = fixture(order, big, count, pattern);
                    let measured: Vec<CountedOffset> = removed.iter().copied().map(CountedOffset::from).collect();
                    let mut actual = source.clone();
                    COMPARISONS.with(|value| value.set(0));
                    remove_entries(&mut actual, &ifd, &measured);
                    let comparisons = COMPARISONS.with(Cell::get);
                    assert_eq!(actual, expected, "count={count}, big={big}, pattern={pattern:?}");
                    assert_eq!(exif_with_resolution(&source, (240.0, 240.0)).as_ref(), expected);
                    let budget = 4 * (count + removed.len() + 1);
                    assert!(
                        comparisons <= budget,
                        "offset comparison work {comparisons} exceeds linear budget {budget}: entries={count}, removed={}, big={big}, pattern={pattern:?}",
                        removed.len()
                    );
                    assert!(comparisons >= removed.len(), "measurement must observe the comparisons that find removals");
                }
            }
        }
    }
}
