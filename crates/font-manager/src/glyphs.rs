//! [`GlyphsPanel`]: Photoshop's Glyphs panel. A family and style picker, a category list (Entire Font, Punctuation,
//! Currency, ...), a grid of glyph cells drawn by the app's text engine, a zoom slider, and the recently used
//! glyphs in a row at the top. Double-click or Enter inserts the glyph where the app can insert.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use std::sync::Arc;

use craft_behavior::{IconId, Selection};
use craft_host::Section;
use craft_ui::contract::{
    ActivateOn, Bind, CollectionEvent, CommandSet, Entry, NoValue, Outcome, Pad, Pattern, Props, Remember as _, SelectMode, Show as _,
    ShowCollection as _, ShowContainer as _,
};
use craft_ui::display::{Shade, Spinner, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::ThumbAspect;
use craft_ui::list::ThumbSize;
use craft_ui::pickers::Select;
use craft_ui::popover::Popover;
use craft_ui::primitives::ContentPainterCx;
use craft_ui::primitives::textures::{Priority, Revision, ThumbPaint, ThumbRender, ThumbRequest, ThumbSource, ThumbnailOptions, Thumbnails};
use craft_ui::runtime::commands::{CommandState, Commands as _};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::{DataColor, SurfaceKind};
use craft_ui::tile_grid::{CaptionPlace, GridZoom, ThumbnailSlider, Tile, TileGrid, TilePartEvent, TileZoom};
use egui::{ColorImage, Context, Rect, Response, Ui};
use serde::{Deserialize, Serialize};

use crate::panel::browse::SCAN_POLL_SECONDS;
use crate::picker::{read_bind, write_bind};

craft_ui::words! {
    /// The panel's words, English by default.
    #[non_exhaustive]
    pub struct GlyphsWords {
        show: "glyphs.show" = "Show:";
        entire_font: "glyphs.entireFont" = "Entire Font";
        recent: "glyphs.recent" = "Recently Used";
        style: "glyphs.style" = "Style";
        zoom: "glyphs.zoom" = "Zoom";
        no_text: "glyphs.noText" = "Edit text to insert glyphs";
        loading: "glyphs.loading" = "Loading fonts…";
        font: "glyphs.font" = "Font";
        category: "glyphs.category" = "Category";
        grid: "glyphs.grid" = "Glyphs";
        count: count "glyphs.count" = "{n} glyph" | "{n} glyphs";
        /// `{code}` the lead glyph's code point, `{count}` the count word.
        selected: template "glyphs.selected" = "U+{code}  ·  {count}";
        /// `{char}` the glyph, `{code}` its code point.
        cell: template "glyphs.cell" = "{char}, U+{code}";
        /// `{code}` the lead character's code point, `{gid}` its glyph id, `{count}` the count word.
        selected_glyph: template "glyphs.selectedGlyph" = "U+{code}  glyph {gid}  ·  {count}";
        /// `{gid}` the lead glyph's id, `{count}` the count word.
        unencoded: template "glyphs.unencoded" = "glyph {gid}  ·  {count}";
        /// `{name}` the glyph's name, `{gid}` its id.
        glyph_cell: template "glyphs.glyphCell" = "{name}, glyph {gid}";
        punctuation: "glyphs.categoryPunctuation" = "Punctuation";
        alternates: "glyphs.categoryAlternates" = "Alternates for Selection";
        numbers: "glyphs.categoryNumbers" = "Numbers";
        currency: "glyphs.categoryCurrency" = "Currency";
        symbols: "glyphs.categorySymbols" = "Symbols";
        math_symbols: "glyphs.categoryMathSymbols" = "Math Symbols";
        arrows: "glyphs.categoryArrows" = "Arrows";
        latin: "glyphs.categoryLatin" = "Latin";
        greek: "glyphs.categoryGreek" = "Greek";
        cyrillic: "glyphs.categoryCyrillic" = "Cyrillic";
        letterlike: "glyphs.categoryLetterlike" = "Letterlike Symbols";
        scripts: "glyphs.categoryScripts" = "Superscripts and Subscripts";
        ornaments: "glyphs.categoryOrnaments" = "Ornaments";
    }
}

/// The glyph categories, Photoshop's list in order. These English names are the stable ids stored in the settings
/// and passed to [`GlyphSource::cells`]; the panel shows them through [`GlyphsWords::category_name`].
pub const CATEGORIES: &[&str] = &[
    "Entire Font",
    "Recently Used",
    "Punctuation",
    "Alternates for Selection",
    "Numbers",
    "Currency",
    "Symbols",
    "Math Symbols",
    "Arrows",
    "Latin",
    "Greek",
    "Cyrillic",
    "Letterlike Symbols",
    "Superscripts and Subscripts",
    "Ornaments",
];

impl GlyphsWords {
    /// The shown name of a [`CATEGORIES`] id (the id itself for any other).
    pub fn category_name<'a>(&'a self, id: &'a str) -> &'a str {
        match id {
            "Entire Font" => self.entire_font.as_str(),
            "Recently Used" => self.recent.as_str(),
            "Punctuation" => self.punctuation.as_str(),
            "Alternates for Selection" => self.alternates.as_str(),
            "Numbers" => self.numbers.as_str(),
            "Currency" => self.currency.as_str(),
            "Symbols" => self.symbols.as_str(),
            "Math Symbols" => self.math_symbols.as_str(),
            "Arrows" => self.arrows.as_str(),
            "Latin" => self.latin.as_str(),
            "Greek" => self.greek.as_str(),
            "Cyrillic" => self.cyrillic.as_str(),
            "Letterlike Symbols" => self.letterlike.as_str(),
            "Superscripts and Subscripts" => self.scripts.as_str(),
            "Ornaments" => self.ornaments.as_str(),
            other => other,
        }
    }
}

/// A grid cell: a character the face maps, or a glyph no character maps (unencoded, positional forms).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GlyphCell {
    /// A character the face maps.
    Char(char),
    /// A glyph id no character maps.
    Glyph(u32),
}

/// What the source says about one shown cell this pass.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct CellInfo {
    /// The glyph id.
    pub gid: Option<u32>,
    /// The glyph's name.
    pub name: Option<String>,
    /// A note line.
    pub note: Option<String>,
    /// Whether the cell is shown dimmed.
    pub dimmed: bool,
    /// Whether the cell has alternate glyphs.
    pub has_alternates: bool,
}

/// Where glyph cells come from: the app's text engine draws a glyph of a face at a pixel size and lists the
/// cells a face has. Apps plug in their own.
pub trait GlyphSource: Send + Sync {
    /// The cells of a category: characters in code point order, then unencoded glyphs. `"Entire Font"` is every
    /// cell; any other id is a [`CATEGORIES`] id or one of the source's [`categories`](GlyphSource::categories).
    fn cells(&self, family: &str, style: &str, category: &str) -> Vec<GlyphCell>;
    /// The glyph of a character or glyph id in the face, `px` pixels tall, as an image for a texture; `None` when
    /// the face lacks it. The panel tints the image with the text colour, so a glyph is white with its coverage in
    /// the alpha channel.
    fn glyph(&self, family: &str, style: &str, cell: GlyphCell, px: u32) -> Option<ColorImage>;
    /// Whether the fonts are read. Not ready, the panel shows a spinner and muted text and no grid, and asks for a
    /// repaint after the poll interval.
    fn ready(&self) -> bool {
        true
    }
    /// Changes when the source's faces change, so the pictures and the cell lists are read again.
    fn revision(&self) -> u64 {
        0
    }
    /// The face's own categories (id, shown name), listed after [`CATEGORIES`].
    fn categories(&self, family: &str, style: &str) -> Vec<(String, String)> {
        let _ = (family, style);
        Vec::new()
    }
    /// Glyph id and name, a note line, dimmed, alternates; called for shown cells only.
    fn describe(&self, family: &str, style: &str, cell: GlyphCell) -> CellInfo {
        let _ = (family, style, cell);
        CellInfo::default()
    }
    /// The popup's cells, the cell's own glyph first; called when the popup opens.
    fn alternates(&self, family: &str, style: &str, cell: GlyphCell) -> Vec<GlyphCell> {
        let _ = (family, style, cell);
        Vec::new()
    }
}

/// The panel's remembered state (`[glyphs_panel]`): category, zoom and the recent glyphs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct GlyphsPanelSettings {
    /// The chosen category, one of [`CATEGORIES`] or a category the source offers.
    pub category: String,
    /// The cell size as a multiple of the medium thumbnail, within [`ZOOM`]; a missing or unusable value is the
    /// zoom's default.
    pub zoom: f32,
    /// Most recent first, at most 25 (Photoshop).
    pub recent: Vec<String>,
}

impl Section for GlyphsPanelSettings {
    const KEY: &'static str = "glyphs_panel";
}

/// What the panel reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct GlyphsPanelOutput {
    /// The panel's response.
    pub response: Response,
    /// Double-click or Enter on a cell (grid or recents) while `can_insert`; the host inserts or applies it. A
    /// `Char` cell still enters the recents.
    pub activated: Option<GlyphCell>,
    /// A pick in a cell's alternates popup.
    pub alternate: Option<AlternatePick>,
    /// The family or style changed in the panel's own pickers.
    pub font_changed: bool,
}

/// The cell whose popup was used and the picked glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AlternatePick {
    /// The cell whose alternates popup was used.
    pub cell: GlyphCell,
    /// The picked glyph.
    pub glyph: GlyphCell,
}

/// The label, tooltip and state of the cell's corner action; the panel takes it in its own scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "glyphs", scope = "glyphs")]
#[non_exhaustive]
pub enum GlyphsCommand {
    /// Opens the cell's alternates popup.
    #[command(label = "Show Alternates")]
    Alternates,
}

/// The one way to paint a glyph cell (thumbnail cache, inset, tint): the panel's grid, recents and popup, and an
/// app's own glyph cells. The painter owns a clone of `ctx` and copies of `family` and `style`, so it borrows only
/// `source`.
pub fn glyph_painter<'s>(
    ctx: &egui::Context,
    source: &'s dyn GlyphSource,
    family: &str,
    style: &str,
    cell: GlyphCell,
) -> impl Fn(&ContentPainterCx, Rect) + use<'s> {
    let thumbs = Thumbnails::<GlyphKey>::in_context(ctx, "craft-font-manager-glyphs", ThumbnailOptions::new());
    let ctx = ctx.clone();
    let key = GlyphKey { family: family.to_owned(), style: style.to_owned(), cell };
    move |cx: &ContentPainterCx<'_>, rect: Rect| {
        let rect = rect.shrink(rect.width().min(rect.height()) * GLYPH_INSET_RATIO);
        let request = ThumbRequest::for_cell(rect.width().max(rect.height()), ctx.pixels_per_point());
        let picture = thumbs.state(&ctx, &GlyphPictures(source), &key, &request, Priority::VISIBLE);
        let ink = DataColor::new(cx.ink());
        thumbs.paint(cx, rect, &picture, &ThumbPaint::default().checker(false).tint(ink));
    }
}

impl Outcome for GlyphsPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// How many recent glyphs are kept (Photoshop).
const RECENT_LIMIT: usize = 25;
/// The cell size of the grid and its zoom: factors of the medium thumbnail.
pub const ZOOM: TileZoom = TileZoom::bounded(1.0, 2.625, 0.125, 1.25);
/// The cells of the recents strip, as a factor of the medium thumbnail.
const RECENT_ZOOM: f64 = 0.75;
/// The room round a glyph in its cell, as a fraction of the cell.
const GLYPH_INSET_RATIO: f32 = 0.1;
/// The most columns the alternates popup lays its cells in.
const ALTERNATE_COLUMNS: usize = 8;
/// The icon of the cell's corner action.
const ALTERNATES_ICON: &str = "more";

/// A glyph picture's key: the face and the cell.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct GlyphKey {
    family: String,
    style: String,
    cell: GlyphCell,
}

/// The glyph pictures of a [`GlyphSource`].
struct GlyphPictures<'a>(&'a dyn GlyphSource);

impl ThumbSource<GlyphKey> for GlyphPictures<'_> {
    fn revision(&self, _: &GlyphKey, _: &ThumbRequest) -> Revision {
        Revision::new(self.0.revision())
    }

    fn render(&self, key: &GlyphKey, request: &ThumbRequest) -> ThumbRender {
        match self.0.glyph(&key.family, &key.style, key.cell, request.side) {
            Some(image) => ThumbRender::Ready(image),
            None => ThumbRender::Blank,
        }
    }
}

/// The cells of a face in a category, kept until the face or the category changes.
#[derive(Clone, Default)]
struct CellsCache {
    key: (String, String, String),
    revision: u64,
    cells: Arc<Vec<GlyphCell>>,
}

/// The tile selection of the main grid.
#[derive(Clone, Default)]
struct Marked(Selection<usize>);

/// The alternates popup: whether it is open, the cell it was opened for and that cell's alternates, read once when
/// it opened.
#[derive(Clone, Default)]
struct Popup {
    open: bool,
    cell: Option<GlyphCell>,
    cells: Vec<GlyphCell>,
}

fn code(c: char) -> String {
    format!("{:04X}", c as u32)
}

/// The recents as `Char` cells, each once, most recent first.
fn recent_cells(recent: &[String]) -> Vec<GlyphCell> {
    let mut seen = Vec::new();
    for c in recent.iter().filter_map(|g| g.chars().next()) {
        if !seen.contains(&c) {
            seen.push(c);
        }
    }
    seen.into_iter().map(GlyphCell::Char).collect()
}

/// The cells of the shown set: the recents, or the source's cells of the category.
fn category_cells(
    cache: &mut CellsCache,
    source: &dyn GlyphSource,
    family: &str,
    style: &str,
    category: &str,
    recent: &[String],
) -> Arc<Vec<GlyphCell>> {
    if category == "Recently Used" {
        return Arc::new(recent_cells(recent));
    }
    let key = (family.to_owned(), style.to_owned(), category.to_owned());
    let revision = source.revision();
    if cache.key != key || cache.revision != revision {
        *cache = CellsCache { key, revision, cells: Arc::new(source.cells(family, style, category)) };
    }
    Arc::clone(&cache.cells)
}

/// The cells as a tile source: a cell's position is its key.
struct Cells<'a>(&'a [GlyphCell]);

impl craft_behavior::DisplayOrder<usize> for Cells<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }
}

impl craft_behavior::ItemSource<usize> for Cells<'_> {}

/// The tile of one glyph cell: its name for assistive tech and the glyph picture.
fn cell_tile<'s>(
    ctx: &Context,
    source: &'s dyn GlyphSource,
    words: &GlyphsWords,
    (family, style): (&str, &str),
    cell: GlyphCell,
    info: &CellInfo,
) -> Tile<'s> {
    let name = match cell {
        GlyphCell::Char(c) => words.cell.fill(&[("char", &c.to_string()), ("code", &code(c))]),
        GlyphCell::Glyph(id) => {
            let gid = id.to_string();
            words.glyph_cell.fill(&[("name", info.name.as_deref().unwrap_or(gid.as_str())), ("gid", gid.as_str())])
        }
    };
    Tile::new(name).thumb(ThumbAspect::SQUARE, glyph_painter(ctx, source, family, style, cell))
}

/// The Glyphs panel body. The family comes from the text selection while editing (the panel follows the caret),
/// else from the panel's own picker.
///
/// - Keys: the grid is one Tab stop; arrows move the focused cell, Page Up/Down a screen, Home/End to the ends,
///   Enter inserts. Hovering a cell shows its character and code point as a tooltip.
/// - A cell is drawn from [`GlyphSource::glyph`] at the zoom size, cached per face and cell. The grid shows
///   only the current category; a cell the source returns no image for stays empty.
/// - A cell the source says has alternates shows a corner action that opens a popup of them; a pick is reported
///   as [`GlyphsPanelOutput::alternate`].
/// - Where the app cannot insert, the cells still show, Enter and double-click do nothing and the footer says so.
/// - While the source is not [ready](GlyphSource::ready) the panel shows a spinner and "Loading fonts…" only.
/// - The font and style lists come from the app ([`families`](GlyphsPanel::families),
///   [`styles`](GlyphsPanel::styles)); without a list the name is shown as text.
#[must_use = "show the panel with `show`"]
#[derive(Pattern, Props)]
pub struct GlyphsPanel<'a> {
    label: Label,
    source: &'a dyn GlyphSource,
    family: Bind<'a, String>,
    style: Bind<'a, String>,
    settings: Bind<'a, GlyphsPanelSettings>,
    /// The families the font list offers.
    #[prop]
    families: &'a [String],
    /// The styles of the family, for the style list.
    #[prop]
    styles: &'a [String],
    /// Whether the app has a place to insert a glyph: a double click or Enter inserts.
    #[prop]
    can_insert: bool,
    /// The host's cell (the selected character); the grid selects it.
    #[prop]
    marked: Option<GlyphCell>,
    /// In this pass only, the grid moves its lead to that cell and scrolls it into view. The host passes it in the
    /// pass its marked cell changes.
    #[prop]
    reveal: Option<GlyphCell>,
    /// Shown in place of the grid when the category has no cells.
    #[prop(into)]
    empty: Option<Label>,
}

impl<'a> GlyphsPanel<'a> {
    /// A panel named `label` over `source`, editing the face `family` and `style` and remembering `settings`.
    pub fn new(
        label: impl Into<Label>,
        source: &'a dyn GlyphSource,
        family: Bind<'a, String>,
        style: Bind<'a, String>,
        settings: Bind<'a, GlyphsPanelSettings>,
    ) -> Self {
        GlyphsPanel {
            label: label.into(),
            source,
            family,
            style,
            settings,
            families: &[],
            styles: &[],
            can_insert: false,
            marked: None,
            reveal: None,
            empty: None,
        }
    }

    /// Shows the panel.
    pub fn show(self, ui: &mut Ui) -> GlyphsPanelOutput {
        let GlyphsPanel {
            label,
            source,
            mut family,
            mut style,
            mut settings,
            families,
            styles,
            can_insert,
            marked: marked_cell,
            reveal: reveal_cell,
            empty,
        } = self;
        let env = Env::of(ui);
        let words = env.words::<GlyphsWords>();
        if !source.ready() {
            let shown = Stack::new().label(label).show(ui, |ui, _| {
                Row::new().show(ui, |ui, _| {
                    Spinner::new().label(words.loading.clone()).show(ui);
                    Text::new(words.loading.clone()).shade(Shade::Muted).show(ui);
                });
            });
            craft_ui::runtime::frame::request_repaint_after(ui.ctx(), SCAN_POLL_SECONDS);
            return GlyphsPanelOutput { response: shown.output, activated: None, alternate: None, font_changed: false };
        }
        let ctx = ui.ctx().clone();
        let mut activated_out: Option<GlyphCell> = None;
        let mut picked: Option<AlternatePick> = None;
        let mut font_changed = false;
        let shown = Stack::new().label(label).command_scope(GlyphsCommand::SCOPE).show(ui, |ui, cx| {
            cx.publish(GlyphsCommand::Alternates, CommandState::new());
            let mut state = read_bind(&mut settings, cx).unwrap_or_default();
            if state.category.is_empty() {
                state.category = CATEGORIES[0].to_owned();
            }
            let zoom = ZOOM.range();
            let size = if state.zoom.is_finite() && state.zoom > 0.0 {
                f64::from(state.zoom).clamp(*zoom.start(), *zoom.end())
            } else {
                ZOOM.default_value()
            };
            let mut zoom_value = size;
            let (held_family, held_style) = (read_bind(&mut family, cx).unwrap_or_default(), read_bind(&mut style, cx).unwrap_or_default());
            let (mut face_family, mut face_style) = (held_family.clone(), held_style.clone());

            Row::new().show(ui, |ui, _| {
                if families.is_empty() {
                    Text::new(face_family.clone()).show(ui);
                } else {
                    let entries: Vec<Entry<String>> = families.iter().map(|name| Entry::new(name.clone(), name.clone())).collect();
                    Select::new(words.font.clone(), Bind::bound(&mut face_family), &entries).searchable().show(ui);
                }
                if styles.is_empty() {
                    Text::new(face_style.clone()).show(ui);
                } else {
                    let entries: Vec<Entry<String>> = styles.iter().map(|name| Entry::new(name.clone(), name.clone())).collect();
                    Select::new(words.style.clone(), Bind::bound(&mut face_style), &entries).show(ui);
                }
            });
            if face_family != held_family {
                write_bind(&mut family, cx, face_family.clone(), "Glyphs font");
            }
            if face_style != held_style {
                write_bind(&mut style, cx, face_style.clone(), "Glyphs style");
            }
            font_changed = face_family != held_family || face_style != held_style;
            let face = (face_family.as_str(), face_style.as_str());

            // A category the face no longer offers shows the whole font.
            let extra = source.categories(face.0, face.1);
            if !CATEGORIES.contains(&state.category.as_str()) && !extra.iter().any(|(id, _)| *id == state.category) {
                state.category = CATEGORIES[0].to_owned();
            }
            let mut category = state.category.clone();
            Row::new().show(ui, |ui, _| {
                Text::new(words.show.clone()).show(ui);
                let entries: Vec<Entry<String>> = CATEGORIES
                    .iter()
                    .map(|name| Entry::new((*name).to_owned(), words.category_name(name).to_owned()))
                    .chain(extra.iter().map(|(id, name)| Entry::new(id.clone(), name.clone())))
                    .collect();
                Select::new(words.category.clone(), Bind::bound(&mut category), &entries).show(ui);
            });
            state.category = category;

            // Recently used.
            Text::new(words.recent.clone()).show(ui);
            let recent = recent_cells(&state.recent);
            let plain = CellInfo::default();
            let mut activated: Option<GlyphCell> = None;
            let strip = TileGrid::<usize>::new(words.recent.clone())
                .thumb_size(ThumbSize::Medium)
                .zoom(GridZoom::Fixed(RECENT_ZOOM))
                .caption(CaptionPlace::None)
                .select(SelectMode::Single)
                .activate(ActivateOn::Click)
                .sizing(craft_ui::contract::Sizing::FillWidth)
                .surface(SurfaceKind::Sunken)
                .pad(Pad::Control)
                .show_items(ui, &Cells(&recent), &mut |key, _| cell_tile(&ctx, source, &words, face, recent[*key], &plain))
                .response
                .events;
            for event in strip {
                if let CollectionEvent::Activate(key) = event
                    && let Some(&cell) = recent.get(key)
                {
                    activated = Some(cell);
                }
            }

            // The grid.
            let cells = category_cells(cx.memory::<CellsCache>(), source, face.0, face.1, &state.category, &state.recent);
            let index_of = |cell: GlyphCell| cells.iter().position(|c| *c == cell);
            let mut marked = cx.memory::<Marked>().clone();
            if let Some(at) = marked_cell.and_then(index_of)
                && marked.0.lead != Some(at)
            {
                marked.0 = Selection::single(at);
            }
            match &empty {
                Some(text) if cells.is_empty() => {
                    Text::new(text.clone()).shade(Shade::Muted).show(ui);
                }
                _ => {
                    let mut grid = TileGrid::<usize>::new(words.grid.clone())
                        .thumb_size(ThumbSize::Medium)
                        .zoom(GridZoom::Fixed(size))
                        .caption(CaptionPlace::None)
                        .select(SelectMode::Single)
                        .selection(Bind::bound(&mut marked.0));
                    if let Some(at) = reveal_cell.and_then(index_of) {
                        grid = grid.reveal(at);
                    }
                    let shown_grid = grid.show_items(ui, &Cells(&cells), &mut |key, _| {
                        let cell = cells[*key];
                        let info = source.describe(face.0, face.1, cell);
                        let mut tile = cell_tile(&ctx, source, &words, face, cell, &info);
                        if info.dimmed {
                            tile = tile.dimmed();
                        }
                        if info.has_alternates {
                            tile = tile.action(IconId::new(ALTERNATES_ICON), GlyphsCommand::Alternates);
                        }
                        match info.note {
                            Some(note) => tile.tooltip(note),
                            None => tile,
                        }
                    });
                    for event in shown_grid.response.events {
                        match event {
                            CollectionEvent::Activate(key) => activated = cells.get(key).copied().or(activated),
                            CollectionEvent::Item(key, TilePartEvent::Action(0)) => {
                                if let Some(&cell) = cells.get(key) {
                                    *cx.memory::<Popup>() = Popup { open: true, cell: Some(cell), cells: source.alternates(face.0, face.1, cell) };
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }

            // The alternates popup, hung from the pointer where the corner action was pressed.
            let mut popup = std::mem::take(cx.memory::<Popup>());
            if let Some(cell) = popup.cell
                && popup.open
            {
                Popover::new(words.alternates.clone(), Bind::bound(&mut popup.open)).at_pointer().show(ui, |ui, body| {
                    let events = TileGrid::<usize>::new(words.alternates.clone())
                        .thumb_size(ThumbSize::Medium)
                        .caption(CaptionPlace::None)
                        .select(SelectMode::Single)
                        .activate(ActivateOn::Click)
                        .card_columns(popup.cells.len().min(ALTERNATE_COLUMNS))
                        .natural_height(true)
                        .show_items(ui, &Cells(&popup.cells), &mut |key, _| {
                            let alternate = popup.cells[*key];
                            let info = source.describe(face.0, face.1, alternate);
                            cell_tile(&ctx, source, &words, face, alternate, &info)
                        })
                        .response
                        .events;
                    for event in events {
                        if let CollectionEvent::Activate(key) = event
                            && let Some(&glyph) = popup.cells.get(key)
                        {
                            picked = Some(AlternatePick { cell, glyph });
                            body.request_close();
                        }
                    }
                });
            }
            if !popup.open || picked.is_some() {
                popup = Popup::default();
            }
            *cx.memory::<Popup>() = popup;

            // Footer: the selection, the count and the zoom.
            let n = cells.len() as u64;
            let count = words.count.text(n, &env.format().integer(n));
            let info = match marked.0.lead.and_then(|lead| cells.get(lead)).copied() {
                Some(GlyphCell::Char(c)) => match source.describe(face.0, face.1, GlyphCell::Char(c)).gid {
                    Some(gid) => words.selected_glyph.fill(&[("code", &code(c)), ("gid", &gid.to_string()), ("count", &count)]),
                    None => words.selected.fill(&[("code", &code(c)), ("count", &count)]),
                },
                Some(GlyphCell::Glyph(gid)) => words.unencoded.fill(&[("gid", &gid.to_string()), ("count", &count)]),
                None => count,
            };
            *cx.memory::<Marked>() = marked;
            Row::new().show(ui, |ui, _| {
                Text::new(info.clone()).show(ui);
                ThumbnailSlider::new(words.zoom.clone(), Bind::bound(&mut zoom_value), ZOOM).show(ui);
            });
            if !can_insert {
                Text::new(words.no_text.clone()).show(ui);
            }

            state.zoom = zoom_value as f32;
            if let Some(cell) = activated
                && can_insert
            {
                if let GlyphCell::Char(c) = cell {
                    let glyph = c.to_string();
                    state.recent.retain(|g| *g != glyph);
                    state.recent.insert(0, glyph);
                    state.recent.truncate(RECENT_LIMIT);
                }
                activated_out = Some(cell);
            }
            if read_bind(&mut settings, cx).is_none_or(|held| held != state) {
                write_bind(&mut settings, cx, state, "Glyphs panel");
            }
        });
        GlyphsPanelOutput { response: shown.output, activated: activated_out, alternate: picked, font_changed }
    }
}
