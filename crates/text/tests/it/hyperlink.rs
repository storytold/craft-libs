//! The Place in This Document list: a place answers its index among the places, not its row.

use craft_text::ui::hyperlink::{CurrentLink, Hyperlink, HyperlinkAnswer, HyperlinkDialog, LinkPlace, LinkPlaceGroup, LinkTarget};
use craft_ui::testing::{Fixture, trial};
use std::cell::RefCell;
use std::rc::Rc;

#[trial]
fn a_grouped_place_answers_its_flat_index() {
    let groups = vec![
        LinkPlaceGroup::new("Headings", vec![LinkPlace::new("Intro", 0), LinkPlace::new("Method", 0)]),
        LinkPlaceGroup::new("Bookmarks", vec![LinkPlace::new("fig1", 0), LinkPlace::new("fig2", 1)]),
    ];
    let dialog = Rc::new(RefCell::new(HyperlinkDialog::new(CurrentLink::None, None).place_groups(groups)));
    let answer = Rc::new(RefCell::new(None));
    let (shown, got) = (dialog.clone(), answer.clone());
    let mut fixture = Fixture::new(move |ui| {
        if let Some(result) = shown.borrow_mut().show(ui).answer {
            *got.borrow_mut() = Some(result);
        }
    });
    fixture.idle(30);
    let place = fixture.by_label("Place in This Document").expect("the Place radio");
    fixture.click(&place).idle(4);
    let pick = fixture.by_label("fig2").expect("the second place of the second group");
    fixture.click(&pick).idle(4);
    let ok = fixture.by_label("OK").expect("OK");
    fixture.click(&ok).idle(30);
    // Rows run Top, Headings, Intro, Method, Bookmarks, fig1, fig2; fig2 is the fourth place.
    assert_eq!(*answer.borrow(), Some(HyperlinkAnswer::set(Hyperlink::new(LinkTarget::Place(3)), None)));
}
