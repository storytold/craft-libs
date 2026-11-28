//! Static text reads its words by value, as AccessKit expects of a `Label`.

use craft_ui::contract::{Common, Component, Cx, PaintUi, Props as _, Role, Show as _};
use craft_ui::primitives::TextRun;
use craft_ui::style::StyleCx;
use craft_ui::testing::{Fixture, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Pos2, Rect, Response};

const AT: Rect = Rect { min: Pos2::new(10.0, 10.0), max: Pos2::new(310.0, 40.0) };

/// A line of static text, named by its label, as a display text is.
#[derive(craft_ui::contract::Component)]
struct Words {
    #[common]
    common: Common,
}

impl Component for Words {
    const ROLE: Role = Role::LABEL;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: WordsStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        ui.draw().text(cx.content_rect, &TextRun::new(cx.resolved.label));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

// A plain text's node carries its words as its value, where assistive technology and kittest read a `Label`'s words,
// as well as its name.
#[trial]
fn a_plain_text_reads_its_words_by_value() {
    let mut fixture = Fixture::new(|ui| {
        Words { common: Common::new() }.label("Esc to cancel").show_at(ui, AT);
    });
    fixture.settle();
    let tree = fixture.tree();
    let labels = tree.with_role(Role::LABEL);
    let node = labels.iter().copied().find(|key| tree.name_of(*key) == Some("Esc to cancel")).expect("the text's node");
    assert_eq!(tree.value_of(node), Some("Esc to cancel"), "the words are the node's value");
}
