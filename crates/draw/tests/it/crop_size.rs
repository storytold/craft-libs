//! The W × H × Resolution rules both crop bars share: the pixel target the fields name, and Front Image with the
//! Perspective limits.

use craft_behavior::kurbo::Size;
use craft_draw::ui::crop::{CropDocKey, CropDocument, CropLength, CropRatio, CropSettings, CropSizeFields, ResolutionUnit};
use craft_ui::runtime::env::{Env, PointSize, UnitKind, Units, with_env};
use craft_ui::testing::{Frames, trial};

/// The units of a scope whose point is `point_size`.
fn units(point_size: PointSize) -> Units {
    let mut frames = Frames::new();
    let mut units = None;
    frames.frame(vec![], |ui| {
        with_env(
            ui,
            |scope| {
                scope.values.set(point_size);
            },
            |ui| units = Some(Env::of(ui).units()),
        )
    });
    units.expect("the frame ran")
}

fn document(width: f64, height: f64, resolution: f64) -> CropDocument {
    CropDocument::new(CropDocKey::new(1), Size::new(width, height), resolution)
}

fn fields(width: Option<(f64, UnitKind)>, height: Option<(f64, UnitKind)>, resolution: &str, unit: ResolutionUnit) -> CropSizeFields {
    CropSizeFields {
        width: width.map(|(value, unit)| CropLength::new(value, unit)),
        height: height.map(|(value, unit)| CropLength::new(value, unit)),
        resolution: resolution.to_owned(),
        resolution_unit: unit,
        ..Default::default()
    }
}

fn near(value: Option<f64>, expected: f64) -> bool {
    value.is_some_and(|v| (v - expected).abs() < 1e-6)
}

#[trial]
fn size_targets_convert_units_and_perspective_limits_round_inward() {
    let postscript = units(PointSize::PostScript);
    let traditional = units(PointSize::Traditional);
    let doc = document(800.0, 600.0, 72.0);
    let inch = |value: f64| Some((value, UnitKind::Inches));

    // Inches convert at the typed resolution, else at the document's.
    let typed = fields(inch(4.0), inch(5.0), "300", ResolutionUnit::PixelsPerInch).target(&doc, &postscript);
    assert!(near(typed.width, 1200.0) && near(typed.height, 1500.0) && near(typed.resolution, 300.0), "{typed:?}");
    let untyped = fields(inch(2.0), inch(1.0), "", ResolutionUnit::PixelsPerInch).target(&doc, &postscript);
    assert!(near(untyped.width, 144.0) && near(untyped.height, 72.0) && untyped.resolution.is_none(), "{untyped:?}");

    // Pixels ignore the resolution; centimetres and the px/cm resolution convert through 2.54.
    let pixels =
        fields(Some((100.0, UnitKind::Pixels)), Some((50.0, UnitKind::Pixels)), "300", ResolutionUnit::PixelsPerInch).target(&doc, &postscript);
    assert!(near(pixels.width, 100.0) && near(pixels.height, 50.0), "{pixels:?}");
    let centimetres = fields(Some((2.54, UnitKind::Centimetres)), inch(1.0), "10", ResolutionUnit::PixelsPerCentimetre).target(&doc, &postscript);
    assert!(near(centimetres.resolution, 25.4) && near(centimetres.width, 25.4) && near(centimetres.height, 25.4), "{centimetres:?}");

    // A point is 1/72 in for PostScript and 1/72.27 in for Traditional.
    let points = fields(Some((72.0, UnitKind::Points)), None, "100", ResolutionUnit::PixelsPerInch);
    assert!(near(points.target(&doc, &postscript).width, 100.0));
    assert!(near(points.target(&doc, &traditional).width, 100.0 * 72.0 / 72.27));

    // A side that is empty, zero or past the largest length has no target; the other side keeps its own.
    let one_side = fields(inch(4.0), None, "", ResolutionUnit::PixelsPerInch).target(&doc, &postscript);
    assert!(near(one_side.width, 288.0) && one_side.height.is_none(), "{one_side:?}");
    let zero = fields(Some((0.0, UnitKind::Pixels)), Some((1e12, UnitKind::Pixels)), "", ResolutionUnit::PixelsPerInch).target(&doc, &postscript);
    assert!(zero.width.is_none() && zero.height.is_none(), "{zero:?}");

    // The settings' target needs the W × H × Resolution mode and both sides.
    let mut settings = CropSettings { size: fields(inch(4.0), inch(5.0), "300", ResolutionUnit::PixelsPerInch), ..Default::default() };
    assert!(settings.target(&doc, &postscript).is_none(), "a ratio that is not W x H x Resolution has no target");
    settings.ratio = CropRatio::Size;
    let target = settings.target(&doc, &postscript).expect("both sides");
    assert!((target.width - 1200.0).abs() < 1e-6 && (target.height - 1500.0).abs() < 1e-6 && near(target.resolution, 300.0));
    settings.size.height = None;
    assert!(settings.target(&doc, &postscript).is_none(), "one side is a free crop");

    // Front Image with the Perspective limits keeps three decimals and rounds inward.
    let high = document(800.0, 600.0, 30_000.0);
    let mut per_inch = CropSizeFields::default();
    per_inch.front_image(&high, Some((1.0, 29_999.999)));
    assert_eq!(per_inch.resolution, "29999.999");
    assert_eq!(per_inch.width, Some(CropLength::new(800.0, UnitKind::Pixels)));
    assert_eq!(per_inch.height, Some(CropLength::new(600.0, UnitKind::Pixels)));
    let mut per_cm = CropSizeFields { resolution_unit: ResolutionUnit::PixelsPerCentimetre, ..Default::default() };
    per_cm.front_image(&high, Some((0.394, 11_811.023)));
    assert_eq!(per_cm.resolution, "11811.023", "30000 / 2.54 rounds to 11811.024, past the limit");
    assert!(per_cm.resolution_ppi().is_some_and(|ppi| ppi <= 29_999.999), "the stored value converts back inside the limit");
    // Without limits the resolution keeps two decimals.
    let mut plain = CropSizeFields::default();
    plain.front_image(&document(10.0, 10.0, 300.456), None);
    assert_eq!(plain.resolution, "300.46");

    // Clear empties the three fields and puts the unit back to px/in; swap exchanges W and H.
    let mut cleared = fields(inch(4.0), inch(5.0), "300", ResolutionUnit::PixelsPerCentimetre);
    cleared.swap();
    assert_eq!((cleared.width, cleared.height), (Some(CropLength::new(5.0, UnitKind::Inches)), Some(CropLength::new(4.0, UnitKind::Inches))));
    cleared.clear_size();
    assert_eq!(cleared, CropSizeFields::default());
}
