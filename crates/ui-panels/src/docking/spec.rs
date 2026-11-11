//! `PanelSpec`: what the dock asks of a panel before it shows its body.

use crate::contract::SizeLimits;
use crate::primitives::IconSpec;
use crate::runtime::env::Label;
use craft_host::target::WindowKind;
use egui::Vec2;

/// Body heights a panel asks for in a dock column while the user has not resized its group. Like [`SizeLimits`]
/// they exclude the tab strip, and a group takes the largest of each among its panels.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct DockHeights {
    /// The height the group gets when the column has room.
    pub default: f32,
    /// The height it gives way down to, so the group that fills the column keeps its `fill`.
    pub compact: f32,
    /// The height it asks for when it fills the column: groups at their default shrink towards `compact` before it
    /// gets less.
    pub fill: f32,
}

impl DockHeights {
    /// A `default` height that does not give way (`compact` is `default`) and asks for no more than its minimum when
    /// it fills the column (`fill` is 0).
    pub const fn new(default: f32) -> Self {
        DockHeights { default, compact: default, fill: 0.0 }
    }

    /// The height the group gives way down to.
    #[must_use]
    pub const fn compact(mut self, compact: f32) -> Self {
        self.compact = compact;
        self
    }

    /// The height the group asks for when it fills the column.
    #[must_use]
    pub const fn fill(mut self, fill: f32) -> Self {
        self.fill = fill;
        self
    }

    /// Whether every height is finite, non-negative and at most [`SizeLimits::BOUND`].
    pub fn valid(self) -> bool {
        [self.default, self.compact, self.fill].iter().all(|height| height.is_finite() && (0.0..=SizeLimits::BOUND).contains(height))
    }
}

/// What the dock reads of a panel: its name and icon, its size policy and how its floating window behaves. The
/// permissions of a panel (movable, floatable, closable, takes tabs, takes splits) belong to the dock model, not
/// to this spec.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PanelSpec {
    /// The panel's name: its tab text, its menu rows' target and the accessible name of its handles.
    pub label: Label,
    /// The icon of the panel's iconic form and flyout cell.
    pub icon: Option<IconSpec<'static>>,
    /// Body size limits.
    pub limits: SizeLimits,
    /// How the panel's group sizes itself in a dock column until the user resizes it. `None` shares the column's
    /// free height with the other groups that have none when no group asks for a fill, and takes the group's
    /// minimum when one does.
    pub heights: Option<DockHeights>,
    /// The size the panel floats at: its width and the most height it takes. `None`: `FloatSpace`'s default size.
    pub float_size: Option<Vec2>,
    /// Whether the panel's floating window may be an OS window of its own. A panel that disallows it keeps its
    /// window in the main window.
    pub allow_native: bool,
    /// What an OS window holding the panel is for. A window holding any `WindowKind::Document` panel is a document
    /// window; the rest are tool windows.
    pub window_kind: WindowKind,
    /// The panel is a document canvas: its group shows it without the tab and side drops of a panel group, and the
    /// host draws it with `DockPanels::group_view`.
    pub canvas: bool,
}

impl PanelSpec {
    /// A panel named `label` with no icon, no limits, no height policy and a tool window of its own when floated.
    pub fn new(label: impl Into<Label>) -> Self {
        PanelSpec {
            label: label.into(),
            icon: None,
            limits: SizeLimits::default(),
            heights: None,
            float_size: None,
            allow_native: true,
            window_kind: WindowKind::Tool,
            canvas: false,
        }
    }

    /// The panel's icon.
    #[must_use]
    pub fn icon(mut self, icon: IconSpec<'static>) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Body size limits.
    #[must_use]
    pub fn limits(mut self, limits: SizeLimits) -> Self {
        self.limits = limits;
        self
    }

    /// The heights the panel asks for in a dock column.
    #[must_use]
    pub fn heights(mut self, heights: DockHeights) -> Self {
        self.heights = Some(heights);
        self
    }

    /// The size the panel floats at.
    #[must_use]
    pub fn float_size(mut self, size: Vec2) -> Self {
        self.float_size = Some(size);
        self
    }

    /// Whether the floating window may be an OS window of its own.
    #[must_use]
    pub fn allow_native(mut self, allow: bool) -> Self {
        self.allow_native = allow;
        self
    }

    /// What an OS window holding the panel is for.
    #[must_use]
    pub fn window_kind(mut self, kind: WindowKind) -> Self {
        self.window_kind = kind;
        self
    }

    /// The panel is a document canvas.
    #[must_use]
    pub fn canvas(mut self) -> Self {
        self.canvas = true;
        self
    }
}
