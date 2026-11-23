//! Buttons: the stacked arrangement's height, the hover report of a click the pointer stays on, the compact
//! group level, under which an inline button drops its label and keeps it as its name and tooltip, the room a button
//! keeps for its widest text, and a picture in the leading icon's place.

use craft_behavior::{IconId, PartKey};
use craft_theme::ControlSize;
use craft_theme::Variant;
use craft_ui::buttons::{Arrangement, Button, ControlPlace, GroupLevel, SplitButton, SplitMenu, ToggleButton};
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{Bind, CommandSet, Entry, Hover, Props as _, Role, Show as _, ShowContainer as _};
use craft_ui::display::Picture;
use craft_ui::list::ThumbAspect;
use craft_ui::pickers::ComboBox;
use craft_ui::runtime::commands::CommandState;
use craft_ui::runtime::env::with_env;
use craft_ui::testing::pictures::sample_picture;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Fixture, trial};
use egui::{Event, Id, Pos2, Rect, Response, Ui};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

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
