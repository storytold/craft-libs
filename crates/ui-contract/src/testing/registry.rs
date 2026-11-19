//! Explicit registrations selected by a test executable or the gallery.

use super::{StoryRegistration, Trial};
use crate::contract::Registration;
use crate::runtime::commands::SetEntry;

/// A component or pattern whose derive supplies its suite registration.
pub trait Registered {
    /// The registration for this concrete type.
    fn registration() -> Registration;
}

/// The suite items selected by a caller.
#[derive(Default)]
pub struct Registry {
    registrations: Vec<Registration>,
    stories: Vec<StoryRegistration>,
    trials: Vec<Trial>,
    story_sets: Vec<SetEntry>,
}

impl Registry {
    /// Collects the items contributed by each selected crate.
    pub fn of(crates: &[fn(&mut Registry)]) -> Self {
        let mut registry = Self::default();
        for register in crates {
            register(&mut registry);
        }
        registry.registrations.sort_by_key(|entry| match entry {
            Registration::Component(probe) => (probe.crate_name, probe.crate_target, probe.module, probe.type_name()),
            Registration::Pattern(probe) => (probe.crate_name, probe.crate_target, probe.module, probe.type_name()),
        });
        registry.stories.sort_by_key(|story| ((story.type_name)(), (story.registration)().crate_name() != story.crate_name, story.file, story.line));
        registry.trials.sort_by_key(Trial::full_name);
        registry.story_sets.sort_by_key(|entry| (entry.name)());
        registry
    }

    /// Adds a component or pattern registration.
    pub fn add_registration(&mut self, registration: Registration) {
        self.registrations.push(registration);
    }

    /// Adds a story.
    pub fn add_story(&mut self, story: StoryRegistration) {
        self.stories.push(story);
    }

    /// Adds a trial.
    pub fn add_trial(&mut self, trial: Trial) {
        self.trials.push(trial);
    }

    /// Adds a command set used by stories.
    pub fn add_story_set(&mut self, set: SetEntry) {
        self.story_sets.push(set);
    }

    /// The selected components and patterns.
    pub fn registrations(&self) -> &[Registration] {
        &self.registrations
    }

    /// The selected stories.
    pub fn stories(&self) -> &[StoryRegistration] {
        &self.stories
    }

    /// The selected trials.
    pub fn trials(&self) -> &[Trial] {
        &self.trials
    }

    /// Installs the selected story command sets before a story context is created.
    /// Returns false if this process already selected a different set.
    pub fn install_story_sets(&self) -> bool {
        crate::runtime::commands::configure_story_sets(&self.story_sets)
    }

    /// The selected story command sets.
    pub fn story_sets(&self) -> &[SetEntry] {
        &self.story_sets
    }
}
