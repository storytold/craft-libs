//! The panels of the dock: the canvas group the host draws, the Navigator and the layer table.

use super::model::{Document, Layer, Panel, Tool};
use super::tools::{MarqueeTool, MoveTool, Tools};
use super::words::MockWords;
use craft_behavior::CanvasView;
use craft_behavior::dock::Permissions;
use craft_behavior::geometry::Directions;
use craft_behavior::kurbo::{Rect as DocRect, Size};
use craft_behavior::{ColumnKey, ColumnWidth, PixelSize, RulerUnits, Ticks, keyed};
use craft_ui::canvas::{
    Canvas, CanvasScrollbars, CanvasSpace, NoTool, PageFrame, Pasteboard, PixelRect, RasterDisplay, RasterSource, RulerText, Rulers, ToolHost,
    tool_pass,
};
use craft_ui::canvas_controls::{Guides, Marquee};
use craft_ui::contract::{
    Bind, BodyCx, CollectionEvent, ColumnKind, ColumnSpec, MemoProps, MemoView, Props as _, Remember as _, SelectMode, Show as _,
    ShowCollection as _, ShowContainer as _, SweepApply, SweepStep, ToggleColumn, WordSetCx as _,
};
use craft_ui::contract::{SizeLimits, Tracked};
use craft_ui::docking::{DockGrid, DockPanels, GroupAt, PanelSpec, PanelSpecs};
use craft_ui::list::{ListRow, ToggleCell};
use craft_ui::patterns::canvas::NavigatorPanel;
use craft_ui::primitives::textures::Revision;
use craft_ui::rails::{LastUsed, ToolRail, ToolSlot};
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
    /// The tool each toolbar slot showed last; the toolbar writes it.
    pub(super) last_used: &'a mut Tracked<LastUsed<Tool>>,
    /// The toolbar's slots.
    pub(super) slots: &'a Tracked<Vec<ToolSlot<Tool>>>,
    /// The tool the toolbar picked in this pass.
    pub(super) picked: Option<Tool>,
}

/// The dock's panel specs: fixed for the mock's four panels.
#[derive(Clone, Copy, Debug, Default, PartialEq, craft_ui::contract::PlainData)]
pub struct MockSpecs;

impl Panels<'_> {
    /// The toolbar's rail in the rect the dock gives it, or its height at a width: as many tool columns as the width
    /// holds. The rail's own padding surrounds the buttons, so no group padding applies.
    fn toolbar(&mut self, ui: &mut Ui, at: GroupAt) -> f32 {
        let current = self.tool;
        let rail = ToolRail::new("Tools", self.slots, &current, self.last_used);
        match at {
            GroupAt::Measure(width) => rail.measure_rail(ui, width, |_, _| {}).y,
            GroupAt::Show(rect) => {
                let output = rail.show_rail(ui, |_, _| {});
                if output.select.is_some() {
                    self.picked = output.select;
                }
                if let Some(next) = output.last_used {
                    **self.last_used = next;
                }
                rect.height()
            }
        }
    }

    /// The document tab's title: the name, the zoom of the device and the current layer.
    fn tab_title(&self, ui: &Ui, cx: &BodyCx<'_>) -> String {
        let document = &self.document;
        let zoom = CanvasSpace::new(*document.view, ui.pixels_per_point()).zoom_device() * 100.0;
        let layer = document.current_layer().map_or("", |layer| layer.name.as_str());
        cx.words::<MockWords>().tab_title.fill(&[("name", &document.title), ("zoom", &cx.format().integer(zoom.round() as i64)), ("layer", layer)])
    }
}

fn name(panel: Panel) -> &'static str {
    match panel {
        Panel::Tools => "Tools",
        Panel::Canvas => "Canvas",
        Panel::Navigator => "Navigator",
        Panel::Layers => "Layers",
    }
}

impl PanelSpecs<Panel> for MockSpecs {
    fn spec(&self, panel: &Panel) -> PanelSpec {
        let spec = PanelSpec::new(name(*panel)).limits(SizeLimits::default());
        match panel {
            Panel::Canvas => spec.canvas(),
            // The toolbar's column is whole tool columns wide.
            Panel::Tools => spec.grid(DockGrid::rail(Tool::slots().len())),
            _ => spec,
        }
    }

    fn permissions(&self, panel: &Panel) -> Permissions {
        match panel {
            // The toolbar never collapses to icons and never shares its column.
            Panel::Tools => Permissions::ALL.collapses(false).shares_column(false),
            _ if panel.protected() => Permissions::PROTECTED,
            _ => Permissions::ALL,
        }
    }
}

impl DockPanels<Panel> for Panels<'_> {
    fn body(&mut self, panel: &Panel, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        match panel {
            Panel::Navigator => {
                let props = NavigatorProps { view: craft_ui::contract::bind_tracked!(self.document.view), size: self.document.size };
                let _ = MemoView::new("navigator", props, navigator).show(ui);
            }
            Panel::Layers => layers(ui, cx, self.document),
            Panel::Canvas | Panel::Tools => {}
        }
    }

    fn group_view(&mut self, panels: &[Panel], _: usize, ui: &mut Ui, at: GroupAt, cx: &mut BodyCx<'_>) -> Option<f32> {
        if panels.contains(&Panel::Tools) {
            return Some(self.toolbar(ui, at));
        }
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

/// The visible layers of a document of `size` pixels, bottom first, at the canvas rect `at`.
pub(super) fn show_layers(ui: &mut Ui, at: egui::Rect, layers: &[Layer], size: PixelSize) {
    for layer in layers.iter().rev().filter(|layer| layer.visible) {
        let source = LayerFill { layer, size };
        let _ = RasterDisplay::new(layer.name.clone(), &source).picture().show_at(ui, at);
    }
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

/// The document area: the pasteboard, the visible layers, the selection, the rulers with their guides and the scroll
/// bars on a canvas whose tool host runs the current tool and reports the one view.
fn canvas(ui: &mut Ui, document: &mut Document, tool: Tool, tools: &mut Tools) {
    let units = ruler_text(document.units);
    let page = DocRect::new(0.0, 0.0, document.size.width, document.size.height);
    let pages = [PageFrame::new(page)];
    let pixels = document.pixel_size();
    let (view, show_rulers) = (*document.view, document.rulers);
    let editing = matches!(tool, Tool::Move | Tool::Marquee);
    let canvas_id = ui.scope_id().with("mock document canvas");
    // What the host reported; none in a measuring run, where it does not run.
    let mut reported = None;
    let mut scrolled = None;
    let mut zero = document.zero;
    let strips = show_rulers.then(|| Rulers::new("Rulers", Bind::bound(&mut zero)).units(&UNITS));
    let shown =
        Canvas::new("Document").id(canvas_id).view(view).document(page.size()).with_rulers(strips).rulers(units, units).show(ui, |ui, body| {
            let _ = Pasteboard::new("Pasteboard", &pages).show_at(ui, body.rect);
            let Document { guides, guide, .. } = &mut *document;
            let selected = *guide;
            let controls = Guides::new("Guides", canvas_id, Bind::bound(guides)).interactive(editing).selected_guides(selected.as_slice());
            if let Some(select) = controls.show_at(ui, body.rect).select {
                *guide = select;
            }
            let bars = CanvasScrollbars::new("Scroll bars", Bind::given(view), page).show_at(ui, body.rect);
            scrolled = bars.view.value().copied();
            // The tool runs after the controls, which take a press first, and before the picture and the selection,
            // which show what it did with this pass's input. The tool leaves the press alone over a scroll bar.
            if let Some(pass) = tool_pass(&body.env) {
                let host = ToolHost::new().momentary(&mut tools.momentary).enabled(!bars.over);
                reported = Some(match tool {
                    Tool::Hand => host.run(ui, &pass, &mut tools.hand),
                    Tool::Zoom => host.run(ui, &pass, &mut tools.zoom),
                    Tool::Move => host.run(ui, &pass, &mut MoveTool::new(document, Directions::Four)),
                    Tool::Marquee => host.run(ui, &pass, &mut MarqueeTool::new(document)),
                    Tool::Brush | Tool::Pencil | Tool::Eraser | Tool::Stamp => host.run(ui, &pass, &mut NoTool),
                });
            }
            show_layers(ui, body.rect, &document.layers, pixels);
            if let Some(rect) = document.marquee {
                let _ = Marquee::rect("Selection", rect).show_at(ui, body.rect);
            }
        });
    if let Some(picked) = shown.output.rulers.and_then(|rulers| rulers.unit_picked) {
        document.units = picked;
    }
    document.zero = zero;
    let reported: craft_ui::canvas::ToolHostResponse = reported.unwrap_or_default();
    if let Some(next) = scrolled.or(reported.view) {
        *document.view = next;
    }
}

/// The layer table: an eye column and a name column over the layers, top first, with several layers selectable and
/// the lead being the current layer.
/// What the Navigator shows: the document's view, which it pans and zooms, and the document's size.
#[derive(MemoProps)]
struct NavigatorProps<'a> {
    view: Bind<'a, CanvasView>,
    size: Size,
}

/// The Navigator, a memoized view: it runs when the view or the document's size changes or the pointer is over it.
fn navigator(props: &mut NavigatorProps<'_>, ui: &mut Ui) {
    let _ = NavigatorPanel::new("Navigator", props.view.reborrowed(), props.size).show(ui);
}

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
