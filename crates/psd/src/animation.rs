//! Legacy Photoshop frame-animation syntax, independent of playback or document semantics.
//!
//! Observed in a CS5-authored PSD: a plug-in image resource (id in 4000..=4999) starts with
//! `maniIRFR`, a big-endian u32 byte length, then `8BIM` / key / u32 length / data records.
//! `AnDs` holds a version-16 ActionDescriptor; other records (including `Roll`) stay opaque.
//! Layer `shmd` items `mlst` and `tmln`, and image resource 1075, also hold versioned descriptors.
//! These undocumented layouts were established by inspecting file structure, not proprietary code.
//!
//! Raw records remain the source of truth: typed accessors do not rewrite them, discard unknown
//! fields, interpret sparse layer-state inheritance, or assume a frame id is its sequence index.
//! This module does **not** import an animation into a document or evaluate timeline tracks.

use crate::descriptor::{Descriptor, Value, VersionedDescriptor};
use crate::error::{PsdError, Result};
use crate::io::{Reader, WriteExt};

/// A record in a `mani` resource. Data includes any padding counted in its stored length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimationRecord<'a> {
    /// Record key (`AnDs`, `Roll`, or an unknown key).
    pub key: [u8; 4],
    /// Borrowed, unmodified payload.
    pub data: &'a [u8],
}

impl AnimationRecord<'_> {
    /// Typed view of `AnDs`; `None` for other records, including `Roll`.
    pub fn descriptor(&self) -> Option<Result<VersionedDescriptor>> {
        (self.key == *b"AnDs").then(|| parse_descriptor(self.data))
    }
}

/// The records of a legacy frame-animation plug-in resource, in stored order.
///
/// Unknown keys and duplicate records are preserved, not silently collapsed. The observed
/// envelope has no alignment bytes outside record lengths. Unsupported envelopes return an error;
/// callers should keep their original image resource unchanged in that case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimationResource<'a> {
    /// Ordered records, borrowing their payloads from the input.
    pub records: Vec<AnimationRecord<'a>>,
}

impl<'a> AnimationResource<'a> {
    /// Parses a complete `maniIRFR` payload, checking the outer and every inner length.
    /// Descriptor decoding is lazy, so unknown/future `AnDs` versions can still round-trip raw.
    pub fn from_bytes(data: &'a [u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        signature(&mut r, *b"mani", "mani")?;
        signature(&mut r, *b"IRFR", "IRFR animation envelope")?;
        let len = r.u32()?;
        let mut body = r.sub(u64::from(len))?;
        if !r.is_empty() {
            return Err(PsdError::invalid("trailing bytes after animation envelope"));
        }
        let mut records = Vec::new();
        while !body.is_empty() {
            signature(&mut body, *b"8BIM", "8BIM animation record")?;
            let key = body.array()?;
            let len = body.u32()?;
            let data = body.bytes_u64(u64::from(len))?;
            records.push(AnimationRecord { key, data });
        }
        Ok(Self { records })
    }

    /// Writes the raw records, preserving their payloads exactly, including unknown fields,
    /// unknown descriptor versions and noncanonical descriptor encodings.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        // Check all lengths before constructing the output; no unchecked 32-bit length casts.
        let mut len = 0u32;
        for record in &self.records {
            let n = u32::try_from(record.data.len()).map_err(|_| PsdError::LimitExceeded("animation record exceeds u32 length"))?;
            len = len.checked_add(12).and_then(|v| v.checked_add(n)).ok_or(PsdError::LimitExceeded("animation envelope exceeds u32 length"))?;
        }
        let mut out = Vec::new();
        out.put(b"maniIRFR");
        out.put_u32(len);
        for record in &self.records {
            out.put(b"8BIM");
            out.put(&record.key);
            out.put_len(record.data.len() as u64, false)?;
            out.put(record.data);
        }
        Ok(out)
    }
}

fn signature(r: &mut Reader<'_>, expected: [u8; 4], name: &'static str) -> Result<()> {
    let found = r.array()?;
    if found != expected {
        return Err(PsdError::InvalidSignature { expected: name, found });
    }
    Ok(())
}

/// Parses a version-16 animation descriptor, allowing up to three zero alignment bytes.
///
/// Raw payloads should be retained separately for byte-exact writing. This accessor is strict
/// about non-padding trailing bytes and does not guess how to read a future descriptor version.
/// It is shared by `AnDs`, layer `mlst` / `tmln`, and document timeline resource 1075.
pub fn parse_descriptor(data: &[u8]) -> Result<VersionedDescriptor> {
    let (value, consumed) = VersionedDescriptor::parse_prefix(data)?;
    let tail = data.get(consumed..).ok_or_else(|| PsdError::invalid("animation descriptor length"))?;
    if tail.len() > 3 || tail.iter().any(|&b| b != 0) {
        return Err(PsdError::invalid("trailing bytes after animation descriptor"));
    }
    Ok(value)
}

/// A frame-definition view. Its id is an opaque signed `long`, not a frame index.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameInfo<'a> {
    /// `FrID`, preserved without reinterpreting its bit pattern.
    pub id: i32,
    /// `FrDl` in centiseconds, or absent. No playback default or browser clamping is applied.
    pub delay_centiseconds: Option<u32>,
    /// All fields, including unknown ones, in their original order.
    pub descriptor: &'a Descriptor,
}

/// Reads the `FrIn` list from an `AnDs` descriptor. A missing list is not an empty animation.
/// Wrong types, missing frame ids and negative delays return errors. Duplicate ids are retained
/// for the document importer to diagnose rather than being silently overwritten here.
pub fn frame_info(descriptor: &Descriptor) -> Result<Vec<FrameInfo<'_>>> {
    let Some(Value::List(list)) = descriptor.get("FrIn") else {
        return Err(PsdError::invalid("animation FrIn must be a list"));
    };
    list.iter()
        .map(|v| {
            let Value::Descriptor(d) = v else { return Err(PsdError::invalid("animation frame must be a descriptor")) };
            let Some(Value::Integer(id)) = d.get("FrID") else { return Err(PsdError::invalid("animation frame FrID must be a long")) };
            let delay_centiseconds = match d.get("FrDl") {
                None => None,
                Some(Value::Integer(n)) => Some(u32::try_from(*n).map_err(|_| PsdError::invalid("negative animation frame delay"))?),
                _ => return Err(PsdError::invalid("animation frame FrDl must be a long")),
            };
            Ok(FrameInfo { id: *id, delay_centiseconds, descriptor: d })
        })
        .collect()
}

/// A stored frame set: explicit playback order is separate from the `FrIn` definitions.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameSet<'a> {
    /// `FsID`, an opaque identifier.
    pub id: i32,
    /// `AFrm`, when present. Kept uninterpreted for the importer to validate.
    pub active_frame: Option<i32>,
    /// `FsFr` frame ids in stored order (including any duplicate or unresolved ids).
    pub frame_ids: Vec<i32>,
    /// Complete original descriptor, including fields not yet understood.
    pub descriptor: &'a Descriptor,
}

/// One sparse layer-state entry from `mlst` / `LaSt`.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerFrameState<'a> {
    /// `FrLs`: frame ids to which this entry refers. They are not frame indices.
    pub frame_ids: Vec<i32>,
    /// Complete sparse state, including visibility, blend options, offsets, masks, effects,
    /// text settings and unknown fields. Omitted properties are deliberately not defaulted.
    pub descriptor: &'a Descriptor,
}

/// A layer's sparse animation states, without attempting to resolve their inheritance.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerFrameStates<'a> {
    /// `LaID`: PSD layer identifier, not the layer's record index.
    pub layer_id: i32,
    /// `LaSt` entries in stored order.
    pub states: Vec<LayerFrameState<'a>>,
}

/// Reads the identifiers and sparse entries of an `mlst` descriptor. This is a syntax view:
/// missing properties stay missing, and duplicate/unresolved references remain available for
/// the importer to validate. The original descriptor remains available to the caller.
pub fn layer_frame_states(descriptor: &Descriptor) -> Result<LayerFrameStates<'_>> {
    let Some(Value::Integer(layer_id)) = descriptor.get("LaID") else {
        return Err(PsdError::invalid("animation layer LaID must be a long"));
    };
    let Some(Value::List(list)) = descriptor.get("LaSt") else {
        return Err(PsdError::invalid("animation layer LaSt must be a list"));
    };
    let states = list
        .iter()
        .map(|v| {
            let Value::Descriptor(d) = v else { return Err(PsdError::invalid("animation layer state must be a descriptor")) };
            let Some(Value::List(ids)) = d.get("FrLs") else { return Err(PsdError::invalid("animation layer FrLs must be a list")) };
            let frame_ids = ids
                .iter()
                .map(|v| match v {
                    Value::Integer(n) => Ok(*n),
                    _ => Err(PsdError::invalid("animation layer frame reference must be a long")),
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(LayerFrameState { frame_ids, descriptor: d })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(LayerFrameStates { layer_id: *layer_id, states })
}

/// Reads `FSts` without inferring order from frame ids or interpreting loop-related fields.
pub fn frame_sets(descriptor: &Descriptor) -> Result<Vec<FrameSet<'_>>> {
    let Some(Value::List(list)) = descriptor.get("FSts") else {
        return Err(PsdError::invalid("animation FSts must be a list"));
    };
    list.iter()
        .map(|v| {
            let Value::Descriptor(d) = v else { return Err(PsdError::invalid("animation frame set must be a descriptor")) };
            let Some(Value::Integer(id)) = d.get("FsID") else { return Err(PsdError::invalid("animation frame set FsID must be a long")) };
            let active_frame = match d.get("AFrm") {
                None => None,
                Some(Value::Integer(n)) => Some(*n),
                _ => return Err(PsdError::invalid("animation frame set AFrm must be a long")),
            };
            let Some(Value::List(ids)) = d.get("FsFr") else { return Err(PsdError::invalid("animation frame set FsFr must be a list")) };
            let frame_ids = ids
                .iter()
                .map(|v| match v {
                    Value::Integer(n) => Ok(*n),
                    _ => Err(PsdError::invalid("animation frame reference must be a long")),
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(FrameSet { id: *id, active_frame, frame_ids, descriptor: d })
        })
        .collect()
}
