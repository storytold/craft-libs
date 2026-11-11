//! The picker's popup: the filter toggles, the class column and the list of family rows.

use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use craft_behavior::{DisplayOrder, IconId, ItemSource, Selection};
use craft_ui::buttons::{Button, ToggleButton};
use craft_ui::choice::{Radio, RadioGroup};
use craft_ui::contract::overlay::Dismiss;
use craft_ui::contract::{
    Bind, CollectionEvent, Cx, Edit as _, Overlay, OverlayUi, Overlays as _, Props as _, Remember as _, Show as _, ShowCollection as _,
    ShowContainer as _, WordSetCx as _, clicked,
};
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::{List, ListRow, RowEvent, ThumbAspect, ThumbSize, TrailingFace};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::Label;
use egui::Rect;

use super::rank::{Language, Listing, list};
use super::{FamilyRow, FontClass, FontFilters, FontPicker, FontPickerWords, POPUP, STAR, Typing, infos, read, ui_language, write};
use crate::panel::faces::{Owner, Shown, paint_sample, shown};
use craft_ui::style::DataColor;

/// The share of a sample thumbnail's height the sample's em takes.
const SAMPLE_EM: f32 = 0.7;
/// The sample thumbnail is this many times as wide as high.
const SAMPLE_ASPECT: (u16, u16) = (4, 1);

/// One position of the list: a family, or the gap between the recents and the other families.
enum Slot {
    Family(FamilyRow),
    Gap,
}

/// The list's slots as a collection source, keyed by position.
struct Slots<'a>(&'a [Slot]);

impl DisplayOrder<usize> for Slots<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn is_section(&self, at: usize) -> bool {
        matches!(self.0[at], Slot::Gap)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        match &self.0[at] {
            Slot::Family(row) => Some(Cow::Borrowed(row.shown.as_str())),
            Slot::Gap => None,
        }
    }
}

impl ItemSource<usize> for Slots<'_> {}

/// The rows with a gap after the recents.
fn slots(rows: Vec<FamilyRow>) -> Vec<Slot> {
    let recents = rows.iter().take_while(|row| row.recent).count();
    let mut slots: Vec<Slot> = Vec::with_capacity(rows.len() + 1);
    for (at, row) in rows.into_iter().enumerate() {
        if at == recents && recents > 0 {
            slots.push(Slot::Gap);
        }
        slots.push(Slot::Family(row));
    }
    slots
}

fn class_label(class: FontClass, words: &FontPickerWords) -> Label {
    match class {
        FontClass::Serif => words.serif.clone(),
        FontClass::SlabSerif => words.slab_serif.clone(),
        FontClass::SansSerif => words.sans_serif.clone(),
        FontClass::Script => words.script.clone(),
        FontClass::Monospace => words.monospace.clone(),
        FontClass::Handwritten => words.handwritten.clone(),
        FontClass::Blackletter => words.blackletter.clone(),
        FontClass::Decorative => words.decorative.clone(),
    }
}

/// The short mark at the end of a row for a class.
fn class_mark(class: FontClass, words: &FontPickerWords) -> Label {
    match class {
        FontClass::Serif => words.mark_serif.clone(),
        FontClass::SlabSerif => words.mark_slab_serif.clone(),
        FontClass::SansSerif => words.mark_sans_serif.clone(),
        FontClass::Script => words.mark_script.clone(),
        FontClass::Monospace => words.mark_monospace.clone(),
        FontClass::Handwritten => words.mark_handwritten.clone(),
        FontClass::Blackletter => words.mark_blackletter.clone(),
        FontClass::Decorative => words.mark_decorative.clone(),
    }
}

/// The filter toggles above the list: the class column's collapse button, variable fonts, favourites and similar
/// fonts, and Clear All.
fn filter_bar(ui: &mut egui::Ui, filters: &mut FontFilters, hide_classes: &mut bool, words: &FontPickerWords) {
    Row::new().show(ui, |ui, _| {
        let (icon, label) = if *hide_classes { ("chevrons_right", &words.show_classes) } else { ("chevrons_left", &words.hide_classes) };
        ToggleButton::new(label.clone(), Bind::bound(hide_classes)).icon(IconId::new(icon)).icon_only().show(ui);
        ToggleButton::new(words.variable.clone(), Bind::bound(&mut filters.variable)).icon(IconId::new("font_variable")).icon_only().show(ui);
        let star = if filters.favorites { "star_filled" } else { "star" };
        ToggleButton::new(words.favorites.clone(), Bind::bound(&mut filters.favorites)).icon(IconId::new(star)).icon_only().show(ui);
        ToggleButton::new(words.similar.clone(), Bind::bound(&mut filters.similar)).icon(IconId::new("font_similar")).icon_only().show(ui);
        let clear = Button::new(words.clear_all.clone()).enabled(filters.any()).show(ui);
        if clicked(&clear) {
            filters.clear();
        }
    });
}

/// The class column: All Classes and the eight classes, one chosen.
fn class_column(ui: &mut egui::Ui, filters: &mut FontFilters, words: &FontPickerWords) {
    let mut chosen = filters.class.and_then(|c| FontClass::ALL.iter().position(|a| *a == c)).map_or(0u8, |i| i as u8 + 1);
    RadioGroup::new(words.classes.clone(), Bind::bound(&mut chosen)).show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            Radio::new(0u8).label(words.all_classes.clone()).show(ui);
            for (at, class) in FontClass::ALL.into_iter().enumerate() {
                Radio::new(at as u8 + 1).label(class_label(class, words)).show(ui);
            }
        });
    });
    filters.class = usize::from(chosen).checked_sub(1).and_then(|i| FontClass::ALL.get(i).copied());
}

/// The sample thumbnail of a row: `text` in the family's own face, nothing until the face is loaded.
fn sample_thumb<'a>(
    picker: &'a FontPicker<'_>,
    infos: &'a super::rank::Infos,
    row: &'a FamilyRow,
    sample: &'a str,
) -> impl FnOnce(&craft_ui::primitives::ContentPainterCx<'_>, Rect) + 'a {
    move |cx, rect| {
        let Some(info) = infos.families.get(&row.family) else { return };
        let text = infos.sample_text(picker.catalog, &row.family, sample, &row.shown);
        if let Shown::Ready(font) = shown(cx, picker.catalog, Owner::Installed, &info.regular) {
            let ink = DataColor::new(cx.ink());
            paint_sample(cx, rect, &font, &text, rect.height() * SAMPLE_EM, ink);
        }
    }
}

/// The list popup: one option row per family, the committed family's row selected. A pick comes back as `Activate`
/// (Enter, Tab or a click), a row the lead moves to is proposed as the preview, and a star flips a favourite.
pub(super) fn popup<'a>(picker: &mut FontPicker<'a>, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, FontPicker<'a>>) {
    let popup = cx.popup(POPUP);
    let words = cx.words::<FontPickerWords>();
    let query = cx.memory::<Typing>().draft.clone().unwrap_or_default();
    let mut hide_classes = cx.memory::<Typing>().hide_classes;
    let family = cx.value().cloned().unwrap_or_default();
    let settings = read(&mut picker.settings, cx);
    let mut filters = read(&mut picker.filters, cx);
    let has_filters = filters.is_some();
    let narrowing = filters.clone().unwrap_or_default();
    let infos = infos(picker.catalog);
    let language_code = ui_language(&cx.env);
    let (folder, collator) = (cx.env.folder(), cx.env.collator());
    let listing = Listing { query: &query, settings: settings.as_ref(), filters: &narrowing, current: &family, language: &language_code };
    let (found, comparable) = list(picker.catalog, &infos, &listing, &Language { folder: &folder, collator: &collator }, true);
    let slots = slots(found);
    let current = slots.iter().position(|slot| matches!(slot, Slot::Family(row) if row.family == family));
    let size = settings.as_ref().map_or(ThumbSize::None, |s| s.preview.thumb());
    let favorites_empty = settings.as_ref().is_none_or(|s| s.favorites.is_empty());
    let mut stamp = DefaultHasher::new();
    query.hash(&mut stamp);
    for slot in &slots {
        if let Slot::Family(row) = slot {
            (&row.family, row.recent).hash(&mut stamp);
        }
    }
    let name = words.list.clone();
    let mut stars: Vec<(String, bool)> = Vec::new();
    let events = ui.show(popup, Overlay::dropdown().dismiss(Dismiss::new()), |ui, _| {
        let mut events = Vec::new();
        Stack::new().show(ui, |ui, _| {
            if let Some(filters) = filters.as_mut() {
                filter_bar(ui, filters, &mut hide_classes, &words);
            }
            Row::new().show(ui, |ui, _| {
                if let (Some(filters), false) = (filters.as_mut(), hide_classes) {
                    class_column(ui, filters, &words);
                }
                Stack::new().show(ui, |ui, _| {
                    if narrowing.similar && !comparable {
                        Text::new(words.no_similar.clone()).show(ui);
                    }
                    if slots.is_empty() {
                        let message = if narrowing.favorites && favorites_empty {
                            &words.no_favorites
                        } else if narrowing.any() {
                            &words.no_match_filters
                        } else {
                            &words.no_match
                        };
                        Text::new(message.clone()).show(ui);
                        return;
                    }
                    let source = Slots(&slots);
                    let selection = current.filter(|_| query.trim().is_empty()).map_or_else(Selection::default, Selection::single);
                    let rows = List::<usize>::options(name.clone()).selection(Bind::given(selection)).rows_stamp(stamp.finish()).show_items(
                        ui,
                        &source,
                        &mut |key, _| match &slots[*key] {
                            Slot::Gap => ListRow::spacer(),
                            Slot::Family(row) => {
                                let icon = if row.starred { "star_filled" } else { "star" };
                                let name = Label::from(words.favorite.fill(&[("row", row.shown.as_str())]));
                                let star = TrailingFace::Icon(IconSpec::svg(IconId::new(icon)));
                                let line = ListRow::new(row.shown.clone()).option().trailing_part(STAR, star, name);
                                let line = match row.class {
                                    Some(class) => line.trailing(class_mark(class, &words)),
                                    None => line,
                                };
                                match size {
                                    ThumbSize::None => line,
                                    size => line.thumb_aspect(
                                        size,
                                        ThumbAspect::new(SAMPLE_ASPECT.0, SAMPLE_ASPECT.1),
                                        sample_thumb(picker, &infos, row, words.sample.as_str()),
                                    ),
                                }
                            }
                        },
                    );
                    events = rows.events;
                });
            });
        });
        events
    });
    let mut picked = None;
    for event in events.into_iter().flatten() {
        match event {
            CollectionEvent::LeadMoved { key, .. } => {
                if let Some(Slot::Family(row)) = slots.get(key) {
                    cx.memory::<Typing>().lead = Some(row.family.clone());
                    cx.propose(row.family.clone());
                }
            }
            CollectionEvent::Activate(key) => picked = Some(key),
            CollectionEvent::Item(key, RowEvent::Trailing { part, .. }) if part == STAR => {
                if let Some(Slot::Family(row)) = slots.get(key) {
                    stars.push((row.family.clone(), !row.starred));
                }
            }
            _ => {}
        }
    }
    cx.memory::<Typing>().hide_classes = hide_classes;
    if let Some(mut changed) = settings.clone().filter(|_| !stars.is_empty()) {
        for (family, on) in &stars {
            changed.set_favorite(family, *on);
        }
        write(&mut picker.settings, cx, changed, "Favorite font");
    }
    if has_filters
        && filters != Some(narrowing)
        && let Some(filters) = filters
    {
        write(&mut picker.filters, cx, filters, "Font filters");
    }
    if let Some(Slot::Family(row)) = picked.and_then(|key| slots.get(key)) {
        picker.picked = Some(row.family.clone());
        cx.propose(row.family.clone());
        cx.commit();
        popup.close(cx);
    }
}
