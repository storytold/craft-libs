//! The filters, their limits and their errors, shared by the CPU reference, the compute passes and
//! the job.

/// The largest side of an image any filter takes.
pub const MAX_SIDE: u32 = 16_384;

/// The most pixels an image any filter takes may have (64 Mi pixels, 256 MiB of RGBA8).
pub const MAX_PIXELS: u64 = 1 << 26;

/// The largest Gaussian blur sigma, in pixels. The kernel reaches `ceil(3 sigma)` pixels to each
/// side, so this caps a blur at 1201 taps a pass.
pub const MAX_SIGMA: f32 = 200.0;

/// Per-channel lookup tables, applied to straight (un-premultiplied) colour.
///
/// `tables[0..3]` map red, green and blue, `tables[3]` maps alpha. For each pixel, with its
/// premultiplied channels `c` and alpha `a` (colour above alpha is read as alpha):
///
/// - the straight colour is `s = (c·255 + a/2) / a` in integer arithmetic, and `0` when `a = 0`;
/// - the new alpha is `a' = tables[3][a]`;
/// - the new premultiplied colour is `(tables[ch][s] · a' + 127) / 255`.
///
/// Every step is integer arithmetic, so the CPU and the GPU give the same bytes. The identity
/// tables give back every valid premultiplied pixel unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lut {
    pub tables: [[u8; 256]; 4],
}

impl Lut {
    /// The tables that change nothing.
    pub fn identity() -> Lut {
        let mut table = [0u8; 256];
        for (i, v) in table.iter_mut().enumerate() {
            *v = i as u8;
        }
        Lut { tables: [table; 4] }
    }
}

impl Default for Lut {
    fn default() -> Self {
        Self::identity()
    }
}

/// One filter on premultiplied RGBA8 images.
#[derive(Clone, Debug, PartialEq)]
pub enum Filter {
    /// A separable Gaussian blur of the premultiplied channels. Pixels outside the image are
    /// transparent black, so the edges fade. `sigma` is in pixels, from 0 (no change) to
    /// [`MAX_SIGMA`]. See [`kernel`] for the weights.
    GaussianBlur { sigma: f32 },
    /// Per-channel lookup tables on straight colour, re-premultiplied.
    Lut(Box<Lut>),
    /// The layer (the second input) over the backdrop (the first input), premultiplied
    /// source-over, with the layer scaled by `opacity` from 0 to 1. `opacity` is quantized to
    /// `o = round(opacity · 255)`; then, with `x·y/255` rounded as `(x·y + 127) / 255`, the layer
    /// is `s = s·o/255` and each output channel is `s + d·(255 − s_alpha)/255`.
    Composite { opacity: f32 },
}

impl Filter {
    /// How many images the filter reads: two for [`Filter::Composite`], one otherwise.
    pub fn inputs(&self) -> usize {
        match self {
            Filter::Composite { .. } => 2,
            Filter::GaussianBlur { .. } | Filter::Lut(_) => 1,
        }
    }

    /// Whether the parameters are in range: a finite sigma from 0 to [`MAX_SIGMA`], a finite
    /// opacity from 0 to 1.
    pub fn validate(&self) -> Result<(), Error> {
        match *self {
            Filter::GaussianBlur { sigma } if !(sigma.is_finite() && (0.0..=MAX_SIGMA).contains(&sigma)) => Err(Error::Sigma(sigma)),
            Filter::Composite { opacity } if !(opacity.is_finite() && (0.0..=1.0).contains(&opacity)) => Err(Error::Opacity(opacity)),
            _ => Ok(()),
        }
    }

    /// The bytes of the largest buffer a run of this filter on `pixels` pixels makes: the blur's
    /// intermediate image holds 8 bytes a pixel, every other buffer 4.
    pub fn largest_buffer(&self, pixels: u64) -> u64 {
        match self {
            Filter::GaussianBlur { .. } => pixels.saturating_mul(8),
            Filter::Lut(_) | Filter::Composite { .. } => pixels.saturating_mul(4),
        }
    }
}

/// The opacity of a [`Filter::Composite`] as the integer weight `0..=255` both paths use.
pub(crate) fn opacity_weight(opacity: f32) -> u32 {
    (0.5 + 255.0 * opacity.clamp(0.0, 1.0)).floor() as u32
}

/// The weights of the Gaussian blur of `sigma`, from the centre tap out: `w[0]` is the centre and
/// `w[i]` the weight of the pixels `i` to each side. The kernel reaches `r = ceil(3 sigma)` pixels;
/// the raw weights `exp(-i² / (2 sigma²))` are scaled so that `w[0] + 2 Σ w[1..=r]` is 1. A sigma
/// of 0 is the single weight 1. Both paths take these weights, computed here once.
pub fn kernel(sigma: f32) -> Result<Vec<f32>, Error> {
    Filter::GaussianBlur { sigma }.validate()?;
    let radius = (3.0 * sigma).ceil() as usize;
    let mut weights = Vec::with_capacity(radius + 1);
    weights.push(1.0f64);
    let two_s2 = 2.0 * f64::from(sigma) * f64::from(sigma);
    for i in 1..=radius {
        let d = i as f64;
        // A sigma so small that 2 sigma² is 0 gives exp(-inf) = 0: the centre alone.
        weights.push((-(d * d) / two_s2).exp());
    }
    let total: f64 = weights[0] + 2.0 * weights[1..].iter().sum::<f64>();
    Ok(weights.into_iter().map(|w| (w / total) as f32).collect())
}

/// The pixel count of a `width × height` image, if both sides are from 1 to [`MAX_SIDE`] and the
/// count is at most [`MAX_PIXELS`].
pub(crate) fn pixels(width: u32, height: u32) -> Result<u64, Error> {
    if width == 0 || height == 0 {
        return Err(Error::Empty { width, height });
    }
    let pixels = u64::from(width) * u64::from(height);
    if width > MAX_SIDE || height > MAX_SIDE || pixels > MAX_PIXELS {
        return Err(Error::TooLarge { width, height });
    }
    Ok(pixels)
}

/// Check that `filter` can run on `inputs`, each a tightly packed `width × height` RGBA8 image.
/// Returns the pixel count.
pub(crate) fn check(filter: &Filter, width: u32, height: u32, inputs: &[&[u8]]) -> Result<u64, Error> {
    filter.validate()?;
    let pixels = pixels(width, height)?;
    if inputs.len() != filter.inputs() {
        return Err(Error::Inputs { expected: filter.inputs(), got: inputs.len() });
    }
    let expected = pixels * 4;
    for input in inputs {
        if input.len() as u64 != expected {
            return Err(Error::Source { width, height, expected, bytes: input.len() as u64 });
        }
    }
    Ok(pixels)
}

/// Why a filter did not run.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("the image is {width} × {height}: both sides must be at least 1")]
    Empty { width: u32, height: u32 },
    #[error("the image is {width} × {height}: the limit is {MAX_SIDE} pixels a side and {MAX_PIXELS} pixels in all")]
    TooLarge { width: u32, height: u32 },
    #[error("a {width} × {height} RGBA8 image is {expected} bytes, but the buffer holds {bytes}")]
    Source { width: u32, height: u32, expected: u64, bytes: u64 },
    #[error("the filter takes {expected} images, not {got}")]
    Inputs { expected: usize, got: usize },
    #[error("the filter needs a buffer of {needed} bytes, but the device allows {limit}")]
    BufferLimit { needed: u64, limit: u64 },
    #[error("the blur sigma {0} is not a finite number from 0 to {MAX_SIGMA}")]
    Sigma(f32),
    #[error("the opacity {0} is not a finite number from 0 to 1")]
    Opacity(f32),
    #[error("{bytes} bytes could not be allocated")]
    Memory { bytes: u64 },
    #[error("the device reported an error: {0}")]
    Device(String),
}

/// `len` default values, or [`Error::Memory`] when they cannot be allocated.
pub(crate) fn zeroed<T: Clone + Default>(len: usize) -> Result<Vec<T>, Error> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| Error::Memory { bytes: (len as u64).saturating_mul(std::mem::size_of::<T>() as u64) })?;
    v.resize(len, T::default());
    Ok(v)
}
