//! `SectionLayout`: reading a saved arrangement leniently, reconciling it with the registered sections, and the
//! rules for hiding, pinning and moving sections.

use craft_ui::sections::{SectionKey, SectionLayout};
use craft_ui::testing::trial;
use serde::{Deserialize, Serialize};

const IDS: [&str; 5] = ["albums", "local", "byDate", "folders", "keywords"];

fn key(id: &str) -> SectionKey {
    SectionKey::new(id)
}

fn registered() -> Vec<SectionKey> {
    IDS.iter().map(|id| key(id)).collect()
}

fn order(layout: &SectionLayout) -> Vec<&str> {
    layout.states().iter().map(|state| state.key.as_str()).collect()
}

#[derive(Serialize, Deserialize)]
struct Settings {
    sidebar: SectionLayout,
}

fn read(text: &str) -> SectionLayout {
    let mut layout = toml::from_str::<Settings>(text).expect("a saved arrangement always reads").sidebar;
    layout.reconcile(&registered());
    layout
}

#[trial]
fn a_damaged_arrangement_is_made_whole() {
    let layout = read(
        "sidebar = [{ id = \"keywords\", collapsed = true }, { id = \"map\" }, \"folders\", { id = \"keywords\", hidden = true }, \
         { id = 7 }, { id = \"albums\", hidden = \"yes\", folded = 1 }]",
    );
    assert_eq!(
        order(&layout),
        ["keywords", "folders", "albums", "local", "byDate"],
        "unknown ids are dropped, missing ones follow in registration order"
    );
    assert!(layout.is_folded(&key("keywords")) && !layout.is_hidden(&key("keywords")), "the first entry for a section counts; `collapsed` is read");
    assert!(!layout.is_hidden(&key("albums")) && !layout.is_folded(&key("albums")), "a flag that is not true or false is off");
}
