//! Precision-preserving, tile-sized source delivery. No profile/transfer conversion
//! or indexed expansion is performed; the existing f32 APIs retain their behavior.
use crate::{BaseType, Channel, Error, Precision, Result, TILE, Xcf, invalid, props, tiles};

#[cfg(test)]
mod tests;

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

struct VisitTarget {
    pointer: usize,
    channels: usize,
    indexed: bool,
    dimensions: Option<(u32, u32)>,
}

pub(super) struct HierarchyHead {
    pub width: u32,
    pub height: u32,
    pub bpp: usize,
    pub(super) level: usize,
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
        let channels = match (self.image.base_type, layer.layer_type) {
            (BaseType::Rgb, 0) => 3,
            (BaseType::Rgb, 1) => 4,
            (BaseType::Gray, 2) | (BaseType::Indexed, 4) => 1,
            (BaseType::Gray, 3) | (BaseType::Indexed, 5) => 2,
            (BaseType::Indexed, 3) if layer.floating_selection => 2,
            _ => return Err(invalid("layer type does not match the image color model")),
        };
        self.visit_hierarchy(
            VisitTarget {
                pointer: layer.hierarchy,
                channels,
                indexed: matches!(layer.layer_type, 4 | 5),
                dimensions: Some((layer.width, layer.height)),
            },
            options,
            budget,
            keep_going,
            visit,
        )
        .map(Some)
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
        let name = cursor.string()?;
        self.meta_budget.set(self.meta_budget.get().checked_sub(name.len()).ok_or_else(|| invalid("channel metadata exceeds the budget"))?);
        let selection = props(&mut cursor)?.iter().any(|p| p.kind == 4);
        let hierarchy = cursor.ptr()?;
        let info = self.visit_hierarchy(
            VisitTarget { pointer: hierarchy, channels: 1, indexed: false, dimensions: Some(dimensions) },
            options,
            budget,
            keep_going,
            visit,
        )?;
        Ok((Channel { name, selection }, info))
    }

    pub(super) fn hierarchy_head(&self, pointer: usize, channels: usize) -> Result<HierarchyHead> {
        let mut cursor = self.c.at(pointer);
        let (width, height, bpp) = (cursor.u32()?, cursor.u32()?, cursor.u32()? as usize);
        if width == 0 || height == 0 || width > crate::MAX_SIDE || height > crate::MAX_SIDE {
            return Err(invalid(format!("drawable size {width}x{height} is out of range")));
        }
        let expected = channels.checked_mul(self.image.precision.bytes).ok_or_else(|| invalid("bpp overflow"))?;
        if !(1..=4).contains(&channels) || bpp != expected {
            return Err(invalid(format!("{bpp} bytes per pixel where {expected} were expected")));
        }
        let level = cursor.ptr()?;
        if level == pointer {
            return Err(invalid("a hierarchy cannot be its own level"));
        }
        Ok(HierarchyHead { width, height, bpp, level })
    }

    pub(super) fn hierarchy_tiles(&self, head: &HierarchyHead, allow_empty: bool, keep_going: &dyn Fn() -> bool) -> Result<Vec<usize>> {
        let mut level = self.c.at(head.level);
        if (level.u32()?, level.u32()?) != (head.width, head.height) {
            return Err(invalid("the level does not match its hierarchy"));
        }
        // The enhanced source contract admits an entirely zero level; retain the
        // legacy f32 entry point's existing rejection of a zero first pointer.
        if allow_empty && level.at(level.pos).ptr()? == 0 {
            return Ok(Vec::new());
        }
        let count =
            (head.width.div_ceil(TILE) as usize).checked_mul(head.height.div_ceil(TILE) as usize).ok_or_else(|| invalid("tile count overflow"))?;
        crate::tile_offsets(&mut level, head.level, count, keep_going)
    }

    fn visit_hierarchy(
        &self,
        target: VisitTarget,
        options: SampleOptions,
        budget: &mut usize,
        keep_going: &dyn Fn() -> bool,
        mut visit: impl FnMut(SourceTile<'_>) -> Result<()>,
    ) -> Result<SourceDrawable> {
        if !keep_going() {
            return Err(Error::Cancelled);
        }
        let VisitTarget { pointer, channels, indexed, dimensions } = target;
        let order = self.source_order(options)?;
        let head = self.hierarchy_head(pointer, channels)?;
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
        let offsets = self.hierarchy_tiles(&head, true, keep_going)?;
        let drawable =
            SourceDrawable { width: head.width, height: head.height, channels, indexed, precision: self.source_precision(), sample_order: order };
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
                // A pre-table tile must not consume the level dimensions or pointers.
                let boundary = if start < head.level { head.level } else { self.c.b.len() };
                let end = offsets.get(index + 1).copied().unwrap_or(boundary).min(boundary);
                let input = self.c.b.get(start..end).ok_or_else(|| invalid("a tile pointer is outside the file"))?;
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
