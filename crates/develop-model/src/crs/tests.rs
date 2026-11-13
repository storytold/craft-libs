use super::*;
use crate::{DevelopSettings, Upright, VignetteStyle, WbMode, apply_partial};

/// A hand-written sidecar in attribute form (as many tools write it).
const SIDECAR: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
    crs:Version="99.0" crs:ProcessVersion="11.0" crs:WhiteBalance="Custom" crs:Temperature="5150" crs:Tint="+12"
    crs:Exposure2012="+0.65" crs:Contrast2012="-14" crs:Highlights2012="-58" crs:Shadows2012="+41"
    crs:Whites2012="+9" crs:Blacks2012="-17" crs:Texture="+6" crs:Clarity2012="+11" crs:Dehaze="+4"
    crs:Vibrance="+19" crs:Saturation="-3" crs:HueAdjustmentOrange="-7" crs:SaturationAdjustmentBlue="-22"
    crs:LuminanceAdjustmentAqua="+13" crs:ParametricShadows="+5" crs:ParametricHighlights="-8"
    crs:ParametricMidtoneSplit="55" crs:CurveRefineSaturation="40" crs:SplitToningShadowHue="210" crs:SplitToningShadowSaturation="14"
    crs:SplitToningHighlightHue="40" crs:SplitToningHighlightSaturation="9" crs:SplitToningBalance="+20"
    crs:ColorGradeMidtoneHue="120" crs:ColorGradeMidtoneSat="6" crs:ColorGradeBlending="70"
    crs:Sharpness="55" crs:SharpenRadius="+1.2" crs:SharpenDetail="30" crs:SharpenEdgeMasking="12"
    crs:LuminanceSmoothing="18" crs:ColorNoiseReduction="25" crs:PostCropVignetteAmount="-21"
    crs:PostCropVignetteStyle="2" crs:GrainAmount="15" crs:GrainSize="30" crs:GrainFrequency="60"
    crs:PerspectiveUpright="2" crs:PerspectiveVertical="-10" crs:AutoLateralCA="1" crs:ConvertToGrayscale="False"
    crs:ShadowTint="-7" crs:RedHue="+12" crs:RedSaturation="+20" crs:GreenHue="-5" crs:GreenSaturation="+3"
    crs:BlueHue="-15" crs:BlueSaturation="+25"
    crs:HasCrop="True" crs:CropTop="0.1" crs:CropLeft="0.05" crs:CropBottom="0.9" crs:CropRight="0.8" crs:CropAngle="1.5">
   <crs:ToneCurvePV2012>
    <rdf:Seq><rdf:li>0, 0</rdf:li><rdf:li>64, 52</rdf:li><rdf:li>192, 205</rdf:li><rdf:li>255, 255</rdf:li></rdf:Seq>
   </crs:ToneCurvePV2012>
   <crs:ToneCurvePV2012Red>
    <rdf:Seq><rdf:li>0, 0</rdf:li><rdf:li>255, 255</rdf:li></rdf:Seq>
   </crs:ToneCurvePV2012Red>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

fn props(x: &str) -> Props {
    craft_metadata::parse_xmp(x).unwrap().properties
}

#[test]
fn maps_common_fields() {
    let p = props(SIDECAR);
    let partial = to_partial(&p, Target::RawAbsolute);
    let s = apply_partial(&DevelopSettings::for_raw(5000.0, 0.0), &partial, 1.0);
    assert!(has_adjustments(&p));
    assert_common_tone_fields(&s);
    assert_common_color_fields(&s);
    assert_common_detail_fields(&s);
    assert_common_geometry_fields(&s);
}

fn assert_common_tone_fields(s: &DevelopSettings) {
    assert_eq!(s.wb.mode, WbMode::Custom);
    assert_eq!((s.wb.temp, s.wb.tint), (5150.0, 12.0));
    assert_eq!(s.light.exposure, 0.65);
    assert_eq!((s.light.contrast, s.light.highlights, s.light.shadows, s.light.whites, s.light.blacks), (-14.0, -58.0, 41.0, 9.0, -17.0));
    assert_eq!((s.effects.texture, s.effects.clarity, s.effects.dehaze), (6.0, 11.0, 4.0));
    assert_eq!((s.color.vibrance, s.color.saturation), (19.0, -3.0));
    assert_eq!(s.mixer.orange.hue, -7.0);
    assert_eq!(s.mixer.blue.sat, -22.0);
    assert_eq!(s.mixer.aqua.lum, 13.0);
    assert_eq!((s.curve.shadows, s.curve.highlights, s.curve.split_mid), (5.0, -8.0, 55.0));
    assert_eq!(s.curve.master.len(), 4);
    assert_eq!(s.curve.refine_saturation, 40.0);
    assert!((s.curve.master[1].x - 64.0 / 255.0).abs() < 1e-9 && (s.curve.master[1].y - 52.0 / 255.0).abs() < 1e-9);
    assert!(s.curve.red.is_empty(), "identity channel curve maps to no curve");
}

fn assert_common_color_fields(s: &DevelopSettings) {
    assert_eq!((s.grading.shadows.hue, s.grading.shadows.sat), (210.0, 14.0));
    assert_eq!((s.grading.highlights.hue, s.grading.highlights.sat), (40.0, 9.0));
    assert_eq!((s.grading.midtones.hue, s.grading.midtones.sat), (120.0, 6.0));
    assert_eq!((s.grading.balance, s.grading.blending), (20.0, 70.0));
}

fn assert_common_detail_fields(s: &DevelopSettings) {
    assert_eq!((s.detail.sharpen_amount, s.detail.sharpen_radius, s.detail.sharpen_detail, s.detail.sharpen_masking), (55.0, 1.2, 30.0, 12.0));
    assert_eq!((s.detail.nr_luminance, s.detail.nr_color), (18.0, 25.0));
    assert_eq!(s.vignette.amount, -21.0);
    assert_eq!(s.vignette.style, VignetteStyle::ColorPriority);
    assert_eq!((s.grain.amount, s.grain.size, s.grain.roughness), (15.0, 30.0, 60.0));
}

fn assert_common_geometry_fields(s: &DevelopSettings) {
    assert_eq!(s.geometry.upright, Upright::Level);
    assert_eq!(s.geometry.vertical, -10.0);
    assert!(s.optics.remove_ca);
    assert_eq!(s.treatment, crate::Treatment::Color);
    let cal = s.calibration;
    assert_eq!(
        (cal.shadows_tint, cal.red_hue, cal.red_sat, cal.green_hue, cal.green_sat, cal.blue_hue, cal.blue_sat),
        (-7.0, 12.0, 20.0, -5.0, 3.0, -15.0, 25.0)
    );
    let c = s.crop.geometry;
    assert_eq!((c.rect.x0, c.rect.y0, c.rect.x1, c.rect.y1, c.angle), (0.05, 0.1, 0.8, 0.9, 1.5));
}

/// A sidecar curve with a `NaN` or `inf` point (Rust's `parse` reads both) is not a curve:
/// it is left out, rather than stored with a point the curve editor can't place.
#[test]
fn curves_with_non_finite_points_are_left_out() {
    for bad in ["NaN, 0", "64, inf", "-inf, 3"] {
        let x = SIDECAR.replace("<rdf:li>64, 52</rdf:li>", &format!("<rdf:li>{bad}</rdf:li>"));
        let partial = to_partial(&props(&x), Target::RawAbsolute);
        let s = apply_partial(&DevelopSettings::for_raw(5000.0, 0.0), &partial, 1.0);
        assert!(s.curve.master.is_empty(), "{bad}: {:?}", s.curve.master);
        assert!(s.curve.master.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
    }
}

#[test]
fn only_present_fields_are_in_the_partial() {
    let x = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description
            xmlns:c="http://ns.adobe.com/camera-raw-settings/1.0/" c:Vibrance="+30" c:GrainAmount="10"/></rdf:RDF>"#;
    let partial = to_partial(&props(x), Target::Any);
    assert_eq!(partial, json!({"color": {"vibrance": 30.0}, "grain": {"amount": 10.0}}));
}

#[test]
fn hdr_edit_mode_turns_hdr_editing_on_or_off() {
    let x = |v: &str| {
        format!(
            r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description
                xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:HDREditMode="{v}" crs:Exposure2012="+1.5"/></rdf:RDF>"#
        )
    };
    let on = apply_partial(&DevelopSettings::default(), &to_partial(&props(&x("1")), Target::RawAbsolute), 1.0);
    assert!(on.hdr.enabled);
    assert_eq!(on.light.exposure, 1.5);
    assert_eq!(on.hdr.max_ev, crate::Hdr::DEFAULT_MAX_EV, "the rest of the HDR settings keep their defaults");
    let true_form = apply_partial(&DevelopSettings::default(), &to_partial(&props(&x("True")), Target::RawAbsolute), 1.0);
    assert!(true_form.hdr.enabled);
    let mut hdr = DevelopSettings::default();
    hdr.hdr.enabled = true;
    assert!(!apply_partial(&hdr, &to_partial(&props(&x("0")), Target::RawAbsolute), 1.0).hdr.enabled, "0 turns it off");
    // without the field nothing about HDR changes
    let plain = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description
            xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Exposure2012="+1.5"/></rdf:RDF>"#;
    assert!(to_partial(&props(plain), Target::RawAbsolute).get("hdr").is_none());
}

#[test]
fn element_form_bw_and_incremental_wb() {
    let x = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
          <rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/">
            <crs:ConvertToGrayscale>True</crs:ConvertToGrayscale>
            <crs:GrayMixerBlue>-35</crs:GrayMixerBlue>
            <crs:IncrementalTemperature>25</crs:IncrementalTemperature>
            <crs:IncrementalTint>-6</crs:IncrementalTint>
            <crs:Temperature>4800</crs:Temperature>
          </rdf:Description></rdf:RDF></x:xmpmeta>"#;
    let p = props(x);
    let rendered = apply_partial(&DevelopSettings::default(), &to_partial(&p, Target::Rendered), 1.0);
    assert_eq!(rendered.treatment, crate::Treatment::Bw);
    assert_eq!(rendered.bw_mix.blue, -35.0);
    assert!((rendered.wb.temp - rel_to_kelvin(25.0)).abs() < 1e-6 && rendered.wb.temp > 6500.0);
    assert_eq!(rendered.wb.tint, -6.0);
    let raw = apply_partial(&DevelopSettings::default(), &to_partial(&p, Target::RawAbsolute), 1.0);
    assert_eq!(raw.wb.temp, 4800.0);
}

#[test]
fn named_wb_and_already_applied() {
    let x = r#"<rdf:Description xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
             crs:WhiteBalance="Daylight" crs:Temperature="5500" crs:Tint="10" crs:AlreadyApplied="True"/>"#;
    let p = props(x);
    assert_eq!(to_partial(&p, Target::RawAbsolute), json!({"wb": {"mode": "daylight"}}));
    assert!(!has_adjustments(&p));
    let bookkeeping = r#"<rdf:Description xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
             xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Version="1" crs:HasSettings="True"/>"#;
    assert!(!has_adjustments(&props(bookkeeping)));
}

#[test]
fn custom_kelvin_on_relative_wb_raws_shifts_from_the_as_shot_white() {
    // Issue storytold/lightcraft#510: a Sony ARW edited in Lightroom to Custom 3578 K / −5 next to an as-shot white
    // of 3650 K / −2 (written for this test from the numbers in the issue).
    let x = r#"<rdf:Description xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
             crs:WhiteBalance="Custom" crs:Temperature="3578" crs:Tint="-5" crs:AsShotTemperature="3650" crs:AsShotTint="-2"/>"#;
    let p = props(x);
    let rel = to_partial(&p, Target::RawRelative);
    // 3650 → 3578 K is +5.5 mired: the same shift from 6500 K lands near 6275 K on our scale
    let t = rel["wb"]["temp"].as_f64().unwrap();
    assert!((t - 6275.0).abs() < 5.0, "{t}");
    assert_eq!(rel["wb"]["tint"], -3.0);
    assert_eq!(rel["wb"]["mode"], "custom");
    // a rendered file without `Incremental*` fields shifts the same way; a raw with a measured
    // illuminant (and a preset) keeps the Kelvin value
    assert_eq!(to_partial(&p, Target::Rendered)["wb"]["temp"], rel["wb"]["temp"]);
    assert_eq!(to_partial(&p, Target::RawAbsolute)["wb"]["temp"], 3578.0);
    assert_eq!(to_partial(&p, Target::Any)["wb"]["temp"], 3578.0);
    // Kelvin read as Kelvin says so, so it stays Kelvin on a raw with that scale; a shift does not
    assert_eq!(to_partial(&p, Target::RawAbsolute)["wb"]["scale"], "kelvin");
    assert_eq!(to_partial(&p, Target::Any)["wb"]["scale"], "kelvin");
    assert!(rel["wb"].get("scale").is_none() && to_partial(&p, Target::Rendered)["wb"].get("scale").is_none());
    // the as-shot fields are a reference, not an adjustment: nothing to report
    assert!(to_partial_report(&p, None, Target::RawRelative, 1.5).1.is_empty());
    // no tint next to the as-shot tint: the custom tint is taken as the shift itself
    let no_tint = props(&x.replace(r#" crs:AsShotTint="-2""#, ""));
    assert_eq!(to_partial(&no_tint, Target::RawRelative)["wb"]["tint"], -5.0);
}

#[test]
fn custom_kelvin_without_an_as_shot_reference_stays_as_shot_and_is_reported() {
    // The same edit from a Lightroom catalog, which keeps no as-shot white for Custom photos:
    // 3578 K read on the relative scale would be a large blue shift (issue storytold/lightcraft#510)
    let x = r#"<rdf:Description xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
             crs:WhiteBalance="Custom" crs:Temperature="3578" crs:Tint="-5" crs:Exposure2012="+0.3"/>"#;
    let p = props(x);
    let (partial, unmapped) = to_partial_report(&p, None, Target::RawRelative, 1.5);
    assert_eq!(partial["wb"], json!({"mode": "asShot"}));
    assert_eq!(partial["light"]["exposure"], 0.3);
    assert_eq!(unmapped.len(), 1, "{unmapped:?}");
    assert!(unmapped[0].starts_with("Temperature, Tint"), "{unmapped:?}");
    // a raw with a measured illuminant and a rendered file read it as before, and report nothing
    assert_eq!(to_partial(&p, Target::RawAbsolute)["wb"]["temp"], 3578.0);
    assert_eq!(to_partial(&p, Target::Rendered)["wb"]["temp"], 3578.0);
    assert!(to_partial_report(&p, None, Target::RawAbsolute, 1.5).1.is_empty());
    // an unusable reference (zero) counts as none
    let zero = props(&x.replace(r#" crs:Tint="-5""#, r#" crs:Tint="-5" crs:AsShotTemperature="0""#));
    assert_eq!(to_partial(&zero, Target::RawRelative)["wb"], json!({"mode": "asShot"}));
}

/// Lightroom writes `crs:Temperature` as Kelvin for a raw whose camera LightCraft has matrices for: the value
/// is read as written, marked Kelvin, and the as-shot reference is not needed (issue storytold/lightcraft#730); the same packet on
/// a raw without matrices still takes the storytold/lightcraft#510 shift.
#[test]
fn kelvin_is_direct_on_raws_whose_camera_has_matrices() {
    let x = r#"<rdf:Description xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
             crs:WhiteBalance="Custom" crs:Temperature="4650" crs:Tint="+3" crs:AsShotTemperature="5400" crs:AsShotTint="-1"/>"#;
    let p = props(x);
    let a7m3 = Target::RawAbsolute; // a camera with measured matrices
    assert_eq!(to_partial(&p, a7m3)["wb"], json!({"mode": "custom", "temp": 4650.0, "tint": 3.0, "scale": "kelvin"}));
    let a7m2 = Target::RawRelative; // one without
    let shifted = to_partial(&p, a7m2);
    assert!(shifted["wb"]["temp"].as_f64().is_some_and(|t| t > 5000.0 && t < 6500.0), "{shifted}");
    assert_eq!(shifted["wb"]["tint"], 4.0);
    assert!(shifted["wb"].get("scale").is_none());
    // without the reference the Kelvin camera still reads it, the other stays As Shot
    let bare = props(&x.replace(r#" crs:AsShotTemperature="5400" crs:AsShotTint="-1""#, ""));
    assert_eq!(to_partial(&bare, a7m3)["wb"]["temp"], 4650.0);
    assert_eq!(to_partial(&bare, a7m2)["wb"], json!({"mode": "asShot"}));
    // a named preset carries no numbers either way
    let daylight = props(&x.replace(r#"crs:WhiteBalance="Custom""#, r#"crs:WhiteBalance="Daylight""#));
    assert_eq!(to_partial(&daylight, a7m3)["wb"], json!({"mode": "daylight"}));
}

#[test]
fn xmp_preset_with_name_and_group() {
    // Written for this test: a minimal preset-shaped packet with localized name/group.
    let x = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
          <rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
             crs:PresetType="Normal" crs:UUID="0123ABCD" crs:Contrast2012="+25" crs:SupportsAmount="True">
            <crs:Name><rdf:Alt><rdf:li xml:lang="x-default">Punchy Test</rdf:li></rdf:Alt></crs:Name>
            <crs:Group><rdf:Alt><rdf:li xml:lang="x-default">My Looks</rdf:li></rdf:Alt></crs:Group>
          </rdf:Description></rdf:RDF></x:xmpmeta>"#;
    let p = preset_from_xmp(x, "file").unwrap();
    assert_eq!((p.name.as_str(), p.group.as_str(), p.id.as_str()), ("Punchy Test", "My Looks", "user.xmp.0123abcd"));
    assert_eq!(p.settings, json!({"light": {"contrast": 25.0}}));
    assert!(!p.builtin);
    assert!(preset_from_xmp("<x/>", "f").is_none());
}

/// A preset packet (written for this test) with the bookkeeping fields Lightroom presets carry.
fn preset_packet(extra_attrs: &str, extra_elems: &str) -> String {
    format!(
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
          <rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
             crs:PresetType="Normal" crs:Cluster="" crs:UUID="0123ABCD" crs:SupportsAmount2="True" crs:SupportsAmount="True"
             crs:RequiresRGBTables="False" crs:Contrast2012="+25" crs:LensProfileEnable="0" crs:OverrideLookVignette="False"
             crs:CropConstrainToWarp="0" crs:AsShotTemperature="5500" crs:AsShotTint="10" {extra_attrs}>
            <crs:SortName><rdf:Alt><rdf:li xml:lang="x-default"/></rdf:Alt></crs:SortName>
            <crs:Description><rdf:Alt><rdf:li xml:lang="x-default"/></rdf:Alt></crs:Description>
            {extra_elems}
          </rdf:Description></rdf:RDF></x:xmpmeta>"#
    )
}

#[test]
fn channel_curves_need_the_master_and_all_channel_curves() {
    // Lightroom applies red / green / blue curves only when the master and all three are present
    let seq = |name: &str, pts: &str| format!("<crs:{name}><rdf:Seq>{pts}</rdf:Seq></crs:{name}>");
    let identity = "<rdf:li>0, 0</rdf:li><rdf:li>255, 255</rdf:li>";
    let bent = "<rdf:li>0, 0</rdf:li><rdf:li>64, 40</rdf:li><rdf:li>255, 255</rdf:li>";
    let packet = |curves: &[String]| {
        format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
          <rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:ToneCurveName2012="Custom">{}
          </rdf:Description></rdf:RDF></x:xmpmeta>"#,
            curves.concat()
        )
    };
    let partial = |curves: &[String]| to_partial(&props(&packet(curves)), Target::RawAbsolute);
    let (master, red, green, blue) = (
        seq("ToneCurvePV2012", identity),
        seq("ToneCurvePV2012Red", identity),
        seq("ToneCurvePV2012Green", bent),
        seq("ToneCurvePV2012Blue", identity),
    );
    // a channel curve alone, or with the master but without its two siblings: ignored
    let alone = partial(std::slice::from_ref(&green));
    assert!(alone.pointer("/curve/green").is_none(), "{alone}");
    let master_and_green = partial(&[master.clone(), green.clone()]);
    assert!(master_and_green.pointer("/curve/green").is_none() && master_and_green.pointer("/curve/master").is_some(), "{master_and_green}");
    // all four: read
    let all = partial(&[master, red, green, blue]);
    assert!(["/curve/master", "/curve/red", "/curve/green", "/curve/blue"].iter().all(|p| all.pointer(p).is_some()), "{all}");
}

fn unmapped(x: &str) -> Vec<String> {
    to_partial_report(&props(x), None, Target::Any, 1.5).1
}

#[test]
fn preset_bookkeeping_is_not_reported_but_real_gaps_are() {
    let empty_points = "<crs:PointColors><rdf:Seq><rdf:li>-1.000000, -1.000000, -1.000000, -1.000000</rdf:li></rdf:Seq></crs:PointColors>";
    assert_eq!(unmapped(&preset_packet(r#"crs:HDREditMode="0""#, empty_points)), Vec::<String>::new());
    // a used Point Color slot and an unknown adjustment still are (an HDR edit is carried over)
    let used_points = "<crs:PointColors><rdf:Seq><rdf:li>0.5, 0.2, 0.1, 10, 0, 0, 0, 0</rdf:li></rdf:Seq></crs:PointColors>";
    let got = unmapped(&preset_packet(r#"crs:HDREditMode="1" crs:FutureSlider="12""#, used_points));
    assert_eq!(got, ["FutureSlider", "PointColors"]);
}

#[test]
fn lightroom_profile_bookkeeping_is_not_reported() {
    // what Lightroom Classic writes next to every raw's edit: none of it changes the rendering
    let bookkeeping = r#"crs:CompatibleVersion="268435456" crs:CameraProfileDigest="0A1B" crs:LensProfileDigest="2C3D"
            crs:LensProfileName="Sony FE 24-70mm" crs:LensProfileFilename="x.lcp" crs:LensProfileIsEmbedded="False"
            crs:LensProfileSetup="LensDefaults" crs:LensProfileDistortionScale="100" crs:LensProfileVignettingScale="100"
            crs:CropConstrainToUnitSquare="1""#;
    let look = r#"<crs:Look rdf:parseType="Resource"><crs:Name>Vivid</crs:Name></crs:Look>"#;
    assert_eq!(unmapped(&preset_packet(bookkeeping, "")), Vec::<String>::new());
    assert_eq!(unmapped(&preset_packet(bookkeeping, look)), ["Look"], "the Look is a real gap");
    // a lens profile's strength away from 100 % is an adjustment we don't carry over
    let scaled = preset_packet(r#"crs:LensProfileDistortionScale="60""#, "");
    assert_eq!(unmapped(&scaled), ["LensProfileDistortionScale"]);
}

#[test]
fn disabled_lens_profile_switch_keeps_embedded_corrections() {
    // Lightroom applies a file's built-in lens corrections regardless of its profile switch,
    // and ours are only those: a preset's `LensProfileEnable=0` must not turn them off
    let p = props(&preset_packet("", ""));
    let mut d = DevelopSettings::default();
    d.optics.lens_profile = true;
    assert!(apply_partial(&d, &to_partial(&p, Target::Any), 1.0).optics.lens_profile);
    let on = props(&preset_packet("", "").replace(r#"crs:LensProfileEnable="0""#, r#"crs:LensProfileEnable="1""#));
    assert!(apply_partial(&DevelopSettings::default(), &to_partial(&on, Target::Any), 1.0).optics.lens_profile);
}
