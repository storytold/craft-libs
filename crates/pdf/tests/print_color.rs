//! Print color: the blending color space of transparency groups
//! (`SerializeSettings::group_color_space`), overprint (`Surface::push_overprint`), knockout
//! groups (`Surface::push_group`), soft-mask backdrop and transfer function, DeviceN colors,
//! images and shadings, and the print output intent with the PDF/X markers.
#![cfg(feature = "write")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use backend::blend::{GroupOptions, Overprint};
use backend::color::device_n::{self, Colorant, DeviceNSpace, Process};
use backend::color::{cmyk, luma};
use backend::configure::{Archival, PdfVersion, ValidationError};
use backend::geom::{PathBuilder, Rect, Size, Transform};
use backend::image::{BitsPerComponent, CustomImage, Image, ImageColorspace};
use backend::mask::{Mask, MaskType, TransferFunction};
use backend::num::NormalizedF32;
use backend::page::PageSettings;
use backend::paint::{Fill, LinearGradient, MeshGradient, MeshPatch, SpreadMethod, Stop, SweepGradient};
use backend::surface::Surface;
use backend::{GroupColorSpace, SerializeSettings, Trapped};
use craft_pdf::write::print::{self, PrintColor};
use craft_pdf::write::{Metadata, WriteError, Writer, backend};
use hayro_syntax::object::{Array, Dict, Name, Object, Stream};

const CMYK_PROFILE: &[u8] = include_bytes!("../../cms/profiles/photocraft-coated-cmyk.icc");

fn rect(surface: &mut Surface<'_>, x: f32, color: cmyk::Color, opacity: f32) {
    let mut path = PathBuilder::new();
    path.push_rect(Rect::from_xywh(x, 10.0, 40.0, 40.0).unwrap());
    surface.set_fill(Some(Fill { paint: color.into(), opacity: NormalizedF32::new(opacity).unwrap(), ..Default::default() }));
    surface.draw_path(&path.finish().unwrap());
}

fn k(value: u8) -> cmyk::Color {
    cmyk::Color::new(0, 0, 0, value)
}

/// Page 1: opacity group, isolated multiply group, soft mask, fill opacity and (with
/// `write-pdf`) a placed page. Page 2: opaque. Page 3: only the fill opacity of page 1, whose
/// graphics state is already written.
fn sample(settings: SerializeSettings, placed: Option<&[u8]>) -> Result<Vec<u8>, WriteError> {
    let mut writer = Writer::new(settings, Metadata::default());
    let page_settings = PageSettings::new(Size::from_wh(300.0, 60.0).unwrap());
    {
        let mut page = writer.document_mut().start_page_with(page_settings.clone());
        let mut surface = page.surface();
        rect(&mut surface, 0.0, cmyk::Color::new(255, 0, 0, 0), 1.0);
        surface.push_opacity(NormalizedF32::new(0.5).unwrap());
        rect(&mut surface, 20.0, k(255), 1.0);
        surface.pop();
        surface.push_isolated();
        surface.push_blend_mode(backend::blend::BlendMode::Multiply);
        rect(&mut surface, 60.0, cmyk::Color::new(0, 255, 0, 0), 1.0);
        surface.pop();
        surface.pop();
        let mut builder = surface.stream_builder();
        let mut mask_surface = builder.surface();
        let mut path = PathBuilder::new();
        path.push_rect(Rect::from_xywh(110.0, 10.0, 20.0, 40.0).unwrap());
        mask_surface.set_fill(Some(Fill { paint: luma::Color::white().into(), ..Default::default() }));
        mask_surface.draw_path(&path.finish().unwrap());
        mask_surface.finish();
        let stream = builder.finish();
        surface.push_mask(Mask::new(stream, MaskType::Luminosity));
        rect(&mut surface, 100.0, k(255), 1.0);
        surface.pop();
        rect(&mut surface, 150.0, k(255), 0.5);
        #[cfg(feature = "write-pdf")]
        if let Some(placed) = placed {
            let pdf = backend::pdf::Pdf::new(Arc::new(placed.to_vec())).unwrap();
            let doc = backend::pdf::PdfDocument::new(Arc::new(pdf));
            surface.draw_pdf_page(&doc, Size::from_wh(60.0, 12.0).unwrap(), 0);
        }
        #[cfg(not(feature = "write-pdf"))]
        let _ = placed;
        surface.finish();
        page.finish();
    }
    for page_index in [2, 3] {
        let mut page = writer.document_mut().start_page_with(page_settings.clone());
        let mut surface = page.surface();
        rect(&mut surface, 150.0, k(255), if page_index == 3 { 0.5 } else { 1.0 });
        surface.finish();
        page.finish();
    }
    writer.finish()
}

fn cmyk_settings() -> SerializeSettings {
    SerializeSettings { group_color_space: GroupColorSpace::Cmyk, ..Default::default() }
}

/// Every transparency group in the file, as (where, `/CS` family or `None`).
#[derive(Debug, Default)]
struct Groups {
    pages: Vec<Option<Option<String>>>,
    forms: Vec<Option<String>>,
    masks: Vec<Option<String>>,
}

fn groups(bytes: &[u8]) -> Groups {
    let pdf = hayro_syntax::Pdf::new(Arc::new(bytes.to_vec())).unwrap();
    let mut out = Groups::default();
    let mut seen = std::collections::HashSet::new();
    for page in pdf.pages().iter() {
        out.pages.push(page.raw().get::<Dict<'_>>(b"Group".as_slice()).map(|g| cs(&g)));
        visit(&page.resources().x_objects, &page.resources().ext_g_states, &mut out, &mut seen);
    }
    out
}

fn visit(x_objects: &Dict<'_>, states: &Dict<'_>, out: &mut Groups, seen: &mut std::collections::HashSet<String>) {
    let mut forms = Vec::new();
    for key in x_objects.keys() {
        if let Some(x) = x_objects.get::<Stream<'_>>(key.as_ref()) {
            forms.push((x, false));
        }
    }
    for key in states.keys() {
        if let Some(mask) = states.get::<Dict<'_>>(key.as_ref()).and_then(|s| s.get::<Dict<'_>>(b"SMask".as_slice()))
            && let Some(source) = mask.get::<Stream<'_>>(b"G".as_slice())
        {
            forms.push((source, true));
        }
    }
    for (x, is_mask) in forms {
        if !seen.insert(format!("{:?}", x.obj_id())) {
            continue;
        }
        let dict = x.dict();
        if let Some(group) = dict.get::<Dict<'_>>(b"Group".as_slice()) {
            if is_mask { out.masks.push(cs(&group)) } else { out.forms.push(cs(&group)) }
        }
        if let Some(res) = dict.get::<Dict<'_>>(b"Resources".as_slice()) {
            let sub = |key: &[u8]| res.get::<Dict<'_>>(key).unwrap_or_default();
            visit(&sub(b"XObject"), &sub(b"ExtGState"), out, seen);
        }
    }
}

fn cs(group: &Dict<'_>) -> Option<String> {
    assert_eq!(group.get::<Name<'_>>(b"S".as_slice()).unwrap().as_str(), "Transparency");
    group.get::<Object<'_>>(b"CS".as_slice()).map(|cs| match cs {
        Object::Name(name) => name.as_str().to_owned(),
        Object::Array(array) => {
            let array: Array<'_> = array;
            let n = array.iter::<Object<'_>>().nth(1).and_then(|o| match o {
                Object::Stream(s) => s.dict().get::<i32>(b"N".as_slice()),
                _ => None,
            });
            format!("ICCBased(N={})", n.unwrap_or(0))
        }
        _ => "?".to_owned(),
    })
}

/// A copy of the sample to place on page 1, written with the same settings.
fn placed(settings: &SerializeSettings) -> Option<Vec<u8>> {
    cfg!(feature = "write-pdf").then(|| sample(settings.clone(), None).unwrap())
}

/// Opacity, isolated and masked content groups; with `write-pdf` also the placed page's
/// group and the three inside it.
const FORMS: usize = if cfg!(feature = "write-pdf") { 7 } else { 3 };
const MASKS: usize = if cfg!(feature = "write-pdf") { 2 } else { 1 };

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn rgb_default_names_rgb_on_every_group_and_writes_no_page_group() {
    let settings = SerializeSettings::default();
    assert_eq!(settings.group_color_space, GroupColorSpace::Rgb);
    let groups = groups(&sample(settings.clone(), placed(&settings).as_deref()).unwrap());
    assert_eq!(groups.pages, vec![None, None, None]);
    assert_eq!(groups.forms, vec![Some("DeviceRGB".to_owned()); FORMS], "{groups:?}");
    assert_eq!(groups.masks, vec![Some("DeviceRGB".to_owned()); MASKS], "{groups:?}");
}

/// Content groups and the placed page write no `/CS`; pages that blend (also only through a
/// graphics state already written for page 1) get a DeviceCMYK page group; the soft-mask
/// group names DeviceCMYK; nothing mentions DeviceRGB.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn cmyk_groups_inherit_a_cmyk_page_group_on_pages_that_blend() {
    let settings = cmyk_settings();
    let bytes = sample(settings.clone(), placed(&settings).as_deref()).unwrap();
    let groups = groups(&bytes);
    let cmyk_group = Some(Some("DeviceCMYK".to_owned()));
    assert_eq!(groups.pages, vec![cmyk_group.clone(), None, cmyk_group], "{groups:?}");
    assert_eq!(groups.forms, vec![None; FORMS], "{groups:?}");
    assert_eq!(groups.masks, vec![Some("DeviceCMYK".to_owned()); MASKS], "{groups:?}");
    assert!(!String::from_utf8_lossy(&bytes).contains("DeviceRGB"));
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn cmyk_groups_use_the_cmyk_profile_without_device_spaces() {
    let settings =
        SerializeSettings { no_device_cs: true, cmyk_profile: Some(backend::icc::ICCProfile::<4>::new(CMYK_PROFILE).unwrap()), ..cmyk_settings() };
    let bytes = sample(settings.clone(), placed(&settings).as_deref()).unwrap();
    let groups = groups(&bytes);
    let icc = Some(Some("ICCBased(N=4)".to_owned()));
    assert_eq!(groups.pages, vec![icc.clone(), None, icc], "{groups:?}");
    assert_eq!(groups.forms, vec![None; FORMS], "{groups:?}");
    assert_eq!(groups.masks, vec![Some("ICCBased(N=1)".to_owned()); MASKS], "{groups:?}");
    assert!(!String::from_utf8_lossy(&bytes).contains("DeviceRGB"));
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn cmyk_groups_without_device_spaces_need_a_cmyk_profile() {
    let settings = SerializeSettings { no_device_cs: true, ..cmyk_settings() };
    let Err(WriteError::Serialization(error)) = sample(settings, None) else { panic!("expected an error") };
    assert_eq!(*error, backend::error::KrillaError::MissingGroupCmykProfile);
}

/// Each page graphics state as its overprint and alpha entries, sorted.
fn page_states(bytes: &[u8]) -> Vec<String> {
    let pdf = hayro_syntax::Pdf::new(Arc::new(bytes.to_vec())).unwrap();
    let page = pdf.pages().iter().next().unwrap();
    let states = &page.resources().ext_g_states;
    let mut out: Vec<String> = states
        .keys()
        .map(|key| {
            let state = states.get::<Dict<'_>>(key.as_ref()).unwrap();
            let mut entries = Vec::new();
            for name in ["op", "OP"] {
                if let Some(value) = state.get::<bool>(name.as_bytes()) {
                    entries.push(format!("{name}={value}"));
                }
            }
            if let Some(mode) = state.get::<i32>(b"OPM".as_slice()) {
                entries.push(format!("OPM={mode}"));
            }
            for name in ["ca", "CA"] {
                if let Some(value) = state.get::<f32>(name.as_bytes()) {
                    entries.push(format!("{name}={value}"));
                }
            }
            entries.join(" ")
        })
        .collect();
    out.sort();
    out
}

fn overprinted(settings: SerializeSettings, overprint: Overprint) -> Result<Vec<u8>, WriteError> {
    let mut writer = Writer::new(settings, Metadata::default());
    let mut page = writer.document_mut().start_page_with(PageSettings::new(Size::from_wh(300.0, 60.0).unwrap()));
    let mut surface = page.surface();
    surface.push_overprint(overprint);
    rect(&mut surface, 0.0, k(255), 1.0);
    surface.pop();
    surface.finish();
    page.finish();
    writer.finish()
}

/// Every push writes `/op`, `/OP` and `/OPM`; a fill opacity and an opacity group pushed
/// inside combine with it; an inner push replaces the outer one.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn overprint_writes_op_and_opm_and_combines_with_inner_state() {
    let mut writer = Writer::new(SerializeSettings::default(), Metadata::default());
    {
        let mut page = writer.document_mut().start_page_with(PageSettings::new(Size::from_wh(300.0, 60.0).unwrap()));
        let mut surface = page.surface();
        surface.push_overprint(Overprint { fill: true, stroke: false, nonzero: true });
        rect(&mut surface, 0.0, k(255), 1.0);
        rect(&mut surface, 50.0, k(255), 0.5);
        surface.push_opacity(NormalizedF32::new(0.25).unwrap());
        rect(&mut surface, 100.0, k(255), 1.0);
        surface.pop();
        surface.push_overprint(Overprint { stroke: true, ..Default::default() });
        rect(&mut surface, 150.0, k(255), 1.0);
        surface.pop();
        surface.pop();
        surface.finish();
        page.finish();
    }
    let bytes = writer.finish().unwrap();
    assert_eq!(
        page_states(&bytes),
        vec!["op=false OP=true OPM=0", "op=true OP=false OPM=1", "op=true OP=false OPM=1 ca=0.25 CA=0.25", "op=true OP=false OPM=1 ca=0.5",]
    );
}

/// PDF/A-2 and later forbid OPM 1 with overprint when CMYK is ICCBased, as it always is
/// under PDF/A; OPM 0 is allowed.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn overprint_mode_one_is_a_pdf_a_error() {
    let has_opm_error = |nonzero: bool| {
        let settings = SerializeSettings {
            configuration: craft_pdf::write::configuration(PdfVersion::Pdf17, Some(Archival::A2_B)).unwrap(),
            cmyk_profile: Some(backend::icc::ICCProfile::<4>::new(CMYK_PROFILE).unwrap()),
            ..Default::default()
        };
        let Err(WriteError::Serialization(error)) = overprinted(settings, Overprint { fill: true, stroke: false, nonzero }) else {
            panic!("expected validation errors (the metadata is missing)")
        };
        let backend::error::KrillaError::Validation(errors) = *error else { panic!("expected validation errors") };
        errors.iter().any(|(error, _)| matches!(error, ValidationError::OverprintModeIccCmyk(_)))
    };
    assert!(has_opm_error(true));
    assert!(!has_opm_error(false));
    assert!(overprinted(SerializeSettings::default(), Overprint { fill: true, stroke: true, nonzero: true }).is_ok());
}

/// Draws two overlapping rects in each of the nested groups (outermost first) and returns
/// the page `/Group` `/CS` and, for each form from the outside in, `I=… K=… CS=…`.
fn grouped(settings: SerializeSettings, nested: &[GroupOptions]) -> (Option<Option<String>>, Vec<String>) {
    let mut writer = Writer::new(settings, Metadata::default());
    {
        let mut page = writer.document_mut().start_page_with(PageSettings::new(Size::from_wh(300.0, 60.0).unwrap()));
        let mut surface = page.surface();
        for options in nested {
            surface.push_group(*options);
            rect(&mut surface, 0.0, cmyk::Color::new(255, 0, 0, 0), 1.0);
            rect(&mut surface, 20.0, k(255), 0.5);
        }
        for _ in nested {
            surface.pop();
        }
        surface.finish();
        page.finish();
    }
    let bytes = writer.finish().unwrap();
    let pdf = hayro_syntax::Pdf::new(Arc::new(bytes)).unwrap();
    let page = pdf.pages().iter().next().unwrap();
    let page_group = page.raw().get::<Dict<'_>>(b"Group".as_slice()).map(|g| cs(&g));
    let mut forms = Vec::new();
    let mut x_objects = page.resources().x_objects.clone();
    loop {
        let keys: Vec<_> = x_objects.keys().collect();
        let [key] = keys.as_slice() else { break };
        let form = x_objects.get::<Stream<'_>>(key.as_ref()).unwrap();
        let dict = form.dict();
        let flag = |group: &Dict<'_>, key: &[u8]| group.get::<bool>(key).map_or("-".to_owned(), |b| b.to_string());
        forms.push(match dict.get::<Dict<'_>>(b"Group".as_slice()) {
            Some(group) => format!("I={} K={} CS={}", flag(&group, b"I"), flag(&group, b"K"), cs(&group).unwrap_or("-".to_owned())),
            None => "no group".to_owned(),
        });
        x_objects = dict.get::<Dict<'_>>(b"Resources".as_slice()).and_then(|r| r.get::<Dict<'_>>(b"XObject".as_slice())).unwrap_or_default();
    }
    (page_group, forms)
}

const KNOCKOUT: GroupOptions = GroupOptions { isolated: false, knockout: true };
const ISOLATED: GroupOptions = GroupOptions { isolated: true, knockout: false };
const BOTH: GroupOptions = GroupOptions { isolated: true, knockout: true };

/// A knockout group that is not isolated is still a transparency group: `/K true`, no `/I`,
/// RGB `/CS` by default and no `/CS` under a CMYK page group.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn knockout_group_without_isolation_is_a_transparency_group() {
    assert_eq!(grouped(SerializeSettings::default(), &[KNOCKOUT]), (None, vec!["I=- K=true CS=DeviceRGB".to_owned()]));
    assert_eq!(grouped(cmyk_settings(), &[KNOCKOUT]), (Some(Some("DeviceCMYK".to_owned())), vec!["I=- K=true CS=-".to_owned()]));
}

/// `/I` and `/K` are set independently; `push_isolated` writes `/I` only.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn knockout_and_isolation_are_independent() {
    assert_eq!(grouped(cmyk_settings(), &[BOTH]).1, vec!["I=true K=true CS=-"]);
    assert_eq!(grouped(cmyk_settings(), &[ISOLATED]).1, vec!["I=true K=- CS=-"]);
    assert_eq!(grouped(cmyk_settings(), &[GroupOptions::default()]).1, vec!["no group"]);
}

/// Nested groups each keep their own attributes.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn nested_groups_keep_their_own_attributes() {
    assert_eq!(
        grouped(SerializeSettings::default(), &[KNOCKOUT, ISOLATED, BOTH]).1,
        vec!["I=- K=true CS=DeviceRGB", "I=true K=- CS=DeviceRGB", "I=true K=true CS=DeviceRGB"]
    );
}

/// One page with a luminosity mask (white backdrop, sampled transfer curve) and an alpha mask
/// (backdrop ignored, identity transfer).
fn masked(settings: SerializeSettings) -> Vec<u8> {
    let mut writer = Writer::new(settings, Metadata::default());
    let mut page = writer.document_mut().start_page_with(PageSettings::new(Size::from_wh(200.0, 60.0).unwrap()));
    let mut surface = page.surface();
    let curve: Vec<NormalizedF32> = [0.0, 0.2, 1.0].iter().map(|v| NormalizedF32::new(*v).unwrap()).collect();
    let masks = [(MaskType::Luminosity, TransferFunction::Sampled(curve), 0.0), (MaskType::Alpha, TransferFunction::Identity, 100.0)];
    for (kind, transfer, x) in masks {
        let mut builder = surface.stream_builder();
        let mut mask_surface = builder.surface();
        let mut path = PathBuilder::new();
        path.push_rect(Rect::from_xywh(x + 10.0, 10.0, 20.0, 40.0).unwrap());
        mask_surface.set_fill(Some(Fill { paint: luma::Color::black().into(), ..Default::default() }));
        mask_surface.draw_path(&path.finish().unwrap());
        mask_surface.finish();
        let stream = builder.finish();
        surface.push_mask(Mask::new(stream, kind).with_backdrop(luma::Color::white()).with_transfer(transfer));
        rect(&mut surface, x, k(255), 1.0);
        surface.pop();
    }
    surface.finish();
    page.finish();
    writer.finish().unwrap()
}

/// `(/S, /BC, /TR)` of each soft mask on page 1, `/TR` as `Identity` or `Type0 Size=n`.
fn soft_masks(bytes: &[u8]) -> Vec<(String, Option<Vec<f32>>, Option<String>)> {
    let pdf = hayro_syntax::Pdf::new(Arc::new(bytes.to_vec())).unwrap();
    let page = &pdf.pages()[0];
    let states = &page.resources().ext_g_states;
    let mut out = Vec::new();
    for key in states.keys() {
        let Some(mask) = states.get::<Dict<'_>>(key.as_ref()).and_then(|s| s.get::<Dict<'_>>(b"SMask".as_slice())) else { continue };
        let kind = mask.get::<Name<'_>>(b"S".as_slice()).unwrap().as_str().to_owned();
        let backdrop = mask.get::<Array<'_>>(b"BC".as_slice()).map(|a| a.iter::<f32>().collect());
        let transfer = mask.get::<Object<'_>>(b"TR".as_slice()).map(|tr| match tr {
            Object::Name(name) => name.as_str().to_owned(),
            Object::Stream(function) => {
                let dict = function.dict();
                let size: Vec<i32> = dict.get::<Array<'_>>(b"Size".as_slice()).unwrap().iter::<i32>().collect();
                assert_eq!(dict.get::<i32>(b"BitsPerSample".as_slice()), Some(16));
                assert_eq!(function.decoded().unwrap().len(), 2 * size[0] as usize);
                format!("Type{} Size={}", dict.get::<i32>(b"FunctionType".as_slice()).unwrap(), size[0])
            }
            _ => "?".to_owned(),
        });
        out.push((kind, backdrop, transfer));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// `/BC` has the components of the mask group's color space (RGB; under
/// `GroupColorSpace::Cmyk`, CMYK, or gray with `no_device_cs`); an alpha mask gets no `/BC`; `/TR` is a Type 0 function or
/// `/Identity`.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn soft_masks_write_backdrop_in_the_group_space_and_transfer() {
    let profile = || Some(backend::icc::ICCProfile::<4>::new(CMYK_PROFILE).unwrap());
    let cases = [
        (SerializeSettings::default(), vec![1.0, 1.0, 1.0]),
        (SerializeSettings { no_device_cs: true, ..Default::default() }, vec![1.0, 1.0, 1.0]),
        (cmyk_settings(), vec![0.0, 0.0, 0.0, 0.0]),
        (SerializeSettings { no_device_cs: true, cmyk_profile: profile(), ..cmyk_settings() }, vec![1.0]),
    ];
    for (settings, white) in cases {
        let masks = soft_masks(&masked(settings));
        assert_eq!(
            masks,
            vec![("Alpha".to_owned(), None, Some("Identity".to_owned())), ("Luminosity".to_owned(), Some(white), Some("Type0 Size=3".to_owned())),]
        );
    }
}

/// With an ICC CMYK page group, a luminosity mask hides the masked object wherever its content
/// paints nothing: without `/BC`, and with a black `/BC` given as gray or as K 100%. Through a
/// CMYK profile K 100% is not black, so a mask group in that space would leave the object
/// showing there.
#[cfg(feature = "raster")]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn icc_cmyk_luminosity_masks_hide_the_object_outside_their_content() {
    use craft_pdf::interpret::InterpreterSettings;
    use craft_pdf::raster::{self, Options, RenderCache};

    let settings =
        SerializeSettings { no_device_cs: true, cmyk_profile: Some(backend::icc::ICCProfile::<4>::new(CMYK_PROFILE).unwrap()), ..cmyk_settings() };
    let mut writer = Writer::new(settings, Metadata::default());
    let mut page = writer.document_mut().start_page_with(PageSettings::new(Size::from_wh(200.0, 60.0).unwrap()));
    let mut surface = page.surface();
    let backdrops: [Option<backend::color::RegularColor>; 3] = [None, Some(luma::Color::black().into()), Some(k(255).into())];
    for (i, backdrop) in backdrops.into_iter().enumerate() {
        let x = 60.0 * i as f32;
        let mut builder = surface.stream_builder();
        let mut mask_surface = builder.surface();
        let mut path = PathBuilder::new();
        path.push_rect(Rect::from_xywh(x + 10.0, 10.0, 20.0, 40.0).unwrap());
        mask_surface.set_fill(Some(Fill { paint: luma::Color::white().into(), ..Default::default() }));
        mask_surface.draw_path(&path.finish().unwrap());
        mask_surface.finish();
        let mut mask = Mask::new(builder.finish(), MaskType::Luminosity);
        if let Some(backdrop) = backdrop {
            mask = mask.with_backdrop(backdrop);
        }
        surface.push_mask(mask);
        rect(&mut surface, x, k(255), 1.0);
        surface.pop();
    }
    surface.finish();
    page.finish();
    let bytes = writer.finish().unwrap();

    let doc = craft_pdf::open_slice(&bytes, None).unwrap();
    let options = Options { background: [255; 4], ..Default::default() };
    let raster = raster::render(&doc.pages()[0], &RenderCache::new(), &InterpreterSettings::default(), &options).unwrap();
    assert_eq!((raster.width, raster.height), (200, 60));
    let page = raster.bytes();
    let red = |x: usize, y: usize| page[(y * 200 + x) * 4];
    for i in 0..3 {
        let x = 60 * i;
        assert!(red(x + 20, 30) < 128, "mask {i}: the object shows inside the mask content");
        assert_eq!((red(x + 5, 30), red(x + 35, 30)), (255, 255), "mask {i}: the object shows outside the mask content");
    }
}

// DeviceN colors (`color::device_n`).

fn spot(name: &str, alternate: cmyk::Color) -> Colorant {
    Colorant::Spot { name: name.to_owned(), alternate }
}

fn gold() -> cmyk::Color {
    cmyk::Color::new(0, 51, 255, 26)
}

/// One page with a fill in each color; returns the PDF or the writer's error.
fn device_n_sample(settings: SerializeSettings, colors: Vec<backend::color::Color>) -> Result<Vec<u8>, WriteError> {
    let mut writer = Writer::new(settings, Metadata::default());
    let mut page = writer.document_mut().start_page_with(PageSettings::new(Size::from_wh(300.0, 60.0).unwrap()));
    let mut surface = page.surface();
    for (i, color) in colors.into_iter().enumerate() {
        let mut path = PathBuilder::new();
        path.push_rect(Rect::from_xywh(10.0 + 50.0 * i as f32, 10.0, 40.0, 40.0).unwrap());
        surface.set_fill(Some(Fill { paint: color.into(), ..Default::default() }));
        surface.draw_path(&path.finish().unwrap());
    }
    surface.finish();
    page.finish();
    writer.finish()
}

fn device_n(colorants: Vec<Colorant>, tints: Vec<u8>) -> backend::color::Color {
    device_n::Color::new(tints, DeviceNSpace::new(colorants).unwrap()).unwrap().into()
}

/// The page's DeviceN color space arrays, as (names, alternate, tint transform, attributes).
fn device_n_spaces(bytes: &[u8], mut each: impl FnMut(Vec<String>, Object<'_>, Stream<'_>, Option<Dict<'_>>)) {
    let pdf = hayro_syntax::Pdf::new(Arc::new(bytes.to_vec())).unwrap();
    let page = pdf.pages().iter().next().unwrap();
    let spaces = &page.resources().color_spaces;
    for key in spaces.keys() {
        let array = spaces.get::<Array<'_>>(key.as_ref()).unwrap();
        let mut items = array.iter::<Object<'_>>();
        let Some(Object::Name(family)) = items.next() else { panic!("color space without a family") };
        if family.as_str() != "DeviceN" {
            continue;
        }
        let Some(Object::Array(names)) = items.next() else { panic!("DeviceN without names") };
        let names = names.iter::<Name<'_>>().map(|n| n.as_str().to_owned()).collect();
        let alternate = items.next().unwrap();
        let Some(Object::Stream(tint)) = items.next() else { panic!("DeviceN tint transform is not a stream") };
        assert_eq!(tint.dict().get::<i32>(b"FunctionType".as_slice()), Some(4));
        let attrs = items.next().map(|a| match a {
            Object::Dict(d) => d,
            _ => panic!("DeviceN attributes are not a dictionary"),
        });
        each(names, alternate, tint, attrs);
    }
}

fn family(cs: &Object<'_>) -> String {
    match cs {
        Object::Name(n) => n.as_str().to_owned(),
        Object::Array(a) => a.iter::<Name<'_>>().next().map(|n| n.as_str().to_owned()).unwrap_or_default(),
        _ => "?".to_owned(),
    }
}

/// The tint transform evaluated by hayro on the given tints.
#[cfg(feature = "interpret")]
fn evaluate(tint: &Stream<'_>, tints: Vec<f32>) -> Vec<f32> {
    let function = hayro_interpret::Function::new(&Object::Stream(tint.clone())).unwrap();
    function.eval(tints.into()).unwrap().iter().map(|v| (v * 100.0).round() / 100.0).collect()
}

/// [Cyan Black] needs no attributes; its tint transform passes the tints to C and K.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn device_n_process_fill_maps_tints_to_their_cmyk_components() {
    let color = device_n(vec![Colorant::Process(Process::Cyan), Colorant::Process(Process::Black)], vec![153, 102]);
    let bytes = device_n_sample(cmyk_settings(), vec![color]).unwrap();
    let mut found = 0;
    device_n_spaces(&bytes, |names, alternate, _tint, attrs| {
        found += 1;
        assert_eq!(names, ["Cyan", "Black"]);
        assert_eq!(family(&alternate), "DeviceCMYK");
        assert!(attrs.is_none());
        #[cfg(feature = "interpret")]
        assert_eq!(evaluate(&_tint, vec![0.6, 0.4]), [0.6, 0.0, 0.0, 0.4]);
    });
    assert_eq!(found, 1);
}

/// Spot and process mixed: an NChannel space whose `/Colorants` holds the spot's
/// Separation and whose `/Process` names the alternate space; with `no_device_cs` both use
/// the `cmyk_profile` ICC space.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn device_n_mixing_spots_and_process_writes_nchannel_attributes() {
    let icc =
        SerializeSettings { no_device_cs: true, cmyk_profile: Some(backend::icc::ICCProfile::<4>::new(CMYK_PROFILE).unwrap()), ..cmyk_settings() };
    for (settings, alternate_family) in [(cmyk_settings(), "DeviceCMYK"), (icc, "ICCBased")] {
        let color = device_n(vec![Colorant::Process(Process::Cyan), spot("Gold", gold())], vec![255, 128]);
        let bytes = device_n_sample(settings, vec![color]).unwrap();
        let mut found = 0;
        device_n_spaces(&bytes, |names, alternate, _tint, attrs| {
            found += 1;
            assert_eq!(names, ["Cyan", "Gold"]);
            assert_eq!(family(&alternate), alternate_family);
            let attrs = attrs.expect("attributes");
            assert_eq!(attrs.get::<Name<'_>>(b"Subtype".as_slice()).unwrap().as_str(), "NChannel");
            let colorants = attrs.get::<Dict<'_>>(b"Colorants".as_slice()).unwrap();
            assert_eq!(colorants.keys().count(), 1);
            let gold = colorants.get::<Array<'_>>(b"Gold".as_slice()).unwrap();
            let gold: Vec<Object<'_>> = gold.iter::<Object<'_>>().collect();
            assert_eq!(family(&gold[0]), "Separation");
            assert!(matches!(&gold[1], Object::Name(n) if n.as_str() == "Gold"));
            assert_eq!(family(&gold[2]), alternate_family);
            let process = attrs.get::<Dict<'_>>(b"Process".as_slice()).unwrap();
            assert_eq!(family(&process.get::<Object<'_>>(b"ColorSpace".as_slice()).unwrap()), alternate_family);
            let components: Vec<String> =
                process.get::<Array<'_>>(b"Components".as_slice()).unwrap().iter::<Name<'_>>().map(|n| n.as_str().to_owned()).collect();
            assert_eq!(components, ["Cyan", "Magenta", "Yellow", "Black"]);
            // Gold at 50% (M 0.1, Y 0.5, K 0.05) over full cyan.
            #[cfg(feature = "interpret")]
            assert_eq!(evaluate(&_tint, vec![1.0, 0.5]), [1.0, 0.1, 0.5, 0.05]);
        });
        assert_eq!(found, 1);
    }
}

/// NChannel's `/Subtype` and `/Process` are PDF 1.6: a 1.4 file (PDF/X-1a) gets only
/// `/Colorants`, a 1.7 file the NChannel attributes.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn device_n_attributes_follow_the_pdf_version() {
    for version in [PdfVersion::Pdf14, PdfVersion::Pdf17] {
        let settings = SerializeSettings {
            configuration: craft_pdf::write::configuration(version, None).unwrap(),
            compress_content_streams: false,
            ..cmyk_settings()
        };
        let color = device_n(vec![Colorant::Process(Process::Cyan), spot("Gold", gold())], vec![255, 128]);
        let bytes = device_n_sample(settings, vec![color]).unwrap();
        let nchannel = version >= PdfVersion::Pdf16;
        assert_eq!(bytes.windows(8).any(|w| w == b"NChannel"), nchannel, "{version:?}");
        let mut found = 0;
        device_n_spaces(&bytes, |_, _, _, attrs| {
            found += 1;
            let attrs = attrs.expect("attributes");
            assert!(attrs.get::<Dict<'_>>(b"Colorants".as_slice()).unwrap().get::<Array<'_>>(b"Gold".as_slice()).is_some());
            assert_eq!(attrs.contains_key(b"Subtype".as_slice()), nchannel, "{version:?}");
            assert_eq!(attrs.contains_key(b"Process".as_slice()), nchannel, "{version:?}");
        });
        assert_eq!(found, 1);
    }
}

/// A spot name keeps one alternate across Separation and DeviceN; PDF/A-2 rejects a second.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn device_n_spot_alternate_must_match_its_separation_fallback() {
    use backend::color::separation::{self, SeparationColorant, SeparationSpace};
    use backend::configure::{Archival, ConfigurationBuilder, ValidationError};
    let settings = SerializeSettings {
        configuration: ConfigurationBuilder::new().with_archival_validator(Archival::A2_B).finish().unwrap(),
        ..Default::default()
    };
    let separation = |alternate: cmyk::Color| -> backend::color::Color {
        separation::Color::new(255, SeparationSpace::new(SeparationColorant::Custom("Gold".to_owned()), alternate.into())).into()
    };
    let gold_device_n = || device_n(vec![spot("Gold", gold()), Colorant::Process(Process::Black)], vec![255, 255]);
    let inconsistent = |colors| match device_n_sample(settings.clone(), colors) {
        Err(WriteError::Serialization(error)) => match *error {
            backend::error::KrillaError::Validation(errors) => {
                errors.iter().any(|(e, _)| *e == ValidationError::InconsistentSeparationFallback(SeparationColorant::Custom("Gold".to_owned())))
            }
            other => panic!("unexpected error {other:?}"),
        },
        Err(other) => panic!("unexpected error {other:?}"),
        Ok(_) => false,
    };
    assert!(!inconsistent(vec![separation(gold()), gold_device_n()]));
    assert!(inconsistent(vec![separation(cmyk::Color::new(0, 40, 255, 0)), gold_device_n()]));
    assert!(inconsistent(vec![gold_device_n(), device_n(vec![spot("Gold", cmyk::Color::new(0, 0, 255, 0))], vec![255])]));
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn device_n_space_rejects_bad_colorant_lists() {
    assert!(DeviceNSpace::new([]).is_none());
    assert!(DeviceNSpace::new([Colorant::Process(Process::Cyan), Colorant::Process(Process::Cyan)]).is_none());
    assert!(DeviceNSpace::new([spot("Black", gold())]).is_none());
    assert!(DeviceNSpace::new([spot("All", gold())]).is_none());
    let space = DeviceNSpace::new([spot("Gold", gold())]).unwrap();
    assert!(device_n::Color::new(vec![1, 2], space).is_none());
}

// DeviceN images and shadings.

#[derive(Clone, Hash)]
struct Samples {
    data: Vec<u8>,
    size: (u32, u32),
    space: ImageColorspace,
    device_n: Option<DeviceNSpace>,
}

impl CustomImage for Samples {
    fn color_channel(&self) -> &[u8] {
        &self.data
    }
    fn alpha_channel(&self) -> Option<&[u8]> {
        None
    }
    fn bits_per_component(&self) -> BitsPerComponent {
        BitsPerComponent::Eight
    }
    fn size(&self) -> (u32, u32) {
        self.size
    }
    fn icc_profile(&self) -> Option<&[u8]> {
        None
    }
    fn color_space(&self) -> ImageColorspace {
        self.space
    }
    fn device_n_space(&self) -> Option<DeviceNSpace> {
        self.device_n.clone()
    }
}

/// One page drawn by `draw`, written with CMYK blending.
fn page_with(draw: impl FnOnce(&mut Surface<'_>)) -> Vec<u8> {
    let mut writer = Writer::new(cmyk_settings(), Metadata::default());
    let mut page = writer.document_mut().start_page_with(PageSettings::new(Size::from_wh(300.0, 60.0).unwrap()));
    let mut surface = page.surface();
    draw(&mut surface);
    surface.finish();
    page.finish();
    writer.finish().unwrap()
}

/// The names of a Separation (one) or DeviceN color space array; the family for others.
fn colorant_names(cs: &Object<'_>) -> (String, Vec<String>) {
    let Object::Array(array) = cs else { return (family(cs), Vec::new()) };
    let items: Vec<Object<'_>> = array.iter::<Object<'_>>().collect();
    let names = match items.get(1) {
        Some(Object::Name(n)) => vec![n.as_str().to_owned()],
        Some(Object::Array(names)) => names.iter::<Name<'_>>().map(|n| n.as_str().to_owned()).collect(),
        _ => Vec::new(),
    };
    (family(cs), names)
}

/// A [Black Gold] duotone: the image's `/ColorSpace` is the DeviceN space, with two 8-bit
/// samples a pixel in colorant order.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn device_n_image_writes_one_sample_per_colorant() {
    let space = DeviceNSpace::new([Colorant::Process(Process::Black), spot("Gold", gold())]).unwrap();
    let data = vec![0, 0, 128, 64, 255, 153];
    let image =
        Image::from_custom(Samples { data: data.clone(), size: (3, 1), space: ImageColorspace::DeviceN, device_n: Some(space) }, false).unwrap();
    let bytes = page_with(|surface| surface.draw_image(image, Size::from_wh(30.0, 10.0).unwrap()));
    let pdf = hayro_syntax::Pdf::new(Arc::new(bytes)).unwrap();
    let page = pdf.pages().iter().next().unwrap();
    let x_objects = &page.resources().x_objects;
    let mut found = 0;
    for key in x_objects.keys() {
        let image = x_objects.get::<Stream<'_>>(key.as_ref()).unwrap();
        let dict = image.dict();
        assert_eq!(dict.get::<Name<'_>>(b"Subtype".as_slice()).unwrap().as_str(), "Image");
        found += 1;
        let cs = dict.get::<Object<'_>>(b"ColorSpace".as_slice()).unwrap();
        assert_eq!(colorant_names(&cs), ("DeviceN".to_owned(), vec!["Black".to_owned(), "Gold".to_owned()]));
        assert_eq!(dict.get::<i32>(b"BitsPerComponent".as_slice()), Some(8));
        assert_eq!((dict.get::<i32>(b"Width".as_slice()), dict.get::<i32>(b"Height".as_slice())), (Some(3), Some(1)));
        assert_eq!(image.decoded().unwrap().as_ref(), data.as_slice());
    }
    assert_eq!(found, 1);
}

/// `Image::from_custom` refuses channel data that doesn't match the dimensions, and dimensions
/// whose channel length overflows, instead of panicking when the page is written.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn custom_image_with_mismatched_or_huge_dimensions_is_refused() {
    let cmyk = |data: Vec<u8>, size| Samples { data, size, space: ImageColorspace::Cmyk, device_n: None };
    assert!(Image::from_custom(cmyk(vec![0; 3], (1, 1)), false).is_err());
    assert!(Image::from_custom(cmyk(vec![0; 4], (u32::MAX, u32::MAX)), false).is_err());
    assert!(Image::from_custom(cmyk(vec![0; 4], (1, 1)), false).is_ok());
}

/// A DeviceN image needs its space from `CustomImage::device_n_space`.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn device_n_image_without_a_space_is_refused() {
    let image = Samples { data: vec![0; 6], size: (3, 1), space: ImageColorspace::DeviceN, device_n: None };
    assert!(Image::from_custom(image, false).is_err());
}

/// A shading's color space family and colorant names, function `/Range` length and
/// function `/Domain`.
type ShadingSummary = ((String, Vec<String>), usize, Vec<f32>);

/// Each shading on the page.
fn shadings(bytes: &[u8]) -> Vec<ShadingSummary> {
    let pdf = hayro_syntax::Pdf::new(Arc::new(bytes.to_vec())).unwrap();
    let page = pdf.pages().iter().next().unwrap();
    let patterns = &page.resources().patterns;
    let mut out = Vec::new();
    for key in patterns.keys() {
        let pattern = patterns.get::<Dict<'_>>(key.as_ref()).unwrap();
        let shading = pattern.get::<Dict<'_>>(b"Shading".as_slice()).unwrap();
        let cs = shading.get::<Object<'_>>(b"ColorSpace".as_slice()).unwrap();
        let function = match shading.get::<Object<'_>>(b"Function".as_slice()).unwrap() {
            Object::Dict(d) => d,
            Object::Stream(s) => s.dict().clone(),
            _ => panic!("shading function is not a dictionary or stream"),
        };
        let floats = |key: &[u8]| -> Vec<f32> { function.get::<Array<'_>>(key).unwrap().iter::<f32>().collect() };
        out.push((colorant_names(&cs), floats(b"Range").len(), floats(b"Domain")));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn stops(colors: [backend::color::Color; 2]) -> Vec<Stop> {
    let [a, b] = colors;
    vec![
        Stop { offset: NormalizedF32::ZERO, color: a, opacity: NormalizedF32::ONE },
        Stop { offset: NormalizedF32::ONE, color: b, opacity: NormalizedF32::ONE },
    ]
}

fn fill_rect(surface: &mut Surface<'_>, x: f32, paint: backend::paint::Paint) {
    let mut path = PathBuilder::new();
    path.push_rect(Rect::from_xywh(x, 10.0, 40.0, 40.0).unwrap());
    surface.set_fill(Some(Fill { paint, ..Default::default() }));
    surface.draw_path(&path.finish().unwrap());
}

fn linear(colors: [backend::color::Color; 2]) -> backend::paint::Paint {
    LinearGradient {
        x1: 0.0,
        y1: 0.0,
        x2: 40.0,
        y2: 0.0,
        transform: Transform::identity(),
        spread_method: SpreadMethod::Pad,
        stops: stops(colors),
        anti_alias: false,
    }
    .into()
}

/// Stops mixing a spot with process colors (linear: exponential function; sweep:
/// PostScript function) become DeviceN shadings with one function output per colorant; pure
/// CMYK stays DeviceCMYK.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn device_n_shading_mixes_a_spot_with_process_colors() {
    use backend::color::separation::{self, SeparationColorant, SeparationSpace};
    let gold_spot = || -> backend::color::Color {
        separation::Color::new(255, SeparationSpace::new(SeparationColorant::Custom("Gold".to_owned()), gold().into())).into()
    };
    let cyan: backend::color::Color = cmyk::Color::new(255, 0, 0, 0).into();
    let bytes = page_with(|surface| {
        fill_rect(surface, 0.0, linear([gold_spot(), cyan.clone()]));
        let sweep = SweepGradient {
            cx: 70.0,
            cy: 30.0,
            start_angle: 0.0,
            end_angle: 360.0,
            transform: Transform::identity(),
            spread_method: SpreadMethod::Pad,
            stops: stops([gold_spot(), luma::Color::black().into()]),
            anti_alias: false,
        };
        fill_rect(surface, 50.0, sweep.into());
        fill_rect(surface, 100.0, linear([cyan.clone(), k(255).into()]));
    });
    let names = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let found = shadings(&bytes);
    assert_eq!(found.len(), 3, "{found:?}");
    assert_eq!(found[0], (("DeviceCMYK".to_owned(), vec![]), 8, vec![0.0, 1.0]));
    assert_eq!(found[1].0, ("DeviceN".to_owned(), names(&["Black", "Gold"])));
    assert_eq!(found[1].1, 4, "sweep: one range pair per colorant");
    assert_eq!(found[1].2.len(), 4, "sweep: the function's domain is the bounding box");
    assert_eq!(found[2], (("DeviceN".to_owned(), names(&["Cyan", "Gold"])), 4, vec![0.0, 1.0]));
}

/// A spot to paper (CMYK 0 or a spot at tint 0) stays a Separation shading.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn device_n_shading_keeps_a_spot_to_paper_as_separation() {
    use backend::color::separation::{self, SeparationColorant, SeparationSpace};
    let gold_spot = |tint| -> backend::color::Color {
        separation::Color::new(tint, SeparationSpace::new(SeparationColorant::Custom("Gold".to_owned()), gold().into())).into()
    };
    let bytes = page_with(|surface| {
        fill_rect(surface, 0.0, linear([gold_spot(255), cmyk::Color::new(0, 0, 0, 0).into()]));
        fill_rect(surface, 50.0, linear([gold_spot(128), gold_spot(0)]));
    });
    let found = shadings(&bytes);
    assert_eq!(found.len(), 2, "{found:?}");
    for (cs, range, domain) in found {
        assert_eq!(cs, ("Separation".to_owned(), vec!["Gold".to_owned()]));
        assert_eq!((range, domain), (2, vec![0.0, 1.0]));
    }
}

// Mesh gradients (`paint::MeshGradient`, Type 6 shadings).

/// A square Coons patch with straight edges, corners clockwise from the top left at (x, y).
fn square(x: f32, y: f32, size: f32, colors: [backend::color::Color; 4], opacities: [f32; 4]) -> MeshPatch {
    let corners = [(x, y), (x + size, y), (x + size, y + size), (x, y + size)];
    let mut points = [(0.0, 0.0); 12];
    for (i, &(ax, ay)) in corners.iter().enumerate() {
        let (bx, by) = corners[(i + 1) % 4];
        for step in 0..3 {
            let t = step as f32 / 3.0;
            points[3 * i + step] = (ax + (bx - ax) * t, ay + (by - ay) * t);
        }
    }
    MeshPatch { points, colors, opacities: opacities.map(|o| NormalizedF32::new(o).unwrap()) }
}

fn mesh(patches: Vec<MeshPatch>) -> backend::paint::Paint {
    MeshGradient { patches, transform: Transform::identity(), anti_alias: false }.into()
}

/// Each Type 6 shading in the file: color space family and colorant names, components a
/// corner (from `/Decode`), and the corner color bytes of each patch.
type MeshSummary = ((String, Vec<String>), usize, Vec<Vec<u8>>);

fn mesh_shadings(bytes: &[u8]) -> Vec<MeshSummary> {
    let pdf = hayro_syntax::Pdf::new(Arc::new(bytes.to_vec())).unwrap();
    let mut out = Vec::new();
    for object in pdf.objects() {
        let Object::Stream(stream) = object else { continue };
        let dict = stream.dict();
        if dict.get::<i32>(b"ShadingType".as_slice()) != Some(6) {
            continue;
        }
        assert_eq!(dict.get::<i32>(b"BitsPerCoordinate".as_slice()), Some(32));
        assert_eq!(dict.get::<i32>(b"BitsPerComponent".as_slice()), Some(8));
        assert_eq!(dict.get::<i32>(b"BitsPerFlag".as_slice()), Some(8));
        let cs = dict.get::<Object<'_>>(b"ColorSpace".as_slice()).unwrap();
        let decode = dict.get::<Array<'_>>(b"Decode".as_slice()).unwrap().iter::<f32>().count();
        let components = (decode - 4) / 2;
        let data = stream.decoded().unwrap();
        let patch_len = 1 + 12 * 8 + 4 * components;
        assert_eq!(data.len() % patch_len, 0, "whole patches of {components} components a corner");
        let corners = data.chunks(patch_len).map(|patch| patch[1 + 12 * 8..].to_vec()).collect();
        out.push((colorant_names(&cs), components, corners));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn gold_spot(tint: u8) -> backend::color::Color {
    use backend::color::separation::{self, SeparationColorant, SeparationSpace};
    separation::Color::new(tint, SeparationSpace::new(SeparationColorant::Custom("Gold".to_owned()), gold().into())).into()
}

/// Mesh corner colors follow the gradient stop rule across every patch: CMYK, gray and a
/// spot become one DeviceN space with process colorants first, so a later patch's spot is
/// written as itself. Corner opacities below 1 add the same mesh in gray as a soft mask.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn mesh_mixing_process_and_spot_corners_is_a_device_n_shading() {
    let cyan = || -> backend::color::Color { cmyk::Color::new(255, 0, 0, 0).into() };
    let paper = || -> backend::color::Color { cmyk::Color::new(0, 0, 0, 0).into() };
    let patches = |opacity: f32| {
        vec![
            square(10.0, 10.0, 40.0, [cyan(), cyan(), luma::Color::black().into(), paper()], [1.0; 4]),
            square(50.0, 10.0, 40.0, [gold_spot(255), gold_spot(128), cyan(), cyan()], [1.0, 1.0, 1.0, opacity]),
        ]
    };
    let names = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let expected = vec![vec![255, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 0], vec![0, 0, 255, 0, 0, 128, 255, 0, 0, 255, 0, 0]];

    let opaque = page_with(|surface| fill_rect(surface, 0.0, mesh(patches(1.0))));
    assert_eq!(mesh_shadings(&opaque), vec![(("DeviceN".to_owned(), names(&["Cyan", "Black", "Gold"])), 3, expected.clone())]);

    let translucent = page_with(|surface| fill_rect(surface, 0.0, mesh(patches(0.5))));
    let found = mesh_shadings(&translucent);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(found[0], (("DeviceGray".to_owned(), vec![]), 1, vec![vec![255; 4], vec![255, 255, 255, 128]]));
    assert_eq!(found[1], (("DeviceN".to_owned(), names(&["Cyan", "Black", "Gold"])), 3, expected));
    assert_eq!(soft_masks(&translucent).len(), 1);
}

/// RGB corners can't share a space with CMYK ones: the document fails instead of writing
/// one patch's colors in the other's space.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn mesh_mixing_rgb_and_cmyk_corners_is_refused() {
    let red: backend::color::Color = backend::color::rgb::Color::new(255, 0, 0).into();
    let cyan: backend::color::Color = cmyk::Color::new(255, 0, 0, 0).into();
    let patches = vec![
        square(10.0, 10.0, 40.0, [red.clone(), red.clone(), red.clone(), red.clone()], [1.0; 4]),
        square(50.0, 10.0, 40.0, [cyan.clone(), cyan.clone(), red.clone(), red], [1.0; 4]),
    ];
    let mut writer = Writer::new(cmyk_settings(), Metadata::default());
    let mut page = writer.document_mut().start_page_with(PageSettings::new(Size::from_wh(300.0, 60.0).unwrap()));
    let mut surface = page.surface();
    fill_rect(&mut surface, 0.0, mesh(patches));
    surface.finish();
    page.finish();
    let Err(WriteError::Serialization(error)) = writer.finish() else { panic!("a mixed RGB and CMYK mesh was written") };
    assert!(matches!(*error, backend::error::KrillaError::MixedMeshColorSpaces(_)), "{error:?}");
}

/// A mesh in one color space is written in that space, three bytes a corner for RGB.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn rgb_mesh_is_a_device_rgb_shading() {
    let rgb = |r, g, b| -> backend::color::Color { backend::color::rgb::Color::new(r, g, b).into() };
    let patch = square(10.0, 10.0, 40.0, [rgb(255, 0, 0), rgb(0, 255, 0), rgb(0, 0, 255), rgb(255, 255, 255)], [1.0; 4]);
    let bytes = page_with(|surface| fill_rect(surface, 0.0, mesh(vec![patch])));
    let corners = vec![vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]];
    assert_eq!(mesh_shadings(&bytes), vec![(("DeviceRGB".to_owned(), vec![]), 3, corners)]);
}

const FOGRA39: &[u8] = include_bytes!("../../cms/profiles/craft-coated-fogra39.icc");

fn text(dict: &Dict<'_>, key: &[u8]) -> Option<String> {
    dict.get::<hayro_syntax::object::String<'_>>(key).map(|s| String::from_utf8_lossy(s.as_bytes()).into_owned())
}

/// One `/OutputIntents` entry.
#[derive(Debug)]
struct Intent {
    subtype: String,
    identifier: Option<String>,
    registry: Option<String>,
    /// The `/DestOutputProfile` object and its decoded bytes.
    profile_object: String,
    profile: Vec<u8>,
}

fn output_intents(bytes: &[u8]) -> Vec<Intent> {
    let pdf = hayro_syntax::Pdf::new(Arc::new(bytes.to_vec())).unwrap();
    let catalog = pdf.xref().get::<Dict<'_>>(pdf.xref().root_id()).unwrap();
    let Some(intents) = catalog.get::<Array<'_>>(b"OutputIntents".as_slice()) else { return Vec::new() };
    intents
        .iter::<Dict<'_>>()
        .map(|intent| {
            let profile = intent.get::<Stream<'_>>(b"DestOutputProfile".as_slice()).unwrap();
            Intent {
                subtype: intent.get::<Name<'_>>(b"S".as_slice()).unwrap().as_str().to_owned(),
                identifier: text(&intent, b"OutputConditionIdentifier"),
                registry: text(&intent, b"RegistryName"),
                profile_object: format!("{:?}", profile.obj_id()),
                profile: profile.decoded().unwrap().into_owned(),
            }
        })
        .collect()
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn print_settings_blend_in_cmyk_and_name_the_profile_as_the_output_intent() {
    let settings = print::settings(PrintColor::fogra39(FOGRA39)).unwrap();
    assert!(!settings.no_device_cs);
    assert_eq!(settings.group_color_space, GroupColorSpace::Cmyk);
    assert_eq!(settings.trapped, Some(Trapped::False));
    assert_eq!(settings.pdfx_version, None);
    let intent = settings.output_intent.as_ref().unwrap();
    assert_eq!((intent.condition_identifier.as_str(), intent.registry_name.as_deref()), ("FOGRA39", Some("http://www.color.org")));

    let bytes = sample(settings.clone(), placed(&settings).as_deref()).unwrap();
    let cmyk_group = Some(Some("DeviceCMYK".to_owned()));
    assert_eq!(groups(&bytes).pages, vec![cmyk_group.clone(), None, cmyk_group]);
    let intents = output_intents(&bytes);
    assert_eq!(intents.len(), 1);
    let intent = &intents[0];
    assert_eq!(
        (intent.subtype.as_str(), intent.identifier.as_deref(), intent.registry.as_deref()),
        ("GTS_PDFX", Some("FOGRA39"), Some("http://www.color.org"))
    );
    assert!(intent.profile == FOGRA39, "the output intent embeds the given profile");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/Trapped/False") && text.contains("<pdf:Trapped>False</pdf:Trapped>"));
    assert!(!text.contains("GTS_PDFXVersion"));
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn cmyk_blending_writes_no_output_intent_or_trapped() {
    let settings = print::cmyk_blending(FOGRA39).unwrap();
    assert_eq!(settings.group_color_space, GroupColorSpace::Cmyk);
    assert!(settings.output_intent.is_none() && settings.trapped.is_none());

    let bytes = sample(settings.clone(), placed(&settings).as_deref()).unwrap();
    let cmyk_group = Some(Some("DeviceCMYK".to_owned()));
    assert_eq!(groups(&bytes).pages, vec![cmyk_group.clone(), None, cmyk_group]);
    assert!(output_intents(&bytes).is_empty());
    assert!(!String::from_utf8_lossy(&bytes).contains("Trapped"));
    assert!(matches!(print::cmyk_blending(b"not a profile"), Err(WriteError::InvalidProfile)));
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn print_settings_reject_a_profile_that_is_not_cmyk() {
    assert!(matches!(print::settings(PrintColor::fogra39(b"not a profile")), Err(WriteError::InvalidProfile)));
}

/// `/Trapped` (`Unknown` included) and `GTS_PDFXVersion` go to the information dictionary and
/// to XMP, `GTS_PDFXConformance` to the dictionary only; without settings none appears.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn trapped_and_pdfx_version_are_written_to_info_and_xmp() {
    let settings = SerializeSettings {
        trapped: Some(Trapped::True),
        pdfx_version: Some("PDF/X-1:2003".to_owned()),
        pdfx_conformance: Some("PDF/X-1a:2003".to_owned()),
        ..Default::default()
    };
    let marked = String::from_utf8_lossy(&sample(settings, None).unwrap()).into_owned();
    assert!(marked.contains("/Trapped/True"), "Info /Trapped");
    assert!(marked.contains("/GTS_PDFXVersion(PDF/X-1:2003)"), "Info /GTS_PDFXVersion");
    assert!(marked.contains("/GTS_PDFXConformance(PDF/X-1a:2003)"), "Info /GTS_PDFXConformance");
    assert!(!marked.contains("pdfxid:GTS_PDFXConformance"), "no XMP conformance property");
    assert!(marked.contains("<pdf:Trapped>True</pdf:Trapped>"), "XMP pdf:Trapped");
    assert!(marked.contains("<pdfxid:GTS_PDFXVersion>PDF/X-1:2003</pdfxid:GTS_PDFXVersion>"), "XMP GTS_PDFXVersion");

    let unknown = SerializeSettings { trapped: Some(Trapped::Unknown), ..Default::default() };
    let unknown = String::from_utf8_lossy(&sample(unknown, None).unwrap()).into_owned();
    assert!(unknown.contains("/Trapped/Unknown") && unknown.contains("<pdf:Trapped>Unknown</pdf:Trapped>"));

    let plain = String::from_utf8_lossy(&sample(SerializeSettings::default(), None).unwrap()).into_owned();
    assert!(!plain.contains("Trapped") && !plain.contains("GTS_PDFX"));
}

/// A one-page PDF/A document with FOGRA39 print settings and the given PDF/X version.
fn pdfa_print(version: PdfVersion, archival: Archival, pdfx_version: Option<&str>) -> Result<Vec<u8>, WriteError> {
    let settings = SerializeSettings {
        no_device_cs: true,
        configuration: craft_pdf::write::configuration(version, Some(archival)).unwrap(),
        pdfx_version: pdfx_version.map(str::to_owned),
        ..print::settings(PrintColor::fogra39(FOGRA39)).unwrap()
    };
    let created = Some(backend::metadata::DateTime::new(2026));
    let metadata = Metadata { title: Some("Print".to_owned()), created, ..Metadata::default() };
    let mut writer = Writer::new_with_language(settings, metadata, Some("en".to_owned()));
    {
        let mut page = writer.document_mut().start_page_with(PageSettings::new(Size::from_wh(60.0, 60.0).unwrap()));
        let mut surface = page.surface();
        rect(&mut surface, 0.0, k(255), 0.5);
        surface.finish();
        page.finish();
    }
    writer.finish()
}

/// Under PDF/A the archival intent names the same profile object as the print intent.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn pdfa_and_print_intents_share_the_profile() {
    let intents = output_intents(&pdfa_print(PdfVersion::Pdf17, Archival::A2_B, None).unwrap());
    let subtypes: Vec<_> = intents.iter().map(|i| i.subtype.as_str()).collect();
    assert_eq!(subtypes, ["GTS_PDFA1", "GTS_PDFX"]);
    assert_eq!(intents[0].profile_object, intents[1].profile_object);
    assert!(intents[0].profile == FOGRA39);
}

/// `pdfxid:GTS_PDFXVersion` needs an XMP extension schema under PDF/A-1 to 3, which krilla
/// doesn't write, so a PDF/X version fails validation there; PDF/A-4 needs none.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn pdfx_version_is_a_pdf_a_1_to_3_error() {
    for (version, archival) in [(PdfVersion::Pdf14, Archival::A1_B), (PdfVersion::Pdf17, Archival::A2_B), (PdfVersion::Pdf17, Archival::A3_B)] {
        let Err(WriteError::Serialization(error)) = pdfa_print(version, archival, Some("PDF/X-4")) else {
            panic!("{archival:?}: expected a validation error")
        };
        let backend::error::KrillaError::Validation(errors) = *error else { panic!("{archival:?}: expected validation errors") };
        assert!(errors.iter().any(|(error, _)| *error == ValidationError::PdfxVersionWithoutExtensionSchema), "{archival:?}: {errors:?}");
    }
    let bytes = pdfa_print(PdfVersion::Pdf20, Archival::A4, Some("PDF/X-4")).unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("<pdfxid:GTS_PDFXVersion>PDF/X-4</pdfxid:GTS_PDFXVersion>"));
}
