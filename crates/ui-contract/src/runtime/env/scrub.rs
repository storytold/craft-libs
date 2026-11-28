//! Which drags scrub a value.

use super::EnvValue;

/// Which drags scrub a value, an `AppValues` env value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, crate::contract::MemoProps)]
#[non_exhaustive]
pub enum ScrubDrags {
    /// The body of a field not in edit and a scrubbing side label scrub.
    #[default]
    FieldAndLabel,
    /// The side label only scrubs; a drag on the body selects text.
    LabelOnly,
    /// Neither scrubs. A component's own scrub parts (a popup slider's chevron) are not affected.
    Off,
}

impl EnvValue for ScrubDrags {
    fn root() -> Self {
        ScrubDrags::default()
    }
}
