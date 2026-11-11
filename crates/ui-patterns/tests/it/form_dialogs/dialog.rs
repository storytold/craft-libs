//! `ParamDialog`: the dialog's own undo history and the Preview box's hold-to-compare.

use craft_ui::contract::{Bind, Role};
use craft_ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog};
use craft_ui::testing::{Fixture, trial};
use egui::load::SizedTexture;
use egui::{Key, Modifiers, TextureId, vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const SPEC: &str = r#"{"radius":0..100=10,"smooth":bool}"#;

// A slider drag is one step and a check box click another: Undo takes them back one by one, Redo brings the drag
// back.
#[trial]
fn undo_steps_the_dialog_values_and_a_drag_is_one_step() {
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let shown = values.clone();
    let mut fixture = Fixture::new(move |ui| {
        ParamDialog::new("blur", "Blur", Param::parse_list(SPEC)).values(Bind::bound(&mut *shown.borrow_mut())).history(true).show(ui);
    });
    fixture.settle();
    let radius = || values.borrow().number("radius");
    let smooth = || values.borrow().bool("smooth").unwrap_or(false);
    let opened = radius();

    let slider = fixture.by_role(Role::SLIDER).into_iter().next().expect("the radius row has a slider");
    fixture.drag(&slider, vec2(60.0, 0.0)).settle();
    let dragged = radius();
    assert_ne!(dragged, opened, "the drag moved the radius");
    let check = fixture.by_label("Smooth").expect("the smooth row has a check box");
    fixture.click(&check).settle();
    assert!(smooth());

    fixture.key(Key::Z, Modifiers::COMMAND).settle();
    assert_eq!((radius(), smooth()), (dragged, false), "the first Undo takes back the click");
    fixture.key(Key::Z, Modifiers::COMMAND).settle();
    assert_eq!(radius(), opened, "the second Undo takes back the whole drag");
    fixture.key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT).settle();
    assert_eq!(radius(), dragged, "Redo brings the drag back");
}

// Holding the Preview box compares and leaves the preview on; the release turns it off.
#[trial]
fn holding_the_preview_box_compares_and_the_release_toggles_it() {
    let seen = Rc::new(Cell::new((false, false)));
    let out = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let picture = SizedTexture::new(TextureId::default(), vec2(64.0, 48.0));
        let shown = ParamDialog::new("blur", "Blur", Param::parse_list(SPEC)).preview(picture).show(ui);
        out.set((shown.preview_on, shown.comparing));
    });
    fixture.settle();
    assert_eq!(seen.get(), (true, false));
    let preview = fixture
        .by_role(Role::CHECK_BOX)
        .into_iter()
        .find(|target| fixture.tree().name_of(target.node) == Some("Preview"))
        .expect("the footer has the Preview box");
    fixture.press(&preview);
    fixture.idle(1);
    assert_eq!(seen.get(), (true, true), "held: the preview stays on and the pane compares");
    fixture.release(&preview).settle();
    assert_eq!(seen.get(), (false, false), "the release turns the preview off");
}
