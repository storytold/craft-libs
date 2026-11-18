//! The ribbon: a tab strip over a scrolling row of command groups.

mod gallery;
mod group;
mod plan;

use std::fmt::Debug;
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::Arc;

use craft_keymap::CommandRef;
use craft_ui::contract::Bind;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::inspect;
use craft_ui::tabs::{StripEnd, TabRole};
use egui::{Id, Pos2, Rect, Ui};

pub use craft_ui::buttons::GroupLevel;
pub use gallery::{Gallery, GalleryFace, GalleryItem, GalleryMore, GalleryOutput};
pub use group::RibbonGroup;
pub use plan::{GroupWidths, plan};

/// The ribbon: a [`TabKind::Ribbon`](craft_ui::tabs::TabKind) strip over a scrolling group row,
/// with minimise and context requests. The strip and the row show inside one inspected region
/// (`inspect::record`), whose record ([`RibbonOutput::controls`]) gives each control's rect and
/// command. On a tab change the group row slides in from the side on which the new tab lies
/// relative to the old one (mirrored in right-to-left), through one `Tween`; with reduce motion the
/// new content shows at rest.
#[must_use = "show the ribbon with `show`"]
pub struct Ribbon<'a, T: Clone + Eq + Hash + Send + Sync + Debug + 'static> {
    stub: PhantomData<(&'a (), T)>,
}

impl<'a, T: Clone + Eq + Hash + Send + Sync + Debug + 'static> Ribbon<'a, T> {
    /// A ribbon named `label` over `tabs`, showing the tab `shown` binds.
    pub fn new(label: impl Into<Label>, tabs: &'a [RibbonTab<T>], shown: Bind<'a, T>) -> Self {
        let _ = (label.into(), tabs, shown);
        todo!("rb.R11")
    }

    /// Items at the end of the tab strip.
    pub fn end(self, items: &'a [StripEnd<'a>]) -> Self {
        let _ = (self, items);
        todo!("rb.R11")
    }

    /// Whether the group row is minimised, bound to the app's state.
    pub fn minimised(self, minimised: Bind<'a, bool>) -> Self {
        let _ = (self, minimised);
        todo!("rb.R11")
    }

    /// Shows the strip and the group row of the shown tab; `body` fills the row for that tab.
    pub fn show(self, ui: &mut Ui, body: impl FnMut(&mut RibbonRow<'_>, &T)) -> RibbonOutput<T> {
        let _ = (self, ui, body);
        todo!("rb.R11")
    }
}

/// One tab of the ribbon.
pub struct RibbonTab<T> {
    stub: PhantomData<(T, TabRole)>,
}

impl<T> RibbonTab<T> {
    /// A plain tab named `label` that `key` identifies.
    pub fn new(key: T, label: impl Into<Label>) -> Self {
        let _ = (key, label.into());
        todo!("rb.R11")
    }

    /// Makes the tab a contextual one: it shows only while its context is on.
    pub fn contextual(self) -> Self {
        let _ = self;
        todo!("rb.R11")
    }
}

/// The row of groups the ribbon's body fills for the shown tab.
pub struct RibbonRow<'a> {
    stub: PhantomData<&'a mut Ui>,
}

/// What the ribbon reports for a pass.
pub struct RibbonOutput<T> {
    /// The tab shown after this pass.
    pub shown: Option<T>,
    /// The tab the user picked this pass.
    pub picked: Option<T>,
    /// A context menu was asked for.
    pub menu: Option<RibbonMenuRequest>,
    /// The inspected record of the strip and the row: each control's rect and command.
    pub controls: Option<Arc<inspect::Recorded>>,
    /// The tab row's rect this pass; `None` while no strip shows.
    pub tabs: Option<Rect>,
}

/// A request for a context menu on a ribbon control or tab.
pub struct RibbonMenuRequest {
    /// The command of the control the request is for, when it has one.
    pub command: Option<CommandRef>,
    /// Where the menu opens.
    pub at: Pos2,
    /// The element the menu returns focus to.
    pub opener: Id,
}
