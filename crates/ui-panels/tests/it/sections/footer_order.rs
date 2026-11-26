//! The order of a dialog footer's buttons by role.

use craft_ui::sections::DialogRole;
use craft_ui::testing::internals::footer_order;
use craft_ui::testing::trial;

/// An alternative answer stands between Cancel and the accepting button off Windows ("Cancel, Save as…, Save
/// anyway"), and a destructive one before Cancel ("Don't save, Cancel, Save"); Windows turns the row around.
#[trial]
fn an_alternative_answer_stands_between_cancel_and_accept() {
    let roles = [DialogRole::Accept, DialogRole::Alternative, DialogRole::Cancel];
    assert_eq!(footer_order(&roles, false), [2, 1, 0], "Cancel, the alternative, then Accept");
    assert_eq!(footer_order(&roles, true), [0, 1, 2], "Windows: Accept, the alternative, Cancel");
    let roles = [DialogRole::Accept, DialogRole::Cancel, DialogRole::Destructive];
    assert_eq!(footer_order(&roles, false), [2, 1, 0], "the destructive answer, Cancel, then Accept");
}
