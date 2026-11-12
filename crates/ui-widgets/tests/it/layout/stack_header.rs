//! A stack's header and footer are its first and last rows: the stack's gap separates them from the body.

use craft_ui::contract::{Gap, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::layout_components::Stack;
use craft_ui::testing::{Fixture, trial};
use egui::Rect;

/// Half a pixel at the scales the box runs.
const ROUNDING: f32 = 0.5;

/// The gaps in a stack with a two-line header, a body line and a footer line: between the header's lines, header to
/// body, body to footer. `gap` is set after the header and footer.
fn gaps(gap: Option<Gap>) -> [f32; 3] {
    let mut fixture = Fixture::new(move |ui| {
        let mut stack = Stack::new()
            .header(|ui, _| {
                Text::new("Head one").show(ui);
                Text::new("Head two").show(ui);
            })
            .footer(|ui, _| {
                Text::new("Foot").show(ui);
            });
        if let Some(gap) = gap {
            stack = stack.gap(gap);
        }
        stack.show(ui, |ui, _| {
            Text::new("Body").show(ui);
        });
    });
    fixture.settle();
    let rect =
        |text: &str| -> Rect { fixture.texts().iter().find(|painted| painted.text == text).unwrap_or_else(|| panic!("no {text} painted")).rect };
    let (one, two, body, foot) = (rect("Head one"), rect("Head two"), rect("Body"), rect("Foot"));
    [two.top() - one.bottom(), body.top() - two.bottom(), foot.top() - body.bottom()]
}

#[trial]
fn a_stack_spaces_its_header_and_footer_from_the_body_by_its_gap() {
    for gap in [None, Some(Gap::Section)] {
        let [rows, header, footer] = gaps(gap);
        assert!(rows > 0.0, "the header's lines are spaced ({gap:?})");
        assert!((header - rows).abs() <= ROUNDING, "header to body {header}, between header lines {rows} ({gap:?})");
        assert!((footer - rows).abs() <= ROUNDING, "body to footer {footer}, between header lines {rows} ({gap:?})");
    }
}
