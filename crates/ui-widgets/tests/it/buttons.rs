//! Buttons: the stacked arrangement's height, the hover report of a click the pointer stays on, the press a button
//! reports as held, the compact group level, under which an inline button drops its label and keeps it as its name and
//! tooltip, the room a button keeps for its widest text, the chord a command button shows, and a picture in the leading
//! icon's place.

use craft_behavior::{IconId, PartKey};
use craft_theme::ControlSize;
use craft_theme::Variant;
use craft_ui::buttons::{
    Arrangement, Button, ButtonChord, ControlPlace, GroupLevel, IconButton, PushButtonLook, SplitButton, SplitMenu, ToggleButton,
};
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{Bind, CommandSet, Entry, Hover, PopupKind, Props as _, Role, Show as _, ShowContainer as _, Trigger};
use craft_ui::display::Picture;
use craft_ui::list::ThumbAspect;
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::pickers::ComboBox;
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::commands::{AppCatalog, Binding, CommandDef, CommandInput, CommandRef, CommandState, Commands as _, Scope};
use craft_ui::runtime::env::{Env, InBar, in_room, with_env};
use craft_ui::testing::input::key_press;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::pictures::sample_picture;
use craft_ui::testing::pointer::{moved, press, release, window_blurred};
use craft_ui::testing::{Fixture, Frames, trial};
use egui::{Event, Id, Key, Modifiers, Pos2, Rect, Response, Ui, pos2, vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub(crate) mod detail_wrapping {
    use super::{Button, Cell, Fixture, Frames, IconId, Key, Modifiers, Rc, Rect, vec2};
    use craft_behavior::Proposal;
    use craft_ui::contract::{LeastWidth, Props as _, Role, Show as _, ShowContainer as _, Sizing};
    use craft_ui::layout_components::Stack;
    use craft_ui::testing::{Target, focus::AccessActionKind, trial};

    const DETAIL: &str = "Learn the essential tools. Choose a skill to see the tutorials for that skill.";

    fn wrapped_detail(fixture: &mut Fixture, title: &str) -> Target {
        let shapes = fixture.frame_shapes();
        let detail = shapes
            .iter()
            .find_map(|shape| match shape {
                egui::Shape::Text(text) if text.galley.text() == DETAIL => Some(text),
                _ => None,
            })
            .expect("the whole detail paints");
        let button = fixture.by_label(title).expect("the button is named");
        assert!(detail.galley.rows.len() > 1, "the detail wraps across lines");
        assert!(button.rect.expand(0.5).contains_rect(detail.visual_bounding_rect()), "the detail fits inside the button");
        button
    }

    #[trial]
    fn a_button_detail_stays_on_one_line_without_the_opt_in() {
        let mut fixture = Fixture::new(|ui| {
            Button::new("Quick guide").detail(DETAIL).sizing(Sizing::FillWidth).show(ui);
        });
        fixture.room(vec2(172.0, 220.0)).settle();
        let rows = fixture.frame_shapes().iter().find_map(|shape| match shape {
            egui::Shape::Text(text) if text.galley.text() == DETAIL => Some(text.galley.rows.len()),
            _ => None,
        });
        assert_eq!(rows, Some(1), "ordinary button detail remains a single line");
    }

    #[trial]
    fn a_wrapped_button_detail_fits_its_face_and_keeps_one_activation_target() {
        const TITLE: &str = "Quick guide";
        let activations = Rc::new(Cell::new(0));
        let seen = Rc::clone(&activations);
        let mut fixture = Fixture::new(move |ui| {
            let button = Button::new(TITLE).icon(IconId::new("copy")).detail(DETAIL).wrap_detail(true).sizing(Sizing::FillWidth);
            if button.show(ui).clicked() {
                seen.set(seen.get() + 1);
            }
        });
        fixture.room(vec2(172.0, 220.0)).settle();
        let button = wrapped_detail(&mut fixture, TITLE);
        assert_eq!(fixture.tree().description_of(button.node), Some(DETAIL));
        assert_eq!(fixture.by_role(Role::BUTTON).len(), 1);

        fixture.click(&button).settle();
        assert_eq!(activations.get(), 1);
        fixture.access(AccessActionKind::Focus, &button);
        assert_eq!(fixture.tree().focused_key(), button.node);
        fixture.key(Key::Enter, Modifiers::NONE).settle();
        assert_eq!(activations.get(), 2, "clicking and pressing Enter each activate the button once");
    }

    #[trial]
    fn a_wrapped_button_detail_measures_at_an_explicit_width() {
        let width = 134.0;
        for proposal in [Proposal::Width(width), Proposal::Size(vec2(width, 100.0))] {
            let mut fixture = Fixture::new(move |ui| {
                let mut button = Button::new("Quick guide").detail(DETAIL).wrap_detail(true);
                let height = button.measure_in(ui, proposal).y;
                let room = Rect::from_min_size(ui.cursor().min, vec2(width, height));
                button.show_at(ui, room);
            });
            fixture.room(vec2(440.0, 220.0)).settle();
            wrapped_detail(&mut fixture, "Quick guide");
        }
    }

    #[trial]
    fn a_wrapped_button_detail_keeps_explicit_unbounded_queries_unwrapped() {
        let sizes = |width| {
            let mut frames = Frames::new();
            let mut out = [egui::Vec2::ZERO; 4];
            frames.idle(1, |ui| {
                let room = Rect::from_min_size(ui.cursor().min, vec2(width, 380.0));
                Stack::new().fill().show_at(ui, room, |ui, _| {
                    let mut button = Button::new("Quick guide").detail(DETAIL).wrap_detail(true);
                    out = [Proposal::Ideal, Proposal::Width(f32::INFINITY), Proposal::Size(vec2(f32::INFINITY, 300.0)), Proposal::Width(width)]
                        .map(|proposal| button.measure_in(ui, proposal));
                });
            });
            out
        };
        let narrow = sizes(134.0);
        let wide = sizes(580.0);
        assert_eq!(narrow[..3], wide[..3], "the unbounded queries are independent of their parent");
        assert!(narrow[3].y > narrow[0].y, "the explicit finite width wraps the caption");
    }

    #[trial]
    fn a_wrapped_button_detail_measures_its_minimum_height_at_its_style_width_floor() {
        let mut frames = Frames::new();
        frames.idle(1, |ui| {
            let mut button = Button::new("Quick guide").detail(DETAIL).wrap_detail(true).least_width(LeastWidth::SideMenu);
            let minimum = button.measure_in(ui, Proposal::Min);
            let at_width = button.measure_in(ui, Proposal::Width(minimum.x));
            assert!((minimum.y - at_width.y).abs() < 0.5, "the minimum height follows the returned width: {minimum:?}, {at_width:?}");
        });
    }

    #[trial]
    fn a_wrapped_button_detail_measures_its_minimum_height_at_its_reserved_title_width() {
        let mut frames = Frames::new();
        frames.idle(1, |ui| {
            let mut button = Button::new("I").detail(DETAIL).wrap_detail(true).room_for("WWWWWWWWW");
            let minimum = button.measure_in(ui, Proposal::Min);
            let at_width = button.measure_in(ui, Proposal::Width(minimum.x));
            assert!((minimum.y - at_width.y).abs() < 0.5, "the caption uses the reserved width: {minimum:?}, {at_width:?}");
        });
    }
}

/// The button kinds that take an arrangement.
#[derive(Clone, Copy, Debug)]
enum Kind {
    Button,
    Toggle,
    Split,
}

/// One stacked control of `kind` with `label` at `size`, shown alone: its rect and the rects of the label lines it
/// paints, in paint order.
fn stacked(kind: Kind, label: &'static str, size: ControlSize) -> (Rect, Vec<Rect>) {
    let at = Rc::new(Cell::new(Id::NULL));
    let seen = Rc::clone(&at);
    let mut fixture = Fixture::new(move |ui| {
        let icon = IconId::new("copy");
        let id = match kind {
            Kind::Button => Button::new(label).icon(icon).arrangement(Arrangement::Stacked).size(size).show(ui).response.id,
            Kind::Toggle => {
                let mut on = false;
                ToggleButton::new(label, Bind::bound(&mut on)).icon(icon).arrangement(Arrangement::Stacked).size(size).show(ui).response.id
            }
            Kind::Split => {
                let entries = [Entry::new(0u8, "Keep source formatting"), Entry::new(1u8, "Keep text only")];
                SplitButton::new(label, &entries)
                    .icon(icon)
                    .arrow_label("Paste options")
                    .arrangement(Arrangement::Stacked)
                    .size(size)
                    .show(ui)
                    .response
                    .id
            }
        };
        seen.set(id);
    });
    fixture.settle();
    // The control's node bounds both of a split button's parts.
    let control = fixture.node_of(at.get()).expect("the control is in the access tree").rect;
    let lines = fixture.texts().iter().filter(|run| !run.text.trim().is_empty()).map(|run| (run.text.clone(), run.rect));
    let lines: Vec<(String, Rect)> = lines.collect();
    for (text, _) in &lines {
        assert!(label.contains(text.as_str()), "{kind:?} paints only its label's lines: {text:?} is not part of {label:?}");
    }
    (control, lines.into_iter().map(|(_, rect)| rect).collect())
}

#[trial]
fn a_stacked_label_stays_inside_the_stacked_height_in_every_density() {
    for kind in [Kind::Button, Kind::Toggle, Kind::Split] {
        for size in [ControlSize::Sm, ControlSize::Md] {
            let (one, one_lines) = stacked(kind, "Paste", size);
            let (two, two_lines) = stacked(kind, "Insert page break", size);
            assert_eq!(one_lines.len(), 1, "{kind:?} at {size:?}: a short label keeps one line");
            assert_eq!(two_lines.len(), 2, "{kind:?} at {size:?}: a long label breaks into two lines");
            for (control, lines) in [(one, &one_lines), (two, &two_lines)] {
                for line in lines.iter() {
                    assert!(control.expand(0.5).contains_rect(*line), "{kind:?} at {size:?}: the line {line:?} lies inside {control:?}");
                }
                // The icon sits above the label: at least a line's height of the control lies over the first line.
                let first = lines[0];
                assert!(
                    first.min.y - control.min.y >= first.height(),
                    "{kind:?} at {size:?}: the icon is above the label ({first:?} in {control:?})"
                );
            }
            assert!(two_lines[1].min.y >= two_lines[0].max.y - 0.5, "{kind:?} at {size:?}: the second line is under the first");
            assert!(two.height() > one.height() + 0.5, "{kind:?} at {size:?}: two lines make a taller control ({two:?} against {one:?})");
        }
    }
}

/// A stacked label breaks where it names its break (`"Page\nNumber"`), whatever its length, and each part shows
/// whole on its own line: the line break is not elided into the first line.
#[trial]
fn a_stacked_label_breaks_at_its_own_line_break() {
    for kind in [Kind::Button, Kind::Toggle, Kind::Split] {
        let (control, lines) = stacked(kind, "Page\nNumber", ControlSize::Md);
        assert_eq!(lines.len(), 2, "{kind:?}: the label breaks at its newline");
        assert!(lines[1].min.y >= lines[0].max.y - 0.5, "{kind:?}: the second part is under the first");
        for line in &lines {
            assert!(control.expand(0.5).contains_rect(*line), "{kind:?}: the line {line:?} lies inside {control:?}");
        }
    }
}

/// In a ribbon, stacked buttons share one height whether their labels take one line or two, and lay out from their
/// tops, so their icons and first lines sit on one line across the row, as the ribbon's large buttons did.
#[trial]
fn a_ribbons_stacked_buttons_share_one_height_and_one_first_line() {
    let ids = Rc::new(RefCell::new(Vec::<Id>::new()));
    let seen = Rc::clone(&ids);
    let mut fixture = Fixture::new(move |ui| {
        in_ribbon(ui, |ui| {
            let one = Button::new("Pictures").icon(IconId::new("copy")).arrangement(Arrangement::Stacked).show(ui).response.id;
            let two = Button::new("Page\nNumber").icon(IconId::new("copy")).arrangement(Arrangement::Stacked).show(ui).response.id;
            *seen.borrow_mut() = vec![one, two];
        });
    });
    fixture.settle();
    let rects: Vec<Rect> = ids.borrow().iter().filter_map(|id| fixture.node_of(*id).map(|node| node.rect)).collect();
    assert_eq!(rects.len(), 2, "both buttons show");
    assert!((rects[0].height() - rects[1].height()).abs() < 0.5, "one height for one and two lines: {rects:?}");
    let top = |text: &str| fixture.texts().iter().find(|run| run.text == text).map_or(f32::NAN, |run| run.rect.min.y);
    let (one, two) = (top("Pictures") - rects[0].min.y, top("Page") - rects[1].min.y);
    assert!((one - two).abs() < 0.5, "the first lines sit as far under each top: {one} against {two}");
}

/// A fixture showing one toggle, the hover reports of each pass of a frame, and a switch that reruns the next
/// frame's pass.
struct HoverTrial {
    fixture: Fixture,
    reports: Rc<RefCell<Vec<Hover>>>,
    rerun: Rc<Cell<bool>>,
}

impl HoverTrial {
    fn new() -> Self {
        let reports = Rc::new(RefCell::new(Vec::new()));
        let rerun = Rc::new(Cell::new(false));
        let on = Rc::new(Cell::new(false));
        let (seen, again) = (Rc::clone(&reports), Rc::clone(&rerun));
        let mut fixture = Fixture::new(move |ui| {
            let mut value = on.get();
            let shown = ToggleButton::new("Snap to grid", Bind::bound(&mut value)).icon(IconId::new("eye")).show(ui);
            on.set(value);
            seen.borrow_mut().push(shown.hover);
            if again.get() && ui.ctx().current_pass_index() == 0 {
                ui.ctx().request_discard("the trial reruns the pass");
            }
        });
        fixture.settle();
        HoverTrial { fixture, reports, rerun }
    }

    /// One frame with `events`; the reports of its passes, first pass first.
    fn frame(&mut self, events: Vec<Event>) -> Vec<Hover> {
        self.reports.borrow_mut().clear();
        self.fixture.frame(events);
        self.reports.borrow().clone()
    }

    /// [`HoverTrial::frame`], its pass run twice.
    fn frame_twice(&mut self, events: Vec<Event>) -> Vec<Hover> {
        self.rerun.set(true);
        let reports = self.frame(events);
        self.rerun.set(false);
        assert_eq!(self.fixture.passes(), 2, "the frame ran a second pass");
        reports
    }
}

#[trial]
fn a_click_holds_until_the_pointer_leaves() {
    let mut trial = HoverTrial::new();
    let toggle = trial.fixture.by_label("Snap to grid").expect("the toggle is in the access tree").center();
    let away = Pos2::new(390.0, 290.0);

    let entered = trial.frame_twice(vec![moved(toggle)]);
    assert!(entered[0].over && entered[0].entered && !entered[0].clicked, "the pointer enters the toggle: {:?}", entered[0]);
    assert!(entered[1].over && !entered[1].entered, "a second pass applies no input of its own: {:?}", entered[1]);

    let pressed = trial.frame(vec![press(toggle)]);
    assert!(pressed.iter().all(|hover| hover.over && !hover.clicked), "a press is not a click yet: {pressed:?}");

    let clicked = trial.frame_twice(vec![release(toggle)]);
    assert!(clicked.iter().all(|hover| hover.over && hover.clicked), "the click shows in both passes of its frame: {clicked:?}");

    let staying = trial.frame(Vec::new());
    assert!(staying.iter().all(|hover| hover.over && hover.clicked && !hover.entered), "the click holds while the pointer stays: {staying:?}");

    let left = trial.frame(vec![moved(away)]);
    assert!(!left[0].over && left[0].left && !left[0].clicked, "leaving drops the click: {:?}", left[0]);

    let back = trial.frame(vec![moved(toggle)]);
    assert!(back[0].over && back[0].entered && !back[0].clicked, "coming back is an entry, not the old click: {:?}", back[0]);
}

/// The command the compact split button's main part is bound to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "buttonsTrial")]
#[non_exhaustive]
enum TraceCommand {
    #[command(label = "Trace", default = "Cmd+Alt+T", modal = "always")]
    Trace,
}

/// The labelled inline button the compact trial shows.
fn save(ui: &mut Ui) -> Response {
    Button::new("Save").icon(IconId::new("save")).show(ui).response
}

/// The labelled split button the compact trial shows: its main part runs `TraceCommand::Trace`.
fn trace(ui: &mut Ui) -> Response {
    let entries = [Entry::new(0u8, "Black and white"), Entry::new(1u8, "Colour")];
    SplitButton::command(TraceCommand::Trace, SplitMenu::Entries(&entries))
        .icon(IconId::new("image"))
        .arrow_label("Tracing presets")
        .show(ui)
        .response
}

/// Shows `show` under `level`.
fn at_level(ui: &mut Ui, level: GroupLevel, show: fn(&mut Ui) -> Response) -> Response {
    with_env(
        ui,
        |overrides| {
            overrides.values.set(level);
        },
        show,
    )
}

/// What a control named `name` shows alone, inside a compact group or not.
struct Shown {
    width: f32,
    /// Whether its label is painted.
    painted: bool,
    /// Whether its name is in the access tree.
    named: bool,
    /// The lines the pointer's rest on its main part adds.
    tooltip: Vec<String>,
}

fn shown(name: &str, show: fn(&mut Ui) -> Response, compact: bool) -> Shown {
    let at = Rc::new(Cell::new(Id::NULL));
    let seen = Rc::clone(&at);
    let mut fixture = Fixture::new(move |ui| {
        let response = if compact { at_level(ui, GroupLevel::Compact, show) } else { show(ui) };
        seen.set(response.id);
    });
    fixture.command_state(TraceCommand::Trace, CommandState::new());
    fixture.settle();
    let painted = fixture.texts().iter().any(|run| run.text == name);
    let named = fixture.by_label(name).is_some();
    // The control's node bounds both of a split button's parts.
    let rect = fixture.node_of(at.get()).expect("the control is in the access tree").rect;
    let before: Vec<String> = fixture.texts().iter().map(|run| run.text.clone()).collect();
    // The main part is at the start, the arrow at the end.
    fixture.frame(vec![moved(Pos2::new(rect.min.x + rect.width() * 0.25, rect.center().y))]);
    fixture.advance(DELAY);
    fixture.settle();
    let added = fixture.texts().iter().map(|run| run.text.clone()).filter(|text| !before.contains(text));
    let tooltip = added.flat_map(|text| text.lines().map(str::to_owned).collect::<Vec<_>>()).filter(|line| !line.is_empty()).collect();
    Shown { width: rect.width(), painted, named, tooltip }
}

#[trial]
fn an_inline_button_drops_its_label_under_a_compact_group_and_names_itself_by_tooltip() {
    type Case<'a> = (&'a str, fn(&mut Ui) -> Response);
    let cases: [Case; 2] = [("Save", save), ("Trace", trace)];
    for (name, show) in cases {
        let full = shown(name, show, false);
        let compact = shown(name, show, true);
        assert!(full.painted, "{name} paints its label at full size");
        assert!(!compact.painted, "{name} paints no label in a compact group");
        assert!(compact.width < full.width - 0.5, "{name} is narrower in a compact group: {} against {}", compact.width, full.width);
        assert!(compact.named, "{name} keeps its label as its accessible name");
        assert!(
            compact.tooltip.iter().any(|line| line.starts_with(name)),
            "{name} shows its label as its tooltip in a compact group: {:?}",
            compact.tooltip
        );
    }

    // The level changes between two passes of one frame: the second pass measures at the new level.
    let compact = Rc::new(Cell::new(false));
    let switch = Rc::new(Cell::new(false));
    let widths = Rc::new(RefCell::new(Vec::new()));
    let (level, flip, seen) = (Rc::clone(&compact), Rc::clone(&switch), Rc::clone(&widths));
    let mut fixture = Fixture::new(move |ui| {
        let at = if level.get() { GroupLevel::Compact } else { GroupLevel::Full };
        seen.borrow_mut().push(at_level(ui, at, save).rect.width());
        if flip.get() && ui.ctx().current_pass_index() == 0 {
            flip.set(false);
            level.set(true);
            ui.ctx().request_discard("the trial changes the level between passes");
        }
    });
    fixture.settle();
    widths.borrow_mut().clear();
    switch.set(true);
    fixture.frame(Vec::new());
    assert_eq!(fixture.passes(), 2, "the frame ran a second pass");
    let passes = widths.borrow().clone();
    let settled = shown("Save", save, true).width;
    assert!(passes[1] < passes[0] - 0.5, "the full-size measurement is not reused at the compact level: {passes:?}");
    assert!((passes[1] - settled).abs() < 0.5, "the second pass measures as a settled compact button does: {passes:?} against {settled}");
}

#[trial]
fn a_button_with_room_for_its_widest_text_keeps_one_width_whatever_its_label() {
    let widths = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&widths);
    let mut fixture = Fixture::new(move |ui| {
        let mut shown = Vec::new();
        for label in ["1%", "500%"] {
            shown.push(Button::new(label).room_for("500%").id_salt(label).show(ui).response.rect.width());
        }
        shown.push(Button::new("1%").id_salt("no room").show(ui).response.rect.width());
        *seen.borrow_mut() = shown;
    });
    fixture.settle();
    let widths = widths.borrow().clone();
    let (least, most, alone) = (widths[0], widths[1], widths[2]);
    assert!((least - most).abs() < 0.5, "1% and 500% with room for 500% are equally wide: {least} against {most}");
    assert!(alone < least - 0.5, "1% without the room is narrower: {alone} against {least}");
}

#[trial]
fn a_click_on_a_buttons_picture_activates_the_button_which_stays_one_access_node() {
    let at = Rc::new(Cell::new(Id::NULL));
    let clicks = Rc::new(Cell::new(0));
    let (seen, counted) = (Rc::clone(&at), Rc::clone(&clicks));
    let mut fixture = Fixture::new(move |ui| {
        let image = || sample_picture(3, [96, 48]);
        let shown = Button::new("Equation")
            .picture(ThumbAspect::new(2, 1), Picture::new("Rendered equation", 3, &image))
            .arrangement(Arrangement::Stacked)
            .show(ui);
        seen.set(shown.response.id);
        if shown.clicked() {
            counted.set(counted.get() + 1);
        }
    });
    fixture.settle();
    let button = fixture.node_of(at.get()).expect("the button is in the access tree");
    let picture = fixture.part_rect(at.get(), PartKey::new("picture")).expect("the button declares its picture's part");
    assert!(button.rect.contains_rect(picture), "the picture {picture:?} lies inside the button {:?}", button.rect);
    assert!(picture.width() > picture.height() + 0.5, "the picture's box is its aspect times as wide as high: {picture:?}");

    fixture.click_at(picture.center());
    assert!(clicks.get() > 0, "a click inside the picture activates the button");
    let tree = fixture.tree();
    let mut pending = vec![(button.node, 0)];
    while let Some((key, depth)) = pending.pop() {
        pending.extend(tree.children_of(key).into_iter().map(|child| (child, depth + 1)));
    }
    // The child `Ui` the button shows its picture in is a generic container, which screen readers pass over.
    assert!(tree.seen_children_of(button.node).is_empty(), "a screen reader finds no node under the button");
    assert!(fixture.by_role(Role::IMAGE).is_empty(), "the picture is not a node of its own");
}

/// The surface of the first style of `type_name` resolved when `show` runs alone, and the width of the control
/// named `name`.
fn seen(type_name: &str, name: &str, show: impl Fn(&mut Ui) + 'static) -> (Option<craft_ui::style::SurfaceLook>, f32) {
    let mut fixture = Fixture::new(move |ui| show(ui));
    fixture.settle();
    let style = fixture.styles_of(type_name).into_iter().next().expect("the control resolved its style");
    (style.looks.surface.clone(), fixture.by_label(name).expect("the control").rect.width())
}

/// Shows `show` where a ribbon group shows its content.
fn in_ribbon(ui: &mut Ui, show: impl FnOnce(&mut Ui)) {
    with_env(
        ui,
        |overrides| {
            overrides.values.set(ControlPlace::Ribbon);
        },
        show,
    );
}

/// In a ribbon a push or split button that asks for no variant is flat at rest: it draws the ghost surface, and its
/// sides take the control's padding, so it is narrower than a ghost button elsewhere. Anywhere else a button keeps its
/// outlined default, and in a ribbon a variant it asks for stays.
#[trial]
fn a_ribbons_buttons_are_flat_at_rest_unless_they_ask_for_a_variant() {
    let button = std::any::type_name::<Button<'static>>();
    let (ribbon, narrow) = seen(button, "Copy", |ui| {
        in_ribbon(ui, |ui| {
            let _ = Button::new("Copy").show(ui);
        });
    });
    let (ghost, wide) = seen(button, "Copy", |ui| {
        let _ = Button::new("Copy").variant(Variant::GHOST).show(ui);
    });
    let (standard, _) = seen(button, "Copy", |ui| {
        let _ = Button::new("Copy").show(ui);
    });
    let (primary, _) = seen(button, "Copy", |ui| {
        in_ribbon(ui, |ui| {
            let _ = Button::new("Copy").variant(Variant::PRIMARY).show(ui);
        });
    });
    assert!(ghost.is_some(), "a button has a surface");
    assert_eq!(ribbon, ghost, "a ribbon button draws the ghost surface");
    let flat = ribbon.clone().expect("a ribbon button has a surface");
    assert_ne!(standard, ghost, "outside a ribbon the default look stays");
    assert_ne!(primary, ghost, "a variant asked for in a ribbon stays");
    assert!(narrow < wide - 0.5, "a ribbon's flat button sits close: {narrow} against {wide}");

    let split = std::any::type_name::<SplitButton<'static, usize>>();
    let (ribbon, narrow) = seen(split, "Paste", |ui| {
        let entries = [Entry::new(1_usize, "Keep text only")];
        in_ribbon(ui, |ui| {
            let _ = SplitButton::new("Paste", &entries).show(ui);
        });
    });
    let (ghost, wide) = seen(split, "Paste", |ui| {
        let entries = [Entry::new(1_usize, "Keep text only")];
        let _ = SplitButton::new("Paste", &entries).variant(Variant::GHOST).show(ui);
    });
    assert_eq!(ribbon, ghost, "a ribbon split button draws the ghost surface");
    assert!(narrow < wide - 0.5, "a ribbon's flat split button sits close: {narrow} against {wide}");

    // A labelled toggle is flat at rest too, as the ribbon's push buttons are: no surface kind and no edge until the
    // pointer is over it, and the on plate while on. A ghost toggle outside a ribbon keeps its raised, outlined edge,
    // so the ribbon toggle is checked against the ribbon's flat push button, not against it.
    let toggle = std::any::type_name::<ToggleButton<'static>>();
    let (ribbon, narrow) = seen(toggle, "Track Changes", |ui| {
        in_ribbon(ui, |ui| {
            let mut on = false;
            let _ = ToggleButton::new("Track Changes", Bind::bound(&mut on)).show(ui);
        });
    });
    let (_, wide) = seen(toggle, "Track Changes", |ui| {
        let mut on = false;
        let _ = ToggleButton::new("Track Changes", Bind::bound(&mut on)).variant(Variant::GHOST).show(ui);
    });
    let (standard, _) = seen(toggle, "Track Changes", |ui| {
        let mut on = false;
        let _ = ToggleButton::new("Track Changes", Bind::bound(&mut on)).show(ui);
    });
    let ribbon = ribbon.expect("a ribbon toggle has a surface");
    assert_eq!(ribbon.kind, flat.kind, "a ribbon toggle draws no surface kind at rest, as a ribbon button");
    assert_eq!(ribbon.fill.off, flat.fill.off, "a ribbon toggle's fill while off is a ribbon button's");
    let edge = ribbon.border.as_ref().map(|border| border.off.rest);
    assert!(edge.is_none() || edge == Some(ribbon.fill.off.rest), "a ribbon toggle shows no edge at rest: {edge:?}");
    assert!(ribbon.fill.on.is_some(), "a ribbon toggle shows the on plate while on");
    // A toggle's style has one variant (`ToggleButtonStyle` lists only `SECONDARY`), so a ghost toggle outside a ribbon
    // draws that same outlined look: the default is checked by its edge, not against the ghost toggle.
    let outlined = standard.clone().expect("a toggle has a surface");
    let standard_edge = outlined.border.as_ref().map(|border| border.off.rest);
    assert!(
        standard_edge.is_some_and(|edge| edge != outlined.fill.off.rest),
        "outside a ribbon a toggle keeps its outlined default: {standard_edge:?}"
    );
    assert_ne!(Some(ribbon), standard, "a ribbon toggle is not the outlined default");
    assert!(narrow < wide - 0.5, "a ribbon's flat toggle sits close: {narrow} against {wide}");
}

/// Under an app's `PushButtonLook::Filled` a push or split button that asks for no variant draws the filled tile: a fill
/// apart from the outline's clear rest, no edge at rest and one under the pointer. A variant it asks for stays, the
/// outline included, and a ribbon keeps its flat look.
#[trial]
fn an_apps_filled_look_fills_its_default_push_buttons() {
    let filled = |ui: &mut Ui, show: &dyn Fn(&mut Ui)| {
        with_env(
            ui,
            |overrides| {
                overrides.values.set(PushButtonLook::Filled);
            },
            show,
        )
    };
    let button = std::any::type_name::<Button<'static>>();
    let (tile, _) = seen(button, "Cancel", move |ui| {
        filled(ui, &|ui| {
            let _ = Button::new("Cancel").show(ui);
        })
    });
    let (outline, _) = seen(button, "Cancel", |ui| {
        let _ = Button::new("Cancel").show(ui);
    });
    let (primary, _) = seen(button, "Cancel", move |ui| {
        filled(ui, &|ui| {
            let _ = Button::new("Cancel").variant(Variant::PRIMARY).show(ui);
        })
    });
    let (plain_primary, _) = seen(button, "Cancel", |ui| {
        let _ = Button::new("Cancel").variant(Variant::PRIMARY).show(ui);
    });
    let (ribbon, _) = seen(button, "Cancel", move |ui| {
        filled(ui, &|ui| {
            in_ribbon(ui, |ui| {
                let _ = Button::new("Cancel").show(ui);
            })
        })
    });
    let (ghost, _) = seen(button, "Cancel", |ui| {
        let _ = Button::new("Cancel").variant(Variant::GHOST).show(ui);
    });
    let (asked, _) = seen(button, "Cancel", move |ui| {
        filled(ui, &|ui| {
            let _ = Button::new("Cancel").variant(Variant::SECONDARY).show(ui);
        })
    });
    assert_eq!(asked, outline, "a button that asks for the outline keeps it");
    let (tile, outline) = (tile.expect("a filled button has a surface"), outline.expect("a button has a surface"));
    assert_ne!(tile.fill.off.rest, outline.fill.off.rest, "a filled button rests on a fill of its own");
    let edge = tile.border.as_ref().expect("a filled button is edged by state").off;
    assert_eq!(edge.rest.egui().a(), 0, "a filled button shows no edge at rest");
    assert!(edge.hover.egui().a() > 0 && edge.pressed.egui().a() > 0, "a filled button is edged under the pointer and the press");
    assert_eq!(primary, plain_primary, "a variant asked for stays");
    assert_eq!(ribbon, ghost, "a ribbon's button stays flat");

    let split = std::any::type_name::<SplitButton<'static, usize>>();
    let (split_tile, _) = seen(split, "Paste", move |ui| {
        filled(ui, &|ui| {
            let entries = [Entry::new(1_usize, "Keep text only")];
            let _ = SplitButton::new("Paste", &entries).show(ui);
        })
    });
    assert_eq!(split_tile.map(|look| look.fill.off.rest), Some(tile.fill.off.rest), "a split button takes the same tile");
}

/// In a ribbon a combo box is as wide as its widest option and its chevron, not a field length.
#[trial]
fn a_ribbons_combo_box_is_as_wide_as_its_options() {
    let width = |ribbon: bool| {
        let mut fixture = Fixture::new(move |ui| {
            let options = [Entry::new("8".to_owned(), "8"), Entry::new("72".to_owned(), "72")];
            let mut size = "11".to_owned();
            let mut show = |ui: &mut Ui| {
                let _ = ComboBox::new("Font Size", Bind::bound(&mut size), &options).show(ui);
            };
            if ribbon {
                in_ribbon(ui, show);
            } else {
                show(ui);
            }
        });
        fixture.settle();
        fixture.by_label("Font Size").expect("the combo box").rect.width()
    };
    let (ribbon, standard) = (width(true), width(false));
    assert!(ribbon < standard / 2.0, "the ribbon's combo box fits two digits and its chevron: {ribbon} against {standard}");
}

#[trial]
fn a_bars_combo_box_is_as_wide_as_its_options() {
    use craft_ui::bars::{Bar, BarKind};
    use craft_ui::style::SurfaceKind;

    #[derive(Clone, Copy)]
    enum Placement {
        Standard,
        InBarEnv,
        CompactOptIn,
        FloatingBar,
    }

    let width = |kind: Placement| {
        let mut fixture = Fixture::new(move |ui| {
            let options = [Entry::new("8".to_owned(), "8"), Entry::new("72".to_owned(), "72")];
            let mut size = "11".to_owned();
            let mut show = |ui: &mut Ui| {
                let mut cb = ComboBox::new("Font Size", Bind::bound(&mut size), &options);
                if matches!(kind, Placement::CompactOptIn) {
                    cb = cb.compact(true);
                }
                let _ = cb.show(ui);
            };
            match kind {
                Placement::Standard | Placement::CompactOptIn => show(ui),
                Placement::InBarEnv => {
                    with_env(
                        ui,
                        |overrides| {
                            overrides.values.set(craft_ui::contract::InBar(true));
                        },
                        show,
                    );
                }
                Placement::FloatingBar => {
                    let _ = Bar::new("Floating Bar", BarKind::Tool).surface(SurfaceKind::Floating).show(ui, |ui, _| {
                        show(ui);
                    });
                }
            }
        });
        fixture.settle();
        fixture.by_label("Font Size").expect("the combo box").rect.width()
    };
    let standard = width(Placement::Standard);
    let in_bar = width(Placement::InBarEnv);
    let opt_in = width(Placement::CompactOptIn);
    let floating_bar = width(Placement::FloatingBar);
    assert!(in_bar < standard / 2.0, "the bar's combo box fits two digits and its chevron: {in_bar} against {standard}");
    assert!((in_bar - opt_in).abs() < 0.5, "opt-in compact combo box matches in_bar: {opt_in} against {in_bar}");
    assert!((in_bar - floating_bar).abs() < 0.5, "floating bar combo box matches in_bar: {floating_bar} against {in_bar}");
}

#[trial]
fn a_nested_in_room_inside_a_bar_does_not_reuse_the_bars_compact_style() {
    use craft_ui::bars::{Bar, BarKind};
    use craft_ui::style::SurfaceKind;

    let mut standard_fixture = Fixture::new(|ui| {
        let options = [Entry::new("8".to_owned(), "8"), Entry::new("72".to_owned(), "72")];
        let mut size = "11".to_owned();
        let _ = ComboBox::new("Standard Size", Bind::bound(&mut size), &options).show(ui);
    });
    standard_fixture.settle();
    let standard = standard_fixture.by_label("Standard Size").expect("the standard combo box").rect.width();

    let seen_keys = Rc::new(Cell::new(None));
    let seen_in_bar = Rc::new(Cell::new(true));
    let keys_out = Rc::clone(&seen_keys);
    let in_bar_out = Rc::clone(&seen_in_bar);

    let mut fixture = Fixture::new(move |ui| {
        let options = [Entry::new("8".to_owned(), "8"), Entry::new("72".to_owned(), "72")];
        let mut bar_size = "11".to_owned();
        let mut room_size = "11".to_owned();
        let _ = Bar::new("Toolbar", BarKind::Tool).surface(SurfaceKind::Floating).show(ui, |ui, _| {
            let bar_key = Env::of(ui).key;
            // Warm the style cache with the bar's compact InBar(true) entry first.
            let _ = ComboBox::new("Bar Size", Bind::bound(&mut bar_size), &options).show(ui);
            in_room(ui, Rect::from_min_size(pos2(0.0, 60.0), vec2(400.0, 80.0)), |ui| {
                let room_env = Env::of(ui);
                in_bar_out.set(room_env.get::<InBar>().0);
                keys_out.set(Some((bar_key, room_env.key)));
                let _ = ComboBox::new("Room Size", Bind::bound(&mut room_size), &options).show(ui);
            });
        });
    });
    fixture.room(vec2(500.0, 200.0));
    fixture.settle();

    let bar_width = fixture.by_label("Bar Size").expect("the bar combo box").rect.width();
    let room_width = fixture.by_label("Room Size").expect("the nested room combo box").rect.width();
    let (bar_key, room_key) = seen_keys.get().expect("keys recorded");

    assert!(!seen_in_bar.get(), "nested in_room resets InBar to false");
    assert_ne!(bar_key, room_key, "nested in_room derives a new EnvKey when resetting InBar");
    assert!(bar_width < standard / 2.0, "the bar's combo box is compact: {bar_width} against {standard}");
    assert!(
        (room_width - standard).abs() < 0.5,
        "combo box in a nested in_room uses standard width rather than the bar's warmed compact style: {room_width} against {standard}"
    );
}

/// A page's side menu (`ControlPlace::SideMenu`) shows its buttons as rows of places: the label in the large body role
/// and padded above and below, so a row is taller and wider than the same ghost button elsewhere. A stacked button
/// there keeps its own look.
#[trial]
fn a_side_menus_buttons_are_taller_rows_with_the_large_body_label() {
    let rect = |side: bool, arrangement: Arrangement| {
        let mut fixture = Fixture::new(move |ui| {
            let show = |ui: &mut Ui| {
                let _ = Button::new("Print").icon(IconId::new("print")).variant(Variant::GHOST).arrangement(arrangement).show(ui);
            };
            if side {
                with_env(
                    ui,
                    |overrides| {
                        overrides.values.set(ControlPlace::SideMenu);
                    },
                    show,
                );
            } else {
                show(ui);
            }
        });
        fixture.settle();
        fixture.by_label("Print").expect("the button").rect
    };
    let (menu, plain) = (rect(true, Arrangement::Inline), rect(false, Arrangement::Inline));
    assert!(menu.height() > plain.height() + 8.0, "a side menu's row is padded as a list row: {} against {}", menu.height(), plain.height());
    assert!(menu.width() > plain.width() + 4.0, "its label is in the larger role: {} against {}", menu.width(), plain.width());
    let (menu, plain) = (rect(true, Arrangement::Stacked), rect(false, Arrangement::Stacked));
    assert_eq!(menu.size(), plain.size(), "a stacked button keeps its own look in a side menu");
}

/// The row of the page shown in a side menu (`selected`) keeps a band, as WordCraft's File menu marks its current page:
/// the brand surface's ink laid thinly over it, a step stronger than the hover's, never the panel's opaque state fill
/// (which drew a light band with dark text over the brand rail). A selected ghost button elsewhere keeps its own look.
#[trial]
fn a_side_menus_selected_row_shows_a_band_over_the_brand() {
    let button = std::any::type_name::<Button<'static>>();
    let (menu, _) = seen(button, "Info", |ui| {
        with_env(
            ui,
            |overrides| {
                overrides.values.set(ControlPlace::SideMenu);
            },
            |ui| {
                let _ = Button::new("Info").variant(Variant::GHOST).selected(true).show(ui);
            },
        );
    });
    let (plain, _) = seen(button, "Info", |ui| {
        let _ = Button::new("Info").variant(Variant::GHOST).selected(true).show(ui);
    });
    let fill = menu.expect("a side menu's row has a surface").fill;
    let on = fill.on.expect("a side menu's row has an on look");
    let (rest, hover, shown) = (fill.off.rest.egui(), fill.off.hover.egui(), on.rest.egui());
    assert_eq!(rest.a(), 0, "a row at rest shows the brand surface");
    assert!(hover.a() > 0 && hover.a() < 64, "the hover band is the ink laid thinly over the brand: {hover:?}");
    assert!(shown.a() > hover.a() && shown.a() < 96, "the shown page's band is a step above the hover's: {shown:?}");
    assert_eq!(on.hover, on.rest, "the shown page's row keeps its band under the pointer");
    assert!(plain.expect("a ghost button has a surface").fill.on.is_none(), "a selected ghost button elsewhere keeps its look");
}

/// A side menu's row shows its icon at the standard icon side, larger than a control's small one, as WordCraft's File
/// rail reads by its page icons; a ghost button elsewhere keeps the control's icon.
#[trial]
fn a_side_menus_row_shows_the_standard_icon() {
    let icon_side = |side: bool| {
        let mut fixture = Fixture::new(move |ui| {
            let show = |ui: &mut Ui| {
                let _ = Button::new("Print").icon(IconId::new("print")).variant(Variant::GHOST).show(ui);
            };
            if side {
                with_env(
                    ui,
                    |overrides| {
                        overrides.values.set(ControlPlace::SideMenu);
                    },
                    show,
                );
            } else {
                show(ui);
            }
        });
        fixture.settle();
        let style = fixture.styles_of(std::any::type_name::<Button<'static>>()).into_iter().next().expect("the button resolved its style");
        style.looks.icons.first().expect("a button has an icon look").side
    };
    let (menu, plain) = (icon_side(true), icon_side(false));
    assert!(menu > plain, "a side menu's icon is the standard one: {menu} against {plain}");
}

/// An icon-only button in a side menu (WordCraft's Back arrow on the File rail) reads in the menu's ink, as a `Button`
/// row's icon does, not in the dark ghost ink that vanished on the brand surface; elsewhere it keeps its own ink.
#[trial]
fn a_side_menus_icon_button_reads_in_the_menus_ink() {
    let rest = |side: bool, icon_only: bool| {
        let mut fixture = Fixture::new(move |ui| {
            let show = |ui: &mut Ui| {
                if icon_only {
                    let _ = IconButton::new(IconId::new("print")).label("Back").show(ui);
                } else {
                    let _ = Button::new("Print").icon(IconId::new("print")).variant(Variant::GHOST).show(ui);
                }
            };
            if side {
                with_env(
                    ui,
                    |overrides| {
                        overrides.values.set(ControlPlace::SideMenu);
                    },
                    show,
                );
            } else {
                show(ui);
            }
        });
        fixture.settle();
        let kind = if icon_only { std::any::type_name::<IconButton<'static>>() } else { std::any::type_name::<Button<'static>>() };
        let style = fixture.styles_of(kind).into_iter().next().expect("the button resolved its style");
        style.looks.icons.first().expect("the button has an icon look").color.off.rest.egui()
    };
    assert_eq!(rest(true, true), rest(true, false), "the icon button takes the ink of the menu's rows");
    assert_ne!(rest(true, true), rest(false, true), "outside a side menu it keeps its own ink");
}

/// The command the arrow trial's split button is bound to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "splitTrial", scope = "splitTrial")]
#[non_exhaustive]
enum SplitCommand {
    #[command(label = "Trace")]
    Trace,
}

/// A click on a split button's arrow opens its menu and never runs the action, read from the output or bound as a
/// command; a click on the main part runs it.
#[trial]
fn a_split_buttons_arrow_never_runs_its_action() {
    let ran = Rc::new(Cell::new(0));
    let count = Rc::clone(&ran);
    let mut fixture = Fixture::new(move |ui| {
        let entries = [Entry::new(0u8, "For all apps")];
        if SplitButton::new("Install", &entries).arrow_label("Install options").show(ui).activated {
            count.set(count.get() + 1);
        }
    });
    fixture.settle();
    let arrow = fixture.by_label("Install options").expect("the arrow part");
    fixture.click(&arrow).settle();
    assert!(!fixture.by_role(Role::MENU).is_empty(), "the arrow opened the menu");
    assert_eq!(ran.get(), 0, "the arrow's click is not the action's");
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.settle();
    let main = fixture.by_label("Install").expect("the split button");
    fixture.click_at(Pos2::new(main.rect.left() + 4.0, main.rect.center().y)).settle();
    assert_eq!(ran.get(), 1, "the main part runs the action");

    let fired = Rc::new(Cell::new(0));
    let count = Rc::clone(&fired);
    let mut fixture = Fixture::new(move |ui| {
        craft_ui::layout_components::Stack::new().command_scope(SplitCommand::SCOPE).show(ui, |ui, scope| {
            while scope.take_command::<SplitCommand>().is_some() {
                count.set(count.get() + 1);
            }
            scope.publish(SplitCommand::Trace, CommandState::new());
            let entries = [Entry::new(0u8, "Colour")];
            let _ = SplitButton::command(SplitCommand::Trace, SplitMenu::Entries(&entries)).arrow_label("Tracing presets").show(ui);
        });
    });
    fixture.settle();
    let arrow = fixture.by_label("Tracing presets").expect("the arrow part");
    fixture.click(&arrow).settle();
    assert!(!fixture.by_role(Role::MENU).is_empty(), "the arrow opened the menu");
    assert_eq!(fired.get(), 0, "the arrow's click does not fire the command");
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.settle();
    let main = fixture.by_label("Trace").expect("the split button");
    fixture.click_at(Pos2::new(main.rect.left() + 4.0, main.rect.center().y)).settle();
    assert_eq!(fired.get(), 1, "the main part fires it");
}

/// A split button's body that closes itself (a colour palette's swatch pick) closes the split's menu layer: before,
/// the close reached only the inline menu inside the layer, which stayed open around an empty body. Opened again, the
/// body shows its rows.
#[trial]
fn a_split_bodys_request_close_closes_the_menu_layer_and_it_opens_again_with_its_body() {
    let picked = Rc::new(Cell::new(0));
    let count = Rc::clone(&picked);
    let mut fixture = Fixture::new(move |ui| {
        let count = Rc::clone(&count);
        let _ = SplitButton::<u8>::body("Shading", move |ui, cx| {
            if craft_ui::contract::clicked(&Button::new("Red").show(ui).response) {
                count.set(count.get() + 1);
                cx.request_close();
            }
        })
        .arrow_label("Shading colours")
        .show(ui);
    });
    fixture.settle();
    let open_menu = |fixture: &mut Fixture| {
        let arrow = fixture.by_label("Shading colours").expect("the arrow part");
        fixture.click(&arrow).settle();
        assert!(!fixture.by_role(Role::MENU).is_empty(), "the arrow opened the menu");
    };
    open_menu(&mut fixture);
    let swatch = fixture.by_label("Red").expect("the body's swatch is shown");
    fixture.click(&swatch).settle();
    assert_eq!(picked.get(), 1, "the swatch ran once");
    assert!(fixture.by_role(Role::MENU).is_empty(), "the body's close closed the menu");
    // An arrow click on a layer left open around an empty body would only close it: the layer itself must have closed
    // for the click to open the menu with its body.
    open_menu(&mut fixture);
    assert!(fixture.by_label("Red").is_some(), "opened again, the menu shows its body");
}

/// An app icon with an accent part, named for this test.
const ACCENT_ICON_NAME: &str = "zz_item4_accent_icon";
static ACCENT_ICONS: craft_ui::primitives::AppIcons = craft_ui::primitives::AppIcons::new(
    "buttons-test-accent",
    None,
    &[(
        ACCENT_ICON_NAME,
        br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect x="3" y="3" width="10" height="18" fill="black"/><rect class="accent" x="15" y="3" width="6" height="18" fill="black"/></svg>"##,
    )],
);

/// The tints of the textured quads a button with the accent icon paints, in paint order, inside a stack of `surface`.
fn accent_icon_tints(surface: craft_ui::style::SurfaceKind) -> Vec<egui::Color32> {
    accent_icon_tints_in(None, surface)
}

/// As `accent_icon_tints`, with the stack of `surface` nested inside a stack of `outer`.
fn accent_icon_tints_in(outer: Option<craft_ui::style::SurfaceKind>, surface: craft_ui::style::SurfaceKind) -> Vec<egui::Color32> {
    craft_ui::primitives::register_app_icons(&ACCENT_ICONS);
    let mut fixture = Fixture::new(move |ui| {
        let inner = |ui: &mut egui::Ui| {
            let _ = craft_ui::layout_components::Stack::new().surface(surface).show(ui, |ui, _| {
                let _ = Button::new("Pen").icon(IconId::new(ACCENT_ICON_NAME)).variant(Variant::GHOST).show(ui);
            });
        };
        match outer {
            Some(outer) => {
                let _ = craft_ui::layout_components::Stack::new().surface(outer).show(ui, |ui, _| inner(ui));
            }
            None => inner(ui),
        }
    });
    fixture.settle();
    fixture
        .frame_shapes()
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Rect(rect) if shape.texture_id() != egui::TextureId::default() => Some(rect.fill),
            egui::Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::default() => mesh.vertices.first().map(|vertex| vertex.color),
            _ => None,
        })
        .collect()
}

/// An app icon on a brand or scrim surface paints its accent parts in the ink of strong fills, as its plain parts:
/// the accent colour would vanish on the brand fill. On a panel the accent parts keep their own colour.
#[trial]
fn an_icons_accent_parts_on_a_brand_surface_paint_in_the_icons_ink() {
    let brand = accent_icon_tints(craft_ui::style::SurfaceKind::Brand);
    let panel = accent_icon_tints(craft_ui::style::SurfaceKind::Panel);
    assert!(brand.len() >= 2 && panel.len() >= 2, "the icon paints an ink and an accent layer: {brand:?} {panel:?}");
    let (ink, accent) = (brand.len() - 2, brand.len() - 1);
    assert_eq!(brand[ink], brand[accent], "on the brand fill the accent parts read in the icon's ink: {brand:?}");
    let (ink, accent) = (panel.len() - 2, panel.len() - 1);
    assert_ne!(panel[ink], panel[accent], "on a panel the accent parts keep the accent colour: {panel:?}");
}

/// A panel nested inside a brand stack sets its own ink: the icon's accent parts keep the accent colour on it, not the
/// strong-fill ink of the rail around it.
#[trial]
fn an_icons_accent_parts_on_a_panel_inside_a_brand_surface_keep_the_accent_colour() {
    let tints = accent_icon_tints_in(Some(craft_ui::style::SurfaceKind::Brand), craft_ui::style::SurfaceKind::Panel);
    assert!(tints.len() >= 2, "the icon paints an ink and an accent layer: {tints:?}");
    let (ink, accent) = (tints.len() - 2, tints.len() - 1);
    assert_ne!(tints[ink], tints[accent], "on a panel inside a brand stack the accent parts keep the accent colour: {tints:?}");
}

/// Runs an icon button through idle frames, then one frame per step of `steps` (given the button's centre and a point
/// far off it), and returns `ButtonOutput::held` of each of those frames.
fn held_per_frame(steps: impl FnOnce(Pos2, Pos2) -> Vec<Vec<Event>>) -> Vec<bool> {
    let (held, rect) = (Cell::new(false), Cell::new(Rect::NOTHING));
    let mut show = |ui: &mut Ui| {
        let out = IconButton::new(IconSpec::svg(IconId::new("eyedropper"))).label("Hold").show(ui);
        held.set(out.held);
        rect.set(out.rect);
    };
    let mut frames = Frames::new();
    frames.idle(3, &mut show);
    let at = rect.get().center();
    let far = at + vec2(300.0, 300.0);
    steps(at, far)
        .into_iter()
        .map(|events| {
            frames.frame(events, &mut show);
            held.get()
        })
        .collect()
}

#[trial]
fn a_held_button_reports_its_press_until_the_release_or_a_cancel() {
    assert_eq!(held_per_frame(|at, _| vec![vec![], vec![moved(at)]]), [false, false], "an idle button is not held");
    assert_eq!(held_per_frame(|at, _| vec![vec![moved(at)], vec![press(at)]]), [false, true], "a press holds it");
    assert_eq!(
        held_per_frame(|at, far| vec![vec![moved(at)], vec![press(at)], vec![moved(far)]]),
        [false, true, true],
        "the pointer leaving the button does not end the press"
    );
    assert_eq!(
        held_per_frame(|at, far| vec![vec![moved(at)], vec![press(at)], vec![moved(far)], vec![release(far)]]),
        [false, true, true, false],
        "the release ends it"
    );
    assert_eq!(
        held_per_frame(|at, _| vec![vec![moved(at)], vec![press(at)], vec![key_press(Key::Escape, Modifiers::NONE)]]),
        [false, true, false],
        "Escape cancels it"
    );
    assert_eq!(
        held_per_frame(|at, _| vec![vec![moved(at)], vec![press(at)], vec![window_blurred()]]),
        [false, true, false],
        "losing the window's focus cancels it"
    );
    let by_key = held_per_frame(|_, _| vec![vec![key_press(Key::Tab, Modifiers::NONE)], vec![key_press(Key::Space, Modifiers::NONE)], vec![]]);
    assert_eq!(by_key, [false, false, false], "a key activation never holds the button");
}

/// The catalog of the chord trial: one command with a default chord, one without.
fn catalog() -> AppCatalog {
    let bound =
        CommandDef::new(CommandRef::new("trial.bound"), "Bound", Scope::app()).with_default(Binding::parse("F7").expect("the trial's chord parses"));
    let plain = CommandDef::new(CommandRef::new("trial.plain"), "Plain", Scope::app());
    AppCatalog::new(vec![bound, plain], 1).expect("the trial's catalog is valid")
}

/// A command button of `command` with `chord` in a room `width` wide: the texts it painted, joined in paint order and
/// split to (text, elided), and the button's width.
fn command_shown(command: &str, chord: ButtonChord, width: f32) -> (Vec<(String, bool)>, f32) {
    let button_width = Cell::new(0.0);
    let mut show = |ui: &mut Ui| {
        let room = Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 40.0));
        button_width.set(in_region(ui, room, |ui| Button::command(CommandRef::new(command)).chord(chord).show(ui).rect.width()));
    };
    let mut frames = Frames::new();
    frames.commands(CommandInput::new().catalog(catalog()));
    frames.idle(3, &mut show);
    (frames.texts().iter().map(|text| (text.text.clone(), text.elided)).collect(), button_width.get())
}

#[trial]
fn a_command_button_shows_its_chord_until_the_room_is_short() {
    // PLAN: assumes a command button hugs its content in the room, so the width of the chordless button is a room that
    // fits the label and not the chord.
    let (wide, _) = command_shown("trial.bound", ButtonChord::Trailing, 400.0);
    let all: String = wide.iter().map(|(text, _)| text.as_str()).collect();
    let (label, chord) = (all.find("Bound"), all.find("F7"));
    assert!(label.is_some() && chord.is_some(), "the label and the chord show in a wide room: {wide:?}");
    assert!(label < chord, "the chord follows the label: {all:?}");

    let (_, label_width) = command_shown("trial.bound", ButtonChord::Hidden, 400.0);
    let (short, _) = command_shown("trial.bound", ButtonChord::Trailing, label_width);
    assert!(short.iter().all(|(text, _)| !text.contains("F7")), "a short room drops the chord: {short:?}");
    assert!(short.iter().any(|(text, elided)| text.contains("Bound") && !elided), "before it elides the label: {short:?}");

    let (plain_chord, _) = command_shown("trial.plain", ButtonChord::Trailing, 400.0);
    let (plain_hidden, _) = command_shown("trial.plain", ButtonChord::Hidden, 400.0);
    assert_eq!(plain_chord, plain_hidden, "a command with no chord shows none");
}

/// A toggle button says whether it is pressed both ways: on is pressed and off is not pressed, bound to a value or
/// given a shared one (a mode of several objects), where a button with no toggle has no pressed state. Guards against
/// an off toggle reading as a plain button.
#[trial]
fn a_toggle_button_reports_pressed_on_and_off() {
    use craft_behavior::Mixed;
    use craft_ui::testing::access::NodeKey;
    for on in [true, false] {
        let ids = Rc::new(Cell::new((Id::NULL, Id::NULL, Id::NULL)));
        let seen = ids.clone();
        let mut fixture = Fixture::new(move |ui| {
            let mut value = on;
            let toggle = ToggleButton::new("Snap to grid", Bind::bound(&mut value)).show(ui).response.id;
            let shared = ToggleButton::new("Unite", Bind::given(Mixed::Same(on))).show(ui).response.id;
            seen.set((toggle, shared, Button::new("Save").show(ui).response.id));
        });
        fixture.settle();
        let (toggle, shared, plain) = ids.get();
        let tree = fixture.tree();
        assert_eq!(tree.toggled_of(NodeKey::of(toggle)), Some(Mixed::Same(on)), "a toggle that is {on}");
        assert_eq!(tree.toggled_of(NodeKey::of(shared)), Some(Mixed::Same(on)), "a toggle given {on}");
        assert_eq!(tree.toggled_of(NodeKey::of(plain)), None, "a plain button has no pressed state");
    }
}

/// An icon-only toggle button in a field with no label text (a dialog's custom row with an empty label), beside an egui
/// check box in an `egui::Grid`, keeps the name its own label gives it.
#[trial]
fn an_icon_only_toggle_in_a_field_without_label_text_keeps_its_name() {
    let mut fixture = Fixture::new(|ui| {
        craft_ui::form::Field::new("").show(ui, |ui, _| {
            egui::Grid::new("inks").num_columns(2).show(ui, |ui| {
                let mut print = true;
                ui.add(egui::Checkbox::without_text(&mut print));
                let _ = ToggleButton::new("Convert Cyan to process", Bind::given(false)).icon(IconId::new("eye")).icon_only().show(ui);
                ui.end_row();
            });
        });
    });
    fixture.settle();
    let toggle = fixture.by_label("Convert Cyan to process").expect("the toggle keeps its name");
    assert!(fixture.by_role(Role::BUTTON).iter().any(|button| button.node == toggle.node), "the named node is the toggle button");
}

/// A menu button given the row its menu picked reads as a combo box named by its label, with that row as its value; a
/// menu button without one stays a button with no value.
#[trial]
fn a_menu_button_with_a_choice_reads_as_a_combo_box_valued_by_it() {
    let mut fixture = Fixture::new(|ui| {
        let align = IconButton::new(IconId::new("align_edge_left")).label("Align To").popup(PopupKind::Menu).choice("Align to Artboard").show(ui);
        Menu::new("Align To").opened_by(&align.response, Trigger::Press).show(ui, |ui, _| {
            MenuItem::new("Align to Artboard").check(true).show(ui);
        });
        IconButton::new(IconId::new("more")).label("More").popup(PopupKind::Menu).show(ui);
    });
    fixture.settle();
    let align = fixture.by_label("Align To").expect("the choice button is in the access tree");
    let more = fixture.by_label("More").expect("the plain menu button is in the access tree");
    let tree = fixture.tree();
    assert_eq!(tree.role_of(align.node), Some(Role::COMBO_BOX), "a button with a choice is a combo box");
    assert_eq!(tree.value_of(align.node), Some("Align to Artboard"), "its value is the choice");
    assert_eq!(tree.expanded_of(align.node), Some(false), "its menu is closed");
    assert_eq!(tree.role_of(more.node), Some(Role::BUTTON), "a menu button without a choice stays a button");
    assert_eq!(tree.value_of(more.node), None, "and has no value");
    fixture.click_at(align.center()).settle();
    assert_eq!(fixture.tree().expanded_of(align.node), Some(true), "it reports its menu open");
}
