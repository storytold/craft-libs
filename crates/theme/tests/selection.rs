//! The component set: the app states it, the user's own choice for the app overrides it, and the suite's choice never
//! carries it.

use craft_theme::{AppChoice, AppDefaults, ComponentSet, ModeChoice, Size, SuiteChoice, ThemeId, ThemePair};

#[test]
fn the_component_set_is_the_apps_unless_the_user_chose_one_for_it_and_never_the_suites() {
    let defaults = AppDefaults {
        theme: ThemePair::same(ThemeId::suite_default()),
        mode: ModeChoice::Light,
        size: Size::Comfortable,
        sizes: Default::default(),
        set: ComponentSet::Productivity,
    };
    let suite = SuiteChoice::default().with_defaults(&defaults);
    assert_eq!(suite.set, ComponentSet::Productivity, "a fresh install shows the app's set");
    let follows = AppChoice::new(None, None, None).apply(&suite);
    assert_eq!(follows.set, ComponentSet::Productivity, "an app choice without a set keeps the app's");
    let own = AppChoice::new(None, None, None).with_set(ComponentSet::Creative).apply(&suite);
    assert_eq!(own.set, ComponentSet::Creative, "the user's set for the app wins");
    let stored: AppChoice = serde_json::from_str(r#"{"theme":null,"mode":null,"size":null}"#).expect("a stored choice without a set reads");
    assert_eq!(stored.apply(&suite).set, ComponentSet::Productivity);
    let shared = SuiteChoice::from(own);
    assert_eq!(shared.with_defaults(&defaults).set, ComponentSet::Productivity, "the suite's choice carries no set to other apps");
}
