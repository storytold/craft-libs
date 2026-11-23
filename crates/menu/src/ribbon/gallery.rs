//! `Gallery`: an in-ribbon gallery with stepping and a More button.
//!
//! The gallery is one component: its parts are a tile per shown item and the column of up, down and More at the
//! reading end, so the tiles are one Tab stop with arrow keys between them. The first shown item is the component's
//! own memory; More opens through the component's overlay.

use std::hash::Hash;

use craft_behavior::{DisplayOrder, Flow, IconId, Insets, ItemSource, Metrics, NavShape, PartKey, Roving, Selection, SelectionChange, Sense};
use craft_keymap::{Chord, CommandRef, Key};
use craft_ui::buttons::IconButton;
use craft_ui::contract::{
    Access, Bind, CollectionEvent, Common, Component, ContextMenuRequest, Cx, DisabledLook, Focus as _, MeasureCx, Name, NoValue, Outcome, Overlay,
    OverlayUi, Overlays as _, PaintUi, PartsCx, Props as _, Remember as _, Role, SelectMode, SetPosition, ShowCollection as _, WordSetCx as _,
};
use craft_ui::display::{Picture, PictureSize};
use craft_ui::list::{ThumbAspect, ThumbSize};
use craft_ui::primitives::textures::{Revision, ThumbSource, Thumbnails};
use craft_ui::primitives::{ContentPainterCx, HAlign, PaintShape, TextRun};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::KeyUse;
use craft_ui::style::{ComponentStyle, Flags, FocusLook, StyleColor, StyleCx, TypeRole};
use craft_ui::tile_grid::{CaptionPlace, Tile, TileGrid};
use egui::{ColorImage, CornerRadius, Rect, Response, Ui, Vec2};

use super::GroupLevel;

#[cfg(feature = "suite")]
pub(crate) mod stories;

const TILE: PartKey = PartKey::new("tile");
const UP: PartKey = PartKey::new("up");
const DOWN: PartKey = PartKey::new("down");
const MORE: PartKey = PartKey::new("more");

/// The keys the strip takes while it holds focus: Up and Down step the tiles as the chevrons do.
const STEP_KEYS: &[Chord] = &[Chord::plain(Key::Up), Chord::plain(Key::Down)];

craft_ui::words! {
    /// The words of the in-ribbon gallery.
    pub struct GalleryWords {
        up: "gallery.up" = "Previous row";
        down: "gallery.down" = "Next row";
        more: "gallery.more" = "More";
    }
}

/// An in-ribbon gallery of tiles with stepping and More. The tile count follows the group's
/// `GroupLevel`. `Image` faces draw from `Picture`'s store, which packs, so packed previews need
/// no other face.
///
/// - The strip shows `full` tiles at `GroupLevel::Full` and `compact` at `Compact`; a `Collapsed` group shows its
///   gallery in a popover at `Full`.
/// - The up and down buttons (and the Up and Down keys on the strip) move the shown tiles by the count shown; the
///   shown run never runs past the last full run, so no tile is empty, and it falls back when the items shrink.
/// - More opens the whole list as a grid, runs a command, or opens the app's own body.
/// - Each tile's picture is [`aspect`](Gallery::aspect) times as wide as the medium thumbnail is high (square unless
///   set), and with `CaptionPlace::Below` the tile names its item under the picture (a style gallery's 'Normal').
#[must_use = "show the gallery with `show`"]
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(overlay, probe(Gallery<'static, usize>))]
pub struct Gallery<'a, K: Clone + Eq + Hash + Send + Sync + 'static> {
    #[common]
    common: Common,
    items: &'a [GalleryItem<'a, K>],
    current: Option<&'a K>,
    full: u8,
    compact: u8,
    /// What the More button opens.
    #[prop]
    more: GalleryMore<'a>,
    /// Whether one rounded frame holds the tiles and the buttons.
    framed: bool,
    /// Whether the buttons give way while every item shows.
    spare_buttons: bool,
    /// The width of each tile's picture as a multiple of its height, the medium thumbnail's (`size.thumb.md`): a wide
    /// sample (a paragraph style's first letters) fills a wide tile instead of shrinking into a square one.
    #[prop]
    aspect: ThumbAspect,
    /// The height of each tile's picture: a thumbnail size.
    thumb_size: ThumbSize,
    /// Where each tile shows its item's name: `CaptionPlace::Below` names it under the picture, in the caption role;
    /// `None` (the default) and `Start` show no name. The name stays each tile's accessible name either way.
    #[prop]
    caption: CaptionPlace,
    /// Each shown tile's rect.
    laid: Vec<Rect>,
    /// The column of stepper buttons, if shown.
    column: Rect,
    picked: Option<K>,
    menu: Option<ContextMenuRequest<K>>,
}

/// The remembered state of a gallery while it is shown.
#[derive(Clone, Default)]
struct State {
    /// The first item the strip shows.
    start: usize,
}

/// The first item of the last full run: stepping never goes past it, so no tile of the strip is empty.
fn last_start(len: usize, visible: usize) -> usize {
    len - len.min(visible)
}

impl<'a, K: Clone + Eq + Hash + Send + Sync + 'static> Gallery<'a, K> {
    /// A gallery named `label` over `items`, with `current` marked. The counts of `tiles` are the caller's: a
    /// gallery without them shows one tile per level.
    pub fn new(label: impl Into<Label>, items: &'a [GalleryItem<'a, K>], current: Option<&'a K>) -> Self {
        Gallery {
            common: Common::new(),
            items,
            current,
            full: 1,
            compact: 1,
            more: GalleryMore::Grid,
            framed: false,
            spare_buttons: false,
            aspect: ThumbAspect::SQUARE,
            thumb_size: ThumbSize::Medium,
            caption: CaptionPlace::None,
            laid: Vec::new(),
            column: Rect::NOTHING,
            picked: None,
            menu: None,
        }
        .label(label)
    }

    /// How many tiles show at the full and the compact level, each at least 1.
    pub fn tiles(mut self, full: u8, compact: u8) -> Self {
        self.full = full.max(1);
        self.compact = compact.max(1);
        self
    }

    /// One rounded frame on the sunken well surface around the tiles and the buttons, as the old Word-style ribbon drew
    /// its Styles gallery.
    pub fn framed(mut self) -> Self {
        self.framed = true;
        self
    }

    /// No up, down and More buttons while every item shows at the level: nothing is left to step to.
    pub fn spare_buttons(mut self) -> Self {
        self.spare_buttons = true;
        self
    }

    /// The height of each tile's picture, a thumbnail size (`size.thumb.*`; medium unless set): a gallery whose
    /// previews are small pages (a style set's title, heading and paragraph) takes the large size, so the pages read.
    /// `ThumbSize::None` keeps the medium size: a gallery tile always shows its picture.
    pub fn thumb(mut self, size: ThumbSize) -> Self {
        self.thumb_size = if size == ThumbSize::None { ThumbSize::Medium } else { size };
        self
    }

    /// Whether the tiles name their items under the pictures.
    fn captioned(&self) -> bool {
        self.caption == CaptionPlace::Below
    }

    /// The tiles the level shows at most.
    fn visible(&self, level: GroupLevel) -> usize {
        usize::from(if level == GroupLevel::Compact { self.compact } else { self.full }).max(1)
    }

    /// Whether the level shows the column of buttons.
    fn buttons(&self, level: GroupLevel) -> bool {
        !(self.spare_buttons && self.items.len() <= self.visible(level))
    }

    /// The room between a framed gallery's edge and its tiles; none without a frame.
    fn inset(&self, style: &GalleryStyle) -> f32 {
        if self.framed { style.inset } else { 0.0 }
    }

    /// The position of the current item.
    fn current_at(&self) -> Option<usize> {
        let key = self.current?;
        self.items.iter().position(|item| item.key == *key)
    }

    /// The first shown item for `state` at `level`, clamped in the pass that reads the count so a shorter list never
    /// shows an empty strip.
    fn start_of(&self, state: &State, level: GroupLevel) -> usize {
        state.start.min(last_start(self.items.len(), self.visible(level)))
    }

    fn sizes(&self, cx: &MeasureCx<'_, Self>) -> Sizes {
        let words = cx.env.words::<GalleryWords>();
        let button = cx.face(IconButton::new(IconId::new("more")).label(words.more.clone()));
        Sizes { tile: cx.face(GalleryTile::blank(self.aspect, self.thumb_size, self.captioned())), button, gap: cx.gap() }
    }
}

/// One tile of a gallery.
pub struct GalleryItem<'a, K> {
    /// What the tile picks.
    pub key: K,
    /// The tile's name.
    pub label: Label,
    /// How the tile is drawn.
    pub face: GalleryFace<'a>,
}

/// How a gallery tile is drawn. Each tile builds a `Picture` from its face when it shows.
#[non_exhaustive]
pub enum GalleryFace<'a> {
    /// An image from `Picture`'s store, made again when `revision` changes.
    #[non_exhaustive]
    Image {
        /// The store key.
        key: u64,
        /// The image's revision.
        revision: Revision,
        /// Makes the image.
        image: &'a dyn Fn() -> ColorImage,
    },
    /// A rendered thumbnail.
    #[non_exhaustive]
    Rendered {
        /// The thumbnail key.
        key: u64,
        /// The thumbnail store.
        thumbnails: &'a Thumbnails<u64>,
        /// What renders the thumbnail.
        source: &'a dyn ThumbSource<u64>,
    },
    /// A tile painted directly.
    Paint(&'a dyn Fn(&ContentPainterCx, Rect)),
}

/// The size every tile's picture has, `size` high and `aspect` times as wide, so a tile measures the same with and
/// without its picture.
fn thumb(picture: Picture<'_>, aspect: ThumbAspect, size: ThumbSize) -> Picture<'_> {
    picture.extent(PictureSize::Thumb(size)).aspect(aspect)
}

/// A picture that paints nothing, for measuring a tile.
fn blank_picture(aspect: ThumbAspect, size: ThumbSize) -> Picture<'static> {
    thumb(Picture::painter(Label::default(), aspect, |_: &ContentPainterCx<'_>, _: Rect| {}), aspect, size)
}

impl<'a> GalleryFace<'a> {
    /// An image from `Picture`'s store under `key`, made by `image` again when `revision` changes.
    pub fn image(key: u64, revision: Revision, image: &'a dyn Fn() -> ColorImage) -> Self {
        GalleryFace::Image { key, revision, image }
    }

    /// A thumbnail under `key`, rendered by `source` into `thumbnails`.
    pub fn rendered(key: u64, thumbnails: &'a Thumbnails<u64>, source: &'a dyn ThumbSource<u64>) -> Self {
        GalleryFace::Rendered { key, thumbnails, source }
    }

    /// The picture this face shows, `size` high and `aspect` times as wide. It is decoration: the tile that holds it
    /// carries the name.
    fn picture(&self, aspect: ThumbAspect, size: ThumbSize) -> Picture<'a> {
        let picture = match self {
            GalleryFace::Image { key, revision, image } => Picture::new(Label::default(), *key, *image).revision(*revision),
            GalleryFace::Rendered { key, thumbnails, source } => Picture::rendered(Label::default(), *key, thumbnails, *source),
            GalleryFace::Paint(paint) => {
                let paint = *paint;
                Picture::painter(Label::default(), aspect, move |cx: &ContentPainterCx<'_>, rect: Rect| paint(cx, rect))
            }
        };
        thumb(picture, aspect, size)
    }
}

/// What the More button of a gallery opens.
#[non_exhaustive]
pub enum GalleryMore<'a> {
    /// A grid of every tile.
    Grid,
    /// The command that opens the app's own surface.
    Command(CommandRef),
    /// The app's menu body.
    Body(&'a mut dyn FnMut(&mut Ui)),
}

/// What a gallery reports for a pass.
#[non_exhaustive]
pub struct GalleryOutput<K> {
    /// The tile picked this pass.
    pub picked: Option<K>,
    /// A context menu was asked for on a tile: a secondary click, the Menu key or Shift+F10.
    pub menu: Option<ContextMenuRequest<K>>,
    /// The gallery's response.
    // PLAN: S-L214 drops this field, but `Outcome` (the component output's bound) reads the response through it.
    pub response: Response,
}

impl<K> Outcome for GalleryOutput<K> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The More grid.

/// The items of the More grid, keyed by their position.
struct Indexes(usize);

impl DisplayOrder<usize> for Indexes {
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

impl ItemSource<usize> for Indexes {}

/// What the More grid reports: the picked item and the item a menu was asked for, by position.
#[derive(Default)]
struct GridResult {
    picked: Option<usize>,
    menu: Option<(usize, egui::Pos2)>,
}

/// The whole list as a grid of the same faces; a click or Enter picks, and a context request is forwarded. The
/// grid sizes its columns from the room it is given.
fn grid<K>(ui: &mut Ui, label: &Label, items: &[GalleryItem<'_, K>], current: Option<usize>, aspect: ThumbAspect, size: ThumbSize) -> GridResult {
    let selection = Selection::new(current.into_iter().collect(), current, current);
    let grid = TileGrid::<usize>::new(label.clone()).select(SelectMode::Single).selection(Bind::given(selection));
    let out = grid.show_items(ui, &Indexes(items.len()), &mut |key, _| match items.get(*key) {
        Some(item) => Tile::new(item.label.clone()).picture(aspect, item.face.picture(aspect, size)),
        None => Tile::new(Label::default()),
    });
    let mut result = GridResult::default();
    for event in out.response.events {
        match event {
            CollectionEvent::Activate(key) => result.picked = Some(key),
            CollectionEvent::Selection(SelectionChange::Set(keys)) => {
                if let Some(key) = keys.last() {
                    result.picked = Some(*key);
                }
            }
            CollectionEvent::ContextMenu(request) => result.menu = Some((request.key, request.at)),
            _ => {}
        }
    }
    result
}

// ---------------------------------------------------------------------------------------------------------------
// The tile.

/// The style of a gallery tile: the theme's selection fill and edge while it is the current one.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
struct GalleryTileStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// Between the tile's edge and its picture.
    pad: f32,
    /// The selection edge's width.
    stroke: f32,
    /// The current tile's fill.
    fill: StyleColor,
    /// The current tile's edge.
    edge: StyleColor,
    /// The ink of the item's name under the picture, in a captioned gallery.
    caption: StyleColor,
    /// Between the picture and the caption.
    gap: f32,
    /// The picture's height at each thumbnail size, small, medium and large (`size.thumb.sm`, `md`, `lg`), as `thumb`
    /// sizes it.
    thumbs: [f32; 3],
}

impl GalleryTileStyle {
    /// The picture's height at `size`.
    fn thumb(&self, size: ThumbSize) -> f32 {
        let [small, medium, large] = self.thumbs;
        match size {
            ThumbSize::Small => small,
            ThumbSize::Large => large,
            _ => medium,
        }
    }
}

/// The face a tile part wears: its picture, and in a captioned gallery the item's name under it, inside the tile's
/// selection look.
#[derive(craft_ui::contract::Component)]
#[component(unregistered)]
struct GalleryTile<'a> {
    #[common]
    common: Common,
    /// The picture shown; a tile with none only has its size.
    picture: Option<Picture<'a>>,
    /// Whether the tile is the gallery's current item.
    selected: bool,
    /// The width of the picture as a multiple of its height.
    aspect: ThumbAspect,
    /// The picture's height, a thumbnail size.
    thumb_size: ThumbSize,
    /// The item's name under the picture; `None` in a gallery without captions.
    caption: Option<Label>,
}

impl<'a> GalleryTile<'a> {
    /// A tile that shows `picture`, `size` high and `aspect` times as wide, with `caption` under it when set.
    fn of(picture: Picture<'a>, selected: bool, aspect: ThumbAspect, size: ThumbSize, caption: Option<Label>) -> Self {
        GalleryTile { common: Common::new(), picture: Some(picture), selected, aspect, thumb_size: size, caption }
    }

    /// A tile with no picture, for measuring; a `captioned` one keeps a line for its caption.
    fn blank(aspect: ThumbAspect, size: ThumbSize, captioned: bool) -> Self {
        GalleryTile { common: Common::new(), picture: None, selected: false, aspect, thumb_size: size, caption: captioned.then(Label::default) }
    }
}

/// The caption's type role.
const CAPTION: TypeRole = TypeRole::Caption;

impl Component for GalleryTile<'_> {
    const DISABLED: DisabledLook = DisabledLook::Faded;
    const ROLE: Role = Role::IMAGE;
    const PARTS: bool = false;
    const OVERLAYS: bool = false;
    const SHOWS_LABEL: bool = false;
    type Style = GalleryTileStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: GalleryTileStyleKey) -> GalleryTileStyle {
        let colors = &cx.tokens.color;
        let pad = cx.tokens.space.pad.chip;
        GalleryTileStyle {
            metrics: Metrics::new().padding(Insets::new(pad, pad, pad, pad)),
            focus: cx.focus_look(),
            pad,
            stroke: cx.tokens.stroke.thin,
            fill: cx.color(colors.state.selected),
            edge: cx.color(colors.line.control_edge),
            caption: cx.color(colors.text.normal),
            gap: cx.tokens.space.gap.tight,
            thumbs: [cx.tokens.size.thumb.sm, cx.tokens.size.thumb.md, cx.tokens.size.thumb.lg],
        }
    }

    /// The picture, and in a captioned gallery the gap and one caption line under it. The caption is as wide as the
    /// picture: a longer name elides.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let picture = cx.face(blank_picture(self.aspect, self.thumb_size));
        match self.caption {
            Some(_) => {
                let line = cx.layout(&TextRun::new("Ag").role(CAPTION)).size().y;
                Vec2::new(picture.x, picture.y + cx.style.gap + line)
            }
            None => picture,
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let area = cx.content_rect;
        if area.width() <= 0.0 || area.height() <= 0.0 {
            return;
        }
        if self.selected {
            let outer = area.expand(cx.style.pad);
            let draw = ui.draw();
            draw.fill(&PaintShape::Rect(outer, CornerRadius::ZERO), cx.style.edge);
            draw.fill(&PaintShape::Rect(outer.shrink(cx.style.stroke), CornerRadius::ZERO), cx.style.fill);
        }
        // A captioned tile keeps the picture's height at the top and shows the name in the rest under it.
        let (area, caption) = match self.caption {
            Some(_) => {
                let picture = Rect::from_min_size(area.min, Vec2::new(area.width(), cx.style.thumb(self.thumb_size).min(area.height())));
                let top = (picture.max.y + cx.style.gap).min(area.max.y);
                (picture, Some(Rect::from_min_max(egui::pos2(area.min.x, top), area.max)))
            }
            None => (area, None),
        };
        if let Some(picture) = self.picture.take() {
            let _ = ui.child("picture", area, picture);
        }
        if let (Some(rect), Some(name)) = (caption, self.caption.as_ref()) {
            let run = TextRun::new(name.as_str()).role(CAPTION).color(cx.style.caption).align(HAlign::Center).elide();
            let _ = ui.draw().text(rect, &run);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The gallery's component.

/// The style of the gallery: the gap between its tiles.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct GalleryStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// A framed gallery's fill, edge and corner radius.
    frame: (StyleColor, StyleColor, f32),
    /// The room inside a frame.
    inset: f32,
    /// The stepper buttons column's track fill and divider.
    track: StyleColor,
    divider: StyleColor,
}

/// The sizes the gallery's faces measure.
struct Sizes {
    tile: Vec2,
    button: Vec2,
    gap: f32,
}

impl<K: Clone + Eq + Hash + Send + Sync + 'static> Component for Gallery<'_, K> {
    const ROLE: Role = Role::RADIO_GROUP;
    type Style = GalleryStyle;
    type Output = GalleryOutput<K>;

    fn style(cx: &StyleCx<'_>, _: GalleryStyleKey<K>) -> GalleryStyle {
        let colors = &cx.tokens.color;
        GalleryStyle {
            metrics: Metrics::new().gap(cx.tokens.space.gap.inline),
            frame: (cx.color(colors.surface.sunken), cx.color(colors.line.control_edge), cx.tokens.shape.radius.control),
            inset: cx.tokens.space.gap.inline,
            track: cx.color(colors.surface.panel),
            divider: cx.color(colors.line.divider),
        }
    }

    fn keys(&self) -> KeyUse {
        Self::KEYS.with_claims(STEP_KEYS)
    }

    /// The room for every tile of the level, so the gallery keeps its width while the last run shows fewer, and the
    /// column of three buttons.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let sizes = self.sizes(cx);
        let level = cx.env.get::<GroupLevel>();
        let slots = self.visible(level) as f32;
        let tiles = slots * sizes.tile.x + (slots - 1.0).max(0.0) * sizes.gap;
        let inner = if self.buttons(level) {
            Vec2::new(tiles + sizes.gap + sizes.button.x, sizes.tile.y.max(3.0 * sizes.button.y))
        } else {
            Vec2::new(tiles, sizes.tile.y)
        };
        // The frame takes room at the sides only: the tiles and buttons already keep their own room above and below.
        inner + Vec2::new(2.0 * self.inset(cx.style), 0.0)
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    /// Places the tiles from the start edge and the column of buttons at the end edge, and declares them as one
    /// roving group in reading order. Which items the tiles hold follows the remembered step, which `update`, `paint`
    /// and `access` read; the part set depends only on the count.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let sizes = self.sizes(cx);
        let level = cx.env.get::<GroupLevel>();
        let shown = self.visible(level).min(self.items.len());
        let buttons = self.buttons(level);
        let rect = rect.shrink2(Vec2::new(self.inset(cx.style), 0.0));
        let (column, rest) = Flow::new(rect, cx.flow.is_rtl()).take_end(if buttons { sizes.button.x } else { 0.0 });
        self.column = if buttons { column } else { Rect::NOTHING };
        self.laid.clear();
        for at in 0..shown {
            let cell = rest.span(at as f32 * (sizes.tile.x + sizes.gap), sizes.tile.x);
            self.laid.push(cell);
            cx.part(TILE.at(at as u64), cell).sense(Sense::CLICK | Sense::FOCUS).focusable();
        }
        let height = column.height() / 3.0;
        let cell = |at: usize| Rect::from_min_size(column.min + Vec2::new(0.0, at as f32 * height), Vec2::new(column.width(), height));
        for (at, key) in [UP, DOWN, MORE].into_iter().enumerate().filter(|_| buttons) {
            let part = cx.part(key, cell(at)).sense(Sense::CLICK | Sense::FOCUS).focusable();
            if key == MORE && !matches!(self.more, GalleryMore::Command(_)) {
                part.toggles("more");
            }
        }
        cx.roving(Roving::new(NavShape::Flat));
    }

    /// The gallery joins the body's roving group; a click or the activation key on a tile picks it, on a chevron
    /// steps, and on More fires the command (a surface of its own opens through the part's toggle). Up and Down on the
    /// strip step as the chevrons do. A pass applies only its own input, so a second pass of the frame steps nothing.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        cx.join_roving();
        let level = cx.env.get::<GroupLevel>();
        let visible = self.visible(level);
        let len = self.items.len();
        let last = last_start(len, visible);
        let start = self.start_of(cx.memory::<State>(), level);
        for at in 0..visible.min(len) {
            if cx.part_activated(TILE.at(at as u64)).is_some()
                && let Some(item) = self.items.get(start + at)
            {
                self.picked = Some(item.key.clone());
            }
        }
        let (up_key, down_key) = (cx.take_claimed(STEP_KEYS[0]), cx.take_claimed(STEP_KEYS[1]));
        let up = cx.part_activated(UP).is_some() || up_key;
        let down = cx.part_activated(DOWN).is_some() || down_key;
        if let Some(source) = cx.part_activated(MORE)
            && let GalleryMore::Command(command) = &self.more
        {
            cx.fire(command.clone(), source);
        }
        let mut next = start;
        if up {
            next = start.saturating_sub(visible);
        }
        if down {
            next = (start + visible).min(last);
        }
        cx.memory::<State>().start = next;
    }

    /// The whole list as a grid of the same faces, an app's own body, or nothing for a command.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        // The More part registers the popup (`toggles`); a gallery whose buttons gave way has none to show.
        if matches!(self.more, GalleryMore::Command(_)) || !self.buttons(cx.env.get::<GroupLevel>()) {
            return;
        }
        let popup = cx.popup("more");
        let name = cx.label();
        let (items, current, aspect, size) = (self.items, self.current_at(), self.aspect, self.thumb_size);
        let more = &mut self.more;
        let shown = ui.show(popup, Overlay::popover(), |ui, _| match more {
            GalleryMore::Body(body) => {
                (**body)(ui);
                GridResult::default()
            }
            _ => grid(ui, &name, items, current, aspect, size),
        });
        let Some(result) = shown else { return };
        if let Some(item) = result.picked.and_then(|at| items.get(at)) {
            self.picked = Some(item.key.clone());
            popup.close(cx);
        }
        if let Some((at, point)) = result.menu
            && let Some(item) = items.get(at)
        {
            self.menu = Some(ContextMenuRequest::new(item.key.clone(), point));
        }
    }

    /// Each tile is a radio button named by its label with its place in the whole list and checked while it is the
    /// current item; the column's buttons are named by the gallery's words.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<GalleryWords>();
        let level = cx.env.get::<GroupLevel>();
        let start = self.start_of(&cx.recall::<State>(), level);
        let current = self.current_at();
        for at in 0..self.visible(level).min(self.items.len()) {
            let Some(item) = self.items.get(start + at) else { continue };
            let node = access.node(TILE.at(at as u64), Role::RADIO_BUTTON);
            node.name = Name::Text(item.label.as_str().to_owned());
            node.position = Some(SetPosition::new(start + at).of(self.items.len()));
            if current == Some(start + at) {
                node.flags |= Flags::CHECKED;
            }
        }
        if !self.buttons(level) {
            return;
        }
        access.node(UP, Role::BUTTON).name = Name::Text(words.up.as_str().to_owned());
        access.node(DOWN, Role::BUTTON).name = Name::Text(words.down.as_str().to_owned());
        access.node(MORE, Role::BUTTON).name = Name::Text(words.more.as_str().to_owned());
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let words = cx.words::<GalleryWords>();
        let level = cx.env.get::<GroupLevel>();
        let visible = self.visible(level);
        let len = self.items.len();
        let last = last_start(len, visible);
        let start = self.start_of(&cx.recall::<State>(), level);
        let current = self.current_at();
        if self.framed {
            let (fill, edge, radius) = cx.style.frame;
            let shape = PaintShape::Rect(cx.rect, CornerRadius::same(radius.round().clamp(0.0, 255.0) as u8));
            let draw = ui.draw();
            draw.fill(&shape, fill);
            draw.stroke(&shape, craft_ui::primitives::StrokeWidth::Thin, edge);
        }
        for at in 0..visible.min(len) {
            if let Some(item) = self.items.get(start + at) {
                let caption = self.captioned().then(|| item.label.clone());
                let tile = GalleryTile::of(
                    item.face.picture(self.aspect, self.thumb_size),
                    current == Some(start + at),
                    self.aspect,
                    self.thumb_size,
                    caption,
                );
                ui.face(TILE.at(at as u64), tile);
            }
        }
        if !self.buttons(level) {
            return;
        }
        if self.framed && self.column.is_positive() {
            let draw = ui.draw();
            let radius = cx.style.frame.2.round().clamp(0.0, 255.0) as u8;
            let track_shape = PaintShape::Rect(self.column, CornerRadius::same(radius));
            draw.fill(&track_shape, cx.style.track);
            draw.stroke(&track_shape, craft_ui::primitives::StrokeWidth::Thin, cx.style.divider);
            let height = self.column.height() / 3.0;
            let y1 = self.column.min.y + height;
            let y2 = self.column.min.y + 2.0 * height;
            draw.rule(craft_ui::style::RuleKind::Divider, egui::pos2(self.column.left(), y1), egui::pos2(self.column.right(), y1));
            draw.rule(craft_ui::style::RuleKind::Divider, egui::pos2(self.column.left(), y2), egui::pos2(self.column.right(), y2));
        }
        ui.face(UP, IconButton::new(IconId::new("chevron_up")).label(words.up.clone()).enabled(start > 0));
        ui.face(DOWN, IconButton::new(IconId::new("chevron_down")).label(words.down.clone()).enabled(start < last));
        ui.face(MORE, IconButton::new(IconId::new("more")).label(words.more.clone()));
    }

    /// The context request is recorded after `update`, so `finish` reads it: a secondary click on a tile, or the Menu
    /// key or Shift+F10 with a tile active, from the tile's bottom start corner. A request from the More grid, made in
    /// `overlay`, wins.
    fn finish(self, cx: Cx<'_, Self>) -> GalleryOutput<K> {
        let start = self.start_of(&cx.recall::<State>(), cx.env.get::<GroupLevel>());
        let menu = self.menu.or_else(|| {
            let part = cx.context_part().filter(|part| part.key == TILE)?;
            let at = usize::try_from(part.index?).ok()?;
            let item = self.items.get(start + at)?;
            Some(ContextMenuRequest::new(item.key.clone(), self.laid.get(at)?.left_bottom()))
        });
        GalleryOutput { picked: self.picked, menu, response: cx.response.clone() }
    }
}
