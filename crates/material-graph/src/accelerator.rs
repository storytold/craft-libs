//! Optional host accelerators for whole-image graph operations.

use std::{any::Any, fmt::Debug, sync::Arc};

use crate::context::Cancel;
use crate::func::{FnEnv, PixelShader, Prog};
use crate::tex::{Grey, Grey16, ImgRef, Tex};

pub struct PixelRequest<'a> {
    pub program: &'a Prog,
    pub environment: &'a FnEnv<'a>,
    pub size: [usize; 2],
    pub cancel: &'a Cancel,
}

#[derive(Clone)]
pub enum PixelOutput {
    Grey(Grey),
    Rgba(Tex),
}

/// Whole-image operations on the compiled graph's 16-bit grayscale values.
pub enum GreyPass<'a> {
    /// A wrapped bilinear identity transform enlarging through successive doublings.
    Enlarge { source: &'a Grey16, side: usize },
    /// A full-frame unmasked blend that keeps the background alpha.
    Blend { foreground: &'a Grey16, background: &'a Grey16, mode: u32, opacity: f32 },
}

/// Packed, quantized grayscale pixels owned by an accelerator.
pub trait ResidentGreyPixels: Debug + Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn read(&self, cancel: &Cancel) -> Result<Vec<u16>, String>;
}

#[derive(Debug)]
pub struct ResidentGrey16 {
    pub w: usize,
    pub h: usize,
    pub div: f32,
    pub alpha: f32,
    pub pixels: Arc<dyn ResidentGreyPixels>,
}

#[derive(Clone, Copy)]
pub enum GreyInput<'a> {
    Host(&'a Grey16),
    Device(&'a ResidentGrey16),
}

impl GreyInput<'_> {
    pub fn info(self) -> (usize, usize, f32, f32) {
        match self {
            Self::Host(t) => (t.w, t.h, t.div, t.alpha),
            Self::Device(t) => (t.w, t.h, t.div, t.alpha),
        }
    }
}

/// Transfers an opaque grayscale float image when a downstream operation needs CPU pixels.
pub trait ResidentFloatPixels: Debug + Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn read(&self, cancel: &Cancel) -> Result<Vec<f32>, String>;
}

#[derive(Debug)]
pub struct ResidentGreyFloat {
    pub w: usize,
    pub h: usize,
    pub pixels: Arc<dyn ResidentFloatPixels>,
}

#[derive(Clone, Copy)]
pub enum GreyTransformInput<'a> {
    Quantized(GreyInput<'a>),
    Float(&'a ResidentGreyFloat),
}

/// The wrapped bilinear identity transform that builds half-float graph pyramids.
pub struct HalfTransform<'a> {
    pub source: GreyTransformInput<'a>,
    pub side: usize,
    /// An explicit pyramid level, or automatic resizing when absent.
    pub mip: Option<u32>,
}

#[derive(Clone, Copy)]
pub enum PixelImageInput<'a> {
    Host(ImgRef<'a>),
    Float(&'a ResidentGreyFloat),
}

pub enum ResidentGreyPass<'a> {
    Enlarge {
        source: GreyInput<'a>,
        side: usize,
    },
    Reduce {
        source: GreyInput<'a>,
        side: usize,
    },
    /// Full-frame blend with its constant result alpha resolved by the graph.
    Blend {
        foreground: GreyInput<'a>,
        background: GreyInput<'a>,
        mask: Option<GreyInput<'a>>,
        mode: u32,
        opacity: f32,
        alpha: f32,
    },
    Levels {
        source: GreyInput<'a>,
        curve: Vec<u16>,
        div: f32,
        alpha: f32,
    },
}

/// Full-frame blend modes that operate independently on each color channel.
pub fn packed_blend_mode(mode: u32) -> bool {
    matches!(mode, 0..=16 | 20..=21 | 32..=36)
}

/// Packed RGBA in the compiled graph's unsigned 16-bit steps.
#[derive(Debug)]
pub struct ResidentColor16 {
    pub w: usize,
    pub h: usize,
    pub alpha: Option<f32>,
    pub pixels: Arc<dyn ResidentGreyPixels>,
}

#[derive(Clone, Copy)]
pub enum ColorInput<'a> {
    Grey(GreyInput<'a>),
    Host(&'a Tex),
    Device(&'a ResidentColor16),
}

/// Parameters shared by the grayscale, color and reference directional warps.
pub struct DirectionalWarpParams {
    pub intensity: f32,
    pub offset: f32,
    pub direction: [f32; 2],
    pub aspect: [f32; 2],
    pub tile: [bool; 2],
}

pub enum ResidentColorPass<'a> {
    Blend {
        foreground: ColorInput<'a>,
        background: ColorInput<'a>,
        mask: Option<ColorInput<'a>>,
        mode: u32,
        opacity: f32,
        alpha_mode: u32,
        alpha: Option<f32>,
    },
    Grayscale {
        source: ColorInput<'a>,
        weights: [f32; 4],
        /// Known binary alpha allows storage in the packed grayscale path.
        alpha: Option<f32>,
    },
    DirectionalWarp {
        source: ColorInput<'a>,
        map: GreyInput<'a>,
        settings: DirectionalWarpParams,
        alpha: Option<f32>,
    },
    WrappedAffine {
        source: &'a ResidentColor16,
        matrix: [f32; 4],
        offset: [f32; 2],
    },
}

pub enum ResidentColorOutput {
    Grey(ResidentGrey16),
    Color(ResidentColor16),
}

pub struct ResidentFilter<'a> {
    pub source: GreyInput<'a>,
    pub size: [usize; 2],
    pub tile: [bool; 2],
    pub div: f32,
    pub operation: FilterOperation<'a>,
}

pub enum FilterOperation<'a> {
    DirectionalWarp { map: GreyInput<'a>, intensity: f32, offset: f32, direction: [f32; 2], aspect: [f32; 2] },
    GradientWarp { map: GreyInput<'a>, displacement: [f32; 2] },
    Box { radius: [f32; 2] },
    ShortLine { major: usize, slope: f32, radius: f32 },
    ShearedLine { major: usize, slope: f32, radius: f32 },
    Sharpen { weights: [f32; 2] },
    Affine { matrix: [f32; 4], offset: [f32; 2], bilinear: bool, mip_side: Option<usize>, edge_fade: [usize; 2] },
}

pub trait Accelerator: Send + Sync {
    /// Identifies changes to rendered pixels for the host's result caches.
    fn cache_tag(&self) -> &str;

    /// `None` leaves this operation to the reference evaluator.
    fn pixel(&self, _request: PixelRequest<'_>) -> Result<Option<PixelOutput>, String> {
        Ok(None)
    }

    /// Draw the ordered instances of a procedural pattern program.
    fn pattern(&self, _request: crate::patterns::RasterRequest<'_>) -> Result<Option<crate::patterns::RasterOutput>, String> {
        Ok(None)
    }

    fn grey_pass(&self, _pass: GreyPass<'_>, _cancel: &Cancel) -> Result<Option<Grey16>, String> {
        Ok(None)
    }

    /// Execute a grayscale operation without fetching its result back to the host.
    fn resident_grey(&self, _pass: ResidentGreyPass<'_>, _cancel: &Cancel) -> Result<Option<ResidentGrey16>, String> {
        Ok(None)
    }

    fn resident_color(&self, _pass: ResidentColorPass<'_>, _cancel: &Cancel) -> Result<Option<ResidentColorOutput>, String> {
        Ok(None)
    }

    /// Execute a grayscale pixel program whose image inputs and output use packed integer samples.
    fn resident_pixel(
        &self,
        _shader: PixelShader,
        _images: &[GreyInput<'_>],
        _size: [usize; 2],
        _cancel: &Cancel,
    ) -> Result<Option<ResidentGrey16>, String> {
        Ok(None)
    }

    /// Execute a spatial filter on packed compiled grayscale graph pixels.
    fn resident_filter(&self, _filter: ResidentFilter<'_>, _cancel: &Cancel) -> Result<Option<ResidentGrey16>, String> {
        Ok(None)
    }

    fn resident_half_transform(&self, _pass: HalfTransform<'_>, _cancel: &Cancel) -> Result<Option<ResidentGreyFloat>, String> {
        Ok(None)
    }

    /// Writes packed grayscale while sampling resident float images and any remaining host images.
    fn resident_textured_pixel(
        &self,
        _shader: PixelShader,
        _images: &[PixelImageInput<'_>],
        _size: [usize; 2],
        _cancel: &Cancel,
    ) -> Result<Option<ResidentGrey16>, String> {
        Ok(None)
    }
}
