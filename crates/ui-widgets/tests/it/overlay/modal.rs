//! Modals: blocked hits and commands below, focus return, and the popups opened from inside them.

use super::kit::{Go, Leaf, Opened, Opener, PlainStyle, Seen};
use craft_behavior::Sense;
use craft_ui::contract::{Common, Component, Cx, MeasureCx, Overlay, OverlayUi, Overlays as _, Props as _, Role, Show as _, Trigger};
use craft_ui::runtime::commands::{FireSource, fire};
use craft_ui::runtime::focus::{FocusTarget, Initial, request};
use craft_ui::runtime::overlay::{Backdrop, OpenFilter, Rank, any_open};
use craft_ui::style::StyleCx;
use craft_ui::testing::focus::is_focused;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use egui::{Id, LayerId, Pos2, Rect, Response, Ui, Vec2};
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
