//! A package's stories of a type another package registers run that type's per-story cases in the package's suite;
//! the cases about the type as a whole (its required states) stay with the package that registers it. The type's own
//! stories list before the visitor's.

use craft_ui::contract::{Common, Component, Cx, MeasureCx, Props as _, Role, Show as _};
use craft_ui::style::StyleCx;
use craft_ui::testing::{Required, Story, StoryData, StoryOutput, StoryRegistration, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Ui, Vec2};

/// The package that declares a story of [`Visited`] without registering it.
const VISITOR: &str = "craft-ui-visitor";

/// A plain box, registered by this binary.
#[derive(craft_ui::contract::Component)]
struct Visited {
    #[common]
    common: Common,
}

impl Component for Visited {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = egui::Response;

    fn style(_: &StyleCx<'_>, _: VisitedStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(80.0, 40.0)
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}

fn plain_box(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Visited { common: Common::new() }.label("Box").show(ui).into()
}

// The visitor's story covers no state: a Default story is the registering package's to have. Its file sorts before
// the home story's.
fn visitor_story() -> StoryRegistration {
    StoryRegistration::new(
        VISITOR,
        "craft_ui_visitor",
        std::any::type_name::<Visited>,
        <Visited as craft_ui::testing::Registered>::registration,
        Story::new("plain_box", plain_box),
    )
    .at("a.rs", 1)
}

fn home_story() -> StoryRegistration {
    StoryRegistration::new(
        env!("CARGO_PKG_NAME"),
        "craft_ui_contract",
        std::any::type_name::<Visited>,
        <Visited as craft_ui::testing::Registered>::registration,
        Story::new("home_box", plain_box).covers(&[Required::Default]),
    )
    .at("z.rs", 1)
}

pub(crate) fn register_fixture(registry: &mut craft_ui::testing::Registry) {
    registry.add_story(visitor_story());
    registry.add_story(home_story());
}

fn fixture_registry() -> craft_ui::testing::Registry {
    craft_ui::testing::Registry::of(&[register_fixture])
}

#[trial]
fn a_types_own_stories_list_before_another_packages() {
    let visited = std::any::type_name::<Visited>();
    let registry = fixture_registry();
    let names: Vec<&str> = registry.stories().iter().filter(|entry| (entry.type_name)() == visited).map(|entry| entry.story.name).collect();
    assert_eq!(names, ["home_box", "plain_box"], "the registering package's story comes first");
}

#[trial]
fn stories_of_another_packages_type_run_its_cases_in_the_declaring_package() {
    let trials = craft_ui::testing::suite::trials(VISITOR, &fixture_registry());
    let names: Vec<String> = trials.iter().map(|trial| trial.name().to_owned()).collect();
    let visited = std::any::type_name::<Visited>();
    assert!(names.contains(&format!("suite::{visited}::reads")), "the visitor runs the visited type's cases: {names:?}");
    assert!(!names.iter().any(|name| name.starts_with("suite::stories::")), "no story is reported unlinked: {names:?}");
    let arguments = libtest_mimic::Arguments { test_threads: Some(1), quiet: true, ..libtest_mimic::Arguments::default() };
    let conclusion = libtest_mimic::run(&arguments, trials);
    assert_eq!(conclusion.num_failed, 0, "the cases pass without the type's required states");
}
