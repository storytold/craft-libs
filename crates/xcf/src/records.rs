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

    pub(crate) fn cancel_safe<T>(&mut self, read: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let retained = self.retained_left;
        let result = read(self);
        if matches!(result, Err(Error::Cancelled)) {
            self.retained_left = retained;
        }
        result
    }

    /// Dropped output owns no retained storage, even on a non-cancellation error.
    /// Successful partial prefixes stay charged, and attempted work is never refunded.
    pub(crate) fn retain_on_success<T, E>(&mut self, read: impl FnOnce(&mut Self) -> std::result::Result<T, E>) -> std::result::Result<T, E> {
        let retained = self.retained_left;
        let result = read(self);
        if result.is_err() {
            self.retained_left = retained;
        }
        result
    }

    pub(crate) fn work(&mut self, bytes: usize) -> std::result::Result<(), StopReason> {
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
    drawable_properties(bytes, offset, 12, limits, budget, keep_going)
}

pub(super) fn channel_properties<'a>(
    bytes: &'a [u8],
    offset: usize,
    limits: Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
) -> Result<Scan<Property<'a>>> {
    drawable_properties(bytes, offset, 8, limits, budget, keep_going)
}

fn drawable_properties<'a>(
    bytes: &'a [u8],
    offset: usize,
    header_bytes: usize,
    limits: Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
) -> Result<Scan<Property<'a>>> {
    check(keep_going)?;
    let mut cursor = Cursor { b: bytes, pos: offset, wide: false };
    let prefix = (|| {
        budget.work(header_bytes + 4)?;
        cursor.take(header_bytes).map_err(|_| StopReason::Malformed)?;
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
    let retained = budget.retained_left;
    let mut records = Vec::new();
    loop {
        if let Err(error) = check(keep_going) {
            budget.retained_left = retained;
            return Err(error);
        }
        let start = cursor.pos;
        let record = (|| {
            budget.work(8)?;
            let kind = cursor.u32().map_err(|_| StopReason::Malformed)?;
            let declared_length = cursor.u32().map_err(|_| StopReason::Malformed)?;
            if kind == 0 {
                return if declared_length == 0 { Ok(None) } else { Err(StopReason::Malformed) };
            }
            if records.len() >= limits.records {
                return Err(StopReason::RecordLimit);
            }
            // These helpers only inspect a fixed number of bounded length fields;
            // they do not scan/copy the payload before its source-work admission.
            let mut measured = 0;
            let length = match kind {
                1 => {
                    let n = measured_word(&mut cursor.at(cursor.pos), budget, &mut measured)? as usize;
                    n.checked_mul(3).and_then(|n| n.checked_add(4)).ok_or(StopReason::Malformed)?
                }
                24 | 48 => measured_length(&cursor, kind, version, limits.string_bytes, budget, &mut measured)?,
                _ => usize::try_from(declared_length).map_err(|_| StopReason::Malformed)?,
            };
            if length > limits.payload_bytes {
                return Err(StopReason::PayloadLimit);
            }
            budget.work(length.checked_sub(measured).ok_or(StopReason::Malformed)?)?;
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

// Measure the legacy incorrect length words without ignoring the caller's
// string cap. Only fixed-count fields are inspected before payload work admission.
fn measured_length(
    cursor: &Cursor<'_>,
    kind: u32,
    version: Option<u32>,
    string_limit: usize,
    budget: &mut Budget,
    measured: &mut usize,
) -> std::result::Result<usize, StopReason> {
    let mut p = cursor.at(cursor.pos);
    p.skip(if kind == 48 { 4 } else { 8 }).map_err(|_| StopReason::Malformed)?;
    if kind == 48 {
        measured_string(&mut p, string_limit, budget, measured)?;
        p.skip(8).map_err(|_| StopReason::Malformed)?;
    } else {
        for _ in 0..3 {
            measured_string(&mut p, string_limit, budget, measured)?;
        }
        if let Some(version) = version {
            if version < 21 {
                measured_string(&mut p, string_limit, budget, measured)?;
                measured_string(&mut p, string_limit, budget, measured)?;
            }
        } else {
            let mut peek = p.at(p.pos);
            let length = measured_word(&mut peek, budget, measured)? as usize;
            if length <= 64 {
                // This probe belongs to the next property, not this payload. Keep its work
                // charged, but do not subtract it from the payload admission below.
                *measured -= 4;
            } else {
                p = peek;
                measured_string_payload(&mut p, length, string_limit)?;
                measured_string(&mut p, string_limit, budget, measured)?;
            }
        }
    }
    Ok(p.pos - cursor.pos)
}

fn measured_word(cursor: &mut Cursor<'_>, budget: &mut Budget, measured: &mut usize) -> std::result::Result<u32, StopReason> {
    // Even a truncated measurement is attempted source work; admit it before reading.
    budget.work(4)?;
    *measured += 4;
    cursor.u32().map_err(|_| StopReason::Malformed)
}

fn measured_string(cursor: &mut Cursor<'_>, string_limit: usize, budget: &mut Budget, measured: &mut usize) -> std::result::Result<(), StopReason> {
    let length = measured_word(cursor, budget, measured)? as usize;
    measured_string_payload(cursor, length, string_limit)
}

fn measured_string_payload(cursor: &mut Cursor<'_>, length: usize, string_limit: usize) -> std::result::Result<(), StopReason> {
    if length > string_limit.min(MAX_STRING) {
        return Err(StopReason::StringLimit);
    }
    cursor.skip(length).map_err(|_| StopReason::Malformed)
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
    budget.retain_on_success(|budget| parasites_read(bytes, source_offset, limits, budget, keep_going))
}

fn parasites_read<'a>(
    bytes: &'a [u8],
    source_offset: usize,
    limits: Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
) -> Result<Scan<Parasite<'a>>> {
    let mut cursor = Cursor { b: bytes, pos: 0, wide: false };
    let retained = budget.retained_left;
    let mut records = Vec::new();
    loop {
        if let Err(error) = check(keep_going) {
            budget.retained_left = retained;
            return Err(error);
        }
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
    #[test]
    fn cancelled_raw_scans_refund_only_dropped_storage_and_allow_retry() {
        let property_bytes = [property(100, b"a"), property(0, b"")].concat();
        let parasite_bytes = parasite(b"a", 0, b"");
        for is_property in [true, false] {
            let mut budget = Budget::new(4096, 2 * std::mem::size_of::<Parasite<'_>>());
            let entry = budget.retained_remaining();
            let calls = std::cell::Cell::new(0);
            let keep = || {
                calls.set(calls.get() + 1);
                calls.get() < 2
            };
            let result = if is_property {
                properties(&property_bytes, 0, Limits::default(), &mut budget, &keep).map(|_| ())
            } else {
                parasites(&parasite_bytes, 0, Limits::default(), &mut budget, &keep).map(|_| ())
            };
            assert!(matches!(result, Err(Error::Cancelled)));
            assert_eq!(budget.retained_remaining(), entry);
            assert!(budget.work_remaining() < 4096);
            let retry_count = if is_property {
                properties(&property_bytes, 0, Limits::default(), &mut budget, &|| true).unwrap().records.len()
            } else {
                parasites(&parasite_bytes, 0, Limits::default(), &mut budget, &|| true).unwrap().records.len()
            };
            assert_eq!(retry_count, 1);
            assert!(budget.retained_remaining() < entry);
        }
    }

    #[test]
    fn measured_strings_obey_custom_limits_without_trusting_declared_lengths() {
        for (kind, version, count) in [(24, 20, 5), (24, 21, 3), (48, 26, 1)] {
            let mut payload = vec![0; if kind == 48 { 4 } else { 8 }];
            for _ in 0..count {
                payload.extend(3u32.to_be_bytes());
                payload.extend(b"ab\0");
            }
            if kind == 48 {
                payload.extend([0; 8]);
            }
            let mut bytes = property(kind, &payload);
            bytes[4..8].copy_from_slice(&0u32.to_be_bytes());
            bytes.extend(property(0, b""));
            let limited =
                properties_with_version(&bytes, 0, version, Limits { string_bytes: 2, ..Default::default() }, &mut Budget::default(), &|| true)
                    .unwrap();
            assert!(limited.records.is_empty());
            assert_eq!(limited.stopped.unwrap().reason, StopReason::StringLimit);
            let exact =
                properties_with_version(&bytes, 0, version, Limits { string_bytes: 3, ..Default::default() }, &mut Budget::default(), &|| true)
                    .unwrap();
            assert!(exact.stopped.is_none());
            assert_eq!(exact.records[0].payload, payload);
        }
    }
    #[test]
    fn malformed_end_length_keeps_prefix_but_zero_end_is_allowed_at_record_limit() {
        let mut bytes = property(100, b"kept");
        let end = bytes.len();
        bytes.extend(property(0, b"not a terminator"));
        let limits = Limits { records: 1, ..Default::default() };
        let scan = properties(&bytes, 0, limits, &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.records.len(), 1);
        assert_eq!(scan.stopped, Some(Diagnostic { offset: end, ordinal: 1, reason: StopReason::Malformed }));
        bytes.truncate(end);
        bytes.extend(property(0, b""));
        let scan = properties(&bytes, 0, limits, &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.records.len(), 1);
        assert!(scan.stopped.is_none());
    }
    #[test]
    fn measurement_work_is_admitted_before_even_a_truncated_length_read() {
        for (kind, prefix) in [(1, vec![]), (24, vec![0; 8]), (48, vec![0; 4])] {
            let bytes = property(kind, &prefix);
            let mut denied = Budget::new(8, usize::MAX);
            let scan = properties_with_version(&bytes, 0, 26, Limits::default(), &mut denied, &|| true).unwrap();
            assert_eq!(scan.stopped.unwrap().reason, StopReason::WorkLimit, "kind {kind}");
            assert!(scan.records.is_empty());
            assert_eq!(denied.work_remaining(), 0);
            let mut admitted = Budget::new(12, usize::MAX);
            let scan = properties_with_version(&bytes, 0, 26, Limits::default(), &mut admitted, &|| true).unwrap();
            assert_eq!(scan.stopped.unwrap().reason, StopReason::Malformed, "kind {kind}");
            assert_eq!(admitted.work_remaining(), 0);
        }
    }

    #[test]
    fn successful_measurements_do_not_charge_payload_length_words_twice() {
        let colormap = [1u32.to_be_bytes().as_slice(), &[1, 2, 3]].concat();
        let mut unit = vec![0; 8];
        for _ in 0..3 {
            unit.extend([1u32.to_be_bytes().as_slice(), &[0]].concat());
        }
        let link = [0u32.to_be_bytes().as_slice(), &1u32.to_be_bytes(), &[0], &[0; 8]].concat();
        for (kind, payload) in [(1, colormap), (24, unit), (48, link)] {
            let bytes = [property(kind, &payload), property(0, &[])].concat();
            let mut budget = Budget::new(bytes.len(), usize::MAX);
            let scan = properties_with_version(&bytes, 0, 26, Limits::default(), &mut budget, &|| true).unwrap();
            assert!(scan.stopped.is_none(), "kind {kind}: {:?}", scan.stopped);
            assert_eq!(scan.records[0].payload, payload);
            assert_eq!(budget.work_remaining(), 0);
        }
    }
    #[test]
    fn overflowing_parasite_offsets_drop_only_the_unreturned_retention() {
        let bytes = parasite(b"name", 0, b"source");
        let mut budget = Budget::new(bytes.len() * 4, 4 * std::mem::size_of::<Parasite<'_>>());
        let held = parasites(&bytes, 0, Limits::default(), &mut budget, &|| true).unwrap();
        assert_eq!(held.records.len(), 1);
        let retained = budget.retained_remaining();
        let work = budget.work_remaining();
        let result = parasites(&bytes, usize::MAX - bytes.len() + 1, Limits::default(), &mut budget, &|| true);
        assert!(matches!(result, Err(Error::Invalid(_))));
        assert_eq!(budget.retained_remaining(), retained);
        assert!(budget.work_remaining() < work);
        let retry = parasites(&bytes, 0, Limits::default(), &mut budget, &|| true).unwrap();
        assert!(retry.stopped.is_none());
        assert_eq!(retry.records[0].payload, b"source");
        assert_eq!(budget.retained_remaining(), 0);
        let full = parasites(&bytes, 0, Limits::default(), &mut budget, &|| true).unwrap();
        assert_eq!(full.stopped.unwrap().reason, StopReason::RetainedLimit);
        assert_eq!(held.records[0].payload, b"source");
    }
}
