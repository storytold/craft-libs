//! `DockRoot`: the room a docking pattern shows its parts in. It resolves the docking sizes for its body and takes the
//! drops released on the room.
//!
//! `DockArea` and `PaneArea` both run their body inside one: the body reads the sizes with
//! `BodyCx::style::<DockRoot<..>>()`, and the drop taker, which runs in the root's `update` (only a component's
//! context registers a drop target), returns what a drop made for the output.

use super::DockSizes;
use super::drop::{self, Arrangement};
use crate::contract::{Common, Component, Container, ContainerProps, Cx, NoValue, Outcome, Pad, Role, Sizing};
use crate::runtime::env::AppValues;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{Metrics, Sense};
use egui::Response;

#[cfg(feature = "testing")]
mod stories;

/// The style of a dock root: the docking sizes of its drop bands, columns, groups and floating windows. Colours,
/// surfaces and shapes belong to the parts the pattern assembles.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct DockRootStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The docking sizes: bands, zones, least extents and the float window.
    sizes: DockSizes,
}

impl DockRootStyle {
    /// The docking sizes: bands, zones, least extents and the float window.
    pub fn sizes(&self) -> &DockSizes {
        &self.sizes
    }
}

/// What a dock root reports.
#[non_exhaustive]
pub struct DockRootOutput<T> {
    /// The root's response.
    pub response: Response,
    /// What the drop released on the root this pass made, as the drop taker returned it.
    pub dropped: Option<T>,
}

impl<T> Outcome for DockRootOutput<T> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Takes the drop released on a dock root, in the root's `update`, and returns what it made.
type TakeDrop<'a, T> = Box<dyn for<'b, 'c> FnMut(&'b mut Cx<'c, DockRoot<'a, T>>) -> Option<T> + 'a>;

/// The room of a docking pattern: it fills the room offered, resolves the docking sizes its body reads, and takes the
/// drops released on it.
///
/// The body reads the sizes with [`BodyCx::style`](crate::contract::BodyCx::style).
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container, probe(DockRoot<'static, ()>))]
pub struct DockRoot<'a, T: 'static> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    take_drop: TakeDrop<'a, T>,
    dropped: Option<T>,
    shows: Option<Arrangement>,
}

impl<'a, T: 'static> DockRoot<'a, T> {
    /// A root that fills the room offered and takes the drops released on it with `take_drop`, which registers the
    /// pattern's drop targets on the root's context and returns what a drop made.
    pub fn new(take_drop: impl for<'b, 'c> FnMut(&'b mut Cx<'c, DockRoot<'a, T>>) -> Option<T> + 'a) -> Self {
        DockRoot {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::Fill).pad(Pad::None),
            take_drop: Box::new(take_drop),
            dropped: None,
            shows: None,
        }
    }

    /// The arrangement the pattern shows, which the drags started in the body record.
    #[doc(hidden)]
    pub fn shows(mut self, arrangement: Arrangement) -> Self {
        self.shows = Some(arrangement);
        self
    }
}

impl<T: 'static> Component for DockRoot<'_, T> {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = DockRootStyle;
    type Output = DockRootOutput<T>;

    fn style(cx: &StyleCx<'_>, _: DockRootStyleKey<T>) -> DockRootStyle {
        DockRootStyle { metrics: Metrics::new(), sizes: DockSizes::style(cx) }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.dropped = (self.take_drop)(cx);
    }

    fn content_env(&self, _: &Cx<'_, Self>, values: &mut AppValues) {
        drop::publish(values, self.shows);
    }

    fn finish(self, cx: Cx<'_, Self>) -> DockRootOutput<T> {
        DockRootOutput { response: cx.response.clone(), dropped: self.dropped }
    }
}

impl<T: 'static> Container for DockRoot<'_, T> {}
