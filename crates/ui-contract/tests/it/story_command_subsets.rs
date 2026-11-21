//! Story command selections stay with each context.

use craft_ui::contract::CommandSet;
use craft_ui::runtime::commands::resolve;
use craft_ui::testing::{Fixture, Registry, trial};
use std::sync::{Arc, Barrier};

#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "subsetAlpha", story)]
enum Alpha {
    #[command(label = "Subset Alpha")]
    Run,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "subsetBeta", story)]
enum Beta {
    #[command(label = "Subset Beta")]
    Run,
}

fn alpha(registry: &mut Registry) {
    registry.add_story_set(__registry_command_alpha());
}
fn beta(registry: &mut Registry) {
    registry.add_story_set(__registry_command_beta());
}

fn selected(alpha_selected: bool, barrier: Arc<Barrier>) {
    let registry = if alpha_selected { Registry::of(&[alpha]) } else { Registry::of(&[beta]) };
    barrier.wait();
    assert!(registry.install_story_sets());
    let mut fixture = Fixture::new(|_| {});
    fixture.settle();
    let alpha_label = resolve(fixture.ctx(), Alpha::Run).label.clone();
    let beta_label = resolve(fixture.ctx(), Beta::Run).label.clone();
    assert_eq!(alpha_label.as_ref() == "Subset Alpha", alpha_selected);
    assert_eq!(beta_label.as_ref() == "Subset Beta", !alpha_selected);
}

#[trial]
fn separate_story_command_subsets_can_run_concurrently() {
    let alpha_registry = Registry::of(&[alpha]);
    let beta_registry = Registry::of(&[beta]);
    assert!(alpha_registry.install_story_sets());
    let mut first = Fixture::new(|_| {});
    first.settle();
    assert!(beta_registry.install_story_sets());
    let mut second = Fixture::new(|_| {});
    second.settle();
    assert_eq!(resolve(first.ctx(), Alpha::Run).label.as_ref(), "Subset Alpha");
    assert_ne!(resolve(first.ctx(), Beta::Run).label.as_ref(), "Subset Beta");
    assert_eq!(resolve(second.ctx(), Beta::Run).label.as_ref(), "Subset Beta");
    assert_ne!(resolve(second.ctx(), Alpha::Run).label.as_ref(), "Subset Alpha");
    let builds = craft_ui::runtime::commands::keymap_builds();
    let mut another = Fixture::new(|_| {});
    another.settle();
    assert_eq!(craft_ui::runtime::commands::keymap_builds(), builds, "a second context reuses its registry's keymap");

    let barrier = Arc::new(Barrier::new(2));
    let first = {
        let barrier = barrier.clone();
        std::thread::spawn(move || selected(true, barrier))
    };
    let second = std::thread::spawn(move || selected(false, barrier));
    assert!(first.join().is_ok());
    assert!(second.join().is_ok());
}
