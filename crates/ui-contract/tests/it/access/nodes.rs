//! Announcements.

use craft_ui::contract::announce;
use craft_ui::contract::{Politeness, Role as UiRole};
use craft_ui::testing::{Frames, trial};

// The same text announced in two frames in a row reads as a change both times, and a discarded pass that
// announces again adds nothing.
#[trial]
fn a_repeated_announcement_changes_the_status_text_and_a_discarded_pass_adds_nothing() {
    let mut frames = Frames::new();
    let mut said = Vec::new();
    for _ in 0..3 {
        frames.frame(vec![], |ui| {
            announce(ui.ctx(), "Saved", Politeness::Polite);
            announce(ui.ctx(), "Saved", Politeness::Polite);
        });
        let tree = frames.tree();
        let status = tree.with_role(UiRole::STATUS);
        said.push(tree.name_of(status[0]).map(str::to_owned));
        assert_eq!(craft_ui::runtime::frame::take_output(&frames.ctx).announcements, ["Saved"]);
    }
    assert!(said.windows(2).all(|pair| pair[0] != pair[1]), "{said:?}");
    assert!(said.iter().all(|text| text.as_deref().is_some_and(|text| text.starts_with("Saved") && !text.contains(". "))));
}
