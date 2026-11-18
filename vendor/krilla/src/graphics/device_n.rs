use std::ops::DerefMut;

use pdf_writer::types::PostScriptOp;
use pdf_writer::{Finish, Name, Ref};

use crate::chunk_container::ChunkContainer;
use crate::color::device_n::DeviceNSpace;
use crate::color::separation::{SeparationColorant, SeparationSpace};
use crate::color::{cmyk, ColorSpace, RegularColor, SpecialColorSpace};
use crate::color::{DEVICE_CMYK, DEVICE_GRAY, DEVICE_RGB};
use crate::configure::{PdfVersion, ValidationError};
use crate::resource::{self, Resource, Resourceable};
use crate::serialize::{Cacheable, MaybeDeviceColorSpace, SerializeContext};
use crate::stream::FilterStreamBuilder;

/// A DeviceN color space, written as
/// `[/DeviceN [names] alternate tintTransform attributes?]`.
///
/// The alternate is DeviceCMYK, or the `cmyk_profile` ICC space with `no_device_cs`. The
/// attributes dictionary lists each spot's Separation color space under `/Colorants`
/// (PDF 1.3). From PDF 1.6, when spots and process colorants mix, it is an `NChannel` space
/// whose `/Process` names the CMYK space and components; `/Subtype` and `/Process` don't
/// exist before 1.6, and PDF/X-1a preflights reject them.
#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub(crate) struct DeviceNColorSpace {
    space: DeviceNSpace,
}

impl DeviceNColorSpace {
    pub fn new(space: DeviceNSpace) -> Self {
        Self { space }
    }
}

impl Cacheable for DeviceNColorSpace {
    fn serialize(
        self,
        sc: &mut SerializeContext,
        chunk_container: &mut ChunkContainer,
        root_ref: Ref,
    ) {
        let space = self.space;
        let alternate_cs = RegularColor::from(cmyk::Color::default()).color_space(sc);
        let alternate = sc.register_colorspace(chunk_container, alternate_cs.into());

        for (name, cmyk) in space.spots() {
            let colorant = SeparationColorant::Custom(name.to_owned());
            if let Err(e) = sc
                .validation_store()
                .validate_colorant(&colorant, cmyk.into())
            {
                sc.register_validation_error(e);
            }
        }

        let tint_ref = tint_transform(&space, sc, chunk_container);

        let mut separations = Vec::new();
        for (name, cmyk) in space.spots() {
            let separation =
                SeparationSpace::new(SeparationColorant::Custom(name.to_owned()), cmyk.into());
            let cs = ColorSpace::Special(SpecialColorSpace::Separation(separation));
            if let MaybeDeviceColorSpace::ColorSpace(cs) =
                sc.register_colorspace(chunk_container, cs)
            {
                separations.push((name.to_owned(), cs.get_ref()));
            }
        }
        let nchannel = sc.serialize_settings().pdf_version() >= PdfVersion::Pdf16
            && space.has_process()
            && !separations.is_empty();

        let chunk = &mut chunk_container.non_stream.color_spaces;
        let mut array = chunk.indirect(root_ref).array();
        array.item(Name(b"DeviceN"));
        array
            .push()
            .array()
            .items(space.colorants().iter().map(|c| Name(c.name().as_bytes())));
        write_color_space(&mut array, &alternate);
        array.item(tint_ref);

        if !separations.is_empty() {
            let mut attrs = array.push().dict();
            if nchannel {
                attrs.pair(Name(b"Subtype"), Name(b"NChannel"));
            }
            let mut colorants = attrs.insert(Name(b"Colorants")).dict();
            for (name, separation) in &separations {
                colorants.pair(Name(name.as_bytes()), *separation);
            }
            colorants.finish();
            if nchannel {
                let mut process = attrs.insert(Name(b"Process")).dict();
                match &alternate {
                    MaybeDeviceColorSpace::ColorSpace(cs) => {
                        process.pair(Name(b"ColorSpace"), cs.get_ref());
                    }
                    _ => {
                        process.pair(Name(b"ColorSpace"), Name(DEVICE_CMYK.as_bytes()));
                    }
                }
                process
                    .insert(Name(b"Components"))
                    .array()
                    .items(PROCESS_NAMES.map(|n| Name(n.as_bytes())));
            }
        }

        array.finish();
    }
}

impl Resourceable for DeviceNColorSpace {
    type Resource = resource::ColorSpace;
}

const PROCESS_NAMES: [&str; 4] = ["Cyan", "Magenta", "Yellow", "Black"];

fn write_color_space(array: &mut pdf_writer::Array<'_>, cs: &MaybeDeviceColorSpace) {
    match cs {
        MaybeDeviceColorSpace::DeviceRgb => array.item(Name(DEVICE_RGB.as_bytes())),
        MaybeDeviceColorSpace::DeviceGray => array.item(Name(DEVICE_GRAY.as_bytes())),
        MaybeDeviceColorSpace::DeviceCMYK => array.item(Name(DEVICE_CMYK.as_bytes())),
        MaybeDeviceColorSpace::ColorSpace(cs) => array.item(cs.get_ref()),
    };
}

/// Write the Type 4 tint transform from the space's tints to CMYK and return its
/// reference. It computes [`DeviceNSpace::alternate`].
fn tint_transform(
    space: &DeviceNSpace,
    sc: &mut SerializeContext,
    chunk_container: &mut ChunkContainer,
) -> Ref {
    use PostScriptOp::*;

    sc.register_validation_error(ValidationError::ContainsPostScript(sc.location));

    let n = space.colorants().len();
    let mut code = Vec::new();
    // The stack holds the n tints; each pass pushes one CMYK component on top.
    for component in 0..4 {
        let terms: Vec<(usize, f32)> = space
            .colorants()
            .iter()
            .enumerate()
            .map(|(i, c)| (i, c.alternate()[component]))
            .filter(|(_, a)| *a > 0.0)
            .collect();
        // Depth of tint `i` below the top, with `extra` values pushed above the tints.
        let depth = |i: usize, extra: usize| (n - 1 - i + extra) as i32;
        let scale = |code: &mut Vec<PostScriptOp<'static>>, a: f32| {
            if a < 1.0 {
                code.extend([Real(a), Mul]);
            }
        };
        match terms.as_slice() {
            [] => code.push(Real(0.0)),
            [(i, a)] => {
                code.extend([Integer(depth(*i, component)), Index]);
                scale(&mut code, *a);
            }
            _ => {
                // Product of (1 - tint × alternate), then 1 minus the product.
                code.push(Real(1.0));
                for (i, a) in &terms {
                    code.extend([Integer(depth(*i, component + 1)), Index]);
                    scale(&mut code, *a);
                    code.extend([Real(1.0), Exch, Sub, Mul]);
                }
                code.extend([Real(1.0), Exch, Sub]);
            }
        }
    }
    // Move the four components below the tints, then drop the tints.
    code.extend([Integer(n as i32 + 4), Integer(4), Roll]);
    code.extend((0..n).map(|_| Pop));

    let encoded = PostScriptOp::encode(&code);
    sc.register_limits(encoded.limits());
    let encoded = FilterStreamBuilder::new_from_content_stream(&encoded, &sc.serialize_settings())
        .finish(&sc.serialize_settings());

    let tint_ref = sc.new_ref();
    let mut chunk = sc.new_chunk();
    let mut function = chunk.post_script_function(tint_ref, encoded.encoded_data());
    encoded.write_filters(function.deref_mut().deref_mut());
    function.domain([0.0, 1.0].repeat(n));
    function.range([0.0, 1.0].repeat(4));
    function.finish();
    chunk_container.streams.shading_functions.push(chunk);
    tint_ref
}
