//! Source-bound auxiliary masks: framing and one-component source samples only.
use super::{SampleOptions, SourceDrawable, SourceTile, VisitTarget};
use crate::effects::StoredEffect;
use crate::records;
use crate::{Channel, Error, Result, Xcf, invalid};

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

/// Validated channel/hierarchy header, bound to the exact backing XCF.
/// Its private identity prevents callers from redirecting a subsequent tile read.
pub struct EffectMaskHeader<'a> {
    source: &'a [u8],
    version: u32,
    offset: u64,
    dimensions: (u32, u32),
    channel: Channel,
    properties: Vec<records::Property<'a>>,
    hierarchy: usize,
}

impl std::fmt::Debug for EffectMaskHeader<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectMaskHeader")
            .field("source_bytes", &self.source.len())
            .field("offset", &self.offset)
            .field("dimensions", &self.dimensions)
            .finish()
    }
}

impl<'a> EffectMaskHeader<'a> {
    pub fn source_offset(&self) -> u64 {
        self.offset
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
        let Some(offset) = effect.source_mask(self.c.b, self.image.version)? else { return Ok(None) };
        let pointer = usize::try_from(offset).map_err(|_| invalid("effect mask offset does not fit in memory"))?;
        // This shared traversal admits the name/property work before decoding or
        // copying it; never rescan an unmetered property list after it stops.
        let properties = records::channel_properties(self.c.b, pointer, records::Limits::default(), metadata, keep_going)?;
        if let Some(stopped) = properties.stopped {
            return Err(EffectMaskError::Metadata(stopped.reason));
        }
        let mut cursor = self.c.at(pointer);
        let dimensions = (cursor.u32()?, cursor.u32()?);
        let name = cursor.string()?;
        self.meta_budget.set(self.meta_budget.get().checked_sub(name.len()).ok_or_else(|| invalid("channel metadata exceeds the budget"))?);
        let channel = Channel { name, selection: properties.records.iter().any(|p| p.kind == 4) };
        let hierarchy = self.c.at(properties.end_offset).ptr()?;
        let head = self.hierarchy_head(hierarchy, 1)?;
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
        if !keep_going() {
            return Err(Error::Cancelled.into());
        }
        Ok(Some(EffectMaskHeader {
            source: self.c.b,
            version: self.image.version,
            offset,
            dimensions,
            channel,
            properties: properties.records,
            hierarchy,
        }))
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
                excluded_source: None,
            },
            options,
            budget,
            keep_going,
            visit,
        )
    }
}
