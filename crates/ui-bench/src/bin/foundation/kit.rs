//! The components the foundation scenarios stand on. Each is the smallest component that exercises the contract
//! path its scenario measures; the family components replace them as they land.

use craft_behavior::{DisplayOrder, ItemSource, PartKey, Selection, Sense, SpanLayout};
use craft_ui::contract::{
    Bind, BindMut, BodyCx, Collection, CollectionProps, CollectionResponse, Common, Component, Container, ContainerProps, Cx, ItemOutput, MeasureCx,
    NoValue, Outcome, PaintUi, PartsCx, Role, SelectMode,
};
use craft_ui::primitives::TextRun;
use craft_ui::style::{ComponentStyle, Corners, Metrics, StyleCx, SurfaceKind};
use egui::{Rect, Response, Vec2, vec2};
use std::convert::Infallible;

/// The height of every list row.
pub const ROW_H: f32 = 20.0;
/// The width of a sheet cell.
pub const CELL_W: f32 = 80.0;

/// A style with a minimum size and no surface.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
pub struct PlainStyle {
    #[style(metrics)]
    metrics: Metrics,
}

impl PlainStyle {
    fn min(min: Vec2) -> Self {
        PlainStyle { metrics: Metrics::new().min_size(min) }
    }
}

/// `n` rows with no text.
pub struct Count(pub usize);

impl DisplayOrder<usize> for Count {
    fn len(&self) -> usize {
        self.0
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0).then_some(*key)
    }
}

impl ItemSource<usize> for Count {}

/// The output of a row.
pub struct RowOut {
    response: Response,
}

impl Outcome for RowOut {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl ItemOutput<Infallible> for RowOut {
    fn into_event(self) -> Option<Infallible> {
        None
    }
}

const CELL: PartKey = PartKey::new("cell");

/// What a row is.
#[derive(Clone, Copy)]
pub enum RowKind {
    /// A sheet row: `cells` cell parts, each painting its own text.
    Cells { row: usize, cells: usize },
}

/// A list row.
#[derive(craft_ui::contract::Component)]
pub struct Row {
    #[common]
    common: Common,
    kind: RowKind,
}

impl Row {
    pub fn new(kind: RowKind) -> Self {
        Row { common: Common::new(), kind }
    }
}

fn cell_rect(rect: Rect, column: usize) -> Rect {
    Rect::from_min_size(rect.min + vec2(column as f32 * CELL_W, 0.0), vec2(CELL_W, rect.height()))
}

impl Component for Row {
    const ROLE: Role = Role::LIST_ITEM;
    type Style = PlainStyle;
    type Output = RowOut;

    fn style(_: &StyleCx<'_>, _: RowStyleKey) -> PlainStyle {
        PlainStyle::min(vec2(10.0, 10.0))
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        vec2(10.0, ROW_H)
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let RowKind::Cells { cells, .. } = self.kind;
        for column in 0..cells {
            cx.part(CELL.at(column as u64), cell_rect(rect, column)).sense(Sense::CLICK).pointer_only("the sheet's keys move between cells");
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let draw = ui.draw();
        let RowKind::Cells { row, cells } = self.kind;
        for column in 0..cells {
            draw.text(cell_rect(cx.rect, column), &TextRun::new(format!("{row}:{column}")));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> RowOut {
        RowOut { response: cx.response.clone() }
    }
}

/// A list over a source, with the selection the scene gives.
#[derive(craft_ui::contract::Component)]
#[component(collection)]
pub struct BenchList {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    items: CollectionProps,
}

impl BenchList {
    pub fn new(select: SelectMode) -> Self {
        BenchList { common: Common::new(), props: ContainerProps::new(), items: CollectionProps::new().select(select) }
    }
}

impl Component for BenchList {
    const ROLE: Role = Role::LIST;
    const PARTS: bool = false;
    type Style = PlainStyle;
    type Output = CollectionResponse<usize, Infallible>;

    fn style(_: &StyleCx<'_>, _: BenchListStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> Self::Output {
        cx.collection_response()
    }
}

impl Collection for BenchList {
    type Key = usize;
    type ItemLayout = SpanLayout;
    type Selection = Selection<usize>;
    type Item<'r> = Row;
    type ItemEvent = Infallible;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn selection(&mut self) -> Option<BindMut<'_, Selection<usize>>> {
        None
    }

    fn item_layout(&self, source: &dyn ItemSource<usize>, width: f32, _: &StyleCx<'_>) -> SpanLayout {
        SpanLayout::new(source, width, ROW_H)
    }
}

/// A leaf that takes `size`.
#[derive(craft_ui::contract::Component)]
pub struct Block {
    #[common]
    common: Common,
    size: Vec2,
}

impl Block {
    pub fn new(size: Vec2) -> Self {
        Block { common: Common::new(), size }
    }
}

impl Component for Block {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: BlockStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        self.size
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

const CONTROL: PartKey = PartKey::new("control");
/// The side of a canvas control.
pub const CONTROL_SIDE: f32 = 12.0;

/// A canvas leaf with `count` control parts, each painted as a knob.
#[derive(craft_ui::contract::Component)]
pub struct BenchCanvas {
    #[common]
    common: Common,
    size: Vec2,
    count: usize,
}

impl BenchCanvas {
    pub fn new(size: Vec2, count: usize) -> Self {
        BenchCanvas { common: Common::new(), size, count }
    }

    fn control(&self, rect: Rect, at: usize) -> Rect {
        let center = rect.min + vec2(40.0 + (at % 10) as f32 * 80.0, 40.0 + (at / 10) as f32 * 120.0);
        Rect::from_center_size(center, Vec2::splat(CONTROL_SIDE))
    }
}

impl Component for BenchCanvas {
    const ROLE: Role = Role::None;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: BenchCanvasStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        self.size
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        for at in 0..self.count {
            cx.part(CONTROL.at(at as u64), self.control(rect, at)).sense(Sense::CLICK).pointer_only("the canvas tools move controls");
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let draw = ui.draw();
        for at in 0..self.count {
            draw.surface(self.control(cx.rect, at), SurfaceKind::Knob, Corners::All);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// A container whose open state the scene owns.
#[derive(craft_ui::contract::Component)]
#[component(container)]
pub struct Panel<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    open: Option<Bind<'a, bool>>,
}

impl<'a> Panel<'a> {
    pub fn new(props: ContainerProps) -> Self {
        Panel { common: Common::new(), props, open: None }
    }

    pub fn bind_open(mut self, open: Bind<'a, bool>) -> Self {
        self.open = Some(open);
        self
    }
}

impl Component for Panel<'_> {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: PanelStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Panel<'_> {
    fn open(&mut self) -> Option<BindMut<'_, bool>> {
        self.open.as_mut().map(Bind::reborrow)
    }

    fn header(&mut self, ui: &mut egui::Ui, _: &mut BodyCx<'_>) {
        ui.allocate_space(vec2(100.0, ROW_H));
    }
}
