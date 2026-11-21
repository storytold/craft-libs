//! `ColorSamplerMarks`: the marker look measured in photocraft #11739 — the centre dot, the 9 px circle, the four
//! 3 px ticks outside it, the number below-right, and the inverted colours when the host hands in the displayed
//! pixels. The measured rows: at 100 % zoom a sampler on the pixel at screen 578, 350 shows a 1 px dot, a circle
//! spanning 574 to 582 on both axes, ticks at 571 to 573 and 583 to 585 on row 350 and at 343 to 345 and 355 to
//! 357 in column 578, and the digit's top-left at +8, +5.

use craft_behavior::CanvasView;
use craft_behavior::kurbo::Point;
use craft_color_ui::ui::{ColorSamplerMarks, ColorSamplerPass, ColorSamplerTool, ColorSamplers, SamplerCarry};
use craft_theme::Srgba;
use craft_ui::contract::Show as _;
use craft_ui::testing::{ToolFixture, trial};
use egui::{Color32, Pos2, Rect, Shape, Stroke, pos2, vec2};

const WHITE: Srgba = Srgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
const BLACK: Srgba = Srgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
const RED: Srgba = Srgba { r: 0.862_745_1, g: 0.117_647_06, b: 0.117_647_06, a: 1.0 };

/// The marker pixels of the measured look, as offsets from the centre pixel: the dot, the circle whose pixels span
/// −4 to +4 on both axes (screen 574 to 582 around 578), and the four 3 px ticks at ±5 to ±7 on the axes (screen
/// 571 to 573 and 583 to 585). The ring is one pixel thick between the axes and the diagonals.
const MARKER: [(i32, i32); 33] = [
    (0, 0),
    (-4, 0),
    (4, 0),
    (0, -4),
    (0, 4),
    (-4, -1),
    (-4, 1),
    (4, -1),
    (4, 1),
    (-1, -4),
    (1, -4),
    (-1, 4),
    (1, 4),
    (-3, -2),
    (-3, 2),
    (3, -2),
    (3, 2),
    (-2, -3),
    (2, -3),
    (-2, 3),
    (2, 3),
    (-7, 0),
    (-6, 0),
    (-5, 0),
    (5, 0),
    (6, 0),
    (7, 0),
    (0, -7),
    (0, -6),
    (0, -5),
    (0, 5),
    (0, 6),
    (0, 7),
];

/// The document size the fixture's canvas carries.
const SIZE: [u32; 2] = [1200, 900];

/// A Color Sampler canvas showing the marks of `points`: the document's origin sits at the canvas' top left, one
/// document unit to `zoom` screen points; `displayed` is the colour shown at every screen point, or none; `carried`
/// is the point the tool carries.
fn marks_fixture(points: &[[f64; 2]], zoom: f64, displayed: Option<Srgba>, carried: Option<SamplerCarry>) -> ToolFixture<ColorSamplerPass<'static>> {
    let mut samplers = ColorSamplers::new();
    for p in points {
        samplers.add(*p, SIZE).unwrap();
    }
    // The fixture owns the pass for the whole trial, so the parts it borrows live as long.
    let samplers: &'static ColorSamplers = Box::leak(Box::new(samplers));
    let tool: &'static mut ColorSamplerTool = Box::leak(Box::new(ColorSamplerTool::new()));
    let read: Option<&'static dyn Fn(Pos2) -> Srgba> = displayed.map(|color| &*Box::leak(Box::new(move |_| color) as Box<dyn Fn(Pos2) -> Srgba>));
    let mut fixture = ToolFixture::with_view(
        "Color Sampler",
        tool.pass(samplers, SIZE),
        move |rect| CanvasView::new(rect, Point::new(f64::from(rect.width()) / (2.0 * zoom), f64::from(rect.height()) / (2.0 * zoom)), zoom),
        move |ui, body| {
            let marks = ColorSamplerMarks::new("Color samplers", samplers);
            let marks = match carried {
                Some(carried) => marks.carried(carried),
                None => marks,
            };
            match read {
                Some(read) => marks.displayed(read).show_at(ui, body.rect),
                None => marks.show_at(ui, body.rect),
            };
        },
    );
    fixture.fixture().room(vec2(800.0, 600.0));
    fixture.fixture().settle();
    fixture
}

/// Where the fixture's view draws document point `doc`: the origin is at the canvas' top left at `zoom`.
fn screen_at(canvas: Rect, doc: [f64; 2], zoom: f32) -> Pos2 {
    pos2(canvas.min.x + doc[0] as f32 * zoom, canvas.min.y + doc[1] as f32 * zoom)
}

/// The painted squares one point on a side and unstrokeled: the marker pixels, with their top-left and fill.
fn painted_pixels(shapes: &[Shape]) -> Vec<(Pos2, Color32)> {
    shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Rect(r) if r.stroke == Stroke::NONE && r.rect.width() == 1.0 && r.rect.height() == 1.0 => Some((r.rect.min, r.fill)),
            _ => None,
        })
        .collect()
}

/// The marker pixel offsets painted around `at` (rounded to whole points, within the marker's reach).
fn offsets_around(pixels: &[(Pos2, Color32)], at: Pos2) -> Vec<[i32; 2]> {
    let mut out: Vec<[i32; 2]> = pixels
        .iter()
        .filter(|(min, _)| (min.x - at.x).abs() <= 8.0 && (min.y - at.y).abs() <= 8.0)
        .map(|(min, _)| [(min.x - at.x).round() as i32, (min.y - at.y).round() as i32])
        .collect();
    out.sort();
    out
}

/// A painted number: its top-left, its digits, and the colours of its glyph mesh vertices.
struct Number<'a> {
    at: Pos2,
    text: &'a str,
    colors: Vec<Color32>,
}

fn painted_numbers(shapes: &[Shape]) -> Vec<Number<'_>> {
    shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Text(t) => {
                let text = t.galley.text();
                (!text.is_empty() && text.chars().all(|c| c.is_ascii_digit())).then(|| Number {
                    at: t.pos,
                    text,
                    colors: t.galley.rows.iter().flat_map(|r| r.row.visuals.mesh.vertices.iter().map(|v| v.color)).collect(),
                })
            }
            _ => None,
        })
        .collect()
}

/// The number painted with its top-left within reach of `at`, if any.
fn number_at(numbers: &[Number<'_>], at: Pos2) -> Option<usize> {
    numbers.iter().position(|n| (n.at.x - at.x).abs() < 20.0 && (n.at.y - at.y).abs() < 20.0)
}

#[trial]
fn the_marker_paints_the_measured_dot_circle_and_ticks_at_100_percent_zoom() {
    let mut fixture = marks_fixture(&[[300.0, 150.0], [390.0, 240.0]], 1.0, Some(WHITE), None);
    let canvas = fixture.canvas().rect;
    let shapes = fixture.fixture().frame_shapes();
    let pixels = painted_pixels(&shapes);

    // Exactly the measured pixels, and nothing else of the marker but the number.
    let mut expected: Vec<[i32; 2]> = MARKER.iter().map(|(dx, dy)| [*dx, *dy]).collect();
    expected.sort();
    for doc in [[300.0, 150.0], [390.0, 240.0]] {
        let at = screen_at(canvas, doc, 1.0);
        assert_eq!(offsets_around(&pixels, at), expected, "the marker of the point on {}", doc.map(|v| v.to_string()).join(", "));
    }
    assert_eq!(pixels.len(), 2 * MARKER.len(), "no marker pixel outside the measured set");
    assert!(!shapes.iter().any(|s| matches!(s, Shape::Circle(_))), "no circle reticle stroke remains");

    // The number's top-left at +8, +5 below-right of the dot, one number per point (no shadow run).
    let numbers = painted_numbers(&shapes);
    assert_eq!(numbers.len(), 2, "one number per point, no drop shadow");
    for (index, doc) in [[300.0, 150.0], [390.0, 240.0]].into_iter().enumerate() {
        let at = screen_at(canvas, doc, 1.0);
        let found = number_at(&numbers, at + vec2(8.0, 5.0)).expect("the number below-right");
        assert_eq!(numbers[found].text, (index + 1).to_string());
        assert_eq!((numbers[found].at.x - at.x).round() as i32, 8, "the number's top-left is +8 right");
        assert_eq!((numbers[found].at.y - at.y).round() as i32, 5, "and +5 down of the dot");
    }
}

#[trial]
fn every_marker_pixel_is_the_displayed_pixel_under_it_flipped() {
    // The measured rows: 127 on white, 128 on black, 92/158/158 on 220/30/30.
    let cases = [(WHITE, Color32::from_rgb(127, 127, 127)), (BLACK, Color32::from_rgb(128, 128, 128)), (RED, Color32::from_rgb(92, 158, 158))];
    for (displayed, flip) in cases {
        let mut fixture = marks_fixture(&[[300.0, 150.0]], 1.0, Some(displayed), None);
        fixture.canvas();
        let shapes = fixture.fixture().frame_shapes();

        let pixels = painted_pixels(&shapes);
        assert_eq!(pixels.len(), MARKER.len(), "the marker pixels of the displayed {displayed:?}");
        assert!(pixels.iter().all(|(_, fill)| *fill == flip), "every marker pixel is the displayed pixel under it flipped at 0x80");

        let numbers = painted_numbers(&shapes);
        assert_eq!(numbers.len(), 1);
        assert!(numbers[0].colors.iter().all(|c| *c == flip), "the number is flipped too, with no halo-coloured shadow run");
    }
}

#[trial]
fn the_marker_and_the_number_keep_their_screen_geometry_at_66_7_percent_zoom() {
    let mut fixture = marks_fixture(&[[300.0, 150.0], [600.0, 240.0]], 2.0 / 3.0, Some(WHITE), None);
    let canvas = fixture.canvas().rect;
    let shapes = fixture.fixture().frame_shapes();
    let pixels = painted_pixels(&shapes);

    let mut expected: Vec<[i32; 2]> = MARKER.iter().map(|(dx, dy)| [*dx, *dy]).collect();
    expected.sort();
    for doc in [[300.0, 150.0], [600.0, 240.0]] {
        let at = screen_at(canvas, doc, 2.0 / 3.0);
        assert_eq!(offsets_around(&pixels, at), expected, "the same screen geometry as at 100 %, at {}", doc.map(|v| v.to_string()).join(", "));
    }
    assert_eq!(pixels.len(), 2 * MARKER.len());

    let numbers = painted_numbers(&shapes);
    assert_eq!(numbers.len(), 2);
    for doc in [[300.0, 150.0], [600.0, 240.0]] {
        let at = screen_at(canvas, doc, 2.0 / 3.0);
        let found = number_at(&numbers, at + vec2(8.0, 5.0)).expect("the number below-right, same place as at 100 %");
        assert_eq!((numbers[found].at.x - at.x).round() as i32, 8);
        assert_eq!((numbers[found].at.y - at.y).round() as i32, 5);
    }
}

#[trial]
fn a_carried_point_draws_the_same_measured_marker() {
    let carried = SamplerCarry { index: None, at: [450.0, 240.0] };
    let mut fixture = marks_fixture(&[[300.0, 150.0]], 1.0, Some(WHITE), Some(carried));
    let canvas = fixture.canvas().rect;
    let shapes = fixture.fixture().frame_shapes();
    let pixels = painted_pixels(&shapes);

    let mut expected: Vec<[i32; 2]> = MARKER.iter().map(|(dx, dy)| [*dx, *dy]).collect();
    expected.sort();
    for doc in [[300.0, 150.0], [450.0, 240.0]] {
        let at = screen_at(canvas, doc, 1.0);
        assert_eq!(offsets_around(&pixels, at), expected, "the carried point draws at the pointer in the same look");
    }
    assert_eq!(pixels.len(), 2 * MARKER.len());
    let numbers = painted_numbers(&shapes);
    assert_eq!(numbers.len(), 2, "and under the next number");
    assert!(numbers.iter().any(|n| n.text == "2"), "the new point is number 2");
}

#[trial]
fn marks_without_displayed_pixels_keep_the_theme_reticle() {
    // A host that hands in no pixels (all hosts today): the double-stroked reticle and the shadowed number.
    let mut fixture = marks_fixture(&[[300.0, 150.0]], 1.0, None, None);
    fixture.canvas();
    let shapes = fixture.fixture().frame_shapes();

    assert_eq!(painted_pixels(&shapes).len(), 0, "no pixel-drawn marker without the host's pixels");
    assert!(shapes.iter().any(|s| matches!(s, Shape::Circle(_))), "the stroked reticle stays");
    let numbers = painted_numbers(&shapes);
    assert_eq!(numbers.len(), 2, "today's number: its halo run under the ink run");
    assert!(numbers.iter().all(|n| n.text == "1"));
}
