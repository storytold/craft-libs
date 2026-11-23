//! Opt-in, bounded access to inert source metadata.
//!
//! Records are borrowed, ordered and never deduplicated. A partial scan explicitly
//! describes why it stopped: a prefix cannot establish a final duplicate value.
//! This does not interpret profiles, text, paths, resource names or application policy.

use crate::{Cursor, Error, MAX_METADATA_BYTES, MAX_PAYLOAD, MAX_STRING, Result};

pub mod metadata;
pub mod paths;
pub mod text;

/// Limits for one property or parasite list.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub records: usize,
    pub string_bytes: usize,
    pub payload_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { records: 4096, string_bytes: MAX_STRING, payload_bytes: MAX_PAYLOAD }
    }
}

/// Separate cumulative source-work and retained-record allowances.
///
/// Reuse this across owners and repeated scans. Attempted work is never refunded,
/// including malformed records. Retention charges cover only the returned vector,
/// conservatively two slots per reserved vector slot; borrowed names and payloads
/// are not copied. Reservations grow geometrically to avoid quadratic copying.
#[derive(Debug)]
pub struct Budget {
    work_left: usize,
    retained_left: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self::new(128 << 20, MAX_METADATA_BYTES)
    }
}

impl Budget {
    pub fn new(work_bytes: usize, retained_bytes: usize) -> Self {
        Self { work_left: work_bytes, retained_left: retained_bytes }
    }

    pub fn work_remaining(&self) -> usize {
        self.work_left
    }

    pub fn retained_remaining(&self) -> usize {
        self.retained_left
    }

    fn work(&mut self, bytes: usize) -> std::result::Result<(), StopReason> {
        self.work_left = self.work_left.checked_sub(bytes).ok_or(StopReason::WorkLimit)?;
        Ok(())
    }

    fn retain<T>(&mut self, records: &mut Vec<T>, value: T) -> std::result::Result<(), StopReason> {
        if records.len() == records.capacity() {
            let slot = std::mem::size_of::<T>().checked_mul(2).filter(|slot| *slot != 0).ok_or(StopReason::RetainedLimit)?;
            let additional = records.capacity().max(1).min(self.retained_left / slot);
            if additional == 0 {
                return Err(StopReason::RetainedLimit);
            }
            let bytes = additional.checked_mul(slot).ok_or(StopReason::RetainedLimit)?;
            // Admit the whole requested growth before allocating, rather than
            // letting push choose an uncharged capacity or copying every record.
            self.retained_left -= bytes;
            if records.try_reserve_exact(additional).is_err() {
                self.retained_left += bytes;
                return Err(StopReason::AllocationFailure);
            }
        }
        records.push(value);
        Ok(())
    }
}

/// Why an optional source scan stopped. None of these implies pixel corruption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Malformed,
    Unsupported,
    RecordLimit,
    StringLimit,
    PayloadLimit,
    WorkLimit,
    RetainedLimit,
    AllocationFailure,
}

/// One bounded diagnostic; scans never allocate diagnostic strings or lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Diagnostic {
    /// Absolute source byte offset at which the unretained record starts.
    pub offset: usize,
    /// Source ordinal of the unretained record, after the usable prefix.
    pub ordinal: usize,
    pub reason: StopReason,
}

/// An ordered prefix and explicit completeness information.
#[derive(Debug)]
pub struct Scan<T> {
    pub records: Vec<T>,
    /// None means the complete list was read, including a property terminator.
    pub stopped: Option<Diagnostic>,
    /// End of the complete list, or start of the first unretained record.
    pub end_offset: usize,
}

impl<T> Scan<T> {
    fn complete(records: Vec<T>, end_offset: usize) -> Self {
        Self { records, stopped: None, end_offset }
    }

    fn stopped(records: Vec<T>, offset: usize, reason: StopReason) -> Self {
        let ordinal = records.len();
        Self { records, stopped: Some(Diagnostic { offset, ordinal, reason }), end_offset: offset }
    }
}

/// A source property, including its original header and its measured payload.
#[derive(Debug, Clone, Copy)]
pub struct Property<'a> {
    pub offset: usize,
    pub kind: u32,
    pub declared_length: u32,
    pub payload: &'a [u8],
    pub raw: &'a [u8],
}

/// A source parasite. Names and payloads remain uninterpreted bytes, including
/// invalid UTF-8; callers can apply the encoding rules for the field they use.
#[derive(Debug, Clone, Copy)]
pub struct Parasite<'a> {
    pub offset: usize,
    pub name: &'a [u8],
    pub flags: u32,
    pub payload: &'a [u8],
    pub raw: &'a [u8],
}

fn check(keep_going: &dyn Fn() -> bool) -> Result<()> {
    if keep_going() { Ok(()) } else { Err(Error::Cancelled) }
}

pub(super) fn layer_properties<'a>(
    bytes: &'a [u8],
    offset: usize,
    limits: Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
) -> Result<Scan<Property<'a>>> {
    check(keep_going)?;
    let mut cursor = Cursor { b: bytes, pos: offset, wide: false };
    let prefix = (|| {
        budget.work(16)?;
        cursor.take(12).map_err(|_| StopReason::Malformed)?;
        let length = cursor.u32().map_err(|_| StopReason::Malformed)? as usize;
        if length > limits.string_bytes {
            return Err(StopReason::StringLimit);
        }
        budget.work(length)?;
        cursor.take(length).map_err(|_| StopReason::Malformed)?;
        Ok(())
    })();
    if let Err(reason) = prefix {
        return Ok(Scan::stopped(Vec::new(), offset, reason));
    }
    properties(bytes, cursor.pos, limits, budget, keep_going)
}

/// Read a property list at an absolute offset in the inflated XCF source.
///
/// Known incorrect length words use the same measured framing as the legacy
/// reader. Truncated optional data returns a disclosed prefix; cancellation is
/// always an error, including for empty lists and exhausted budgets. Use
/// [`properties_with_version`] when the file version is available; this legacy
/// versionless entry point retains its historical unit-framing fallback.
pub fn properties<'a>(
    bytes: &'a [u8],
    offset: usize,
    limits: Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
) -> Result<Scan<Property<'a>>> {
    properties_version(bytes, offset, None, limits, budget, keep_going)
}

/// Like [`properties`], using the known file version for version-dependent
/// payload framing (notably historical five-string `PROP_USER_UNIT` records).
pub fn properties_with_version<'a>(
    bytes: &'a [u8],
    offset: usize,
    version: u32,
    limits: Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
) -> Result<Scan<Property<'a>>> {
    properties_version(bytes, offset, Some(version), limits, budget, keep_going)
}

fn properties_version<'a>(
    bytes: &'a [u8],
    offset: usize,
    version: Option<u32>,
    limits: Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
) -> Result<Scan<Property<'a>>> {
    let mut cursor = Cursor { b: bytes, pos: offset, wide: false };
    let mut records = Vec::new();
    loop {
        check(keep_going)?;
        let start = cursor.pos;
        let record = (|| {
            budget.work(8)?;
            let kind = cursor.u32().map_err(|_| StopReason::Malformed)?;
            let declared_length = cursor.u32().map_err(|_| StopReason::Malformed)?;
            if kind == 0 {
                return Ok(None);
            }
            if records.len() >= limits.records {
                return Err(StopReason::RecordLimit);
            }
            // These helpers only inspect a fixed number of bounded length fields;
            // they do not scan/copy the payload before its source-work admission.
            let length = match kind {
                1 => {
                    let n = cursor.at(cursor.pos).u32().map_err(|_| StopReason::Malformed)? as usize;
                    n.checked_mul(3).and_then(|n| n.checked_add(4)).ok_or(StopReason::Malformed)?
                }
                24 => crate::user_unit_len(&cursor, version).map_err(|_| StopReason::Malformed)?,
                48 => crate::link_layer_len(&cursor).map_err(|_| StopReason::Malformed)?,
                _ => usize::try_from(declared_length).map_err(|_| StopReason::Malformed)?,
            };
            if length > limits.payload_bytes {
                return Err(StopReason::PayloadLimit);
            }
            budget.work(length)?;
            let payload = cursor.take(length).map_err(|_| StopReason::Malformed)?;
            let raw = bytes.get(start..cursor.pos).ok_or(StopReason::Malformed)?;
            Ok(Some(Property { offset: start, kind, declared_length, payload, raw }))
        })();
        match record {
            Ok(None) => return Ok(Scan::complete(records, cursor.pos)),
            Ok(Some(record)) => {
                if let Err(reason) = budget.retain(&mut records, record) {
                    return Ok(Scan::stopped(records, start, reason));
                }
            }
            Err(reason) => return Ok(Scan::stopped(records, start, reason)),
        }
    }
}

/// Read every parasite from one property payload without deduplicating names.
/// `source_offset` is the payload's absolute position in the inflated XCF.
pub fn parasites<'a>(
    bytes: &'a [u8],
    source_offset: usize,
    limits: Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
) -> Result<Scan<Parasite<'a>>> {
    let mut cursor = Cursor { b: bytes, pos: 0, wide: false };
    let mut records = Vec::new();
    loop {
        check(keep_going)?;
        let start = cursor.pos;
        let absolute = source_offset.checked_add(start).ok_or_else(|| crate::invalid("parasite source offset overflow"))?;
        if start == bytes.len() {
            return Ok(Scan::complete(records, absolute));
        }
        if records.len() >= limits.records {
            return Ok(Scan::stopped(records, absolute, StopReason::RecordLimit));
        }
        let record = (|| {
            budget.work(4)?;
            let name_length = cursor.u32().map_err(|_| StopReason::Malformed)? as usize;
            if name_length > limits.string_bytes {
                return Err(StopReason::StringLimit);
            }
            budget.work(name_length)?;
            let name = cursor.take(name_length).map_err(|_| StopReason::Malformed)?;
            let name = name.strip_suffix(&[0]).unwrap_or(name);
            budget.work(8)?;
            let flags = cursor.u32().map_err(|_| StopReason::Malformed)?;
            let length = cursor.u32().map_err(|_| StopReason::Malformed)? as usize;
            if length > limits.payload_bytes {
                return Err(StopReason::PayloadLimit);
            }
            budget.work(length)?;
            let payload = cursor.take(length).map_err(|_| StopReason::Malformed)?;
            let raw = bytes.get(start..cursor.pos).ok_or(StopReason::Malformed)?;
            Ok(Parasite { offset: absolute, name, flags, payload, raw })
        })();
        match record {
            Ok(record) => {
                if let Err(reason) = budget.retain(&mut records, record) {
                    return Ok(Scan::stopped(records, absolute, reason));
                }
            }
            Err(reason) => return Ok(Scan::stopped(records, absolute, reason)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn property(kind: u32, payload: &[u8]) -> Vec<u8> {
        [kind.to_be_bytes().as_slice(), (payload.len() as u32).to_be_bytes().as_slice(), payload].concat()
    }

    fn parasite(name: &[u8], flags: u32, payload: &[u8]) -> Vec<u8> {
        [(name.len() as u32).to_be_bytes().as_slice(), name, flags.to_be_bytes().as_slice(), (payload.len() as u32).to_be_bytes().as_slice(), payload]
            .concat()
    }

    #[test]
    fn ordered_parasites_keep_duplicate_empty_values_flags_and_non_utf8_names() {
        let bytes = [parasite(b"same\0", 1, b"first"), parasite(b"same\0", 2, b""), parasite(b"\xff\0", 3, b"last")].concat();
        let scan = parasites(&bytes, 40, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        assert_eq!(
            scan.records.iter().map(|record| (record.name, record.flags, record.payload)).collect::<Vec<_>>(),
            [(b"same".as_slice(), 1, b"first".as_slice()), (b"same".as_slice(), 2, b"".as_slice()), (b"\xff".as_slice(), 3, b"last".as_slice())]
        );
        assert!(scan.records.windows(2).all(|pair| pair[0].offset < pair[1].offset));
        assert_eq!(scan.end_offset, 40 + bytes.len());
        assert_eq!(scan.records[0].raw.as_ptr(), bytes.as_ptr());
    }

    #[test]
    fn malformed_aliases_and_distinct_owners_share_nonrefundable_work() {
        let valid = parasite(b"name\0", 0, b"value");
        let bytes = [valid.as_slice(), &[0, 0, 0]].concat();
        let mut budget = Budget::new(valid.len() + 4, MAX_METADATA_BYTES);
        let first = parasites(&bytes, 20, Limits::default(), &mut budget, &|| true).unwrap();
        assert_eq!(first.records.len(), 1);
        assert_eq!(first.stopped, Some(Diagnostic { offset: 20 + valid.len(), ordinal: 1, reason: StopReason::Malformed }));
        assert_eq!(budget.work_remaining(), 0);
        let retained = budget.retained_remaining();
        for offset in [20, 200] {
            let next = parasites(&bytes, offset, Limits::default(), &mut budget, &|| true).unwrap();
            assert_eq!(next.stopped.map(|diagnostic| diagnostic.reason), Some(StopReason::WorkLimit));
            assert!(next.records.is_empty());
        }
        assert_eq!(budget.retained_remaining(), retained);
    }

    #[test]
    fn property_work_and_retention_have_independent_exact_boundaries() {
        let bytes = [property(100, b"borrowed payload"), property(0, b"")].concat();
        let retained = 2 * std::mem::size_of::<Property<'_>>();
        let mut exact = Budget::new(bytes.len(), retained);
        let scan = properties(&bytes, 0, Limits::default(), &mut exact, &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        assert_eq!(scan.records[0].payload, b"borrowed payload");
        assert_eq!(exact.work_remaining(), 0);
        assert_eq!(exact.retained_remaining(), 0);
        let mut work_short = Budget::new(bytes.len() - 1, retained);
        let scan = properties(&bytes, 0, Limits::default(), &mut work_short, &|| true).unwrap();
        assert_eq!(scan.records.len(), 1);
        assert_eq!(scan.stopped.map(|diagnostic| diagnostic.reason), Some(StopReason::WorkLimit));
        let mut retention_short = Budget::new(bytes.len(), retained - 1);
        let scan = properties(&bytes, 0, Limits::default(), &mut retention_short, &|| true).unwrap();
        assert!(scan.records.is_empty());
        assert_eq!(scan.stopped.map(|diagnostic| diagnostic.reason), Some(StopReason::RetainedLimit));
        assert!(retention_short.work_remaining() < bytes.len());
    }

    #[test]
    fn property_prefix_limits_and_measured_length_preserve_source_offsets() {
        // PROP_COLORMAP's historical declared size is n+4, but its payload is 3n+4.
        let mut bytes = property(1, &[0, 0, 0, 1, 20, 30, 40]);
        bytes[4..8].copy_from_slice(&5u32.to_be_bytes());
        let second = bytes.len();
        bytes.extend(property(100, b"later"));
        bytes.extend(property(0, b""));
        let scan = properties(&bytes, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        assert_eq!(scan.records[0].declared_length, 5);
        assert_eq!(scan.records[0].payload.len(), 7);
        assert_eq!(scan.records[1].offset, second);
        let limits = Limits { records: 1, ..Limits::default() };
        let limited = properties(&bytes, 0, limits, &mut Budget::default(), &|| true).unwrap();
        assert_eq!(limited.records.len(), 1);
        assert_eq!(limited.stopped, Some(Diagnostic { offset: second, ordinal: 1, reason: StopReason::RecordLimit }));
        let truncated = properties(&bytes[..second + 10], 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(truncated.records.len(), 1);
        assert_eq!(truncated.stopped.map(|diagnostic| diagnostic.reason), Some(StopReason::Malformed));
    }

    #[test]
    fn cancellation_precedes_empty_spent_and_partial_optional_scans() {
        for bytes in [&[][..], &[0, 0, 0][..]] {
            let mut budget = Budget::new(0, 0);
            assert!(matches!(parasites(bytes, 0, Limits::default(), &mut budget, &|| false), Err(Error::Cancelled)));
            assert!(matches!(properties(bytes, 0, Limits::default(), &mut budget, &|| false), Err(Error::Cancelled)));
        }
        let bytes = [parasite(b"a", 0, b""), parasite(b"b", 0, b"")].concat();
        let checks = std::cell::Cell::new(0);
        let keep_going = || {
            checks.set(checks.get() + 1);
            checks.get() < 2
        };
        assert!(matches!(parasites(&bytes, 0, Limits::default(), &mut Budget::default(), &keep_going), Err(Error::Cancelled)));
    }

    #[test]
    fn name_and_payload_limits_decline_records_without_retaining_their_spans() {
        let bytes = parasite(b"name\0", 0, b"payload");
        let limits = Limits { string_bytes: 4, ..Limits::default() };
        let mut budget = Budget::new(4, 0);
        let scan = parasites(&bytes, 0, limits, &mut budget, &|| true).unwrap();
        assert_eq!(scan.stopped.map(|diagnostic| diagnostic.reason), Some(StopReason::StringLimit));
        assert_eq!(budget.work_remaining(), 0);
        let limits = Limits { payload_bytes: 6, ..Limits::default() };
        let mut budget = Budget::default();
        let retained = budget.retained_remaining();
        let scan = parasites(&bytes, 0, limits, &mut budget, &|| true).unwrap();
        assert_eq!(scan.stopped.map(|diagnostic| diagnostic.reason), Some(StopReason::PayloadLimit));
        assert!(scan.records.is_empty());
        assert_eq!(budget.retained_remaining(), retained);
        let property = property(100, b"payload");
        let scan = properties(&property, 0, limits, &mut budget, &|| true).unwrap();
        assert_eq!(scan.stopped.map(|diagnostic| diagnostic.reason), Some(StopReason::PayloadLimit));
    }

    #[test]
    fn opt_in_accessors_keep_legacy_metadata_and_pixels_unchanged() {
        use crate::tests::{TestImage, TestLayer};
        for version in [0, 11, 18] {
            let mut image = TestImage::rgb8(1, 1);
            image.version = version;
            image.image_props = vec![(100, b"first".to_vec()), (100, b"last".to_vec())];
            let mut layer = TestLayer::rgba("source", 1, 1, vec![20, 30, 40, 255]);
            layer.props = vec![(100, b"layer".to_vec())];
            image.layers.push(layer);
            let bytes = image.write();
            let parsed = crate::Xcf::parse(&bytes).unwrap();
            let before = parsed.image().clone();
            let mut budget = Budget::default();
            let records = parsed.image_properties(Limits::default(), &mut budget, &|| true).unwrap();
            assert_eq!(records.stopped, None);
            assert_eq!(
                records.records.iter().filter(|property| property.kind == 100).map(|property| property.payload).collect::<Vec<_>>(),
                [b"first".as_slice(), b"last".as_slice()]
            );
            let records = parsed.layer_properties(0, Limits::default(), &mut budget, &|| true).unwrap();
            assert_eq!(records.stopped, None);
            assert!(records.records.iter().any(|property| property.kind == 100 && property.payload == b"layer"));
            assert!(parsed.layer_properties(1, Limits::default(), &mut budget, &|| true).is_err());
            assert_eq!(&before, parsed.image());
            let pixels = parsed.layer_pixels(0, &mut 1024, &|| true).unwrap().unwrap();
            assert_eq!(pixels.data.len(), 4);
            assert!((pixels.data[0] - 20.0 / 255.0).abs() < 1e-6);
        }
    }
}
