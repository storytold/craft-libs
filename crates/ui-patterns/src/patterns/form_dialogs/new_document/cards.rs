//! The left column of New Document: the category tabs over the grid of preset cards, or the list of saved presets.

use super::presets::{DEFAULT_ID, DocumentOptions, DocumentPreset, NewDocumentPresetWords, PresetCategory, size_line};
use super::settings::{CardCx, CardSource, DocumentCard, DocumentSettings, RasterSetup};
use super::{NewDocumentWords, Read, Session, background_label, depth_label, mode_label};
use crate::buttons::{Button, ButtonOutput};
use crate::contract::{Bind, CollectionEvent, MemoProps, Props as _, SelectMode, Show as _, ShowCollection as _, clicked};
use crate::display::{Shade, Text};
use crate::list::{List, ListRow, ThumbAspect};
use crate::patterns::presets::{PresetId, PresetLibrary};
use crate::primitives::{ContentPainterCx, StrokeWidth};
use crate::runtime::env::{Env, EnvValue, Label, UnitKind};
use crate::style::TypeRole;
use crate::tabs::{Tab, TabKind, TabStrip};
use crate::tile_grid::{CardText, GridZoom, Tile, TileGrid};
use craft_behavior::{DisplayOrder, ItemSource, Selection, SelectionChange, keyed};
use egui::{CornerRadius, Rect, Stroke, StrokeKind, Ui, vec2};
use std::borrow::Cow;
use std::cell::Cell;

/// A tab of the preset browser.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum TabId {
    /// Recent documents and the app's default sizes.
    #[default]
    Recent,
    /// The category at this position of the dialog's list.
    Category(usize),
    /// The presets the user saved.
    Saved,
}

impl TabId {
    /// The id of the `tab` binding: `"recent"`, the category's id, or `"saved"`.
    pub(super) fn key<P>(self, categories: &[PresetCategory<P>]) -> String {
        match self {
            TabId::Recent => RECENT_KEY.to_owned(),
            TabId::Saved => SAVED_KEY.to_owned(),
            TabId::Category(at) => categories.get(at).map_or_else(|| RECENT_KEY.to_owned(), |category| category.id.to_string()),
        }
    }

    /// The tab a [`key`](Self::key) names; `None` for an id no tab has.
    pub(super) fn from_key<P>(key: &str, categories: &[PresetCategory<P>]) -> Option<TabId> {
        match key {
            RECENT_KEY => Some(TabId::Recent),
            SAVED_KEY => Some(TabId::Saved),
            _ => categories.iter().position(|category| category.id == key).map(TabId::Category),
        }
    }
}

const RECENT_KEY: &str = "recent";
const SAVED_KEY: &str = "saved";

/// The card the user picked in this pass.
pub(super) struct Picked<S> {
    pub(super) card: DocumentCard<S>,
    pub(super) index: usize,
    /// Enter or a double click: create the document from the card.
    pub(super) activated: bool,
}

/// What the left column reports.
#[derive(Default)]
pub(super) struct Left<S> {
    pub(super) tab: Option<TabId>,
    pub(super) picked: Option<Picked<S>>,
    /// The saved presets the Delete intent removed.
    pub(super) delete: Vec<PresetId>,
}

/// The cards as a display order keyed by position.
struct CardList<'a, S>(&'a [DocumentCard<S>]);

impl<S: DocumentSettings> DisplayOrder<usize> for CardList<'_, S> {
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
        self.0.get(at).map(|card| Cow::Borrowed(card.title.as_str()))
    }
}

impl<S: DocumentSettings> ItemSource<usize> for CardList<'_, S> {}

/// The columns of preset cards the grid holds: the dialog's left side is as wide as they are.
const CARD_COLUMNS: usize = 3;

/// The columns of Recent's cards with `recent_items`.
const RECENT_COLUMNS: usize = 4;

/// The picture size of Recent's cards with `recent_items`, against a preset card's: four of them take about the room
/// three preset cards do.
const RECENT_ZOOM: f64 = 0.75;

/// What the cards of the open tab are built from.
fn card_cx<'a, S: DocumentSettings>(session: &Session<S>, values: &'a S, read: &Read<'a, S>) -> CardCx<'a, S> {
    CardCx {
        values,
        setup: read.setup,
        orientation: session.orientation,
        recents: read.recents,
        presets: read.categories,
        units: read.units,
        words: read.words,
        preset_words: read.preset_words,
    }
}

/// The position of the clipboard's card in Recent: the dialog opens on it.
pub(super) fn clipboard_at<S: DocumentSettings>(session: &Session<S>, values: &S, read: &Read<'_, S>) -> Option<usize> {
    S::recent_cards(&card_cx(session, values, read)).iter().position(|card| card.source == CardSource::Clipboard)
}

/// Shows the full-width category strip above the two columns.
pub(super) fn tabs<S: DocumentSettings>(ui: &mut Ui, session: &Session<S>, read: &Read<'_, S>, left: &mut Left<S>) {
    let words = read.words;
    let tabs: Vec<TabId> =
        std::iter::once(TabId::Recent).chain((0..read.categories.len()).map(TabId::Category)).chain(std::iter::once(TabId::Saved)).collect();
    let source = keyed(&tabs, |tab| *tab);
    let strip = TabStrip::new(words.tabs.clone())
        .kind(TabKind::Text)
        .shown(session.tab)
        .show_items(ui, &source, &mut |tab, _| Tab::new(tab_title(*tab, read.categories, words)));
    for event in strip.response.events {
        if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
            && let Some(tab) = keys.last()
        {
            left.tab = Some(*tab);
        }
    }
}

/// Shows the cards or saved presets in the category picked in this pass.
pub(super) fn show<S: DocumentSettings>(ui: &mut Ui, session: &Session<S>, values: &S, read: &Read<'_, S>, left: &mut Left<S>) {
    let words = read.words;
    let tab = left.tab.unwrap_or(session.tab);
    let cards = cards_of(tab, read.library, read.recent_items, &card_cx(session, values, read));
    let selected = if tab == session.tab { selected_of(tab, session, values, &cards) } else { None };
    let selection = selected.map_or_else(|| Selection::new(Vec::new(), None, None), Selection::single);
    let stamp = stamp_of(tab, cards.len());
    let list = CardList(&cards);
    if tab == TabId::Saved {
        // Each row's Delete button removes its preset, as the Delete key removes the selected ones.
        let pressed: Cell<Option<usize>> = Cell::new(None);
        let shown = List::<usize>::new(words.saved_list.clone())
            .select(SelectMode::Single)
            .deletable(true)
            .empty(words.no_saved.clone())
            .rows_stamp(stamp)
            .selection(Bind::given(selection))
            .show_items(ui, &list, &mut |index, _| {
                let at = *index;
                let pressed = &pressed;
                ListRow::new(cards[at].title.clone()).detail(cards[at].line.clone()).trailing_field(
                    Button::new(words.delete.clone()).id_salt(("delete", at)),
                    move |button: ButtonOutput| {
                        if clicked(&button) {
                            pressed.set(Some(at));
                        }
                    },
                )
            });
        route(shown.events, &cards, left);
        if let Some(card) = pressed.get().and_then(|at| cards.get(at)) {
            left.delete.extend(saved_id(card));
        }
    } else {
        let count = read.units.number(cards.len() as f64, 0..=0);
        let items = tab == TabId::Recent && read.recent_items;
        let heading = Label::from(if items { &words.recent_heading } else { &words.presets_heading }.fill(&[("n", &count)]));
        Text::new(heading).role(TypeRole::SectionCaption).shade(Shade::Muted).show(ui);
        let look = Env::of(ui).get::<PresetCardLook>();
        let mut grid = TileGrid::<usize>::new(words.cards.clone())
            .card_columns(CARD_COLUMNS)
            .card()
            .card_text(match look {
                PresetCardLook::Outline => CardText::Preset,
                PresetCardLook::Page => CardText::Page,
            })
            .fill_row()
            .aspect(2.0);
        if items {
            // Four smaller cards to a row, about the room three preset cards take.
            grid = grid.card_columns(RECENT_COLUMNS).zoom(GridZoom::Fixed(RECENT_ZOOM));
        }
        let shown = grid.select(SelectMode::Single).detail_line().rows_stamp(stamp).selection(Bind::given(selection)).show_items(
            ui,
            &list,
            &mut |index, _| tile_of(&cards[*index], look),
        );
        route(shown.response.events, &cards, left);
    }
}

/// The library preset a Saved row stands for.
fn saved_id<S>(card: &DocumentCard<S>) -> Option<PresetId> {
    match &card.source {
        CardSource::Saved(id) => Some(*id),
        _ => None,
    }
}

/// Turns a collection's events into the card picked: a selection change picks, Enter or a double click activates.
fn route<E, S: DocumentSettings>(events: Vec<CollectionEvent<usize, E>>, cards: &[DocumentCard<S>], left: &mut Left<S>) {
    for event in events {
        match event {
            CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } => {
                if let Some(index) = keys.last() {
                    pick(left, cards, *index, false);
                }
            }
            CollectionEvent::Activate(index) => pick(left, cards, index, true),
            CollectionEvent::Delete(keys) => left.delete.extend(keys.iter().filter_map(|index| cards.get(*index).and_then(saved_id))),
            _ => {}
        }
    }
}

/// Records the card at `index` as the one picked this pass; an activation stays once it was seen.
fn pick<S: DocumentSettings>(left: &mut Left<S>, cards: &[DocumentCard<S>], index: usize, activated: bool) {
    let Some(card) = cards.get(index) else { return };
    let activated = activated || left.picked.as_ref().is_some_and(|picked| picked.index == index && picked.activated);
    left.picked = Some(Picked { card: card.clone(), index, activated });
}

/// The position the grid shows selected: the card picked in the tab, else the recent document the details show.
fn selected_of<S: DocumentSettings>(tab: TabId, session: &Session<S>, values: &S, cards: &[DocumentCard<S>]) -> Option<usize> {
    match session.card {
        Some(at) => (at < cards.len()).then_some(at),
        None if tab == TabId::Recent => cards.iter().position(|card| card.source == CardSource::Recent && card.settings.same_document(values)),
        None => None,
    }
}

/// A number that differs for each tab and card count, so the lead and selection start over with the listed cards.
fn stamp_of(tab: TabId, count: usize) -> u64 {
    let code = match tab {
        TabId::Recent => 0,
        TabId::Category(at) => at as u64 + 1,
        TabId::Saved => u64::from(u32::MAX),
    };
    (code << 32) | count as u64
}

fn tab_title<P>(tab: TabId, categories: &[PresetCategory<P>], words: &NewDocumentWords) -> Label {
    match tab {
        TabId::Recent => words.recent.clone(),
        TabId::Saved => words.saved.clone(),
        TabId::Category(at) => categories.get(at).map_or_else(Label::default, |category| match category.id.as_ref() {
            "photo" => words.photo.clone(),
            "print" => words.print.clone(),
            "art" => words.art.clone(),
            "web" => words.web.clone(),
            "mobile" => words.mobile.clone(),
            "film" => words.film.clone(),
            _ => category.title.clone(),
        }),
    }
}

/// The cards `tab` lists.
fn cards_of<S: DocumentSettings>(tab: TabId, library: &PresetLibrary<S>, recent_items: bool, cx: &CardCx<'_, S>) -> Vec<DocumentCard<S>> {
    match tab {
        TabId::Category(at) => {
            cx.presets.get(at).map_or_else(Vec::new, |category| category.presets.iter().map(|preset| S::preset_card(preset, cx)).collect())
        }
        TabId::Saved => library
            .items()
            .into_iter()
            .filter(|preset| preset.value.fits())
            .map(|preset| {
                let mut card = S::saved_card(&preset.name, &preset.value, cx);
                card.source = CardSource::Saved(preset.id);
                card
            })
            .collect(),
        TabId::Recent => {
            let mut cards = S::recent_cards(cx);
            // With `recent_items` the default sizes stand in for recent documents until there are some.
            if recent_items && !cx.recents.is_empty() {
                cards.retain(|card| !matches!(card.source, CardSource::Preset { .. }));
            }
            cards
        }
    }
}

/// A preset's name: the word set's for a built-in id, the app's own English name otherwise.
fn preset_title(preset: &DocumentPreset, setup: &RasterSetup, words: &NewDocumentWords, preset_words: &NewDocumentPresetWords) -> Label {
    if preset.id == DEFAULT_ID {
        return match &setup.app_name {
            Some(app) => Label::from(words.default_size_named.fill(&[("app", app.as_str())])),
            None => words.default_size.clone(),
        };
    }
    preset_words.name_of(preset.id).cloned().unwrap_or_else(|| Label::from(preset.name.to_owned()))
}

/// The card of a built-in preset, turned with the orientation; a print preset shows in the unit the settings show.
pub(super) fn raster_preset_card(preset: &DocumentPreset, cx: &CardCx<'_, DocumentOptions>) -> DocumentCard<DocumentOptions> {
    let preset = cx.orientation.map_or_else(|| preset.clone(), |orientation| preset.turned(orientation));
    let mut settings = preset.options(cx.setup.print_ppi, cx.setup.screen_ppi);
    if preset.in_inches() && !matches!(cx.values.unit, UnitKind::Pixels | UnitKind::Percent) {
        settings.unit = cx.values.unit;
    }
    DocumentCard::new(
        preset_title(&preset, cx.setup, cx.words, cx.preset_words),
        size_line(&settings, cx.words, cx.units),
        settings,
        CardSource::Preset { artboard: Some(preset.artboard) },
    )
}

/// The row of a saved preset. The dialog sets the source to the library's id; it restores everything but the name
/// either way.
pub(super) fn raster_saved_card(name: &str, saved: &DocumentOptions, cx: &CardCx<'_, DocumentOptions>) -> DocumentCard<DocumentOptions> {
    let words = cx.words;
    let line = words.saved_detail.fill(&[
        ("size", &size_line(saved, words, cx.units)),
        ("mode", mode_label(words, saved.color_mode).as_str()),
        ("depth", depth_label(words, saved.depth).as_str()),
    ]);
    DocumentCard::new(Label::from(name.to_owned()), line, saved.clone(), CardSource::Recent)
}

/// The cards Recent lists: the clipboard's size, the recent documents, then the app's default sizes not yet among them.
pub(super) fn raster_recent_cards(cx: &CardCx<'_, DocumentOptions>) -> Vec<DocumentCard<DocumentOptions>> {
    let (setup, words, units) = (cx.setup, cx.words, cx.units);
    let mut cards = Vec::new();
    if let Some(size) = setup.clipboard {
        let settings = DocumentOptions { size, pixels_per_inch: setup.screen_ppi, unit: UnitKind::Pixels, ..DocumentOptions::default() };
        cards.push(DocumentCard::new(words.clipboard.clone(), size_line(&settings, words, units), settings, CardSource::Clipboard));
    }
    for recent in cx.recents {
        let title = recent_title(recent, cx);
        let line = size_line(recent, words, units);
        let detail = words.recent_detail.fill(&[
            ("title", title.as_str()),
            ("size", &line),
            ("mode", mode_label(words, recent.color_mode).as_str()),
            ("background", background_label(words, recent.background).as_str()),
        ]);
        cards.push(DocumentCard::new(title, line, recent.clone(), CardSource::Recent).tooltip(detail));
    }
    for preset in &setup.recent_defaults {
        let mut card = raster_preset_card(preset, cx);
        card.source = CardSource::Preset { artboard: None };
        let listed = cx.recents.iter().any(|recent| recent.size == card.settings.size && recent.pixels_per_inch == card.settings.pixels_per_inch);
        if !listed {
            cards.push(card);
        }
    }
    cards
}

/// A recent document takes the name of the built-in preset it matches.
fn recent_title(recent: &DocumentOptions, cx: &CardCx<'_, DocumentOptions>) -> Label {
    let matches = |preset: &&DocumentPreset| {
        recent.size.width == preset.width && recent.size.height == preset.height && recent.pixels_per_inch == preset.pixels_per_inch
    };
    let found = cx.presets.iter().flat_map(|category| category.presets.iter()).chain(cx.setup.recent_defaults.iter()).find(matches);
    found.map_or_else(|| cx.words.custom.clone(), |preset| preset_title(preset, cx.setup, cx.words, cx.preset_words))
}

/// How New Document's preset cards show their page. An app sets it once at its root (`RootEnv::values`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, EnvValue, MemoProps)]
#[env(style)]
#[non_exhaustive]
pub enum PresetCardLook {
    /// A small page outline in content ink on a raised card, the name and size line centred under it.
    #[default]
    Outline,
    /// Blank paper on its shadow (`Tile::page`) on a flat card (`CardText::Page`): the name strong on up to two lines,
    /// the size line under it. VectorCraft's preset cards, as production drew them.
    Page,
}

/// The tile of a card: the page at the card's aspect in `look`, the title and the size line.
fn tile_of<S: DocumentSettings>(card: &DocumentCard<S>, look: PresetCardLook) -> Tile<'static> {
    let page = card.settings.page();
    let tile = Tile::new(card.title.clone()).detail(card.line.clone());
    let tile = match look {
        PresetCardLook::Outline => {
            tile.flat_thumbnail().thumb(ThumbAspect::new(2, 1), move |painter: &ContentPainterCx<'_>, rect: Rect| paint_page(painter, rect, page))
        }
        PresetCardLook::Page => tile.page(page.0, page.1),
    };
    match &card.tooltip {
        Some(tooltip) => tile.tooltip(tooltip.clone()),
        None => tile,
    }
}

/// A page icon inside the tile's picture, in content ink so the tile alone draws the selection ring.
fn paint_page(painter: &ContentPainterCx<'_>, picture: Rect, (width, height): (f64, f64)) {
    let page = page_rect(picture, width, height);
    let stroke = Stroke::new(painter.stroke_width(StrokeWidth::Thin), painter.ink());
    painter.painter.rect_stroke(page, CornerRadius::ZERO, stroke, StrokeKind::Inside);
}

/// The page's aspect inside `room`, capped at the 30-point size of the original outline.
fn page_rect(room: Rect, width: f64, height: f64) -> Rect {
    const PAGE_SIDE: f32 = 30.0;
    let (width, height) = (width.max(1.0) as f32, height.max(1.0) as f32);
    let scale = (room.width().min(PAGE_SIDE) / width).min(room.height().min(PAGE_SIDE) / height);
    Rect::from_center_size(room.center(), vec2(width * scale, height * scale))
}
