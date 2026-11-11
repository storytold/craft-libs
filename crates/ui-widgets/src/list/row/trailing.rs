//! A row's trailing items: texts, pressable parts and hosted components.

use crate::contract::PaintUi;
use crate::primitives::{IconSpec, Span, TextRun};
use crate::runtime::env::Label;
use craft_theme::TypeRole;
use egui::Rect;

/// The key of a pressable trailing item, chosen by the caller: a row's parts are told apart by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct TrailingPart(pub(super) u16);

impl TrailingPart {
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
}

impl TrailingFace {
    /// A disclosure face of `open`.
    pub const fn disclosure(open: bool) -> Self {
        TrailingFace::Disclosure { open }
    }
}

/// Shows a hosted component at the rect the row gives it; the index is the item's place among the trailing items.
pub(super) type HostedShow<'a> = Box<dyn FnOnce(&mut PaintUi<'_>, usize, Rect) + 'a>;

/// What a trailing item is.
pub(in crate::list) enum TrailingItem<'a> {
    /// A caption-role text as wide as it measures.
    Text(Label),
    /// An icon of the row's icon size, named for the screen reader and the tooltip.
    Icon { icon: IconSpec<'static>, name: Label },
    /// A pressable face of the row's icon size.
    Part { part: TrailingPart, face: TrailingFace, name: Label, enabled: bool },
    /// A component of its own, shown in a short field's width; its stop joins the row's group and it takes its own keys.
    Hosted(Option<HostedShow<'a>>),
}

/// A trailing item and whether it drops out for room; only a text can be optional.
pub(in crate::list) struct Trailing<'a> {
    pub item: TrailingItem<'a>,
    pub optional: bool,
}

/// The run of a trailing text.
pub(super) fn trailing_run(text: &Label) -> TextRun<'_> {
    TextRun::spans(vec![Span::new(text.as_str()).muted()]).role(TypeRole::Caption)
}
