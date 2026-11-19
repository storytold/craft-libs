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
    selection: std::sync::OnceLock<std::sync::Arc<crate::runtime::commands::StorySelection>>,
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
        let _ = self.selection.take();
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

    /// Selects story command sets for contexts created on the current thread.
    /// Returns false if the selected definitions are invalid.
    pub fn install_story_sets(&self) -> bool {
        crate::runtime::commands::configure_story_sets(self.story_selection())
    }

    pub(crate) fn story_selection(&self) -> std::sync::Arc<crate::runtime::commands::StorySelection> {
        self.selection.get_or_init(|| std::sync::Arc::new(crate::runtime::commands::StorySelection::new(self.story_sets.clone()))).clone()
    }

    /// The selected story command sets.
    pub fn story_sets(&self) -> &[SetEntry] {
        &self.story_sets
    }
}
