use super::*;
use crate::{DevelopSettings, MaskOp, MaskShape};

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

fn settings() -> (DevelopSettings, Vec<String>) {
    let d = craft_metadata::parse_xmp(PACKET).unwrap();
    let (partial, unmapped) = crate::crs::to_partial_report(&d.properties, Some(&d.values), crate::crs::Target::Any, 1.5);
    (DevelopSettings::default().merged(&partial).expect("valid settings"), unmapped)
}

#[test]
fn corrections_become_masks() {
    let (s, unmapped) = settings();
    assert_eq!(s.light.exposure, 0.2, "global settings still map");
    assert_eq!(s.masks.len(), 3, "inactive corrections are skipped: {:?}", s.masks);
    let sky = &s.masks[0];
    assert_eq!(sky.name, "Darker sky");
    assert_eq!((sky.adjust.exposure, sky.adjust.saturation, sky.adjust.temp, sky.adjust.amount), (-1.0, 30.0, -10.0, 100.0));
    assert_eq!(sky.components[0].shape, MaskShape::Sky);
    assert_eq!(sky.components[1].op, MaskOp::Intersect);
    let MaskShape::Linear { start, end } = &sky.components[1].shape else { panic!() };
    assert_eq!((start.y, end.y), (0.0, 0.6), "full effect at the Full point");
    let rad = &s.masks[1];
    assert_eq!((rad.name.as_str(), rad.adjust.contrast, rad.adjust.amount), ("Mask 2", 20.0, 50.0));
    let MaskShape::Radial { center, rx, ry, angle, feather, invert } = &rad.components[0].shape else { panic!() };
    assert!((center.x - 0.5).abs() < 1e-9 && (center.y - 0.5).abs() < 1e-9);
    assert!((rx - 0.3).abs() < 1e-9 && (ry - 0.2 / 1.5).abs() < 1e-9, "radii in long-edge units: {rx} {ry}");
    assert_eq!((*angle, *feather, *invert), (10.0, 40.0, false), "Flipped=\"true\" is Lightroom's ordinary radial: effect inside");
    assert!(!rad.components[0].invert);
    let MaskShape::LuminanceRange { lo, hi, lo_feather, .. } = &rad.components[1].shape else { panic!() };
    assert!(lo < hi && *lo_feather > 0.0);
    let brush = &s.masks[2];
    assert_eq!(brush.adjust.clarity, 40.0);
    assert_eq!(brush.components.len(), 1, "the brush's dab runs are one component");
    let MaskShape::Brush { strokes } = &brush.components[0].shape else { panic!() };
    assert_eq!((strokes.len(), strokes[0].points.len(), strokes[0].flow, strokes[0].feather), (2, 2, 80.0, 30.0));
    assert_eq!(unmapped, vec!["Mask: AI mask (9)".to_string()]);
}

#[test]
fn range_to_white_stays_open_and_unmapped_local_sliders_are_reported() {
    // written for this test: a highlights range (full from 0.85 up to white) carrying a local slider we lack
    let x = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/">
 <crs:MaskGroupBasedCorrections><rdf:Seq>
  <rdf:li><rdf:Description crs:What="Correction" crs:LocalClarity2012="-0.3" crs:LocalGrain="0.4" crs:LocalCurveRefineSaturation="100">
   <crs:CorrectionMasks><rdf:Seq>
    <rdf:li crs:What="Mask/RangeMask"><crs:CorrectionRangeMask crs:Type="2" crs:LumRange="0.6 0.85 1 1"/></rdf:li>
   </rdf:Seq></crs:CorrectionMasks>
  </rdf:Description></rdf:li>
 </rdf:Seq></crs:MaskGroupBasedCorrections>
</rdf:Description></rdf:RDF></x:xmpmeta>"#;
    let d = craft_metadata::parse_xmp(x).unwrap();
    let (partial, unmapped) = crate::crs::to_partial_report(&d.properties, Some(&d.values), crate::crs::Target::Any, 1.5);
    let s = DevelopSettings::default().merged(&partial).expect("valid settings");
    let MaskShape::LuminanceRange { hi, hi_feather, .. } = &s.masks[0].components[0].shape else { panic!() };
    assert_eq!((*hi, *hi_feather), (1.0, 0.0), "brighter than scene white is still in the range");
    assert_eq!(s.masks[0].adjust.clarity, -30.0);
    assert_eq!(unmapped, ["Mask: LocalGrain"], "a refine saturation of 100 is neutral, not reported");
}

#[test]
fn presets_add_masks_once() {
    let (s, _) = settings();
    let partial = json!({"masks": serde_json::to_value(&s.masks).unwrap(), "light": {"exposure": 0.5}});
    let p = crate::Preset { id: "t".into(), name: "t".into(), group: "g".into(), settings: partial, favorite: false, builtin: false };
    let mut mine = DevelopSettings::default();
    mine.masks.push(crate::Mask { id: 7, name: "Mine".into(), ..Default::default() });
    let once = p.apply(&mine, 1.0);
    assert_eq!(once.masks.len(), 4, "added to the photo's own mask");
    assert_eq!(once.masks[1].id, 8, "fresh ids");
    assert_eq!(p.apply(&once, 1.0).masks.len(), 4, "applying again adds nothing");
    let half = p.apply(&mine, 0.5);
    assert_eq!(half.masks[1].adjust.amount, 50.0, "amount scales the masks");
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
    let (partial, unmapped) = crate::crs::to_partial_report(&d.properties, Some(&d.values), crate::crs::Target::Any, 1.5);
    assert!(unmapped.is_empty(), "{unmapped:?}");
    let s = DevelopSettings::default().merged(&partial).unwrap();
    assert_eq!(s.masks.len(), 1, "{:?}", s.masks);
    s
}

/// (the radial's `invert`, its feather, the component's `invert`).
fn radial_of(s: &DevelopSettings) -> (bool, f64, bool) {
    let c = &s.masks[0].components[0];
    let MaskShape::Radial { invert, feather, .. } = &c.shape else { panic!("{:?}", c.shape) };
    (*invert, *feather, c.invert)
}

#[test]
fn lightroom_radial_flipped_is_inside() {
    // as Lightroom Classic writes an ordinary radial mask today (storytold/lightcraft#1815)
    let s = radial_settings("MaskGroupBasedCorrections", r#"crs:MaskInverted="false" crs:Feather="25" crs:Flipped="true" crs:Version="2""#);
    assert_eq!(radial_of(&s), (false, 25.0, false));
    // no MaskInverted at all reads the same
    let s = radial_settings("MaskGroupBasedCorrections", r#"crs:Feather="25" crs:Flipped="true" crs:Version="2""#);
    assert_eq!(radial_of(&s), (false, 25.0, false));
}

#[test]
fn lightroom_radial_mask_inverted_is_outside() {
    let s = radial_settings("MaskGroupBasedCorrections", r#"crs:MaskInverted="true" crs:Feather="25" crs:Flipped="true" crs:Version="2""#);
    assert_eq!(radial_of(&s), (false, 25.0, true), "the inversion is the component's");
    // version 2 with Flipped="false": outside, the legacy sense of the flag
    let s = radial_settings("MaskGroupBasedCorrections", r#"crs:MaskInverted="false" crs:Feather="25" crs:Flipped="false" crs:Version="2""#);
    assert_eq!(radial_of(&s), (true, 25.0, false));
}

#[test]
fn versionless_group_radial_is_inside_with_half_the_feather() {
    // Lightroom renders both inside and rewrites them Flipped="true", Feather halved, Version="2"
    for flipped in ["true", "false"] {
        let s = radial_settings("MaskGroupBasedCorrections", &format!(r#"crs:MaskInverted="false" crs:Feather="50" crs:Flipped="{flipped}""#));
        assert_eq!(radial_of(&s), (false, 25.0, false), "Flipped={flipped}");
    }
}

#[test]
fn legacy_radial_filter_is_outside_unless_flipped() {
    // the process-2012 Radial Filter: effect outside by default, "Invert Mask" = Flipped="true"
    let s = radial_settings("CircularGradientBasedCorrections", r#"crs:Feather="50" crs:Flipped="false""#);
    assert_eq!(radial_of(&s), (true, 50.0, false));
    assert_eq!(radial_of(&radial_settings("CircularGradientBasedCorrections", r#"crs:Feather="50""#)), (true, 50.0, false));
    let s = radial_settings("CircularGradientBasedCorrections", r#"crs:Feather="50" crs:Flipped="true""#);
    assert_eq!(radial_of(&s), (false, 50.0, false));
}

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

#[test]
fn retouch_areas_become_spots() {
    use crate::SpotMode;
    let d = craft_metadata::parse_xmp(SPOTS).unwrap();
    let (partial, unmapped) = crate::crs::to_partial_report(&d.properties, Some(&d.values), crate::crs::Target::Any, 1.5);
    let s = DevelopSettings::default().merged(&partial).expect("valid settings");
    assert_eq!(s.light.exposure, 0.1, "global settings still map");
    assert_eq!(s.spots.len(), 4, "the gradient 'spot' is skipped: {:?}", s.spots);
    let heal = &s.spots[0];
    assert_eq!((heal.mode, heal.points.len(), heal.opacity, heal.feather), (SpotMode::Heal, 1, 100.0, 0.0));
    assert!((heal.points[0].x - 0.4).abs() < 1e-9 && (heal.points[0].y - 0.3).abs() < 1e-9);
    assert!((heal.size - 0.01).abs() < 1e-9, "a landscape target: width fractions are long-edge fractions");
    let src = heal.source_offset.expect("explicit source");
    assert!((src.x - 0.05).abs() < 1e-9 && src.y.abs() < 1e-9, "source stored absolute, ours an offset: {src:?}");
    let clone = &s.spots[1];
    assert_eq!((clone.mode, clone.opacity, clone.feather, clone.source_offset), (SpotMode::Clone, 50.0, 25.0, None));
    let brushed = &s.spots[2];
    assert_eq!((brushed.points.len(), brushed.size), (3, 0.03));
    let src = brushed.source_offset.expect("explicit source");
    assert!((src.x - 0.1).abs() < 1e-9 && (src.y - 0.1).abs() < 1e-9);
    let circ = &s.spots[3];
    assert!((circ.points[0].x - 0.5).abs() < 1e-9 && (circ.points[0].y - 0.5).abs() < 1e-9 && (circ.size - 0.01).abs() < 1e-9);
    assert_eq!(unmapped, vec!["Spot: Gradient".to_string()], "{unmapped:?}");
}

#[test]
fn portrait_target_scales_spot_sizes() {
    let d = craft_metadata::parse_xmp(SPOTS).unwrap();
    let (partial, _) = crate::crs::to_partial_report(&d.properties, Some(&d.values), crate::crs::Target::Any, 2.0 / 3.0);
    let s = DevelopSettings::default().merged(&partial).unwrap();
    assert!((s.spots[0].size - 0.01 * 2.0 / 3.0).abs() < 1e-9, "a width fraction on a portrait target: {}", s.spots[0].size);
}

#[test]
fn legacy_retouch_info_is_read_on_its_own() {
    let packet = SPOTS.replace("crs:RetouchAreas", "crs:Ignored");
    let d = craft_metadata::parse_xmp(&packet).unwrap();
    let (partial, unmapped) = crate::crs::to_partial_report(&d.properties, Some(&d.values), crate::crs::Target::Any, 1.5);
    let s = DevelopSettings::default().merged(&partial).unwrap();
    assert_eq!(s.spots.len(), 1, "{:?}", s.spots);
    assert_eq!(s.spots[0].mode, crate::SpotMode::Heal);
    assert!((s.spots[0].points[0].x - 0.4).abs() < 1e-9 && (s.spots[0].size - 0.01).abs() < 1e-9);
    assert!(s.spots[0].source_offset.is_some_and(|o| (o.x - 0.05).abs() < 1e-9));
    assert!(unmapped.iter().any(|u| u == "Ignored"), "{unmapped:?}");
    assert!(!unmapped.iter().any(|u| u == "RetouchInfo"), "{unmapped:?}");
}
