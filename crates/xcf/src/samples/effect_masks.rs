//! Source-bound auxiliary masks: framing and one-component source samples only.
use super::{SampleHeaderError, SampleOptions, SourceDrawable, SourceTile, VisitTarget};
use crate::effects::StoredEffect;
use crate::records;
use crate::{Channel, Error, Result, Xcf, invalid};
use std::ops::Range;

#[cfg(test)]
mod tests;

/// Optional metadata refusal stays typed; it never claims that owner pixels are corrupt.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum EffectMaskError {
    #[error(transparent)]
    Source(#[from] crate::Error),
    #[error("auxiliary mask metadata scan stopped: {0:?}")]
    Metadata(records::StopReason),
}

impl From<SampleHeaderError> for EffectMaskError {
    fn from(error: SampleHeaderError) -> Self {
        match error {
            SampleHeaderError::Source(error) => Self::Source(error),
            SampleHeaderError::Metadata(reason) => Self::Metadata(reason),
        }
    }
}

/// Validated channel/hierarchy header, bound to the exact backing XCF.
/// Its private identity prevents callers from redirecting a subsequent tile read.
pub struct EffectMaskHeader<'a> {
    source: &'a [u8],
    version: u32,
    channel_header: Range<usize>,
    // The authenticated whole effect envelope, including properties and mask pointer.
    effect_header: Range<usize>,
    dimensions: (u32, u32),
    channel: Channel,
    properties: Vec<records::Property<'a>>,
    hierarchy: usize,
}

impl std::fmt::Debug for EffectMaskHeader<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectMaskHeader")
            .field("source_bytes", &self.source.len())
            .field("offset", &self.source_offset())
            .field("dimensions", &self.dimensions)
            .finish()
    }
}

impl<'a> EffectMaskHeader<'a> {
    pub fn source_offset(&self) -> u64 {
        self.channel_header.start as u64
    }
    pub fn width(&self) -> u32 {
        self.dimensions.0
    }
    pub fn height(&self) -> u32 {
        self.dimensions.1
    }
    pub fn channel(&self) -> &Channel {
        &self.channel
    }
    /// Original ordered source properties from the same bounded header scan.
    /// Payloads remain borrowed and uninterpreted; no additional scan is performed.
    /// Use [`records::metadata::channel_from_properties`] for typed channel evidence.
    pub fn properties(&self) -> &[records::Property<'a>] {
        &self.properties
    }
}

impl<'a> Xcf<'a> {
    /// Validate an auxiliary effect-mask header without decoding tiles or spending
    /// the caller's pixel allowance. Zero means no mask. The effect must be the
    /// unmodified envelope read from this exact backing source/version.
    /// Reuse `metadata` across owners/repeated headers: source work is never refunded.
    /// Channel names also use the XCF reader's existing copied-metadata allowance.
    pub fn effect_mask_header(
        &self,
        effect: &StoredEffect<'_>,
        metadata: &mut records::Budget,
        keep_going: &dyn Fn() -> bool,
    ) -> std::result::Result<Option<EffectMaskHeader<'a>>, EffectMaskError> {
        if !keep_going() {
            return Err(Error::Cancelled.into());
        }
        let Some((offset, effect_header)) = effect.source_mask(self.c.b, self.image.version)? else { return Ok(None) };
        let pointer = usize::try_from(offset).map_err(|_| invalid("effect mask offset does not fit in memory"))?;
        // This shared traversal admits the name/property work before decoding or
        // copying it; never rescan an unmetered property list after it stops.
        metadata.retain_on_success(|metadata| {
            let properties = records::channel_properties(self.c.b, pointer, records::Limits::default(), metadata, keep_going)?;
            if let Some(stopped) = properties.stopped {
                return Err(EffectMaskError::Metadata(stopped.reason));
            }
            let mut cursor = self.c.at(pointer);
            let dimensions = (cursor.u32()?, cursor.u32()?);
            let mut channel_end = self.c.at(properties.end_offset);
            let pointer_bytes = if channel_end.wide { 8 } else { 4 };
            metadata.work(pointer_bytes).map_err(EffectMaskError::Metadata)?;
            let hierarchy = channel_end.ptr()?;
            // V26 channel records also own their effect list, including its
            // terminator. Those bytes must never become mask coverage samples.
            self.channel_effects_tail(&mut channel_end, hierarchy, keep_going, Some(metadata))?;
            let channel_header = pointer..channel_end.pos;
            if channel_header.start < effect_header.end && channel_header.end > effect_header.start {
                return Err(invalid("effect mask channel overlaps its effect envelope").into());
            }
            let head = self.hierarchy_head(hierarchy, 1, Some(metadata))?;
            if dimensions != (head.width, head.height) {
                return Err(invalid("effect mask dimensions do not match their hierarchy").into());
            }
            let bytes = (head.width as usize)
                .checked_mul(head.height as usize)
                .and_then(|pixels| pixels.checked_mul(head.bpp))
                .ok_or_else(|| invalid("effect mask byte count overflow"))?;
            if bytes > crate::MAX_PIXEL_BYTES {
                return Err(invalid("effect mask exceeds the source pixel limit").into());
            }
            let fixed_level_size = 8 + pointer_bytes;
            metadata.work(fixed_level_size).map_err(EffectMaskError::Metadata)?;
            self.validate_first_level(&head)?;
            let fixed_end = head.level.checked_add(fixed_level_size).ok_or_else(|| invalid("level header overflow"))?;
            // Header validation cannot publish metadata for a hierarchy/level
            // borrowed from either source record and defer failure until pixels.
            if [&channel_header, &effect_header].into_iter().any(|range| head.header.start < range.end && head.header.end > range.start) {
                return Err(invalid("a hierarchy overlaps its source record").into());
            }
            if [&channel_header, &effect_header].into_iter().any(|range| head.level < range.end && fixed_end > range.start) {
                return Err(invalid("a level header overlaps its source record").into());
            }
            if !keep_going() {
                return Err(Error::Cancelled.into());
            }
            // The property scan already bounded the counted name. Require the
            // terminator before copying it, while accepting the wire empty string.
            let name_length = cursor.u32()? as usize;
            let name_bytes = match cursor.take(name_length)?.split_last() {
                None => &[][..],
                Some((&0, name)) => name,
                Some(_) => return Err(invalid("effect mask name has no terminating NUL").into()),
            };
            if name_bytes.contains(&0) {
                return Err(invalid("effect mask name contains an interior NUL").into());
            }
            let name = String::from_utf8_lossy(name_bytes).into_owned();
            self.meta_budget.set(self.meta_budget.get().checked_sub(name.len()).ok_or_else(|| invalid("channel metadata exceeds the budget"))?);
            // The effect pointer establishes the role. Preserve any selection
            // marker as source evidence without reporting this mask as selection.
            let channel = Channel { name, selection: false };
            Ok(Some(EffectMaskHeader {
                source: self.c.b,
                version: self.image.version,
                channel_header,
                effect_header,
                dimensions,
                channel,
                properties: properties.records,
                hierarchy,
            }))
        })
    }

    /// Deliver exact one-component source samples with the same precision/order
    /// and tile scratch rules as layer masks/channels. Logical decoded bytes are
    /// charged once before the first callback and never refunded after failure.
    /// A callback may see a prefix: callers publishing atomically must stage it.
    pub fn visit_effect_mask_tiles(
        &self,
        header: &EffectMaskHeader<'_>,
        options: SampleOptions,
        budget: &mut usize,
        keep_going: &dyn Fn() -> bool,
        visit: impl FnMut(SourceTile<'_>) -> Result<()>,
    ) -> Result<SourceDrawable> {
        if !keep_going() {
            return Err(Error::Cancelled);
        }
        if !std::ptr::eq(header.source, self.c.b) || header.version != self.image.version {
            return Err(invalid("effect mask header belongs to a different source"));
        }
        self.visit_hierarchy(
            VisitTarget {
                pointer: header.hierarchy,
                channels: 1,
                indexed: false,
                dimensions: Some(header.dimensions),
                coverage: true,
                transfer_is_assumed: false,
                excluded_source: &[header.channel_header.clone(), header.effect_header.clone()],
            },
            options,
            budget,
            keep_going,
            visit,
        )
    }
}
