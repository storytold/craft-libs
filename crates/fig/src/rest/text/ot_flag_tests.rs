//! OpenType feature toggles through override layers: each list comes from the topmost layer that
//! has it, so an override that turns a feature on wins over the main turning it off.

use super::super::tests::{arr, o};
use super::super::{Kv, View};
use super::text_style;

fn tags(t: &[&'static str]) -> Kv<'static> {
    arr(t.iter().map(|n| Kv::Enum(n)).collect())
}

#[test]
fn an_override_turning_a_feature_on_wins_over_the_main_turning_it_off() {
    let main = o(vec![("toggledOffOTFeatures", tags(&["LIGA"])), ("toggledOnOTFeatures", tags(&["SS01"]))]);
    let over = o(vec![("toggledOnOTFeatures", tags(&["LIGA"]))]);
    let v = View::of(&main).over(&View::of(&over));
    let flags = &text_style(&v, None)["opentypeFlags"];
    assert_eq!(flags["LIGA"], 1, "{flags}");
    // The override's list replaces the main's: SS01 is no longer on.
    assert!(flags.get("SS01").is_none(), "{flags}");
    // Turned off over a main turning it on, it is off.
    let (main, over) = (o(vec![("toggledOnOTFeatures", tags(&["LIGA"]))]), o(vec![("toggledOffOTFeatures", tags(&["LIGA"]))]));
    let v = View::of(&main).over(&View::of(&over));
    assert_eq!(text_style(&v, None)["opentypeFlags"]["LIGA"], 0);
}
