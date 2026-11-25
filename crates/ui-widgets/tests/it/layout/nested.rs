//! Nested content-sized blocks record their size in the pass that first shows them.

use craft_behavior::{ColumnWidth, Sense};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Common, Component, Container, ContainerProps, Cx, Role, Show as _, ShowContainer as _};
use craft_ui::layout::BodyLayout;
use craft_ui::popover::Popover;
use craft_ui::style::StyleCx;
use craft_ui::testing::{Fixture, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::Response;
use std::cell::Cell;
use std::rc::Rc;

/// A content-sized container whose body is laid out by `layout`.
#[derive(Component)]
#[component(container)]
struct Block {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl Block {
    fn new(layout: BodyLayout) -> Self {
        Block { common: Common::new(), props: ContainerProps::new().layout(layout) }
    }
}

impl Component for Block {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: BlockStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Block {}

static CELLS: [ColumnWidth; 2] = [ColumnWidth::Content, ColumnWidth::Content];

/// 6.3: a popover whose body nests a row in a grid in a row opens at its final width on its first visible frame
/// and runs one discard pass: the first pass records every nested block, so the pass that follows places them all by
/// their real sizes.
#[trial]
fn a_popover_with_nested_blocks_opens_at_its_final_size_within_one_discard_pass() {
    let open = Rc::new(Cell::new(false));
    let shown_open = open.clone();
    let mut fixture = Fixture::new(move |ui| {
        let opener = Button::new("Brush").show(ui);
        let mut is_open = shown_open.get();
        Popover::new("Brush options", Bind::bound(&mut is_open)).opened_by(&opener).show(ui, |ui, _| {
            Block::new(BodyLayout::row()).show(ui, |ui, _| {
                Block::new(BodyLayout::grid(&CELLS)).show(ui, |ui, _| {
                    Block::new(BodyLayout::row()).show(ui, |ui, _| {
                        Button::new("Reset the brush to its defaults").show(ui);
                        Button::new("Apply").show(ui);
                    });
                    Button::new("Close").show(ui);
                });
            });
        });
        shown_open.set(is_open);
    });
    fixture.settle();
    open.set(true);
    let mut widths = Vec::new();
    let mut passes = Vec::new();
    for _ in 0..4 {
        fixture.frame(Vec::new());
        widths.push(craft_ui::testing::frames::widest_layer(fixture.ctx()));
        passes.push(fixture.passes());
    }
    fixture.idle(8);
    let settled = craft_ui::testing::frames::widest_layer(fixture.ctx());
    let tolerance = 0.5;
    assert!(widths[0] > 0.0, "the layer shows on the first frame: {widths:?} {passes:?}");
    assert!(
        (widths[0] - settled).abs() < tolerance && widths.iter().all(|width| (width - settled).abs() < tolerance),
        "the layer has its final width from its first frame: {widths:?} against {settled}"
    );
    assert!(passes[0] <= 2, "the opening frame runs at most one discard pass: {passes:?}");
}
