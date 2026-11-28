//! Typed, inert source metadata over the ordered raw-record scans.
//!
//! No native document/view policy or XML/profile interpretation lives here.
//! Values are the last valid values in the inspected prefix; callers must check
//! completeness before treating them as final in the presence of duplicates.

mod channel;
pub use channel::{ChannelColorIndicates, ChannelMetadata, channel, channel_from_properties};

use super::{Budget, Diagnostic, Limits as RecordLimits, Property, Scan, StopReason};
use crate::{Error, Guide, Orientation, Result};

/// An interpreted value and the absolute source offset that supplied it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Evidence<T> {
    pub offset: usize,
    pub value: T,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resolution {
    pub horizontal: f32,
    pub vertical: f32,
}

/// Source unit definition, including unknown display-unit codes.
/// The strings are borrowed field bytes, without their terminal NUL.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UserUnit<'a> {
    pub factor: f32,
    pub digits: u32,
    pub strings: [Option<&'a [u8]>; 5],
}

/// Signed source positions; canvas clipping and native sampler limits belong to consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SamplePoint {
    pub x: i32,
    pub y: i32,
    /// Modern pick mode and padding are preserved, not executed or normalized.
    pub pick_mode: Option<u32>,
    pub reserved: [u32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketKind {
    Comment,
    Exif,
    ImageMetadata,
    Metadata,
}

/// A known packet, still uninterpreted. Creator/provenance fields inside XML or
/// EXIF remain source bytes; unknown parasites remain in the original raw scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Packet<'a> {
    pub kind: PacketKind,
    pub flags: u32,
    pub bytes: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueKind {
    InvalidValue,
    PartialRecord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Issue {
    pub offset: usize,
    pub property_kind: u32,
    pub kind: IssueKind,
}

/// Per-result record limits. Cumulative work/storage use the caller's shared Budget.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub guides: usize,
    pub sample_points: usize,
    pub packets: usize,
    pub issues: usize,
    pub records: RecordLimits,
}

impl Default for Limits {
    fn default() -> Self {
        Self { guides: 65_536, sample_points: 65_536, packets: 4096, issues: 64, records: RecordLimits::default() }
    }
}

#[derive(Debug, Default)]
pub struct ImageMetadata<'a> {
    pub resolution: Option<Evidence<Resolution>>,
    pub unit: Option<Evidence<u32>>,
    pub user_unit: Option<Evidence<UserUnit<'a>>>,
    pub guides: Vec<Evidence<Guide>>,
    pub legacy_sample_points: Vec<Evidence<SamplePoint>>,
    pub sample_points: Vec<Evidence<SamplePoint>>,
    /// A complete record, or an empty modern property, establishes modern evidence.
    /// Invalid short modern properties do not erase eligible legacy evidence.
    pub has_modern_sample_points: bool,
    /// Nonempty `icc-profile` bytes only; ICC validity and color policy are not inferred.
    pub profile: Option<Evidence<&'a [u8]>>,
    /// Where the raw parasite scan stopped. Typed conversion can stop earlier;
    /// this remains the raw diagnostic even then. `stopped` identifies the outer property.
    pub parasite_stopped: Option<Diagnostic>,
    /// First parasite omitted by typed conversion, with its source offset and ordinal.
    /// None when conversion consumes the available raw prefix; `parasite_stopped`
    /// can still identify a later malformed or quota-limited raw suffix.
    pub parasite_typed_stopped: Option<Diagnostic>,
    pub packets: Vec<Evidence<Packet<'a>>>,
    pub issues: Vec<Issue>,
    pub source_stopped: Option<Diagnostic>,
    pub stopped: Option<Diagnostic>,
}

impl ImageMetadata<'_> {
    pub fn complete(&self) -> bool {
        self.source_stopped.is_none() && self.stopped.is_none() && self.parasite_stopped.is_none() && self.parasite_typed_stopped.is_none()
    }
}

#[derive(Debug, Default)]
pub struct LayerMetadata {
    /// `PROP_ACTIVE_LAYER` source marker, with no choice of native target/identity.
    pub selected: Option<Evidence<()>>,
    pub edit_mask: Option<Evidence<u32>>,
    pub show_mask: Option<Evidence<u32>>,
    pub issues: Vec<Issue>,
    pub source_stopped: Option<Diagnostic>,
    pub stopped: Option<Diagnostic>,
}

impl LayerMetadata {
    pub fn complete(&self) -> bool {
        self.source_stopped.is_none() && self.stopped.is_none()
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

struct Reader<'a> {
    limits: Limits,
    budget: &'a mut Budget,
    keep_going: &'a dyn Fn() -> bool,
}

impl Reader<'_> {
    fn check(&self) -> ReadResult<()> {
        if (self.keep_going)() { Ok(()) } else { Err(Failure::Cancelled) }
    }

    fn charge(&mut self, bytes: usize) -> ReadResult<()> {
        self.check()?;
        self.budget.work(bytes)?;
        Ok(())
    }

    fn retain<T>(&mut self, output: &mut Vec<T>, value: T, limit: usize) -> ReadResult<()> {
        self.check()?;
        if output.len() >= limit {
            return Err(StopReason::RecordLimit.into());
        }
        self.budget.retain(output, value)?;
        Ok(())
    }

    fn issue(&mut self, issues: &mut Vec<Issue>, property: &Property<'_>, kind: IssueKind) -> ReadResult<()> {
        self.retain(issues, Issue { offset: property.offset, property_kind: property.kind, kind }, self.limits.issues)
    }

    fn parasite_name_terminated(&mut self, record: &super::Parasite<'_>) -> ReadResult<bool> {
        // The raw scanner deliberately keeps malformed names. Check the original field,
        // not the stripped alias, before it can replace recognized typed evidence.
        self.charge(4)?;
        let Some(length) = record.raw.first_chunk::<4>().copied().map(u32::from_be_bytes) else { return Ok(false) };
        if length == 0 {
            return Ok(true);
        }
        self.charge(1)?;
        Ok(record.raw.get(4..).and_then(|bytes| bytes.get(..length as usize)).is_some_and(|name| name.last() == Some(&0)))
    }

    fn image_parasite<'a>(&mut self, property: &Property<'a>, record: &super::Parasite<'a>, output: &mut ImageMetadata<'a>) -> ReadResult<()> {
        if !self.parasite_name_terminated(record)? {
            return self.issue(&mut output.issues, property, IssueKind::InvalidValue);
        }
        if record.name == b"icc-profile" {
            if record.payload.is_empty() {
                self.issue(&mut output.issues, property, IssueKind::InvalidValue)?;
            } else {
                output.profile = Some(Evidence { offset: record.offset, value: record.payload });
            }
            return Ok(());
        }
        let kind = match record.name {
            b"gimp-comment" => PacketKind::Comment,
            b"exif-data" => PacketKind::Exif,
            b"gimp-image-metadata" => PacketKind::ImageMetadata,
            b"gimp-metadata" => PacketKind::Metadata,
            _ => return Ok(()),
        };
        let value = Evidence { offset: record.offset, value: Packet { kind, flags: record.flags, bytes: record.payload } };
        self.retain(&mut output.packets, value, self.limits.packets)
    }

    fn image_property<'a>(&mut self, property: &Property<'a>, output: &mut ImageMetadata<'a>) -> ReadResult<()> {
        // Parsing borrowed typed data is additional work after raw framing.
        // Parasites use their existing bounded scanner and its own work admission.
        self.charge(8)?;
        if matches!(property.kind, 18 | 19 | 22 | 24 | 27 | 39) {
            self.charge(property.payload.len())?;
        }
        let data = property.payload;
        match property.kind {
            19 => {
                let resolution = if let Some(&[x, y]) = data.as_chunks::<4>().0.as_array::<2>().filter(|_| data.len() == 8) {
                    let (horizontal, vertical) = (f32::from_be_bytes(x), f32::from_be_bytes(y));
                    [horizontal, vertical].into_iter().all(|axis| axis.is_finite() && axis > 0.).then_some(Resolution { horizontal, vertical })
                } else {
                    None
                };
                if let Some(value) = resolution {
                    output.resolution = Some(Evidence { offset: property.offset, value });
                } else {
                    self.issue(&mut output.issues, property, IssueKind::InvalidValue)?;
                }
            }
            22 => {
                if let Some(word) = word(data) {
                    output.unit = Some(Evidence { offset: property.offset, value: word });
                } else {
                    self.issue(&mut output.issues, property, IssueKind::InvalidValue)?;
                }
            }
            24 => {
                if let Some(value) = user_unit(data, self.limits.records.string_bytes)? {
                    output.user_unit = Some(Evidence { offset: property.offset, value });
                } else {
                    self.issue(&mut output.issues, property, IssueKind::InvalidValue)?;
                }
            }
            18 => {
                let (records, tail) = data.as_chunks::<5>();
                for (index, &[a, b, c, d, axis]) in records.iter().enumerate() {
                    self.check()?;
                    let offset = record_offset(property, index, 5)?;
                    let orientation = match axis {
                        1 => Orientation::Horizontal,
                        2 => Orientation::Vertical,
                        _ => {
                            self.issue(&mut output.issues, property, IssueKind::InvalidValue)?;
                            continue;
                        }
                    };
                    // Preserve source off-canvas/legacy positions. The caller decides
                    // whether its document model can represent them.
                    let value = Guide { position: i32::from_be_bytes([a, b, c, d]), orientation };
                    self.retain(&mut output.guides, Evidence { offset, value }, self.limits.guides)?;
                }
                if !tail.is_empty() {
                    self.issue(&mut output.issues, property, IssueKind::PartialRecord)?;
                }
            }
            27 | 39 => {
                let modern = property.kind == 39;
                let stride = if modern { 20 } else { 8 };
                let records = data.chunks_exact(stride);
                if modern && (data.is_empty() || data.len() >= stride) {
                    output.has_modern_sample_points = true;
                }
                let partial_tail = !records.remainder().is_empty();
                for (index, record) in records.enumerate() {
                    self.check()?;
                    let offset = record_offset(property, index, stride)?;
                    let words = record.as_chunks::<4>().0;
                    let x = words.first().copied().ok_or(StopReason::Malformed)?;
                    let y = words.get(1).copied().ok_or(StopReason::Malformed)?;
                    let (pick_mode, reserved) = if modern {
                        let mode = words.get(2).copied().ok_or(StopReason::Malformed)?;
                        let a = words.get(3).copied().ok_or(StopReason::Malformed)?;
                        let b = words.get(4).copied().ok_or(StopReason::Malformed)?;
                        (Some(u32::from_be_bytes(mode)), [u32::from_be_bytes(a), u32::from_be_bytes(b)])
                    } else {
                        (None, [0; 2])
                    };
                    let value = Evidence { offset, value: SamplePoint { x: i32::from_be_bytes(x), y: i32::from_be_bytes(y), pick_mode, reserved } };
                    if output.sample_points.len().saturating_add(output.legacy_sample_points.len()) >= self.limits.sample_points {
                        return Err(StopReason::RecordLimit.into());
                    }
                    let target = if modern { &mut output.sample_points } else { &mut output.legacy_sample_points };
                    self.retain(target, value, self.limits.sample_points)?;
                }
                if partial_tail {
                    self.issue(&mut output.issues, property, IssueKind::PartialRecord)?;
                }
            }
            21 => {
                let offset = property.offset.checked_add(8).ok_or(StopReason::Malformed)?;
                let retained = self.budget.retained_left;
                let scan = super::parasites(data, offset, self.limits.records, self.budget, self.keep_going).map_err(|error| match error {
                    Error::Cancelled => Failure::Cancelled,
                    Error::Invalid(_) => Failure::Stop(StopReason::Malformed),
                })?;
                let scan_charge = retained - self.budget.retained_left;
                output.parasite_stopped = scan.stopped;
                let result: ReadResult<()> = (|| {
                    for (ordinal, record) in scan.records.iter().enumerate() {
                        self.check()?;
                        let converted = self.image_parasite(property, record, output);
                        if let Err(Failure::Stop(reason)) = &converted {
                            output.parasite_typed_stopped = Some(Diagnostic { offset: record.offset, ordinal, reason: *reason });
                        }
                        converted?;
                    }
                    if let Some(stopped) = scan.stopped {
                        return Err(stopped.reason.into());
                    }
                    Ok(())
                })();
                // Only the temporary borrowed-record vector is dropped here;
                // packets/issues already returned remain charged, as does all work.
                drop(scan);
                self.budget.retained_left += scan_charge;
                result?;
            }
            _ => {}
        }
        Ok(())
    }
}

/// Decode supported image metadata without changing legacy `Image` fields.
/// Raw source scans remain the authority for unknown fields and original bytes.
pub fn image<'a>(properties: &Scan<Property<'a>>, limits: Limits, budget: &mut Budget, keep_going: &dyn Fn() -> bool) -> Result<ImageMetadata<'a>> {
    budget.cancel_safe(|budget| image_read(properties, limits, budget, keep_going))
}

fn image_read<'a>(properties: &Scan<Property<'a>>, limits: Limits, budget: &mut Budget, keep_going: &dyn Fn() -> bool) -> Result<ImageMetadata<'a>> {
    let mut reader = Reader { limits, budget, keep_going };
    reader.check().map_err(|_| Error::Cancelled)?;
    let mut output = ImageMetadata { source_stopped: properties.stopped, ..Default::default() };
    for (ordinal, property) in properties.records.iter().enumerate() {
        reader.check().map_err(|_| Error::Cancelled)?;
        let reason = if ordinal >= limits.records.records {
            Some(StopReason::RecordLimit)
        } else if property.payload.len() > limits.records.payload_bytes {
            Some(StopReason::PayloadLimit)
        } else {
            None
        };
        if let Some(reason) = reason {
            output.stopped = Some(Diagnostic { offset: property.offset, ordinal, reason });
            break;
        }
        match reader.image_property(property, &mut output) {
            Ok(()) => {}
            Err(Failure::Cancelled) => return Err(Error::Cancelled),
            Err(Failure::Stop(reason)) => {
                output.stopped = Some(Diagnostic { offset: property.offset, ordinal, reason });
                break;
            }
        }
    }
    reader.check().map_err(|_| Error::Cancelled)?;
    Ok(output)
}

/// Decode source layer-selection/mask markers. No native IDs or active-target
/// arbitration are inferred; all complete empty selection markers are evidence.
pub fn layer(properties: &Scan<Property<'_>>, limits: Limits, budget: &mut Budget, keep_going: &dyn Fn() -> bool) -> Result<LayerMetadata> {
    budget.cancel_safe(|budget| layer_read(properties, limits, budget, keep_going))
}

fn layer_read(properties: &Scan<Property<'_>>, limits: Limits, budget: &mut Budget, keep_going: &dyn Fn() -> bool) -> Result<LayerMetadata> {
    let mut reader = Reader { limits, budget, keep_going };
    reader.check().map_err(|_| Error::Cancelled)?;
    let mut output = LayerMetadata { source_stopped: properties.stopped, ..Default::default() };
    for (ordinal, property) in properties.records.iter().enumerate() {
        reader.check().map_err(|_| Error::Cancelled)?;
        let reason = if ordinal >= limits.records.records {
            Some(StopReason::RecordLimit)
        } else if property.payload.len() > limits.records.payload_bytes {
            Some(StopReason::PayloadLimit)
        } else {
            None
        };
        if let Some(reason) = reason {
            output.stopped = Some(Diagnostic { offset: property.offset, ordinal, reason });
            break;
        }
        let result = (|| {
            reader.charge(8)?;
            if matches!(property.kind, 2 | 12 | 13) {
                reader.charge(property.payload.len())?;
            }
            match property.kind {
                2 if property.payload.is_empty() => output.selected = Some(Evidence { offset: property.offset, value: () }),
                12 | 13 => {
                    if let Some(value) = word(property.payload) {
                        let target = if property.kind == 12 { &mut output.edit_mask } else { &mut output.show_mask };
                        *target = Some(Evidence { offset: property.offset, value });
                    } else {
                        reader.issue(&mut output.issues, property, IssueKind::InvalidValue)?;
                    }
                }
                2 => reader.issue(&mut output.issues, property, IssueKind::InvalidValue)?,
                _ => {}
            }
            Ok(())
        })();
        match result {
            Ok(()) => {}
            Err(Failure::Cancelled) => return Err(Error::Cancelled),
            Err(Failure::Stop(reason)) => {
                output.stopped = Some(Diagnostic { offset: property.offset, ordinal, reason });
                break;
            }
        }
    }
    reader.check().map_err(|_| Error::Cancelled)?;
    Ok(output)
}

fn record_offset(property: &Property<'_>, index: usize, stride: usize) -> ReadResult<usize> {
    index
        .checked_mul(stride)
        .and_then(|offset| offset.checked_add(8))
        .and_then(|offset| property.offset.checked_add(offset))
        .ok_or(StopReason::Malformed.into())
}

fn word(bytes: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(*bytes.as_array::<4>()?))
}

fn user_unit(bytes: &[u8], string_limit: usize) -> ReadResult<Option<UserUnit<'_>>> {
    let Some(factor) = bytes.first_chunk::<4>().copied().map(f32::from_be_bytes) else { return Ok(None) };
    let Some(digits) = bytes.get(4..8).and_then(word) else { return Ok(None) };
    let mut valid = factor.is_finite() && factor > 0.;
    let Some(mut remaining) = bytes.get(8..) else { return Ok(None) };
    let mut strings = [None; 5];
    let mut count = 0;
    for slot in &mut strings {
        if remaining.is_empty() {
            break;
        }
        let Some(length) = remaining.first_chunk::<4>().copied().map(u32::from_be_bytes) else { return Ok(None) };
        // Admit the declared field before inspecting its bytes, including malformed strings.
        // Otherwise an oversized invalid value could masquerade as a complete typed scan.
        if length as usize > string_limit {
            return Err(StopReason::StringLimit.into());
        }
        let Some(payload) = remaining.get(4..) else { return Ok(None) };
        let Some((value, rest)) = payload.split_at_checked(length as usize) else { return Ok(None) };
        let value = if value.is_empty() {
            value
        } else {
            match value.strip_suffix(&[0]) {
                Some(value) => value,
                None => {
                    valid = false;
                    value
                }
            }
        };
        // Semantic errors cannot conceal the length of a later reachable field.
        valid &= !value.contains(&0) && std::str::from_utf8(value).is_ok();
        *slot = Some(value);
        remaining = rest;
        count += 1;
    }
    Ok((valid && remaining.is_empty() && matches!(count, 3 | 5)).then_some(UserUnit { factor, digits, strings }))
}

#[cfg(test)]
mod tests;
