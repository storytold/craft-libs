//! What a row reports: RowEvent and RowOutput.

#[cfg(doc)]
use super::ListRow;
use super::thumb::ThumbTarget;
use super::trailing::TrailingPart;
use crate::contract::{ItemOutput, NoValue, OpenChange, Outcome};
use craft_keymap::Mods;
use egui::{Rect, Response};

/// What a row reports besides what the collection reads from its pointer input.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RowEvent {
    /// The link glyph before an extra thumbnail was clicked: `linked` is the state it asks for.
    #[non_exhaustive]
    LinkToggled {
        /// The extra thumbnail the glyph belongs to, counted from 0 in the order added.
        thumb: usize,
        /// The requested state.
        linked: bool,
    },
    /// A click landed on a thumbnail.
    #[non_exhaustive]
    ThumbClicked {
        /// The thumbnail.
        target: ThumbTarget,
        /// The modifiers held at the click; none for a key activation.
        mods: Mods,
        /// The press count of the click (`craft_behavior::press::count_clicks`): 1 for a single click, 2 for a double
        /// click, reported after the single click's own report. An activation by key or AccessKit counts 1.
        clicks: u32,
    },
    /// A drag started on a thumbnail of a row with [`ListRow::thumb_drag`] for it: the thumbnail is dragged, not the
    /// row, and the drag session runs with the payload given.
    ThumbDrag(ThumbTarget),
    /// A trailing part ([`ListRow::trailing_part`]) was activated; the selection is unchanged.
    #[non_exhaustive]
    Trailing {
        /// The part.
        part: TrailingPart,
        /// The modifiers held at the click; none for a key activation.
        mods: Mods,
    },
}

/// What a row returns.
#[derive(Debug)]
#[non_exhaustive]
pub struct RowOutput {
    /// The row's response.
    pub response: Response,
    /// What the row's parts reported this pass.
    pub event: Option<RowEvent>,
    /// The disclosure of a group row was clicked: the state it asks for. The collection reports it as
    /// `CollectionEvent::Open`, as it does the keys.
    pub open: Option<OpenChange<()>>,
    /// Where the label and detail are drawn, the full height of the row.
    pub label: Rect,
}

impl Outcome for RowOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl ItemOutput<RowEvent> for RowOutput {
    fn into_event(self) -> Option<RowEvent> {
        self.event
    }

    fn open_request(&self) -> Option<OpenChange<()>> {
        self.open.clone()
    }

    fn label_rect(&self) -> Option<Rect> {
        Some(self.label)
    }
}
