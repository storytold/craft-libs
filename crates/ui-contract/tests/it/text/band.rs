//! A run laid out in room equal to its own width.

use craft_behavior::Sense;
use craft_ui::contract::{Common, Component, Cx, MeasureCx, Props as _, Role, Show as _};
use craft_ui::primitives::{TextRun, Wrap};
use craft_ui::style::StyleCx;
use craft_ui::testing::{Fixture, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Response, Vec2};
use std::cell::Cell;
use std::rc::Rc;

const WORDS: [&str; 9] = ["200", "100", "44%", "40%", "201", "45%", "Hello", "Width", "mm"];

/// Lays each word out unbounded and again in room equal to that width, and counts the words whose width is less than
/// half a point past a whole point: the widths a whole-point wrap width would round down.
#[derive(Component)]
struct OwnWidth {
    #[common]
    common: Common,
    below_half: Rc<Cell<usize>>,
}

impl Component for OwnWidth {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: OwnWidthStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let mut below_half = 0;
        for word in WORDS {
            for wrap in [Wrap::Wrap, Wrap::Elide] {
                let run = TextRun::new(word).wrap(wrap);
                let whole = cx.layout_at(&run, f32::INFINITY);
                let width = whole.size().x;
                let fitted = cx.layout_at(&run, width);
                assert_eq!(fitted.galley.rows.len(), 1, "{word:?} {wrap:?}");
                assert!(!fitted.elided, "{word:?} {wrap:?}");
                assert_eq!(fitted.size().x, width, "{word:?} {wrap:?}: room equal to its width keeps its width");
            }
            let width = cx.layout_at(&TextRun::new(word), f32::INFINITY).size().x;
            if width.fract() > 0.0 && width.fract() < 0.5 {
                below_half += 1;
            }
        }
        self.below_half.set(below_half);
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

#[trial]
fn room_equal_to_its_width_keeps_a_word_whole() {
    for pixels_per_point in [1.0, 2.0] {
        let below_half = Rc::new(Cell::new(0));
        let output = below_half.clone();
        let mut fixture = Fixture::new(move |ui| {
            OwnWidth { common: Common::new(), below_half: output.clone() }.id_salt("own-width").show(ui);
        });
        fixture.pixels_per_point(pixels_per_point);
        fixture.frame(Vec::new());
        assert!(below_half.get() > 0, "at {pixels_per_point}x some word must be less than half a point past a whole point");
    }
}
