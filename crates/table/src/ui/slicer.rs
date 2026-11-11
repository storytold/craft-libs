//! Slicers: tiles that filter a table by a column's values.

use std::borrow::Cow;

use craft_behavior::{DisplayOrder, IconId, ItemSource, Metrics, Selection, Sense, TileLayout};
use craft_theme::Variant;
use craft_theme::variant::{Emphasis, Tone};
use craft_ui::buttons::IconButton;
use craft_ui::contract::{
    ActivateOn, BodyCx, Collection, CollectionEvent, CollectionProps, CollectionResponse, Common, Component, Container, ContainerProps, Cx,
    HeaderRegion, NoValue, Outcome, PaintUi, Pattern, Props, Role, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing,
    Styled as _, WordSetCx as _, clicked,
};
use craft_ui::display::Text;
use craft_ui::primitives::{IconSpec, TextRun};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::{ComponentStyle, Flags, FocusLook, Stateful, StyleColor, StyleCx, SurfaceKind, SurfaceLook, TextLook, TypeRole};
use egui::{Rect, Response, Ui, Vec2, pos2};

#[cfg(feature = "screen-testing")]
mod stories;

craft_ui::words! {
    /// The slicer's words, English by default.
    #[non_exhaustive]
    pub struct SlicerWords {
        /// The accessible name and tooltip of the funnel button in the header.
        clear: "slicer.clear" = "Clear Filter";
    }
}

/// What a click on a slicer did.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SlicerAction {
    /// Toggle the value of this tile.
    Toggle(String),
    /// Clear the filter.
    Clear,
}

/// What the slicer did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct SlicerOutput {
    /// The slicer card's response.
    pub response: Response,
    /// What the user did, if anything. The funnel wins over a tile when both report in one pass.
    pub action: Option<SlicerAction>,
}

impl Outcome for SlicerOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Excel's slicer: a raised card whose header holds the caption and the funnel button (tinted while a filter is on),
/// over value tiles in `columns` columns (on: the accent fill with the on-accent text; off: the hover fill with muted
/// text). A click, Enter or Space on a tile reports [`SlicerAction::Toggle`] with its value, the funnel reports
/// [`SlicerAction::Clear`]; elsewhere a press is the app's (select, drag the object).
///
/// The tiles are one Tab stop whose arrows move between them in reading order and wrap over the rows; the funnel
/// button is a Tab stop of its own. A card too small for all the tiles scrolls them. `zoom` is the sheet zoom the
/// slicer sits at.
#[derive(Pattern)]
pub struct Slicer<'a> {
    caption: Label,
    items: &'a [(String, bool)],
    columns: u32,
    filtered: bool,
    zoom: f32,
}

impl<'a> Slicer<'a> {
    /// A slicer titled `caption` over `items`: each value and whether it is on.
    pub fn new(caption: impl Into<Label>, items: &'a [(String, bool)]) -> Self {
        Slicer { caption: caption.into(), items, columns: 1, filtered: false, zoom: 1.0 }
    }

    /// The number of tile columns; 1 by default.
    #[must_use]
    pub fn columns(mut self, columns: u32) -> Self {
        self.columns = columns.max(1);
        self
    }

    /// Whether a filter is on; the funnel button is then tinted.
    #[must_use]
    pub fn filtered(mut self, on: bool) -> Self {
        self.filtered = on;
        self
    }

    /// The sheet zoom the slicer sits at.
    #[must_use]
    pub fn zoom(mut self, zoom: f32) -> Self {
        self.zoom = zoom;
        self
    }

    /// Shows the slicer in `rect` and reports what the user did.
    pub fn show(self, ui: &mut Ui, rect: Rect) -> SlicerOutput {
        let Slicer { caption, items, columns, filtered, zoom } = self;
        let source = Values(items);
        let card = SlicerCard::new(caption.clone(), filtered);
        let shown = card.show_at(ui, rect, |ui, _| {
            let tiles = SlicerTiles::new(columns, zoom).label(caption.clone());
            tiles.show_items(ui, &source, &mut |key, _| SlicerTile::new(items[*key].0.clone()).on(items[*key].1))
        });
        let toggled = shown.inner.and_then(|tiles| toggled(&tiles, items));
        let SlicerCardOutput { response, cleared } = shown.output;
        let action = if cleared { Some(SlicerAction::Clear) } else { toggled.map(SlicerAction::Toggle) };
        SlicerOutput { response, action }
    }
}

/// The value a tile press or key reported.
fn toggled(tiles: &CollectionResponse<usize, ()>, items: &[(String, bool)]) -> Option<String> {
    tiles.events.iter().find_map(|event| match event {
        CollectionEvent::Activate(key) => items.get(*key).map(|(value, _)| value.clone()),
        _ => None,
    })
}

/// The values keyed by position.
struct Values<'a>(&'a [(String, bool)]);

impl DisplayOrder<usize> for Values<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|(value, _)| Cow::Borrowed(value.as_str()))
    }
}

impl ItemSource<usize> for Values<'_> {}

/// What the card reports.
struct SlicerCardOutput {
    response: Response,
    cleared: bool,
}

impl Outcome for SlicerCardOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The style of the card: the surface comes from the container.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
struct SlicerCardStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The raised card: the caption in its header, the funnel button at the header's end and the tiles in its body.
#[derive(Component, Props)]
#[component(container)]
struct SlicerCard {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    filtered: bool,
    cleared: bool,
}

impl SlicerCard {
    fn new(caption: Label, filtered: bool) -> Self {
        let props = ContainerProps::new()
            .surface(SurfaceKind::Raised)
            .sizing(Sizing::Fill)
            .header(HeaderRegion::new().rule(true).trailing(true))
            .collection_body();
        SlicerCard { common: Common::new(), props, filtered, cleared: false }.label(caption)
    }
}

impl Component for SlicerCard {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = SlicerCardStyle;
    type Output = SlicerCardOutput;

    fn style(_: &StyleCx<'_>, _: SlicerCardStyleKey) -> SlicerCardStyle {
        SlicerCardStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::DRAG
    }

    fn finish(self, cx: Cx<'_, Self>) -> SlicerCardOutput {
        SlicerCardOutput { response: cx.response.clone(), cleared: self.cleared }
    }
}

impl Container for SlicerCard {
    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        Text::new(cx.label()).role(TypeRole::BodyStrong).show(ui);
    }

    fn controls(&mut self, ui: &mut Ui, _: &mut BodyCx<'_>) {
        let clear = Env::of(ui).words::<SlicerWords>().clear.clone();
        let variant = if self.filtered { Variant::look(Emphasis::Ghost, Tone::Accent) } else { Variant::GHOST };
        let button =
            IconButton::new(IconSpec::svg(IconId::new("funnel"))).label(clear.clone()).tooltip(clear.as_str().to_owned()).variant(variant).show(ui);
        if clicked(&button) {
            self.cleared = true;
        }
    }
}

/// The style of the tiles collection.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
struct SlicerTilesStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The tiles: a collection of equal tiles in a fixed number of columns, one Tab stop, no selection of its own. A press
/// or Space on a tile activates it.
#[derive(Component, Props)]
#[component(collection)]
struct SlicerTiles {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    #[collection]
    items: CollectionProps,
    columns: u32,
    zoom: f32,
}

impl SlicerTiles {
    fn new(columns: u32, zoom: f32) -> Self {
        SlicerTiles {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::Fill),
            items: CollectionProps::new().select(SelectMode::None).activate(ActivateOn::Click),
            columns: columns.max(1),
            zoom: if zoom.is_finite() && zoom > 0.0 { zoom } else { 1.0 },
        }
    }
}

impl Component for SlicerTiles {
    const ROLE: Role = Role::LIST_BOX;
    const PARTS: bool = false;
    type Style = SlicerTilesStyle;
    type Output = CollectionResponse<usize, ()>;

    fn style(_: &StyleCx<'_>, _: SlicerTilesStyleKey) -> SlicerTilesStyle {
        SlicerTilesStyle::default()
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> Self::Output {
        cx.collection_response()
    }
}

impl Collection for SlicerTiles {
    type Key = usize;
    type ItemLayout = TileLayout;
    type Selection = Selection<usize>;
    type Item<'r> = SlicerTile;
    type ItemEvent = ();

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn item_layout(&self, source: &dyn ItemSource<usize>, width: f32, cx: &StyleCx<'_>) -> TileLayout {
        let gap = cx.tokens.space.gap.tight;
        // `TileLayout` counts the columns that fit; a tile one step narrower than the column pitch keeps the count at
        // `columns` however the division rounds.
        let pitch = (width + gap) / self.columns as f32;
        let tile_width = (pitch - gap).max(1.0);
        let tile = Vec2::new(f32::from_bits(tile_width.to_bits() - 1), cx.control().height * self.zoom);
        TileLayout::new(source, width, tile, gap)
    }
}

/// The style of a tile.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
struct SlicerTileStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    #[style(text)]
    text: TextLook,
    /// The space between the tile's edge and its text.
    inset: f32,
}

/// One value tile.
#[derive(Component, Props)]
struct SlicerTile {
    #[common]
    common: Common,
    /// Whether the value is on.
    #[prop]
    on: bool,
}

impl SlicerTile {
    fn new(value: String) -> Self {
        SlicerTile { common: Common::new(), on: false }.label(value)
    }
}

impl Component for SlicerTile {
    const ROLE: Role = Role::LIST_ITEM;
    const ALT_ROLES: &'static [Role] = &[Role::LIST_BOX_OPTION];
    const OVERLAYS: bool = false;
    type Style = SlicerTileStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: SlicerTileStyleKey) -> SlicerTileStyle {
        let colors = &cx.tokens.color;
        let (hover, solid): (StyleColor, StyleColor) = (cx.color(colors.state.hover), cx.color(colors.accent.solid));
        let (muted, on_accent): (StyleColor, StyleColor) = (cx.color(colors.text.muted), cx.color(colors.text.on_accent));
        SlicerTileStyle {
            surface: SurfaceLook::new(Stateful::new(hover).when(Flags::CHECKED, solid)).radius(cx.variant_look().radius),
            metrics: Metrics::new().min_size(Vec2::new(0.0, cx.control().height)),
            focus: cx.focus_look(),
            text: TextLook::new(TypeRole::Body, Stateful::new(muted).when(Flags::CHECKED, on_accent)),
            inset: cx.tokens.space.gap.tight,
        }
    }

    fn measure(&mut self, cx: &craft_ui::contract::MeasureCx<'_, Self>) -> Vec2 {
        let text = cx.layout(&TextRun::new(cx.resolved.label).role(cx.style.text.role)).size();
        Vec2::new(text.x + 2.0 * cx.style.inset, text.y)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn flags(&self) -> Flags {
        if self.on { Flags::CHECKED } else { Flags::empty() }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let inset = cx.style.inset;
        let area = Rect::from_min_max(pos2(cx.rect.left() + inset, cx.rect.top()), pos2(cx.rect.right() - inset, cx.rect.bottom()));
        let run = TextRun::new(cx.resolved.label).role(cx.style.text.role).color(cx.resolve(&cx.style.text.color)).elide();
        ui.draw().text(area, &run);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
