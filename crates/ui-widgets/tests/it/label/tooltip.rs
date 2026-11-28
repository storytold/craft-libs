//! The tooltip of a control that paints no label: its own label shows there, inside a field or not; a control named
//! only by its field label shows none, and the field's label stays where it was. A bound command's hint is the
//! tooltip's second line, and makes a disabled control a focus stop that shows it. A label the layout cut shows its full
//! text as its tooltip.

use super::kit::PlainStyle;
use craft_behavior::Sense;
use craft_ui::buttons::Button;
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{
    CommandSet, Common, Component, Cx, GlyphLabel, LabelKind, LabelScheme, MeasureCx, Props as _, Role, Show as _, ShowContainer as _, SideLabel,
};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::runtime::commands::CommandState;
use craft_ui::style::StyleCx;
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{Fixture, trial};
use craft_ui::tile_grid::{CaptionPlace, Tile};
use egui::{Id, Key, Modifiers, Pos2, Rect, Response, Ui, Vec2};
use std::cell::Cell;
use std::rc::Rc;

/// A mark that paints no text.
#[derive(craft_ui::contract::Component)]
struct Mark {
    #[common]
    common: Common,
}

impl Component for Mark {
    const ROLE: Role = Role::IMAGE;
    const SHOWS_LABEL: bool = false;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: MarkStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(24.0)
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// The texts the pointer's rest on what `show` shows (it returns its rect) adds after the tooltip delay.
fn texts_added_by_hover(show: impl Fn(&mut Ui) -> Rect + 'static) -> Vec<String> {
    texts_added_by_hover_in(show, |_| {})
}

/// [`texts_added_by_hover`] in a fixture `prepare` sets up first (the state of a command).
fn texts_added_by_hover_in(show: impl Fn(&mut Ui) -> Rect + 'static, prepare: impl FnOnce(&mut Fixture)) -> Vec<String> {
    let at = Rc::new(Cell::new(Rect::NOTHING));
    let seen = Rc::clone(&at);
    let mut fixture = Fixture::new(move |ui| seen.set(show(ui)));
    prepare(&mut fixture);
    fixture.settle();
    // A cut run carries its full text too: runs are told apart by whether they are cut.
    let before: Vec<(String, bool)> = fixture.texts().iter().map(|run| (run.text.clone(), run.elided)).collect();
    let placed: Vec<(String, Rect)> = fixture.texts().iter().map(|run| (run.text.clone(), run.rect)).collect();
    fixture.frame(vec![moved(at.get().center())]);
    fixture.advance(DELAY);
    fixture.settle();
    // The tooltip shows on its own layer: every text shown before stays where it was, a field's label included.
    for shown in &placed {
        assert!(
            fixture.texts().iter().any(|run| run.text == shown.0 && run.rect == shown.1),
            "{:?} stays at {:?} under the tooltip",
            shown.0,
            shown.1
        );
    }
    let after = fixture.texts().iter().map(|run| (run.text.clone(), run.elided));
    after.filter(|run| !before.contains(run)).map(|(text, _)| text).collect()
}

/// A mark in the field "Layer", with `own` as its own label.
fn mark_in_field(own: Option<&'static str>) -> Vec<String> {
    texts_added_by_hover(move |ui| {
        let shown = Field::new("Layer").show(ui, |ui, _| {
            let mark = Mark { common: Common::new() };
            let mark = match own {
                Some(label) => mark.label(label),
                None => mark,
            };
            mark.show(ui).rect
        });
        shown.inner.unwrap_or(Rect::NOTHING)
    })
}

/// A mark in the field "Leading", whose label shows as a glyph at the row's start, with `hint` as its tooltip.
fn glyph_label(hint: Option<&'static str>) -> Vec<String> {
    texts_added_by_hover(move |ui| {
        let side = SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::text("A")));
        let side = match hint {
            Some(hint) => side.tooltip(hint),
            None => side,
        };
        Field::new("Leading").side(side).show(ui, |ui, _| Mark { common: Common::new() }.show(ui));
        // The glyph sits at the start of the row.
        Rect::from_min_size(Pos2::ZERO, Vec2::new(8.0, 24.0))
    })
}

#[trial]
fn a_control_that_paints_no_label_shows_its_own_label_as_tooltip() {
    assert_eq!(mark_in_field(Some("Lock")), ["Lock"], "a mark with its own label shows it inside a field");
    assert!(mark_in_field(None).is_empty(), "a mark the field names shows no tooltip");

    let tile = texts_added_by_hover(|ui| {
        let rect = Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::splat(96.0));
        Tile::new("Harbour at dusk").caption(CaptionPlace::None).show_at(ui, rect);
        rect
    });
    assert_eq!(tile, ["Harbour at dusk"], "a tile with no caption shows its name");

    let long = "Harbour at dusk, seen from the lighthouse on the northern breakwater";
    let cut = texts_added_by_hover(move |ui| {
        let rect = Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::new(96.0, 128.0));
        Tile::new(long).show_at(ui, rect);
        rect
    });
    assert_eq!(cut, [long], "a tile whose caption is cut shows its whole name");

    assert_eq!(glyph_label(None), ["Leading"], "a glyph label shows its name");
    assert_eq!(glyph_label(Some("Leading (1/1000 em)")), ["Leading (1/1000 em)"], "a glyph label's tooltip replaces its name");
}

/// A label the layout cut shows its full text as the tooltip of a rest on it.
#[trial]
fn an_elided_label_shows_its_full_text_as_its_tooltip() {
    let long = "Harbour at dusk, seen from the lighthouse on the northern breakwater";
    let shown = texts_added_by_hover(move |ui| ui.allocate_ui(Vec2::new(72.0, 32.0), |ui| Button::new(long).show(ui).rect).inner);
    assert_eq!(shown, [long], "the cut label's tooltip is its whole text, shown once");
}

const SPOT: craft_behavior::PartKey = craft_behavior::PartKey::new("spot");

/// A layer of pointer-only spots over content it does not paint (comments over a page): it takes no room of its own,
/// is shown at a rect, and each spot carries a tip.
#[derive(craft_ui::contract::Component)]
struct Spots {
    #[common]
    common: Common,
    spot: Rect,
    at: craft_ui::contract::TooltipAt,
}

impl Component for Spots {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: SpotsStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn parts(&mut self, _: Rect, cx: &mut craft_ui::contract::PartsCx<'_, Self>) {
        let tip = craft_ui::contract::TooltipSpec::new(craft_ui::runtime::env::Label::from("Alpha\nUnchecked comment")).at(self.at);
        let _ =
            cx.part(SPOT, self.spot).sense(Sense::CLICK | Sense::CONTEXT_CLICK | Sense::DRAG).tooltip(tip).pointer_only("a panel reaches the spots");
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// A part's tip shows when the pointer rests on the part, hung from the part or from the pointer, also on a component
/// that takes no room of its own and is shown at a rect.
#[trial]
fn a_part_tip_shows_on_a_rest_on_the_part() {
    use craft_ui::contract::TooltipAt;
    for at in [TooltipAt::Pointer, TooltipAt::Control] {
        let spot = Rect::from_min_size(Pos2::new(60.0, 60.0), Vec2::splat(30.0));
        let added = texts_added_by_hover(move |ui| {
            Spots { common: Common::new(), spot, at }.show_at(ui, Rect::from_min_size(Pos2::ZERO, Vec2::splat(200.0)));
            spot
        });
        assert!(added.iter().any(|text| text.contains("Alpha")), "the spot's tip shows ({at:?}): {added:?}");
    }
}

/// The command the hint trials bind: an app command with a chord.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "tooltipHintTrial")]
#[non_exhaustive]
enum HintCommand {
    #[command(label = "Bold", default = "Cmd+B", modal = "always")]
    Bold,
}

const HINT: &str = "Make the selected text heavier";

/// The lines of the texts `added` holds, in order: a run with a line break counts as its lines.
fn lines(added: &[String]) -> Vec<String> {
    added.iter().flat_map(|text| text.lines().map(str::to_owned)).filter(|line| !line.is_empty()).collect()
}

#[trial]
fn a_command_hint_is_the_tooltips_second_line() {
    let button = || {
        |ui: &mut Ui| {
            ui.add_space(20.0);
            Button::command(HintCommand::Bold).show(ui).rect
        }
    };
    let hinted = texts_added_by_hover_in(button(), |fixture| {
        fixture.command_state(HintCommand::Bold, CommandState::new().hint(HINT));
    });
    let shown = lines(&hinted);
    assert_eq!(shown.len(), 2, "the tooltip has two lines: {shown:?}");
    assert!(shown[0].contains("Bold") && shown[0] != "Bold", "the first is the label with its chord added: {shown:?}");
    assert!(shown[1].contains(HINT), "the second is the hint: {shown:?}");

    let plain = texts_added_by_hover_in(button(), |fixture| {
        fixture.command_state(HintCommand::Bold, CommandState::new());
    });
    assert!(plain.is_empty(), "with no hint a button that paints its label shows no tooltip: {plain:?}");
}

/// The stops Tab reaches, in order, among the buttons Before, Bold (disabled, bound to `HintCommand::Bold`) and After,
/// and the texts the tooltip adds once the keyboard rests on Bold.
fn tab_through_disabled(hint: Option<&'static str>) -> (Vec<&'static str>, Vec<String>) {
    let ids = Rc::new(Cell::new([Id::NULL; 3]));
    let seen = Rc::clone(&ids);
    let mut fixture = Fixture::new(move |ui| {
        let before = Button::new("Before").show(ui).id;
        let bold = Button::command(HintCommand::Bold).show(ui).id;
        let after = Button::new("After").show(ui).id;
        seen.set([before, bold, after]);
    });
    let state = CommandState::new().enabled(false);
    fixture.command_state(HintCommand::Bold, hint.map_or(state.clone(), |hint| state.hint(hint)));
    fixture.settle();
    let names = ["Before", "Bold", "After"];
    let mut reached = Vec::new();
    let mut added = Vec::new();
    for _ in 0..3 {
        fixture.key(Key::Tab, Modifiers::NONE);
        fixture.settle();
        let Some(at) = fixture.focused().and_then(|focused| ids.get().iter().position(|id| *id == focused)) else { continue };
        reached.push(names[at]);
        if at == 1 {
            let before: Vec<String> = fixture.texts().iter().map(|run| run.text.clone()).collect();
            fixture.advance(DELAY);
            fixture.settle();
            added = fixture.texts().iter().map(|run| run.text.clone()).filter(|text| !before.contains(text)).collect();
        }
    }
    (reached, added)
}

#[trial]
fn a_disabled_command_with_a_hint_is_a_tab_stop_that_shows_its_reason() {
    let (reached, added) = tab_through_disabled(Some(HINT));
    assert_eq!(reached, ["Before", "Bold", "After"], "Tab reaches the disabled button with a hint");
    assert!(lines(&added).iter().any(|line| line == HINT), "and its tooltip shows the hint on keyboard focus: {added:?}");

    let (reached, _) = tab_through_disabled(None);
    assert_eq!(&reached[..2], ["Before", "After"], "without a hint Tab skips the disabled button");
}

/// The tooltip of a control inside a body (a canvas control over a page body, its tip kept at the screen's bottom
/// edge): its layer is not part of the body's flow, so the body's kept transcript does not check the tip's text. Once
/// it shows, it keeps one rect and every frame runs one pass, none a discard pass for a tip that measured another
/// size.
#[trial]
fn a_tooltip_shown_from_inside_a_body_keeps_its_rect_frame_to_frame() {
    let mut fixture = Fixture::new(|ui| {
        craft_ui::layout_components::Stack::new().show(ui, |ui, _| {
            Mark { common: Common::new() }.label("Lock").show_at(ui, Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 380.0)));
        });
    });
    fixture.room(Vec2::new(600.0, 400.0));
    fixture.settle();
    fixture.frame(vec![moved(Pos2::new(300.0, 200.0))]);
    fixture.advance(DELAY);
    fixture.settle();
    assert!(fixture.texts().iter().any(|run| run.text == "Lock"), "the tooltip shows");
    let mut frames = Vec::new();
    for _ in 0..6 {
        fixture.frame(Vec::new());
        frames.push((fixture.passes(), fixture.texts().iter().find(|run| run.text == "Lock").map(|run| run.rect)));
    }
    let first = frames[0].1;
    assert!(first.is_some() && frames.iter().all(|(passes, rect)| *passes == 1 && *rect == first), "one pass a frame and one rect: {frames:?}");
}

/// A tooltip longer than a mark's cell and far shorter than the layer's cap.
const STROKE_TIP: &str = "Set shape stroke type";

/// The width of the tooltip a rest on a mark labelled `STROKE_TIP` shows in a wide window, with `inspector` in an
/// inspector's field, whose control fills the cell its label leaves; and the window's width.
fn stroke_tip_width(inspector: bool) -> (f32, f32) {
    let at = Rc::new(Cell::new(Rect::NOTHING));
    let seen = Rc::clone(&at);
    let mut fixture = Fixture::new(move |ui| {
        let field = |ui: &mut Ui| {
            Field::new("Stroke").show(ui, |ui, _| seen.set(Mark { common: Common::new() }.label(STROKE_TIP).show(ui).rect));
        };
        if inspector {
            FieldGroup::new("Shape").labels(LabelScheme::Inspector).show(ui, |ui, _| field(ui));
        } else {
            field(ui);
        }
    });
    let room = Vec2::new(1600.0, 600.0);
    fixture.room(room);
    fixture.settle();
    fixture.frame(vec![moved(at.get().center())]);
    fixture.advance(DELAY);
    fixture.settle();
    let tip = fixture.by_role(Role::TOOLTIP).first().map(|tip| tip.rect).expect("the tooltip shows");
    (tip.width(), room.x)
}

/// A control beside an inspector label fills its cell, its tooltip does not: the tooltip layer takes its text's width,
/// as it does outside an inspector, not the layer's cap of the window (photocraft#16593).
#[trial]
fn a_tooltip_of_a_control_in_an_inspector_cell_keeps_its_text_width() {
    let (plain, _) = stroke_tip_width(false);
    let (inspector, room) = stroke_tip_width(true);
    assert!(inspector <= plain + 0.5, "the tooltip in an inspector cell is as wide as outside one: {inspector} vs {plain}");
    assert!(inspector < room / 4.0, "the tooltip does not span the window: {inspector} of {room}");
}

/// A press on a control hides its tooltip until the pointer leaves the control, so what the press opens (a popover
/// under a swatch) is not covered; a rest after the pointer came back shows it again.
#[trial]
fn a_press_on_a_control_hides_its_tooltip_until_the_pointer_leaves() {
    let at = Rc::new(Cell::new(Rect::NOTHING));
    let seen = Rc::clone(&at);
    let mut fixture = Fixture::new(move |ui| {
        ui.add_space(20.0);
        seen.set(Button::new("Stroke").tooltip(STROKE_TIP.to_owned()).show(ui).rect);
    });
    fixture.settle();
    let shows = |fixture: &Fixture| fixture.texts().iter().any(|run| run.text == STROKE_TIP);
    let center = at.get().center();
    fixture.frame(vec![moved(center)]);
    fixture.advance(DELAY);
    fixture.settle();
    assert!(shows(&fixture), "a rest on the button shows its tooltip");

    fixture.click_at(center);
    fixture.advance(DELAY);
    fixture.settle();
    assert!(!shows(&fixture), "after a press on the button its tooltip stays hidden while the pointer rests there");

    fixture.frame(vec![moved(at.get().right_bottom() + Vec2::splat(200.0))]);
    fixture.advance(DELAY);
    fixture.settle();
    fixture.frame(vec![moved(center)]);
    fixture.advance(DELAY);
    fixture.settle();
    assert!(shows(&fixture), "once the pointer left and came back, a rest shows the tooltip again");
}
