//! Alpha and luminosity masks.

use std::ops::DerefMut;

use pdf_writer::{Finish, Name, Ref};

use crate::chunk_container::ChunkContainer;
use crate::color::RegularColor;
use crate::geom::{Rect, Transform};
use crate::graphics::shading_function::{GradientProperties, ShadingFunction};
use crate::graphics::xobject::{GroupRole, XObject};
use crate::num::NormalizedF32;
use crate::resource;
use crate::resource::Resourceable;
use crate::serialize::{Cacheable, GroupColorSpace, SerializeContext};
use crate::stream::StreamBuilder;
use crate::stream::{FilterStreamBuilder, Stream};

/// A mask. Can be a luminance mask or an alpha mask.
///
/// IMPORTANT: Note that you must only use a mask in the document that you created it with!
/// If you use it in a different document, you will end up with an invalid PDF file.
// TODO: Maybe we can enfore the above somehow?
#[derive(PartialEq, Eq, Debug, Hash)]
pub struct Mask {
    /// The stream of the mask.
    stream: Stream,
    /// The type of the mask.
    mask_type: MaskType,
    /// A custom bbox of the mask. The only reason we need this is that for gradients with
    /// transparencies, we create a custom mask where we call the shading operator. In this case,
    /// we want to manually set the bbox of the underlying XObject to match the shape that the
    /// gradient is being applied to.
    custom_bbox: Option<Rect>,
    /// `/BC`: the backdrop color of a luminosity mask.
    backdrop: Option<RegularColor>,
    /// `/TR`: the transfer function from group alpha or luminosity to mask value.
    transfer: Option<TransferFunction>,
}

impl Mask {
    /// Create a new mask. `stream` contains the content description
    /// of the mask, and `mask_type` indicates the type of mask.
    pub fn new(stream: Stream, mask_type: MaskType) -> Self {
        Self {
            stream,
            mask_type,
            custom_bbox: None,
            backdrop: None,
            transfer: None,
        }
    }

    /// Set the backdrop color (`/BC`) of a luminosity mask: the color the mask group is
    /// composited over, which gives the mask value wherever the mask content paints nothing.
    /// The default is black, so the masked object is hidden outside the mask content.
    ///
    /// The color is written in the mask group's color space: RGB under
    /// [`GroupColorSpace::Rgb`]; under [`GroupColorSpace::Cmyk`], DeviceCMYK, or sGrey when
    /// `no_device_cs` is set. A color from another space is converted with the device
    /// formulas; luma maps to equal RGB components or to black ink only.
    ///
    /// Only luminosity masks have a backdrop color. On an alpha mask this returns the mask
    /// unchanged.
    pub fn with_backdrop(mut self, color: impl Into<RegularColor>) -> Self {
        if self.mask_type == MaskType::Luminosity {
            self.backdrop = Some(color.into());
        }
        self
    }

    /// Set the transfer function (`/TR`) that maps the group alpha or luminosity to the mask
    /// value. Without one, PDF readers use the identity.
    ///
    /// PDF/A-2 to 4 forbid the `TR` key of graphics states (6.2.5), not of soft-mask
    /// dictionaries, so krilla's validators accept it. PDF/X forbids transfer functions in
    /// graphics states; check the target preflight before using a soft-mask `/TR` in PDF/X.
    pub fn with_transfer(mut self, transfer: TransferFunction) -> Self {
        self.transfer = Some(transfer);
        self
    }

    /// Create a new mask for a shading to encode the opacity channels.
    pub(crate) fn new_from_shading(
        gradient_properties: GradientProperties,
        shading_transform: Transform,
        bbox: Rect,
        serializer_context: &mut SerializeContext,
        chunk_container: &mut ChunkContainer,
    ) -> Option<Self> {
        match &gradient_properties {
            GradientProperties::RadialAxialGradient(rag) => {
                if rag.stops.iter().all(|s| s.opacity.get() == 1.0) {
                    return None;
                }
            }
            GradientProperties::PostScriptGradient(psg) => {
                if psg.stops.iter().all(|s| s.opacity.get() == 1.0) {
                    return None;
                }
            }
            GradientProperties::MeshGradient(mg) => {
                if mg
                    .patches
                    .iter()
                    .all(|p| p.opacities.iter().all(|o| o.get() == 1.0))
                {
                    return None;
                }
            }
        }

        let shading_function = ShadingFunction::new(gradient_properties, true);

        let shading_stream = {
            let mut builder = StreamBuilder::new(serializer_context, chunk_container);
            let mut surface = builder.surface();
            surface.push_transform(&shading_transform);
            surface.draw_shading(&shading_function);
            surface.pop();
            surface.finish();
            builder.finish()
        };

        Some(Self {
            stream: shading_stream,
            mask_type: MaskType::Luminosity,
            custom_bbox: Some(bbox),
            backdrop: None,
            transfer: None,
        })
    }
}

/// A mask type.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum MaskType {
    /// A luminosity mask.
    Luminosity,
    /// An alpha mask.
    Alpha,
}

impl MaskType {
    /// Return the PDF name of the mask type.
    pub(crate) fn to_name(self) -> Name<'static> {
        match self {
            MaskType::Alpha => Name(b"Alpha"),
            MaskType::Luminosity => Name(b"Luminosity"),
        }
    }
}

/// The transfer function (`/TR`) of a soft mask.
#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub enum TransferFunction {
    /// Mask values equal the group alpha or luminosity (`/Identity`).
    Identity,
    /// A curve sampled at equal steps over the input range 0 to 1, first sample at 0 and last
    /// at 1, interpolated linearly in between. Written as a Type 0 (sampled) function with
    /// 16-bit samples. An empty curve is written as `/Identity`; a single sample gives a
    /// constant mask value.
    Sampled(Vec<NormalizedF32>),
}

impl Cacheable for Mask {
    fn serialize(
        self,
        sc: &mut SerializeContext,
        chunk_container: &mut ChunkContainer,
        root_ref: Ref,
    ) {
        // Under RGB blending a mask source group is written like a content group, so
        // keeping the content role there leaves object deduplication as it was.
        let role = match sc.serialize_settings().group_color_space {
            GroupColorSpace::Rgb => GroupRole::Content,
            GroupColorSpace::Cmyk => GroupRole::MaskSource,
        };
        let x_object = sc.register_cacheable(
            chunk_container,
            XObject::with_role(self.stream, false, true, self.custom_bbox, role),
        );

        let backdrop = self
            .backdrop
            .filter(|_| self.mask_type == MaskType::Luminosity)
            .map(|color| {
                let settings = sc.serialize_settings();
                match settings.group_color_space {
                    GroupColorSpace::Rgb => backdrop_rgb(color).to_vec(),
                    GroupColorSpace::Cmyk if settings.no_device_cs => vec![backdrop_gray(color)],
                    GroupColorSpace::Cmyk => backdrop_cmyk(color).to_vec(),
                }
            });

        let transfer = match &self.transfer {
            Some(TransferFunction::Sampled(samples)) if !samples.is_empty() => {
                Some(Some(write_sampled_transfer(sc, chunk_container, samples)))
            }
            Some(_) => Some(None),
            None => None,
        };

        let chunk = &mut chunk_container.non_stream.masks;
        let mut dict = chunk.indirect(root_ref).dict();
        dict.pair(Name(b"Type"), Name(b"Mask"));
        dict.pair(Name(b"S"), self.mask_type.to_name());
        dict.pair(Name(b"G"), x_object);

        if let Some(backdrop) = backdrop {
            dict.insert(Name(b"BC")).array().items(backdrop);
        }

        match transfer {
            Some(Some(function)) => {
                dict.pair(Name(b"TR"), function);
            }
            Some(None) => {
                dict.pair(Name(b"TR"), Name(b"Identity"));
            }
            None => {}
        }

        dict.finish();
    }
}

/// A backdrop color in an RGB mask group.
fn backdrop_rgb(color: RegularColor) -> [f32; 3] {
    match color {
        RegularColor::Rgb(rgb) => rgb.to_pdf_color(),
        RegularColor::Luma(luma) => [luma.to_pdf_color(); 3],
        RegularColor::Cmyk(cmyk) => {
            let [c, m, y, k] = cmyk.to_pdf_color();
            [
                (1.0 - c) * (1.0 - k),
                (1.0 - m) * (1.0 - k),
                (1.0 - y) * (1.0 - k),
            ]
        }
    }
}

/// A backdrop color in a gray mask group.
fn backdrop_gray(color: RegularColor) -> f32 {
    match color {
        RegularColor::Luma(luma) => luma.to_pdf_color(),
        RegularColor::Rgb(rgb) => {
            let [r, g, b] = rgb.to_pdf_color();
            0.3 * r + 0.59 * g + 0.11 * b
        }
        RegularColor::Cmyk(cmyk) => {
            let [c, m, y, k] = cmyk.to_pdf_color();
            1.0 - (0.3 * c + 0.59 * m + 0.11 * y + k).min(1.0)
        }
    }
}

/// A backdrop color in a CMYK mask group.
fn backdrop_cmyk(color: RegularColor) -> [f32; 4] {
    match color {
        RegularColor::Cmyk(cmyk) => cmyk.to_pdf_color(),
        RegularColor::Luma(luma) => [0.0, 0.0, 0.0, 1.0 - luma.to_pdf_color()],
        RegularColor::Rgb(rgb) => {
            let [r, g, b] = rgb.to_pdf_color();
            let k = 1.0 - r.max(g).max(b);
            if k >= 1.0 {
                return [0.0, 0.0, 0.0, 1.0];
            }
            let ink = |v: f32| (1.0 - v - k) / (1.0 - k);
            [ink(r), ink(g), ink(b), k]
        }
    }
}

/// Write a sampled transfer curve as a Type 0 function and return its reference.
fn write_sampled_transfer(
    sc: &mut SerializeContext,
    chunk_container: &mut ChunkContainer,
    samples: &[NormalizedF32],
) -> Ref {
    let root_ref = sc.new_ref();
    let serialize_settings = sc.serialize_settings();

    // A Type 0 function needs two samples to span its domain.
    let samples = match samples {
        [only] => vec![*only, *only],
        _ => samples.to_vec(),
    };
    let data: Vec<u8> = samples
        .iter()
        .flat_map(|sample| ((sample.get() * 65535.0).round() as u16).to_be_bytes())
        .collect();

    let stream = FilterStreamBuilder::new_from_binary_data(&data).finish(&serialize_settings);
    let mut chunk = sc.new_chunk();
    let mut function = chunk.sampled_function(root_ref, stream.encoded_data());
    stream.write_filters(function.deref_mut().deref_mut());
    function.domain([0.0, 1.0]);
    function.range([0.0, 1.0]);
    function.size([samples.len() as i32]);
    function.bits_per_sample(16);
    function.finish();
    chunk_container.streams.shading_functions.push(chunk);

    root_ref
}

impl Resourceable for Mask {
    type Resource = resource::XObject;
}
