use super::*;
use craft_metadata::XmpValue;

/// A hand-written settings file laid out as Photoshop 26's Camera Raw Filter saves one (the
/// field set and order follow Camera Raw 18; the values are synthetic), plus element-form
/// settings, which XMP allows and some writers use.
const FIXTURE: &[u8] = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core 7.0-c000 1.000000, 0000/00/00-00:00:00        ">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
   crs:Version="18.1.1" crs:ProcessVersion="15.4" crs:WhiteBalance="Custom"
   crs:IncrementalTemperature="0" crs:IncrementalTint="+10" crs:Exposure2012="+0.05"
   crs:Contrast2012="-5" crs:Highlights2012="-20" crs:Shadows2012="+10" crs:Whites2012="+35"
   crs:Blacks2012="0" crs:Texture="+30" crs:Clarity2012="+5" crs:Vibrance="+20" crs:Saturation="-5"
   crs:ParametricShadowSplit="25" crs:ParametricMidtoneSplit="50" crs:ParametricHighlightSplit="75"
   crs:Sharpness="15" crs:SharpenRadius="+1.0" crs:LuminanceSmoothing="5"
   crs:LuminanceNoiseReductionContrast="0" crs:HueAdjustmentGreen="+25" crs:SaturationAdjustmentGreen="+20"
   crs:ColorGradeBlending="50" crs:LensProfileEnable="0" crs:VignetteAmount="-20"
   crs:PerspectiveScale="100" crs:GrainAmount="10" crs:RedHue="-5" crs:GrainSeed="2834497962"
   crs:ToneCurveName2012="Linear" crs:CameraProfile="Default Color" crs:HasSettings="True" crs:HasCrop="False">
   <crs:Dehaze>+12</crs:Dehaze>
   <crs:ToneCurvePV2012>
<rdf:Seq>
 <rdf:li>0, 0</rdf:li>
 <rdf:li>255, 255</rdf:li>
</rdf:Seq>
   </crs:ToneCurvePV2012>
   <crs:ToneCurvePV2012Red>
<rdf:Seq>
 <rdf:li>0, 0</rdf:li>
 <rdf:li>128, 140</rdf:li>
 <rdf:li>255, 255</rdf:li>
</rdf:Seq>
   </crs:ToneCurvePV2012Red>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

fn text<'a>(s: &'a Settings, name: &str) -> &'a str {
    s.text(name).unwrap_or_else(|| panic!("no {name}"))
}

#[test]
fn reads_a_photoshop_preset() {
    let s = Settings::parse(FIXTURE).unwrap();
    assert_eq!(s.entries.len(), 35 + 3, "the attributes and the three elements");
    assert_eq!((text(&s, "Version"), text(&s, "WhiteBalance"), text(&s, "CameraProfile")), ("18.1.1", "Custom", "Default Color"));
    assert_eq!(number(text(&s, "IncrementalTint")), Some(10.0));
    assert_eq!(number(text(&s, "Exposure2012")), Some(0.05));
    assert_eq!(number(text(&s, "Contrast2012")), Some(-5.0));
    assert_eq!(number(text(&s, "GrainSeed")), Some(2_834_497_962.0));
    // An element-form setting reads as an attribute does.
    assert_eq!(number(text(&s, "Dehaze")), Some(12.0));
    let Some(Value::Seq(linear)) = s.get("ToneCurvePV2012") else { panic!("not a sequence") };
    assert_eq!(points(linear), Some(vec![[0.0, 0.0], [255.0, 255.0]]));
    let Some(Value::Seq(red)) = s.get("ToneCurvePV2012Red") else { panic!("not a sequence") };
    assert_eq!(points(red), Some(vec![[0.0, 0.0], [128.0, 140.0], [255.0, 255.0]]));
    assert_eq!(s.text("ToneCurvePV2012"), None, "a sequence is not text");
    assert_eq!(s.crop(), CropState::None);
    assert_eq!(toolkit(FIXTURE).as_deref(), Some("Adobe XMP Core 7.0-c000 1.000000, 0000/00/00-00:00:00        "));
}

#[test]
fn structures_other_prefixes_and_other_namespaces() {
    // The file's own prefix for the namespace does not matter; other namespaces are not settings.
    let xmp = format!(
        "<a:xmpmeta xmlns:a=\"{META_NS}\"><r:RDF xmlns:r=\"{RDF_NS}\"><r:Description xmlns:c=\"{CRS_NS}\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" c:Texture=\"+3\" dc:format=\"image/x-canon-cr2\" about=\"\">\
         <c:Look><r:Description c:Name=\"Adobe Color\"/></c:Look><c:MaskGroupBasedCorrections><r:Seq><r:li><r:Description c:What=\"Correction\"/></r:li></r:Seq></c:MaskGroupBasedCorrections>\
         <dc:title>t</dc:title><c:Empty/></r:Description></r:RDF></a:xmpmeta>"
    );
    let s = Settings::parse(xmp.as_bytes()).unwrap();
    let names: Vec<&str> = s.entries.keys().map(String::as_str).collect();
    // A description inside a structure belongs to the structure, not to the settings.
    assert_eq!(names, ["Empty", "Look", "MaskGroupBasedCorrections", "Texture"]);
    let Some(Value::Structured(look)) = s.get("Look") else { panic!("not a structure") };
    assert_eq!(look.field("crs:Name").and_then(XmpValue::text), Some("Adobe Color"), "fields carry the canonical prefix");
    let Some(Value::Structured(masks)) = s.get("MaskGroupBasedCorrections") else { panic!("a sequence of descriptions is a structure") };
    assert_eq!(masks.items().first().and_then(|m| m.field("crs:What")).and_then(XmpValue::text), Some("Correction"));
    assert_eq!(s.get("Empty"), Some(&Value::Text(String::new())));
    assert_eq!(s.get("format"), None);
    // The last value of a repeated name wins.
    let second = format!("/><rdf:Description xmlns:crs=\"{CRS_NS}\" crs:Texture=\"2\"/></rdf:RDF>");
    let twice = with("crs:Texture=\"1\"").replace("/></rdf:RDF>", &second);
    assert_eq!(Settings::parse(twice.as_bytes()).unwrap().text("Texture"), Some("2"));
}

#[test]
fn numbers_and_points() {
    for (s, v) in [("+10", 10.0), (" -0.50 ", -0.5), ("0", 0.0), ("1e2", 100.0), ("2834497962", 2_834_497_962.0)] {
        assert_eq!(number(s), Some(v), "{s}");
    }
    for s in ["", "+", "NaN", "inf", "-inf", "1,5", "True"] {
        assert_eq!(number(s), None, "{s}");
    }
    assert_eq!(
        (signed(0.05, 2), signed(-12.0, 0), signed(0.0, 0), signed(1.0, 1), signed(0.004, 2)),
        ("+0.05".into(), "-12".into(), "0".into(), "+1.0".into(), "+0.00".into())
    );
    let items = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(points(&items(&["0, 0", " 32 ,22", "+255,255"])), Some(vec![[0.0, 0.0], [32.0, 22.0], [255.0, 255.0]]));
    assert_eq!(points(&[]), Some(vec![]));
    for bad in [&["0 0"][..], &["0, x"], &["0, 0", ""], &["1, 2, 3"]] {
        assert_eq!(points(&items(bad)), None, "{bad:?}");
    }
}

/// A settings file nested 20 000 deep is an error. The small stack makes a regression abort
/// instead of passing.
#[test]
fn deeply_nested_settings_are_an_error_not_a_stack_overflow() {
    let xml = format!("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">{}{}</x:xmpmeta>", "<g>".repeat(20_000), "</g>".repeat(20_000));
    let run = move || (Settings::parse(xml.as_bytes()).err(), toolkit(xml.as_bytes()));
    let (e, tk) = std::thread::Builder::new().stack_size(512 * 1024).spawn(run).unwrap().join().unwrap();
    assert!(matches!(&e, Some(Error::Xml(m)) if m.contains("deeply nested")), "{e:?}");
    assert_eq!(tk, None);
}

#[test]
fn malformed_input_is_an_error() {
    let big = vec![b' '; MAX_XMP_BYTES + 1];
    let cases: [(&[u8], Error); 4] = [
        (&big, Error::TooLarge),
        (&[0xff, 0xfe, 0x00], Error::NotText),
        (b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\"/></rdf:RDF></x:xmpmeta>", Error::NoSettings),
        (b"<rdf:Description xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\" xmlns:crs=\"http://example.com/not-camera-raw\" crs:Texture=\"1\"/>", Error::NoSettings),
    ];
    for (bytes, want) in cases {
        assert_eq!(Settings::parse(bytes).err(), Some(want));
    }
    for bytes in [&b""[..], b"not xml", b"<x:xmpmeta", b"<a><b></a>", b"<!DOCTYPE x [<!ENTITY e \"v\">]><x/>", b"{\"crs\": 1}"] {
        // Not XMP: refused by the XMP reader, or read as a packet with no settings.
        assert!(matches!(Settings::parse(bytes), Err(Error::Xml(_) | Error::NoSettings)), "{:?}", String::from_utf8_lossy(bytes));
    }
    // Truncations and bit flips of a real file never panic.
    for cut in 0..FIXTURE.len() {
        let _ = Settings::parse(&FIXTURE[..cut]).map(|s| s.crop());
    }
    let mut state = 0x9e37_79b9_u32;
    for _ in 0..500 {
        let mut b = FIXTURE.to_vec();
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let i = state as usize % b.len();
        b[i] ^= (state >> 24) as u8 | 1;
        let _ = Settings::parse(&b).map(|s| s.crop());
        let _ = toolkit(&b);
    }
}

#[test]
fn the_toolkit_names_the_writer() {
    let mut own = Settings::default();
    own.set("Texture", "+3");
    let own = own.to_xmp("Example \"App\" & Co");
    assert_eq!(toolkit(own.as_bytes()).as_deref(), Some("Example \"App\" & Co"));
    let wrapped = format!("\u{feff}<?xpacket begin=\"\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n{own}<?xpacket end=\"w\"?>");
    assert_eq!(toolkit(wrapped.as_bytes()).as_deref(), Some("Example \"App\" & Co"), "an xpacket wrapper and a byte order mark");
    assert_eq!(Settings::parse(wrapped.as_bytes()).unwrap().text("Texture"), Some("+3"));
    // A file without the marker, with another namespace's xmptk, and one that is not XMP.
    assert_eq!(toolkit(with("crs:Texture=\"1\"").as_bytes()), None);
    assert_eq!(toolkit(b"<x:xmpmeta xmlns:x=\"http://example.com/\" x:xmptk=\"Example\"/>"), None);
    assert_eq!(toolkit(b"<xmpmeta xmptk=\"Example\"/>"), None);
    assert_eq!(toolkit(b"PK\x03\x04"), None);
    assert_eq!(toolkit(&vec![b' '; MAX_XMP_BYTES + 1]), None);
}

#[test]
fn a_written_file_reads_back_the_same() {
    let mut s = Settings::default();
    s.set("Version", "18.4");
    s.set("Exposure2012", signed(0.5, 2));
    s.set("CameraProfile", "A <\"quoted\"> & 'odd' name\tx");
    s.set_crop(Crop { top: 0.125, left: 0.25, bottom: 0.875, right: 0.5 });
    s.set("HasSettings", "True");
    s.set_seq("ToneCurvePV2012", vec!["0, 0".into(), "128, <140>".into(), "255, 255".into()]);
    s.set_seq("ToneCurvePV2012Red", vec![]);
    let xmp = s.to_xmp("Example");
    assert!(xmp.starts_with(
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"Example\">\n <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n  <rdf:Description rdf:about=\"\"\n    xmlns:crs=\"http://ns.adobe.com/camera-raw-settings/1.0/\"\n   crs:CameraProfile="
    ));
    assert!(xmp.contains("\n   crs:CropAngle=\"0\"\n   crs:CropBottom=\"0.875000\"\n   crs:CropLeft=\"0.250000\"\n   crs:CropRight=\"0.500000\"\n   crs:CropTop=\"0.125000\"\n   crs:Exposure2012=\"+0.50\"\n   crs:HasCrop=\"True\"\n   crs:HasSettings=\"True\"\n   crs:Version=\"18.4\">\n"), "{xmp}");
    assert!(xmp.contains("   <crs:ToneCurvePV2012>\n    <rdf:Seq>\n     <rdf:li>0, 0</rdf:li>\n     <rdf:li>128, &lt;140&gt;</rdf:li>\n     <rdf:li>255, 255</rdf:li>\n    </rdf:Seq>\n   </crs:ToneCurvePV2012>\n"));
    assert!(xmp.ends_with("   </crs:ToneCurvePV2012Red>\n  </rdf:Description>\n </rdf:RDF>\n</x:xmpmeta>\n"));
    let back = Settings::parse(xmp.as_bytes()).unwrap();
    assert_eq!(back, s);
    assert_eq!(back.crop(), CropState::Crop(Crop { top: 0.125, left: 0.25, bottom: 0.875, right: 0.5 }));
    // What cannot be written is left out: a structure, and a name that is not an XML name.
    let mut odd = Settings::default();
    let foreign =
        XmpValue::Struct([("dc:title".to_string(), XmpValue::Text("x".into())), ("crs:Name".to_string(), XmpValue::Text("n".into()))].into());
    odd.entries.insert("Look".into(), Value::Structured(foreign));
    odd.set("bad name=\"x\"", "1");
    odd.set("crs:Prefixed", "1");
    odd.set("Texture", "1");
    let back = Settings::parse(odd.to_xmp("Example").as_bytes()).unwrap();
    let names: Vec<&str> = back.entries.keys().map(String::as_str).collect();
    assert_eq!(names, ["Look", "Texture"]);
    let Some(Value::Structured(look)) = back.get("Look") else { panic!("not a structure") };
    assert_eq!((look.field("crs:Name").and_then(XmpValue::text), look.field("dc:title")), (Some("n"), None), "only crs: fields are written");
    // No settings at all still writes well-formed XMP.
    assert_eq!(Settings::parse(Settings::default().to_xmp("Example").as_bytes()).err(), Some(Error::NoSettings));
}

/// A settings file with the `crs:` attributes `attrs`.
fn with(attrs: &str) -> String {
    format!(
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description xmlns:crs=\"{CRS_NS}\" crs:RawFileName=\"IMG_0004.CR2\" {attrs}/></rdf:RDF></x:xmpmeta>"
    )
}

fn crop_of(attrs: &str) -> CropState {
    Settings::parse(with(attrs).as_bytes()).unwrap().crop()
}

#[test]
fn reads_the_crop() {
    let c = crop_of(r#"crs:HasCrop="True" crs:CropTop="0.1" crs:CropLeft="0.2" crs:CropBottom="0.9" crs:CropRight="0.75" crs:CropAngle="0""#);
    assert_eq!(c, CropState::Crop(Crop { top: 0.1, left: 0.2, bottom: 0.9, right: 0.75 }));
    // Element form, and the keys spread over two descriptions, read the same.
    let xmp = format!(
        "<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description xmlns:crs=\"{CRS_NS}\"><crs:HasCrop>True</crs:HasCrop><crs:CropBottom>0.5</crs:CropBottom></rdf:Description><rdf:Description xmlns:crs=\"{CRS_NS}\" crs:CropRight=\"0.5\"/></rdf:RDF>"
    );
    let c = Settings::parse(xmp.as_bytes()).unwrap().crop();
    assert_eq!(c, CropState::Crop(Crop { top: 0.0, left: 0.0, bottom: 0.5, right: 0.5 }), "missing edges are the frame's");
}

#[test]
fn no_crop_without_has_crop_true() {
    let keys = r#"crs:CropTop="0.1" crs:CropLeft="0.2" crs:CropBottom="0.9" crs:CropRight="0.75" crs:CropAngle="3""#;
    for has in ["", r#"crs:HasCrop="False""#, r#"crs:HasCrop="false""#, r#"crs:HasCrop=" FALSE ""#] {
        assert_eq!(crop_of(&format!("{has} {keys}")), CropState::None, "{has:?}");
    }
    // The whole frame is no crop, and nothing to report.
    assert_eq!(
        crop_of(r#"crs:HasCrop="True" crs:CropTop="0" crs:CropLeft="0" crs:CropBottom="1" crs:CropRight="1" crs:CropAngle="0""#),
        CropState::None
    );
    assert_eq!(crop_of(r#"crs:HasCrop="yes" crs:CropBottom="0.5""#), CropState::Unusable(vec!["HasCrop"]));
}

/// Hostile and straightened crops: out-of-range fractions are clamped to the frame; an inverted,
/// empty, unreadable or straightened crop is unusable and its keys are named.
#[test]
fn unusable_crops_are_reported() {
    let c = crop_of(r#"crs:HasCrop="True" crs:CropTop="-0.5" crs:CropLeft="0.25" crs:CropBottom="7" crs:CropRight="1.0000001""#);
    assert_eq!(c, CropState::Crop(Crop { top: 0.0, left: 0.25, bottom: 1.0, right: 1.0 }));
    for bad in [
        r#"crs:CropTop="0.8" crs:CropLeft="0.1" crs:CropBottom="0.2" crs:CropRight="0.9" crs:CropAngle="0""#,
        r#"crs:CropTop="0.5" crs:CropLeft="0.1" crs:CropBottom="0.5" crs:CropRight="0.9" crs:CropAngle="0""#,
        r#"crs:CropTop="2" crs:CropLeft="0.1" crs:CropBottom="3" crs:CropRight="0.9" crs:CropAngle="0""#,
        r#"crs:CropTop="NaN" crs:CropLeft="0.1" crs:CropBottom="0.9" crs:CropRight="0.9" crs:CropAngle="0""#,
        r#"crs:CropTop="0.1" crs:CropLeft="inf" crs:CropBottom="0.9" crs:CropRight="0.9" crs:CropAngle="0""#,
        r#"crs:CropTop="0.1" crs:CropLeft="0.1" crs:CropBottom="0.9" crs:CropRight="" crs:CropAngle="0""#,
        r#"crs:CropTop="0.1" crs:CropLeft="0.1" crs:CropBottom="0.9" crs:CropRight="0.9" crs:CropAngle="1.38""#,
        r#"crs:CropTop="0.1" crs:CropLeft="0.1" crs:CropBottom="0.9" crs:CropRight="0.9" crs:CropAngle="NaN""#,
    ] {
        assert_eq!(crop_of(&format!(r#"crs:HasCrop="True" {bad}"#)), CropState::Unusable(CROP_KEYS.to_vec()), "{bad}");
    }
    assert_eq!(crop_of(r#"crs:HasCrop="True" crs:CropAngle="-2.5""#), CropState::Unusable(vec!["CropAngle"]), "only the keys the file has");
}

/// Corners map as the pixels do when the frame is turned upright.
#[test]
fn crops_follow_the_orientation() {
    let c = Crop { top: 0.1, left: 0.2, bottom: 0.6, right: 0.5 };
    let want = [
        (1, Crop { top: 0.1, left: 0.2, bottom: 0.6, right: 0.5 }),
        (2, Crop { top: 0.1, left: 0.5, bottom: 0.6, right: 0.8 }),
        (3, Crop { top: 0.4, left: 0.5, bottom: 0.9, right: 0.8 }),
        (4, Crop { top: 0.4, left: 0.2, bottom: 0.9, right: 0.5 }),
        (5, Crop { top: 0.2, left: 0.1, bottom: 0.5, right: 0.6 }),
        (6, Crop { top: 0.2, left: 0.4, bottom: 0.5, right: 0.9 }),
        (7, Crop { top: 0.5, left: 0.4, bottom: 0.8, right: 0.9 }),
        (8, Crop { top: 0.5, left: 0.1, bottom: 0.8, right: 0.6 }),
        (0, Crop { top: 0.1, left: 0.2, bottom: 0.6, right: 0.5 }),
    ];
    for (o, w) in want {
        let got = c.oriented(o);
        let close = [got.top - w.top, got.left - w.left, got.bottom - w.bottom, got.right - w.right].iter().all(|d| d.abs() < 1e-6);
        assert!(close, "orientation {o}: {got:?}");
    }
    assert_eq!(c.pixels(10, 20), Some((2, 2, 5, 12)));
    assert_eq!(Crop { top: 0.5, left: 0.5, bottom: 0.5001, right: 0.5001 }.pixels(10, 10), Some((5, 5, 6, 6)), "at least a pixel");
    assert_eq!(Crop { top: 0.999, left: 0.999, bottom: 1.0, right: 1.0 }.pixels(10, 10), Some((9, 9, 10, 10)), "inside the frame");
    assert_eq!(c.pixels(0, 10), None);
}

#[test]
fn structures_are_written_back_as_they_were_read() {
    // A local correction as Lightroom writes it, with a nested sequence and a text property.
    let correction = format!(
        "<x:xmpmeta xmlns:x=\"{META_NS}\"><rdf:RDF xmlns:rdf=\"{RDF_NS}\"><rdf:Description xmlns:crs=\"{CRS_NS}\" crs:Exposure2012=\"+0.50\">\
         <crs:GradientBasedCorrections><rdf:Seq><rdf:li><rdf:Description crs:What=\"Correction\" crs:LocalExposure2012=\"0.25\">\
         <crs:CorrectionMasks><rdf:Seq><rdf:li crs:What=\"Mask/Gradient\" crs:ZeroX=\"0.1\" crs:FullX=\"0.9\"/></rdf:Seq></crs:CorrectionMasks>\
         <crs:CorrectionName>Sky &amp; sea</crs:CorrectionName></rdf:Description></rdf:li></rdf:Seq></crs:GradientBasedCorrections>\
         </rdf:Description></rdf:RDF></x:xmpmeta>"
    );
    let s = Settings::parse(correction.as_bytes()).unwrap();
    let Some(Value::Structured(v)) = s.get("GradientBasedCorrections") else { panic!("not a structure") };
    let d = &v.items()[0];
    let text = |v: &XmpValue, k: &str| v.field(k).and_then(XmpValue::text).map(str::to_string);
    assert_eq!((text(d, "crs:LocalExposure2012"), text(d, "crs:CorrectionName")), (Some("0.25".into()), Some("Sky & sea".into())));
    let mask = &d.field("crs:CorrectionMasks").unwrap().items()[0];
    assert_eq!((text(mask, "crs:What"), text(mask, "crs:ZeroX")), (Some("Mask/Gradient".into()), Some("0.1".into())));
    assert_eq!(s.entries.len(), 2, "the correction's own properties are not settings");
    let back = Settings::parse(s.to_xmp("Example").as_bytes()).unwrap();
    assert_eq!(back, s);
}
