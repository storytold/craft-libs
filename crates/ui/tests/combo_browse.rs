//! Press-and-browse from outside the crate: the row helpers a custom `ComboBox::show_ui` menu
//! needs are public (`craft_ui::browse_hover`, `craft_ui::browse_pick`), so an application's own
//! row callbacks highlight under a held press and are chosen by the release over them, exactly as
//! the built-in choices are (#469).

use craft_ui::{ComboBox, browse_hover, browse_pick};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::{Harness, kittest::Queryable as _};

/// A combo box whose rows the app draws itself, reporting where its button is.
struct Custom {
    picked: Option<usize>,
    button: Rect,
}

fn custom() -> Harness<'static, Custom> {
    Harness::builder().with_size(vec2(400.0, 400.0)).build_ui_state(
        |ui, s: &mut Custom| {
            let inner = ComboBox::new("size", "Size").selected_text("None").show_ui(ui, |ui| {
                for (i, name) in ["Small", "Medium", "Large"].into_iter().enumerate() {
                    let row = ui.selectable_label(false, name);
                    // The helpers a custom row uses, called the way an application would.
                    let row = browse_hover(ui, row);
                    if row.clicked() || browse_pick(ui, &row) {
                        s.picked = Some(i);
                    }
                }
            });
            s.button = inner.response.rect;
        },
        Custom { picked: None, button: Rect::NOTHING },
    )
}

fn pointer_button<S>(h: &mut Harness<'_, S>, at: Pos2, pressed: bool) {
    if pressed {
        h.event(Event::PointerMoved(at));
        h.step();
    }
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.step();
}

/// A row the app draws itself, through the public helpers, opens on the press, stays up while the
/// button is held and is chosen by the release over it — the contract `show_index` gets built-in
/// and the custom `show_ui*` menu methods need too.
#[test]
fn a_custom_row_browses_on_the_press_and_the_release_over_it_picks() {
    let mut h = custom();
    h.run();
    let button = h.state().button.center();
    pointer_button(&mut h, button, true);
    h.run();
    assert!(h.query_by_label("Medium").is_some(), "the press opens the menu");
    assert_eq!(h.state().picked, None, "and chooses nothing yet");

    let entry = h.get_by_label("Large").rect().center();
    h.event(Event::PointerMoved(entry));
    h.step();
    pointer_button(&mut h, entry, false);
    assert_eq!(h.state().picked, Some(2), "the release over the custom row chooses it");
    h.run();
    assert!(h.query_by_label("Medium").is_none(), "picking closes the menu");
}
