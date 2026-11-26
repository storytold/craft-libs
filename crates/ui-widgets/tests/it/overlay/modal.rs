//! Modals: blocked hits and commands below, focus return, and the popups opened from inside them.

use super::kit::{Go, Leaf, Opened, Opener, PlainStyle, Seen};
use craft_behavior::Sense;
use craft_ui::buttons::Button;
use craft_ui::contract::{
    Common, Component, Cx, MeasureCx, Overlay, OverlayUi, Overlays as _, Props as _, Role, Show as _, ShowContainer as _, Trigger,
};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::runtime::commands::{FireSource, fire};
use craft_ui::runtime::focus::{FocusTarget, Initial, request};
use craft_ui::runtime::overlay::{Backdrop, CloseReason, OpenFilter, Rank, any_open};
use craft_ui::style::StyleCx;
use craft_ui::testing::focus::is_focused;
use craft_ui::testing::overlay::{PressJudgement, press_judgement};
use craft_ui::testing::pointer::{moved, press, release, secondary};
use craft_ui::testing::{Frames, trial};
use egui::{Context, Event, Id, LayerId, Pos2, Rect, Response, Ui, Vec2};
use std::cell::{Cell, RefCell};

const OPENER: Rect = Rect { min: Pos2::new(100.0, 100.0), max: Pos2::new(180.0, 120.0) };
const BELOW: Rect = Rect { min: Pos2::new(600.0, 500.0), max: Pos2::new(660.0, 530.0) };

#[trial]
fn modal_dialogs_dim_the_window_but_modeless_windows_do_not() {
    assert_eq!(Overlay::dialog().rank, Rank::modal(Backdrop::Scrim));
    assert_eq!(Overlay::modal().rank, Rank::modal(Backdrop::None), "a modal popover need not dim its content");
    assert_eq!(Overlay::dialog().modeless().rank, Rank::Window);
}

/// A modal whose focus starts on its first stop: the content here has no field and no default button, where a dialog
/// starts with nothing focused.
fn modal_on_first() -> Overlay {
    let mut overlay = Overlay::modal();
    overlay.focus = overlay.focus.initial(Initial::First);
    overlay
}

#[trial]
fn a_modal_blocks_hits_and_commands_below_and_returns_focus_on_close() {
    let mut frames = Frames::new();
    let below = RefCell::new(Seen::default());
    let content = RefCell::new(Seen::default());
    let opened = Opened::default();
    let (open, close) = (Cell::new(false), Cell::new(false));
    let opener_id = Cell::new(Id::NULL);
    let mut show = |ui: &mut Ui| {
        opened.shown.set(false);
        let opener = Opener::new(&opened, &content, Trigger::Manual, modal_on_first).open(open.get()).close(close.get());
        let response = opener.id_salt("opener").show_at(ui, OPENER);
        opener_id.set(response.id);
        Leaf::new(&below, Vec2::new(60.0, 30.0)).id_salt("below").show_at(ui, BELOW);
    };
    frames.idle(3, &mut show);
    assert!(fire(&frames.ctx, Go::Go, FireSource::Automation), "a command fires while no modal is open");
    // Focus the opener, then open the modal.
    request(&frames.ctx, FocusTarget::Id(opener_id.get()));
    frames.idle(2, &mut show);
    assert!(is_focused(&frames.ctx, opener_id.get()), "the opener has focus");
    open.set(true);
    frames.frame(vec![], &mut show);
    open.set(false);
    frames.idle(30, &mut show);
    assert!(opened.shown.get(), "the modal shows");
    assert!(any_open(&frames.ctx, OpenFilter::Modal), "the layer stack lists the modal");
    assert!(!fire(&frames.ctx, Go::Go, FireSource::Automation), "a modal blocks commands below it");
    let content_id = content.borrow().id;
    assert!(content_id.is_some_and(|id| is_focused(&frames.ctx, id)), "focus moved into the modal");
    // A press below the modal reaches nothing.
    let at = BELOW.center();
    frames.frame(vec![moved(at)], &mut show);
    frames.frame(vec![press(at)], &mut show);
    frames.frame(vec![release(at)], &mut show);
    frames.idle(2, &mut show);
    assert_eq!(below.borrow().presses, 0, "the modal blocks hits below it");
    assert!(opened.shown.get(), "a press outside does not close a dialog");
    // Closing returns focus to the opener and unblocks commands.
    close.set(true);
    frames.frame(vec![], &mut show);
    close.set(false);
    frames.idle(30, &mut show);
    assert!(!opened.shown.get(), "the modal closed");
    assert!(!any_open(&frames.ctx, OpenFilter::Modal));
    assert!(is_focused(&frames.ctx, opener_id.get()), "focus returned to the opener");
    assert!(fire(&frames.ctx, Go::Go, FireSource::Automation), "commands fire again after the modal closed");
}

/// A control whose modal holds a dropdown [`Opener`] (a click toggles it): a dialog with a dropdown field.
#[derive(Component)]
#[component(overlay)]
struct DialogOpener<'a> {
    #[common]
    common: Common,
    open: bool,
    close: bool,
    close_dropdown: bool,
    dialog: &'a Opened,
    dropdown: &'a Opened,
    option: &'a RefCell<Seen>,
    /// The dropdown opener inside the modal, as it showed last.
    field: &'a Cell<Id>,
}

impl Component for DialogOpener<'_> {
    const ROLE: Role = Role::BUTTON;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: DialogOpenerStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(80.0, 20.0)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("dialog");
        popup.trigger(Trigger::Manual, cx);
        if self.open {
            popup.open(cx);
        }
        if self.close {
            popup.close(cx);
        }
    }

    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("dialog");
        let (dropdown, option, field, close_dropdown) = (self.dropdown, self.option, self.field, self.close_dropdown);
        let shown = ui.show(popup, modal_on_first(), |ui, _| {
            let opener = Opener::new(dropdown, option, Trigger::Click, Overlay::dropdown).close(close_dropdown);
            field.set(opener.id_salt("field").show(ui).id);
        });
        if shown.is_some() {
            self.dialog.shown.set(true);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// #7113: a trackpad tap delivers the press and the release in one frame, which raises the modal it lands on and
/// opens the dropdown in the same pass. The dropdown's layer, already in the order from an earlier open, still goes
/// above the modal, and a tap on its option selects it.
#[trial]
fn a_popup_created_before_a_modal_paints_above_it_and_takes_a_one_frame_tap() {
    let mut frames = Frames::new();
    let (dialog, dropdown) = (Opened::default(), Opened::default());
    let option = RefCell::new(Seen::default());
    let field = Cell::new(Id::NULL);
    let (open, close, close_dropdown) = (Cell::new(false), Cell::new(false), Cell::new(false));
    let mut show = |ui: &mut Ui| {
        dialog.shown.set(false);
        dropdown.shown.set(false);
        let opener = DialogOpener {
            common: Common::new(),
            open: open.get(),
            close: close.get(),
            close_dropdown: close_dropdown.get(),
            dialog: &dialog,
            dropdown: &dropdown,
            option: &option,
            field: &field,
        };
        opener.id_salt("dialog").show_at(ui, OPENER);
    };
    let pulse = |flag: &Cell<bool>, frames: &mut Frames, show: &mut dyn FnMut(&mut Ui)| {
        flag.set(true);
        frames.frame(vec![], &mut *show);
        flag.set(false);
        frames.idle(30, &mut *show);
    };
    let tap = |at: Pos2, frames: &mut Frames, show: &mut dyn FnMut(&mut Ui)| {
        frames.frame(vec![moved(at), press(at), release(at)], &mut *show);
        frames.idle(30, &mut *show);
    };
    let rect_of = |frames: &Frames, id: Option<Id>| id.and_then(|id| frames.ctx.read_response(id)).map(|response| (response.rect, response.layer_id));
    frames.idle(3, &mut show);

    // The dropdown opens once inside the dialog and closes, so its layer is in the order; then the dialog closes.
    pulse(&open, &mut frames, &mut show);
    assert!(dialog.shown.get(), "the dialog shows");
    let (field_rect, _) = rect_of(&frames, Some(field.get())).expect("the dropdown field shows");
    tap(field_rect.center(), &mut frames, &mut show);
    assert!(dropdown.shown.get(), "a tap opens the dropdown");
    pulse(&close_dropdown, &mut frames, &mut show);
    assert!(!dropdown.shown.get(), "the dropdown closed");
    pulse(&close, &mut frames, &mut show);
    assert!(!dialog.shown.get(), "the dialog closed");

    // The dialog opens again above the dropdown's old place; a tap on its field raises it and opens the dropdown.
    pulse(&open, &mut frames, &mut show);
    let (field_rect, modal_layer) = rect_of(&frames, Some(field.get())).expect("the dropdown field shows again");
    tap(field_rect.center(), &mut frames, &mut show);
    assert!(dropdown.shown.get(), "a tap opens the dropdown again");
    let (option_rect, option_layer) = rect_of(&frames, option.borrow().id).expect("the option shows");
    let order: Vec<LayerId> = frames.ctx.memory(|memory| memory.layer_ids().collect());
    let place = |layer| order.iter().position(|shown| *shown == layer);
    assert_ne!(option_layer, modal_layer, "the option is on the dropdown's own layer");
    assert!(place(option_layer) > place(modal_layer), "#7113: the dropdown paints above the modal it opened from");
    assert_eq!(frames.ctx.layer_id_at(option_rect.center()), Some(option_layer), "the option's point hits the dropdown");

    // A one-frame tap on the option selects it.
    let presses = option.borrow().presses;
    tap(option_rect.center(), &mut frames, &mut show);
    assert!(option.borrow().presses > presses, "the tap's press reaches the option");
    assert_eq!(option.borrow().activations, 1, "the tap selects the option");
}

/// A modal dialog whose card holds a button with a context menu and a body of its own below it: a dialog with a menu
/// inside its card. Each real pass of the card adds a line to `runs` for a failing assertion's report: what the
/// window's layer stack judged the pass's presses by (the absorb zone, the absorbed press and the layers of the last
/// pass with their parents and ranks) and the card's menu with the layer it shows in and how it closed.
#[derive(Component)]
#[component(overlay)]
struct CardMenu<'a> {
    #[common]
    common: Common,
    open: bool,
    dialog: &'a Opened,
    /// The button the menu hangs from, as it showed last.
    target: &'a RefCell<Option<Rect>>,
    /// The card's own body, which a press the menu takes should not reach.
    inside: &'a RefCell<Seen>,
    /// The card body's response, as it showed last.
    body: &'a Cell<Option<Id>>,
    /// How the card's menu closed, in the order it closed.
    menus: &'a RefCell<Vec<CloseReason>>,
    /// What each real pass judged its presses by, and how the menu closed in it.
    runs: &'a RefCell<Vec<String>>,
}

impl Component for CardMenu<'_> {
    const ROLE: Role = Role::BUTTON;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: CardMenuStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(80.0, 20.0)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("dialog");
        popup.trigger(Trigger::Manual, cx);
        if self.open {
            popup.open(cx);
        }
    }

    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("dialog");
        let (dialog, target, inside, body, menus, runs) = (self.dialog, self.target, self.inside, self.body, self.menus, self.runs);
        let shown = ui.show(popup, modal_on_first(), |ui, _| {
            let live = !ui.is_sizing_pass() && !craft_ui::runtime::measure::active(ui.ctx());
            let opener = Button::new("Target").show(ui);
            if live {
                *target.borrow_mut() = Some(opener.rect);
            }
            let menu = Menu::context("Card menu", &opener).id_salt("card-menu").show(ui, |ui, _| {
                MenuItem::new("Row").show(ui);
            });
            if live && let Some(why) = menu.output.closed {
                menus.borrow_mut().push(why);
            }
            // The card's body stands well below the button, so a press on it lands beside the menu that hangs from the
            // button, and beside that menu's anchor.
            ui.add_space(120.0);
            let leaf = Leaf::new(inside, Vec2::new(220.0, 120.0)).id_salt("card-body").show(ui);
            if live {
                body.set(Some(leaf.id));
                runs.borrow_mut().push(judged_pass(ui.ctx(), menu.output.response.id, menu.output.closed));
            }
        });
        if shown.is_some() {
            dialog.shown.set(true);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// What the layer stack judged the running pass's presses by and how the card's menu closed in it, for a failing
/// assertion's report: the absorb zone, the absorbed press, each layer of the last pass (its id, rank, the layer it was
/// opened from inside, its rect and anchor, and whether it fades out) and the menu's layer with its close reason.
fn judged_pass(ctx: &Context, menu: Id, closed: Option<CloseReason>) -> String {
    let judged = press_judgement(ctx);
    let layers: Vec<String> = judged
        .layers
        .iter()
        .map(|entry| {
            let exiting = if entry.exiting { " exiting" } else { "" };
            format!("{:?} {:?} parent {:?} at {:?} anchor {:?}{exiting}", entry.id, entry.rank, entry.parent, entry.rect, entry.anchor)
        })
        .collect();
    format!(
        "pass {}: zone {:?}, absorbed press {:?}, menu {menu:?} closed {closed:?}, stack [{}]",
        ctx.cumulative_pass_nr(),
        judged.zone,
        judged.absorbed,
        layers.join("; ")
    )
}

/// The pointer events of `events` in words: each move's point, and each button's press or release and its point.
fn events_in_words(events: &[Event]) -> String {
    let words: Vec<String> = events
        .iter()
        .map(|event| match event {
            Event::PointerMoved(pos) => format!("move to {pos:?}"),
            Event::PointerButton { pos, button, pressed, .. } => format!("{button:?} {} at {pos:?}", if *pressed { "press" } else { "release" }),
            other => format!("{other:?}"),
        })
        .collect();
    if words.is_empty() { "no events".to_owned() } else { words.join(", ") }
}

/// One frame with `events`, traced in `trace` for a failing assertion's report: the events in words, how many passes
/// the frame ran and a line for each real pass of the card (`judged_pass`).
fn traced(events: Vec<Event>, frames: &mut Frames, show: &mut dyn FnMut(&mut Ui), runs: &RefCell<Vec<String>>, trace: &mut Vec<String>) {
    let words = events_in_words(&events);
    runs.borrow_mut().clear();
    let passes = frames.frame(events, &mut *show);
    let lines: Vec<String> = std::mem::take(&mut *runs.borrow_mut());
    let judged = if lines.is_empty() { "no real pass of the card".to_owned() } else { lines.join(" | ") };
    trace.push(format!("frame {}: {words}, {passes} passes; {judged}", trace.len()));
}

/// The card the dialog shows hosts the menu opened inside it, so a press elsewhere on the card is outside the menu: the
/// menu closes (`CloseReason::Outside`), the dialog stays open, and the press reaches neither the card's body nor what
/// lies below the dialog.
#[trial]
fn a_press_beside_a_menu_inside_a_modal_dialog_closes_the_menu_and_leaves_the_dialog() {
    let mut frames = Frames::new();
    let (dialog, inside, below) = (Opened::default(), RefCell::new(Seen::default()), RefCell::new(Seen::default()));
    let (target, body, menus) = (RefCell::new(None), Cell::new(None), RefCell::new(Vec::new()));
    let judged = RefCell::new(PressJudgement::default());
    // Each real pass of the card leaves a line of what it judged its presses by, and each frame a line of its events
    // and those passes: a failing assertion reports them, so a failure says where the press went.
    let runs: RefCell<Vec<String>> = RefCell::new(Vec::new());
    let mut trace: Vec<String> = Vec::new();
    let open = Cell::new(false);
    let mut show = |ui: &mut Ui| {
        dialog.shown.set(false);
        let card = CardMenu {
            common: Common::new(),
            open: open.get(),
            dialog: &dialog,
            target: &target,
            inside: &inside,
            body: &body,
            menus: &menus,
            runs: &runs,
        };
        card.id_salt("dialog").show_at(ui, OPENER);
        Leaf::new(&below, Vec2::new(60.0, 30.0)).id_salt("below").show_at(ui, BELOW);
        // The press zone and the layers the layer stack judged the last pass by, for the trial's preconditions.
        if !ui.is_sizing_pass() && !craft_ui::runtime::measure::active(ui.ctx()) {
            *judged.borrow_mut() = press_judgement(ui.ctx());
        }
    };
    let right_click = |at: Pos2, frames: &mut Frames, show: &mut dyn FnMut(&mut Ui), runs: &RefCell<Vec<String>>, trace: &mut Vec<String>| {
        traced(vec![moved(at)], frames, &mut *show, runs, trace);
        traced(vec![secondary(at, true)], frames, &mut *show, runs, trace);
        traced(vec![secondary(at, false)], frames, &mut *show, runs, trace);
        for _ in 0..10 {
            traced(vec![], frames, &mut *show, runs, trace);
        }
    };
    frames.idle(3, &mut show);
    open.set(true);
    frames.frame(vec![], &mut show);
    open.set(false);
    frames.idle(30, &mut show);
    assert!(dialog.shown.get(), "the dialog shows");
    // A right click on the button in the card opens the card's menu.
    let button = (*target.borrow()).expect("the button in the card shows");
    right_click(button.center(), &mut frames, &mut show, &runs, &mut trace);
    let judged = {
        let judged = judged.borrow();
        (*judged).clone()
    };
    let card = judged
        .layers
        .iter()
        .find(|entry| matches!(entry.rank, Rank::Modal { .. }))
        .map(|entry| entry.rect)
        .expect("the dialog's card is a modal layer");
    let menu =
        judged.layers.iter().find(|entry| matches!(entry.rank, Rank::Popup)).map(|entry| entry.rect).expect("the card's menu is a popup layer");
    assert!(menus.borrow().is_empty(), "the click that opened the menu leaves it open:\n{}", trace.join("\n"));
    // A press elsewhere on the card, away from the menu and its anchor, closes the menu and nothing else.
    let body = body.get().and_then(|id| frames.ctx.read_response(id)).map(|response| response.rect).expect("the card's body shows");
    let at = body.center_bottom() - Vec2::new(0.0, 8.0);
    assert!(
        card.contains(at) && !menu.contains(at),
        "the press at {at:?} lands on the card {card:?}, beside the menu {menu:?}:\n{}",
        trace.join("\n")
    );
    assert!(
        judged.layers.iter().filter_map(|entry| entry.anchor).all(|anchor| !anchor.contains(at)),
        "the press at {at:?} lands away from the menu's anchor:\n{}",
        trace.join("\n")
    );
    traced(vec![moved(at)], &mut frames, &mut show, &runs, &mut trace);
    traced(vec![press(at)], &mut frames, &mut show, &runs, &mut trace);
    traced(vec![release(at)], &mut frames, &mut show, &runs, &mut trace);
    for _ in 0..10 {
        traced(vec![], &mut frames, &mut show, &runs, &mut trace);
    }
    assert_eq!(*menus.borrow(), vec![CloseReason::Outside], "the press elsewhere on the card closes the menu:\n{}", trace.join("\n"));
    assert!(dialog.shown.get(), "the dialog stays open:\n{}", trace.join("\n"));
    assert_eq!(inside.borrow().presses, 0, "the menu takes the press that closes it:\n{}", trace.join("\n"));
    // A press outside the dialog closes the menu opened again inside it, and reaches nothing below the dialog.
    menus.borrow_mut().clear();
    right_click(button.center(), &mut frames, &mut show, &runs, &mut trace);
    assert!(menus.borrow().is_empty(), "the menu opens again inside the dialog:\n{}", trace.join("\n"));
    let outside = BELOW.center();
    traced(vec![moved(outside)], &mut frames, &mut show, &runs, &mut trace);
    traced(vec![press(outside)], &mut frames, &mut show, &runs, &mut trace);
    traced(vec![release(outside)], &mut frames, &mut show, &runs, &mut trace);
    for _ in 0..10 {
        traced(vec![], &mut frames, &mut show, &runs, &mut trace);
    }
    assert_eq!(*menus.borrow(), vec![CloseReason::Outside], "the press outside the dialog closes the menu:\n{}", trace.join("\n"));
    assert!(dialog.shown.get(), "the dialog stays open:\n{}", trace.join("\n"));
    assert!(any_open(&frames.ctx, OpenFilter::Modal), "the dialog is still the open modal:\n{}", trace.join("\n"));
    assert_eq!(below.borrow().presses, 0, "the press outside the dialog reaches nothing below it:\n{}", trace.join("\n"));
}
