//! A row's trailing items: texts, pressable parts and hosted components.

use super::ListRow;
use crate::contract::Hosted as HostedLeaf;
use crate::primitives::{IconSpec, Span, TextRun};
use crate::runtime::env::Label;
use craft_theme::TypeRole;
use egui::Vec2;

/// The key of a pressable trailing item, chosen by the caller: a row's parts are told apart by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct TrailingPart(pub(super) u16);

impl TrailingPart {
    /// The row's own menu button (`ListRow::row_menu`).
    pub const MENU: TrailingPart = TrailingPart(u16::MAX);

    /// The part numbered `n`.
    pub const fn new(n: u16) -> Self {
        TrailingPart(n)
    }
}

/// How a trailing part looks.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TrailingFace {
    /// An icon button face.
    Icon(IconSpec<'static>),
    /// The disclosure face of an open or closed section.
    #[non_exhaustive]
    Disclosure {
        /// Whether the section is open.
        open: bool,
    },
    /// A short text in the caption role's semibold face, as wide as it measures (a layer's `fx`).
    Strong(Label),
}

impl TrailingFace {
    /// A disclosure face of `open`.
    pub const fn disclosure(open: bool) -> Self {
        TrailingFace::Disclosure { open }
    }
}

/// The key a hosted trailing component of item `at` is measured and shown under.
pub(in crate::list) const fn field_key(at: usize) -> (&'static str, usize) {
    ("field", at)
}

/// The key the hosted component `at` under a row's label (`ListRow::below`) is measured and shown under; its own name
/// keeps it apart from every trailing item's key.
pub(in crate::list) const fn below_key(at: usize) -> (&'static str, usize) {
    ("below", at)
}

/// The key the hosted component `at` in a column's cell (`ListRow::cell_field`) is measured and shown under; its own
/// name keeps it apart from the trailing and `below` keys.
pub(in crate::list) const fn cell_key(at: usize) -> (&'static str, usize) {
    ("cell", at)
}

/// A hosted component's size where the row has room, and the least width it narrows to where it has not.
#[derive(Clone, Copy)]
pub(in crate::list) struct HostedSize {
    pub ideal: Vec2,
    pub least: f32,
}

/// A hosted item: the component until it is shown, and its size once measured in this pass.
pub(in crate::list) struct Hosted<'a> {
    pub item: Option<Box<dyn for<'r> HostedLeaf<ListRow<'r>> + 'a>>,
    pub size: Option<HostedSize>,
}

/// What a trailing item is.
pub(in crate::list) enum TrailingItem<'a> {
    /// A caption-role text as wide as it measures, semibold when `strong`; a short mark carries the `name` it stands
    /// for, its tooltip.
    Text { text: Label, name: Option<Label>, strong: bool },
    /// An icon of the row's icon size, named for the screen reader and the tooltip.
    Icon { icon: IconSpec<'static>, name: Label },
    /// A pressable face of the row's icon size; a `revealed` one shows only on a hovered or selected row, while it has
    /// focus or while the menu it opens is open.
    Part { part: TrailingPart, face: TrailingFace, name: Label, enabled: bool, revealed: bool },
    /// A component of its own, in a slot as wide as it measures; its stop joins the row's group and it takes its own
    /// keys.
    Hosted(Hosted<'a>),
}

/// A trailing item and whether it drops out for room.
pub(in crate::list) struct Trailing<'a> {
    pub item: TrailingItem<'a>,
    pub optional: bool,
}

/// The run of a trailing text.
pub(super) fn trailing_run(text: &Label, strong: bool) -> TextRun<'_> {
    let span = Span::new(text.as_str()).muted();
    TextRun::spans(vec![if strong { span.strong() } else { span }]).role(TypeRole::Caption)
}
