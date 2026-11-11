//! What one paint pass records for later steps: text elision, the first baseline and the faces painted.

use crate::runtime::memory::sub;
use craft_behavior::{EditEvent, PartRef};
use egui::Id;

/// What `Draw` calls record. Step 16 reads the elided text and step 17 the baseline.
#[derive(Debug, Default)]
#[doc(hidden)]
pub struct PaintRecord {
    /// The full text of the first elided run, shown as the tooltip and accessible name when no tooltip is set.
    pub elided: Option<String>,
    /// The screen y of the first text's baseline; step 17 turns it into a distance from the component's top, which
    /// `LabelColumn` aligns a row label to next frame.
    pub baseline: Option<f32>,
    /// The faces painted this frame.
    pub faces: Vec<FaceRecord>,
    /// Child edits adopted with `cx.adopt`, read at step 15.
    pub adopted: Vec<Adopted>,
}

/// An adopted child event with its value type erased, queued in the paint record and read back at step 15.
#[doc(hidden)]
pub struct Adopted(Box<dyn std::any::Any>);

impl Adopted {
    pub(crate) fn new<T: 'static>(event: EditEvent<T>) -> Self {
        Adopted(Box::new(event))
    }

    /// The event, when it carries values of type `T`.
    pub(crate) fn take<T: 'static>(self) -> Option<EditEvent<T>> {
        self.0.downcast::<EditEvent<T>>().ok().map(|event| *event)
    }
}

impl std::fmt::Debug for Adopted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Adopted")
    }
}

/// A face painted this frame with the facts its component's `access` declared; step 16 fills the part's
/// sub-node from it.
#[derive(Debug, Clone, PartialEq)]
#[doc(hidden)]
pub struct FaceRecord {
    /// The part the face looks for.
    pub part: PartRef,
    /// The instance's resolved label, the full text when elided.
    pub name: String,
    /// The face's value flags.
    pub flags: crate::style::Flags,
    /// The face's role.
    pub role: crate::runtime::access::Role,
    /// The face's numeric value, from its `access`.
    pub numeric: Option<crate::runtime::access::Numeric>,
    /// The face's explicit expanded state, `false` when closed.
    pub expanded: Option<bool>,
    /// Whether the face is enabled: a disabled face makes its part's sub-node disabled.
    pub enabled: bool,
    /// The screen y of the face's first text baseline, which the composite's record takes when it has none yet.
    pub baseline: Option<f32>,
}

/// The base id of the face painted for `part` of the composite `id`: owned by the composite, so the face's hover
/// and press animations key by it.
pub(crate) fn face_id(composite: Id, part: PartRef) -> Id {
    sub(composite, ("face", part))
}
