//! `StateSet`: what a style resolves against.

use craft_theme::{Flags, Mix, Resolve, State, StateMix, Stateful};

/// The state of a component or one of its parts at step 12: the interaction state, the value flags and the
/// animated blend. `StateSet.flags` is the only source of AccessKit states.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct StateSet {
    /// Disabled, pressed, hover or rest; `Flags::OPEN` counts as pressed.
    pub state: State,
    /// The merged value flags.
    pub flags: Flags,
    /// The animated hover, press and on values.
    pub t: StateMix,
}

impl StateSet {
    /// A state set.
    pub const fn new(state: State, flags: Flags, t: StateMix) -> Self {
        StateSet { state, flags, t }
    }

    /// The value of `look` in this state.
    pub fn resolve<T: Mix>(&self, look: &Stateful<T>) -> T {
        look.resolve(self.state, self.flags, self.t)
    }
}

impl Default for StateSet {
    fn default() -> Self {
        StateSet::new(State::Rest, Flags::empty(), StateMix::REST)
    }
}
