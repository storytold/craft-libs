//! The render tests of LightCraft's `crs:` import (`lightcraft-engine` `crs_masks/tests.rs`) whose mapping halves are
//! in `craft_develop_model::crs_masks`: they need the pipeline to check what a mask or a spot does to the picture.

use craft_develop_model::DevelopSettings;
use serde_json::json;

/// A packet in the shape local corrections take (values written for this test).
const PACKET: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Exposure2012="0.2">
 <crs:MaskGroupBasedCorrections><rdf:Seq>
  <rdf:li><rdf:Description crs:What="Correction" crs:CorrectionAmount="1" crs:CorrectionActive="true" crs:CorrectionName="Darker sky"
     crs:LocalExposure2012="-0.25" crs:LocalSaturation="0.3" crs:LocalTemperature="-0.1">
   <crs:CorrectionMasks><rdf:Seq>
    <rdf:li crs:What="Mask/Image" crs:MaskSubType="2" crs:MaskBlendMode="0" crs:MaskInverted="false"/>
    <rdf:li crs:What="Mask/Gradient" crs:MaskBlendMode="2" crs:FullX="0.5" crs:FullY="0" crs:ZeroX="0.5" crs:ZeroY="0.6"/>
   </rdf:Seq></crs:CorrectionMasks>
  </rdf:Description></rdf:li>
  <rdf:li><rdf:Description crs:What="Correction" crs:CorrectionAmount="0.5" crs:LocalContrast2012="0.2">
   <crs:CorrectionMasks><rdf:Seq>
    <rdf:li crs:What="Mask/CircularGradient" crs:Top="0.3" crs:Left="0.2" crs:Bottom="0.7" crs:Right="0.8" crs:Angle="10" crs:Feather="40" crs:Flipped="true" crs:Version="2"/>
    <rdf:li crs:What="Mask/RangeMask" crs:MaskBlendMode="2"><crs:CorrectionRangeMask crs:Type="2" crs:LumRange="0.4 0.5 0.8 0.9"/></rdf:li>
    <rdf:li crs:What="Mask/Image" crs:MaskSubType="9"/>
   </rdf:Seq></crs:CorrectionMasks>
  </rdf:Description></rdf:li>
  <rdf:li><rdf:Description crs:What="Correction" crs:CorrectionActive="false" crs:LocalExposure2012="1">
   <crs:CorrectionMasks><rdf:Seq><rdf:li crs:What="Mask/Image" crs:MaskSubType="1"/></rdf:Seq></crs:CorrectionMasks>
  </rdf:Description></rdf:li>
 </rdf:Seq></crs:MaskGroupBasedCorrections>
 <crs:PaintBasedCorrections><rdf:Seq>
  <rdf:li crs:What="Correction" crs:LocalClarity2012="0.4"><crs:CorrectionMasks><rdf:Seq>
   <rdf:li crs:What="Mask/Paint" crs:Radius="0.05" crs:Flow="0.8" crs:CenterWeight="0.3" crs:MaskValue="1">
    <crs:Dabs><rdf:Seq><rdf:li>d 0.1 0.2</rdf:li><rdf:li>d 0.15 0.22</rdf:li></rdf:Seq></crs:Dabs></rdf:li>
   <rdf:li crs:What="Mask/Paint" crs:Radius="0.02" crs:MaskValue="1"><crs:Dabs><rdf:Seq><rdf:li>d 0.5 0.5</rdf:li></rdf:Seq></crs:Dabs></rdf:li>
  </rdf:Seq></crs:CorrectionMasks></rdf:li>
 </rdf:Seq></crs:PaintBasedCorrections>
</rdf:Description></rdf:RDF></x:xmpmeta>"#;

/// Spot removal in the shape Lightroom Classic writes it (values written for this test):
/// a healed circle with an explicit source, a cloned circle with an automatic source, a
/// brushed removal, a circular-gradient spot, one without a mask, and the legacy list.
const SPOTS: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Exposure2012="0.1">
 <crs:RetouchInfo><rdf:Seq>
  <rdf:li>centerX = 0.4, centerY = 0.3, radius = 0.01, sourceState = sourceSetExplicitly, sourceX = 0.45, sourceY = 0.3, spotType = heal</rdf:li>
 </rdf:Seq></crs:RetouchInfo>
 <crs:RetouchAreas><rdf:Seq>
  <rdf:li><rdf:Description crs:SpotType="heal" crs:SourceState="sourceSetExplicitly" crs:Method="poisson" crs:SourceX="0.45" crs:OffsetY="0.3" crs:Opacity="1" crs:Feather="0" crs:Seed="1">
   <crs:Masks><rdf:Seq><rdf:li crs:What="Mask/Ellipse" crs:MaskActive="true" crs:MaskValue="1" crs:X="0.4" crs:Y="0.3" crs:SizeX="0.01" crs:SizeY="0.015"/></rdf:Seq></crs:Masks>
  </rdf:Description></rdf:li>
  <rdf:li><rdf:Description crs:SpotType="clone" crs:SourceState="sourceAutoComputed" crs:Opacity="0.5" crs:Feather="0.25">
   <crs:Masks><rdf:Seq><rdf:li crs:What="Mask/Ellipse" crs:X="0.7" crs:Y="0.6" crs:SizeX="0.02" crs:SizeY="0.02"/></rdf:Seq></crs:Masks>
  </rdf:Description></rdf:li>
  <rdf:li><rdf:Description crs:SpotType="heal" crs:SourceState="sourceSetExplicitly" crs:SourceX="0.2" crs:OffsetY="0.9">
   <crs:Masks><rdf:Seq><rdf:li crs:What="Mask/Paint" crs:Radius="0.03" crs:MaskValue="1">
    <crs:Dabs><rdf:Seq><rdf:li>d 0.1 0.8</rdf:li><rdf:li>d 0.12 0.82</rdf:li><rdf:li>d 0.14 0.84</rdf:li></rdf:Seq></crs:Dabs></rdf:li></rdf:Seq></crs:Masks>
  </rdf:Description></rdf:li>
  <rdf:li><rdf:Description crs:SpotType="heal" crs:SourceX="0.6" crs:OffsetY="0.5">
   <crs:Masks><rdf:Seq><rdf:li crs:What="Mask/CircularGradient" crs:Top="0.48" crs:Left="0.49" crs:Bottom="0.52" crs:Right="0.51"/></rdf:Seq></crs:Masks>
  </rdf:Description></rdf:li>
  <rdf:li><rdf:Description crs:SpotType="heal">
   <crs:Masks><rdf:Seq><rdf:li crs:What="Mask/Gradient" crs:FullX="0" crs:FullY="0" crs:ZeroX="1" crs:ZeroY="1"/></rdf:Seq></crs:Masks>
  </rdf:Description></rdf:li>
 </rdf:Seq></crs:RetouchAreas>
</rdf:Description></rdf:RDF></x:xmpmeta>"#;

fn settings() -> (DevelopSettings, Vec<String>) {
    let d = craft_metadata::parse_xmp(PACKET).unwrap();
    let (partial, unmapped) = craft_develop_model::crs::to_partial_report(&d.properties, Some(&d.values), craft_develop_model::crs::Target::Any, 1.5);
    (DevelopSettings::default().merged(&partial).expect("valid settings"), unmapped)
}

#[test]
fn masks_render() {
    let (s, _) = settings();
    let img = crate::scenes::demo_library()[0].render(96, 64);
    let render = |d: &DevelopSettings| crate::render(&img, &Default::default(), d, &crate::RenderRequest::fit(96, 64)).image.data;
    assert_ne!(render(&DevelopSettings { light: s.light, ..Default::default() }), render(&s), "the masks change the picture");
}

#[test]
fn spots_render() {
    let d = craft_metadata::parse_xmp(SPOTS).unwrap();
    let (partial, _) = craft_develop_model::crs::to_partial_report(&d.properties, Some(&d.values), craft_develop_model::crs::Target::Any, 1.5);
    let s = DevelopSettings::default().merged(&partial).unwrap();
    let img = crate::scenes::demo_library()[0].render(96, 64);
    let render = |d: &DevelopSettings| crate::render(&img, &Default::default(), d, &crate::RenderRequest::fit(96, 64)).image.data;
    assert_ne!(render(&DevelopSettings { light: s.light, ..Default::default() }), render(&s), "the spots change the picture");
}

#[test]
fn sidecar_radials_fit_the_photo() {
    let mut v = json!({"masks": [{"components": [{"shape": {"kind": "radial", "rx": 0.3, "ry": 0.2}}]}]});
    craft_develop_model::crs_masks::refit_radials(&mut v, 1.5, 1.0);
    let s = &v["masks"][0]["components"][0]["shape"];
    assert!((s["rx"].as_f64().unwrap() - 0.3).abs() < 1e-9);
    assert!((s["ry"].as_f64().unwrap() - 0.3).abs() < 1e-9, "square photo: y radius = its box half-height");
}

/// One radial correction in `container`, with `attrs` on its mask component (values written
/// for this test; the component attributes follow what Lightroom Classic writes).
fn radial_packet(container: &str, attrs: &str) -> String {
    format!(
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Exposure2012="0">
 <crs:{container}><rdf:Seq>
  <rdf:li><rdf:Description crs:What="Correction" crs:CorrectionAmount="1" crs:LocalExposure2012="0.25">
   <crs:CorrectionMasks><rdf:Seq>
    <rdf:li crs:What="Mask/CircularGradient" crs:MaskValue="1" crs:Top="0.25" crs:Left="0.25" crs:Bottom="0.75" crs:Right="0.75" crs:Angle="0" crs:Midpoint="50" crs:Roundness="0" {attrs}/>
   </rdf:Seq></crs:CorrectionMasks>
  </rdf:Description></rdf:li>
 </rdf:Seq></crs:{container}>
</rdf:Description></rdf:RDF></x:xmpmeta>"#
    )
}

fn radial_settings(container: &str, attrs: &str) -> DevelopSettings {
    let d = craft_metadata::parse_xmp(&radial_packet(container, attrs)).unwrap();
    let (partial, unmapped) = craft_develop_model::crs::to_partial_report(&d.properties, Some(&d.values), craft_develop_model::crs::Target::Any, 1.5);
    assert!(unmapped.is_empty(), "{unmapped:?}");
    let s = DevelopSettings::default().merged(&partial).unwrap();
    assert_eq!(s.masks.len(), 1, "{:?}", s.masks);
    s
}

/// How much brighter the mask makes the centre and a corner of a flat grey frame (8-bit green).
fn lift_inside_outside(s: &DevelopSettings) -> (i32, i32) {
    let img = crate::raster::Image { width: 90, height: 60, data: vec![[0.18f32; 3]; 90 * 60] };
    let render = |d: &DevelopSettings| crate::render(&img, &Default::default(), d, &crate::RenderRequest::fit(90, 60)).image;
    let (with, without) = (render(s), render(&DevelopSettings { light: s.light, ..Default::default() }));
    let g = |im: &crate::raster::Rgba8, x: usize, y: usize| i32::from(im.data[y * im.width + x][1]);
    (g(&with, 45, 30) - g(&without, 45, 30), g(&with, 3, 3) - g(&without, 3, 3))
}

/// The render halves of the radial tests of `craft_develop_model::crs_masks` (the mapping halves are there): an
/// ordinary Lightroom radial lifts the inside of the circle, `MaskInverted` the outside.
#[test]
fn lightroom_radial_flipped_is_inside() {
    // as Lightroom Classic writes an ordinary radial mask today (storytold/lightcraft#1815)
    let s = radial_settings("MaskGroupBasedCorrections", r#"crs:MaskInverted="false" crs:Feather="25" crs:Flipped="true" crs:Version="2""#);
    let (inside, outside) = lift_inside_outside(&s);
    assert!(inside > 20 && outside == 0, "the effect is inside the circle: {inside} {outside}");
}

#[test]
fn lightroom_radial_mask_inverted_is_outside() {
    let s = radial_settings("MaskGroupBasedCorrections", r#"crs:MaskInverted="true" crs:Feather="25" crs:Flipped="true" crs:Version="2""#);
    let (inside, outside) = lift_inside_outside(&s);
    assert!(inside == 0 && outside > 20, "the effect is outside the circle: {inside} {outside}");
}

#[test]
fn versionless_group_radial_is_inside_with_half_the_feather() {
    // Lightroom renders both inside and rewrites them Flipped="true", Feather halved, Version="2"
    for flipped in ["true", "false"] {
        let s = radial_settings("MaskGroupBasedCorrections", &format!(r#"crs:MaskInverted="false" crs:Feather="50" crs:Flipped="{flipped}""#));
        let (inside, outside) = lift_inside_outside(&s);
        assert!(inside > 20 && outside == 0, "Flipped={flipped}: {inside} {outside}");
    }
}

#[test]
fn legacy_radial_filter_is_outside_unless_flipped() {
    // the process-2012 Radial Filter: effect outside by default, "Invert Mask" = Flipped="true"
    let s = radial_settings("CircularGradientBasedCorrections", r#"crs:Feather="50" crs:Flipped="false""#);
    let (inside, outside) = lift_inside_outside(&s);
    assert!(inside == 0 && outside > 20, "default: outside: {inside} {outside}");
    let s = radial_settings("CircularGradientBasedCorrections", r#"crs:Feather="50" crs:Flipped="true""#);
    let (inside, outside) = lift_inside_outside(&s);
    assert!(inside > 20 && outside == 0, "inverted: inside: {inside} {outside}");
}
