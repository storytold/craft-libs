//! `ComboBox`: a text part with a chevron that filters a list of options as the user types.
//!
//! The text is the value: any text can be typed, and an option is a quick way to type its text. The list is the
//! component's own popup on the dropdown preset, so focus stays in the text part and the host hands the list keys to
//! the list. The option the keys make active puts its text in the field; the list's Escape puts back what was typed.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::dress::RowDress;
use super::rows::EntryRows;
use super::words::PickerWords;
use crate::buttons::IconButton;
use crate::contract::{
    Bind, BindMut, CollectionEvent, Common, Component, Cx, Edit as _, EditResponse, Entry, Focus as _, Input, LeadSource, MeasureCx, Outcome,
    Overlay, OverlayUi, Overlays as _, PaintUi, PartsCx, Pointer as _, Props as _, Remember as _, Role, ShowCollection as _, WordSetCx as _,
    step_stop, step_stops,
};
use crate::list::{List, ListRow};
use crate::primitives::{TextKey, TextOptions, TextRun};
use crate::runtime::env::Label;
use crate::runtime::overlay::CloseReason;
use crate::text::{Adornment, place, reserve};
use craft_behavior::{HitLevel, HitPriority, NavMove, PartKey, PlainText, Preview, Selection, Sense};
use craft_i18n::matcher::Matcher;
use egui::{Rect, Response, Vec2};

/// The text part.
const TEXT: PartKey = PartKey::new("text");
/// The chevron that opens and closes the list.
const CHEVRON: PartKey = PartKey::new("chevron");

fn text_key() -> TextKey<String> {
    TextKey::new(TEXT)
}

/// What the combo box keeps between passes: the draft the list was last filtered for, what the keys in the open
/// list put in, and the option text the open list previews.
#[derive(Clone, Default)]
struct Typing {
    draft: Option<String>,
    keyed: Option<Keyed>,
    /// The option text the preview shows; a pass reports a preview only when this changes.
    lead: Option<String>,
    /// The pointer proposed the lead, so it ends when the pointer leaves the rows (a keyed row keeps its preview).
    lead_by_pointer: bool,
}

/// The option text the keys in the open list put in the field, and the draft from before the keys (`None` when the keys
/// began the typing run), which the list's Escape puts back.
#[derive(Clone)]
struct Keyed {
    shown: String,
    before: Option<String>,
}

/// What a [`ComboBox`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ComboBoxOutput {
    /// The field's edit.
    pub edit: EditResponse<String>,
    /// What this pass did to the preview of the option under the pointer or the keyboard row of the open list: it ends
    /// when the pointer leaves the rows, the list closes or a row is picked. The field's text and value stay.
    pub preview: Preview<String>,
}

impl Outcome for ComboBoxOutput {
    type Value = String;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// A text field with a list of options. The typed text filters the list by the options' labels: the labels that
/// match it best first, ignoring case and accents. The chevron and Alt+Down open the list on every option. Enter or a
/// click on a row puts the option's text in the field; Escape closes the list, and the next Escape restores the text.
/// Up, Down and the wheel on the focused, closed field step to the previous or next stepped option ([`Entry::stepped`]).
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(overlay)]
pub struct ComboBox<'a> {
    #[common]
    common: Common,
    value: Bind<'a, String>,
    options: &'a [Entry<String>],
    /// Shown while the text is empty.
    #[prop(into)]
    placeholder: Option<Label>,
    /// Adds the host's content to each option's row.
    #[prop(func = "Fn(&String, ListRow<'a>) -> ListRow<'a>")]
    decorate: Option<RowDress<'a, String>>,
    empty: String,
    /// What this pass did to the preview, reported in [`ComboBoxOutput::preview`].
    preview: Preview<String>,
    /// Whether the combo box sizes to its options alone, as in a toolbar or ribbon.
    #[style_key]
    #[prop]
    compact: bool,
}

impl<'a> ComboBox<'a> {
    /// A combo box named `label` over the text `value`, offering `options`; an option's value is the text it puts in
    /// the field.
    pub fn new(label: impl Into<Label>, value: Bind<'a, String>, options: &'a [Entry<String>]) -> Self {
        ComboBox {
            common: Common::new(),
            value,
            options,
            placeholder: None,
            decorate: None,
            empty: String::new(),
            preview: Preview::Unchanged,
            compact: false,
        }
        .label(label)
    }

    /// Previews `text`, led by the pointer or by the keys. The pass reports it only when the previewed text changes,
    /// so a second pass of a frame, which sees the lead its first pass stored, reports nothing.
    fn show_preview(&mut self, cx: &mut Cx<'_, Self>, text: String, by_pointer: bool) {
        let typing = cx.memory::<Typing>();
        typing.lead_by_pointer = by_pointer;
        if typing.lead.as_ref() != Some(&text) {
            typing.lead = Some(text.clone());
            self.preview = Preview::Shows(text);
        }
    }

    /// Ends the preview, if one runs; the pass reports the end once.
    fn end_preview(&mut self, cx: &mut Cx<'_, Self>) {
        let typing = cx.memory::<Typing>();
        typing.lead_by_pointer = false;
        if typing.lead.take().is_some() {
            self.preview = Preview::Ended;
        }
    }

    fn text_options(&self) -> TextOptions {
        let mut options = TextOptions::new();
        options.placeholder = self.placeholder.clone();
        options
    }

    fn trailing(name: Label, arrow: crate::controls::disclosure::DropdownArrow) -> [Adornment; 1] {
        [Adornment::button(CHEVRON, crate::controls::disclosure::DisclosureGlyph::dropdown_of(arrow), name)]
    }

    /// Whether the list has anything to offer: a list of separators and headings opens nothing.
    fn has_items(&self) -> bool {
        self.options.iter().any(Entry::is_item)
    }

    /// Whether any option's label matches `query`.
    fn matches_any(&self, cx: &Cx<'_, Self>, query: &str) -> bool {
        let matcher = Matcher::new(&cx.env.folder(), query);
        self.options.iter().filter(|option| option.is_item()).any(|option| matcher.score(cx.env.resolve(option.label()).as_str()).is_some())
    }

    /// The option whose text steps from the field's text in the direction of the move, over the stepped options.
    fn neighbour(&self, text: &str, forward: bool) -> Option<usize> {
        let here = self.options.iter().position(|option| option.value().is_some_and(|value| value == text));
        step_stop(&step_stops(self.options), here, forward)
    }
}

impl Component for ComboBox<'_> {
    const ROLE: Role = Role::COMBO_BOX;
    const INPUT: Input = Input::Text;
    const KEYS: crate::runtime::focus::KeyUse = crate::runtime::focus::KeyUse::of(crate::contract::Input::Text).with_moves();
    type Style = crate::text::FieldStyle;
    type Output = ComboBoxOutput;

    /// A field; in a ribbon (`ControlPlace::Ribbon`) or toolbar (`InBar`), or when `compact` is set,
    /// sized by its options alone, as a ribbon's font size box is.
    fn style(cx: &crate::style::StyleCx<'_>, key: ComboBoxStyleKey) -> crate::text::FieldStyle {
        let field = crate::text::FieldStyle::at(cx);
        let in_bar = cx.env::<crate::contract::InBar>().0;
        let ribbon = cx.env::<crate::buttons::ControlPlace>() == crate::buttons::ControlPlace::Ribbon;
        if key.compact || ribbon || in_bar { field.sized_by_content() } else { field }
    }

    /// The wheel steps the text through the options.
    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS | Sense::WHEEL_Y
    }

    /// The widest of the current text, the placeholder and the options, and the room of the chevron: the field keeps
    /// its size whichever option is picked.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let current = cx.value().map_or("", String::as_str);
        let placeholder = self.placeholder.as_ref().map_or("", Label::as_str);
        let texts = self.options.iter().filter_map(|option| option.value().map(String::as_str)).chain([current, placeholder]);
        let widest = texts.map(|text| cx.field_text(&TextRun::new(text))).fold(Vec2::ZERO, Vec2::max);
        let name = cx.words::<PickerWords>().combo_options.clone();
        Vec2::new(widest.x + reserve(cx.style, &[], &Self::trailing(name, cx.env.get())), widest.y)
    }

    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let value = cx.value();
        let name = cx.words::<PickerWords>().combo_options.clone();
        let trailing = Self::trailing(name, cx.env.get());
        let placed = place(cx.content_rect, cx.style, &[], &trailing, cx.flow.is_rtl());
        if let Some(at) = placed.trailing.first() {
            let chevron = cx.part(CHEVRON, *at);
            if self.has_items() {
                chevron
                    .sense(Sense::CLICK | Sense::KEEP_FOCUS)
                    .priority(HitPriority::new(HitLevel::Handle, 0))
                    .pointer_only("Alt+Down opens the list")
                    .toggles("options");
            } else {
                chevron.inactive(true);
            }
        }
        if let Some(value) = value.or(Some(&self.empty)) {
            cx.text(text_key(), placed.text, &PlainText, value, &self.text_options());
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, String>> {
        Some(self.value.reborrow())
    }

    /// A changed draft filters the list: it opens on the matches, and closes when there are none. A draft that ended
    /// (committed, cancelled or replaced by a pick) closes it. The text the keys in the open list put in is not
    /// typing: the list stays filtered by what was typed. While the list is closed, Up, Down and the wheel step
    /// through the options.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let list = cx.popup("options");
        let draft = cx.text(&text_key()).and_then(|outcome| outcome.draft.clone());
        let typing = cx.memory::<Typing>();
        let by_keys = typing.keyed.as_ref().is_some_and(|keyed| draft.as_ref() == Some(&keyed.shown));
        let changed = !by_keys && typing.draft != draft;
        if changed {
            typing.keyed = None;
            typing.draft.clone_from(&draft);
        }
        if changed {
            match &draft {
                Some(text) if self.matches_any(cx, text) => list.open(cx),
                _ => list.close(cx),
            }
        }
        if list.is_open(cx) {
            return;
        }
        // A list closed by any path (a pick, Escape, a click outside, the draft ending) ends its preview.
        self.end_preview(cx);
        let text = cx.value().cloned().unwrap_or_default();
        while let Some((step, mods)) = cx.intents().take_move() {
            let forward = match step {
                NavMove::Down if !mods.fine => true,
                NavMove::Up if !mods.fine => false,
                _ => continue,
            };
            if let Some(value) = self.neighbour(&text, forward).and_then(|at| self.options[at].value()) {
                cx.set(value.clone());
            }
        }
        if let Some(wheel) = cx.take_wheel()
            && wheel.scroll.y != 0.0
            && let Some(value) = self.neighbour(&text, wheel.scroll.y < 0.0).and_then(|at| self.options[at].value())
        {
            cx.set(value.clone());
        }
    }

    /// The list: one option row per match, the row of the field's text checked and active when the list opens. The
    /// row the keys make active puts its text in the field as a change of the typing run; the list's Escape puts back
    /// the text from before the keys. A pick comes back as `Activate` (Enter or a click) and puts the option's text
    /// in the field. The row under the pointer or the keys is previewed (the field's value stays); the preview ends
    /// when the pointer leaves the rows, a row is picked or the list closes. Each pass reports only its own input's
    /// events, so a rerun pass with no new lead reports `Unchanged`.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        crate::menu::text_menu(ui, cx, &self.text_options());
        let popup = cx.popup("options");
        let options = self.options;
        let query = cx.memory::<Typing>().draft.clone().unwrap_or_default();
        let labels: Vec<Label> = options.iter().map(|option| cx.env.resolve(option.label())).collect();
        let current = cx.value().and_then(|text| options.iter().position(|option| option.value() == Some(text)));
        let name = cx.label();
        let decorate = self.decorate.as_ref();
        let rows = EntryRows::new(options, &labels);
        let mut over_rows = false;
        let mut under_pointer: Option<usize> = None;
        let list = cx.id.with("options");
        let events = ui.show(popup, Overlay::dropdown().list(list), |ui, _| {
            let selection = current.filter(|_| query.trim().is_empty()).map_or_else(Selection::default, Selection::single);
            let shown = List::<usize>::options(name.clone())
                .id(list)
                .selection(Bind::given(selection))
                .filter(&query, |key| vec![labels[*key].clone()])
                .ranked(true)
                .show_items(ui, &rows, &mut |key, _| rows.row(*key, current, decorate, None));
            under_pointer = shown.shown.iter().find(|row| row.hovered).map(|row| row.key);
            over_rows = under_pointer.is_some();
            shown.events
        });
        let (mut picked, mut keyed_to, mut pointed_to) = (None, None, None);
        for event in events.into_iter().flatten() {
            match event {
                CollectionEvent::Activate(key) => picked = Some(key),
                CollectionEvent::LeadMoved { key, by: LeadSource::Keys, .. } => keyed_to = Some(key),
                CollectionEvent::LeadMoved { key, by: LeadSource::Pointer, .. } => pointed_to = Some(key),
                _ => {}
            }
        }
        if let Some(value) = picked.and_then(|key| options.get(key)).and_then(Entry::value) {
            cx.memory::<Typing>().keyed = None;
            self.end_preview(cx);
            cx.set(value.clone());
            popup.close(cx);
        } else if let Some(text) = keyed_to.and_then(|key| options.get(key)).and_then(Entry::value) {
            let draft = cx.text(&text_key()).and_then(|outcome| outcome.draft.clone());
            let typing = cx.memory::<Typing>();
            let before = typing.keyed.take().map_or(draft, |keyed| keyed.before);
            typing.keyed = Some(Keyed { shown: text.clone(), before });
            cx.set_draft(Some(text.clone()));
            self.show_preview(cx, text.clone(), false);
        } else if let Some(text) = pointed_to.and_then(|key| options.get(key)).and_then(Entry::value) {
            self.show_preview(cx, text.clone(), true);
        }
        // A preview the pointer proposed ends when the pointer leaves the rows. The list keeps its lead on the row, so
        // the pointer coming back to that row reports no move: the row under it proposes again.
        let (by_pointer, led) = {
            let typing = cx.memory::<Typing>();
            (typing.lead_by_pointer, typing.lead.is_some())
        };
        if picked.is_none() {
            if by_pointer && !over_rows {
                self.end_preview(cx);
            } else if over_rows
                && !led
                && let Some(text) = under_pointer.and_then(|key| options.get(key)).and_then(Entry::value)
            {
                self.show_preview(cx, text.clone(), true);
            }
        }
        if ui.closing(popup).is_some() {
            self.end_preview(cx);
        }
        match ui.closing(popup) {
            Some(CloseReason::Escape) => {
                if let Some(keyed) = cx.memory::<Typing>().keyed.take() {
                    cx.set_draft(keyed.before);
                }
            }
            Some(_) => cx.memory::<Typing>().keyed = None,
            None => {}
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let name = cx.words::<PickerWords>().combo_options.clone();
        ui.face(CHEVRON, IconButton::new(crate::controls::disclosure::DisclosureGlyph::dropdown_of(cx.env.get())).label(name));
    }

    fn finish(self, cx: Cx<'_, Self>) -> ComboBoxOutput {
        ComboBoxOutput { edit: cx.edit_response(), preview: self.preview }
    }
}
