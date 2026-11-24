//! Precision-preserving, tile-sized source delivery. No profile/transfer conversion
//! or indexed expansion is performed; the existing f32 APIs retain their behavior.
use crate::{Channel, Compression, Error, Precision, Result, TILE, Xcf, channel_props, invalid, tiles};

mod effect_masks;
pub use effect_masks::{EffectMaskError, EffectMaskHeader};

#[cfg(test)]
mod tests;

/// Preserve optional metadata policy separately from malformed source framing.
#[derive(Debug)]
pub(super) enum SampleHeaderError {
    Source(Error),
    Metadata(crate::records::StopReason),
}

impl From<Error> for SampleHeaderError {
    fn from(error: Error) -> Self {
        Self::Source(error)
    }
}

impl From<SampleHeaderError> for Error {
    fn from(error: SampleHeaderError) -> Self {
        match error {
            SampleHeaderError::Source(error) => error,
            SampleHeaderError::Metadata(reason) => invalid(match reason {
                crate::records::StopReason::WorkLimit => "sample header exceeds the metadata work limit",
                _ => "sample header exceeds a metadata limit",
            }),
        }
    }
}

/// Byte order of drawable samples only. Structural words/pointers are always big-endian.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleOrder {
    BigEndian,
    LittleEndian,
}

/// How an additive source visitor handles ambiguous high-depth XCF 4–11 samples.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HistoricalOrder {
    /// Preserve existing shared callers' big-endian interpretation.
    #[default]
    Default,
    /// The caller explicitly selected an order for this source.
    Explicit(SampleOrder),
    /// Refuse ambiguous samples until the caller obtains an explicit choice.
    RequireExplicit,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SampleOptions {
    pub historical_order: HistoricalOrder,
}

/// Source transfer interpretation; sample delivery never applies a transfer curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transfer {
    Linear,
    Nonlinear,
    Perceptual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourcePrecision {
    pub storage: Precision,
    pub transfer: Transfer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SampleOrderRequest {
    pub version: u32,
    pub precision: SourcePrecision,
}

/// Exact stored values. F16 retains its IEEE binary16 bits, including signed zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SourceSample {
    U8(u8),
    U16(u16),
    U32(u32),
    F16(u16),
    F32(f32),
    F64(f64),
}
impl SourceSample {
    /// Normalize integer endpoints, preserving floating HDR and nonfinite values.
    /// Coverage policy and rejection of nonfinite values belong to the consumer.
    pub fn normalized_f64(self) -> f64 {
        match self {
            Self::U8(v) => f64::from(v) / 255.,
            Self::U16(v) => f64::from(v) / 65535.,
            Self::U32(v) => f64::from(v) / f64::from(u32::MAX),
            Self::F16(v) => f64::from(crate::half_to_f32(v)),
            Self::F32(v) => f64::from(v),
            Self::F64(v) => v,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceDrawable {
    pub width: u32,
    pub height: u32,
    /// Stored components, including alpha when present. Indexed color is not expanded.
    pub channels: usize,
    pub indexed: bool,
    pub precision: SourcePrecision,
    pub sample_order: SampleOrder,
    /// Ordinary saved channels, nonlinear-U8 selections and floating selections
    /// with unconfirmed target encodings carry no exact per-drawable transfer
    /// marker in XCF.
    /// When true, `precision.transfer` is the reader's default assumption rather
    /// than an encoding established by the file. Stored samples are unchanged.
    pub transfer_is_assumed: bool,
}

/// Borrowed interleaved source bytes, valid only during this tile's callback.
#[derive(Clone, Copy, Debug)]
pub struct SourceTile<'a> {
    /// Coordinates within the drawable, before its layer offset is applied.
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub drawable: SourceDrawable,
    pub data: &'a [u8],
}
impl SourceTile<'_> {
    /// Iterate exact F32 source words after validating the tile's complete interleaved layout
    /// once. Other storage representations return `None`; no narrowing, transfer conversion,
    /// or nonfinite-value policy is applied. Signed zero and NaN payload bits are preserved.
    #[inline]
    pub fn f32_samples(&self) -> Result<Option<impl ExactSizeIterator<Item = f32> + '_>> {
        let precision = self.drawable.precision.storage;
        if !precision.float || precision.bytes != 4 {
            return Ok(None);
        }
        let samples = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|pixels| pixels.checked_mul(self.drawable.channels))
            .ok_or_else(|| invalid("tile sample count overflow"))?;
        let expected = samples.checked_mul(4).ok_or_else(|| invalid("tile byte count overflow"))?;
        if self.data.len() != expected {
            return Err(invalid("F32 source bytes do not match the tile dimensions and channels"));
        }
        let (words, trailing) = self.data.as_chunks::<4>();
        if !trailing.is_empty() {
            return Err(invalid("incomplete F32 source word"));
        }
        let little = self.drawable.sample_order == SampleOrder::LittleEndian;
        Ok(Some(words.iter().map(move |word| if little { f32::from_le_bytes(*word) } else { f32::from_be_bytes(*word) })))
    }

    /// Read one interleaved stored component without f32 narrowing or transfer conversion.
    pub fn sample(&self, index: usize) -> Result<SourceSample> {
        let precision = self.drawable.precision.storage;
        let start = index.checked_mul(precision.bytes).ok_or_else(|| invalid("sample offset overflow"))?;
        let end = start.checked_add(precision.bytes).ok_or_else(|| invalid("sample offset overflow"))?;
        let bytes = self.data.get(start..end).ok_or_else(|| invalid("sample is outside its tile"))?;
        let little = self.drawable.sample_order == SampleOrder::LittleEndian;
        match (precision.float, precision.bytes) {
            (false, 1) => bytes.first().copied().map(SourceSample::U8).ok_or_else(|| invalid("missing byte sample")),
            (false, 2) | (true, 2) => {
                let raw = *bytes.first_chunk::<2>().ok_or_else(|| invalid("short 16-bit sample"))?;
                let value = if little { u16::from_le_bytes(raw) } else { u16::from_be_bytes(raw) };
                Ok(if precision.float { SourceSample::F16(value) } else { SourceSample::U16(value) })
            }
            (false, 4) | (true, 4) => {
                let raw = *bytes.first_chunk::<4>().ok_or_else(|| invalid("short 32-bit sample"))?;
                let value = if little { u32::from_le_bytes(raw) } else { u32::from_be_bytes(raw) };
                Ok(if precision.float { SourceSample::F32(f32::from_bits(value)) } else { SourceSample::U32(value) })
            }
            (true, 8) => {
                let raw = *bytes.first_chunk::<8>().ok_or_else(|| invalid("short 64-bit sample"))?;
                Ok(SourceSample::F64(if little { f64::from_le_bytes(raw) } else { f64::from_be_bytes(raw) }))
            }
            _ => Err(invalid("unsupported source sample representation")),
        }
    }
}

struct VisitTarget<'r> {
    pointer: usize,
    channels: usize,
    indexed: bool,
    dimensions: Option<(u32, u32)>,
    coverage: bool,
    transfer_is_assumed: bool,
    excluded_source: &'r [std::ops::Range<usize>],
}

pub(super) struct HierarchyHead {
    pub width: u32,
    pub height: u32,
    pub bpp: usize,
    pub(super) level: usize,
    header: std::ops::Range<usize>,
    dummy_headers: [Option<(usize, usize)>; 31],
}

impl Xcf<'_> {
    pub fn source_precision(&self) -> SourcePrecision {
        let storage = self.image.precision;
        let transfer = if storage.linear {
            Transfer::Linear
        } else if self.image.version >= 7 && self.precision_code % 100 == 75 {
            Transfer::Perceptual
        } else {
            Transfer::Nonlinear
        };
        SourcePrecision { storage, transfer }
    }

    /// A question, not a guessed interpretation. Default visitors still match legacy callers.
    pub fn historical_sample_order_request(&self) -> Option<SampleOrderRequest> {
        ((4..=11).contains(&self.image.version) && self.image.precision.bytes > 1)
            .then(|| SampleOrderRequest { version: self.image.version, precision: self.source_precision() })
    }

    fn source_order(&self, options: SampleOptions) -> Result<SampleOrder> {
        if self.historical_sample_order_request().is_none() {
            return Ok(SampleOrder::BigEndian);
        }
        match options.historical_order {
            HistoricalOrder::Default => Ok(SampleOrder::BigEndian),
            HistoricalOrder::Explicit(order) => Ok(order),
            HistoricalOrder::RequireExplicit => Err(invalid("historical XCF samples require an explicit byte order")),
        }
    }

    /// Deliver original source tiles; no full-image sample buffer is allocated.
    /// `budget` charges logical decoded source bytes once before the first callback.
    /// Pointer tables and one tile's scratch are separately bounded by file/tile limits.
    /// Charges are not refunded after errors. Callbacks may observe a prefix before
    /// failure/cancellation, so a consumer publishing atomically must stage its output.
    pub fn visit_layer_tiles(
        &self,
        index: usize,
        options: SampleOptions,
        budget: &mut usize,
        keep_going: &dyn Fn() -> bool,
        visit: impl FnMut(SourceTile<'_>) -> Result<()>,
    ) -> Result<Option<SourceDrawable>> {
        if !keep_going() {
            return Err(Error::Cancelled);
        }
        let layer = self.image.layers.get(index).ok_or_else(|| invalid("no such layer"))?;
        if layer.hierarchy == 0 {
            return Ok(None);
        }
        let (channels, indexed) = layer.source_layout(self.image.base_type)?;
        let (coverage, transfer_is_assumed) = self.floating_gray_transfer(layer, keep_going)?;
        self.visit_hierarchy(
            VisitTarget {
                pointer: layer.hierarchy,
                channels,
                indexed,
                dimensions: Some((layer.width, layer.height)),
                coverage,
                transfer_is_assumed,
                excluded_source: std::slice::from_ref(&layer.source_record),
            },
            options,
            budget,
            keep_going,
            visit,
        )
        .map(Some)
    }

    // Layer and saved-channel identities take precedence over a conflicting
    // mask pointer. Corroborate only a mask target with no conflicting role,
    // without retaining metadata or following its effect or pixel offsets.
    fn floating_gray_transfer(&self, layer: &crate::Layer, keep_going: &dyn Fn() -> bool) -> Result<(bool, bool)> {
        if !layer.floating_selection || layer.layer_type != 3 {
            return Ok((false, false));
        }
        let Some(target) = layer.floating_target.filter(|offset| *offset != 0) else { return Ok((false, true)) };
        if let Some((_, owner)) = self.layer_sources.iter().zip(&self.image.layers).find(|(source, _)| u64::try_from(**source).ok() == Some(target)) {
            return Ok((false, owner.floating_selection)); // Ordinary layer color retains the canvas transfer.
        }
        if self.channels.iter().any(|source| u64::try_from(*source).ok() == Some(target)) {
            let nonlinear_u8 =
                self.image.precision.bytes == 1 && !self.image.precision.float && self.source_precision().transfer == Transfer::Nonlinear;
            return Ok((!nonlinear_u8, true)); // Same unconfirmed default as a saved channel.
        }
        if let Some(owner) = self.image.layers.iter().find(|owner| owner.mask != 0 && u64::try_from(owner.mask).ok() == Some(target)) {
            match self.confirm_mask_record(owner, keep_going) {
                Ok(()) => return Ok((true, false)),
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(_) => return Ok((false, true)),
            }
        }
        Ok((false, true)) // Unknown targets cannot establish the stored transfer.
    }

    fn confirm_mask_record(&self, owner: &crate::Layer, keep_going: &dyn Fn() -> bool) -> Result<()> {
        if !keep_going() {
            return Err(Error::Cancelled);
        }
        let mut cursor = self.c.at(owner.mask);
        if (cursor.u32()?, cursor.u32()?) != (owner.width, owner.height) {
            return Err(invalid("mask dimensions do not match their owner"));
        }
        let name_length = cursor.u32()? as usize;
        if name_length > crate::MAX_STRING {
            return Err(invalid("mask name exceeds the source limit"));
        }
        let name = cursor.take(name_length)?;
        if !name.is_empty() && name.last() != Some(&0) {
            return Err(invalid("mask name is not NUL terminated"));
        }
        for count in 0..=4096 {
            if !keep_going() {
                return Err(Error::Cancelled);
            }
            let kind = cursor.u32()?;
            let length = cursor.u32()? as usize;
            if kind == 0 {
                if length != 0 {
                    return Err(invalid("mask property terminator has a payload"));
                }
                let hierarchy = cursor.ptr()?;
                self.channel_effects_tail(&mut cursor, hierarchy, keep_going, None)?;
                let fixed_size = 12 + if cursor.wide { 8 } else { 4 };
                let end = hierarchy.checked_add(fixed_size).ok_or_else(|| invalid("mask hierarchy header overflow"))?;
                if hierarchy == 0 || end > cursor.b.len() || (hierarchy < cursor.pos && end > owner.mask) {
                    return Err(invalid("mask hierarchy header is outside the file or overlaps its record"));
                }
                return Ok(());
            }
            if count == 4096 || length > crate::MAX_PAYLOAD {
                return Err(invalid("mask properties exceed the source limits"));
            }
            cursor.take(length)?;
        }
        Err(invalid("unterminated mask properties"))
    }

    pub fn visit_layer_mask_tiles(
        &self,
        index: usize,
        options: SampleOptions,
        budget: &mut usize,
        keep_going: &dyn Fn() -> bool,
        visit: impl FnMut(SourceTile<'_>) -> Result<()>,
    ) -> Result<Option<(Channel, SourceDrawable)>> {
        if !keep_going() {
            return Err(Error::Cancelled);
        }
        let layer = self.image.layers.get(index).ok_or_else(|| invalid("no such layer"))?;
        if layer.mask == 0 {
            return Ok(None);
        }
        self.visit_channel_at(layer.mask, Some((layer.width, layer.height)), options, budget, keep_going, visit).map(Some)
    }

    pub fn visit_channel_tiles(
        &self,
        index: usize,
        options: SampleOptions,
        budget: &mut usize,
        keep_going: &dyn Fn() -> bool,
        visit: impl FnMut(SourceTile<'_>) -> Result<()>,
    ) -> Result<(Channel, SourceDrawable)> {
        if !keep_going() {
            return Err(Error::Cancelled);
        }
        let ptr = *self.channels.get(index).ok_or_else(|| invalid("no such channel"))?;
        self.visit_channel_at(ptr, None, options, budget, keep_going, visit)
    }

    fn visit_channel_at(
        &self,
        ptr: usize,
        owner: Option<(u32, u32)>,
        options: SampleOptions,
        budget: &mut usize,
        keep_going: &dyn Fn() -> bool,
        visit: impl FnMut(SourceTile<'_>) -> Result<()>,
    ) -> Result<(Channel, SourceDrawable)> {
        let mut cursor = self.c.at(ptr);
        let dimensions = (cursor.u32()?, cursor.u32()?);
        if owner.is_some_and(|owner| owner != dimensions) {
            return Err(invalid("mask dimensions do not match their owner"));
        }
        // An unanswered interpretation question publishes no samples and must
        // not spend the metadata needed by the caller's explicit retry.
        self.source_order(options)?;
        let name = cursor.string()?;
        self.meta_budget.set(self.meta_budget.get().checked_sub(name.len()).ok_or_else(|| invalid("channel metadata exceeds the budget"))?);
        let name_bytes = name.len();
        let result = (|| {
            // Only the saved-channel list can establish the image selection.
            // A stray marker on an owner's mask does not change its role/extent.
            let selection = {
                let properties = channel_props(&mut cursor)?;
                if owner.is_none() {
                    if properties.iter().any(|property| property.kind == 4 && !property.data.is_empty()) {
                        return Err(invalid("selection marker payload must be empty"));
                    }
                    properties.iter().any(|property| property.kind == 4)
                } else {
                    false
                }
            };
            if selection && dimensions != (self.image.width, self.image.height) {
                return Err(invalid("selection dimensions do not match the canvas"));
            }
            let hierarchy = cursor.ptr()?;
            self.channel_effects_tail(&mut cursor, hierarchy, keep_going, None)?;
            // Fresh selections default to linear coverage. Nonlinear-U8 saved
            // selections may instead retain an ordinary channel's buffer, with
            // no wire marker distinguishing that provenance. Preserve samples
            // and disclose the default as assumed in that case.
            let nonlinear_u8 =
                self.image.precision.bytes == 1 && !self.image.precision.float && self.source_precision().transfer == Transfer::Nonlinear;
            let info = self.visit_hierarchy(
                VisitTarget {
                    pointer: hierarchy,
                    channels: 1,
                    indexed: false,
                    dimensions: Some(dimensions),
                    coverage: owner.is_some() || selection || !nonlinear_u8,
                    transfer_is_assumed: owner.is_none() && (!selection || nonlinear_u8),
                    excluded_source: std::slice::from_ref(&(ptr..cursor.pos)),
                },
                options,
                budget,
                keep_going,
                visit,
            )?;
            Ok((Channel { name, selection }, info))
        })();
        if result.is_err() {
            // Refund only this name: a callback may have retained other channels
            // through the same XCF. Decoded pixel work is never refunded.
            self.meta_budget.set(self.meta_budget.get().saturating_add(name_bytes));
        }
        result
    }

    // Since v26 channels append an effect-pointer list. Treat the entire
    // bounded list, including its terminator, as framing rather than samples.
    pub(super) fn channel_effects_tail(
        &self,
        cursor: &mut crate::Cursor<'_>,
        hierarchy: usize,
        keep_going: &dyn Fn() -> bool,
        mut budget: Option<&mut crate::records::Budget>,
    ) -> std::result::Result<(), SampleHeaderError> {
        if self.image.version < 26 {
            // Older channels end at their hierarchy pointer. A later writer may
            // leave a zero word in the gap, but only a tile pointer can establish
            // that bytes in the gap are samples.
            return Ok(());
        }
        let width = if self.c.wide { 8 } else { 4 };
        let forward_boundary = (hierarchy != 0 && hierarchy >= cursor.pos).then_some(hierarchy);
        for count in 0..=1024 {
            if !keep_going() {
                return Err(Error::Cancelled.into());
            }
            if forward_boundary.is_some_and(|end| cursor.pos.checked_add(width).is_none_or(|next| next > end)) {
                return Err(invalid("channel effects overlap their hierarchy").into());
            }
            if let Some(budget) = budget.as_deref_mut() {
                budget.work(width).map_err(SampleHeaderError::Metadata)?;
            }
            if cursor.take(width)?.iter().all(|byte| *byte == 0) {
                return Ok(());
            }
            if count == 1024 {
                return Err(invalid("too many channel effects").into());
            }
        }
        Err(invalid("unterminated channel effects").into())
    }

    pub(super) fn hierarchy_head(
        &self,
        pointer: usize,
        channels: usize,
        mut metadata: Option<&mut crate::records::Budget>,
    ) -> std::result::Result<HierarchyHead, SampleHeaderError> {
        let mut admit = |length| {
            if let Some(metadata) = metadata.as_deref_mut() {
                metadata.work(length).map_err(SampleHeaderError::Metadata)?;
            }
            Ok::<_, SampleHeaderError>(())
        };
        let mut cursor = self.c.at(pointer);
        let pointer_bytes = if cursor.wide { 8 } else { 4 };
        admit(12 + pointer_bytes)?;
        let (width, height, bpp) = (cursor.u32()?, cursor.u32()?, cursor.u32()? as usize);
        if width == 0 || height == 0 || width > crate::MAX_SIDE || height > crate::MAX_SIDE {
            return Err(invalid(format!("drawable size {width}x{height} is out of range")).into());
        }
        let expected = channels.checked_mul(self.image.precision.bytes).ok_or_else(|| invalid("bpp overflow"))?;
        if !(1..=4).contains(&channels) || bpp != expected {
            return Err(invalid(format!("{bpp} bytes per pixel where {expected} were expected")).into());
        }
        let level = cursor.ptr()?;
        if level == pointer {
            return Err(invalid("a hierarchy cannot be its own level").into());
        }
        // The fixed fields are followed by unused mip-level pointers and a
        // mandatory zero. Protect those words as framing too. A u32-sized
        // dimension can halve at most 32 times, so never scan an input-sized list.
        // Unused offsets need not fit the host or address the file. Only bounded,
        // corroborated in-file dummy headers are inspected below.
        let mut dummy_offsets = [0u64; 31];
        for index in 0..u32::BITS as usize {
            admit(pointer_bytes)?;
            let word = cursor.take(pointer_bytes)?;
            if word.iter().all(|byte| *byte == 0) {
                let header = pointer..cursor.pos;
                let mut dummy_headers = [None; 31];
                let (mut mip_width, mut mip_height) = (width, height);
                for (slot, offset) in dummy_headers.iter_mut().zip(dummy_offsets.iter().take(index)) {
                    mip_width /= 2;
                    mip_height /= 2;
                    let Ok(at) = usize::try_from(*offset) else { continue };
                    // Dummy levels end in a fixed zero WORD, not a pointer.
                    let Some(end) = at.checked_add(12) else { continue };
                    if at < header.end && end > header.start {
                        continue;
                    }
                    let Some(bytes) = self.c.b.get(at..end) else { continue };
                    admit(bytes.len())?;
                    // Only corroborated empty mip headers define a boundary;
                    // arbitrary/unusable unused offsets remain ignored.
                    if bytes.get(..4) == Some(mip_width.to_be_bytes().as_slice())
                        && bytes.get(4..8) == Some(mip_height.to_be_bytes().as_slice())
                        && bytes.get(8..).is_some_and(|word| word.iter().all(|byte| *byte == 0))
                    {
                        *slot = Some((at, end));
                    }
                }
                return Ok(HierarchyHead { width, height, bpp, level, header, dummy_headers });
            }
            if let Some(slot) = dummy_offsets.get_mut(index) {
                *slot = if cursor.wide {
                    u64::from_be_bytes(word.try_into().map_err(|_| invalid("invalid hierarchy pointer width"))?)
                } else {
                    u64::from(u32::from_be_bytes(word.try_into().map_err(|_| invalid("invalid hierarchy pointer width"))?))
                };
            }
        }
        Err(invalid("hierarchy level list exceeds 32 levels or has no zero terminator").into())
    }

    /// Validate only the fixed first-level header, including the physical first
    /// tile-pointer field. Its value may be zero; no tile table or pixels are read.
    pub(super) fn validate_first_level(&self, head: &HierarchyHead) -> Result<()> {
        let pointer_bytes = if self.c.wide { 8 } else { 4 };
        let end = head.level.checked_add(8 + pointer_bytes).ok_or_else(|| invalid("level header overflow"))?;
        if head.level < head.header.end && end > head.header.start {
            return Err(invalid("a level header overlaps its hierarchy header"));
        }
        let mut level = self.c.at(head.level);
        if (level.u32()?, level.u32()?) != (head.width, head.height) {
            return Err(invalid("the level does not match its hierarchy"));
        }
        level.take(pointer_bytes)?;
        Ok(())
    }

    // Header starts are hard boundaries, even when tile records precede them.
    // Both pixel APIs must use the same slice to avoid decoding framing as samples.
    pub(super) fn tile_input(
        &self,
        head: &HierarchyHead,
        start: usize,
        next: Option<usize>,
        raw_bytes: usize,
        excluded: &[std::ops::Range<usize>],
    ) -> Result<&[u8]> {
        if head.header.contains(&start) {
            return Err(invalid("a tile pointer is inside its hierarchy header"));
        }
        let uncompressed = self.image.compression == Compression::None;
        let mut end = if uncompressed { self.c.b.len() } else { next.unwrap_or(self.c.b.len()) };
        if start < head.header.start {
            end = end.min(head.header.start);
        }
        if start < head.level {
            end = end.min(head.level);
        }
        for range in excluded {
            if range.contains(&start) {
                return Err(invalid("a tile pointer is inside its source record"));
            }
            if start < range.start {
                end = end.min(range.start);
            }
        }
        let raw_end = if uncompressed { Some(start.checked_add(raw_bytes).ok_or_else(|| invalid("raw tile extent overflow"))?) } else { None };
        for &(begin, stop) in head.dummy_headers.iter().flatten() {
            // A raw tile's byte count, together with a following primary
            // pointer or exact record/EOF boundary, can disprove a dummy
            // header inside its samples. A dummy ending exactly at EOF or
            // at a record boundary may instead be all that follows a short tile.
            if raw_end.is_some_and(|raw_end| {
                start < begin
                    && raw_end <= end
                    && ((next == Some(raw_end) && begin < raw_end) || (next.is_none_or(|next| next == start) && raw_end == end && stop < raw_end))
            }) {
                continue;
            }
            if (begin..stop).contains(&start) {
                return Err(invalid("a tile pointer is inside a dummy-level header"));
            }
            if start < begin {
                end = end.min(begin);
            }
        }
        if let Some(raw_end) = raw_end {
            end = end.min(raw_end);
        }
        self.c.b.get(start..end).ok_or_else(|| invalid("a tile pointer is outside the file"))
    }

    pub(super) fn hierarchy_tiles(
        &self,
        head: &mut HierarchyHead,
        allow_empty: bool,
        excluded: &[std::ops::Range<usize>],
        keep_going: &dyn Fn() -> bool,
    ) -> Result<Vec<usize>> {
        if !keep_going() {
            return Err(Error::Cancelled);
        }
        self.validate_first_level(head)?;
        let overlaps = |start: usize, end: usize| excluded.iter().any(|range| start < range.end && end > range.start);
        if overlaps(head.header.start, head.header.end) {
            return Err(invalid("a hierarchy overlaps its source record"));
        }
        let fixed_end = head.level.checked_add(if self.c.wide { 16 } else { 12 }).ok_or_else(|| invalid("level header overflow"))?;
        if overlaps(head.level, fixed_end) {
            return Err(invalid("a level header overlaps its source record"));
        }
        let mut level = self.c.at(head.level);
        level.take(8)?;
        // The enhanced source contract admits an entirely zero level; retain the
        // legacy f32 entry point's existing rejection of a zero first pointer.
        if allow_empty && level.at(level.pos).ptr()? == 0 {
            return Ok(Vec::new());
        }
        let count =
            (head.width.div_ceil(TILE) as usize).checked_mul(head.height.div_ceil(TILE) as usize).ok_or_else(|| invalid("tile count overflow"))?;
        let offsets = crate::tile_offsets(&mut level, head.level, count, self.image.compression, keep_going)?;
        if head.level < head.header.end && level.pos > head.header.start {
            return Err(invalid("a level table overlaps its hierarchy header"));
        }
        if overlaps(head.level, level.pos) {
            return Err(invalid("a level table overlaps its source record"));
        }
        // An unused mip pointer cannot claim bytes in the drawable record or
        // the primary table, even when adjacent pixels match its fields.
        for candidate in &mut head.dummy_headers {
            if candidate.is_some_and(|(start, end)| (start < level.pos && end > head.level) || overlaps(start, end)) {
                *candidate = None;
            }
        }
        Ok(offsets)
    }

    fn visit_hierarchy(
        &self,
        target: VisitTarget<'_>,
        options: SampleOptions,
        budget: &mut usize,
        keep_going: &dyn Fn() -> bool,
        mut visit: impl FnMut(SourceTile<'_>) -> Result<()>,
    ) -> Result<SourceDrawable> {
        if !keep_going() {
            return Err(Error::Cancelled);
        }
        let VisitTarget { pointer, channels, indexed, dimensions, coverage, transfer_is_assumed, excluded_source } = target;
        let order = self.source_order(options)?;
        let mut head = self.hierarchy_head(pointer, channels, None)?;
        if dimensions.is_some_and(|size| size != (head.width, head.height)) {
            return Err(invalid("drawable dimensions do not match their hierarchy"));
        }
        if indexed && self.image.precision.bytes != 1 {
            return Err(invalid("indexed samples must be 8-bit"));
        }
        let bytes = (head.width as usize)
            .checked_mul(head.height as usize)
            .and_then(|n| n.checked_mul(head.bpp))
            .ok_or_else(|| invalid("drawable overflow"))?;
        if bytes > crate::MAX_PIXEL_BYTES {
            return Err(invalid("drawable exceeds the source pixel limit"));
        }
        *budget = budget.checked_sub(bytes).ok_or_else(|| invalid("the layers exceed the source pixel limit"))?;
        let offsets = self.hierarchy_tiles(&mut head, true, excluded_source, keep_going)?;
        let mut precision = self.source_precision();
        if coverage {
            // Layer masks contain coverage, not color in the image's transfer.
            precision.transfer = Transfer::Linear;
            precision.storage.linear = true;
        }
        let drawable =
            SourceDrawable { width: head.width, height: head.height, channels, indexed, precision, sample_order: order, transfer_is_assumed };
        let across = head.width.div_ceil(TILE) as usize;
        let count = across.checked_mul(head.height.div_ceil(TILE) as usize).ok_or_else(|| invalid("tile count overflow"))?;
        for index in 0..count {
            if !keep_going() {
                return Err(Error::Cancelled);
            }
            let x = (index % across) as u32 * TILE;
            let y = (index / across) as u32 * TILE;
            let width = (head.width - x).min(TILE);
            let height = (head.height - y).min(TILE);
            let raw = if let Some(&start) = offsets.get(index) {
                let input =
                    self.tile_input(&head, start, offsets.get(index + 1).copied(), width as usize * height as usize * head.bpp, excluded_source)?;
                tiles::decode(input, self.image.compression, head.bpp, width as usize * height as usize)?
            } else {
                let length = (width as usize)
                    .checked_mul(height as usize)
                    .and_then(|n| n.checked_mul(head.bpp))
                    .ok_or_else(|| invalid("tile size overflow"))?;
                let mut raw = Vec::new();
                raw.try_reserve_exact(length).map_err(|_| invalid("not enough memory for a source tile"))?;
                raw.resize(length, 0);
                raw
            };
            visit(SourceTile { x, y, width, height, drawable, data: &raw })?;
        }
        if !keep_going() {
            return Err(Error::Cancelled);
        }
        Ok(drawable)
    }
}
