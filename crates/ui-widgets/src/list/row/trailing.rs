//! A row's trailing items: texts, pressable parts and hosted components.

use super::ListRow;
use crate::contract::{MeasureCx, PaintUi, Show};
use crate::primitives::{IconSpec, Span, TextRun};
use crate::runtime::env::Label;
use craft_behavior::Proposal;
use craft_theme::TypeRole;
use egui::{Rect, Vec2};

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
}

impl TrailingFace {
    /// A disclosure face of `open`.
    pub const fn disclosure(open: bool) -> Self {
        TrailingFace::Disclosure { open }
    }
}

/// A component a row hosts at its trailing end, erased: measured for its slot, then shown once.
pub(in crate::list) trait HostedItem {
    /// The component's size at its ideal and the least width it narrows to, as `PaintUi::child` will show it under the
    /// key of trailing item `at`.
    fn measure(&mut self, cx: &MeasureCx<'_, ListRow<'_>>, at: usize) -> HostedSize;

    /// Shows the component at `rect` and hands its output on.
    fn show(self: Box<Self>, ui: &mut PaintUi<'_>, at: usize, rect: Rect);
}

/// A hosted component and what takes its output; `shown` false keeps its slot empty.
pub(super) struct Hosting<C, F> {
    pub component: C,
    pub output: F,
    pub shown: bool,
}

impl<C: Show, F: FnOnce(C::Output)> HostedItem for Hosting<C, F> {
    fn measure(&mut self, cx: &MeasureCx<'_, ListRow<'_>>, at: usize) -> HostedSize {
        let ideal = cx.child(("field", at), &mut self.component, Proposal::Ideal);
        let least = cx.child(("field", at), &mut self.component, Proposal::Min).x;
        HostedSize { ideal, least }
    }

    fn show(self: Box<Self>, ui: &mut PaintUi<'_>, at: usize, rect: Rect) {
        if self.shown {
            (self.output)(ui.child(("field", at), rect, self.component));
        }
    }
}

/// A hosted component's size where the row has room, and the least width it narrows to where it has not.
#[derive(Clone, Copy)]
pub(in crate::list) struct HostedSize {
    pub ideal: Vec2,
    pub least: f32,
}

/// A hosted item: the component until it is shown, and its size once measured in this pass.
pub(in crate::list) struct Hosted<'a> {
    pub item: Option<Box<dyn HostedItem + 'a>>,
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
