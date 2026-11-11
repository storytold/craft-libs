//! Every popup sits on one popup surface: a fill under its content and one shadow, whether a container mounts it as
//! a layer or a component shows it from its `overlay` hook, and whatever its body paints itself. Its content sits
//! inside the popup padding, and a popup beside its owner does not cover it.

use craft_ui::testing::{Drive, Fixture, stories, trial};
use egui::{Rect, Shape};

/// Each story, opened by its drive, with a text only its popup shows.
const OPEN: [(&str, &str); 4] = [
    // Hook form, a list body.
    ("select_open", "Screen"),
    // Hook form, an inline menu body.
    ("split_button_open", "Colour"),
    // Container form.
    ("popup_list", "Square"),
    // Hook form, a dock group body that paints its own panel.
    ("open_beside_its_column", "Panel 0"),
];

/// The story `name`, opened by its drive, and the shapes of the frame after it settled.
fn open(name: &str) -> (Fixture, Vec<Shape>) {
    let entry = stories().into_iter().find(|entry| entry.story.name == name).unwrap_or_else(|| panic!("story {name}"));
    let mut fixture = Fixture::story(&entry.story);
    fixture.settle();
    if let Drive::Custom(open) = entry.story.drive {
        open(&mut fixture);
    }
    fixture.settle();
    let shapes = fixture.frame_shapes();
    (fixture, shapes)
}

/// The rect of the one popup surface: the fill a surface paints right after its shadow.
fn surface(name: &str, shapes: &[Shape]) -> Rect {
    let shadows: Vec<usize> = (0..shapes.len()).filter(|at| matches!(&shapes[*at], Shape::Rect(rect) if rect.blur_width > 0.0)).collect();
    assert_eq!(shadows.len(), 1, "{name}: one shadow, under the popup");
    match shapes.get(shadows[0] + 1) {
        Some(Shape::Rect(fill)) if fill.blur_width == 0.0 && fill.fill.a() > 0 => fill.rect,
        other => panic!("{name}: the popup's surface is filled over its shadow, not {other:?}"),
    }
}

/// The rect of the text `text` painted on `surface`.
fn text_rect(shapes: &[Shape], surface: Rect, text: &str) -> Option<Rect> {
    shapes.iter().find_map(|shape| match shape {
        Shape::Text(run) if run.galley.job.text.replace('&', "") == text => {
            Some(run.galley.rect.translate(run.pos.to_vec2())).filter(|rect| surface.intersects(*rect))
        }
        _ => None,
    })
}

#[trial]
fn an_open_popup_paints_one_filled_surface_with_one_shadow_under_its_content() {
    for (name, row) in OPEN {
        let (_, shapes) = open(name);
        let surface = surface(name, &shapes);
        let content = text_rect(&shapes, surface, row).unwrap_or_else(|| panic!("{name}: the popup shows {row}"));
        assert!(surface.contains_rect(content), "{name}: the surface {surface:?} lies under the popup's {row} at {content:?}");
    }
}

/// The option rows of a popup list sit inside the popup's padding, the same on every side, with their texts in one
/// column. In a list that picks a value, the chosen row is outlined and checked.
#[trial]
fn a_popup_list_insets_its_rows_and_marks_the_chosen_one() {
    for (name, rows, chosen) in
        [("select_open", ["Normal", "Multiply", "Screen"], Some("Multiply")), ("popup_list", ["Round", "Square", "Fan"], None)]
    {
        let (_, shapes) = open(name);
        let surface = surface(name, &shapes);
        let texts: Vec<Rect> = rows.iter().map(|row| text_rect(&shapes, surface, row).unwrap_or_else(|| panic!("{name}: {row} shows"))).collect();
        assert!(texts.windows(2).all(|pair| (pair[0].left() - pair[1].left()).abs() < 0.5), "{name}: the row texts start in one column: {texts:?}");
        // The row rects: the rects inside the surface that hold a row's text.
        let row_of = |text: Rect| {
            shapes.iter().find_map(|shape| match shape {
                Shape::Rect(rect)
                    if rect.blur_width == 0.0 && rect.rect != surface && surface.contains_rect(rect.rect) && rect.rect.contains_rect(text) =>
                {
                    Some(rect)
                }
                _ => None,
            })
        };
        let first = row_of(texts[0]).unwrap_or_else(|| panic!("{name}: the first row's rect"));
        let inset = first.rect.left() - surface.left();
        assert!(inset > 0.0, "{name}: the rows sit inside the popup's padding");
        for (side, gap) in [("end", surface.right() - first.rect.right()), ("top", first.rect.top() - surface.top())] {
            assert!((gap - inset).abs() < 0.5, "{name}: the {side} padding {gap} matches the start padding {inset}");
        }
        let Some(chosen) = chosen else { continue };
        let at = rows.iter().position(|row| *row == chosen).unwrap_or_default();
        let row = shapes
            .iter()
            .find_map(|shape| match shape {
                Shape::Rect(rect) if rect.stroke.width > 0.0 && rect.rect.contains_rect(texts[at]) && rect.rect != surface => Some(rect.rect),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{name}: the chosen row is outlined"));
        let mark = shapes.iter().any(|shape| {
            !matches!(shape, Shape::Rect(_) | Shape::Text(_)) && {
                let bounds = shape.visual_bounding_rect();
                row.contains_rect(bounds) && bounds.right() <= texts[at].left()
            }
        });
        assert!(mark, "{name}: the chosen row shows a mark before its text");
    }
}

/// A flyout hangs beside its iconic column: it touches the column's edge and covers none of it.
#[trial]
fn a_flyout_hangs_beside_its_column_without_covering_it() {
    let (fixture, shapes) = open("open_beside_its_column");
    let surface = surface("flyout", &shapes);
    let column = fixture.by_label("Panels").expect("the column").rect;
    assert!(surface.left() >= column.right() - 0.5, "the flyout {surface:?} covers none of the column {column:?}");
    assert!(surface.left() - column.right() < 0.5, "the flyout {surface:?} touches the column {column:?}");
}
