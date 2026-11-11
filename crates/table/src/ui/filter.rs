//! The filter button in a table's header cell.

use craft_behavior::{IconId, Intent, Metrics, Sense};
use craft_theme::Variant;
use craft_ui::contract::{
    Access, ColumnSort, Common, Component, Cx, Focus as _, MeasureCx, Name, NoValue, Outcome, PaintUi, Pointer as _, Props, Role, Styled as _,
    WordSetCx as _,
};
use craft_ui::primitives::IconSpec;
use craft_ui::style::{ComponentStyle, FocusLook, IconLook, StyleCx, SurfaceKind, SurfaceLook};
use egui::{Rect, Response, Vec2, pos2};

#[cfg(feature = "screen-testing")]
mod stories;

craft_ui::words! {
    /// The filter button's words, English by default.
    #[non_exhaustive]
    pub struct FilterButtonWords {
        /// The accessible name of a column's button while no filter is on; `{column}` is the header's text.
        filter: template "filterButton.filter" = "{column} (Showing All)";
        /// The accessible name while a filter is on.
        filtered: template "filterButton.filtered" = "{column}: Filter applied";
    }
}

/// What the filter button reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FilterButtonEvent {
    /// The app opens its filter menu under the button: a click, Enter, Space or Alt+Down on the focused button. Every
    /// input of one pass is one event.
    Open,
}

/// What the filter button did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct FilterButtonOutput {
    /// The button's response.
    pub response: Response,
    /// What the user did, if anything.
    pub event: Option<FilterButtonEvent>,
}

impl Outcome for FilterButtonOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The style of the filter button: the raised control surface, the glyph at the control's icon side and the inset
/// between the glyphs and the edge.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[style(variants = [Variant::SECONDARY])]
#[non_exhaustive]
pub struct FilterButtonStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The chevron, the funnel and the sort arrow.
    #[style(icon)]
    glyph: IconLook,
    /// The space between the glyphs and the button's edge.
    inset: f32,
}

/// The drop-down button of a header cell: a chevron, a funnel while the column is filtered, and beside it the sort
/// arrow of a sorted column. A click, Enter, Space or Alt+Down on the focused button reports
/// [`FilterButtonEvent::Open`]: the app opens its filter menu under the button. It is one Tab stop named by
/// [`FilterButtonWords`] with the column's header text, and it measures its own size from the control's icon side.
#[derive(Component, Props)]
pub struct FilterButton {
    #[common]
    common: Common,
    column: String,
    /// Whether the column is filtered: the button then shows a funnel.
    #[prop]
    filtered: bool,
    /// The column's sort, if any: the button then shows the sort arrow.
    #[prop]
    sort: Option<ColumnSort>,
    event: Option<FilterButtonEvent>,
}

impl FilterButton {
    /// The filter button of the column whose header reads `column`, for the accessible name.
    pub fn new(column: impl Into<String>) -> Self {
        FilterButton { common: Common::new(), column: column.into(), filtered: false, sort: None, event: None }
    }
}

impl Component for FilterButton {
    const ROLE: Role = Role::BUTTON;
    const PARTS: bool = false;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = FilterButtonStyle;
    type Output = FilterButtonOutput;

    fn style(cx: &StyleCx<'_>, _: FilterButtonStyleKey) -> FilterButtonStyle {
        let parts = cx.variant_look();
        let mut surface = SurfaceLook::new(parts.fill.clone()).radius(parts.radius).kind(SurfaceKind::Raised).kind_down(SurfaceKind::Pressed);
        if let Some(edge) = &parts.edge {
            surface = surface.border(edge.clone(), cx.tokens.stroke.thin);
        }
        FilterButtonStyle {
            surface,
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            glyph: IconLook::new(cx.control().icon, parts.text),
            inset: cx.tokens.space.gap.tight,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let side = cx.style.glyph.side;
        let glyphs = if self.sort.is_some() { 2.0 } else { 1.0 };
        Vec2::new(side * glyphs + 2.0 * cx.style.inset, side + 2.0 * cx.style.inset)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        cx.join_roving();
        let opened = cx.intents().take(Intent::Open);
        if opened || cx.activation().is_some() {
            self.event = Some(FilterButtonEvent::Open);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<FilterButtonWords>();
        let word = if self.filtered { &words.filtered } else { &words.filter };
        access.name(Name::Text(word.fill(&[("column", self.column.as_str())])));
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (side, rtl) = (cx.style.glyph.side, cx.flow.is_rtl());
        let color = cx.resolve(&cx.style.glyph.color);
        let draw = ui.draw();
        let inner = cx.rect.shrink(cx.style.inset);
        // The chevron or funnel sits at the reading end; the sort arrow beside it, at the reading start.
        let (base_x, arrow_x) = if rtl { (inner.left(), inner.right() - side) } else { (inner.right() - side, inner.left()) };
        let slot = |x: f32| Rect::from_min_size(pos2(x, inner.center().y - side / 2.0), Vec2::splat(side));
        let base = if self.filtered { "funnel" } else { "chevron_down" };
        draw.icon(slot(base_x), &IconSpec::svg(IconId::new(base)).color(color));
        if let Some(sort) = &self.sort {
            let arrow = if sort.descending { "sort_descending" } else { "sort_ascending" };
            draw.icon(slot(arrow_x), &IconSpec::svg(IconId::new(arrow)).color(color));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> FilterButtonOutput {
        FilterButtonOutput { response: cx.response.clone(), event: self.event }
    }
}
