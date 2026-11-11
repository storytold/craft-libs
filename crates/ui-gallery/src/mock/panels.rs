//! The panels of the dock: the canvas group the host draws, the Navigator and the layer table.

use super::model::{Document, Layer, Panel, Tool};
use super::tools::{MarqueeTool, MoveTool, Tools};
use super::words::MockWords;
use craft_behavior::dock::Permissions;
use craft_behavior::kurbo::Rect as DocRect;
use craft_behavior::{ColumnKey, ColumnWidth, PixelSize, RulerUnits, Ticks, keyed};
use craft_ui::canvas::{
    Canvas, CanvasScrollbars, CanvasSpace, NoTool, PageFrame, Pasteboard, PixelRect, RasterDisplay, RasterSource, RulerText, Rulers, ToolHost,
};
use craft_ui::canvas_controls::{Guides, Marquee};
use craft_ui::contract::SizeLimits;
use craft_ui::contract::{
    Bind, BodyCx, CollectionEvent, ColumnKind, ColumnSpec, Remember as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, SweepApply,
    SweepStep, ToggleColumn, WordSetCx as _,
};
use craft_ui::docking::{DockPanels, GroupAt, PanelSpec};
use craft_ui::list::{ListRow, ToggleCell};
use craft_ui::patterns::canvas::NavigatorPanel;
use craft_ui::primitives::textures::Revision;
use craft_ui::runtime::env::UnitKind;
use craft_ui::shell::CanvasPanel;
use craft_ui::table::Table;
use craft_ui::tabs::{DocumentTab, DocumentTabs, DocumentTabsEvent};
use egui::{Color32, ColorImage, Ui};
use std::borrow::Cow;

/// The units the rulers' menu offers.
const UNITS: [UnitKind; 5] = [UnitKind::Pixels, UnitKind::Points, UnitKind::Inches, UnitKind::Centimetres, UnitKind::Millimetres];

/// The resolution of the mock document, in pixels per inch.
const PIXELS_PER_INCH: f64 = 72.0;

const NAME: ColumnKey = ColumnKey::new("name");
const VISIBLE: ColumnKey = ColumnKey::new("visible");

/// The dock's panels over the open document.
pub(super) struct Panels<'a> {
    pub(super) document: &'a mut Document,
    pub(super) tool: Tool,
    pub(super) tools: &'a mut Tools,
}

impl Panels<'_> {
    /// The document tab's title: the name, the zoom of the device and the current layer.
    fn tab_title(&self, ui: &Ui, cx: &BodyCx<'_>) -> String {
        let document = &self.document;
        let zoom = CanvasSpace::new(document.view, ui.pixels_per_point()).zoom_device() * 100.0;
        let layer = document.current_layer().map_or("", |layer| layer.name.as_str());
        cx.words::<MockWords>().tab_title.fill(&[("name", &document.title), ("zoom", &cx.format().integer(zoom.round() as i64)), ("layer", layer)])
    }
}

fn name(panel: Panel) -> &'static str {
    match panel {
        Panel::Canvas => "Canvas",
        Panel::Navigator => "Navigator",
        Panel::Layers => "Layers",
    }
}

impl DockPanels<Panel> for Panels<'_> {
    fn spec(&self, panel: &Panel) -> PanelSpec {
        let spec = PanelSpec::new(name(*panel)).limits(SizeLimits::default());
        if *panel == Panel::Canvas { spec.canvas() } else { spec }
    }

    fn permissions(&self, panel: &Panel) -> Permissions {
        if panel.protected() { Permissions::PROTECTED } else { Permissions::ALL }
    }

    fn body(&mut self, panel: &Panel, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        match panel {
            Panel::Navigator => {
                let size = self.document.size;
                let _ = NavigatorPanel::new("Navigator", Bind::bound(&mut self.document.view), size).show(ui);
            }
            Panel::Layers => layers(ui, cx, self.document),
            Panel::Canvas => {}
        }
    }

    fn group_view(&mut self, panels: &[Panel], _: usize, ui: &mut Ui, at: GroupAt, cx: &mut BodyCx<'_>) -> Option<f32> {
        // The canvas fills the rect it is shown in; protected, it never sits in a flyout, which is what measures.
        let GroupAt::Show(rect) = at else { return None };
        if !panels.contains(&Panel::Canvas) {
            return None;
        }
        let title = self.tab_title(ui, cx);
        let modified = self.document.dirty;
        let mut events = Vec::new();
        let document = &mut *self.document;
        let (tool, tools) = (self.tool, &mut *self.tools);
        CanvasPanel::new()
            .tabs(|ui, _| {
                let tabs = [DocumentTab::new(0_usize, title.clone()).modified(modified)];
                events = DocumentTabs::new("Documents", &tabs).selected(0).show(ui).events;
            })
            .show(ui, |ui, _| canvas(ui, document, tool, tools));
        for event in events {
            // The mock holds one document, so selecting its tab changes nothing; closing it discards the edits and
            // opens the document again as it was.
            if matches!(event, DocumentTabsEvent::CloseRequested(_)) {
                *self.document = Document::default();
            }
        }
        Some(rect.height())
    }
}

/// The rulers' text in `kind`.
fn ruler_text(kind: UnitKind) -> RulerText {
    let per_doc = kind.per_inch(PIXELS_PER_INCH).map_or(1.0, |per_inch| per_inch / PIXELS_PER_INCH);
    RulerText::new(RulerUnits::new(per_doc, Ticks::decimal(kind == UnitKind::Pixels)), kind)
}

/// A layer as the raster display reads it: its colour over its bounds, transparent elsewhere.
struct LayerFill<'a> {
    layer: &'a Layer,
    size: PixelSize,
}

impl RasterSource for LayerFill<'_> {
    fn size(&self) -> PixelSize {
        self.size
    }

    fn revision(&self) -> Revision {
        let b = self.layer.bounds;
        Revision::of(&(self.layer.id, self.layer.color.to_array(), [b.x0, b.y0, b.x1, b.y1]))
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        None
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        let [width, height] = out.size;
        let bounds = self.layer.bounds;
        for row in 0..height {
            for column in 0..width {
                let x = region.x0 + (column as u32 * factor) as i32;
                let y = region.y0 + (row as u32 * factor) as i32;
                let inside = (bounds.x0..bounds.x1).contains(&x) && (bounds.y0..bounds.y1).contains(&y);
                out.pixels[row * width + column] = if inside { self.layer.color } else { Color32::TRANSPARENT };
            }
        }
    }
}

/// What the canvas body found out in a pass.
struct BodyResult {
    /// The view the scroll bars set.
    scrolled: Option<craft_behavior::CanvasView>,
    /// The pointer is over a scroll bar, so the tool leaves the press alone.
    over_bars: bool,
}

/// The document area: the pasteboard, the visible layers, the selection, the rulers with their guides and the scroll
/// bars on a canvas whose tool host runs the current tool and reports the one view.
fn canvas(ui: &mut Ui, document: &mut Document, tool: Tool, tools: &mut Tools) {
    let units = ruler_text(document.units);
    let page = DocRect::new(0.0, 0.0, document.size.width, document.size.height);
    let pages = [PageFrame::new(page)];
    let pixels = document.pixel_size();
    let (view, show_rulers) = (document.view, document.rulers);
    let editing = matches!(tool, Tool::Move | Tool::Marquee);
    let shown = {
        let Document { layers, guides, guide, marquee, zero, units: unit, .. } = &mut *document;
        Canvas::new("Document").view(view).document(page.size()).show_rulers(show_rulers).rulers(units, units, *zero).show(ui, |ui, body| {
            let _ = Pasteboard::new("Pasteboard", &pages).show_at(ui, body.rect);
            for layer in layers.iter().rev().filter(|layer| layer.visible) {
                let source = LayerFill { layer, size: pixels };
                let _ = RasterDisplay::new(layer.name.clone(), &source).show_at(ui, body.rect);
            }
            if let Some(rect) = *marquee {
                let _ = Marquee::rect("Selection", rect).show_at(ui, body.rect);
            }
            let mut content = body.rect;
            let mut pending = None;
            if show_rulers {
                let rulers = Rulers::new("Rulers", Bind::bound(zero)).units(&UNITS).show_at(ui, body.rect);
                content = rulers.content_rect();
                pending = rulers.guide;
                if let Some(picked) = rulers.unit_picked {
                    *unit = picked;
                }
            }
            let mut controls = Guides::new("Guides", Bind::bound(guides)).interactive(editing);
            if let Some(selected) = *guide {
                controls = controls.selected_guide(selected);
            }
            if let Some(drag) = pending {
                controls = controls.pending(drag);
            }
            if let Some(select) = controls.show_at(ui, body.rect).select {
                *guide = select;
            }
            let bars = CanvasScrollbars::new("Scroll bars", Bind::given(view), page).show_at(ui, content);
            BodyResult { scrolled: bars.view.value().copied(), over_bars: bars.over }
        })
    };
    let result = shown.inner;
    let host = ToolHost::new().momentary(&mut tools.momentary).enabled(!result.as_ref().is_some_and(|result| result.over_bars));
    let reported = match tool {
        Tool::Hand => host.run(ui, &shown.output, &mut tools.hand),
        Tool::Zoom => host.run(ui, &shown.output, &mut tools.zoom),
        Tool::Move => host.run(ui, &shown.output, &mut MoveTool),
        Tool::Marquee => host.run(ui, &shown.output, &mut MarqueeTool::new(document)),
        Tool::Brush | Tool::Pencil | Tool::Eraser | Tool::Stamp => host.run(ui, &shown.output, &mut NoTool),
    };
    if let Some(next) = result.and_then(|result| result.scrolled).or(reported.view) {
        document.view = next;
    }
}

/// The layer table: an eye column and a name column over the layers, top first, with several layers selectable and
/// the lead being the current layer.
fn layers(ui: &mut Ui, cx: &mut BodyCx<'_>, document: &mut Document) {
    let columns = vec![
        ColumnSpec::new(VISIBLE, "Visible", ColumnWidth::Controls(1.5)).kind(ColumnKind::Toggle(ToggleColumn::sweepable())),
        ColumnSpec::new(NAME, "Layer", ColumnWidth::fill(1.0)),
    ];
    let events = {
        let Document { layers, selected, .. } = &mut *document;
        let source = keyed(layers, |layer| layer.id)
            .with_text(|layer| Cow::Borrowed(layer.name.as_str()))
            .with_toggled(|layer, column| *column == VISIBLE && layer.visible);
        Table::new("Layers", columns)
            .select(SelectMode::Multi)
            .selection(Bind::bound(selected))
            .show_items(ui, &source, &mut |id, icx| {
                let layer = layers.iter().find(|layer| layer.id == *id);
                let name = layer.map(|layer| layer.name.clone()).unwrap_or_default();
                ListRow::new(name).cells(icx.cells()).toggle(ToggleCell::eye(VISIBLE, layer.is_some_and(|layer| layer.visible)))
            })
            .events
    };
    for event in events {
        let CollectionEvent::Toggle(edit) = event else { continue };
        let step = cx.keep::<SweepApply>().feed(&edit);
        if let Some(SweepStep::Flip(toggle) | SweepStep::Restore(toggle)) = step {
            document.set_visible(toggle.key, toggle.on);
        }
    }
    document.keep_current();
}
