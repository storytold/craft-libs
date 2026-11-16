//! Link annotations, destinations and page boxes whose geometry has no finite place on the
//! page make `Writer::finish` return an error instead of panicking (krilla patch 13).
#![cfg(feature = "write")]
#![allow(clippy::unwrap_used, clippy::panic)]

use backend::action::LinkAction;
use backend::annotation::{LinkAnnotation, Target};
use backend::destination::XyzDestination;
use backend::error::KrillaError;
use backend::geom::{Point, Quadrilateral, Rect};
use backend::page::PageSettings;
use craft_pdf::write::{Metadata, WriteError, Writer, backend};

fn link() -> Target {
    Target::Action(LinkAction::new("https://example.com".into()).into())
}

fn quad(x: f32, y: f32) -> Quadrilateral {
    Quadrilateral([Point::from_xy(x, y); 4])
}

fn write(settings: PageSettings, annotation: Option<LinkAnnotation>) -> Result<Vec<u8>, KrillaError> {
    let mut writer = Writer::new(Default::default(), Metadata::default());
    let mut page = writer.document_mut().start_page_with(settings);
    if let Some(annotation) = annotation {
        page.add_annotation(annotation.into());
    }
    page.finish();
    match writer.finish() {
        Ok(pdf) => Ok(pdf),
        Err(WriteError::Serialization(error)) => Err(*error),
        Err(error) => panic!("unexpected error: {error}"),
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn unplaceable_geometry_is_an_error() {
    let page = || PageSettings::from_wh(40.0, 20.0).unwrap();
    let big_page = || PageSettings::from_wh(40.0, 2e38).unwrap();

    let ok = write(page(), Some(LinkAnnotation::new_with_quad_points(vec![quad(1.0, 2.0)], link())));
    assert!(ok.is_ok(), "{ok:?}");

    let quads = [vec![], vec![quad(f32::NAN, f32::NAN)], vec![quad(1.0, 2.0), quad(f32::INFINITY, 2.0)], vec![quad(-3e38, 0.0), quad(3e38, 0.0)]];
    for (case, quads) in quads.into_iter().enumerate() {
        let result = write(page(), Some(LinkAnnotation::new_with_quad_points(quads, link())));
        assert!(matches!(result, Err(KrillaError::InvalidAnnotationGeometry(None))), "case {case}: {result:?}");
    }

    // Finite, but 2e38 + 2e38 overflows when flipped into PDF coordinates.
    let far = Rect::from_ltrb(0.0, -2e38, 10.0, -1e38).unwrap();
    let result = write(big_page(), Some(LinkAnnotation::new(far, link())));
    assert!(matches!(result, Err(KrillaError::InvalidAnnotationGeometry(None))), "{result:?}");

    let result = write(big_page().with_crop_box(Some(far)), None);
    assert!(matches!(result, Err(KrillaError::InvalidPageBox(0))), "{result:?}");

    let missing_page = Target::Destination(XyzDestination::new(5, Point::from_xy(1.0, 1.0)).into());
    let result = write(page(), Some(LinkAnnotation::new(Rect::from_xywh(0.0, 0.0, 5.0, 5.0).unwrap(), missing_page)));
    assert!(matches!(result, Err(KrillaError::InvalidDestination(5))), "{result:?}");

    let far_point = Target::Destination(XyzDestination::new(0, Point::from_xy(1.0, -3e38)).into());
    let result = write(big_page(), Some(LinkAnnotation::new(Rect::from_xywh(0.0, 0.0, 5.0, 5.0).unwrap(), far_point)));
    assert!(matches!(result, Err(KrillaError::InvalidDestination(0))), "{result:?}");
}
