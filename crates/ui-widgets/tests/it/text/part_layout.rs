//! A text part's laid-out text for the component that shows it: set at a size of the caller's own, and placed on screen
//! where its rows are drawn, so an editor paints gutters beside them.

use craft_behavior::{Metrics, PartKey, PlainText, Sense};
use craft_ui::contract::{
    Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Input, MeasureCx, PaintUi, PartsCx, Props as _, Role, Show as _,
};
use craft_ui::primitives::{TextKey, TextOptions};
use craft_ui::style::{ComponentStyle, StyleCx};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Pos2, Rect, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

const TEXT: PartKey = PartKey::new("text");

/// Half a point: rects compared across passes may round apart by that much.
const SNAP: f32 = 0.5;

#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
struct PlainStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What an editor's paint read of its text part: the part's rect, the layout's origin and each row's screen top and
/// bottom.
#[derive(Clone, Debug, Default)]
struct Painted {
    part: Option<Rect>,
    origin: Option<Pos2>,
    rows: Vec<(f32, f32)>,
}

/// A multi-line text part of a fixed size, whose paint reads the part's outcome as a gutter would.
#[derive(Component)]
struct Editor<'a> {
    #[common]
    common: Common,
    value: Bind<'a, String>,
    size: Option<f32>,
    top: bool,
    painted: &'a RefCell<Painted>,
}

impl Component for Editor<'_> {
    const ROLE: Role = Role::TEXT_INPUT;
    const INPUT: Input = Input::Text;
    const OVERLAYS: bool = false;
    type Style = PlainStyle;
    // The value is the component's: its output carries the `String` it edits, so the part reads it as `cx.value()`.
    type Output = EditResponse<String>;

    fn style(_: &StyleCx<'_>, _: EditorStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(240.0, 80.0)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let mut options = TextOptions::new();
        options.lines = Some(3);
        options.size = self.size;
        options.top = self.top;
        let rect = cx.content_rect;
        if let Some(value) = cx.value() {
            cx.text(TextKey::new(TEXT), rect, &PlainText, value, &options);
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, String>> {
        Some(self.value.reborrow())
    }

    fn paint(&mut self, _: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(outcome) = cx.text(&TextKey::<String>::new(TEXT)) else { return };
        let Some(layout) = &outcome.layout else { return };
        let origin = outcome.origin;
        let rows = layout.galley.rows.iter().map(|row| (origin.y + row.min_y(), origin.y + row.max_y())).collect();
        *self.painted.borrow_mut() = Painted { part: Some(outcome.rect), origin: Some(origin), rows };
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<String> {
        cx.edit_response()
    }
}

/// Shows an editor over `text` set at `size` and returns what its paint read after the fixture settles. The text starts at
/// the top of its part as an editor's does.
fn editor(text: &str, size: Option<f32>) -> (Fixture, Rc<RefCell<String>>, Rc<RefCell<Painted>>) {
    shown_editor(text, size, true)
}

/// [`editor`], with the text starting at the top of its part or not (`TextOptions::top`).
fn shown_editor(text: &str, size: Option<f32>, top: bool) -> (Fixture, Rc<RefCell<String>>, Rc<RefCell<Painted>>) {
    let (value, painted) = (Rc::new(RefCell::new(text.to_owned())), Rc::new(RefCell::new(Painted::default())));
    let (shown, read) = (value.clone(), painted.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut held = shown.borrow_mut();
        Editor { common: Common::new(), value: Bind::bound(&mut *held), size, top, painted: &read }.id_salt("editor").show(ui);
    });
    fixture.settle();
    (fixture, value, painted)
}

/// A size of the caller's own sets the text at that size, not the role's: the rows of a part set at 30 points are
/// taller than those of the same text at its role's size, and still lie in order down the part.
#[trial]
fn a_part_set_at_its_own_size_lays_its_rows_out_at_that_size() {
    let row = |painted: &Painted| painted.rows.first().map_or(0.0, |(top, bottom)| bottom - top);
    let (_plain, _, at_role) = editor("one\ntwo", None);
    let (_large, _, at_size) = editor("one\ntwo", Some(30.0));
    let (at_role, at_size) = (at_role.borrow().clone(), at_size.borrow().clone());
    assert!(row(&at_role) > 0.0 && at_role.rows.len() == 2, "the role's text lays out two rows: {at_role:?}");
    assert!(row(&at_size) > row(&at_role) * 1.5, "30 points sets taller rows than the role's size: {at_size:?} vs {at_role:?}");
    assert!(at_size.rows.windows(2).all(|pair| pair[1].0 >= pair[0].1 - SNAP), "the rows lie in order: {at_size:?}");
}

/// A multi-line text shorter than its part starts at the part's top when the part asks for it (`TextOptions::top`): its
/// one row lies in the top half of the part. Without it the text keeps the placement every field had, centred on the
/// part's height.
#[trial]
fn a_short_multi_line_text_starts_at_the_top_of_its_part_when_asked_to() {
    let (_fixture, _, painted) = editor("one", None);
    let painted = painted.borrow().clone();
    let part = painted.part.expect("the part is laid out");
    let row = painted.rows.first().copied().expect("the text has a row");
    assert!(row.0 >= part.top() - SNAP && row.1 < part.center().y, "the row sits at the top of the part: {painted:?}");
    let (_fixture, _, kept) = shown_editor("one", None, false);
    let kept = kept.borrow().clone();
    let row = kept.rows.first().copied().expect("the text has a row");
    assert!(row.0 < part.center().y && row.1 > part.center().y, "without the option the row is centred on the part: {kept:?}");
}

/// The outcome's origin is where the layout is drawn: at rest its rows start inside the part, and once the text is
/// longer than the part and scrolled to the caret at its end, the origin moves up with the text, so the last row is
/// the one in view at the part's bottom.
#[trial]
fn the_origin_follows_the_text_where_it_is_drawn() {
    let lines = (1..=12).map(|line| line.to_string()).collect::<Vec<_>>().join("\n");
    let (mut fixture, value, painted) = editor(&lines, None);
    let rest = painted.borrow().clone();
    let part = rest.part.expect("the part is laid out");
    let first = rest.rows.first().copied().expect("the text has rows");
    assert!(part.expand(SNAP).contains(rest.origin.expect("the layout is placed")), "at rest the layout starts in the part: {rest:?}");
    assert!(first.0 >= part.top() - SNAP && first.1 <= part.bottom() + SNAP, "the first row shows in the part: {rest:?}");

    // Focus puts the caret at the end of plain text.
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.type_text("!");
    fixture.idle(3);
    assert!(value.borrow().ends_with("12!"), "the typing lands at the end: {:?}", value.borrow());
    let scrolled = painted.borrow().clone();
    let last = scrolled.rows.last().copied().expect("the text has rows");
    assert!(scrolled.origin.is_some_and(|origin| origin.y < part.top()), "the origin moves up with the scrolled text: {scrolled:?}");
    assert!(last.1 <= part.bottom() + SNAP && last.0 >= part.top() - SNAP, "the caret's row, the last, is drawn in the part: {scrolled:?}");
}
