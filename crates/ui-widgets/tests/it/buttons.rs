//! Buttons: the stacked arrangement's height, the hover report of a click the pointer stays on, the compact
//! group level, under which an inline button drops its label and keeps it as its name and tooltip, the room a button
//! keeps for its widest text, and a picture in the leading icon's place.

use craft_behavior::{IconId, PartKey};
use craft_theme::ControlSize;
use craft_ui::buttons::{Arrangement, Button, GroupLevel, SplitButton, SplitMenu, ToggleButton};
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{Bind, CommandSet, Entry, Hover, Props as _, Role, Show as _};
use craft_ui::display::Picture;
use craft_ui::list::ThumbAspect;
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
        |ui| show(ui),
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
    let cases: [(&str, fn(&mut Ui) -> Response); 2] = [("Save", save), ("Trace", trace)];
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
        eprintln!("DIAG rf-button depth {depth} {key:?}: role {:?} hidden {}", tree.role_of(key), tree.is_hidden(key));
        pending.extend(tree.children_of(key).into_iter().map(|child| (child, depth + 1)));
    }
    // The child `Ui` the button shows its picture in is a generic container, which screen readers pass over.
    assert!(tree.seen_children_of(button.node).is_empty(), "a screen reader finds no node under the button");
    assert!(fixture.by_role(Role::IMAGE).is_empty(), "the picture is not a node of its own");
}
