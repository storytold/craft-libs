//! `Select`: a closed field that opens a listbox of entries and feeds the picked one to its binding (15.4, 15.5).
//!
//! The field is a leaf that shows a `SelectFace`; its listbox is the component's own popup, shown from the `overlay`
//! hook on the dropdown preset. Focus stays on the field: the host hands the list keys to the listbox while it is
//! open, and Escape closes it without a change.

use super::dress::RowDress;
use super::face::{SelectFace, toggle_on_key};
use super::rows::EntryRows;
use super::words::PickerWords;
use crate::buttons::IconButton;
use crate::contract::overlay::{Popup, Trigger};
use crate::contract::{
    Access, Bind, BindMut, BodyCx, CollectionEvent, Common, Component, Cx, Edit as _, EditResponse, Entry, Focus as _, LeadOnOpen, LeadSource,
    MeasureCx, Numeric, Overlay, OverlayUi, Overlays as _, PaintUi, PartsCx, Pointer as _, PopupKind, Props as _, Remember as _, Role, Show as _,
    ShowCollection as _, ShowContainer as _, WordSetCx as _, step_stop, step_stops,
};
use crate::list::{List, ListRow};
use crate::menu::Menu;
use crate::runtime::env::Label;
use crate::runtime::focus::{self, FocusTarget};
use crate::style::{ComponentStyle, Ink, StyleCx, SurfaceLook, TextLook, TypeRole};
use crate::text::TextField;
use craft_behavior::nav::{StepResult, ValueAxes, ValueSteps};
use craft_behavior::{
    CancelReason, GesturePhase, HitLevel, HitPriority, IconId, Metrics, NavMove, PartKey, Selection, Sense, TypeAhead, Value, wheel_step,
};
use craft_theme::{Stateful, States};
use egui::{Ui, Vec2};
use std::borrow::Cow;

#[cfg(feature = "testing")]
mod stories;

/// The part that shows the face over the whole field and opens the list on a click.
const FACE: PartKey = PartKey::new("face");

/// The style of a select: the field well and focus edge, which the face is shown over, and the text and muted inks
/// the face reads as its host.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SelectStyle {
    #[style(surface)]
    surface: SurfaceLook,
    /// No padding of its own: the face carries the field's.
    #[style(metrics)]
    metrics: Metrics,
    /// The face's text.
    #[style(text)]
    text: TextLook,
    /// The face's placeholder text.
    #[style(text)]
    muted: TextLook,
}

/// The icon of the button in the popup header that opens the header menu.
const HEADER_MENU_ICON: &str = "more";

/// The rows of the menu a select's popup header opens: the host's menu body.
struct HeaderMenu<'a>(Box<HeaderRows<'a>>);

type HeaderRows<'a> = dyn FnMut(&mut Ui, &mut BodyCx<'_>) + 'a;

impl<'a> HeaderMenu<'a> {
    fn new(rows: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'a) -> Self {
        HeaderMenu(Box::new(rows))
    }
}

/// A field that shows the picked entry and opens a listbox of all of them. A press, Enter, Space or Alt+Down opens the
/// listbox with the picked entry active, and typing opens it on the first entry the typed letters begin. The open list
/// is one gesture: each entry the keys (Up, Down, Page keys, Home, End, type-ahead) make active is proposed as the
/// value, Enter or a click picks, and Escape, Alt+Up or a press outside close it and put the value back. Closed, the
/// arrow keys, Home, End and the wheel (while the field is focused or armed) step the pick through the stepped entries
/// ([`Entry::stepped`]); from a value that is none of them, forward lands on the first and back on the last. The field is as wide as its widest entry, placeholder or mixed word, so it keeps its
/// width whichever is shown.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(overlay, probe(Select<'static, u8>))]
pub struct Select<'a, T: Value + 'static> {
    #[common]
    common: Common,
    value: Bind<'a, T>,
    entries: &'a [Entry<T>],
    /// A filter field above the entries narrows them as the user types.
    #[prop(flag)]
    searchable: bool,
    /// Shown, muted, while the value is none of the entries.
    #[prop(into)]
    placeholder: Option<Label>,
    /// Adds the host's content to each entry's row.
    #[prop(func = "Fn(&T, ListRow<'a>) -> ListRow<'a>")]
    decorate: Option<RowDress<'a, T>>,
    /// The row under the pointer is proposed as the value while the list is open, as the rows the keys reach are;
    /// closing the list without a pick restores the value.
    #[prop(flag)]
    preview: bool,
    /// A button in the popup header opens a menu of these rows (a list's own options: sort, view).
    #[prop(func = "FnMut(&mut Ui, &mut BodyCx<'_>)")]
    header_menu: Option<HeaderMenu<'a>>,
}

impl<'a, T: Value + 'static> Select<'a, T> {
    /// A select named `label` over `value`, choosing among `entries`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, T>, entries: &'a [Entry<T>]) -> Self {
        Select { common: Common::new(), value, entries, searchable: false, placeholder: None, decorate: None, preview: false, header_menu: None }
            .label(label)
    }

    fn position(&self, value: Option<&T>) -> Option<usize> {
        value.and_then(|value| self.entries.iter().position(|entry| entry.value() == Some(value)))
    }

    /// Sets the value to the entry at `at` unless it is the value already.
    fn pick(&self, cx: &mut Cx<'_, Self>, at: usize) {
        if let Some(value) = self.entries[at].value()
            && self.position(cx.value()) != Some(at)
        {
            cx.set(value.clone());
        }
    }
}

/// The stop a move over the closed field reaches: `stops` are the positions of the stepped entries and `from` the
/// position of the current value.
fn step(stops: &[usize], from: Option<usize>, mv: NavMove) -> Option<usize> {
    match mv {
        NavMove::Up | NavMove::Back => step_stop(stops, from, false),
        NavMove::Down | NavMove::Forward => step_stop(stops, from, true),
        NavMove::Start => stops.first().copied(),
        NavMove::End => stops.last().copied(),
        _ => None,
    }
}

impl<T: Value + 'static> Component for Select<'_, T> {
    const ROLE: Role = Role::COMBO_BOX;
    const KEYS: crate::runtime::focus::KeyUse = crate::runtime::focus::KeyUse::of(crate::contract::Input::Pointer).with_moves().with_type_ahead();
    type Style = SelectStyle;
    type Output = EditResponse<T>;

    fn style(cx: &StyleCx<'_>, _: SelectStyleKey<T>) -> SelectStyle {
        let off = cx.color(cx.tokens.color.text.disabled);
        let (ink, muted) = (cx.ink(Ink::OnPanel), cx.color(cx.tokens.color.text.faint));
        SelectStyle {
            surface: cx.dropdown(),
            metrics: Metrics::new(),
            text: TextLook::new(TypeRole::Body, Stateful::new(ink).off(States::new(ink, ink, ink, off))),
            muted: TextLook::new(TypeRole::Body, Stateful::new(muted).off(States::new(muted, muted, muted, off))),
        }
    }

    /// The widest face of an entry, the placeholder and the mixed word, so the field keeps its size whichever is shown.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let mixed = cx.words::<PickerWords>().mixed.clone();
        let entries = self.entries.iter().filter(|entry| !entry.is_separator()).map(Entry::label);
        let texts = entries.chain(self.placeholder.as_ref()).chain(std::iter::once(&mixed));
        texts.map(|text| cx.face(SelectFace::new(cx.env.resolve(text)))).fold(Vec2::ZERO, Vec2::max)
    }

    fn bind(&mut self) -> Option<BindMut<'_, T>> {
        Some(self.value.reborrow())
    }

    fn parts(&mut self, rect: egui::Rect, cx: &mut PartsCx<'_, Self>) {
        cx.part(FACE, rect)
            .sense(Sense::CLICK | Sense::HOVER | Sense::TAKE_FOCUS)
            .priority(HitPriority::new(HitLevel::Body, 0))
            .pointer_only("Enter, Space and Alt+Down open the list")
            .toggles("list");
    }

    /// Enter and Space toggle the listbox; a press on the face toggles it through the part. While it is closed, typed
    /// letters open it on the first entry they begin, the arrow keys, Home, End and the wheel step the pick through the
    /// stepped entries, and AccessKit sets it. A proposal the list closed on without a pick is cancelled.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let list = cx.popup("list");
        toggle_on_key(cx, list);
        if !list.is_open(cx) && matches!(cx.gesture(), Some(GesturePhase::Begin | GesturePhase::Update)) {
            cx.cancel(CancelReason::Dismissed);
        }
        let stops = step_stops(self.entries);
        if list.is_open(cx) || stops.is_empty() {
            return;
        }
        let held = self.position(cx.value());
        while let Some(text) = cx.intents().take_text() {
            let labels: Vec<Label> = self.entries.iter().map(|entry| cx.env.resolve(entry.label())).collect();
            let typed = text.to_lowercase();
            let found = TypeAhead::pick(&typed, held, labels.len(), |at| Cow::Borrowed(labels[at].as_str()));
            if let Some(value) = found.filter(|&at| self.entries[at].is_enabled()).and_then(|at| self.entries[at].value()) {
                cx.propose(value.clone());
                list.open(cx);
                return;
            }
        }
        if let Some(at) = cx.intents().take_set_value() {
            let to = (at.round().max(0.0) as usize).min(stops.len() - 1);
            self.pick(cx, stops[to]);
        }
        while let Some((mv, _)) = cx.intents().take_move() {
            if let Some(to) = step(&stops, self.position(cx.value()), mv) {
                self.pick(cx, to);
            }
        }
        // The wheel gives one notch, up for a step forward.
        if let Some(wheel) = cx.take_wheel()
            && let StepResult::Set(along) =
                wheel_step(&ValueSteps::new(0.0, -1.0, 1.0, 1.0, 1.0), ValueAxes::Both, &wheel, cx.env.direction().is_rtl())
            && along != 0.0
            && let Some(to) = step_stop(&stops, held, along > 0.0)
        {
            self.pick(cx, to);
        }
    }

    /// The picked entry's place among the entries that can be picked, shown as its label, so AccessKit steps and sets it.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let stops = step_stops(self.entries);
        let range = 0.0..=stops.len().saturating_sub(1) as f64;
        let held = self.position(cx.value());
        let numeric = match held.and_then(|held| stops.iter().position(|&stop| stop == held)) {
            Some(at) => Numeric::new(at as f64, range).text(cx.env.resolve(self.entries[stops[at]].label()).as_str()),
            None => Numeric::empty(range),
        };
        access.numeric(numeric.step(1.0));
    }

    /// The listbox: one option row per entry, a spacer for a separator, the picked entry's row selected. Each row the
    /// keys move the lead to is proposed, and with a preview each row the pointer moves it to; a pick comes back from
    /// the list as Enter or a click on a row and commits.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("list");
        let entries = self.entries;
        let labels: Vec<Label> = entries.iter().map(|entry| cx.env.resolve(entry.label())).collect();
        let name = cx.label();
        let words = cx.words::<PickerWords>();
        let held = self.position(cx.value());
        let (searchable, dress) = (self.searchable, self.decorate.as_ref());
        let mut header_menu = self.header_menu.as_mut();
        let events = ui.show(popup, Overlay::dropdown(), |ui, body| {
            let mut query = String::new();
            if searchable || header_menu.is_some() {
                ui.horizontal(|ui| {
                    if searchable {
                        let typed = body.memory::<Query>();
                        if Popup::opening(popup, cx) {
                            *typed = Query::default();
                        }
                        let field = TextField::search(search_name(&words, &name), Bind::bound(&mut typed.text)).show(ui);
                        query.clone_from(&typed.text);
                        // The dropdown leaves the key focus on the select; the filter field takes it once the layer has
                        // recorded the select as its opener, so typing reaches the field.
                        let taken = &mut body.memory::<Query>().focused;
                        if field.response.has_focus() {
                            *taken = true;
                        } else if !*taken && !Popup::opening(popup, cx) {
                            focus::request(ui.ctx(), FocusTarget::Id(field.response.id));
                        }
                    }
                    if let Some(rows) = header_menu.as_mut() {
                        let more = IconButton::new(IconId::new(HEADER_MENU_ICON)).popup(PopupKind::Menu).label(words.header_menu.clone()).show(ui);
                        // The toggle claims the button's activation, so the list it sits in stays open under the menu.
                        if crate::contract::clicked(&more) {
                            Popup::toggle(more.id, cx);
                        }
                        Menu::new(words.header_menu.clone()).opened_by(&more, Trigger::Manual).show(ui, |ui, menu| (rows.0)(ui, menu));
                    }
                });
            }
            let rows = EntryRows::new(entries, &labels);
            let selection = held.filter(|_| query.trim().is_empty()).map_or_else(Selection::default, Selection::single);
            // While a query narrows the list the lead starts nowhere, so Down lands on the first match and Enter picks it.
            let lead = if query.trim().is_empty() { LeadOnOpen::First } else { LeadOnOpen::Selected };
            List::options(name.clone())
                .empty(words.no_matches.clone())
                .selection(Bind::given(selection))
                .lead_on_open(lead)
                .filter(&query, |at| labels[*at].clone())
                .show_items(ui, &rows, &mut |at, _| rows.row(*at, held, dress))
                .events
        });
        let mut picked = None;
        for event in events.into_iter().flatten() {
            match event {
                CollectionEvent::LeadMoved { key, by, .. } if by == LeadSource::Keys || self.preview => {
                    if let Some(value) = entries.get(key).and_then(Entry::value).filter(|value| cx.value() != Some(*value)) {
                        cx.propose(value.clone());
                    }
                }
                CollectionEvent::Activate(at) => picked = Some(at),
                _ => {}
            }
        }
        if let Some(value) = picked.and_then(|at| entries.get(at)).and_then(Entry::value) {
            cx.propose(value.clone());
            cx.commit();
            popup.close(cx);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (text, muted) = match self.position(cx.value()) {
            Some(at) => (cx.env.resolve(self.entries[at].label()), false),
            None if cx.value().is_none() => (cx.env.resolve(&cx.words::<PickerWords>().mixed.clone()), true),
            None => (self.placeholder.as_ref().map_or(Label::Empty, |text| cx.env.resolve(text)), true),
        };
        let mut face = SelectFace::new(text);
        if let Some(icon) = self.position(cx.value()).and_then(|at| self.entries[at].icon_spec()) {
            face = face.leading(icon.clone());
        }
        ui.face(FACE, if muted { face.placeholder() } else { face });
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<T> {
        cx.edit_response()
    }
}

/// The filter text of a searchable select's open list, and whether its field has taken the key focus yet.
#[derive(Clone, Default)]
struct Query {
    text: String,
    focused: bool,
}

/// The filter field's accessible name: the select's own name leads, so the field reads as part of it.
fn search_name(words: &PickerWords, select: &Label) -> Label {
    if select.as_str().is_empty() { words.search.clone() } else { Label::from(format!("{} {}", select.as_str(), words.search.as_str())) }
}
