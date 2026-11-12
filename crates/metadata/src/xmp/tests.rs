use super::*;

/// A program's own namespace, as an application would pass it.
const LC: XmpNamespace = XmpNamespace { prefix: "lc", uri: "http://ns.example.com/lc/1.0/" };

fn write_lc(meta: &Metadata, properties: &[(&str, &str)]) -> String {
    write_xmp_with(meta, &XmpWrite { toolkit: "Editor", namespace: Some(LC), properties })
}

fn parse_xmp(s: &str) -> Result<XmpData, XmpError> {
    parse_xmp_with(s, &[LC])
}

fn settings(d: &XmpData) -> Option<&str> {
    d.properties.get("lc:settings").and_then(|v| v.first()).map(String::as_str)
}

fn full() -> Metadata {
    Metadata {
        // write_xmp doesn't emit regions yet, so the
        // roundtrip fixture must leave this empty or `roundtrip_all_fields` can't round-trip it.
        regions: Vec::new(),
        focus: None,
        make: Some("Maker & Sons".into()),
        model: Some("X <1>".into()),
        serial_number: Some("SN1".into()),
        software: Some("Editor".into()),
        lens_make: Some("L".into()),
        lens_model: Some("24-70mm".into()),
        lens_serial_number: Some("LS".into()),
        lens_spec: Some([24.0, 70.0, 2.8, 2.8]),
        capture_time: DateTime::parse_iso("2022-05-06T07:08:09.123-04:00"),
        exposure_time: Some(1.0 / 125.0),
        f_number: Some(5.6),
        iso: Some(800),
        focal_length: Some(35.0),
        focal_length_35mm: Some(52.0),
        flash: Some(Flash { fired: true, raw: 0x59 }),
        exposure_bias: Some(-1.0 / 3.0),
        exposure_program: Some(2),
        metering_mode: Some(3),
        white_balance: Some(1),
        orientation: Some(Orientation::Rotate270),
        gps: Some(Gps { latitude: 48.858222, longitude: -2.2945, altitude: Some(-3.5) }),
        width: Some(8000),
        height: Some(6000),
        artist: Some("Ann \"Photo\" Lee".into()),
        copyright: Some("© 2022".into()),
        copyright_marked: Some(true),
        usage_terms: Some("Editorial use only; no <resale>".into()),
        copyright_url: Some("https://example.com/licence?a=1&b=2".into()),
        title: Some("Tïtle".into()),
        caption: Some("Line one\nline two".into()),
        alt_text: Some("A tree by the sea".into()),
        extended_description: Some("Long <description>".into()),
        sublocation: Some("Champ de Mars".into()),
        city: Some("Paris".into()),
        state: Some("Île-de-France".into()),
        country: Some("France".into()),
        keywords: vec!["tree".into(), "sky & sea".into()],
        hierarchical_keywords: vec!["Places|France|Paris".into()],
        rating: Some(-1),
        label: Some("Green".into()),
    }
}

#[test]
fn roundtrip_all_fields() {
    let m = full();
    let json = r#"{"version":1,"exposure":0.5,"note":"<&>"}"#;
    let x = write_lc(&m, &[("settings", json)]);
    let d = parse_xmp(&x).unwrap();
    assert_eq!(settings(&d), Some(json));
    let mut got = d.metadata.clone();
    // floats go through rationals
    assert!((got.exposure_bias.unwrap() + 1.0 / 3.0).abs() < 1e-5);
    got.exposure_bias = m.exposure_bias;
    let g = got.gps.unwrap();
    assert!((g.latitude - 48.858222).abs() < 1e-6 && (g.longitude + 2.2945).abs() < 1e-6, "{g:?}");
    got.gps = m.gps;
    assert_eq!(got, m);
}

#[test]
fn empty_metadata_roundtrip() {
    let x = write_xmp(&Metadata::default());
    let d = parse_xmp(&x).unwrap();
    assert_eq!(d.metadata, Metadata::default());
    assert_eq!(settings(&d), None);
}

#[test]
fn foreign_namespace_cannot_impersonate_known_metadata() {
    for body in [
        r#"xmp:Rating="2" lc:settings="foreign settings"/>"#,
        r#"><xmp:Rating>2</xmp:Rating><lc:settings>foreign settings</lc:settings></rdf:Description>"#,
    ] {
        let packet = format!(
            r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description
                xmlns:xmp="https://example.org/vendor/" xmlns:lc="https://example.org/vendor/" {body}</rdf:RDF>"#
        );
        let d = parse_xmp(&packet).unwrap();
        assert_eq!(d.metadata.rating, None, "foreign rating: {packet}");
        assert_eq!(settings(&d), None, "foreign settings: {packet}");
        assert!(!d.properties.contains_key("xmp:Rating"));
        assert!(!d.values.contains_key("lc:settings"));
        assert!(d.properties.values().any(|v| v == &["foreign settings"]));
        assert!(d.properties.keys().all(|key| !key.contains('/')), "opaque names must not create struct paths");
    }
}

#[test]
fn foreign_namespace_names_are_distinct_and_scoped() {
    let packet = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/">
                <xmp:Rating xmlns:xmp="https://example.org/one/">1</xmp:Rating>
                <xmp:Rating xmlns:xmp="https://example.org/two/">2</xmp:Rating>
                <xmp:Rating>4</xmp:Rating>
            </rdf:Description></rdf:RDF>"#;
    let d = parse_xmp(packet).unwrap();
    assert_eq!(d.metadata.rating, Some(4), "a local rebinding must not escape its scope");
    assert_eq!(d.properties.len(), 3, "different foreign URIs must not collide");
    assert_eq!(d.values.len(), 3);
    assert_eq!(d.properties["xmp:Rating"], vec!["4"]);
    assert!(d.properties.values().any(|v| v == &["1"]));
    assert!(d.properties.values().any(|v| v == &["2"]));
}

#[test]
fn attribute_form_foreign_prefixes_and_structs() {
    let x = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description rdf:about="" xmlns:a="http://ns.adobe.com/xap/1.0/" xmlns:d="http://purl.org/dc/elements/1.1/"
               xmlns:e="http://ns.adobe.com/exif/1.0/" a:Rating="4" a:Label="Blue" e:FNumber="40/10" e:GPSLatitude="10,30N" e:GPSLongitude="20,15,36W">
              <d:title><rdf:Alt><rdf:li xml:lang="de">Titel</rdf:li><rdf:li xml:lang="x-default">Title &amp; more &#233;</rdf:li></rdf:Alt></d:title>
              <e:Flash><rdf:Description e:Fired="False" e:Mode="2"/></e:Flash>
              <d:subject><rdf:Bag><rdf:li>k1</rdf:li><rdf:li>k2</rdf:li></rdf:Bag></d:subject>
            </rdf:Description></rdf:RDF></x:xmpmeta>"#;
    let d = parse_xmp(x).unwrap();
    let m = d.metadata;
    assert_eq!(m.rating, Some(4));
    assert_eq!(m.label.as_deref(), Some("Blue"));
    assert_eq!(m.f_number, Some(4.0));
    assert_eq!(m.title.as_deref(), Some("Title & more é"));
    assert_eq!(m.keywords, vec!["k1".to_string(), "k2".to_string()]);
    assert_eq!(m.flash, Some(Flash { fired: false, raw: 0x10 }));
    let g = m.gps.unwrap();
    assert!((g.latitude - 10.5).abs() < 1e-9);
    assert!((g.longitude + 20.26).abs() < 1e-9);
    assert_eq!(d.properties["dc:title"], vec!["Title & more é".to_string(), "Titel".to_string()]);
}

/// Copyright status, usage terms and info URL (XMP Rights Management schema): written as
/// `xmpRights:Marked` (Boolean), `xmpRights:UsageTerms` (Lang Alt) and `xmpRights:WebStatement` (URI).
#[test]
fn rights_management_fields() {
    let m = Metadata { copyright_marked: Some(false), usage_terms: Some("CC0".into()), ..Default::default() };
    let x = write_xmp(&m);
    assert!(x.contains("<xmpRights:Marked>False</xmpRights:Marked>"), "{x}");
    assert!(x.contains("<xmpRights:UsageTerms>\n    <rdf:Alt>\n     <rdf:li xml:lang=\"x-default\">CC0</rdf:li>"), "{x}");
    assert!(!x.contains("WebStatement"), "unset fields are left out");
    assert_eq!(parse_xmp(&x).unwrap().metadata, m);
    // attribute form, other prefixes; anything but True / False is "unknown"
    let read = |marked: &str| {
        let x = format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
                <rdf:Description rdf:about="" xmlns:r="http://ns.adobe.com/xap/1.0/rights/" r:Marked="{marked}" r:WebStatement="https://example.org/c"/>
                </rdf:RDF></x:xmpmeta>"#
        );
        parse_xmp(&x).unwrap().metadata
    };
    assert_eq!(read("True").copyright_marked, Some(true));
    assert_eq!(read("false").copyright_marked, Some(false));
    assert_eq!(read("").copyright_marked, None);
    assert_eq!(read("maybe").copyright_marked, None);
    assert_eq!(read("True").copyright_url.as_deref(), Some("https://example.org/c"));
}

/// A zero or negative `exif:ISOSpeedRatings` is "no ISO" (as in the EXIF reader), not `Some(0)`, and
/// must not hide a valid `exifEX:PhotographicSensitivity`.
#[test]
fn non_positive_iso_is_ignored() {
    let read = |attrs: &str| {
        let x = format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
                <rdf:Description rdf:about="" xmlns:exif="http://ns.adobe.com/exif/1.0/" xmlns:exifEX="http://cipa.jp/exif/1.0/" {attrs}/>
                </rdf:RDF></x:xmpmeta>"#
        );
        parse_xmp(&x).unwrap().metadata.iso
    };
    assert_eq!(read(r#"exif:ISOSpeedRatings="0" exifEX:PhotographicSensitivity="400""#), Some(400));
    assert_eq!(read(r#"exif:ISOSpeedRatings="-100""#), None);
    assert_eq!(read(r#"exif:ISOSpeedRatings="0""#), None);
    assert_eq!(read(r#"exif:ISOSpeedRatings="200""#), Some(200));
}

#[test]
fn negative_xmp_pixel_width_is_ignored() {
    let read = |attrs: &str| {
        let x = format!(
            r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
                <rdf:Description xmlns:tiff="http://ns.adobe.com/tiff/1.0/" xmlns:exif="http://ns.adobe.com/exif/1.0/" {attrs}/>
            </rdf:RDF>"#
        );
        parse_xmp(&x).unwrap().metadata.width
    };

    assert_eq!(read(r#"tiff:ImageWidth="-1""#), None);
    assert_eq!(read(r#"tiff:ImageWidth="-1" exif:PixelXDimension="640""#), Some(640));
    assert_eq!(read(r#"exif:PixelXDimension="-1""#), None);
}

#[test]
fn malformed_inputs() {
    assert!(parse_xmp("<a><b></a>").is_ok() || parse_xmp("<a><b></a>").is_err());
    assert_eq!(parse_xmp("").unwrap().metadata, Metadata::default());
    assert_eq!(parse_xmp("not xml at all").unwrap().metadata, Metadata::default());
    let deep = "<a>".repeat(1000);
    assert_eq!(parse_xmp(&deep), Err(XmpError::TooComplex));
    let x = write_lc(&full(), &[("settings", "{}")]);
    for n in (0..x.len()).step_by(7) {
        if x.is_char_boundary(n) {
            let _ = parse_xmp(&x[..n]);
        }
    }
}

#[test]
fn lc_extras_and_crs_prefix() {
    let x = write_lc(&Metadata::default(), &[("settings", "{\"a\":1}"), ("flag", "pick"), ("bad name", "x")]);
    assert!(!x.contains("xmlns:crs"), "we never write crs:");
    let d = parse_xmp(&x).unwrap();
    assert_eq!(settings(&d), Some("{\"a\":1}"));
    assert_eq!(d.properties["lc:flag"], vec!["pick".to_string()]);
    assert!(!x.contains("bad name"));
    let foreign = r#"<rdf:Description xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
            xmlns:cr="http://ns.adobe.com/camera-raw-settings/1.0/" cr:Exposure2012="+0.5"/>"#;
    assert_eq!(parse_xmp(foreign).unwrap().properties["crs:Exposure2012"], vec!["+0.5".to_string()]);
}

#[test]
fn gps_coords() {
    assert_eq!(parse_gps_coord("45,30N"), Some(45.5));
    assert_eq!(parse_gps_coord("45,30,36S"), Some(-45.51));
    assert_eq!(parse_gps_coord("45X"), None);
    assert_eq!(parse_gps_coord(""), None);
    assert_eq!(parse_gps_coord("é"), None);
    assert_eq!(fmt_gps_coord(-45.51, 'N', 'S'), "45,30.600000S");
    assert_eq!(fmt_gps_coord(10.9999999999, 'N', 'S'), "11,0.000000N");
    assert_eq!(fmt_gps_coord(-0.9999999999, 'E', 'W'), "1,0.000000W");
}

/// Like the EXIF and GPX readers, XMP drops a position outside latitude ±90° / longitude ±180°.
#[test]
fn xmp_gps_out_of_range_rejected() {
    let gps = |lat: &str, lon: &str| {
        let x = format!(
            r#"<rdf:Description xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
                xmlns:e="http://ns.adobe.com/exif/1.0/" e:GPSLatitude="{lat}" e:GPSLongitude="{lon}"/>"#
        );
        parse_xmp(&x).unwrap().metadata.gps
    };
    assert_eq!(gps("95,0N", "200,30E"), None);
    assert_eq!(gps("95,0N", "10,0E"), None);
    assert_eq!(gps("10,0S", "200,30W"), None);
    assert_eq!(gps("90,0,1N", "0,0E"), None);
    assert_eq!(gps("90,0N", "180,0E"), Some(Gps { latitude: 90.0, longitude: 180.0, altitude: None }));
    assert_eq!(gps("90,0S", "180,0W"), Some(Gps { latitude: -90.0, longitude: -180.0, altitude: None }));
}

/// MWG Region Guidelines v2.0, element form (`rdf:parseType="Resource"`, self-closing `Area`) — how
/// Lightroom typically writes it. One named face, one untyped/unnamed region (still usable), one
/// region with a description. `AppliedToDimensions` is present but irrelevant: normalized areas
/// (the default, and the only unit real files use) don't need it.
#[test]
fn mwg_regions_element_form() {
    let x = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description rdf:about=""
                xmlns:mwg-rs="http://www.metadataworkinggroup.com/schemas/regions/"
                xmlns:stArea="http://ns.adobe.com/xmp/sType/Area#"
                xmlns:stDim="http://ns.adobe.com/xap/1.0/sType/Dimensions#">
              <mwg-rs:Regions rdf:parseType="Resource">
                <mwg-rs:AppliedToDimensions rdf:parseType="Resource" stDim:w="4000" stDim:h="3000" stDim:unit="pixel"/>
                <mwg-rs:RegionList>
                  <rdf:Bag>
                    <rdf:li rdf:parseType="Resource">
                      <mwg-rs:Area rdf:parseType="Resource" stArea:x="0.5" stArea:y="0.4" stArea:w="0.2" stArea:h="0.3" stArea:unit="normalized"/>
                      <mwg-rs:Name>Jane Doe</mwg-rs:Name>
                      <mwg-rs:Type>Face</mwg-rs:Type>
                    </rdf:li>
                    <rdf:li rdf:parseType="Resource">
                      <mwg-rs:Area rdf:parseType="Resource" stArea:x="0.1" stArea:y="0.1" stArea:w="0.05" stArea:h="0.05" stArea:unit="normalized"/>
                    </rdf:li>
                    <rdf:li rdf:parseType="Resource">
                      <mwg-rs:Area rdf:parseType="Resource" stArea:x="0.8" stArea:y="0.2" stArea:w="0.1" stArea:h="0.1" stArea:unit="normalized"/>
                      <mwg-rs:Type>Pet</mwg-rs:Type>
                      <mwg-rs:Name>Rex</mwg-rs:Name>
                      <mwg-rs:Description>Good boy</mwg-rs:Description>
                    </rdf:li>
                  </rdf:Bag>
                </mwg-rs:RegionList>
              </mwg-rs:Regions>
            </rdf:Description></rdf:RDF></x:xmpmeta>"#;
    let regions = parse_xmp(x).unwrap().metadata.regions;
    assert_eq!(regions.len(), 3, "{regions:?}");

    assert_eq!(regions[0].kind, RegionKind::Face);
    assert_eq!(regions[0].name.as_deref(), Some("Jane Doe"));
    assert_eq!(regions[0].description, None);
    let r = regions[0].rect;
    assert!((r.x0 - 0.4).abs() < 1e-9 && (r.x1 - 0.6).abs() < 1e-9, "{r:?}");
    assert!((r.y0 - 0.25).abs() < 1e-9 && (r.y1 - 0.55).abs() < 1e-9, "{r:?}");

    // No mwg-rs:Type or mwg-rs:Name at all: still a usable region, kind falls back to `Other("")`.
    assert_eq!(regions[1].kind, RegionKind::Other(String::new()));
    assert_eq!(regions[1].name, None);

    assert_eq!(regions[2].kind, RegionKind::Pet);
    assert_eq!(regions[2].name.as_deref(), Some("Rex"));
    assert_eq!(regions[2].description.as_deref(), Some("Good boy"));
}

/// Lightroom Classic writes a rotated photo's (Exif orientation 3 / 6 / 8) box in the sensor's frame
/// and records the orientation as `mwg-rs:Rotation` (±π / −π/2 / +π/2); the reader returns the
/// upright frame.
#[test]
fn mwg_regions_quarter_turn_rotation_is_mapped_to_the_upright_frame() {
    let x = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description rdf:about=""
                xmlns:mwg-rs="http://www.metadataworkinggroup.com/schemas/regions/"
                xmlns:stArea="http://ns.adobe.com/xmp/sType/Area#">
              <mwg-rs:Regions rdf:parseType="Resource">
                <mwg-rs:RegionList>
                  <rdf:Bag>
                    <rdf:li rdf:parseType="Resource" mwg-rs:Rotation="-1.57080" mwg-rs:Type="Face">
                      <mwg-rs:Area rdf:parseType="Resource" stArea:x="0.3" stArea:y="0.4" stArea:w="0.2" stArea:h="0.3"/>
                    </rdf:li>
                    <rdf:li rdf:parseType="Resource" mwg-rs:Rotation="1.57080" mwg-rs:Type="Face">
                      <mwg-rs:Area rdf:parseType="Resource" stArea:x="0.3" stArea:y="0.4" stArea:w="0.2" stArea:h="0.3"/>
                    </rdf:li>
                    <rdf:li rdf:parseType="Resource" mwg-rs:Rotation="0.00000" mwg-rs:Type="Face">
                      <mwg-rs:Area rdf:parseType="Resource" stArea:x="0.3" stArea:y="0.4" stArea:w="0.2" stArea:h="0.3"/>
                    </rdf:li>
                    <rdf:li rdf:parseType="Resource" mwg-rs:Rotation="3.14159" mwg-rs:Type="Face">
                      <mwg-rs:Area rdf:parseType="Resource" stArea:x="0.3" stArea:y="0.4" stArea:w="0.2" stArea:h="0.3"/>
                    </rdf:li>
                    <rdf:li rdf:parseType="Resource" mwg-rs:Rotation="-3.14159" mwg-rs:Type="Face">
                      <mwg-rs:Area rdf:parseType="Resource" stArea:x="0.3" stArea:y="0.4" stArea:w="0.2" stArea:h="0.3"/>
                    </rdf:li>
                  </rdf:Bag>
                </mwg-rs:RegionList>
              </mwg-rs:Regions>
            </rdf:Description></rdf:RDF></x:xmpmeta>"#;
    let regions = parse_xmp(x).unwrap().metadata.regions;
    assert_eq!(regions.len(), 5, "{regions:?}");
    let near = |r: Rect, b: [f64; 4]| [r.x0 - b[0], r.y0 - b[1], r.x1 - b[2], r.y1 - b[3]].iter().all(|d| d.abs() < 1e-9);
    assert!(near(regions[0].rect, [0.45, 0.2, 0.75, 0.4]), "−π/2 (orientation 6): {:?}", regions[0].rect);
    assert!(near(regions[1].rect, [0.25, 0.6, 0.55, 0.8]), "+π/2 (orientation 8): {:?}", regions[1].rect);
    assert!(near(regions[2].rect, [0.2, 0.25, 0.4, 0.55]), "0 is untouched: {:?}", regions[2].rect);
    assert!(near(regions[3].rect, [0.6, 0.45, 0.8, 0.75]), "π (orientation 3): {:?}", regions[3].rect);
    assert!(near(regions[4].rect, [0.6, 0.45, 0.8, 0.75]), "−π (orientation 3): {:?}", regions[4].rect);
}

/// The size a packet's boxes were drawn on is its `AppliedToDimensions`, unless a box says how the photo was turned
/// (Lightroom's `mwg-rs:Rotation`: those are upright already) or there is no size to go by.
#[test]
fn regions_drawn_on_is_the_applied_to_size_unless_a_box_says_how_it_was_turned() {
    let packet = |dims: &str, rotation: &str| {
        format!(
            r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description rdf:about="" xmlns:mwg-rs="http://www.metadataworkinggroup.com/schemas/regions/"
                xmlns:stArea="http://ns.adobe.com/xmp/sType/Area#" xmlns:stDim="http://ns.adobe.com/xap/1.0/sType/Dimensions#">
              <mwg-rs:Regions rdf:parseType="Resource">{dims}
                <mwg-rs:RegionList><rdf:Bag>
                  <rdf:li rdf:parseType="Resource" mwg-rs:Type="Face">
                    <mwg-rs:Area rdf:parseType="Resource" stArea:x="0.3" stArea:y="0.4" stArea:w="0.2" stArea:h="0.3"/>
                  </rdf:li>
                  <rdf:li rdf:parseType="Resource" mwg-rs:Type="Face"{rotation}>
                    <mwg-rs:Area rdf:parseType="Resource" stArea:x="0.6" stArea:y="0.4" stArea:w="0.2" stArea:h="0.3"/>
                  </rdf:li>
                </rdf:Bag></mwg-rs:RegionList>
              </mwg-rs:Regions>
            </rdf:Description></rdf:RDF>"#
        )
    };
    let dims = r#"<mwg-rs:AppliedToDimensions rdf:parseType="Resource" stDim:w="6000" stDim:h="4000" stDim:unit="pixel"/>"#;
    let drawn_on = |x: &str| parse_xmp(x).unwrap().regions_drawn_on();
    assert_eq!(drawn_on(&packet(dims, "")), Some((6000.0, 4000.0)));
    assert_eq!(drawn_on(&packet(dims, r#" mwg-rs:Rotation="-1.57080""#)), None, "one box turned by Lightroom");
    assert_eq!(drawn_on(&packet("", "")), None, "no AppliedToDimensions");
    assert_eq!(drawn_on(r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about=""/></rdf:RDF>"#), None);
}

/// A second, differently-shaped encoding: each region is its own `rdf:Description` (rather than an
/// `rdf:li rdf:parseType="Resource"`), `mwg-rs:Type` as an attribute instead of an element, and an
/// unrecognised `mwg-rs:Type` value kept verbatim.
#[test]
fn mwg_regions_attribute_form_and_unknown_type() {
    let x = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description rdf:about=""
                xmlns:mwg-rs="http://www.metadataworkinggroup.com/schemas/regions/"
                xmlns:stArea="http://ns.adobe.com/xmp/sType/Area#">
              <mwg-rs:Regions rdf:parseType="Resource">
                <mwg-rs:RegionList>
                  <rdf:Bag>
                    <rdf:li>
                      <rdf:Description mwg-rs:Type="BarCode">
                        <mwg-rs:Area stArea:x="0.3" stArea:y="0.3" stArea:w="0.1" stArea:h="0.1"/>
                      </rdf:Description>
                    </rdf:li>
                    <rdf:li>
                      <rdf:Description mwg-rs:Type="Sticker">
                        <mwg-rs:Area stArea:x="0.6" stArea:y="0.6" stArea:w="0.1" stArea:h="0.1"/>
                      </rdf:Description>
                    </rdf:li>
                  </rdf:Bag>
                </mwg-rs:RegionList>
              </mwg-rs:Regions>
            </rdf:Description></rdf:RDF>"#;
    let regions = parse_xmp(x).unwrap().metadata.regions;
    assert_eq!(regions.len(), 2, "{regions:?}");
    assert_eq!(regions[0].kind, RegionKind::BarCode);
    assert_eq!(regions[1].kind, RegionKind::Other("Sticker".to_string()));
}

/// Hostile/malformed regions are dropped individually, never panic, and never poison the rest of
/// the list: missing Area, non-finite or absurd numbers, zero/negative size, and a pixel-unit area
/// with no `AppliedToDimensions` to convert it (so it can't be stored as a nonsense fraction).
#[test]
fn mwg_regions_hostile_inputs_are_dropped_not_panicking() {
    let x = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description rdf:about=""
                xmlns:mwg-rs="http://www.metadataworkinggroup.com/schemas/regions/"
                xmlns:stArea="http://ns.adobe.com/xmp/sType/Area#">
              <mwg-rs:Regions rdf:parseType="Resource">
                <mwg-rs:RegionList>
                  <rdf:Bag>
                    <rdf:li rdf:parseType="Resource"><mwg-rs:Name>No Area</mwg-rs:Name></rdf:li>
                    <rdf:li rdf:parseType="Resource"><mwg-rs:Area rdf:parseType="Resource" stArea:x="NaN" stArea:y="0.1" stArea:w="0.1" stArea:h="0.1"/></rdf:li>
                    <rdf:li rdf:parseType="Resource"><mwg-rs:Area rdf:parseType="Resource" stArea:x="1e300" stArea:y="0.1" stArea:w="0.1" stArea:h="0.1"/></rdf:li>
                    <rdf:li rdf:parseType="Resource"><mwg-rs:Area rdf:parseType="Resource" stArea:x="0.5" stArea:y="0.5" stArea:w="0" stArea:h="0.1"/></rdf:li>
                    <rdf:li rdf:parseType="Resource"><mwg-rs:Area rdf:parseType="Resource" stArea:x="0.5" stArea:y="0.5" stArea:w="-0.1" stArea:h="0.1"/></rdf:li>
                    <rdf:li rdf:parseType="Resource"><mwg-rs:Area rdf:parseType="Resource" stArea:x="100" stArea:y="100" stArea:w="50" stArea:h="50" stArea:unit="pixel"/></rdf:li>
                    <rdf:li rdf:parseType="Resource"><mwg-rs:Area rdf:parseType="Resource" stArea:x="0.5" stArea:y="0.5" stArea:w="0.2" stArea:h="0.2"/><mwg-rs:Type>Face</mwg-rs:Type></rdf:li>
                  </rdf:Bag>
                </mwg-rs:RegionList>
              </mwg-rs:Regions>
            </rdf:Description></rdf:RDF>"#;
    let regions = parse_xmp(x).unwrap().metadata.regions;
    assert_eq!(regions.len(), 1, "only the last, well-formed region should survive: {regions:?}");
    assert_eq!(regions[0].kind, RegionKind::Face);
}

/// Regions are clipped to the photo frame, and ones that miss the photo entirely are dropped: a
/// hostile sidecar can't park a box at x=1e5 (or a 1e5-wide one) that the loupe or the People view
/// would then draw or crop far outside the image.
#[test]
fn mwg_regions_are_clipped_to_the_frame_and_off_photo_ones_dropped() {
    let x = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description rdf:about=""
                xmlns:mwg-rs="http://www.metadataworkinggroup.com/schemas/regions/"
                xmlns:stArea="http://ns.adobe.com/xmp/sType/Area#">
              <mwg-rs:Regions rdf:parseType="Resource">
                <mwg-rs:RegionList>
                  <rdf:Bag>
                    <rdf:li rdf:parseType="Resource"><mwg-rs:Area rdf:parseType="Resource" stArea:x="1e5" stArea:y="0.5" stArea:w="0.1" stArea:h="0.1"/><mwg-rs:Name>Far right</mwg-rs:Name></rdf:li>
                    <rdf:li rdf:parseType="Resource"><mwg-rs:Area rdf:parseType="Resource" stArea:x="-3" stArea:y="-3" stArea:w="0.5" stArea:h="0.5"/><mwg-rs:Name>Far left</mwg-rs:Name></rdf:li>
                    <rdf:li rdf:parseType="Resource"><mwg-rs:Area rdf:parseType="Resource" stArea:x="1.2" stArea:y="0.5" stArea:w="0.2" stArea:h="0.2"/><mwg-rs:Name>Touching edge</mwg-rs:Name></rdf:li>
                    <rdf:li rdf:parseType="Resource"><mwg-rs:Area rdf:parseType="Resource" stArea:x="0.5" stArea:y="0.5" stArea:w="1e5" stArea:h="1e5"/><mwg-rs:Name>Huge</mwg-rs:Name></rdf:li>
                    <rdf:li rdf:parseType="Resource"><mwg-rs:Area rdf:parseType="Resource" stArea:x="0.95" stArea:y="0.05" stArea:w="0.2" stArea:h="0.2"/><mwg-rs:Name>Corner</mwg-rs:Name></rdf:li>
                  </rdf:Bag>
                </mwg-rs:RegionList>
              </mwg-rs:Regions>
            </rdf:Description></rdf:RDF>"#;
    let regions = parse_xmp(x).unwrap().metadata.regions;
    let names: Vec<_> = regions.iter().map(|r| r.name.as_deref().unwrap_or("")).collect();
    assert_eq!(names, ["Huge", "Corner"], "{regions:?}");
    for r in &regions {
        let q = r.rect;
        assert!(q.x0 >= 0.0 && q.y0 >= 0.0 && q.x1 <= 1.0 && q.y1 <= 1.0 && !q.is_empty(), "{q:?}");
    }
    assert_eq!(regions[0].rect, Rect::UNIT);
    let c = regions[1].rect;
    assert!((c.x0 - 0.85).abs() < 1e-9 && c.x1 == 1.0 && c.y0 == 0.0 && (c.y1 - 0.15).abs() < 1e-9, "{c:?}");
}

/// `extract()`'s precedence (EXIF/IPTC never carry regions; XMP is read and filled in like keywords).
#[test]
fn regions_merge_like_keywords() {
    let with_region = Region { rect: Rect::from_center(Point::new(0.5, 0.5), 0.2, 0.2), kind: RegionKind::Face, name: None, description: None };
    let mut a = Metadata::default();
    let b = Metadata { regions: vec![with_region.clone()], ..Default::default() };
    a.fill_missing(&b);
    assert_eq!(a.regions, vec![with_region.clone()]);

    let mut a = Metadata::default();
    a.overlay_user_fields(&b);
    assert_eq!(a.regions, vec![with_region]);
}

#[test]
fn writer_drops_characters_xml_1_0_forbids() {
    let bad = "a\u{1}b\u{0}c\u{FFFE}d\u{FFFF}e\u{1F}\tf\ng\rh";
    let m = Metadata { make: Some(bad.into()), caption: Some(bad.into()), keywords: vec![bad.into()], ..Default::default() };
    let x = write_lc(&m, &[("settings", bad)]);
    let illegal: Vec<char> =
        x.chars().filter(|&c| (c < '\u{20}' && !matches!(c, '\t' | '\n' | '\r')) || c == '\u{FFFE}' || c == '\u{FFFF}').collect();
    assert!(illegal.is_empty(), "illegal XML chars in output: {illegal:?}");
    assert!(x.contains(">abcde\tf\ng\rh<"), "allowed characters must survive: {x}");
}

#[test]
fn numeric_references_to_forbidden_characters_are_not_decoded() {
    assert_eq!(entity("#1"), None);
    assert_eq!(entity("#0"), None);
    assert_eq!(entity("#xFFFE"), None);
    assert_eq!(entity("#x41"), Some('A'));
    assert_eq!(entity("#9"), Some('\t'));
}

/// Flattened keys repeat their whole ancestor path, so a long nested name with a wide child
/// fan-out would otherwise retain its prefix once per leaf: the cumulative key budget stops it
/// (a 65 KB document with 3000 leaves must not retain ~2.4 MB of key text).
#[test]
fn flattened_keys_stay_within_the_cumulative_budget() {
    let level = "n".repeat(200);
    let mut doc = String::from(
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\" xmlns:lc=\"https://vendor.example/lc/\"><rdf:RDF><rdf:Description><lc:root rdf:parseType=\"Resource\">",
    );
    for _ in 0..4 {
        doc.push_str(&format!("<lc:{level} rdf:parseType=\"Resource\">"));
    }
    // each leaf has a name of its own, so each one adds a key
    for i in 0..3000 {
        doc.push_str(&format!("<lc:leaf{i} lc:att=\"v\"/>"));
    }
    doc.push_str("</lc:root>");
    for _ in 0..4 {
        doc.push_str(&format!("</lc:{level}>"));
    }
    doc.push_str("</rdf:Description></rdf:RDF></x:xmpmeta>");
    let d = parse_xmp(&doc).unwrap();
    let total: usize = d.properties.keys().map(String::len).sum();
    assert!(total <= MAX_KEY_BYTES, "flattened-key text {total} is over the {MAX_KEY_BYTES} budget");
    assert!(d.properties.len() < 3000, "keys stop being added once the budget is spent");
    assert!(!d.properties.is_empty(), "properties parsed before the budget are kept");
}

#[test]
fn zero_padded_numeric_references_decode() {
    assert_eq!(unescape_attr("C&#0000000097;non &#x0000041; &#000000065;"), "Canon A A");
    assert_eq!(unescape_attr("&abc&#xyz;&;"), "&abc&#xyz;&;");
}

#[test]
fn entity_rejects_signed_numeric_references() {
    assert_eq!(entity("#+65"), None);
    assert_eq!(entity("#x+41"), None);
    assert_eq!(entity("#65"), Some('A'));
    assert_eq!(entity("#x41"), Some('A'));
}

#[test]
fn malformed_capture_time_falls_back_and_keeps_raw_property() {
    let packet = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:exif="http://ns.adobe.com/exif/1.0/" xmlns:xmp="http://ns.adobe.com/xap/1.0/" exif:DateTimeOriginal="2026-01-01T" xmp:CreateDate="2026-01-02T10:00:00Z"/></rdf:RDF></x:xmpmeta>"#;
    let parsed = parse_xmp(packet).unwrap();
    assert_eq!(
        parsed.metadata.capture_time.map(|d| (d.year, d.month, d.day, d.hour, d.minute, d.second, d.offset_minutes)),
        Some((2026, 1, 2, 10, 0, 0, Some(0)))
    );
    assert_eq!(parsed.properties["exif:DateTimeOriginal"], vec!["2026-01-01T".to_string()]);
}

#[test]
fn malformed_capture_time_without_fallback_is_not_projected() {
    let packet = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:exif="http://ns.adobe.com/exif/1.0/" exif:DateTimeOriginal="2026-01-01T"/></rdf:RDF></x:xmpmeta>"#;
    let parsed = parse_xmp(packet).unwrap();
    assert_eq!(parsed.metadata.capture_time, None);
    assert_eq!(parsed.properties["exif:DateTimeOriginal"], vec!["2026-01-01T".to_string()]);
}

#[test]
fn many_ampersands_before_one_semicolon_are_read_once() {
    // 200 000 `&` then a `;`: each `&` once searched the whole rest of the attribute for the `;`
    let hostile = format!("{};&amp;", "&".repeat(200_000));
    assert_eq!(unescape_attr(&hostile), format!("{};&", "&".repeat(200_000)));
    assert_eq!(unescape_attr("&lt;a&gt; &#x10FFFF; &unknownreference;"), "<a> \u{10FFFF} &unknownreference;");
}

/// Children that add no key (empty structs) still had their whole ancestor path copied, each,
/// after the budget was spent: a 250 KB path under 100 000 such children was 25 GB of copying.
#[test]
fn paths_are_not_built_past_the_key_budget() {
    let level = "n".repeat(60_000);
    let mut doc = String::from(
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\" xmlns:lc=\"https://vendor.example/lc/\"><rdf:RDF><rdf:Description>",
    );
    for _ in 0..4 {
        doc.push_str(&format!("<lc:{level} rdf:parseType=\"Resource\">"));
    }
    for _ in 0..100_000 {
        doc.push_str("<lc:e rdf:parseType=\"Resource\"/>");
    }
    for _ in 0..4 {
        doc.push_str(&format!("</lc:{level}>"));
    }
    doc.push_str("</rdf:Description></rdf:RDF></x:xmpmeta>");
    let t = std::time::Instant::now();
    let _ = parse_xmp(&doc);
    assert!(t.elapsed().as_secs() < 10, "took {:?}", t.elapsed());
}

/// The caller's namespace is declared where the read-only `crs` sits in the table, is known to the
/// reader only when passed, and is canonicalised by its URI whatever prefix a packet binds it to.
#[test]
fn the_callers_namespace_is_written_and_read_only_when_passed() {
    let x = write_lc(&Metadata::default(), &[("flag", "pick")]);
    assert!(x.contains("x:xmptk=\"Editor\""));
    assert!(x.contains(
        "xmlns:Iptc4xmpCore=\"http://iptc.org/std/Iptc4xmpCore/1.0/xmlns/\"\n    xmlns:lc=\"http://ns.example.com/lc/1.0/\"\n    xmlns:mwg-rs="
    ));
    assert!(!x.contains("xmlns:crs"));
    let renamed = x.replace("xmlns:lc=", "xmlns:zz=").replace("lc:flag", "zz:flag");
    assert_eq!(parse_xmp_with(&renamed, &[LC]).unwrap().properties["lc:flag"], vec!["pick".to_string()]);
    assert!(!crate::parse_xmp(&renamed).unwrap().properties.contains_key("lc:flag"));

    let plain = write_xmp(&Metadata::default());
    assert!(plain.contains("x:xmptk=\"craft-metadata\"") && !plain.contains("xmlns:lc") && !plain.contains("xmlns:crs"));
    // properties without a namespace to put them in are not written
    let orphan = write_xmp_with(&Metadata::default(), &XmpWrite { properties: &[("flag", "pick")], ..Default::default() });
    assert_eq!(orphan, plain);
}

#[test]
fn resource_struct_attributes_and_elements_flatten_like_description_fields() {
    let forms = [
        r#"<c:Parameters rdf:parseType="Resource" c:ConvertToGrayscale="True"><c:Version>1</c:Version></c:Parameters>"#,
        r#"<c:Parameters><rdf:Description c:ConvertToGrayscale="True"><c:Version>1</c:Version></rdf:Description></c:Parameters>"#,
        r#"<c:Parameters rdf:parseType="Resource"><c:ConvertToGrayscale>True</c:ConvertToGrayscale><c:Version>1</c:Version></c:Parameters>"#,
        r#"<c:Parameters rdf:parseType="Resource" c:ConvertToGrayscale="True" c:Version="1"/>"#,
    ];
    let mut previous = None;
    for form in forms {
        let xml = format!(
            r#"<rdf:Description xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:c="http://ns.adobe.com/camera-raw-settings/1.0/"><c:Look rdf:parseType="Resource">{form}</c:Look></rdf:Description>"#
        );
        let packet = parse_xmp(&xml).unwrap();
        assert_eq!(packet.properties["crs:Look/crs:Parameters/crs:ConvertToGrayscale"], vec!["True"]);
        assert_eq!(packet.properties["crs:Look/crs:Parameters/crs:Version"], vec!["1"]);
        assert!(packet.properties.keys().all(|key| !key.contains("rdf:")));
        if let Some((properties, values)) = previous {
            assert_eq!(packet.properties, properties);
            assert_eq!(packet.values, values);
        }
        previous = Some((packet.properties, packet.values));
    }
}
