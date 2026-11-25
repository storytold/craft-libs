//! The text layout memo seen from a component's measure.

use craft_behavior::Sense;
use craft_ui::contract::{Common, Component, Cx, MeasureCx, Props as _, Role, Show as _};
use craft_ui::primitives::TextRun;
use craft_ui::style::StyleCx;
use craft_ui::testing::{Fixture, last_pass_text_layouts, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Response, Vec2};
use std::cell::Cell;
use std::rc::Rc;

#[derive(Component)]
struct LeastTwice {
    #[common]
    common: Common,
    measured: Rc<Cell<u32>>,
}

impl Component for LeastTwice {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: LeastTwiceStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let run = TextRun::new("the complete least layout is retained").elide();
        let first = cx.least(&run);
        let count = cx.ui.ctx().fonts(|fonts| fonts.num_galleys_in_cache());
        let second = cx.least(&run);
        assert_eq!(first.width, second.width);
        assert!(std::sync::Arc::ptr_eq(&first.whole.galley, &second.whole.galley));
        assert_eq!(cx.ui.ctx().fonts(|fonts| fonts.num_galleys_in_cache()), count, "a repeated least request performs no new layouts");
        self.measured.set(self.measured.get() + 1);
        second.size()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

#[trial]
fn repeated_measure_least_reuses_both_results() {
    let measured = Rc::new(Cell::new(0));
    let output = measured.clone();
    let mut fixture = Fixture::new(move |ui| {
        LeastTwice { common: Common::new(), measured: output.clone() }.id_salt("least-twice").show(ui);
    });
    fixture.frame(Vec::new());
    let first_count = measured.get();
    assert!(first_count > 0);
    fixture.frame(Vec::new());
    assert!(measured.get() > first_count, "the second pass must actually request the least layout");
    assert_eq!(last_pass_text_layouts(fixture.ctx()), 0, "repeated least requests reuse the whole layout and least width");
}
