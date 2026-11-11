//! The left column of New Document: the category tabs over the grid of preset cards, or the list of saved presets.

use super::presets::{DEFAULT_ID, DocumentOptions, DocumentPreset, NewDocumentPresetWords, size_line};
use super::{NewDocumentWords, Read, Session, Setup, background_label, depth_label, mode_label};
use crate::contract::{Bind, CollectionEvent, Props as _, SelectMode, Show as _, ShowCollection as _};
use crate::display::{Shade, Text};
use crate::list::{List, ListRow, ThumbAspect};
use crate::patterns::presets::PresetId;
use crate::primitives::{ContentPainterCx, PaintShape};
use crate::runtime::env::{Label, UnitKind};
use crate::style::TypeRole;
use crate::tabs::{Tab, TabKind, TabStrip};
use crate::tile_grid::{Tile, TileGrid};
use craft_behavior::{DisplayOrder, ItemSource, Selection, SelectionChange, keyed};
use egui::{CornerRadius, Rect, Ui, vec2};
use std::borrow::Cow;

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

/// A card of the grid, or a saved preset's row, ready to show and to apply.
#[derive(Clone)]
pub(super) struct Card {
    pub(super) title: Label,
    /// The size line under the title.
    pub(super) line: String,
    /// What the card sets: the size and unit, or (a recent document, a saved preset) everything but the name.
    pub(super) settings: DocumentOptions,
    pub(super) restores_all: bool,
    /// The hover text of a recent document, which names everything it restores.
    pub(super) detail: Option<String>,
    /// The width and height the page outline has.
    pub(super) aspect: (u32, u32),
    /// The library preset a Saved row stands for.
    pub(super) saved: Option<PresetId>,
}

/// The card the user picked in this pass.
pub(super) struct Picked {
    pub(super) card: Card,
    pub(super) index: usize,
    /// Enter or a double click: create the document from the card.
    pub(super) activated: bool,
}

/// What the left column reports.
#[derive(Default)]
pub(super) struct Left {
    pub(super) tab: Option<TabId>,
    pub(super) picked: Option<Picked>,
    /// The saved presets the Delete intent removed.
    pub(super) delete: Vec<PresetId>,
}

/// The cards as a display order keyed by position.
struct CardList<'a>(&'a [Card]);

impl DisplayOrder<usize> for CardList<'_> {
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

impl ItemSource<usize> for CardList<'_> {}

/// Shows the tabs and what the open tab lists, and reports the tab and the card the user picked.
pub(super) fn show(ui: &mut Ui, session: &Session, values: &DocumentOptions, read: &Read<'_>, left: &mut Left) {
    let (setup, words) = (read.setup, read.words);
    let tabs: Vec<TabId> =
        std::iter::once(TabId::Recent).chain((0..setup.categories.len()).map(TabId::Category)).chain(std::iter::once(TabId::Saved)).collect();
    let source = keyed(&tabs, |tab| *tab);
    let strip = TabStrip::new(words.tabs.clone())
        .kind(TabKind::Text)
        .shown(session.tab)
        .show_items(ui, &source, &mut |tab, _| Tab::new(tab_title(*tab, setup, words)));
    for event in strip.response.events {
        if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
            && let Some(tab) = keys.last()
        {
            left.tab = Some(*tab);
        }
    }

    let tab = left.tab.unwrap_or(session.tab);
    let cards = cards_of(tab, session, values, read);
    let selected = if tab == session.tab { selected_of(tab, session, values, &cards) } else { None };
    let selection = selected.map_or_else(|| Selection::new(Vec::new(), None, None), Selection::single);
    let stamp = stamp_of(tab, cards.len());
    let list = CardList(&cards);
    if tab == TabId::Saved {
        let shown = List::<usize>::new(words.saved_list.clone())
            .select(SelectMode::Single)
            .deletable(true)
            .empty(words.no_saved.clone())
            .rows_stamp(stamp)
            .selection(Bind::given(selection))
            .show_items(ui, &list, &mut |index, _| ListRow::new(cards[*index].title.clone()).detail(cards[*index].line.clone()));
        route(shown.events, &cards, left);
    } else {
        let count = read.units.number(cards.len() as f64, 0..=0);
        Text::new(words.presets_heading.fill(&[("n", &count)])).role(TypeRole::Label).shade(Shade::Muted).show(ui);
        let shown = TileGrid::<usize>::new(words.cards.clone())
            .select(SelectMode::Single)
            .detail_line()
            .rows_stamp(stamp)
            .selection(Bind::given(selection))
            .show_items(ui, &list, &mut |index, _| tile_of(&cards[*index]));
        route(shown.response.events, &cards, left);
    }
}

/// Turns a collection's events into the card picked: a selection change picks, Enter or a double click activates.
fn route<E>(events: Vec<CollectionEvent<usize, E>>, cards: &[Card], left: &mut Left) {
    for event in events {
        match event {
            CollectionEvent::Selection(SelectionChange::Set(keys)) => {
                if let Some(index) = keys.last() {
                    pick(left, cards, *index, false);
                }
            }
            CollectionEvent::Activate(index) => pick(left, cards, index, true),
            CollectionEvent::Delete(keys) => left.delete.extend(keys.iter().filter_map(|index| cards.get(*index).and_then(|card| card.saved))),
            _ => {}
        }
    }
}

/// Records the card at `index` as the one picked this pass; an activation stays once it was seen.
fn pick(left: &mut Left, cards: &[Card], index: usize, activated: bool) {
    let Some(card) = cards.get(index) else { return };
    let activated = activated || left.picked.as_ref().is_some_and(|picked| picked.index == index && picked.activated);
    left.picked = Some(Picked { card: card.clone(), index, activated });
}

/// The position the grid shows selected: the card picked in the tab, else the recent document the details show.
fn selected_of(tab: TabId, session: &Session, values: &DocumentOptions, cards: &[Card]) -> Option<usize> {
    match session.card {
        Some(at) => (at < cards.len()).then_some(at),
        None if tab == TabId::Recent => cards.iter().position(|card| card.restores_all && card.settings.same_document(values)),
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

fn tab_title(tab: TabId, setup: &Setup, words: &NewDocumentWords) -> Label {
    match tab {
        TabId::Recent => words.recent.clone(),
        TabId::Saved => words.saved.clone(),
        TabId::Category(at) => setup.categories.get(at).map_or_else(Label::default, |category| match category.id {
            "photo" => words.photo.clone(),
            "print" => words.print.clone(),
            "art" => words.art.clone(),
            "web" => words.web.clone(),
            "mobile" => words.mobile.clone(),
            "film" => words.film.clone(),
            _ => Label::from(category.title.to_owned()),
        }),
    }
}

/// A preset's name: the word set's for a built-in id, the app's own English name otherwise.
fn preset_title(preset: &DocumentPreset, setup: &Setup, words: &NewDocumentWords, preset_words: &NewDocumentPresetWords) -> Label {
    if preset.id == DEFAULT_ID {
        return match &setup.app_name {
            Some(app) => Label::from(words.default_size_named.fill(&[("app", app.as_str())])),
            None => words.default_size.clone(),
        };
    }
    preset_words.name_of(preset.id).cloned().unwrap_or_else(|| Label::from(preset.name.to_owned()))
}

/// The cards `tab` lists.
fn cards_of(tab: TabId, session: &Session, values: &DocumentOptions, read: &Read<'_>) -> Vec<Card> {
    let Read { setup, recents, library, words, preset_words, units, .. } = *read;
    let preset_card = |preset: &DocumentPreset| {
        let preset = session.orientation.map_or_else(|| preset.clone(), |orientation| preset.turned(orientation));
        let mut settings = preset.options(setup.print_ppi, setup.screen_ppi);
        if preset.is_print() && !matches!(values.unit, UnitKind::Pixels | UnitKind::Percent) {
            settings.unit = values.unit;
        }
        Card {
            title: preset_title(&preset, setup, words, preset_words),
            line: size_line(&settings, words, units),
            aspect: (settings.size.width, settings.size.height),
            settings,
            restores_all: false,
            detail: None,
            saved: None,
        }
    };
    match tab {
        TabId::Category(at) => setup.categories.get(at).map_or_else(Vec::new, |category| category.presets.iter().map(preset_card).collect()),
        TabId::Saved => library
            .items()
            .into_iter()
            .filter(|preset| preset.value.fits())
            .map(|preset| {
                let settings = &preset.value;
                let line = words.saved_detail.fill(&[
                    ("size", &size_line(settings, words, units)),
                    ("mode", mode_label(words, settings.color_mode).as_str()),
                    ("depth", depth_label(words, settings.depth).as_str()),
                ]);
                Card {
                    title: Label::from(preset.name.clone()),
                    line,
                    aspect: (settings.size.width, settings.size.height),
                    settings: settings.clone(),
                    restores_all: true,
                    detail: None,
                    saved: Some(preset.id),
                }
            })
            .collect(),
        TabId::Recent => {
            let mut cards = Vec::new();
            if let Some(size) = setup.clipboard {
                let settings = DocumentOptions { size, pixels_per_inch: setup.screen_ppi, unit: UnitKind::Pixels, ..DocumentOptions::default() };
                cards.push(Card {
                    title: words.clipboard.clone(),
                    line: size_line(&settings, words, units),
                    aspect: (size.width, size.height),
                    settings,
                    restores_all: false,
                    detail: None,
                    saved: None,
                });
            }
            for recent in recents {
                let title = recent_title(recent, setup, words, preset_words);
                let line = size_line(recent, words, units);
                let detail = words.recent_detail.fill(&[
                    ("title", title.as_str()),
                    ("size", &line),
                    ("mode", mode_label(words, recent.color_mode).as_str()),
                    ("background", background_label(words, recent.background).as_str()),
                ]);
                cards.push(Card {
                    title,
                    line,
                    aspect: (recent.size.width, recent.size.height),
                    settings: recent.clone(),
                    restores_all: true,
                    detail: Some(detail),
                    saved: None,
                });
            }
            for preset in &setup.recent_defaults {
                let card = preset_card(preset);
                let listed =
                    recents.iter().any(|recent| recent.size == card.settings.size && recent.pixels_per_inch == card.settings.pixels_per_inch);
                if !listed {
                    cards.push(card);
                }
            }
            cards
        }
    }
}

/// A recent document takes the name of the built-in preset it matches.
fn recent_title(recent: &DocumentOptions, setup: &Setup, words: &NewDocumentWords, preset_words: &NewDocumentPresetWords) -> Label {
    let matches = |preset: &&DocumentPreset| {
        recent.size.width == preset.width && recent.size.height == preset.height && recent.pixels_per_inch == preset.pixels_per_inch
    };
    let found = setup.categories.iter().flat_map(|category| category.presets.iter()).chain(setup.recent_defaults.iter()).find(matches);
    found.map_or_else(|| words.custom.clone(), |preset| preset_title(preset, setup, words, preset_words))
}

/// The tile of a card: the page outline at the card's aspect, the title and the size line.
fn tile_of(card: &Card) -> Tile<'static> {
    let aspect = card.aspect;
    let tile = Tile::new(card.title.clone())
        .detail(card.line.clone())
        .thumb(ThumbAspect::SQUARE, move |painter: &ContentPainterCx<'_>, rect: Rect| paint_page(painter, rect, aspect));
    match &card.detail {
        Some(detail) => tile.tooltip(detail.clone()),
        None => tile,
    }
}

/// The page outline in the picture: a well, and the page's outline at its aspect inside it.
fn paint_page(painter: &ContentPainterCx<'_>, picture: Rect, (width, height): (u32, u32)) {
    let well = painter.plot_frame(picture);
    painter.outline(&PaintShape::Rect(page_rect(well, width, height), CornerRadius::ZERO));
}

/// The largest rect of the page's aspect that fits `room`, centred in it.
fn page_rect(room: Rect, width: u32, height: u32) -> Rect {
    let (width, height) = (width.max(1) as f32, height.max(1) as f32);
    let scale = (room.width() / width).min(room.height() / height);
    Rect::from_center_size(room.center(), vec2(width * scale, height * scale))
}
