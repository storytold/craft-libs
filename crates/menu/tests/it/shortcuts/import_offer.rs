//! The first-launch import offer.

use craft_keymap::import::kys;
use craft_keymap::{Binding, CommandDef, ForeignSet, ImportOptions, Keymap, Overrides, Platform, Scope, Scopes};
use craft_menu::shortcuts::{ImportOffer, NoOffer, OfferRecord};
use craft_ui::testing::trial;

const PSP: &str = "<photoshop-keyboard-shortcuts version=\"4\" filename=\"$$$/FileName/Presets/KeyboardCustomization/Mine=Mine\">\
    <command name=\"Save\"><shortcut>Cmd+S</shortcut></command>\
    <command name=\"Rulers\"><shortcut>Cmd+K</shortcut></command></photoshop-keyboard-shortcuts>";

fn keymap() -> Keymap {
    let command = |id: &str, label: &str, key: &str| CommandDef::new(id, label, Scope::app()).with_default(Binding::parse(key).expect("a binding"));
    Keymap::new(vec![command("file.save", "Save", "Cmd+S"), command("view.rulers", "Rulers", "Cmd+R")], Scopes::new(), Platform::Mac)
}

fn set(text: &str) -> ForeignSet {
    kys::read(text).expect("a set")
}

fn prepare(km: &Keymap, record: Option<&OfferRecord>) -> Result<ImportOffer, NoOffer> {
    ImportOffer::prepare("/ps/Keyboard Shortcuts.psp", &set(PSP), km, &ImportOptions::default(), record)
}

/// No record and no overrides offers; a record, own overrides or a set equal to the defaults gives `NoOffer` with
/// the record to store.
#[trial]
fn the_offer_is_made_once_and_only_for_a_set_that_changes_something() {
    let offer = prepare(&keymap(), None).expect("an offer");
    assert_eq!((offer.set_name.as_str(), offer.changes), ("Mine", 1), "Save keeps its key; Rulers changes");

    let answered = OfferRecord::new("/ps/Keyboard Shortcuts.psp", false);
    let refused = prepare(&keymap(), Some(&answered)).expect_err("answered before");
    assert_eq!((refused.clone(), refused.record("/ps")), (NoOffer::Answered, None));

    let mut own = keymap();
    let mut overrides = Overrides::new();
    overrides.insert("file.save", vec!["Cmd+Shift+S".into()]);
    own.set_overrides(overrides);
    let refused = prepare(&own, None).expect_err("own shortcuts");
    assert_eq!(refused, NoOffer::OwnShortcuts);
    assert_eq!(refused.record("/ps"), Some(OfferRecord::new("/ps", false)));

    let defaults = ImportOffer::prepare("/ps/x.psp", &set(&PSP.replace("Cmd+K", "Cmd+R")), &keymap(), &ImportOptions::default(), None);
    assert_eq!(defaults.expect_err("no change"), NoOffer::NothingChanges);

    let unnamed = ImportOffer::prepare("/ps/My Keys.psp", &set(&PSP.replace(" filename=", " x=")), &keymap(), &ImportOptions::default(), None)
        .expect("an offer");
    assert_eq!(unnamed.set_name, "My Keys.psp", "without a name in the file the offer names the file");
}
