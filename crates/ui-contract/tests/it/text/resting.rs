//! A text part's resting text is drawn while the part is idle; an edit made to an idle part (the host replacing its
//! selection) is made to the value's own text.

use craft_behavior::{PartKey, PlainText, Sense};
use craft_ui::contract::{Bind, BindMut, Common, Component, Edit as _, EditResponse, Input, MeasureCx, PartsCx, Role, Show as _};
use craft_ui::primitives::{TextKey, TextOptions};
use craft_ui::style::StyleCx;
use craft_ui::testing::{Fixture, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Id, Rect, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const TEXT: PartKey = PartKey::new("text");

/// One text part over a string: it shows `resting` while idle, and `insert` replaces its selection in the pass it is set.
#[derive(craft_ui::contract::Component)]
struct Resting<'a> {
    #[common]
    common: Common,
    value: Bind<'a, String>,
    resting: &'static str,
    insert: Option<&'static str>,
}

impl Component for Resting<'_> {
    const ROLE: Role = Role::TEXT_INPUT;
    const INPUT: Input = Input::Text;
    type Style = PlainStyle;
    type Output = EditResponse<String>;

    fn style(_: &StyleCx<'_>, _: RestingStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(120.0, 24.0)
    }

    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let options =
            TextOptions { resting: Some(self.resting.to_owned()), replace_selection: self.insert.map(str::to_owned), ..TextOptions::default() };
        let (shown, rect) = (cx.value().cloned().unwrap_or_default(), cx.content_rect);
        cx.text(TextKey::<String>::new(TEXT), rect, &PlainText, &shown, &options);
    }

    fn bind(&mut self) -> Option<BindMut<'_, String>> {
        Some(self.value.reborrow())
    }

    fn finish(self, cx: craft_ui::contract::Cx<'_, Self>) -> EditResponse<String> {
        cx.edit_response()
    }
}

/// The host replaces the selection of an idle part that shows resting text: the insert goes into the value's own text
/// ("42" and "7" make "427"), not into the resting text drawn in its place ("(42)7").
#[trial]
fn an_edit_to_an_idle_part_is_made_to_the_value_not_to_its_resting_text() {
    let value = Rc::new(RefCell::new(String::from("42")));
    let insert = Rc::new(Cell::new(None));
    let id = Rc::new(Cell::new(Id::NULL));
    let (held, once, seen) = (value.clone(), insert.clone(), id.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut text = held.borrow_mut();
        let out = Resting { common: Common::new(), value: Bind::bound(&mut *text), resting: "(42)", insert: once.take() }.show(ui);
        seen.set(out.response.id);
    });
    fixture.settle();
    assert_eq!(fixture.tree().value(id.get()), Some("(42)"), "an idle part shows its resting text");
    insert.set(Some("7"));
    fixture.settle();
    assert_eq!(*value.borrow(), "427", "the insert went into the value's own text");
}
