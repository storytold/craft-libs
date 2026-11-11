//! The backdrop layering, the layered image and the freeze mask source.

use craft_behavior::PixelSize;
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::primitives::Revision;
use egui::{Color32, ColorImage};

use crate::ui::liquify::session::BackdropMode;

/// The most a tint's alpha can be: the layer is drawn as it is.
const OPAQUE: u8 = u8::MAX;

/// How the image and the backdrop are drawn for Show Backdrop: the opacity (0..=255) of the image and of the
/// backdrop drawn behind or in front of it.
///
/// | Mode | Image | Backdrop |
/// |---|---|---|
/// | none shown | opaque | — |
/// | In Front | opaque | in front, at the opacity |
/// | Behind | at the opacity | behind, opaque |
/// | Blend | at 1 − opacity | in front, at the opacity |
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct BackdropLayering {
    /// The opacity of the image.
    pub image: u8,
    /// The opacity of the backdrop drawn behind the image, if any.
    pub behind: Option<u8>,
    /// The opacity of the backdrop drawn in front of the image, if any.
    pub front: Option<u8>,
}

impl BackdropLayering {
    /// The layering for a backdrop that is shown (`shown`) in `mode` at `opacity` (0..=100).
    pub fn of(shown: bool, mode: BackdropMode, opacity: f64) -> BackdropLayering {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:876 `backdrop_layering` (Photoshop's modes,
        // measured on its Liquify, #7609).
        let fraction = if opacity.is_finite() { (opacity / 100.0).clamp(0.0, 1.0) } else { 0.0 };
        let alpha = |a: f64| (a.clamp(0.0, 1.0) * f64::from(OPAQUE)).round() as u8;
        if !shown {
            return BackdropLayering { image: OPAQUE, behind: None, front: None };
        }
        match mode {
            BackdropMode::InFront => BackdropLayering { image: OPAQUE, behind: None, front: Some(alpha(fraction)) },
            BackdropMode::Behind => BackdropLayering { image: alpha(fraction), behind: Some(OPAQUE), front: None },
            BackdropMode::Blend => BackdropLayering { image: alpha(1.0 - fraction), behind: None, front: Some(alpha(fraction)) },
        }
    }

    /// Whether the image is drawn as it is, with nothing behind or in front of it.
    pub(crate) fn is_plain(&self) -> bool {
        self.behind.is_none() && self.front.is_none() && self.image == OPAQUE
    }
}

/// The image with the backdrop laid behind and in front of it as a [`BackdropLayering`] says,
/// composited into one picture so the canvas draws it through its one image.
pub struct LayeredImage<'a> {
    image: &'a dyn RasterSource,
    backdrop: &'a dyn RasterSource,
    layering: BackdropLayering,
}

impl<'a> LayeredImage<'a> {
    /// `image` laid out as `layering` says over and under `backdrop`.
    pub fn new(image: &'a dyn RasterSource, backdrop: &'a dyn RasterSource, layering: BackdropLayering) -> Self {
        Self { image, backdrop, layering }
    }
}

/// `texel` (sRGBA, unmultiplied) with its alpha scaled by `opacity`.
fn faded(texel: Color32, opacity: u8) -> Color32 {
    let alpha = (u16::from(texel.a()) * u16::from(opacity) + 127) / 255;
    Color32::from_rgba_premultiplied(texel.r(), texel.g(), texel.b(), alpha as u8)
}

/// `top` over `bottom`, both sRGBA unmultiplied.
fn over(bottom: Color32, top: Color32) -> Color32 {
    let (ta, ba) = (f32::from(top.a()) / 255.0, f32::from(bottom.a()) / 255.0);
    let alpha = ta + ba * (1.0 - ta);
    if alpha <= 0.0 {
        return Color32::TRANSPARENT;
    }
    let mix = |t: u8, b: u8| ((f32::from(t) * ta + f32::from(b) * ba * (1.0 - ta)) / alpha).round() as u8;
    Color32::from_rgba_premultiplied(mix(top.r(), bottom.r()), mix(top.g(), bottom.g()), mix(top.b(), bottom.b()), (alpha * 255.0).round() as u8)
}

impl RasterSource for LayeredImage<'_> {
    fn size(&self) -> PixelSize {
        self.image.size()
    }

    fn revision(&self) -> Revision {
        let layering = (self.layering.image, self.layering.behind, self.layering.front);
        Revision::of(&(self.image.revision(), self.backdrop.revision(), layering))
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        None
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        out.pixels.fill(Color32::TRANSPARENT);
        let backdrop_fits = self.backdrop.size() == self.image.size();
        let mut layer = ColorImage::filled(out.size, Color32::TRANSPARENT);
        let layers = [
            self.layering.behind.filter(|_| backdrop_fits).map(|opacity| (self.backdrop, opacity)),
            Some((self.image, self.layering.image)),
            self.layering.front.filter(|_| backdrop_fits).map(|opacity| (self.backdrop, opacity)),
        ];
        for (source, opacity) in layers.into_iter().flatten() {
            layer.pixels.fill(Color32::TRANSPARENT);
            source.read(region, factor, &mut layer);
            for (below, above) in out.pixels.iter_mut().zip(&layer.pixels) {
                *below = over(*below, faded(*above, opacity));
            }
        }
    }
}

/// The freeze mask at its reduced resolution, answering `read` in document pixels: each mask texel covers `texel`
/// document pixels each way.
pub(crate) struct MaskAtTexel<'a> {
    pub(crate) mask: &'a dyn RasterSource,
    pub(crate) texel: u32,
    pub(crate) document: PixelSize,
}

impl RasterSource for MaskAtTexel<'_> {
    fn size(&self) -> PixelSize {
        self.document
    }

    fn revision(&self) -> Revision {
        Revision::of(&(self.mask.revision(), self.texel))
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        None
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        out.pixels.fill(Color32::TRANSPARENT);
        let texel = i32::try_from(self.texel.max(1)).unwrap_or(1);
        let mask = PixelRect::of_size(self.mask.size());
        let wanted = PixelRect::new(
            region.x0.div_euclid(texel),
            region.y0.div_euclid(texel),
            (region.x1 + texel - 1).div_euclid(texel),
            (region.y1 + texel - 1).div_euclid(texel),
        )
        .intersect(&mask);
        if wanted.is_empty() {
            return;
        }
        let mut reduced = ColorImage::filled([wanted.width() as usize, wanted.height() as usize], Color32::TRANSPARENT);
        self.mask.read(wanted, 1, &mut reduced);
        let [width, height] = out.size;
        let step = i32::try_from(factor.max(1)).unwrap_or(1);
        for row in 0..height {
            let y = (region.y0 + row as i32 * step).div_euclid(texel) - wanted.y0;
            for column in 0..width {
                let x = (region.x0 + column as i32 * step).div_euclid(texel) - wanted.x0;
                if (0..wanted.width()).contains(&x) && (0..wanted.height()).contains(&y) {
                    out.pixels[row * width + column] = reduced.pixels[y as usize * reduced.size[0] + x as usize];
                }
            }
        }
    }
}
