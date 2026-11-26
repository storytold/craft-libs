//! Selection parts: read-only text the user focuses with a press to select and copy, which Tab passes over
//! (`SelectableText`, a notice's title and message, a toast's text, a metadata table's values).

use craft_theme::variant::Tone;
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, CollectionEvent, Props as _, Show as _};
use craft_ui::notices::{Notice, NoticeAction, ToastStack};
use craft_ui::runtime::env::{RootEnv, WindowKey};
use craft_ui::runtime::focus::{KeyUse, host_canvas};
use craft_ui::runtime::frame::{ChromeToggle, take_output};
use craft_ui::runtime::toast;
use craft_ui::table::{MetadataGroup, MetadataRow, MetadataTable};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::{SelectableText, TextField};
use egui::{Event, Id, Key, Modifiers, Rect, Sense, Vec2, ViewportId};
use std::cell::Cell;
use std::rc::Rc;

fn stop(fixture: &Fixture) -> Option<Id> {
    fixture.focused_stop().map(|(id, _)| id)
}

/// Tab goes from the field before the text to the one after it; a press on the text focuses it, and Select All then
/// Copy puts the whole text on the clipboard. Cut copies it too, as a label's selection does: read-only text removes
/// nothing, and the Cut is the text's, not the app's.
#[trial]
fn tab_passes_over_read_only_text_and_a_press_selects_it_for_copy() {
    let ids = Rc::new([Cell::new(Id::NULL), Cell::new(Id::NULL), Cell::new(Id::NULL)]);
    let seen = ids.clone();
    let mut fixture = Fixture::new(move |ui| {
        seen[0].set(TextField::new("Before", Bind::given(String::new())).id_salt("before").show(ui).response.id);
        seen[1].set(SelectableText::new("Path", "/projects/poster.indd").show(ui).id);
        seen[2].set(TextField::new("After", Bind::given(String::new())).id_salt("after").show(ui).response.id);
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    assert_eq!(stop(&fixture), Some(ids[0].get()));
    fixture.key(Key::Tab, Modifiers::NONE);
    assert_eq!(stop(&fixture), Some(ids[2].get()), "Tab passes over the read-only text");

    let text = fixture.by_label("Path").expect("the text has a node");
    fixture.click(&text);
    assert_eq!(stop(&fixture), Some(ids[1].get()), "a press focuses the text");
    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.frame(vec![Event::Copy]);
    assert_eq!(fixture.copied(), ["/projects/poster.indd"]);
    fixture.frame(vec![Event::Cut]);
    assert_eq!(fixture.copied(), ["/projects/poster.indd"], "Cut copies the selected text");
}

/// Where `text` was painted.
fn painted(fixture: &Fixture, text: &str) -> Rect {
    fixture.texts().iter().find(|painted| painted.text == text).unwrap_or_else(|| panic!("{text:?} is painted")).rect
}

/// Select All then Copy in the focused text.
fn copy_all(fixture: &mut Fixture) -> Vec<String> {
    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.frame(vec![Event::Copy]);
    fixture.copied().to_vec()
}

/// A notice's title and message are two selection parts: Tab goes from the field before the notice to its action and
/// on to the field after it; a press on the message focuses it and Copy copies the message alone; a drag over the
/// start of the title selects part of it.
#[trial]
fn notice_title_and_message_select_and_copy_apart() {
    let ids = Rc::new([Cell::new(Id::NULL), Cell::new(Id::NULL), Cell::new(Id::NULL)]);
    let seen = ids.clone();
    let mut fixture = Fixture::new(move |ui| {
        seen[0].set(TextField::new("Before", Bind::given(String::new())).id_salt("before").show(ui).response.id);
        let notice = Notice::new(Tone::Warn, "The link to poster.png is broken.").title("Missing file").action(NoticeAction::new("Relink"));
        seen[1].set(notice.id_salt("notice").show(ui).response.id);
        seen[2].set(TextField::new("After", Bind::given(String::new())).id_salt("after").show(ui).response.id);
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.key(Key::Tab, Modifiers::NONE);
    assert_eq!(stop(&fixture), Some(ids[1].get()), "Tab reaches the notice's action");
    assert!(fixture.focused_stop().is_some_and(|(_, part)| part.is_some()));
    fixture.key(Key::Tab, Modifiers::NONE);
    assert_eq!(stop(&fixture), Some(ids[2].get()), "Tab passes over the title and message");

    let message = painted(&fixture, "The link to poster.png is broken.");
    fixture.click_at(message.center());
    assert_eq!(stop(&fixture), Some(ids[1].get()), "a press focuses the message");
    assert_eq!(copy_all(&mut fixture), ["The link to poster.png is broken."]);

    let title = painted(&fixture, "Missing file");
    fixture.drag_at(title.left_center() + Vec2::new(1.0, 0.0), Vec2::new(title.width() / 2.0, 0.0));
    fixture.frame(vec![Event::Copy]);
    let copied = fixture.copied().last().cloned().unwrap_or_default();
    assert!(!copied.is_empty() && "Missing file".starts_with(&copied) && copied != "Missing file", "the drag selected {copied:?}");
}

/// A toast card's message is selectable: a press, Select All and Copy copy it and leave the card up, and Tab passes
/// over the text to the card's dismiss button.
#[trial]
fn toast_text_selects_and_copies() {
    let mut fixture = Fixture::new(|ui| {
        let ctx = ui.ctx().clone();
        let window = WindowKey::new("toast-text");
        if toast::cards(&ctx, &window).is_empty() {
            let card = toast::Toast::new("poster.png was saved to Pictures.").tone(Tone::Good).title("Exported").timeout(None);
            toast::post(&ctx, &window, card);
        }
        ToastStack::show(ui, &window, ui.max_rect());
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    let dismiss = fixture.focused_stop();
    fixture.key(Key::Tab, Modifiers::NONE);
    assert_eq!(fixture.focused_stop(), dismiss, "Tab stays on the dismiss button, the card's one Tab stop");
    assert!(dismiss.is_some_and(|(_, part)| part.is_some()), "Tab reaches the dismiss button");

    let message = painted(&fixture, "poster.png was saved to Pictures.");
    fixture.click_at(message.center());
    assert_eq!(copy_all(&mut fixture), ["poster.png was saved to Pictures."]);
    fixture.settle();
    assert!(fixture.texts().iter().any(|painted| painted.text == "Exported"), "the card stays up");
}

/// A metadata table's value is selectable: a press on it, Select All and Copy copy the value, the press selects no
/// row, and Tab passes over the values.
#[trial]
fn metadata_value_copies_without_selecting_the_row() {
    let selected = Rc::new(Cell::new(false));
    let seen = selected.clone();
    let groups = vec![MetadataGroup::new("Camera", vec![MetadataRow::new("Make", "FUJIFILM"), MetadataRow::new("Model", "X100V")])];
    let mut fixture = Fixture::new(move |ui| {
        let out = MetadataTable::new("Metadata", &groups).show(ui);
        if out.table.iter().flat_map(|table| &table.events).any(|event| matches!(event, CollectionEvent::Selection { .. })) {
            seen.set(true);
        }
    });
    fixture.settle();
    for _ in 0..3 {
        fixture.key(Key::Tab, Modifiers::NONE);
        assert!(fixture.focused_stop().is_none_or(|(_, part)| part.is_none()), "Tab passes over the values: {:?}", fixture.focused_stop());
    }

    let value = painted(&fixture, "X100V");
    fixture.click_at(value.center());
    assert_eq!(copy_all(&mut fixture), ["X100V"]);
    assert!(!selected.get(), "the press on the value selected no row");
}

/// In a window with a document canvas whose Tab toggles the panels, read-only text a press or a drag focused to select
/// from is no text field: Tab toggles the panels from it, as from the button a press focused, and leaves the focus on
/// it. Guards the press that focuses a selection part, which no framework shape wins (egui's text widget takes it), being
/// recorded as a press.
#[trial]
fn tab_toggles_the_panels_from_read_only_text_a_press_or_drag_focused() {
    let text = Rc::new(Cell::new(Id::NULL));
    let seen = text.clone();
    let canvas = Id::unique("document canvas");
    let mut fixture = Fixture::new(move |ui| {
        Button::new("Save").show(ui);
        seen.set(SelectableText::new("Path", "/projects/poster.indd").show(ui).id);
        ui.interact(ui.available_rect_before_wrap(), canvas, Sense::click());
        host_canvas(ui.ctx(), canvas, KeyUse::CANVAS);
    });
    let mut root = RootEnv::default();
    root.tab_toggles_chrome = true;
    fixture.root(root);
    fixture.settle();
    let tab = |fixture: &mut Fixture| {
        take_output(fixture.ctx());
        fixture.key(Key::Tab, Modifiers::NONE);
        take_output(fixture.ctx()).chrome_toggles
    };
    let path = painted(&fixture, "/projects/poster.indd");
    for dragged in [false, true] {
        let button = fixture.by_label("Save").expect("the button has a node");
        fixture.click(&button);
        assert_eq!(tab(&mut fixture), [(ViewportId::ROOT, ChromeToggle::Panels)], "the pressed button (dragged {dragged})");
        if dragged {
            fixture.drag_at(path.left_center() + Vec2::new(1.0, 0.0), Vec2::new(path.width() / 2.0, 0.0));
        } else {
            fixture.click_at(path.center());
        }
        assert_eq!(stop(&fixture), Some(text.get()), "the gesture focused the text (dragged {dragged})");
        assert_eq!(tab(&mut fixture), [(ViewportId::ROOT, ChromeToggle::Panels)], "the pressed text (dragged {dragged})");
        assert_eq!(stop(&fixture), Some(text.get()), "the toggle moved no focus (dragged {dragged})");
    }
}
