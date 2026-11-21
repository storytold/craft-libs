//! Where the options bar sits (docked at the top or the bottom, or floating) and the grip that moves it.

use crate::contract::{Common, Component, Cx, MeasureCx, Role};
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::Metrics;
use egui::{Response, Vec2};

/// Where the options bar sits: docked at the top or the bottom, or floating with its top-left corner `at`, relative to
/// the body. A non-finite `at` counts as unset. Floating keeps the edge; docking keeps `at`.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct OptionsBarPlace {
    /// Docked at the bottom instead of the top.
    #[serde(default)]
    pub bottom: bool,
    /// Floating over the body instead of docked.
    #[serde(default)]
    pub floating: bool,
    /// The floating bar's top-left corner relative to the body, once moved.
    #[serde(default)]
    pub at: Option<[f32; 2]>,
}

impl OptionsBarPlace {
    /// A place docked at the `bottom` or the top, or `floating` with the top-left corner `at`.
    pub const fn new(bottom: bool, floating: bool, at: Option<[f32; 2]>) -> Self {
        Self { bottom, floating, at }
    }
}

/// What a drag of a [`BarGrip`] carries: where the pointer holds the bar and how big the bar is.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct OptionsBarDrag {
    /// The pointer's offset inside the bar where the drag began.
    pub grab: Vec2,
    /// The bar's size.
    pub size: Vec2,
}

/// The style of a bar grip.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct BarGripStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The grip of an options bar: a drag on it starts a drag session that moves the bar and docks or floats it on the
/// release.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct BarGrip {
    #[common]
    common: Common,
}

impl BarGrip {
    /// A grip.
    pub fn new() -> Self {
        BarGrip { common: Common::new() }
    }
}

impl Default for BarGrip {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for BarGrip {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const INPUT: craft_behavior::Input = craft_behavior::Input::Pointer;
    const OVERLAYS: bool = false;
    type Style = BarGripStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: BarGripStyleKey) -> BarGripStyle {
        BarGripStyle { metrics: Metrics::new() }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let _ = cx;
        todo!("ch.K11")
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        let _ = cx;
        todo!("ch.K11")
    }
}
