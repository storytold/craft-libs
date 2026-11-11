//! [`GlyphsPanel`]: Photoshop's Glyphs panel. A family and style picker, a category list (Entire Font, Punctuation,
//! Currency, ...), a grid of glyph cells drawn by the app's text engine, a zoom slider, and the recently used
//! glyphs in a row at the top. Double-click or Enter inserts the glyph into the text being edited.

#[cfg(feature = "screen-testing")]
mod stories;

use std::sync::Arc;

use craft_behavior::Selection;
use craft_host::Section;
use craft_ui::contract::{
    ActivateOn, Bind, CollectionEvent, Entry, NoValue, Outcome, Pattern, Props, Remember as _, SelectMode, Show as _, ShowCollection as _,
    ShowContainer as _,
};
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::ThumbAspect;
use craft_ui::list::ThumbSize;
use craft_ui::pickers::Select;
use craft_ui::primitives::ContentPainterCx;
use craft_ui::primitives::textures::{Priority, Revision, ThumbPaint, ThumbRender, ThumbRequest, ThumbSource, ThumbnailOptions, Thumbnails};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::DataColor;
use craft_ui::tile_grid::{CaptionPlace, ThumbnailSlider, Tile, TileGrid, TileZoom};
use egui::{ColorImage, Rect, Response, Ui};
use serde::{Deserialize, Serialize};

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
        font: "glyphs.font" = "Font";
        category: "glyphs.category" = "Category";
        grid: "glyphs.grid" = "Glyphs";
        count: count "glyphs.count" = "{n} glyph" | "{n} glyphs";
        /// `{code}` the lead glyph's code point, `{count}` the count word.
        selected: template "glyphs.selected" = "U+{code}  ·  {count}";
        /// `{char}` the glyph, `{code}` its code point.
        cell: template "glyphs.cell" = "{char}, U+{code}";
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
/// and passed to [`GlyphSource::in_category`]; the panel shows them through [`GlyphsWords::category_name`].
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

/// Where glyph cells come from: the app's text engine draws a glyph of a face at a pixel size and lists the
/// characters a face has. Apps plug in their own.
pub trait GlyphSource: Send + Sync {
    /// Every character the face (family, style) has a glyph for, in code point order.
    fn charmap(&self, family: &str, style: &str) -> Vec<char>;
    /// The glyph of `c` in the face, `px` pixels tall, as an image for a texture; `None` when the face lacks it.
    /// The panel tints the image with the text colour, so a glyph is white with its coverage in the alpha channel.
    fn glyph(&self, family: &str, style: &str, c: char, px: u32) -> Option<ColorImage>;
    /// Whether `c` is in `category` (one of [`CATEGORIES`]).
    fn in_category(&self, c: char, category: &str) -> bool;
}

/// The panel's remembered state (`[glyphs_panel]`): category, zoom and the recent glyphs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct GlyphsPanelSettings {
    /// The chosen category, one of [`CATEGORIES`].
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
    /// A glyph to insert at the caret (double-click, Enter on the focused cell); only while text is edited.
    pub insert: Option<String>,
    /// The family or style changed in the panel's own pickers.
    pub font_changed: bool,
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
pub const ZOOM: TileZoom = TileZoom::bounded(0.75, 2.0, 0.125, 1.0);
/// The cells of the recents strip, as a factor of the medium thumbnail.
const RECENT_ZOOM: f32 = 0.75;
/// The room round a glyph in its cell, as a fraction of the cell.
const GLYPH_INSET_RATIO: f32 = 0.1;

/// A glyph picture's key: the face and the character.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct GlyphKey {
    family: String,
    style: String,
    c: char,
}

/// The glyph pictures of a [`GlyphSource`].
struct GlyphPictures<'a>(&'a dyn GlyphSource);

impl ThumbSource<GlyphKey> for GlyphPictures<'_> {
    fn revision(&self, _: &GlyphKey, _: &ThumbRequest) -> Revision {
        Revision::new(0)
    }

    fn render(&self, key: &GlyphKey, request: &ThumbRequest) -> ThumbRender {
        match self.0.glyph(&key.family, &key.style, key.c, request.side) {
            Some(image) => ThumbRender::Ready(image),
            None => ThumbRender::Blank,
        }
    }
}

/// The characters of a face in a category, kept until the face or the category changes.
#[derive(Clone, Default)]
struct CharsCache {
    key: (String, String, String),
    chars: Arc<Vec<char>>,
}

/// The tile selection of the main grid.
#[derive(Clone, Default)]
struct Marked(Selection<usize>);

fn code(c: char) -> String {
    format!("{:04X}", c as u32)
}

/// The characters of the shown set: the recents, or the face's charmap narrowed by the category.
fn category_chars(cache: &mut CharsCache, source: &dyn GlyphSource, family: &str, style: &str, category: &str, recent: &[String]) -> Arc<Vec<char>> {
    if category == "Recently Used" {
        let mut seen = Vec::new();
        for c in recent.iter().filter_map(|g| g.chars().next()) {
            if !seen.contains(&c) {
                seen.push(c);
            }
        }
        return Arc::new(seen);
    }
    let key = (family.to_owned(), style.to_owned(), category.to_owned());
    if cache.key != key {
        let charmap = source.charmap(family, style);
        let chars = if category == CATEGORIES[0] { charmap } else { charmap.into_iter().filter(|c| source.in_category(*c, category)).collect() };
        *cache = CharsCache { key, chars: Arc::new(chars) };
    }
    Arc::clone(&cache.chars)
}

/// The characters as a tile source: a character's position is its key.
struct Chars<'a>(&'a [char]);

impl craft_behavior::DisplayOrder<usize> for Chars<'_> {
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

impl craft_behavior::ItemSource<usize> for Chars<'_> {}

/// The Glyphs panel body. The family comes from the text selection while editing (the panel follows the caret),
/// else from the panel's own picker.
///
/// - Keys: the grid is one Tab stop; arrows move the focused cell, Page Up/Down a screen, Home/End to the ends,
///   Enter inserts. Hovering a cell shows its character and code point as a tooltip.
/// - A cell is drawn from [`GlyphSource::glyph`] at the zoom size, cached per face and character. The grid shows
///   only the current category; a cell the source returns no image for stays empty.
/// - Without text being edited the cells still show, Enter and double-click do nothing and the footer says so.
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
    /// Whether text is being edited (inserting is possible).
    #[prop]
    editing: bool,
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
        GlyphsPanel { label: label.into(), source, family, style, settings, families: &[], styles: &[], editing: false }
    }

    /// Shows the panel.
    pub fn show(self, ui: &mut Ui) -> GlyphsPanelOutput {
        let GlyphsPanel { label, source, mut family, mut style, mut settings, families, styles, editing } = self;
        let env = Env::of(ui);
        let words = env.words::<GlyphsWords>();
        let thumbs = Thumbnails::<GlyphKey>::in_context(ui.ctx(), "craft-font-manager-glyphs", ThumbnailOptions::new());
        let pictures = GlyphPictures(source);
        let mut insert: Option<String> = None;
        let mut font_changed = false;
        let shown = Stack::new().label(label).show(ui, |ui, cx| {
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

            let mut category = state.category.clone();
            Row::new().show(ui, |ui, _| {
                Text::new(words.show.clone()).show(ui);
                let entries: Vec<Entry<String>> =
                    CATEGORIES.iter().map(|name| Entry::new((*name).to_owned(), words.category_name(name).to_owned())).collect();
                Select::new(words.category.clone(), Bind::bound(&mut category), &entries).show(ui);
            });
            state.category = category;

            let paint = |family: &str, style: &str, c: char| {
                let (thumbs, pictures) = (thumbs.clone(), &pictures);
                let key = GlyphKey { family: family.to_owned(), style: style.to_owned(), c };
                move |cx: &ContentPainterCx<'_>, rect: Rect| {
                    let ctx = cx.painter.ctx().clone();
                    let rect = rect.shrink(rect.width().min(rect.height()) * GLYPH_INSET_RATIO);
                    let request = ThumbRequest::for_cell(rect.width().max(rect.height()), ctx.pixels_per_point());
                    let picture = thumbs.state(&ctx, pictures, &key, &request, Priority::VISIBLE);
                    let ink = DataColor::new(cx.ink());
                    thumbs.paint(cx, rect, &picture, &ThumbPaint::default().checker(false).tint(ink));
                }
            };

            // Recently used.
            Text::new(words.recent.clone()).show(ui);
            let recent: Vec<char> = state.recent.iter().filter_map(|g| g.chars().next()).collect();
            let mut activated: Option<String> = None;
            let strip = TileGrid::<usize>::new(words.recent.clone())
                .thumb_size(ThumbSize::Medium)
                .zoom(RECENT_ZOOM)
                .caption(CaptionPlace::None)
                .select(SelectMode::Single)
                .activate(ActivateOn::Click)
                .sizing(craft_ui::contract::Sizing::FillWidth)
                .show_items(ui, &Chars(&recent), &mut |key, _| {
                    let c = recent[*key];
                    let name = words.cell.fill(&[("char", &c.to_string()), ("code", &code(c))]);
                    Tile::new(name).thumb(ThumbAspect::SQUARE, paint(&face_family, &face_style, c))
                });
            for event in strip.response.events {
                if let CollectionEvent::Activate(key) = event
                    && let Some(c) = recent.get(key)
                {
                    activated = Some(c.to_string());
                }
            }

            // The grid.
            let chars = category_chars(cx.memory::<CharsCache>(), source, &face_family, &face_style, &state.category, &state.recent);
            let mut marked = cx.memory::<Marked>().clone();
            let grid = TileGrid::<usize>::new(words.grid.clone())
                .thumb_size(ThumbSize::Medium)
                .zoom(size as f32)
                .caption(CaptionPlace::None)
                .select(SelectMode::Single)
                .selection(Bind::bound(&mut marked.0))
                .show_items(ui, &Chars(&chars), &mut |key, _| {
                    let c = chars[*key];
                    let name = words.cell.fill(&[("char", &c.to_string()), ("code", &code(c))]);
                    Tile::new(name).thumb(ThumbAspect::SQUARE, paint(&face_family, &face_style, c))
                });
            for event in grid.response.events {
                if let CollectionEvent::Activate(key) = event
                    && let Some(c) = chars.get(key)
                {
                    activated = Some(c.to_string());
                }
            }

            // Footer: the selection, the count and the zoom.
            let n = chars.len() as u64;
            let count = words.count.text(n, &env.format().integer(n));
            let info = match marked.0.lead.and_then(|lead| chars.get(lead)) {
                Some(&c) => words.selected.fill(&[("code", &code(c)), ("count", &count)]),
                None => count,
            };
            *cx.memory::<Marked>() = marked;
            Row::new().show(ui, |ui, _| {
                Text::new(info.clone()).show(ui);
                ThumbnailSlider::new(words.zoom.clone(), Bind::bound(&mut zoom_value), ZOOM).show(ui);
            });
            if !editing {
                Text::new(words.no_text.clone()).show(ui);
            }

            state.zoom = zoom_value as f32;
            if let Some(glyph) = activated
                && editing
            {
                state.recent.retain(|g| *g != glyph);
                state.recent.insert(0, glyph.clone());
                state.recent.truncate(RECENT_LIMIT);
                insert = Some(glyph);
            }
            if read_bind(&mut settings, cx).is_none_or(|held| held != state) {
                write_bind(&mut settings, cx, state, "Glyphs panel");
            }
        });
        GlyphsPanelOutput { response: shown.output, insert, font_changed }
    }
}
