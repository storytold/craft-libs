//! Source-only saved path geometry. No fill, paint or native shape policy.

use super::{Budget, Diagnostic, Property, Scan, StopReason};
use crate::{Cursor, Error, Result};

pub(crate) mod index;

/// Per-list structural limits; cumulative work and storage use the shared Budget.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub paths: usize,
    pub points: usize,
    pub strokes: usize,
    pub name_bytes: usize,
    pub payload_bytes: usize,
    pub parasites: usize,
    pub properties: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            paths: 1024,
            points: 262_144,
            strokes: 262_144,
            name_bytes: crate::MAX_STRING,
            payload_bytes: crate::MAX_PAYLOAD,
            parasites: 4096,
            properties: 4096,
        }
    }
}

/// A finite source position and up to four pressure/tilt/wheel axes.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Point {
    pub position: [f64; 2],
    /// Pressure, x tilt, y tilt and wheel, valid only for indices below
    /// `Stroke::axes - 2`. Unstored axes are zero placeholders, not defaults:
    /// their source defaults are pressure 1.0, tilt 0.0 and wheel 0.5.
    pub auxiliary: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Knot {
    pub incoming: Point,
    pub anchor: Point,
    pub outgoing: Point,
}

/// Unknown kinds retain raw bytes only when they fit the known Bezier-shaped
/// frame (a type word and `axes` floats per point). No other unknown-kind layout
/// is inferred; a frame mismatch stops the optional list. Only kind 1 has knots.
#[derive(Debug)]
pub struct Stroke<'a> {
    pub kind: u32,
    pub closed: bool,
    /// Number of stored axes, including x/y. Consult this before reading a
    /// point's auxiliary axes; explicit stored zero values remain zero.
    pub axes: u32,
    pub knots: Vec<Knot>,
    pub raw: &'a [u8],
}

/// Source identity is independent of the consumer's native path indices.
#[derive(Debug)]
pub struct SourcePath<'a> {
    pub source_index: usize,
    pub offset: usize,
    /// UTF-8 label, or an empty compatibility placeholder for an undecodable
    /// legacy label. Use `decoded_name` to distinguish that case from empty.
    pub name: &'a str,
    pub tattoo: Option<u32>,
    pub selected: bool,
    pub invalid_selection_marker: bool,
    pub visible: u32,
    pub linked: u32,
    pub legacy_state: Option<u8>,
    pub legacy_version: Option<u32>,
    pub properties: Vec<Property<'a>>,
    /// Historical vector-path framed parasite bytes. Standalone path parasite
    /// properties stay ordered in `properties`; use `parasite_payloads` for both.
    pub parasites: &'a [u8],
    pub strokes: Vec<Stroke<'a>>,
    pub raw: &'a [u8],
}

/// Borrowed framed parasite bytes and their absolute source offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParasiteSpan<'a> {
    pub offset: usize,
    pub bytes: &'a [u8],
}

impl SourcePath<'_> {
    /// Original label bytes without the optional final wire NUL. Legacy labels
    /// may use an unknown locale encoding; no encoding is guessed here.
    pub fn name_bytes(&self) -> &[u8] {
        let length = self.raw.get(..4).and_then(crate::be_u32).unwrap_or(0) as usize;
        let bytes = self.raw.get(4..).and_then(|bytes| bytes.get(..length)).unwrap_or_default();
        bytes.strip_suffix(&[0]).unwrap_or(bytes)
    }

    /// A decoded label when its original bytes are UTF-8. Modern labels are
    /// always decoded; `None` preserves an undecodable legacy label explicitly.
    pub fn decoded_name(&self) -> Option<&str> {
        std::str::from_utf8(self.name_bytes()).ok()
    }

    /// The same ordered payloads as `parasite_payloads`, with absolute offsets
    /// suitable for `super::parasites`. No payload is copied or scanned.
    pub fn parasite_spans(&self) -> impl Iterator<Item = ParasiteSpan<'_>> {
        let vector = (!self.parasites.is_empty())
            .then(|| {
                let relative = self.parasites.as_ptr().addr().checked_sub(self.raw.as_ptr().addr())?;
                Some(ParasiteSpan { offset: self.offset.checked_add(relative)?, bytes: self.parasites })
            })
            .flatten();
        vector.into_iter().chain(
            self.properties
                .iter()
                .filter(|property| property.kind == 21)
                .filter_map(|property| Some(ParasiteSpan { offset: property.offset.checked_add(8)?, bytes: property.payload })),
        )
    }

    /// Borrow the framed parasite payloads in source order, for decoding with
    /// [`super::parasites`]. Historical vectors supply one nonempty span;
    /// standalone paths supply every PROP_PARASITES payload, including empty or
    /// duplicate properties. This accessor does not scan or copy their contents.
    pub fn parasite_payloads(&self) -> impl Iterator<Item = &[u8]> {
        (!self.parasites.is_empty())
            .then_some(self.parasites)
            .into_iter()
            .chain(self.properties.iter().filter(|property| property.kind == 21).map(|property| property.payload))
    }

    pub fn geometry_complete(&self) -> bool {
        self.strokes.iter().all(|stroke| stroke.kind == 1)
    }
}

enum Failure {
    Stop(StopReason),
    Cancelled,
}
impl From<StopReason> for Failure {
    fn from(reason: StopReason) -> Self {
        Self::Stop(reason)
    }
}
type ReadResult<T> = std::result::Result<T, Failure>;

struct Reader<'a, 'b> {
    cursor: Cursor<'a>,
    source_offset: usize,
    limits: Limits,
    points_left: usize,
    budget: &'b mut Budget,
    keep_going: &'b dyn Fn() -> bool,
}

impl<'a> Reader<'a, '_> {
    fn check(&self) -> ReadResult<()> {
        if (self.keep_going)() { Ok(()) } else { Err(Failure::Cancelled) }
    }
    fn offset(&self) -> ReadResult<usize> {
        self.source_offset.checked_add(self.cursor.pos).ok_or(StopReason::Malformed.into())
    }
    fn take(&mut self, bytes: usize) -> ReadResult<&'a [u8]> {
        self.check()?;
        self.budget.work(bytes)?;
        self.cursor.take(bytes).map_err(|_| StopReason::Malformed.into())
    }
    fn word(&mut self) -> ReadResult<u32> {
        Ok(crate::be_u32(self.take(4)?).unwrap_or(0))
    }
    fn count(&mut self, limit: usize) -> ReadResult<usize> {
        let count = self.word()? as usize;
        if count > limit { Err(StopReason::RecordLimit.into()) } else { Ok(count) }
    }
    fn name_bytes(&mut self) -> ReadResult<&'a [u8]> {
        let length = self.word()? as usize;
        if length > self.limits.name_bytes {
            return Err(StopReason::StringLimit.into());
        }
        let raw = self.take(length)?;
        if raw.is_empty() {
            return Ok(raw);
        }
        let raw = raw.strip_suffix(&[0]).ok_or(Failure::Stop(StopReason::Malformed))?;
        // Admit both scans before checking the decoded label. Opaque parasite
        // names have their own framing path and are not subjected to this rule.
        self.budget.work(raw.len().checked_mul(2).ok_or(StopReason::WorkLimit)?)?;
        if raw.contains(&0) {
            return Err(StopReason::Malformed.into());
        }
        Ok(raw)
    }
    fn name(&mut self) -> ReadResult<&'a str> {
        std::str::from_utf8(self.name_bytes()?).map_err(|_| StopReason::Malformed.into())
    }
    fn flag(&mut self) -> ReadResult<bool> {
        match self.word()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(StopReason::Malformed.into()),
        }
    }
    fn point(&mut self, axes: usize) -> ReadResult<Point> {
        let mut point = Point::default();
        for axis in 0..axes {
            let value = f32::from_bits(self.word()?);
            if !value.is_finite() {
                return Err(StopReason::Malformed.into());
            }
            if axis < 2 {
                if let Some(slot) = point.position.get_mut(axis) {
                    *slot = f64::from(value);
                }
            } else if let Some(slot) = point.auxiliary.get_mut(axis - 2) {
                *slot = value;
            }
        }
        Ok(point)
    }
    fn strokes(&mut self, count: usize) -> ReadResult<Vec<Stroke<'a>>> {
        let mut strokes = Vec::new();
        for _ in 0..count {
            self.check()?;
            let start = self.cursor.pos;
            let kind = self.word()?;
            let closed = self.flag()?;
            let axes = self.count(6)?;
            if axes < 2 {
                return Err(StopReason::Malformed.into());
            }
            let points = self.count(self.points_left)?;
            self.points_left -= points;
            // The public XCF specification defines only the kind-1 Bezier
            // frame. Retain other kind tags within this bounded frame only;
            // do not guess a different point layout from an unknown tag.
            let length = points.checked_mul((axes + 1) * 4).ok_or(Failure::Stop(StopReason::Malformed))?;
            let raw = self.take(length)?;
            let mut knots = Vec::new();
            if kind == 1 {
                if !points.is_multiple_of(3) {
                    return Err(StopReason::Malformed.into());
                }
                let mut p = Reader {
                    cursor: Cursor { b: raw, pos: 0, wide: false },
                    source_offset: 0,
                    limits: self.limits,
                    points_left: 0,
                    budget: &mut *self.budget,
                    keep_going: self.keep_going,
                };
                for _ in 0..points / 3 {
                    let mut triple = [Point::default(); 3];
                    for (index, point) in triple.iter_mut().enumerate() {
                        if p.word()? != if index == 1 { 0 } else { 1 } {
                            return Err(StopReason::Malformed.into());
                        }
                        *point = p.point(axes)?;
                    }
                    let [incoming, anchor, outgoing] = triple;
                    p.budget.retain(&mut knots, Knot { incoming, anchor, outgoing })?;
                }
            }
            let raw = self.cursor.b.get(start..self.cursor.pos).ok_or(Failure::Stop(StopReason::Malformed))?;
            self.budget.retain(&mut strokes, Stroke { kind, closed, axes: axes as u32, knots, raw })?;
        }
        Ok(strokes)
    }
    fn vector(&mut self, source_index: usize, active: u32) -> ReadResult<SourcePath<'a>> {
        let start = self.cursor.pos;
        let offset = self.offset()?;
        let name = self.name()?;
        let tattoo = self.word()?;
        let visible = self.word()?;
        let linked = self.word()?;
        let parasites = self.count(self.limits.parasites)?;
        let count = self.count(self.limits.strokes)?;
        let parasite_start = self.cursor.pos;
        for _ in 0..parasites {
            // Parasite names are opaque bytes, unlike the UTF-8 path label.
            let length = self.word()? as usize;
            if length > self.limits.name_bytes {
                return Err(StopReason::StringLimit.into());
            }
            let name = self.take(length)?;
            if !name.is_empty() && name.last() != Some(&0) {
                return Err(StopReason::Malformed.into());
            }
            self.word()?;
            let length = self.word()? as usize;
            if length > self.limits.payload_bytes {
                return Err(StopReason::PayloadLimit.into());
            }
            self.take(length)?;
        }
        let parasites = self.cursor.b.get(parasite_start..self.cursor.pos).ok_or(Failure::Stop(StopReason::Malformed))?;
        let strokes = self.strokes(count)?;
        let raw = self.cursor.b.get(start..self.cursor.pos).ok_or(Failure::Stop(StopReason::Malformed))?;
        Ok(SourcePath {
            source_index,
            offset,
            name,
            tattoo: (tattoo != 0).then_some(tattoo),
            selected: source_index == active as usize,
            invalid_selection_marker: false,
            visible,
            linked,
            legacy_state: None,
            legacy_version: None,
            properties: Vec::new(),
            parasites,
            strokes,
            raw,
        })
    }
}

impl<'a> Reader<'a, '_> {
    fn legacy_point(&mut self, raw: &'a [u8], index: usize, version: u32) -> ReadResult<(u32, Point)> {
        let start = index.checked_mul(12).ok_or(Failure::Stop(StopReason::Malformed))?;
        self.check()?;
        self.budget.work(12)?;
        let mut cursor = Cursor { b: raw, pos: start, wide: false };
        let kind = cursor.u32().map_err(|_| Failure::Stop(StopReason::Malformed))?;
        if !(1..=3).contains(&kind) {
            return Err(StopReason::Malformed.into());
        }
        let mut point = Point::default();
        for slot in &mut point.position {
            let word = cursor.u32().map_err(|_| Failure::Stop(StopReason::Malformed))?;
            *slot = if version == 1 { f64::from(word as i32) } else { f64::from(f32::from_bits(word)) };
            if !slot.is_finite() {
                return Err(StopReason::Malformed.into());
            }
        }
        Ok((kind, point))
    }

    fn legacy(&mut self, source_index: usize, active: u32) -> ReadResult<SourcePath<'a>> {
        let start = self.cursor.pos;
        let offset = self.offset()?;
        // Old locale-encoded labels do not invalidate their saved geometry.
        // The exact source remains available through SourcePath::name_bytes.
        let name = std::str::from_utf8(self.name_bytes()?).unwrap_or_default();
        let linked = self.word()?;
        let state = self.take(1)?.first().copied().unwrap_or(0);
        let closed = self.flag()?;
        let points = self.count(self.points_left)?;
        self.points_left -= points;
        let version = self.word()?;
        if !(1..=3).contains(&version) {
            return Err(StopReason::Malformed.into());
        }
        if version >= 2 {
            self.word()?;
        }
        let tattoo = if version >= 3 { self.word()? } else { 0 };
        let raw_points = self.take(points.checked_mul(12).ok_or(Failure::Stop(StopReason::Malformed))?)?;
        let mut strokes = Vec::new();
        let mut first = 0;
        for end in 1..=points {
            if end != points && self.legacy_point(raw_points, end, version)?.0 != 3 {
                continue;
            }
            let count = end - first;
            let closed = end != points || closed;
            if count % 3 != if closed { 0 } else { 2 } {
                return Err(StopReason::Malformed.into());
            }
            if strokes.len() >= self.limits.strokes {
                return Err(StopReason::RecordLimit.into());
            }
            let mut knots = Vec::new();
            for index in (first..end).step_by(3) {
                let (kind, anchor) = self.legacy_point(raw_points, index, version)?;
                if !matches!(kind, 1 | 3) {
                    return Err(StopReason::Malformed.into());
                }
                let (kind, outgoing) = self.legacy_point(raw_points, index + 1, version)?;
                if kind != 2 {
                    return Err(StopReason::Malformed.into());
                }
                let incoming = if index == first && !closed {
                    anchor
                } else {
                    let (kind, point) = self.legacy_point(raw_points, if index == first { end - 1 } else { index - 1 }, version)?;
                    if kind != 2 {
                        return Err(StopReason::Malformed.into());
                    }
                    point
                };
                self.budget.retain(&mut knots, Knot { incoming, anchor, outgoing })?;
            }
            let raw = raw_points.get(first * 12..end * 12).ok_or(Failure::Stop(StopReason::Malformed))?;
            self.budget.retain(&mut strokes, Stroke { kind: 1, closed, axes: 2, knots, raw })?;
            first = end;
        }
        let raw = self.cursor.b.get(start..self.cursor.pos).ok_or(Failure::Stop(StopReason::Malformed))?;
        Ok(SourcePath {
            source_index,
            offset,
            name,
            tattoo: (tattoo != 0).then_some(tattoo),
            selected: source_index == active as usize,
            invalid_selection_marker: false,
            visible: 0,
            linked,
            legacy_state: Some(state),
            legacy_version: Some(version),
            properties: Vec::new(),
            parasites: &[],
            strokes,
            raw,
        })
    }
}

fn read_list<'a>(
    bytes: &'a [u8],
    source_offset: usize,
    modern: bool,
    limits: Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
) -> Result<Scan<SourcePath<'a>>> {
    read_list_into(bytes, source_offset, modern, limits, budget, keep_going, Vec::new())
}

// Appending avoids a second list container and never stages a decoded suffix
// that output growth cannot retain. Public readers still start their own list.
fn read_list_into<'a>(
    bytes: &'a [u8],
    source_offset: usize,
    modern: bool,
    limits: Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
    mut records: Vec<SourcePath<'a>>,
) -> Result<Scan<SourcePath<'a>>> {
    if !keep_going() {
        return Err(Error::Cancelled);
    }
    // A caller-supplied base must address the complete source span before any
    // geometry is retained; otherwise an eventual end overflow drops its charge.
    source_offset.checked_add(bytes.len()).ok_or_else(|| crate::invalid("path source offset overflow"))?;
    let initial_retained = budget.retained_left;
    let mut reader =
        Reader { cursor: Cursor { b: bytes, pos: 0, wide: false }, source_offset, limits, points_left: limits.points, budget, keep_going };
    let header = (|| {
        reader.check()?;
        if bytes.len() > limits.payload_bytes {
            return Err(StopReason::PayloadLimit.into());
        }
        if modern && reader.word()? != 1 {
            return Err(StopReason::Malformed.into());
        }
        Ok((reader.word()?, reader.word()? as usize))
    })();
    let (active, count) = match header {
        Ok(header) => header,
        Err(Failure::Cancelled) => {
            reader.budget.retained_left = initial_retained;
            return Err(Error::Cancelled);
        }
        Err(Failure::Stop(reason)) => return Ok(Scan::stopped(records, source_offset, reason)),
    };
    let mut strokes_left = limits.strokes;
    for index in 0..count {
        if !keep_going() {
            reader.budget.retained_left = initial_retained;
            return Err(Error::Cancelled);
        }
        let offset = match reader.offset() {
            Ok(offset) => offset,
            _ => return Ok(Scan::stopped(records, source_offset, StopReason::Malformed)),
        };
        if index >= limits.paths {
            return Ok(Scan::stopped(records, offset, StopReason::RecordLimit));
        }
        let retained = reader.budget.retained_left;
        reader.limits.strokes = strokes_left;
        let path = if modern { reader.vector(index, active) } else { reader.legacy(index, active) };
        let result = path.and_then(|path| {
            // Both supported and unknown framed strokes consume the per-list
            // allowance. A rejected path stops this list without losing its prefix.
            strokes_left = strokes_left.saturating_sub(path.strokes.len());
            reader.budget.retain(&mut records, path).map_err(Failure::from)
        });
        match result {
            Ok(()) => {}
            Err(Failure::Cancelled) => {
                reader.budget.retained_left = initial_retained;
                return Err(Error::Cancelled);
            }
            Err(Failure::Stop(reason)) => {
                // Refund only the rejected path's dropped storage. Earlier paths
                // remain charged, and source work is never refunded.
                reader.budget.retained_left = retained;
                return Ok(Scan::stopped(records, offset, reason));
            }
        }
    }
    let end = source_offset.checked_add(reader.cursor.pos).ok_or_else(|| crate::invalid("path source offset overflow"))?;
    if reader.cursor.pos != bytes.len() {
        return Ok(Scan::stopped(records, end, StopReason::Malformed));
    }
    Ok(Scan::complete(records, end))
}

/// Decode a legacy PROP_PATHS payload without native curve subdivision.
pub fn legacy<'a>(
    bytes: &'a [u8],
    source_offset: usize,
    limits: Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
) -> Result<Scan<SourcePath<'a>>> {
    read_list(bytes, source_offset, false, limits, budget, keep_going)
}

/// Decode PROP_VECTORS, retaining source parasites and unknown stroke tags only
/// within the specified Bezier-shaped framing. Other layouts are not supported.
/// See <https://developer.gimp.org/core/standards/xcf/#image-properties>.
pub fn vectors<'a>(
    bytes: &'a [u8],
    source_offset: usize,
    limits: Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
) -> Result<Scan<SourcePath<'a>>> {
    read_list(bytes, source_offset, true, limits, budget, keep_going)
}

// Path-local property IDs have no image/layer-specific length corrections.
// Unknown property payloads are opaque and framed by their declared length.
fn local_properties<'a>(bytes: &'a [u8], limits: Limits, budget: &mut Budget, keep_going: &dyn Fn() -> bool) -> Result<Scan<Property<'a>>> {
    let mut cursor = Cursor { b: bytes, pos: 0, wide: false };
    let mut records = Vec::new();
    loop {
        if !keep_going() {
            return Err(Error::Cancelled);
        }
        let start = cursor.pos;
        let record = (|| {
            budget.work(8)?;
            let kind = cursor.u32().map_err(|_| StopReason::Malformed)?;
            let declared_length = cursor.u32().map_err(|_| StopReason::Malformed)?;
            if kind == 0 {
                return if declared_length == 0 { Ok(None) } else { Err(StopReason::Malformed) };
            }
            if records.len() >= limits.properties {
                return Err(StopReason::RecordLimit);
            }
            let length = declared_length as usize;
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
            Ok(Some(property)) => {
                if let Err(reason) = budget.retain(&mut records, property) {
                    return Ok(Scan::stopped(records, start, reason));
                }
            }
            Err(reason) => return Ok(Scan::stopped(records, start, reason)),
        }
    }
}

/// Decode one length-framed version-18+ path at an absolute source offset.
pub fn standalone<'a>(
    bytes: &'a [u8],
    offset: usize,
    source_index: usize,
    limits: Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
) -> Result<Scan<SourcePath<'a>>> {
    let mut points_left = limits.points;
    standalone_with_points(bytes, offset, source_index, limits, budget, keep_going, &mut points_left)
}

fn standalone_with_points<'a>(
    bytes: &'a [u8],
    offset: usize,
    source_index: usize,
    limits: Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
    points_left: &mut usize,
) -> Result<Scan<SourcePath<'a>>> {
    let retained = budget.retained_left;
    let mut reader =
        Reader { cursor: Cursor { b: bytes, pos: offset, wide: false }, source_offset: 0, limits, points_left: limits.points, budget, keep_going };
    let path = (|| {
        reader.check()?;
        if limits.paths == 0 {
            return Err(StopReason::RecordLimit.into());
        }
        let name = reader.name()?;
        let length = reader.word()? as usize;
        if length > limits.payload_bytes {
            return Err(StopReason::PayloadLimit.into());
        }
        let payload_start = reader.cursor.pos;
        let raw = reader.take(length)?;
        let scan = local_properties(raw, limits, reader.budget, keep_going).map_err(|_| Failure::Cancelled)?;
        if let Some(diagnostic) = scan.stopped {
            return Err(diagnostic.reason.into());
        }
        let mut properties = scan.records;
        let mut tattoo = None;
        let mut visible = 0;
        let mut selected = false;
        let mut invalid_selection_marker = false;
        for property in &mut properties {
            reader.check()?;
            property.offset = property.offset.checked_add(payload_start).ok_or(Failure::Stop(StopReason::Malformed))?;
            match property.kind {
                8 if property.payload.len() == 4 => visible = crate::be_u32(property.payload).unwrap_or(0),
                8 => return Err(StopReason::Malformed.into()),
                20 if property.payload.len() == 4 => tattoo = crate::be_u32(property.payload).filter(|value| *value != 0),
                20 => return Err(StopReason::Malformed.into()),
                43 if property.payload.is_empty() => selected = true,
                43 => invalid_selection_marker = true,
                _ => {}
            }
        }
        let end = reader.cursor.pos;
        let mut inner = Reader {
            cursor: Cursor { b: raw, pos: scan.end_offset, wide: false },
            source_offset: payload_start,
            limits,
            points_left: (*points_left).min(limits.points),
            budget: &mut *reader.budget,
            keep_going,
        };
        if inner.word()? != 1 {
            return Err(StopReason::Malformed.into());
        }
        let count = inner.count(limits.strokes)?;
        let strokes = inner.strokes(count)?;
        if inner.cursor.pos != raw.len() {
            return Err(StopReason::Malformed.into());
        }
        let raw = bytes.get(offset..end).ok_or(Failure::Stop(StopReason::Malformed))?;
        let mut records = Vec::new();
        inner.budget.retain(
            &mut records,
            SourcePath {
                source_index,
                offset,
                name,
                tattoo,
                selected,
                invalid_selection_marker,
                visible,
                linked: 0,
                legacy_state: None,
                legacy_version: None,
                properties,
                parasites: &[],
                strokes,
                raw,
            },
        )?;
        *points_left = inner.points_left;
        Ok(Scan::complete(records, end))
    })();
    match path {
        Ok(scan) => Ok(scan),
        Err(Failure::Cancelled) => {
            reader.budget.retained_left = retained;
            Err(Error::Cancelled)
        }
        Err(Failure::Stop(reason)) => {
            reader.budget.retained_left = retained;
            Ok(Scan { records: Vec::new(), stopped: Some(Diagnostic { offset, ordinal: source_index, reason }), end_offset: offset })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Original synthetic wire records, authored from the public XCF layouts.
    fn word(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_be_bytes());
    }
    fn name(bytes: &mut Vec<u8>, value: &str) {
        word(bytes, value.len() as u32 + 1);
        bytes.extend(value.as_bytes());
        bytes.push(0);
    }
    fn cubic(bytes: &mut Vec<u8>, kind: u32, closed: bool) {
        for value in [kind, u32::from(closed), 3, 3] {
            word(bytes, value);
        }
        for (kind, x, y) in [(1, -4.5f32, 2.0f32), (0, 1.0, 3.0), (1, 6.0, 7.0)] {
            for value in [kind, x.to_bits(), y.to_bits(), 0.5f32.to_bits()] {
                word(bytes, value);
            }
        }
    }
    fn vector_path(bytes: &mut Vec<u8>, label: &str, tattoo: u32, kind: u32) {
        name(bytes, label);
        for value in [tattoo, 0, 0, 0, 1] {
            word(bytes, value);
        }
        cubic(bytes, kind, true);
    }
    fn vectors_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        for value in [1, 1, 2] {
            word(&mut bytes, value);
        }
        vector_path(&mut bytes, "first", 7, 1);
        vector_path(&mut bytes, "second", 7, 99);
        bytes
    }

    #[test]
    fn vector_source_identity_cubic_geometry_and_unknown_strokes_remain_distinct() {
        let bytes = vectors_bytes();
        let scan = vectors(&bytes, 100, Limits::default(), &mut super::super::Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        assert_eq!(scan.records.len(), 2);
        let first = &scan.records[0];
        assert_eq!((first.source_index, first.name, first.tattoo, first.selected), (0, "first", Some(7), false));
        let stroke = &first.strokes[0];
        assert!(stroke.closed && first.geometry_complete());
        assert_eq!(stroke.knots.len(), 1, "source geometry must not subdivide a single closed cubic for a native renderer");
        assert_eq!(stroke.knots[0].incoming.position, [-4.5, 2.0]);
        assert_eq!(stroke.knots[0].anchor.position, [1.0, 3.0]);
        assert_eq!(stroke.knots[0].outgoing.position, [6.0, 7.0]);
        assert_eq!(stroke.knots[0].anchor.auxiliary[0], 0.5);
        assert!(scan.records[1].selected);
        assert_eq!(scan.records[1].tattoo, Some(7), "do not guess which duplicate tattoo is authoritative");
        assert!(!scan.records[1].geometry_complete());
        assert_eq!(scan.records[1].strokes[0].kind, 99);
        assert!(!scan.records[1].strokes[0].raw.is_empty());
    }

    fn legacy_bytes(version: u32, closed: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        word(&mut bytes, 0);
        word(&mut bytes, 1);
        name(&mut bytes, "old");
        word(&mut bytes, 1);
        bytes.push(4);
        word(&mut bytes, u32::from(closed));
        word(&mut bytes, if closed { 3 } else { 2 });
        word(&mut bytes, version);
        if version >= 2 {
            word(&mut bytes, 0);
        }
        if version >= 3 {
            word(&mut bytes, 17);
        }
        for (kind, x, y) in [(1, -4, 2), (2, 6, 7), (2, 8, 9)].into_iter().take(if closed { 3 } else { 2 }) {
            word(&mut bytes, kind);
            word(&mut bytes, if version == 1 { x as u32 } else { (x as f32).to_bits() });
            word(&mut bytes, if version == 1 { y as u32 } else { (y as f32).to_bits() });
        }
        bytes
    }

    #[test]
    fn undecodable_legacy_names_keep_both_paths_and_exact_geometry() {
        for version in 1..=3 {
            let mut bytes = legacy_bytes(version, true);
            // Same wire length as "old": an original locale-encoded label.
            bytes[12..16].copy_from_slice(&[b'C', b'a', 0xe9, 0]);
            bytes[4..8].copy_from_slice(&2u32.to_be_bytes());
            bytes.extend_from_slice(&legacy_bytes(version, true)[8..]);
            let scan = legacy(&bytes, 97, Limits::default(), &mut Budget::default(), &|| true).unwrap();
            assert!(scan.stopped.is_none());
            assert_eq!(scan.records.len(), 2);
            let first = &scan.records[0];
            assert_eq!(first.offset, 105);
            assert_eq!(first.name_bytes(), &[b'C', b'a', 0xe9]);
            assert_eq!(first.decoded_name(), None);
            assert_eq!(first.name, "");
            assert_eq!(first.strokes[0].knots[0].anchor.position, [-4.0, 2.0]);
            assert_eq!(first.strokes[0].knots[0].outgoing.position, [6.0, 7.0]);
            assert!(first.strokes[0].closed);
            assert_eq!(scan.records[1].decoded_name(), Some("old"));
            assert_eq!(scan.records[1].strokes[0].knots[0].anchor.position, [-4.0, 2.0]);
            assert_eq!(first.name_bytes().as_ptr(), bytes[12..].as_ptr());
        }
        let mut modern = vectors_bytes();
        modern[16] = 0xff;
        let scan = vectors(&modern, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped.unwrap().reason, StopReason::Malformed);
        let mut bad = legacy_bytes(1, false);
        bad[15] = b'X';
        assert_eq!(legacy(&bad, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap().stopped.unwrap().reason, StopReason::Malformed);
        bad[15] = 0;
        bad[13] = 0;
        assert_eq!(legacy(&bad, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap().stopped.unwrap().reason, StopReason::Malformed);
    }

    #[test]
    fn legacy_versions_preserve_signed_coordinates_closure_and_source_flags() {
        for version in 1..=3 {
            for closed in [false, true] {
                let bytes = legacy_bytes(version, closed);
                let scan = legacy(&bytes, 0, Limits::default(), &mut super::super::Budget::default(), &|| true).unwrap();
                assert_eq!(scan.stopped, None);
                let path = &scan.records[0];
                assert!(path.selected);
                assert_eq!(path.linked, 1);
                assert_eq!(path.legacy_state, Some(4));
                assert_eq!(path.tattoo, if version == 3 { Some(17) } else { None });
                assert_eq!(path.strokes[0].closed, closed);
                let knot = &path.strokes[0].knots[0];
                assert_eq!(knot.anchor.position, [-4.0, 2.0]);
                assert_eq!(knot.incoming.position, if closed { [8.0, 9.0] } else { [-4.0, 2.0] });
            }
        }
    }

    #[test]
    fn standalone_property_identity_diagnostics_and_outer_eof_are_explicit() {
        let mut payload = Vec::new();
        for value in [20, 4, 31, 43, 0, 0, 0, 1, 1] {
            word(&mut payload, value);
        }
        cubic(&mut payload, 1, false);
        let mut bytes = vec![0; 7];
        name(&mut bytes, "standalone");
        word(&mut bytes, payload.len() as u32);
        bytes.extend(payload);
        let scan = standalone(&bytes, 7, 4, Limits::default(), &mut super::super::Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        assert_eq!((scan.records[0].source_index, scan.records[0].tattoo, scan.records[0].selected), (4, Some(31), true));
        let broken = standalone(&bytes[..bytes.len() - 1], 7, 4, Limits::default(), &mut super::super::Budget::default(), &|| true).unwrap();
        assert!(broken.records.is_empty());
        assert_eq!(broken.stopped.unwrap().reason, super::super::StopReason::Malformed);
    }

    #[test]
    fn limits_and_failed_geometry_preserve_prefix_and_refund_only_dropped_storage() {
        let mut bytes = vectors_bytes();
        // The second path allocates a valid first stroke before a later stroke fails.
        let last_header = bytes.len() - 48 - 16;
        bytes[last_header - 4..last_header].copy_from_slice(&2u32.to_be_bytes());
        bytes[last_header..last_header + 4].copy_from_slice(&1u32.to_be_bytes());
        let invalid = bytes.len();
        cubic(&mut bytes, 1, true);
        bytes[invalid + 12..invalid + 16].copy_from_slice(&2u32.to_be_bytes());
        let mut first = Vec::new();
        for value in [1, 1, 1] {
            word(&mut first, value);
        }
        vector_path(&mut first, "first", 7, 1);
        let mut first_budget = super::super::Budget::default();
        let first_scan = vectors(&first, 0, Limits::default(), &mut first_budget, &|| true).unwrap();
        assert_eq!(first_scan.stopped, None);
        let mut budget = super::super::Budget::default();
        let scan = vectors(&bytes, 0, Limits::default(), &mut budget, &|| true).unwrap();
        assert_eq!(scan.records.len(), 1);
        assert!(scan.stopped.is_some());
        assert_eq!(budget.retained_remaining(), first_budget.retained_remaining());
        assert!(budget.work_remaining() < first_budget.work_remaining());
        let spent = budget.work_remaining();
        let again = vectors(&bytes, 0, Limits { paths: 1, ..Limits::default() }, &mut budget, &|| true).unwrap();
        assert_eq!(again.records.len(), 1);
        assert!(again.stopped.is_some());
        assert!(budget.work_remaining() < spent);
        let mut none = super::super::Budget::new(1 << 20, 0);
        let valid = vectors_bytes();
        let scan = vectors(&valid, 0, Limits::default(), &mut none, &|| true).unwrap();
        assert!(scan.records.is_empty());
        assert_eq!(scan.stopped.unwrap().reason, super::super::StopReason::RetainedLimit);
        assert_eq!(none.retained_remaining(), 0);
    }

    #[test]
    fn path_cancellation_wins_with_spent_work_and_mid_geometry() {
        let bytes = vectors_bytes();
        assert!(matches!(vectors(&bytes, 0, Limits::default(), &mut super::super::Budget::new(0, 0), &|| false), Err(crate::Error::Cancelled)));
        let visits = std::cell::Cell::new(0);
        let keep = || {
            visits.set(visits.get() + 1);
            visits.get() < 30
        };
        assert!(matches!(vectors(&bytes, 0, Limits::default(), &mut super::super::Budget::default(), &keep), Err(crate::Error::Cancelled)));
    }

    #[test]
    fn finite_source_coordinates_do_not_inherit_native_geometry_clamps() {
        let mut bytes = Vec::new();
        for value in [1, 0, 1] {
            word(&mut bytes, value);
        }
        name(&mut bytes, "large");
        for value in [0, 0, 0, 0, 1] {
            word(&mut bytes, value);
        }
        let stroke = bytes.len();
        cubic(&mut bytes, 1, false);
        bytes[stroke + 20..stroke + 24].copy_from_slice(&f32::MAX.to_bits().to_be_bytes());
        let scan = vectors(&bytes, 0, Limits::default(), &mut super::super::Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        assert_eq!(scan.records[0].strokes[0].knots[0].incoming.position[0], f64::from(f32::MAX));
        bytes[stroke + 20..stroke + 24].copy_from_slice(&f32::NAN.to_bits().to_be_bytes());
        let scan = vectors(&bytes, 0, Limits::default(), &mut super::super::Budget::default(), &|| true).unwrap();
        assert!(scan.records.is_empty());
        assert_eq!(scan.stopped.unwrap().reason, super::super::StopReason::Malformed);
        let mut bytes = legacy_bytes(1, false);
        let x = bytes.len() - 24 + 4;
        bytes[x..x + 4].copy_from_slice(&i32::MIN.to_be_bytes());
        let scan = legacy(&bytes, 0, Limits::default(), &mut super::super::Budget::default(), &|| true).unwrap();
        assert_eq!(scan.stopped, None);
        assert_eq!(scan.records[0].strokes[0].knots[0].anchor.position[0], f64::from(i32::MIN));
    }

    #[test]
    fn truncated_sources_and_hostile_counts_stop_without_unbounded_geometry() {
        let modern = vectors_bytes();
        let old = legacy_bytes(3, true);
        for (bytes, is_modern) in [(&modern, true), (&old, false)] {
            for end in 0..bytes.len() {
                let mut budget = super::super::Budget::new(4096, 4096);
                let scan = if is_modern {
                    vectors(&bytes[..end], 0, Limits::default(), &mut budget, &|| true)
                } else {
                    legacy(&bytes[..end], 0, Limits::default(), &mut budget, &|| true)
                }
                .unwrap();
                assert!(scan.stopped.is_some(), "truncated source accepted at {end}");
            }
        }
        let mut hostile = modern.clone();
        hostile[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        let scan = vectors(&hostile, 0, Limits { paths: 1, ..Limits::default() }, &mut super::super::Budget::default(), &|| true).unwrap();
        assert_eq!(scan.records.len(), 1);
        assert_eq!(scan.stopped.unwrap().reason, super::super::StopReason::RecordLimit);
        let scan = standalone(&[], usize::MAX, 0, Limits::default(), &mut super::super::Budget::default(), &|| true).unwrap();
        assert!(scan.records.is_empty());
        assert_eq!(scan.stopped.unwrap().reason, super::super::StopReason::Malformed);
    }
    #[test]
    fn cancelled_direct_path_readers_release_partial_geometry_and_allow_retry() {
        let mut payload = Vec::new();
        for value in [20, 4, 31, 0, 0, 1, 1] {
            word(&mut payload, value);
        }
        cubic(&mut payload, 1, false);
        let mut standalone_bytes = Vec::new();
        name(&mut standalone_bytes, "path");
        word(&mut standalone_bytes, payload.len() as u32);
        standalone_bytes.extend(payload);
        for (kind, bytes) in [(0, legacy_bytes(3, true)), (1, vectors_bytes()), (2, standalone_bytes)] {
            let read = |budget: &mut Budget, keep: &dyn Fn() -> bool| match kind {
                0 => legacy(&bytes, 0, Limits::default(), budget, keep),
                1 => vectors(&bytes, 0, Limits::default(), budget, keep),
                _ => standalone(&bytes, 0, 0, Limits::default(), budget, keep),
            };
            let mut charged_cancellations = 0;
            for stop in 2..100 {
                let calls = std::cell::Cell::new(0);
                let keep = || {
                    calls.set(calls.get() + 1);
                    calls.get() < stop
                };
                let mut budget = Budget::new(1 << 20, 1 << 20);
                if matches!(read(&mut budget, &keep), Err(Error::Cancelled)) {
                    assert_eq!(budget.retained_remaining(), 1 << 20, "kind {kind}, checkpoint {stop}");
                    if budget.work_remaining() < 1 << 20 {
                        charged_cancellations += 1;
                    }
                    assert!(read(&mut budget, &|| true).unwrap().stopped.is_none());
                }
            }
            assert!(charged_cancellations > 3);
        }
    }
    #[test]
    fn stroke_limit_is_shared_across_each_historical_source_list() {
        let mut legacy_two = legacy_bytes(3, true);
        legacy_two[4..8].copy_from_slice(&2u32.to_be_bytes());
        legacy_two.extend_from_slice(&legacy_bytes(3, true)[8..]);
        let mut unknown_first = Vec::new();
        for value in [1, 0, 2] {
            word(&mut unknown_first, value);
        }
        vector_path(&mut unknown_first, "unknown", 1, 99);
        vector_path(&mut unknown_first, "known", 2, 1);
        for (modern, bytes) in [(false, legacy_two), (true, vectors_bytes()), (true, unknown_first)] {
            for strokes in [0, 1, 2] {
                let scan = read_list(&bytes, 40, modern, Limits { strokes, ..Default::default() }, &mut Budget::default(), &|| true).unwrap();
                assert_eq!(scan.records.len(), strokes);
                assert_eq!(scan.stopped.map(|diagnostic| diagnostic.reason), (strokes < 2).then_some(StopReason::RecordLimit));
                if strokes == 1 {
                    assert_eq!(scan.records[0].strokes.len(), 1);
                    assert_eq!(scan.stopped.unwrap().ordinal, 1);
                }
            }
        }
    }

    #[test]
    fn vector_parasites_accessor_borrows_the_original_ordered_framed_span() {
        let mut bytes = Vec::new();
        for value in [1, 0, 1] {
            word(&mut bytes, value);
        }
        name(&mut bytes, "vector");
        for value in [1, 1, 0, 2, 0] {
            word(&mut bytes, value);
        }
        let start = bytes.len();
        for (flag, value) in [(1, b"first".as_slice()), (2, b"last".as_slice())] {
            name(&mut bytes, "same");
            word(&mut bytes, flag);
            word(&mut bytes, value.len() as u32);
            bytes.extend(value);
        }
        let scan = vectors(&bytes, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert!(scan.stopped.is_none());
        let payloads = scan.records[0].parasite_payloads().collect::<Vec<_>>();
        assert_eq!(payloads, [&bytes[start..]]);
        assert_eq!(payloads[0].as_ptr(), bytes[start..].as_ptr());
        let shifted = vectors(&bytes, 113, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        let span = shifted.records[0].parasite_spans().next().unwrap();
        assert_eq!((span.offset, span.bytes.as_ptr()), (113 + start, payloads[0].as_ptr()));
        let decoded = super::super::parasites(span.bytes, span.offset, Default::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(decoded.records[0].offset, span.offset);
        assert_eq!(decoded.records[1].offset, span.offset + decoded.records[0].raw.len());
        let records = super::super::parasites(payloads[0], start, Default::default(), &mut Budget::default(), &|| true).unwrap();
        assert!(records.stopped.is_none());
        assert_eq!(
            records.records.iter().map(|record| (record.name, record.flags, record.payload)).collect::<Vec<_>>(),
            [(b"same".as_slice(), 1, b"first".as_slice()), (b"same".as_slice(), 2, b"last".as_slice())]
        );
        let empty = legacy_bytes(3, true);
        let legacy = legacy(&empty, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(legacy.records[0].parasite_payloads().count(), 0);
    }

    #[test]
    fn standalone_parasite_spans_preserve_duplicate_empty_payloads_and_offsets() {
        let mut payload = Vec::new();
        let mut starts = Vec::new();
        for body in [b"".as_slice(), b"opaque", b""] {
            word(&mut payload, 21);
            word(&mut payload, body.len() as u32);
            starts.push(payload.len());
            payload.extend_from_slice(body);
        }
        for value in [0, 0, 1, 0] {
            word(&mut payload, value);
        }
        let mut bytes = vec![0; 19];
        name(&mut bytes, "standalone");
        word(&mut bytes, payload.len() as u32);
        let base = bytes.len();
        bytes.extend(payload);
        let scan = standalone(&bytes, 19, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert!(scan.stopped.is_none());
        let path = &scan.records[0];
        let spans = path.parasite_spans().collect::<Vec<_>>();
        assert_eq!(spans.len(), 3);
        assert_eq!(path.parasite_payloads().collect::<Vec<_>>(), spans.iter().map(|span| span.bytes).collect::<Vec<_>>());
        for (span, start) in spans.iter().zip(starts) {
            assert_eq!(span.offset, base + start);
            assert_eq!(span.bytes.as_ptr(), bytes[span.offset..].as_ptr());
        }
        assert!(spans[0].bytes.is_empty() && spans[2].bytes.is_empty());
        assert_eq!(spans[1].bytes, b"opaque");
        assert_eq!(path.decoded_name(), Some("standalone"));
    }
    #[test]
    fn overflowing_source_spans_are_rejected_before_path_retention() {
        for modern in [false, true] {
            let bytes = if modern { vectors_bytes() } else { legacy_bytes(3, true) };
            let mut budget = Budget::default();
            let retained = budget.retained_remaining();
            let work = budget.work_remaining();
            let base = usize::MAX - if modern { 12 } else { 8 };
            assert!(matches!(read_list(&bytes, base, modern, Limits::default(), &mut budget, &|| true), Err(Error::Invalid(_))));
            assert_eq!(budget.retained_remaining(), retained);
            assert_eq!(budget.work_remaining(), work, "no source parsing occurred");
            assert!(matches!(read_list(&bytes, base, modern, Limits::default(), &mut budget, &|| false), Err(Error::Cancelled)));
            let exact = read_list(&bytes, usize::MAX - bytes.len(), modern, Limits::default(), &mut budget, &|| true).unwrap();
            assert!(exact.stopped.is_none());
            assert_eq!(exact.end_offset, usize::MAX);
            assert!(!exact.records.is_empty());
        }
    }

    #[test]
    fn unknown_tags_keep_only_bezier_framing_and_decline_axes_only_layouts() {
        let mut framed = Vec::new();
        for value in [1, 0, 1] {
            word(&mut framed, value);
        }
        name(&mut framed, "unknown then Bezier");
        for value in [1, 0, 0, 0, 2] {
            word(&mut framed, value);
        }
        let unknown_start = framed.len();
        cubic(&mut framed, 99, false);
        let known_start = framed.len();
        cubic(&mut framed, 1, false);
        let complete = vectors(&framed, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert!(complete.stopped.is_none());
        let strokes = &complete.records[0].strokes;
        assert_eq!(strokes.len(), 2);
        assert!(strokes[0].knots.is_empty());
        assert_eq!(strokes[0].raw, &framed[unknown_start..known_start]);
        assert_eq!(strokes[1].raw, &framed[known_start..]);
        assert_eq!(strokes[1].knots[0].anchor.position, [1., 3.]);

        let mut incompatible = Vec::new();
        for value in [1, 0, 2] {
            word(&mut incompatible, value);
        }
        vector_path(&mut incompatible, "retained prefix", 1, 1);
        let rejected = incompatible.len();
        name(&mut incompatible, "unframed unknown");
        for value in [2, 0, 0, 0, 2, 99, 0, 2, 1] {
            word(&mut incompatible, value);
        }
        // The public specification does not define unknown kinds. This alternative
        // axes-only layout is intentionally outside the supported framing subset.
        word(&mut incompatible, 2f32.to_bits());
        word(&mut incompatible, 3f32.to_bits());
        cubic(&mut incompatible, 1, false);
        let payload = incompatible.as_slice();
        let scan = vectors(payload, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(scan.records.len(), 1);
        assert_eq!(scan.stopped, Some(Diagnostic { offset: rejected, ordinal: 1, reason: StopReason::Malformed }));
        assert_eq!(scan.records[0].raw, &payload[12..rejected]);
        // The caller still owns the original bounded raw payload, including the
        // unhandled frame; the parser does not claim decoded geometry for it.
        assert!(!payload[rejected..].is_empty());
    }
    #[test]
    fn opaque_vector_parasite_names_preserve_geometry_and_raw_evidence() {
        let mut bytes = Vec::new();
        for value in [1, 0, 1] {
            word(&mut bytes, value);
        }
        name(&mut bytes, "");
        for value in [7, 1, 0, 1, 1, 2] {
            word(&mut bytes, value);
        }
        bytes.extend([0xff, 0]);
        word(&mut bytes, 5);
        word(&mut bytes, 3);
        bytes.extend(b"raw");
        cubic(&mut bytes, 1, true);
        let scan = vectors(&bytes, 0, Limits { name_bytes: 2, ..Default::default() }, &mut Budget::default(), &|| true).unwrap();
        assert!(scan.stopped.is_none());
        assert_eq!(scan.records[0].strokes[0].knots[0].anchor.position, [1., 3.]);
        let payload = scan.records[0].parasite_payloads().next().unwrap();
        let raw = super::super::parasites(payload, 0, Default::default(), &mut Budget::default(), &|| true).unwrap();
        assert!(raw.stopped.is_none());
        assert_eq!(raw.records[0].name, &[0xff]);
        assert_eq!(raw.records[0].flags, 5);
        assert_eq!(raw.records[0].payload, b"raw");
        let limited = vectors(&bytes, 0, Limits { name_bytes: 1, ..Default::default() }, &mut Budget::default(), &|| true).unwrap();
        assert_eq!(limited.stopped.unwrap().reason, StopReason::StringLimit);
        // A path label remains UTF-8 even when opaque parasite names are accepted.
        let mut invalid_label = bytes;
        invalid_label[12..16].copy_from_slice(&2u32.to_be_bytes());
        invalid_label.insert(16, 0xff);
        let label = vectors(&invalid_label, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
        assert_eq!(label.stopped.unwrap().reason, StopReason::Malformed);
    }
    #[test]
    fn opaque_vector_parasite_names_require_only_the_wire_terminator() {
        for raw_name in [b"".as_slice(), &[0xff, 0][..], &[0xff, b'A'][..]] {
            let mut bytes = Vec::new();
            for value in [1, 0, 1] {
                word(&mut bytes, value);
            }
            name(&mut bytes, "path");
            for value in [1, 1, 0, 1, 1] {
                word(&mut bytes, value);
            }
            word(&mut bytes, raw_name.len() as u32);
            bytes.extend(raw_name);
            for value in [7, 0] {
                word(&mut bytes, value);
            }
            cubic(&mut bytes, 1, true);
            let scan = vectors(&bytes, 0, Limits::default(), &mut Budget::default(), &|| true).unwrap();
            if raw_name.last().is_some_and(|byte| *byte != 0) {
                assert!(scan.records.is_empty());
                assert_eq!(scan.stopped.unwrap().reason, StopReason::Malformed);
            } else {
                assert!(scan.stopped.is_none());
                assert_eq!(scan.records[0].strokes[0].knots.len(), 1);
                let payload = scan.records[0].parasite_payloads().next().unwrap();
                let raw = super::super::parasites(payload, 0, Default::default(), &mut Budget::default(), &|| true).unwrap();
                assert_eq!(raw.records[0].name, raw_name.strip_suffix(&[0]).unwrap_or(raw_name));
            }
        }
    }
    #[test]
    fn standalone_point_allowance_honors_both_limits_and_commits_only_on_success() {
        let mut payload = Vec::new();
        for value in [0, 0, 1, 1] {
            word(&mut payload, value);
        }
        cubic(&mut payload, 1, false);
        let mut bytes = Vec::new();
        name(&mut bytes, "path");
        word(&mut bytes, payload.len() as u32);
        bytes.extend(payload);
        for (local, incoming, admitted) in [(3, 2, false), (2, 3, false), (3, 3, true), (6, 6, true)] {
            let mut remaining = incoming;
            let mut budget = Budget::default();
            let retained = budget.retained_remaining();
            let work = budget.work_remaining();
            let scan =
                standalone_with_points(&bytes, 0, 0, Limits { points: local, ..Default::default() }, &mut budget, &|| true, &mut remaining).unwrap();
            assert!(budget.work_remaining() < work);
            if admitted {
                assert!(scan.stopped.is_none());
                assert_eq!(scan.records[0].strokes[0].knots.len(), 1);
                assert_eq!(remaining, incoming.min(local) - 3);
                assert!(budget.retained_remaining() < retained);
            } else {
                assert!(scan.records.is_empty());
                assert_eq!(scan.stopped.unwrap().reason, StopReason::RecordLimit);
                assert_eq!(remaining, incoming, "rejected geometry does not commit a point debit");
                assert_eq!(budget.retained_remaining(), retained);
            }
        }
    }
}
