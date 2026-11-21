//! A codec-agnostic decoded video frame.
//!
//! Each decoder in this crate returns its own picture type. [`VideoFrame`] holds what they have in
//! common: planar samples at their stored width (`u8` or `u16`), per-plane bit depth, chroma
//! subsampling, an optional alpha plane, colour tags as ITU-T H.273 code points, and timing.
//! Every decoder output converts into it with `VideoFrame::try_from`, behind the codec's feature.
//! A conversion moves the sample buffers; it does not copy or rescale samples. Paletted video is
//! the exception: VMD's decoder holds palette indices, and converting it (`&vmd::Decoder`) writes
//! new 8-bit R, G, B planes from its palette.
//!
//! Plane order is fixed by [`ColorFamily`]: `[Y, Cb, Cr]`, `[R, G, B]` or `[Y]`. Decoders that
//! store RGB in another order (DNx, VP9 and identity-matrix AV1 store G, B, R) are reordered by
//! the conversion.
//!
//! The constructors check that the planes and their buffers agree in size, so reading a row
//! never goes out of bounds. They do not check that sample values fit the bit depth.

mod convert;

/// Why a frame or plane was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// A plane's buffer, stride or bit depth does not fit its size.
    #[error("invalid plane: {0}")]
    Plane(&'static str),
    /// The planes do not fit the frame size, colour family or subsampling.
    #[error("invalid frame: {0}")]
    Shape(&'static str),
}

/// The samples of one plane, at the width the decoder stores them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Samples {
    /// 8-bit storage (bit depth 1 to 8).
    U8(Vec<u8>),
    /// 16-bit storage (bit depth 1 to 16), values below `1 << bit_depth`.
    U16(Vec<u16>),
}

impl Samples {
    /// Number of stored samples.
    pub fn len(&self) -> usize {
        match self {
            Samples::U8(v) => v.len(),
            Samples::U16(v) => v.len(),
        }
    }

    /// Whether no samples are stored.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One plane of samples: `height` rows of `width` samples, each row starting `stride` samples
/// after the previous one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plane {
    width: u32,
    height: u32,
    stride: usize,
    bit_depth: u8,
    samples: Samples,
}

impl Plane {
    /// A plane with the given row stride. Fails when `stride < width`, when `bit_depth` is 0, above
    /// 16, or above 8 for [`Samples::U8`], or when `samples` is shorter than the rows need.
    pub fn new(width: u32, height: u32, stride: usize, bit_depth: u8, samples: Samples) -> Result<Plane, FrameError> {
        let w = usize::try_from(width).map_err(|_| FrameError::Plane("width does not fit in usize"))?;
        let h = usize::try_from(height).map_err(|_| FrameError::Plane("height does not fit in usize"))?;
        if stride < w {
            return Err(FrameError::Plane("stride is less than the width"));
        }
        let max_depth = match samples {
            Samples::U8(_) => 8,
            Samples::U16(_) => 16,
        };
        if bit_depth == 0 || bit_depth > max_depth {
            return Err(FrameError::Plane("bit depth does not fit the sample storage"));
        }
        let needed = match h.checked_sub(1) {
            None => 0,
            Some(last) => last.checked_mul(stride).and_then(|n| n.checked_add(w)).ok_or(FrameError::Plane("plane size overflows"))?,
        };
        if samples.len() < needed {
            return Err(FrameError::Plane("buffer is shorter than the plane"));
        }
        Ok(Plane { width, height, stride, bit_depth, samples })
    }

    /// A plane whose rows are packed (`stride == width`).
    pub fn packed(width: u32, height: u32, bit_depth: u8, samples: Samples) -> Result<Plane, FrameError> {
        let stride = usize::try_from(width).map_err(|_| FrameError::Plane("width does not fit in usize"))?;
        Plane::new(width, height, stride, bit_depth, samples)
    }

    /// Samples per row.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Rows.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Distance between the starts of two rows, in samples.
    pub fn stride(&self) -> usize {
        self.stride
    }

    /// Significant bits per sample.
    pub fn bit_depth(&self) -> u8 {
        self.bit_depth
    }

    /// The stored samples, rows `stride` apart.
    pub fn samples(&self) -> &Samples {
        &self.samples
    }

    /// The stored samples, giving up the plane.
    pub fn into_samples(self) -> Samples {
        self.samples
    }

    /// Row `y` of an 8-bit plane: `None` for a 16-bit plane or a row past the end.
    pub fn row_u8(&self, y: u32) -> Option<&[u8]> {
        match &self.samples {
            Samples::U8(v) => v.get(self.row_range(y)?),
            Samples::U16(_) => None,
        }
    }

    /// Row `y` of a 16-bit plane: `None` for an 8-bit plane or a row past the end.
    pub fn row_u16(&self, y: u32) -> Option<&[u16]> {
        match &self.samples {
            Samples::U16(v) => v.get(self.row_range(y)?),
            Samples::U8(_) => None,
        }
    }

    fn row_range(&self, y: u32) -> Option<std::ops::Range<usize>> {
        if y >= self.height {
            return None;
        }
        let start = usize::try_from(y).ok()?.checked_mul(self.stride)?;
        Some(start..start.checked_add(usize::try_from(self.width).ok()?)?)
    }
}

/// What the colour planes hold, and in which order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorFamily {
    /// `[Y, Cb, Cr]`, chroma subsampled by [`VideoFrame::subsampling`].
    YCbCr,
    /// `[R, G, B]`, all at full size.
    Rgb,
    /// `[Y]` alone.
    Gray,
}

/// Chroma subsampling as log2 factors: the chroma planes are `width.div_ceil(1 << x)` by
/// `height.div_ceil(1 << y)`. FFV1 codes shifts up to 3; the other codecs stop at 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Subsampling {
    /// log2 of the horizontal factor (0 to 3).
    pub x: u8,
    /// log2 of the vertical factor (0 to 3).
    pub y: u8,
}

impl Subsampling {
    /// 4:2:0.
    pub const S420: Subsampling = Subsampling { x: 1, y: 1 };
    /// 4:2:2.
    pub const S422: Subsampling = Subsampling { x: 1, y: 0 };
    /// 4:4:4, also used for RGB and gray frames.
    pub const S444: Subsampling = Subsampling { x: 0, y: 0 };

    /// Largest shift a frame accepts.
    pub const MAX_SHIFT: u8 = 3;

    /// Chroma plane size for a luma size. A shift above [`Subsampling::MAX_SHIFT`] counts as that.
    pub fn chroma_size(self, width: u32, height: u32) -> (u32, u32) {
        (width.div_ceil(1 << self.x.min(Self::MAX_SHIFT)), height.div_ceil(1 << self.y.min(Self::MAX_SHIFT)))
    }
}

/// Colour description as ITU-T H.273 code points.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ColorTags {
    /// `ColourPrimaries` (2 = unspecified).
    pub primaries: u8,
    /// `TransferCharacteristics` (2 = unspecified).
    pub transfer: u8,
    /// `MatrixCoefficients` (0 = identity, used for RGB; 2 = unspecified).
    pub matrix: u8,
    /// `VideoFullRangeFlag`: full range when true. False when the stream does not say, except
    /// that an RGB frame that does not signal a range is full range.
    pub full_range: bool,
}

impl ColorTags {
    /// Nothing signalled.
    pub const UNSPECIFIED: ColorTags = ColorTags { primaries: 2, transfer: 2, matrix: 2, full_range: false };
}

impl Default for ColorTags {
    fn default() -> Self {
        ColorTags::UNSPECIFIED
    }
}

/// Scan structure of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum FieldOrder {
    /// Progressive, or not signalled.
    #[default]
    Progressive,
    /// Two interlaced fields; the top field (even rows) comes first.
    TopFieldFirst,
    /// Two interlaced fields; the bottom field (odd rows) comes first.
    BottomFieldFirst,
}

/// A decoded frame in planar form, independent of the codec.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoFrame {
    width: u32,
    height: u32,
    family: ColorFamily,
    subsampling: Subsampling,
    planes: Vec<Plane>,
    alpha: Option<Plane>,
    /// Colour description.
    pub color: ColorTags,
    /// The timestamp passed to the decoder with the packet that produced the frame; `None` for
    /// codecs whose decoders take no timestamp (the intra-only codecs).
    pub pts: Option<i64>,
    /// Sample aspect ratio, `None` when not signalled.
    pub sar: Option<(u32, u32)>,
    /// Field order.
    pub field_order: FieldOrder,
    /// Decoded in a decoder's draft mode (some filtering skipped): the samples are approximate.
    pub draft: bool,
}

impl VideoFrame {
    /// A frame from its planes, in the order [`ColorFamily`] gives. Fails when the number of planes
    /// does not fit `family`, a plane's size does not follow from `width`, `height` and
    /// `subsampling`, a shift is above 3, or an RGB or gray frame is subsampled. The alpha plane is
    /// full size. The colour tags and timing start unset, except that an RGB frame starts full
    /// range.
    pub fn new(
        width: u32,
        height: u32,
        family: ColorFamily,
        subsampling: Subsampling,
        planes: Vec<Plane>,
        alpha: Option<Plane>,
    ) -> Result<VideoFrame, FrameError> {
        if subsampling.x > Subsampling::MAX_SHIFT || subsampling.y > Subsampling::MAX_SHIFT {
            return Err(FrameError::Shape("subsampling shift above 3"));
        }
        if family != ColorFamily::YCbCr && subsampling != Subsampling::S444 {
            return Err(FrameError::Shape("only YCbCr frames are subsampled"));
        }
        let expected = if family == ColorFamily::Gray { 1 } else { 3 };
        if planes.len() != expected {
            return Err(FrameError::Shape("plane count does not fit the colour family"));
        }
        let chroma = subsampling.chroma_size(width, height);
        for (i, p) in planes.iter().enumerate() {
            let size = if i == 0 { (width, height) } else { chroma };
            if (p.width, p.height) != size {
                return Err(FrameError::Shape("plane size does not fit the frame"));
            }
        }
        if let Some(a) = &alpha
            && (a.width, a.height) != (width, height)
        {
            return Err(FrameError::Shape("alpha plane size does not fit the frame"));
        }
        Ok(VideoFrame {
            width,
            height,
            family,
            subsampling,
            planes,
            alpha,
            color: ColorTags { full_range: family == ColorFamily::Rgb, ..ColorTags::UNSPECIFIED },
            pts: None,
            sar: None,
            field_order: FieldOrder::Progressive,
            draft: false,
        })
    }

    /// Luma width (the size of plane 0).
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Luma height.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// What the planes hold.
    pub fn family(&self) -> ColorFamily {
        self.family
    }

    /// Chroma subsampling ([`Subsampling::S444`] for RGB and gray).
    pub fn subsampling(&self) -> Subsampling {
        self.subsampling
    }

    /// The colour planes, in the order [`ColorFamily`] gives.
    pub fn planes(&self) -> &[Plane] {
        &self.planes
    }

    /// Colour plane `i`, `None` past the last one.
    pub fn plane(&self, i: usize) -> Option<&Plane> {
        self.planes.get(i)
    }

    /// The alpha plane (0 = transparent, the bit depth's maximum = opaque).
    pub fn alpha(&self) -> Option<&Plane> {
        self.alpha.as_ref()
    }

    /// Bit depth of plane 0. Chroma may differ; see [`Plane::bit_depth`].
    pub fn bit_depth(&self) -> u8 {
        self.planes.first().map_or(0, Plane::bit_depth)
    }

    /// The colour planes and the alpha plane, giving up the frame.
    pub fn into_planes(self) -> (Vec<Plane>, Option<Plane>) {
        (self.planes, self.alpha)
    }
}

#[cfg(test)]
mod tests;
