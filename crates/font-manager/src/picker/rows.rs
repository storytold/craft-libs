//! The picker's popup: the preview banner, the filter toggles, the class column and the list of family rows with the
//! styles of the families opened in it.

use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use craft_behavior::{DisplayOrder, IconId, ItemSource, Preview, Selection};
use craft_font_catalog::Catalog;
use craft_ui::buttons::{Button, ToggleButton};
use craft_ui::choice::{Radio, RadioGroup};
use craft_ui::contract::overlay::Dismiss;
use craft_ui::contract::{
    Bind, CollectionEvent, Cx, Edit as _, LeadSource, Overlay, OverlayUi, Overlays as _, Props as _, Remember as _, Show as _, ShowCollection as _,
    ShowContainer as _, WordSetCx as _, clicked,
};
use craft_ui::display::{Picture, PictureSize, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::{List, ListRow, RowEvent, ThumbAspect, ThumbSize, TrailingFace};
use craft_ui::primitives::{ContentPainterCx, IconSpec};
use craft_ui::runtime::env::Label;
use egui::Rect;

use super::rank::{Infos, Language, Listing, list, shown_name};
use super::{
    FamilyRow, FontClass, FontFilters, FontFormat, FontPicker, FontPickerSettings, FontPickerWords, POPUP, STAR, Typing, infos, read, ui_language,
    write,
};
use crate::panel::faces::{Owner, Shown, paint_sample, shown};
use craft_ui::style::DataColor;

/// The share of a sample thumbnail's height the sample's em takes.
const SAMPLE_EM: f32 = 0.7;
/// The sample thumbnail is this many times as wide as high.
const SAMPLE_ASPECT: (u16, u16) = (4, 1);
/// The banner's sample is this many times as wide as high.
const BANNER_ASPECT: (u16, u16) = (12, 1);

/// One position of the list: a family, one style of an open family, or the gap between the recents and the other
/// families.
enum Slot {
    Family { row: FamilyRow, open: bool },
    Style { family: String, style: String },
    Gap,
}

/// What names a slot across passes. A family opening shifts the positions after it, so the list keys its rows by what
/// they are and the lead stays on its row.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum SlotKey {
    Gap,
    Family { family: String, recent: bool },
    Style { family: String, style: String },
}

impl Slot {
    fn key(&self) -> SlotKey {
        match self {
            Slot::Family { row, .. } => SlotKey::Family { family: row.family.clone(), recent: row.recent },
            Slot::Style { family, style } => SlotKey::Style { family: family.clone(), style: style.clone() },
            Slot::Gap => SlotKey::Gap,
        }
    }

    /// The family the slot stands for, which the lead previews.
    fn family(&self) -> Option<&str> {
        match self {
            Slot::Family { row, .. } => Some(&row.family),
            Slot::Style { family, .. } => Some(family),
            Slot::Gap => None,
        }
    }

    /// What picking the slot picks: the family, and the style of a style row.
    fn pick(&self) -> Option<(&str, Option<&str>)> {
        match self {
            Slot::Family { row, .. } => Some((&row.family, None)),
            Slot::Style { family, style } => Some((family, Some(style))),
            Slot::Gap => None,
        }
    }
}

fn slot_of<'s>(slots: &'s [Slot], key: &SlotKey) -> Option<&'s Slot> {
    slots.iter().find(|slot| slot.key() == *key)
}

/// The list's slots as a collection source: a tree two levels deep.
struct Slots<'a>(&'a [Slot]);

impl DisplayOrder<SlotKey> for Slots<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> SlotKey {
        self.0[at].key()
    }

    fn index_of(&self, key: &SlotKey) -> Option<usize> {
        self.0.iter().position(|slot| slot.key() == *key)
    }

    fn depth(&self, at: usize) -> usize {
        usize::from(matches!(self.0[at], Slot::Style { .. }))
    }

    /// A family with several styles. A recent row has none: the family is listed again below the gap.
    fn has_children(&self, at: usize) -> bool {
        matches!(&self.0[at], Slot::Family { row, .. } if row.styles > 1 && !row.recent)
    }

    fn open(&self, at: usize) -> bool {
        matches!(self.0[at], Slot::Family { open: true, .. })
    }

    fn is_section(&self, at: usize) -> bool {
        matches!(self.0[at], Slot::Gap)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        match &self.0[at] {
            Slot::Family { row, .. } => Some(Cow::Borrowed(row.shown.as_str())),
            Slot::Style { style, .. } => Some(Cow::Borrowed(style.as_str())),
            Slot::Gap => None,
        }
    }
}

impl ItemSource<SlotKey> for Slots<'_> {}

/// The rows with a gap after the recents and the styles of each expanded family under its row.
fn slots(catalog: &Catalog, rows: Vec<FamilyRow>, expanded: &[String]) -> Vec<Slot> {
    let recents = rows.iter().take_while(|row| row.recent).count();
    let mut slots: Vec<Slot> = Vec::with_capacity(rows.len() + 1);
    for (at, row) in rows.into_iter().enumerate() {
        if at == recents && recents > 0 {
            slots.push(Slot::Gap);
        }
        let open = !row.recent && row.styles > 1 && expanded.contains(&row.family);
        let family = row.family.clone();
        slots.push(Slot::Family { row, open });
        if open {
            let mut seen: Vec<String> = Vec::new();
            for face in catalog.styles(&family) {
                if !seen.contains(&face.style) {
                    seen.push(face.style.clone());
                    slots.push(Slot::Style { family: family.clone(), style: face.style });
                }
            }
        }
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

/// The name of a format's badge.
fn format_label(format: FontFormat, words: &FontPickerWords) -> Label {
    match format {
        FontFormat::OpenType => words.format_open_type.clone(),
        FontFormat::TrueType => words.format_true_type.clone(),
        FontFormat::Variable => words.format_variable.clone(),
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

/// A thumbnail painting `text` in the regular face of `family`; nothing for a family the catalog lacks or an empty
/// text, and nothing until the face is loaded.
fn face_thumb<'a>(catalog: &'a Arc<Catalog>, infos: &'a Infos, family: &'a str, text: String) -> impl Fn(&ContentPainterCx<'_>, Rect) + 'a {
    move |cx, rect| {
        let Some(info) = infos.families.get(family) else { return };
        if text.is_empty() {
            return;
        }
        if let Shown::Ready(font) = shown(cx, catalog, Owner::Installed, &info.regular) {
            let ink = DataColor::new(cx.ink());
            paint_sample(cx, rect, &font, &text, rect.height() * SAMPLE_EM, ink);
        }
    }
}

/// The banner: `text` in the face of `family`, on one line cut at the end, in a picture as high as the large thumbnail.
fn banner(ui: &mut egui::Ui, catalog: &Arc<Catalog>, infos: &Infos, family: &str, shown_name: &str, text: &str) {
    Picture::painter(shown_name.to_owned(), ThumbAspect::new(BANNER_ASPECT.0, BANNER_ASPECT.1), face_thumb(catalog, infos, family, text.to_owned()))
        .extent(PictureSize::Thumb(ThumbSize::Large))
        .show(ui);
}

/// The name the banner shows for `family`: its row's shown name when the list holds one, else its name in the
/// interface language, else the family key.
fn banner_shown_name(slots: &[Slot], infos: &Infos, family: &str, language: &str, settings: Option<&FontPickerSettings>) -> String {
    let row = slots.iter().find_map(|slot| match slot {
        Slot::Family { row, .. } if row.family == family => Some(row.shown.clone()),
        _ => None,
    });
    row.or_else(|| {
        let english = settings.is_some_and(|s| s.names_in_english);
        infos.families.get(family).map(|info| shown_name(info, family, language, english))
    })
    .unwrap_or_else(|| family.to_owned())
}

/// Reports `family` as rested once per open list: the host fetches a served family on it.
fn rest<'a>(picker: &mut FontPicker<'a>, cx: &mut Cx<'_, FontPicker<'a>>, family: &str) {
    let typing = cx.memory::<Typing>();
    if !typing.rested.iter().any(|f| f == family) {
        typing.rested.push(family.to_owned());
        picker.rested = Some(family.to_owned());
    }
}

/// Shows `family` as the live preview, led by the pointer or by the keys; the pass reports it when the family changes.
fn show_preview<'a>(picker: &mut FontPicker<'a>, cx: &mut Cx<'_, FontPicker<'a>>, family: String, by_pointer: bool) {
    let typing = cx.memory::<Typing>();
    typing.lead_by_pointer = by_pointer;
    if typing.lead.as_ref() != Some(&family) {
        typing.lead = Some(family.clone());
        picker.preview = Preview::Shows(family);
    }
}

/// Ends the live preview, if one runs; the pass reports the end.
pub(super) fn end_preview<'a>(picker: &mut FontPicker<'a>, cx: &mut Cx<'_, FontPicker<'a>>) {
    let typing = cx.memory::<Typing>();
    typing.lead_by_pointer = false;
    if typing.lead.take().is_some() {
        picker.preview = Preview::Ended;
    }
}

/// The list popup: one option row per family, the committed family's row selected, and the styles of the open
/// families under their rows. A pick comes back as `Activate` (Enter, Tab or a click), a row the lead moves to is
/// reported as the preview (the field's value and text stay), and a star flips a favourite. Each pass reports only the events of its own input.
pub(super) fn popup<'a>(picker: &mut FontPicker<'a>, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, FontPicker<'a>>) {
    let popup = cx.popup(POPUP);
    let words = cx.words::<FontPickerWords>();
    let (query, lead, expanded) = {
        let typing = cx.memory::<Typing>();
        (typing.draft.clone().unwrap_or_default(), typing.lead.clone(), typing.expanded.clone())
    };
    let mut hide_classes = cx.memory::<Typing>().hide_classes;
    let family = cx.value().cloned().unwrap_or_default();
    let settings = read(&mut picker.settings, cx);
    let mut filters = read(&mut picker.filters, cx);
    let has_filters = filters.is_some();
    let narrowing = filters.clone().unwrap_or_default();
    let catalog = picker.catalog;
    let infos = infos(catalog);
    let language_code = ui_language(&cx.env);
    let (folder, collator) = (cx.env.folder(), cx.env.collator());
    let listing = Listing {
        query: &query,
        settings: settings.as_ref(),
        filters: &narrowing,
        current: &family,
        language: &language_code,
        served: picker.served,
    };
    let (found, comparable) = list(catalog, &infos, &listing, &Language { folder: &folder, collator: &collator }, true);
    let slots = slots(catalog, found, &expanded);
    let current = slots.iter().position(|slot| matches!(slot, Slot::Family { row, .. } if row.family == family));
    let size = settings.as_ref().map_or(ThumbSize::None, |s| s.preview.thumb());
    let favorites_empty = settings.as_ref().is_none_or(|s| s.favorites.is_empty());
    // Rows are keyed, so the stamp names only what rebuilds the list: opening a family keeps the lead.
    let mut stamp = DefaultHasher::new();
    (&query, narrowing.class, narrowing.favorites, narrowing.variable, narrowing.similar).hash(&mut stamp);
    let banner_family = lead.unwrap_or_else(|| family.clone());
    let banner_text = picker.preview_text;
    let name = words.list.clone();
    let mut stars: Vec<(String, bool)> = Vec::new();
    let mut over_rows = false;
    let mut under_pointer: Option<SlotKey> = None;
    let events = ui.show(popup, Overlay::dropdown().dismiss(Dismiss::new().focus_left(true)), |ui, _| {
        let mut events = Vec::new();
        Stack::new().show(ui, |ui, _| {
            if let Some(text) = banner_text {
                let shown_name = banner_shown_name(&slots, &infos, &banner_family, &language_code, settings.as_ref());
                banner(ui, catalog, &infos, &banner_family, &shown_name, text);
            }
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
                    let selection =
                        current.filter(|_| query.trim().is_empty()).map_or_else(Selection::default, |at| Selection::single(slots[at].key()));
                    let rows = List::<SlotKey>::options(name.clone())
                        .tree(true)
                        .selection(Bind::given(selection))
                        .rows_stamp(stamp.finish())
                        .show_items(ui, &source, &mut |_, item| match &slots[item.at] {
                            Slot::Gap => ListRow::spacer(),
                            Slot::Style { style, .. } => ListRow::new(style.clone()).option().depth(1),
                            Slot::Family { row, open } => {
                                let icon = if row.starred { "star_filled" } else { "star" };
                                let name = Label::from(words.favorite.fill(&[("row", row.shown.as_str())]));
                                let star = TrailingFace::Icon(IconSpec::svg(IconId::new(icon)));
                                let mut line =
                                    ListRow::new(row.shown.clone()).option().depth(0).check(current == Some(item.at)).trailing_part(STAR, star, name);
                                if source.has_children(item.at) {
                                    line = line.group(*open);
                                }
                                if let Some(class) = row.class {
                                    line = line.trailing_named(class_mark(class, &words), class_label(class, &words));
                                }
                                line = match row.format {
                                    Some(format) => line.trailing_named(Label::from(format.badge().to_owned()), format_label(format, &words)),
                                    None => line.trailing_named(words.served.clone(), words.served.clone()),
                                };
                                if size == ThumbSize::None {
                                    return line;
                                }
                                let samples =
                                    infos.sample_text(catalog, &row.family, words.sample.as_str(), words.sample_arabic.as_str(), &row.shown);
                                let aspect = ThumbAspect::new(SAMPLE_ASPECT.0, SAMPLE_ASPECT.1);
                                line = line.thumb_aspect(size, aspect, face_thumb(catalog, &infos, &row.family, samples.latin));
                                match samples.arabic {
                                    Some(arabic) => line.extra_thumb(face_thumb(catalog, &infos, &row.family, arabic)),
                                    None => line,
                                }
                            }
                        });
                    over_rows = rows.response.contains_pointer();
                    let pointer = ui.ctx().pointer_hover_pos();
                    under_pointer = pointer.and_then(|at| rows.shown.iter().find(|row| row.rect.contains(at)).map(|row| row.key.clone()));
                    events = rows.events;
                });
            });
        });
        events
    });
    let mut picked: Option<SlotKey> = None;
    for event in events.into_iter().flatten() {
        match event {
            CollectionEvent::LeadMoved { key, by } => {
                if let Some(family) = slot_of(&slots, &key).and_then(Slot::family) {
                    show_preview(picker, cx, family.to_owned(), by == LeadSource::Pointer);
                }
            }
            CollectionEvent::Rested(key) => {
                if let Some(family) = slot_of(&slots, &key).and_then(Slot::family) {
                    rest(picker, cx, family);
                }
            }
            CollectionEvent::Open(change) => {
                if let Some(Slot::Family { row, .. }) = slot_of(&slots, &change.key) {
                    let typing = cx.memory::<Typing>();
                    typing.expanded.retain(|f| *f != row.family);
                    if change.open {
                        typing.expanded.push(row.family.clone());
                    }
                }
            }
            CollectionEvent::Activate(key) => picked = Some(key),
            CollectionEvent::Item(key, RowEvent::Trailing { part, .. }) if part == STAR => {
                if let Some(Slot::Family { row, .. }) = slot_of(&slots, &key) {
                    stars.push((row.family.clone(), !row.starred));
                }
            }
            _ => {}
        }
    }
    cx.memory::<Typing>().hide_classes = hide_classes;
    // A preview the pointer proposed ends when the pointer leaves the rows (a keyed row keeps its preview). The
    // collection keeps its lead on the row, so the pointer coming back to that row reports no move: the row under it
    // proposes again.
    let (by_pointer, led) = {
        let typing = cx.memory::<Typing>();
        (typing.lead_by_pointer, typing.lead.is_some())
    };
    if by_pointer && !over_rows {
        end_preview(picker, cx);
    } else if over_rows
        && !led
        && let Some(family) = under_pointer.and_then(|key| slot_of(&slots, &key).and_then(Slot::family).map(str::to_owned))
    {
        show_preview(picker, cx, family, true);
    }
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
    if let Some((family, style)) = picked.and_then(|key| slot_of(&slots, &key)).and_then(Slot::pick) {
        picker.picked = Some(family.to_owned());
        picker.picked_style = style.map(str::to_owned);
        end_preview(picker, cx);
        cx.propose(family.to_owned());
        cx.commit();
        popup.close(cx);
    }
}
