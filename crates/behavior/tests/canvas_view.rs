//! The canvas view's mapping between document and screen.

use craft_behavior::CanvasView;
use craft_behavior::canvas::{ZoomScheme, zoom};
use emath::{Rect as ScreenRect, pos2, vec2};
use kurbo::{Point, Rect, Size};

fn view() -> CanvasView {
    CanvasView::new(ScreenRect::from_min_max(pos2(40.0, 30.0), pos2(840.0, 630.0)), Point::new(500.0, 400.0), 0.75)
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn a_document_point_survives_the_trip_to_the_screen_and_back() {
    let views = [
        view(),
        view().rotation(33.0),
        view().rotation(-120.0).flip_x(true),
        view().aspect(2.0).flip_x(true),
        view().rotation(90.0).aspect(0.5),
        view().rotation(217.5).flip_x(true).aspect(1.5),
    ];
    for view in views {
        for p in [Point::new(0.0, 0.0), Point::new(512.0, 300.0), Point::new(-40.0, 1200.0)] {
            let back = view.to_doc(view.to_screen(p));
            assert!(near(back.x, p.x) && near(back.y, p.y), "{p:?} came back as {back:?} through {view:?}");
        }
    }
}

#[test]
fn quarter_turns_map_a_rect_to_an_axis_aligned_quad() {
    let doc = Rect::new(10.0, 20.0, 310.0, 220.0);
    for turns in 0..8 {
        for flip in [false, true] {
            let quad = view().rotation(90.0 * f64::from(turns)).flip_x(flip).aspect(1.25).quad(doc);
            for i in 0..4 {
                let (a, b) = (quad[i], quad[(i + 1) % 4]);
                assert!(a.x == b.x || a.y == b.y, "edge {i} of turn {turns} (flip {flip}) is slanted: {a:?} to {b:?}");
            }
        }
    }
    let turned = view().rotation(30.0).quad(doc);
    assert!(turned[0].x != turned[1].x && turned[0].y != turned[1].y, "a 30 degree turn slants the edges");
}

#[test]
fn fit_and_fill_measure_the_stretched_document() {
    let size = Size::new(1000.0, 500.0);
    let rect = |w: f32, h: f32| ScreenRect::from_min_size(pos2(0.0, 0.0), vec2(w, h));
    let stage = |aspect: f64, rect: ScreenRect| CanvasView::new(rect, Point::new(0.0, 0.0), 1.0).aspect(aspect);

    // Fit keeps 20 points of margin per side: 1000 points across for a 1000 px wide document, 2000 shown at aspect 2.
    let (mut square, mut wide) = (stage(1.0, rect(1040.0, 800.0)), stage(2.0, rect(1040.0, 800.0)));
    zoom::fit(&mut square, size, 1.0, 20.0, &ZoomScheme::PHOTOSHOP);
    zoom::fit(&mut wide, size, 1.0, 20.0, &ZoomScheme::PHOTOSHOP);
    assert!(near(square.zoom, 1.0), "{}", square.zoom);
    assert!(near(wide.zoom, 0.5), "{}", wide.zoom);
    assert!(near(wide.center.x, 500.0) && near(wide.center.y, 250.0), "{:?}", wide.center);

    // On a square rect the height limits fill at both aspects; the 1000 by 400 rect below is where the stretched width decides.
    let (mut square, mut wide) = (stage(1.0, rect(1000.0, 1000.0)), stage(2.0, rect(1000.0, 1000.0)));
    zoom::fill(&mut square, size, 1.0, &ZoomScheme::PHOTOSHOP);
    zoom::fill(&mut wide, size, 1.0, &ZoomScheme::PHOTOSHOP);
    assert!(near(square.zoom, 2.0) && near(wide.zoom, 2.0), "{} {}", square.zoom, wide.zoom);
    // A wide rect: the unstretched document is limited by its width, the stretched one by its height.
    let (mut square, mut wide) = (stage(1.0, rect(1000.0, 400.0)), stage(2.0, rect(1000.0, 400.0)));
    zoom::fill(&mut square, size, 1.0, &ZoomScheme::PHOTOSHOP);
    zoom::fill(&mut wide, size, 1.0, &ZoomScheme::PHOTOSHOP);
    assert!(near(square.zoom, 1.0), "{}", square.zoom);
    assert!(near(wide.zoom, 0.8), "{}", wide.zoom);
    assert!(near(wide.center.x, 500.0) && near(wide.center.y, 250.0), "{:?}", wide.center);
}

#[test]
fn a_scheme_steps_and_clamps_in_its_own_units_about_its_hundred() {
    let (small, big) = (Size::new(100.0, 100.0), Size::new(10_000.0, 10_000.0));

    let illustrator = ZoomScheme::ILLUSTRATOR;
    let up = illustrator.step(1.0, 1, big);
    assert!(near(up, 1.5), "{up}");
    assert!(near(illustrator.step(up, -1, big), 1.0), "a step in then out returns to the rung");
    assert!(near(illustrator.min(small), 0.0313) && near(illustrator.min(big), 0.0313), "a fixed floor ignores the document");

    let photoshop = ZoomScheme::PHOTOSHOP;
    assert!(near(photoshop.min(small), 0.02), "{}", photoshop.min(small));
    assert!(photoshop.min(big) < photoshop.min(small), "the floor follows the shorter side");

    let doubled = photoshop.with_hundred(2.0);
    for device in [0.5, 3.0, 40.0] {
        assert!(near(doubled.device(doubled.percent(device)), device), "{device}");
    }
    assert!(near(doubled.step(2.0, 1, small), 4.0), "the rung after the scheme's 100 % is the next rung times it");
    assert!(near(photoshop.clamp(f64::NAN, small), 1.0) && near(doubled.clamp(f64::NAN, small), 2.0), "a non-finite zoom is 100 %");
}

#[test]
fn a_stretched_view_draws_and_reads_the_document_over_whole_pixels_while_its_zoom_stays_nominal() {
    // A 600×450 document at a third: the host shows it over 201×151 device pixels (ceil(size × zoom)) and says so
    // through the stretch; the zoom behind it stays 1/3, so Zoom Out steps to 25 %, not back onto 33.33 %.
    let zoom = f64::from(1.0f32 / 3.0);
    let stretch = [201.0 / 600.0 / zoom, 151.0 / 450.0 / zoom];
    let plain = CanvasView::new(ScreenRect::from_min_max(pos2(0.0, 0.0), pos2(640.0, 400.0)), Point::new(300.0, 225.0), zoom);
    let view = plain.stretch(stretch);
    let shown = view.to_screen(Point::new(600.0, 450.0)) - view.to_screen(Point::new(0.0, 0.0));
    assert!((f64::from(shown.x) - 201.0).abs() < 1e-4 && (f64::from(shown.y) - 151.0).abs() < 1e-4, "{shown:?}");
    let moved = view.to_doc(pos2(100.0, 137.0)) - view.to_doc(pos2(100.0, 100.0));
    assert!(near(moved.y, 37.0 * 450.0 / 151.0) && near(moved.x, 0.0), "{moved:?}");
    let moved = view.to_doc(pos2(180.0, 100.0)) - view.to_doc(pos2(100.0, 100.0));
    assert!(near(moved.x, 80.0 * 600.0 / 201.0), "{moved:?}");
    for p in [Point::new(0.0, 0.0), Point::new(512.0, 300.0), Point::new(-40.0, 1200.0)] {
        let back = view.to_doc(view.to_screen(p));
        assert!(near(back.x, p.x) && near(back.y, p.y), "{p:?} came back as {back:?}");
    }
    let turned = view.rotation(33.0).flip_x(true).aspect(1.5);
    for p in [Point::new(0.0, 0.0), Point::new(512.0, 300.0)] {
        let back = turned.to_doc(turned.to_screen(p));
        assert!(near(back.x, p.x) && near(back.y, p.y), "{p:?} came back as {back:?} through {turned:?}");
    }
    assert!(near(view.zoom_x(), 201.0 / 600.0) && near(view.doc_len(151.0), 450.0));
    assert!(view.layer_transform().is_none() && plain.layer_transform().is_some());
    let mut stepped = view;
    assert!(zoom::step(&mut stepped, -1, Size::new(600.0, 450.0), 1.0, &ZoomScheme::PHOTOSHOP));
    assert!(near(stepped.zoom, 0.25), "{}", stepped.zoom);
    // A stretch that is no number or not positive counts as none.
    let odd = plain.stretch([f64::NAN, 0.0]);
    assert_eq!(odd.to_screen(Point::new(600.0, 450.0)), plain.to_screen(Point::new(600.0, 450.0)));
}
