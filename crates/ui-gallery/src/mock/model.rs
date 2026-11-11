//! What the mock editor reads and edits: a tool, a layer list, a canvas view, a document title and a dirty flag.

use craft_behavior::dock::{Column, DockEdge, Group, Layout, Node, SplitSize, Workspace};
use craft_behavior::kurbo::Rect as DocRect;
use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{Axis, PixelSize};
use craft_behavior::{CanvasView, Selection};
use craft_ui::bars::StatusItem;
use craft_ui::canvas::{Guide, GuideKey, PickStep, PixelRect, tool_pick};
use craft_ui::contract::CommandSet;
use craft_ui::rails::{LastUsed, RailTool, SlotId, ToolSlot};
use craft_ui::runtime::env::UnitKind;
use egui::{Color32, Rect};

/// The width of the dock column at the end edge.
const SIDE_WIDTH: f32 = 220.0;

/// The width of the mock document, in document units.
const DOCUMENT_WIDTH: f64 = 1200.0;

/// The height of the mock document, in document units.
const DOCUMENT_HEIGHT: f64 = 800.0;

/// The share of the dock the canvas group takes.
const CANVAS_SHARE: f32 = 0.8;

/// The tools of the rail. Each is a command of the `tools` group; a tool alone in its slot carries its letter, and the
/// letter of a slot with several is the pick command of [`ToolKey`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "mockTool")]
#[non_exhaustive]
pub enum Tool {
    /// Move.
    #[command(label = "Move", icon = "move", kind = "tool", group = "tools", default = "V")]
    Move,
    /// Rectangular Marquee.
    #[command(label = "Rectangular Marquee", icon = "marquee_rect", kind = "tool", group = "tools", default = "M")]
    Marquee,
    /// Brush.
    #[command(label = "Brush", icon = "brush", kind = "tool", group = "tools")]
    Brush,
    /// Pencil.
    #[command(label = "Pencil", icon = "pencil", kind = "tool", group = "tools")]
    Pencil,
    /// Eraser.
    #[command(label = "Eraser", icon = "eraser", kind = "tool", group = "tools", default = "E")]
    Eraser,
    /// Clone Stamp.
    #[command(label = "Clone Stamp", icon = "stamp", kind = "tool", group = "tools", default = "S")]
    Stamp,
    /// Hand.
    #[command(label = "Hand", icon = "hand", kind = "tool", group = "tools", default = "H")]
    Hand,
    /// Zoom.
    #[command(label = "Zoom", icon = "zoom_in", kind = "tool", group = "tools", default = "Z")]
    Zoom,
}

/// The keys of the tool slots and the colour pair that are no tool: the letter of a slot with several tools picks
/// the one it shows, Shift with the letter cycles them, and X and D act on the colour pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "mockToolKey")]
#[non_exhaustive]
pub enum ToolKey {
    /// Picks the Brush slot's tool.
    #[command(label = "Brush Tools", default = "B")]
    PickBrush,
    /// Cycles the Brush slot's tools.
    #[command(label = "Cycle Brush Tools", default = "Shift+B")]
    CycleBrush,
    /// Swaps the foreground and background colours.
    #[command(label = "Swap Foreground and Background Colors", default = "X")]
    SwapColors,
    /// Sets the colours to black and white.
    #[command(label = "Default Foreground and Background Colors", default = "D")]
    ResetColors,
}

impl Tool {
    /// The rail's slots: Move and the Marquee alone, then the freehand tools that share a slot.
    pub fn slots() -> Vec<ToolSlot<Tool>> {
        let tool = |tool: Tool| RailTool::new(tool, tool);
        vec![
            ToolSlot::new(0, vec![tool(Tool::Move)]),
            ToolSlot::new(1, vec![tool(Tool::Marquee)]),
            ToolSlot::new(2, vec![tool(Tool::Brush), tool(Tool::Pencil)]),
            ToolSlot::new(3, vec![tool(Tool::Eraser)]),
            ToolSlot::new(4, vec![tool(Tool::Stamp)]),
            ToolSlot::new(5, vec![tool(Tool::Hand)]),
            ToolSlot::new(6, vec![tool(Tool::Zoom)]),
        ]
    }

    /// The slot that holds the tool.
    pub fn slot(self) -> SlotId {
        match self {
            Tool::Move => 0,
            Tool::Marquee => 1,
            Tool::Brush | Tool::Pencil => 2,
            Tool::Eraser => 3,
            Tool::Stamp => 4,
            Tool::Hand => 5,
            Tool::Zoom => 6,
        }
    }

    /// The tool a key picks in the slot `slot` when `current` is the current tool.
    pub fn pick(slot: SlotId, current: Tool, last_used: &LastUsed<Tool>, step: PickStep) -> Option<Tool> {
        let slots = Tool::slots();
        let group = slots.iter().find(|group| group.id == slot)?;
        let tools: Vec<&RailTool<Tool>> = group.tools.iter().collect();
        tool_pick(&tools, slot, &current, last_used, step, |_| true).map(|tool| tool.id)
    }
}

/// The foreground and background colours, as 24-bit RGB values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ColorPair {
    /// The foreground colour.
    pub foreground: u32,
    /// The background colour.
    pub background: u32,
}

impl Default for ColorPair {
    fn default() -> Self {
        ColorPair { foreground: 0x00_0000, background: 0xFF_FFFF }
    }
}

impl ColorPair {
    /// Swaps the two colours.
    pub fn swap(&mut self) {
        std::mem::swap(&mut self.foreground, &mut self.background);
    }

    /// Sets black and white.
    pub fn reset(&mut self) {
        *self = ColorPair::default();
    }
}

/// The panels of the dock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Panel {
    /// The document tabs over the canvas.
    Canvas,
    /// The Navigator.
    Navigator,
    /// The layer list.
    Layers,
}

impl Panel {
    /// Every panel, in the Window menu's order.
    pub const ALL: [Panel; 3] = [Panel::Canvas, Panel::Navigator, Panel::Layers];

    /// Whether the panel stays docked and open: the canvas.
    pub const fn protected(self) -> bool {
        matches!(self, Panel::Canvas)
    }
}

/// The stable identity of a layer: it survives reordering, deleting and adding other layers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LayerId(u64);

/// One layer of the document: a named fill of one rectangle of the page.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Layer {
    /// The layer's identity within its document.
    pub id: LayerId,
    /// The name the layers panel shows; distinct within a document.
    pub name: String,
    /// Whether the layer shows on the canvas.
    pub visible: bool,
    /// The colour the layer fills its bounds with.
    pub color: Color32,
    /// The part of the page the layer covers, in pixels.
    pub bounds: PixelRect,
}

impl Layer {
    /// A visible layer.
    pub fn new(id: LayerId, name: impl Into<String>, color: Color32, bounds: PixelRect) -> Self {
        Layer { id, name: name.into(), visible: true, color, bounds }
    }
}

/// A rectangle being dragged out with the Marquee: where the press began and where the pointer is now.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct MarqueeDrag {
    /// The document point of the press.
    pub start: Point,
    /// The document point of the pointer.
    pub now: Point,
}

impl MarqueeDrag {
    /// The rectangle the drag spans.
    pub fn rect(&self) -> DocRect {
        DocRect::from_points(self.start, self.now)
    }
}

/// The open document.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Document {
    /// The title of the window and the tab.
    pub title: String,
    /// Whether the document has unsaved changes.
    pub dirty: bool,
    /// The size in document units.
    pub size: Size,
    /// The layers, top first.
    pub layers: Vec<Layer>,
    /// The selected layers; the lead is the current layer.
    pub selected: Selection<LayerId>,
    /// The canvas view. A view that was never shown opens fitted.
    pub view: CanvasView,
    /// The ruler zero point.
    pub zero: Point,
    /// The unit the rulers count in.
    pub units: UnitKind,
    /// The guides dragged out of the rulers.
    pub guides: Vec<Guide>,
    /// The selected guide.
    pub guide: Option<GuideKey>,
    /// The selection the Marquee made.
    pub marquee: Option<DocRect>,
    /// The marquee being dragged out.
    pub dragging: Option<MarqueeDrag>,
    /// Whether the rulers show.
    pub rulers: bool,
    /// The number the next layer gets as its identity.
    next_layer: u64,
}

impl Default for Document {
    fn default() -> Self {
        let size = Size::new(DOCUMENT_WIDTH, DOCUMENT_HEIGHT);
        let view = CanvasView::new(Rect::NOTHING, Point::new(size.width / 2.0, size.height / 2.0), 1.0);
        let (width, height) = (DOCUMENT_WIDTH as i32, DOCUMENT_HEIGHT as i32);
        let layers = vec![
            Layer::new(LayerId(2), "Mountains", Color32::from_rgb(96, 110, 130), PixelRect::new(0, height * 5 / 8, width, height)),
            Layer::new(LayerId(1), "Sky", Color32::from_rgb(150, 190, 235), PixelRect::new(0, 0, width, height * 3 / 4)),
            Layer::new(LayerId(0), "Background", Color32::from_rgb(235, 226, 205), PixelRect::new(0, 0, width, height)),
        ];
        Document {
            title: "Landscape.psd".to_owned(),
            dirty: false,
            size,
            selected: Selection::single(layers[0].id),
            layers,
            view,
            zero: Point::ZERO,
            units: UnitKind::Pixels,
            guides: Vec::new(),
            guide: None,
            marquee: None,
            dragging: None,
            rulers: true,
            next_layer: 3,
        }
    }
}

impl Document {
    /// The index of the current layer, counted from the top.
    pub fn current_index(&self) -> Option<usize> {
        let lead = self.selected.lead?;
        self.layers.iter().position(|layer| layer.id == lead)
    }

    /// The current layer: the lead of the selection.
    pub fn current_layer(&self) -> Option<&Layer> {
        self.current_index().map(|at| &self.layers[at])
    }

    /// The size in pixels.
    pub fn pixel_size(&self) -> PixelSize {
        PixelSize::new(self.size.width as u32, self.size.height as u32)
    }

    /// The rectangle the Marquee shows: the one being dragged, else the selection.
    pub fn marquee_rect(&self) -> Option<DocRect> {
        self.dragging.map(|drag| drag.rect()).or(self.marquee)
    }

    /// Gives the document `size`, which marks it changed when the size differs.
    pub fn resize(&mut self, size: PixelSize) {
        let size = Size::new(f64::from(size.width), f64::from(size.height));
        if self.size != size {
            self.size = size;
            self.dirty = true;
        }
    }

    /// Makes the first layer current when the selection lost its lead (a click on no row).
    pub fn keep_current(&mut self) {
        if self.current_index().is_none()
            && let Some(top) = self.layers.first()
        {
            self.selected = Selection::single(top.id);
        }
    }

    /// Adds a layer above the current one and makes it the only selected one, which marks the document changed.
    pub fn add_layer(&mut self) {
        let number = self.next_layer;
        self.next_layer += 1;
        let name = (self.layers.len() + 1..)
            .map(|number| format!("Layer {number}"))
            .find(|name| self.layers.iter().all(|layer| layer.name != *name))
            .unwrap_or_default();
        let (width, height) = (self.size.width as i32, self.size.height as i32);
        let shade = u8::try_from(number * 40 % 200).unwrap_or(0);
        let bounds = PixelRect::new(width / 4, height / 4, width * 3 / 4, height * 3 / 4);
        let layer = Layer::new(LayerId(number), name, Color32::from_rgb(220, 90 + shade / 2, 60 + shade), bounds);
        let at = self.current_index().unwrap_or(0);
        self.selected = Selection::single(layer.id);
        self.layers.insert(at, layer);
        self.dirty = true;
    }

    /// Shows or hides the layer `id`, which marks the document changed.
    pub fn set_visible(&mut self, id: LayerId, visible: bool) {
        if let Some(layer) = self.layers.iter_mut().find(|layer| layer.id == id).filter(|layer| layer.visible != visible) {
            layer.visible = visible;
            self.dirty = true;
        }
    }

    /// Deletes the current layer and selects the layer below it (the one above for the lowest), which marks the
    /// document changed. The last layer stays.
    pub fn delete_current(&mut self) -> Option<Layer> {
        let at = self.current_index().filter(|_| self.layers.len() > 1)?;
        let removed = self.layers.remove(at);
        let below = &self.layers[at.min(self.layers.len() - 1)];
        self.selected = Selection::single(below.id);
        self.dirty = true;
        Some(removed)
    }
}

/// Everything the mock edits.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct MockState {
    /// The dock's panels and where they sit.
    pub workspace: Workspace<Panel>,
    /// The open document.
    pub document: Document,
    /// The current tool.
    pub tool: Tool,
    /// The tool each rail slot showed last.
    pub last_used: LastUsed<Tool>,
    /// The status bar's item.
    pub status_item: StatusItem,
    /// The foreground and background colours.
    pub colors: ColorPair,
    /// The panel whose frame shows alone.
    pub maximized: Option<Panel>,
}

impl Default for MockState {
    fn default() -> Self {
        let side = Column::new(vec![Group::new(vec![Panel::Navigator], 0), Group::new(vec![Panel::Layers], 0)]).with_width(SIDE_WIDTH);
        let root = Node::split(Axis::X, SplitSize::Ratio(CANVAS_SHARE), Node::tabs(vec![Panel::Canvas], 0), Node::dock(DockEdge::End, vec![side]));
        MockState {
            workspace: Workspace::new(Layout::new(Some(root), Vec::new())),
            document: Document::default(),
            tool: Tool::Move,
            last_used: LastUsed::default(),
            status_item: StatusItem::DocumentDimensions,
            colors: ColorPair::default(),
            maximized: None,
        }
    }
}
