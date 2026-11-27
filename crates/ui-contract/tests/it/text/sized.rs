//! A single-line text part set at a size of the caller's own: its row is taller than the role's and still lies in the
//! part, on the part's centre line where the role's own row sits.

use craft_behavior::{PartKey, PlainText, Sense};
use craft_ui::contract::{
    Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Input, MeasureCx, PaintUi, PartsCx, Props as _, Role, Show as _,
};
use craft_ui::primitives::{TextKey, TextOptions};
use craft_ui::style::StyleCx;
use craft_ui::testing::{Fixture, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Rect, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

const TEXT: PartKey = PartKey::new("text");

/// A size of the caller's own, well above the body role's: its row is much taller than the role's.
const LARGE: f32 = 40.0;

/// A part far taller than the text it shows, to read off the row the text takes.
const ROOMY: f32 = 200.0;

/// Half a point: rects compared across passes may round apart by that much.
const SNAP: f32 = 0.5;

/// What the trial's paint read of the text part: the part's rect.
#[derive(Clone, Copy, Debug, Default)]
struct Painted {
    part: Option<Rect>,
}

/// A single-line text part showing a value at a size the caller sets.
#[derive(Component)]
struct Sized<'a> {
    #[common]
    common: Common,
    value: Bind<'a, String>,
    size: Option<f32>,
    height: f32,
    painted: &'a RefCell<Painted>,
}

impl Component for Sized<'_> {
    const ROLE: Role = Role::TEXT_INPUT;
    const INPUT: Input = Input::Text;
    const OVERLAYS: bool = false;
    type Style = PlainStyle;
    type Output = EditResponse<String>;

    fn style(_: &StyleCx<'_>, _: SizedStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    /// The part's room: a component sizes the part for a text of the caller's own size, as tall as its row.
    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(240.0, self.height)
    }

    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let options = TextOptions { size: self.size, ..TextOptions::default() };
        let (shown, rect) = (cx.value().cloned().unwrap_or_default(), cx.content_rect);
        cx.text(TextKey::<String>::new(TEXT), rect, &PlainText, &shown, &options);
    }

    fn bind(&mut self) -> Option<BindMut<'_, String>> {
        Some(self.value.reborrow())
    }

    fn paint(&mut self, _: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if let Some(outcome) = cx.text(&TextKey::<String>::new(TEXT)) {
            self.painted.borrow_mut().part = Some(outcome.rect);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<String> {
        cx.edit_response()
    }
}

/// The part "42" is shown in at `size` in a part `height` tall, its painted row, and the clip the row is painted under.
fn sized(size: Option<f32>, height: f32) -> (Fixture, Rect, Rect, Rect) {
    let (value, painted) = (Rc::new(RefCell::new(String::from("42"))), Rc::new(RefCell::new(Painted::default())));
    let (shown, read) = (value.clone(), painted.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut held = shown.borrow_mut();
        Sized { common: Common::new(), value: Bind::bound(&mut *held), size, height, painted: &read }.id_salt("sized").show(ui);
    });
    fixture.settle();
    let part = painted.borrow().part.expect("the text part is laid out");
    let (row, clip) = {
        let painted = fixture.texts().iter().find(|text| text.text == "42").expect("the value is painted");
        (painted.rect, painted.clip)
    };
    (fixture, part, row, clip)
}

/// A size of the caller's own sets a row taller than the role's, and the part a component sizes for it is as tall as
/// that row. The row keeps its place in the part, centred on the part's centre line where the role's own row sits, and
/// the clip that follows it stays inside: a baseline taken from the role would carry the taller row above the part's
/// top. Without a size the role's row keeps its place.
#[trial]
fn a_single_line_part_set_at_its_own_size_keeps_its_row_in_its_part() {
    // A part far taller than the text shows what the row it lays out takes, and how much room a measure leaves the part.
    let (_, roomy, taken, _) = sized(Some(LARGE), ROOMY);
    let row_room = taken.height() + (ROOMY - roomy.height());
    let (_sized, part, row, clip) = sized(Some(LARGE), row_room);
    let (_role, role_part, role_row, _) = sized(None, ROOMY);
    assert!((part.height() - row.height()).abs() <= SNAP, "the part is as tall as the sized row: {part:?} vs {row:?}");
    assert!(row.height() > role_row.height() * 1.5, "{LARGE} points sets a taller row than the role's: {row:?} vs {role_row:?}");
    assert!(
        role_row.top() >= role_part.top() - SNAP && role_row.bottom() <= role_part.bottom() + SNAP,
        "the role's row lies in its part: {role_row:?} in {role_part:?}"
    );
    assert!((role_row.center().y - role_part.center().y).abs() <= SNAP, "the role's row is centred on its part: {role_row:?} in {role_part:?}");
    assert!(row.top() >= part.top() - SNAP && row.bottom() <= part.bottom() + SNAP, "the sized row lies in its part: {row:?} in {part:?}");
    assert!((row.center().y - part.center().y).abs() <= SNAP, "the sized row is centred on its part: {row:?} in {part:?}");
    assert!(
        clip.top() >= part.top() - SNAP && clip.bottom() <= part.bottom() + SNAP,
        "the sized text is not clipped by a rect beyond the part: {clip:?} in {part:?}"
    );
}
