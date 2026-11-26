//! The picker's popup: the preview banner, the filter toggles, the class column and the list of the host's rows and
//! the family rows with the styles of the families opened in it. A family row reads, in reading order: its star, the disclosure of its styles
//! (or the empty disclosure column), the name, the format badge and the sample in the family's face.

use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use craft_behavior::{ColumnKey, ColumnWidth, DisplayOrder, IconId, ItemSource, Preview, Selection, SizeHint};
use craft_font_catalog::Catalog;
use craft_theme::variant::Variant;
use craft_ui::buttons::{Button, IconButton, ToggleButton};
use craft_ui::choice::{Radio, RadioGroup};
use craft_ui::contract::overlay::Dismiss;
use craft_ui::contract::{
    Bind, CollectionEvent, ColumnSpec, Cx, Edit as _, LeadSource, Overlay, OverlayUi, Overlays as _, Props as _, Remember as _, Show as _,
    ShowCollection as _, ShowContainer as _, ToggleColumn, WordSetCx as _, clicked,
};
use craft_ui::display::{Divider, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::{List, ListRow, RowEvent, ThumbSize, ToggleCell, ToggleFace, ToggleState, TrailingFace, TreeLeaves};
use craft_ui::primitives::{FacesCx, IconSpec};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::memory::StageAccess;

use super::banner::FontPreviewBanner;
use super::rank::{FamilyInfos, Language, Listing, host_rows, list, shown_name};
use super::{
    FamilyRow, FamilySet, FilterLayout, FontClass, FontFilters, FontFormat, FontPicker, FontPickerKind, FontPickerSettings, FontPickerWords,
    FontPreviewSize, FontScript, HostFontGroup, HostFontKind, HostFontRow, POPUP, SAMPLE_EM, SIMILAR, Sample, Typing, infos_of, read, ui_language,
    write,
};
use crate::panel::faces::{self, Owner, Shown};
use crate::panel::specimen::{FontSample, SampleFace};

/// The sample column is this many times as wide as the sample band is high.
const SAMPLE_WIDTH: f32 = 4.0;
/// The star column: a favourite's star, on every family row.
const STAR: ColumnKey = ColumnKey::new("star");
/// The column of the name, the disclosure, the check and, at its end, the format badge: as wide as its widest row, and
/// at least as wide as the names the field holds.
const NAME: ColumnKey = ColumnKey::new("name");
/// The column of the sample in the family's face.
const SAMPLE: ColumnKey = ColumnKey::new("sample");

/// The headings of the `Plain` list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Heading {
    Recent,
    All,
}

/// Which gap a gap slot is: the one after a host group (before the next group or the families), or the one between
/// the recents and the other families.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Gap {
    Host(usize),
    Recents,
}

/// One position of the list: a host group's heading, a host row (`at` in group `group`), a family, one style of an
/// open family, a gap, a heading of the `Plain` list, or one of the host's action rows.
enum Slot {
    HostHeading { group: usize, name: Label },
    Host { group: usize, at: usize, row: HostFontRow },
    Family { row: FamilyRow, open: bool },
    Style { family: String, style: String },
    Gap(Gap),
    Heading(Heading),
    Action(usize),
}

/// What shapes the slots besides the rows.
struct Layout {
    /// Headings over the recents and the families, and the action rows after them: the `Plain` list with nothing typed.
    sections: bool,
    /// How many action rows follow the families.
    actions: usize,
}

/// What names a slot across passes. A family opening shifts the positions after it, so the list keys its rows by what
/// they are and the lead stays on its row.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum SlotKey {
    Gap(Gap),
    Heading(Heading),
    HostHeading(usize),
    Host { group: usize, at: usize },
    Action(usize),
    Family { family: String, recent: bool },
    Style { family: String, style: String },
}

impl Slot {
    fn key(&self) -> SlotKey {
        match self {
            Slot::HostHeading { group, .. } => SlotKey::HostHeading(*group),
            Slot::Host { group, at, .. } => SlotKey::Host { group: *group, at: *at },
            Slot::Family { row, .. } => SlotKey::Family { family: row.family.clone(), recent: row.recent },
            Slot::Style { family, style } => SlotKey::Style { family: family.clone(), style: style.clone() },
            Slot::Gap(gap) => SlotKey::Gap(*gap),
            Slot::Heading(heading) => SlotKey::Heading(*heading),
            Slot::Action(at) => SlotKey::Action(*at),
        }
    }

    /// The family the slot stands for, which the lead previews. A host row has no catalogue family to preview or
    /// rest on.
    fn family(&self) -> Option<&str> {
        match self {
            Slot::Family { row, .. } => Some(&row.family),
            Slot::Style { family, .. } => Some(family),
            Slot::HostHeading { .. } | Slot::Host { .. } | Slot::Gap(_) | Slot::Heading(_) | Slot::Action(_) => None,
        }
    }

    /// What picking the slot picks: the family, and the style of a style row.
    fn pick(&self) -> Option<(&str, Option<&str>)> {
        match self {
            Slot::Family { row, .. } => Some((&row.family, None)),
            Slot::Style { family, style } => Some((family, Some(style))),
            Slot::HostHeading { .. } | Slot::Host { .. } | Slot::Gap(_) | Slot::Heading(_) | Slot::Action(_) => None,
        }
    }
}

fn slot_of<'s>(slots: &'s [Slot], key: &SlotKey) -> Option<&'s Slot> {
    slots.iter().find(|slot| slot.key() == *key)
}

/// The list's slots as a collection source: a tree two levels deep. A family row is as high as its sample needs.
struct Slots<'a> {
    slots: &'a [Slot],
    /// The family rows' height.
    row: f32,
}

impl DisplayOrder<SlotKey> for Slots<'_> {
    fn len(&self) -> usize {
        self.slots.len()
    }

    fn key(&self, at: usize) -> SlotKey {
        self.slots[at].key()
    }

    fn index_of(&self, key: &SlotKey) -> Option<usize> {
        self.slots.iter().position(|slot| slot.key() == *key)
    }

    fn depth(&self, at: usize) -> usize {
        usize::from(matches!(self.slots[at], Slot::Style { .. }))
    }

    /// A family with several styles. A recent row has none: the family is listed again below the gap.
    fn has_children(&self, at: usize) -> bool {
        matches!(&self.slots[at], Slot::Family { row, .. } if row.styles > 1 && !row.recent)
    }

    fn open(&self, at: usize) -> bool {
        matches!(self.slots[at], Slot::Family { open: true, .. })
    }

    fn is_section(&self, at: usize) -> bool {
        matches!(self.slots[at], Slot::Gap(_) | Slot::Heading(_) | Slot::HostHeading { .. })
    }

    /// A notice is text, which neither the keys nor a click take.
    fn enabled(&self, at: usize) -> bool {
        !matches!(&self.slots[at], Slot::Host { row, .. } if matches!(row.kind, HostFontKind::Notice))
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        match &self.slots[at] {
            Slot::HostHeading { name, .. } => Some(Cow::Borrowed(name.as_str())),
            Slot::Host { row, .. } => Some(Cow::Borrowed(row.name.as_str())),
            Slot::Family { row, .. } => Some(Cow::Borrowed(row.shown.as_str())),
            Slot::Style { style, .. } => Some(Cow::Borrowed(style.as_str())),
            Slot::Gap(_) | Slot::Heading(_) | Slot::Action(_) => None,
        }
    }
}

impl ItemSource<SlotKey> for Slots<'_> {
    fn size_hint(&self, at: usize) -> SizeHint {
        match &self.slots[at] {
            Slot::Family { .. } => SizeHint::Height(self.row),
            Slot::Host { row, .. } if matches!(row.kind, HostFontKind::Face(_)) => SizeHint::Height(self.row),
            _ => SizeHint::Auto,
        }
    }

    fn toggled(&self, at: usize, column: &ColumnKey) -> bool {
        *column == STAR && matches!(&self.slots[at], Slot::Family { row, .. } if row.starred)
    }
}

/// The `shown` host rows (as `host_rows` lists them) by group, each under its group's heading, with a gap after each
/// group that has rows; then the family rows with a gap after the recents (headings over the recents and the
/// families under `layout.sections`), the styles of each expanded family under its row, and the action rows at the
/// end.
fn slots(
    catalog: &Catalog,
    host: &[HostFontGroup<'_>],
    shown: &[(usize, usize)],
    rows: Vec<FamilyRow>,
    expanded: &[String],
    layout: &Layout,
) -> Vec<Slot> {
    let recents = rows.iter().take_while(|row| row.recent).count();
    let mut slots: Vec<Slot> = Vec::with_capacity(shown.len() + rows.len() + 2);
    let mut last_group: Option<usize> = None;
    for &(group, at) in shown {
        let Some(of) = host.get(group) else { continue };
        let Some(row) = of.rows.get(at) else { continue };
        if last_group != Some(group) {
            if let Some(last) = last_group {
                slots.push(Slot::Gap(Gap::Host(last)));
            }
            if let Some(name) = &of.heading {
                slots.push(Slot::HostHeading { group, name: name.clone() });
            }
            last_group = Some(group);
        }
        slots.push(Slot::Host { group, at, row: row.clone() });
    }
    if let Some(last) = last_group
        && !rows.is_empty()
    {
        slots.push(Slot::Gap(Gap::Host(last)));
    }

    for (at, row) in rows.into_iter().enumerate() {
        if layout.sections {
            if at == 0 && recents > 0 {
                slots.push(Slot::Heading(Heading::Recent));
            }
            if at == recents {
                slots.push(Slot::Heading(Heading::All));
            }
        } else if at == recents && recents > 0 {
            slots.push(Slot::Gap(Gap::Recents));
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
    if layout.sections {
        slots.extend((0..layout.actions).map(Slot::Action));
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
fn filter_bar(ui: &mut egui::Ui, filters: &mut FontFilters, hide_classes: &mut bool, current: &str, words: &FontPickerWords) {
    Row::new().show(ui, |ui, _| {
        let (icon, label) = if *hide_classes { ("chevrons_right", &words.show_classes) } else { ("chevrons_left", &words.hide_classes) };
        ToggleButton::new(label.clone(), Bind::bound(hide_classes)).icon(IconId::new(icon)).icon_only().show(ui);
        ToggleButton::new(words.variable.clone(), Bind::bound(&mut filters.variable)).icon(IconId::new("font_variable")).icon_only().show(ui);
        let star = if filters.favorites { "star_filled" } else { "star" };
        ToggleButton::new(words.favorites.clone(), Bind::bound(&mut filters.favorites)).icon(IconId::new(star)).icon_only().show(ui);
        let mut similar = filters.similar.is_some();
        ToggleButton::new(words.similar.clone(), Bind::bound(&mut similar)).icon(IconId::new("font_similar")).icon_only().show(ui);
        if similar != filters.similar.is_some() {
            filters.similar = similar.then(|| current.to_owned());
        }
        let clear = Button::new(words.clear_all.clone()).variant(Variant::GHOST).enabled(filters.any()).show(ui);
        if clicked(&clear) {
            filters.clear();
        }
    });
}

/// The classes as a collection source: All Classes (key 0), then each class (its position in `FontClass::ALL` plus one).
struct Classes<'a>(&'a [Label]);

impl DisplayOrder<u8> for Classes<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> u8 {
        at as u8
    }

    fn index_of(&self, key: &u8) -> Option<usize> {
        (usize::from(*key) < self.0.len()).then_some(usize::from(*key))
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.0[at].as_str()))
    }
}

impl ItemSource<u8> for Classes<'_> {}

/// The class column: All Classes and the eight classes as a list of options, the chosen one selected.
fn class_column(ui: &mut egui::Ui, filters: &mut FontFilters, words: &FontPickerWords) {
    let chosen = filters.class.and_then(|c| FontClass::ALL.iter().position(|a| *a == c)).map_or(0u8, |i| i as u8 + 1);
    let labels: Vec<Label> =
        std::iter::once(words.all_classes.clone()).chain(FontClass::ALL.into_iter().map(|class| class_label(class, words))).collect();
    let mut selection = Selection::single(chosen);
    List::<u8>::options(words.classes.clone())
        .selection(Bind::bound(&mut selection))
        .show_items(ui, &Classes(&labels), &mut |key, _| ListRow::new(labels[usize::from(*key)].clone()).option().check(*key == chosen));
    let picked = selection.items.first().copied().unwrap_or(chosen);
    filters.class = usize::from(picked).checked_sub(1).and_then(|i| FontClass::ALL.get(i).copied());
}

/// The sizes the preview size row offers; the three largest sizes share the largest thumbnail.
const PREVIEW_SIZES: [FontPreviewSize; 3] = [FontPreviewSize::Small, FontPreviewSize::Medium, FontPreviewSize::Large];

fn script_label(script: FontScript, words: &FontPickerWords) -> Label {
    match script {
        FontScript::Latin => words.script_latin.clone(),
        FontScript::Japanese => words.script_japanese.clone(),
        FontScript::Thai => words.script_thai.clone(),
    }
}

/// A chip of an exclusive choice: on while `slot` holds `value`. A click sets `value`, or `off` when it was on.
fn choice_chip<T: PartialEq>(ui: &mut egui::Ui, label: Label, slot: &mut T, value: T, off: T) {
    let held = *slot == value;
    let mut on = held;
    ToggleButton::new(label, Bind::bound(&mut on)).show(ui);
    if on != held {
        *slot = if on { value } else { off };
    }
}

/// The chip of a host set: on while its key is in `keys` and the set is enabled.
fn set_chip(ui: &mut egui::Ui, set: &FamilySet<'_>, keys: &mut Vec<String>) {
    let held = set.enabled && keys.iter().any(|key| key == set.key);
    let mut on = held;
    ToggleButton::new(set.label.clone(), Bind::bound(&mut on)).enabled(set.enabled).show(ui);
    if on != held {
        keys.retain(|key| key != set.key);
        if on {
            keys.push(set.key.to_owned());
        }
    }
}

/// The `Chips` filter area: the script row; Favorites, the host sets, Any Kind and the classes; Font Technology and
/// Variable; the similar chip while Show Similar Fonts is on; and the Font Preview Size row while samples show.
fn filter_chips(
    ui: &mut egui::Ui,
    filters: &mut FontFilters,
    preview: &mut Option<FontPreviewSize>,
    sets: &[FamilySet<'_>],
    scripts: &[FontScript],
    words: &FontPickerWords,
) {
    Stack::new().show(ui, |ui, _| {
        if !scripts.is_empty() {
            Row::new().wrap(true).show(ui, |ui, _| {
                choice_chip(ui, words.all_scripts.clone(), &mut filters.script, None, None);
                for script in scripts {
                    choice_chip(ui, script_label(*script, words), &mut filters.script, Some(*script), None);
                }
            });
        }
        Row::new().wrap(true).show(ui, |ui, _| {
            ToggleButton::new(words.favorites.clone(), Bind::bound(&mut filters.favorites)).show(ui);
            for set in sets {
                set_chip(ui, set, &mut filters.sets);
            }
            choice_chip(ui, words.any_kind.clone(), &mut filters.class, None, None);
            for class in FontClass::ALL {
                choice_chip(ui, class_label(class, words), &mut filters.class, Some(class), None);
            }
        });
        for hint in sets.iter().filter(|set| !set.enabled).filter_map(|set| set.hint.as_ref()) {
            Text::new(hint.clone()).show(ui);
        }
        Row::new().wrap(true).show(ui, |ui, _| {
            Text::new(words.font_technology.clone()).show(ui);
            ToggleButton::new(words.variable.clone(), Bind::bound(&mut filters.variable)).show(ui);
        });
        if let Some(reference) = filters.similar.clone() {
            Row::new().show(ui, |ui, _| {
                Text::new(Label::from(words.similar_to.fill(&[("family", reference.as_str())]))).show(ui);
                let clear = IconButton::new(IconId::new("close")).label(words.clear_similar.clone()).show(ui);
                if clicked(&clear) {
                    filters.similar = None;
                }
            });
        }
        if let Some(size) = preview.as_mut().filter(|size| **size != FontPreviewSize::None) {
            let last = PREVIEW_SIZES.len() as u8 - 1;
            let before = PREVIEW_SIZES.iter().position(|s| s == size).map_or(last, |at| at as u8);
            let mut chosen = before;
            Text::new(words.preview_size.clone()).show(ui);
            RadioGroup::new(words.preview_size.clone(), Bind::bound(&mut chosen)).show(ui, |ui, _| {
                Row::new().wrap(true).show(ui, |ui, _| {
                    for (at, label) in [&words.preview_small, &words.preview_medium, &words.preview_large].into_iter().enumerate() {
                        Radio::new(at as u8).label(label.clone()).show(ui);
                    }
                });
            });
            if chosen != before
                && let Some(next) = PREVIEW_SIZES.get(usize::from(chosen))
            {
                *size = *next;
            }
        }
    });
}

/// The sample of a family row: its text ([`row_text`]) in the regular face of `family` at `size.side × SAMPLE_EM`, in a
/// band of the thumbnail side `size.side` as the banner's, named "{shown}: {text}" as the font manager names its
/// samples, so the row keeps the family's name to itself; no face for a family the catalog lacks.
fn sample<'a>(
    at: &impl FacesCx,
    catalog: &'a Arc<Catalog>,
    infos: &'a FamilyInfos,
    family: &str,
    shown: &str,
    text: &str,
    size: Sample,
) -> FontSample<'a> {
    let text = row_text(at, catalog, infos, family, text, shown);
    let face =
        infos.families.get(family).map_or(SampleFace::Missing, |info| SampleFace::Face { catalog, owner: Owner::Installed, face: &info.regular });
    FontSample::new(format!("{shown}: {text}"), text, face, size.side * SAMPLE_EM).band(size.side)
}

/// What `family`'s sample says ([`FamilyInfos::sample_text`]) once its regular face is ready; `text` while it loads,
/// when the sample shows nothing yet.
fn row_text(at: &impl FacesCx, catalog: &Arc<Catalog>, infos: &FamilyInfos, family: &str, text: &str, shown: &str) -> String {
    let Some(info) = infos.families.get(family) else { return text.to_owned() };
    match faces::shown(at, catalog, Owner::Installed, &info.regular) {
        Shown::Ready(_) => {
            faces::bytes(at, Owner::Installed, &info.regular).map_or_else(|| text.to_owned(), |face| infos.sample_text(family, text, shown, &face))
        }
        _ => text.to_owned(),
    }
}

/// The line of a host row: a face with its name as the sample in its own face, an action, or a notice that takes no
/// activation. A face row lays out in the list's columns as a family row does.
fn host_line<'r>(row: &'r HostFontRow, checked: bool, cells: &[(ColumnKey, egui::Rect)], size: ThumbSize, sample: Sample) -> ListRow<'r> {
    match &row.kind {
        HostFontKind::Notice => ListRow::new(row.name.clone()).depth(0).enabled(false),
        HostFontKind::Action => ListRow::new(row.name.clone()).option().depth(0),
        HostFontKind::Face(face) => {
            let line = ListRow::new(row.name.clone()).cells(cells).option().depth(0).check(checked);
            match face.as_ref().filter(|_| size != ThumbSize::None) {
                Some(font) => {
                    let text = row.name.as_str();
                    let sample = FontSample::new(format!("{text}: {text}"), text, SampleFace::Font(font), sample.side * SAMPLE_EM).band(sample.side);
                    line.cell_field(SAMPLE, sample, |_| {})
                }
                None => line,
            }
        }
    }
}

/// The name the banner shows for `family`: its row's shown name when the list holds one, else its name in the
/// interface language, else the family key.
fn banner_shown_name(slots: &[Slot], infos: &FamilyInfos, family: &str, language: &str, settings: Option<&FontPickerSettings>) -> String {
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
    let (ctx, owner) = (cx.stage().ctx().clone(), cx.base());
    let typing = cx.memory::<Typing>();
    typing.lead_by_pointer = by_pointer;
    if typing.lead.as_ref() != Some(&family) {
        typing.lead = Some(family.clone());
        typing.watch.arm(&ctx, owner);
        picker.preview = Preview::Shows(family);
    }
}

/// Ends the live preview, if one runs; the pass reports the end.
pub(super) fn end_preview<'a>(picker: &mut FontPicker<'a>, cx: &mut Cx<'_, FontPicker<'a>>) {
    let typing = cx.memory::<Typing>();
    typing.lead_by_pointer = false;
    typing.watch.disarm();
    if typing.lead.take().is_some() {
        picker.preview = Preview::Ended;
    }
}

/// The list popup: the host rows above, one option row per family, the current row selected, and the styles of the
/// open families under their rows. A pick comes back as `Activate` (Enter, Tab or a click), a family row the lead
/// moves to is reported as the preview (the field's value and text stay), and a star flips a favourite. A host face
/// or action row picked is reported as `host_picked` and proposes no family. Each pass reports only the events of its
/// own input.
pub(super) fn popup<'a>(picker: &mut FontPicker<'a>, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, FontPicker<'a>>) {
    let popup = cx.popup(POPUP);
    let words = cx.words::<FontPickerWords>();
    let (query, drafting, lead, expanded) = {
        let typing = cx.memory::<Typing>();
        (typing.draft.clone().unwrap_or_default(), typing.draft.is_some(), typing.lead.clone(), typing.expanded.clone())
    };
    let mut hide_classes = cx.memory::<Typing>().hide_classes;
    let family = cx.value().cloned().unwrap_or_default();
    let settings = read(&mut picker.settings, cx);
    let plain = picker.kind == FontPickerKind::Plain;
    // The plain list has no filters: neither the bar nor the column shows, and nothing narrows the list.
    let mut filters = read(&mut picker.filters, cx).filter(|_| !plain);
    let actions = picker.actions;
    let has_filters = filters.is_some();
    let chips = picker.filter_layout == FilterLayout::Chips;
    let on_white = settings.as_ref().is_some_and(|s| s.samples_on_white);
    let mut preview_size = settings.as_ref().map(|s| s.preview);
    let (sets, scripts) = (picker.sets, picker.scripts);
    let narrowing = filters.clone().unwrap_or_default();
    let catalog = picker.catalog;
    let infos = infos_of(&cx.response.ctx, cx.id, catalog);
    let language_code = ui_language(&cx.env);
    let (folder, collator) = (cx.env.folder(), cx.env.collator());
    let listing = Listing { query: &query, settings: settings.as_ref(), filters: &narrowing, language: &language_code, served: picker.served, sets };
    let (found, comparable) = list(catalog, &infos, &listing, &Language { folder: &folder, collator: &collator }, true);
    let layout = Layout { sections: plain && query.trim().is_empty(), actions: actions.len() };
    let shown_host = host_rows(picker.host, &query, &folder);
    let slots = slots(catalog, picker.host, &shown_host, found, &expanded, &layout);
    let current = match picker.current_host.as_deref() {
        Some(key) => slots.iter().position(|slot| matches!(slot, Slot::Host { row, .. } if row.key == key)),
        None => slots.iter().position(|slot| matches!(slot, Slot::Family { row, .. } if row.family == family)),
    };
    let size = settings.as_ref().map_or(ThumbSize::None, |s| s.preview.thumb());
    let sample_size = cx.style.sample(size);
    let mut columns = Vec::with_capacity(4);
    if !plain {
        columns.push(ColumnSpec::toggle(STAR, words.favorites.clone(), ToggleColumn::default()).icon(IconId::new("star")));
    }
    columns.push(ColumnSpec::new(NAME, words.list.clone(), ColumnWidth::Content).at_least(cx.style.name_column));
    if size != ThumbSize::None {
        columns.push(ColumnSpec::new(SAMPLE, words.sample.clone(), ColumnWidth::Fixed(sample_size.side * SAMPLE_WIDTH)));
    }
    let favorites_empty = settings.as_ref().is_none_or(|s| s.favorites.is_empty());
    // Rows are keyed, so the stamp names only what rebuilds the list: opening a family keeps the lead.
    let mut stamp = DefaultHasher::new();
    (&query, narrowing.class, narrowing.favorites, narrowing.variable, &narrowing.similar, narrowing.script, &narrowing.sets, on_white, chips)
        .hash(&mut stamp);
    let banner_family = lead.unwrap_or_else(|| family.clone());
    // The library picker always shows the banner, with the sample word when the host gives no text.
    let banner_text = picker.preview_text.or((!plain).then_some(words.sample.as_str()));
    // Every row's sample is the banner's text: the host's text, else the sample word.
    let sample_text = picker.preview_text.unwrap_or(words.sample.as_str());
    let name = words.list.clone();
    let mut stars: Vec<(String, bool)> = Vec::new();
    let mut over_rows = false;
    let mut under_pointer: Option<SlotKey> = None;
    // The field's list keys go to the family list, not to the class column beside it.
    let families = cx.id.with("families");
    let overlay = Overlay::dropdown().dismiss(Dismiss::new().focus_left(true)).list(families);
    let faces_at = &*cx;
    let events = ui.show(popup, overlay, |ui, _| {
        let mut events = Vec::new();
        Stack::new().show(ui, |ui, _| {
            if let Some(text) = banner_text {
                let shown_name = banner_shown_name(&slots, &infos, &banner_family, &language_code, settings.as_ref());
                let face = infos.families.get(&banner_family).map_or(SampleFace::Missing, |info| SampleFace::Face {
                    catalog,
                    owner: Owner::Installed,
                    face: &info.regular,
                });
                let label = words.preview_of.fill(&[("family", shown_name.as_str())]);
                // What the family's row shows: the text, or its name or first glyphs when the face can't draw it.
                let shown_text = row_text(faces_at, catalog, &infos, &banner_family, text, &shown_name);
                FontPreviewBanner::new(label, &shown_name, &shown_text, face).on_white(on_white).show(ui);
            }
            if let Some(filters) = filters.as_mut() {
                if chips {
                    filter_chips(ui, filters, &mut preview_size, sets, scripts, &words);
                } else {
                    filter_bar(ui, filters, &mut hide_classes, &family, &words);
                }
                Divider::horizontal().show(ui);
            }
            Row::new().show(ui, |ui, _| {
                if let (Some(filters), false, false) = (filters.as_mut(), hide_classes, chips) {
                    class_column(ui, filters, &words);
                }
                Stack::new().show(ui, |ui, _| {
                    if narrowing.similar.is_some() && !comparable {
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
                    let source = Slots { slots: &slots, row: sample_size.row };
                    // The library list keeps the disclosure column on every row, so names stay put while a query
                    // narrows the list; the plain list has no styles to open.
                    let leaves = if plain { TreeLeaves::Compact } else { TreeLeaves::Aligned };
                    let selection =
                        current.filter(|_| query.trim().is_empty()).map_or_else(Selection::default, |at| Selection::single(slots[at].key()));
                    let rows = List::<SlotKey>::options(name.clone())
                        .id(families)
                        .tree(true)
                        .tree_leaves(leaves)
                        .columns(columns.clone())
                        .header(false)
                        .selection(Bind::given(selection))
                        .rows_stamp(stamp.finish())
                        .show_items(ui, &source, &mut |_, item| match &slots[item.at] {
                            Slot::Gap(_) => ListRow::separator(),
                            Slot::HostHeading { name, .. } => ListRow::new(name.clone()).heading(),
                            Slot::Host { row, .. } => host_line(row, current == Some(item.at), item.cells(), size, sample_size),
                            Slot::Heading(Heading::Recent) => ListRow::new(words.recently_used.clone()).section_heading(),
                            Slot::Heading(Heading::All) => ListRow::new(words.all_fonts.clone()).section_heading(),
                            Slot::Action(at) => ListRow::new(actions[*at].clone()).option().depth(0),
                            Slot::Style { style, .. } => {
                                ListRow::new(style.clone()).cells(item.cells()).in_label_column().option().depth(1).check(false)
                            }
                            Slot::Family { row, open } => {
                                let mut line = ListRow::new(row.shown.clone()).cells(item.cells()).option().depth(0).check(current == Some(item.at));
                                // The plain list has no filters, so `has_filters` keeps the similar part off it too.
                                if chips && has_filters {
                                    let similar = TrailingFace::Icon(IconSpec::svg(IconId::new("font_similar")));
                                    let similar_name = Label::from(words.show_similar.fill(&[("row", row.shown.as_str())]));
                                    line = line.trailing_part(SIMILAR, similar, similar_name);
                                }
                                if !plain {
                                    let face = ToggleFace::icons(IconId::new("star_filled"), IconId::new("star"));
                                    let name = Label::from(words.favorite.fill(&[("row", row.shown.as_str())]));
                                    line = line.toggle(ToggleCell::new(STAR, face, ToggleState::of(row.starred), name).always_visible());
                                }
                                if source.has_children(item.at) {
                                    line = line.group(*open);
                                }
                                line = match row.format {
                                    Some(format) if !plain => {
                                        line.trailing_named(Label::from(format.badge().to_owned()), format_label(format, &words))
                                    }
                                    Some(_) => line,
                                    None => line.trailing_named(words.served.clone(), words.served.clone()),
                                };
                                if size == ThumbSize::None {
                                    return line;
                                }
                                let sample = sample(faces_at, catalog, &infos, &row.family, &row.shown, sample_text, sample_size).on_white(on_white);
                                line.cell_field(SAMPLE, sample, |_| {})
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
                if let Some(slot) = slot_of(&slots, &key) {
                    match slot.family() {
                        Some(family) => show_preview(picker, cx, family.to_owned(), by == LeadSource::Pointer),
                        // A host row previews nothing, so the family the lead left stops showing.
                        None => end_preview(picker, cx),
                    }
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
            CollectionEvent::Item(key, RowEvent::Trailing { part, .. }) if part == SIMILAR => {
                if let (Some(filters), Some(Slot::Family { row, .. })) = (filters.as_mut(), slot_of(&slots, &key)) {
                    filters.similar = Some(row.family.clone());
                }
            }
            CollectionEvent::Toggle(edit) => {
                if let Some(toggle) = edit.value()
                    && toggle.column == STAR
                    && let Some(Slot::Family { row, .. }) = slot_of(&slots, &toggle.key)
                {
                    stars.push((row.family.clone(), toggle.on));
                }
            }
            _ => {}
        }
    }
    let typing = cx.memory::<Typing>();
    typing.hide_classes = hide_classes;
    typing.over_rows = over_rows;
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
    if let Some(mut changed) = settings.clone() {
        for (family, on) in &stars {
            changed.set_favorite(family, *on);
        }
        if let Some(size) = preview_size {
            changed.preview = size;
        }
        if Some(&changed) != settings.as_ref() {
            let label = if stars.is_empty() { "Font preview size" } else { "Favorite font" };
            write(&mut picker.settings, cx, changed, label);
        }
    }
    if has_filters
        && filters != Some(narrowing)
        && let Some(filters) = filters
    {
        write(&mut picker.filters, cx, filters, "Font filters");
    }
    let chosen = picked.and_then(|key| slot_of(&slots, &key));
    if let Some(Slot::Action(at)) = chosen {
        picker.action = Some(*at);
        end_preview(picker, cx);
        popup.close(cx);
    } else if let Some(Slot::Host { row, .. }) = chosen
        && !matches!(row.kind, HostFontKind::Notice)
    {
        picker.host_picked = Some(row.key.clone());
        end_preview(picker, cx);
        // A host pick writes no family, so a typed query goes back to the field's text.
        if drafting {
            cx.set_draft(None);
        }
        popup.close(cx);
    } else if let Some((family, style)) = chosen.and_then(Slot::pick) {
        picker.picked = Some(family.to_owned());
        picker.picked_style = style.map(str::to_owned);
        end_preview(picker, cx);
        // A press on a row can end a typed name first, and that commit may already have set this family.
        if cx.value().map(String::as_str) != Some(family) {
            cx.propose(family.to_owned());
            cx.commit();
        }
        popup.close(cx);
    }
}
