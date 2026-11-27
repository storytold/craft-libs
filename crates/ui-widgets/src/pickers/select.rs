//! `Select`: a closed field that opens a listbox of entries and feeds the picked one to its binding (15.4, 15.5).
//!
//! The field is a leaf that shows a `SelectFace`; its listbox is the component's own popup, shown from the `overlay`
//! hook on the dropdown preset. Focus stays on the field: the host hands the list keys to the listbox while it is
//! open, and Escape closes it without a change.

use super::dress::{RowDress, SelectPicture};
use super::face::{SelectFace, toggle_on_key};
use super::rows::EntryRows;
use super::search;
use super::words::PickerWords;
use crate::buttons::IconButton;
use crate::contract::overlay::{Popup, Trigger};
use crate::contract::{
    Access, Bind, BindMut, BodyCx, CollectionEvent, Common, Component, Cx, Edit as _, EditResponse, Entry, Focus as _, InBar, LeadOnOpen, LeadSource,
    MeasureCx, Numeric, Overlay, OverlayUi, Overlays as _, PaintUi, PartsCx, Pointer as _, PopupHandle, PopupKind, Props as _, Role, Show as _,
    ShowCollection as _, ShowContainer as _, Sizing, WordSetCx as _, step_stop, step_stops,
};
use crate::display::{Shade, Text};
use crate::layout_components::Row;
use crate::list::{List, ListRow, ThumbAspect};
use crate::menu::Menu;
use crate::primitives::ContentPainterCx;
use crate::runtime::env::Label;
use crate::runtime::focus::FocusMove;
use crate::runtime::overlay::CloseReason;
use crate::style::{ComponentStyle, Ink, StyleCx, SurfaceKind, SurfaceLook, TextLook, TypeRole};
use craft_behavior::nav::{StepResult, ValueAxes, ValueSteps};
use craft_behavior::overlay::MinWidth;
use craft_behavior::{
    CancelReason, GesturePhase, HitLevel, HitPriority, IconId, Metrics, NavMove, PartKey, Selection, Sense, TypeAhead, Value, WheelPhase, wheel_step,
};
use craft_theme::{Flags, Stateful, States};
use egui::{Rect, Ui, Vec2};
use std::borrow::Cow;

#[cfg(feature = "suite")]
pub(crate) mod stories;

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
    /// The least width of the box: `size.length.short` for a quiet select, none for a field.
    least_width: f32,
}

/// How a select's closed box looks and how wide it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SelectLook {
    /// A field as wide as its widest entry, so its width stays whichever entry is picked.
    #[default]
    Field,
    /// A quiet box on the panel surface with no edge until it has focus, as wide as the entry it shows (text, chevron
    /// and padding): at least `size.length.short`, at most a field length, where a longer text elides. It changes
    /// width with the pick (a workspace switcher at the end of an app bar).
    Quiet,
    /// A field filled with `surface.control`, with no edge until it has focus, as wide as its widest entry:
    /// VectorCraft 0.24's status bar selects.
    Filled,
}

/// The icon of the button in the popup header that opens the header menu.
const HEADER_MENU_ICON: &str = "more";

/// The options a searchable select keeps whatever the query: a destination of its own beside the searched ones (Save
/// as PDF among the printers).
struct SearchKept<'a, T>(Box<dyn Fn(&T) -> bool + 'a>);

impl<'a, T> SearchKept<'a, T> {
    /// Keeps the options `kept` answers true for.
    fn new(kept: impl Fn(&T) -> bool + 'a) -> Self {
        SearchKept(Box::new(kept))
    }
}

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
/// arrow keys, Home and End step the pick through the stepped entries ([`Entry::stepped`]), and so does the wheel over
/// the field, open or closed, one entry a notch; from a value that is none of them, forward lands on the first and back
/// on the last. The field is as wide as its widest entry, placeholder or mixed word, at least a field length, so it
/// keeps its width whichever is shown. A content-sized select fits that widest face, up to a field length; a longer one
/// elides. A wheel run is one edit gesture, committed when the wheel gesture ends; over the open list it takes over the
/// entry the keys previewed.
///
/// The face shows the picked entry's [closed label](Entry::closed_label) where it has one; the list, type-ahead and
/// the accessible value keep the full label. A [`picture`](Select::picture) draws each entry's thumbnail in its row
/// and before the face's text, or alone on the face with [`picture_only`](Select::picture_only); the picture is
/// square unless [`picture_aspect`](Select::picture_aspect) sets its width.
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
    /// The search field's name; "{label} Search" without it.
    #[prop(into)]
    search_label: Option<Label>,
    /// Shown above the list while the query matches none of the searched options; the list's "No matches" without it.
    #[prop(into)]
    no_matches: Option<Label>,
    /// Options the search never hides, shown after the matches.
    #[prop(func = "Fn(&T) -> bool")]
    search_kept: Option<SearchKept<'a, T>>,
    /// The open list is at least this wide, in points, within the screen; the field's width without it.
    #[prop(quantity = "width of the open list, in points")]
    list_width: Option<f32>,
    /// Shown, muted, while the value is none of the entries.
    #[prop(into)]
    placeholder: Option<Label>,
    /// The closed face and accessible label for a bound value outside the entries. The picture,
    /// if supplied, receives the bound value. This adds no option to the list.
    #[prop(into)]
    unlisted_label: Option<Label>,
    /// Adds the host's content to each entry's row.
    #[prop(func = "Fn(&T, ListRow<'a>) -> ListRow<'a>")]
    decorate: Option<RowDress<'a, T>>,
    /// Paints an entry's picture, given its value: the thumbnail of its row and of the face while it is picked.
    #[prop(func = "Fn(&T, &ContentPainterCx, Rect)")]
    picture: Option<SelectPicture<'a, T>>,
    /// The face shows the picked entry's picture alone; its label stays the accessible value.
    #[prop(flag)]
    picture_only: bool,
    /// The picture's width as a multiple of its height, in the rows and on the face; square without it.
    #[prop]
    picture_aspect: ThumbAspect,
    /// The row under the pointer is proposed as the value while the list is open, as the rows the keys reach are;
    /// closing the list without a pick restores the value.
    #[prop(flag)]
    preview: bool,
    /// A button in the popup header opens a menu of these rows (a list's own options: sort, view).
    #[prop(func = "FnMut(&mut Ui, &mut BodyCx<'_>)")]
    header_menu: Option<HeaderMenu<'a>>,
    /// How the closed box looks and how wide it is.
    #[style_key]
    #[prop]
    look: SelectLook,
}

impl<'a, T: Value + 'static> Select<'a, T> {
    /// A select named `label` over `value`, choosing among `entries`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, T>, entries: &'a [Entry<T>]) -> Self {
        Select {
            common: Common::new(),
            value,
            entries,
            searchable: false,
            search_label: None,
            no_matches: None,
            search_kept: None,
            list_width: None,
            placeholder: None,
            unlisted_label: None,
            decorate: None,
            picture: None,
            picture_only: false,
            picture_aspect: ThumbAspect::SQUARE,
            preview: false,
            header_menu: None,
            look: SelectLook::Field,
        }
        .label(label)
    }

    fn position(&self, value: Option<&T>) -> Option<usize> {
        value.and_then(|value| self.entries.iter().position(|entry| entry.value() == Some(value)))
    }

    fn unlisted_face(&self) -> (Option<&Label>, bool) {
        match self.unlisted_label.as_ref() {
            Some(label) => (Some(label), false),
            None => (self.placeholder.as_ref(), true),
        }
    }

    /// The unresolved face text and whether it stands in for a pick.
    fn shown<'s>(&'s self, value: Option<&T>, mixed: &'s Label) -> (Option<&'s Label>, bool) {
        match self.position(value) {
            Some(at) => (Some(self.entries[at].face_label()), false),
            None if value.is_none() => (Some(mixed), true),
            None => self.unlisted_face(),
        }
    }

    /// Whether the faces fit their text: a quiet select, and the selects `fits` names.
    fn fits(&self, sizing: Option<Sizing>, in_bar: bool) -> bool {
        self.look == SelectLook::Quiet || fits(sizing, in_bar)
    }

    /// The face of `text`, with the picture's room before it, or the picture alone, when the select has a picture.
    fn sized_face(&self, text: Label, picture: bool, fit: bool) -> SelectFace<'static> {
        let face = SelectFace::new(text).fit(fit);
        if self.picture.is_some() && picture { face.thumb(self.picture_aspect, |_, _| {}).name(!self.picture_only) } else { face }
    }

    /// Sets the value to the entry at `at` unless it is the value already.
    fn pick(&self, cx: &mut Cx<'_, Self>, at: usize) {
        if let Some(value) = self.entries[at].value()
            && self.position(cx.value()) != Some(at)
        {
            cx.set(value.clone());
        }
    }

    /// The closed field's keys: typed letters open the list on the first enabled match, AccessKit sets the pick and
    /// the arrow keys, Home and End step it through `stops`.
    fn closed_keys(&self, cx: &mut Cx<'_, Self>, list: PopupHandle, stops: &[usize]) {
        let held = self.position(cx.value());
        while let Some(text) = cx.intents().take_text() {
            // Keep the entry indices for wrapping around the held value, with no label for headings or disabled items.
            let labels: Vec<Label> =
                self.entries.iter().map(|entry| if entry.is_enabled() { cx.env.resolve(entry.label()) } else { Label::Empty }).collect();
            let folder = cx.env.folder();
            let found = TypeAhead::pick(&text, held, labels.len(), |at| Cow::Borrowed(labels[at].as_str()), |s| folder.key(s));
            if let Some(value) = found.and_then(|at| self.entries[at].value()) {
                cx.propose(value.clone());
                list.open(cx);
                return;
            }
        }
        if let Some(at) = cx.intents().take_set_value() {
            // `Edit::set` is a one-shot change: a wheel run's open proposal is committed first.
            if cx.in_wheel_run() {
                cx.commit();
            }
            let to = (at.round().max(0.0) as usize).min(stops.len() - 1);
            self.pick(cx, stops[to]);
        }
        while let Some((mv, _)) = cx.intents().take_move() {
            if cx.in_wheel_run() {
                cx.commit();
            }
            if let Some(to) = step(stops, self.position(cx.value()), mv) {
                self.pick(cx, to);
            }
        }
    }
}

/// Whether a select's faces fit their text: a select sized to its content, and one in a bar (`InBar`) that sets no
/// sizing of its own.
fn fits(sizing: Option<Sizing>, in_bar: bool) -> bool {
    match sizing {
        Some(sizing) => sizing == Sizing::Content,
        None => in_bar,
    }
}

/// The closed box of a quiet or filled select: `fill` at the well radius, the hover fill under the pointer, and an edge
/// only in the focus colour while it has focus.
fn borderless(cx: &StyleCx<'_>, fill: craft_theme::Rgba) -> SurfaceLook {
    let colors = &cx.tokens.color;
    let (rest, hover) = (cx.color(fill), cx.color(colors.state.hover));
    let fill = Stateful::new(rest).off(States::new(rest, hover, hover, rest));
    // The edge takes the fill's colour in every state, so it shows only with focus.
    let edge = fill.clone().when(Flags::FOCUSED, cx.color(colors.line.focus));
    SurfaceLook::new(fill).radius(cx.surface(SurfaceKind::Well).radius).border(edge, cx.tokens.stroke.thin).border_replaces_edge()
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

    fn style(cx: &StyleCx<'_>, key: SelectStyleKey<T>) -> SelectStyle {
        let off = cx.color(cx.tokens.color.text.disabled);
        let (ink, muted) = (cx.ink(Ink::OnPanel), cx.color(cx.tokens.color.text.faint));
        let (surface, least_width) = match key.look {
            SelectLook::Field => (cx.dropdown(), 0.0),
            SelectLook::Quiet => (borderless(cx, cx.tokens.color.surface.panel), cx.tokens.size.length.short),
            SelectLook::Filled => (borderless(cx, cx.tokens.color.surface.control), 0.0),
        };
        SelectStyle {
            surface,
            metrics: Metrics::new(),
            text: TextLook::new(TypeRole::Body, Stateful::new(ink).off(States::new(ink, ink, ink, off))),
            muted: TextLook::new(TypeRole::Body, Stateful::new(muted).off(States::new(muted, muted, muted, off))),
            least_width,
        }
    }

    /// The face senses the pointer; the wheel steps the pick.
    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::WHEEL_Y
    }

    /// The widest face of an entry (its closed label, or its picture), the placeholder and the mixed word, so the field
    /// keeps its size whichever is shown; a quiet select's face as it shows now. At least the least width.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let mixed = cx.words::<PickerWords>().mixed.clone();
        let fit = self.fits(cx.resolved.sizing(), cx.env.get::<InBar>().0);
        let face = |(text, picture): (&Label, bool)| cx.face(self.sized_face(cx.env.resolve(text), picture, fit));
        let size = if self.look == SelectLook::Quiet {
            let (text, muted) = self.shown(cx.value(), &mixed);
            face((text.unwrap_or(&Label::Empty), !muted))
        } else {
            let entries = self.entries.iter().filter(|entry| entry.is_item()).map(|entry| (entry.face_label(), true));
            let (unlisted, muted) = self.unlisted_face();
            let texts = entries.chain(unlisted.map(|text| (text, !muted))).chain(std::iter::once((&mixed, false)));
            texts.map(face).fold(Vec2::ZERO, Vec2::max)
        };
        Vec2::new(size.x.max(cx.style.least_width), size.y)
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
    /// letters open it on the first entry they begin, the arrow keys, Home and End step the pick through the stepped
    /// entries, and AccessKit sets it; the wheel steps it open or closed. A proposal the list closed on without a pick
    /// is cancelled.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let list = cx.popup("list");
        toggle_on_key(cx, list);
        // A wheel run proposes while the list is closed; only a proposal the open list made is dropped here.
        if !list.is_open(cx) && !cx.in_wheel_run() && matches!(cx.gesture(), Some(GesturePhase::Begin | GesturePhase::Update)) {
            cx.cancel(CancelReason::Dismissed);
        }
        let stops = step_stops(self.entries);
        if stops.is_empty() {
            return;
        }
        if !list.is_open(cx) {
            self.closed_keys(cx, list, &stops);
        }
        // The wheel steps the pick over the field with the list open or closed, one stop a notch, down for a step
        // forward (as a combo box and a tab strip step); a run over the open list takes over the rows the keys
        // previewed. The end share of the gesture has no scroll and commits the run.
        if let Some(wheel) = cx.take_wheel() {
            if matches!(wheel.phase, WheelPhase::End) {
                cx.wheel_run(wheel.phase, None);
            } else if let StepResult::Set(along) =
                wheel_step(&ValueSteps::new(0.0, -f64::MAX, f64::MAX, 1.0, 1.0).fine(1.0), ValueAxes::Both, &wheel, cx.env.direction().is_rtl())
                && along != 0.0
            {
                let mut to = self.position(cx.value());
                for _ in 0..along.abs().round() as usize {
                    to = step_stop(&stops, to, along < 0.0);
                }
                if let Some(value) = to.and_then(|to| self.entries[to].value()) {
                    cx.wheel_run(wheel.phase, Some(value.clone()));
                }
            }
        }
    }

    /// The picked entry's place among the entries that can be picked, shown as its label, so AccessKit steps and sets it.
    /// A value that names no entry reads as its unlisted label or placeholder, as shown on the face.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let stops = step_stops(self.entries);
        let range = 0.0..=stops.len().saturating_sub(1) as f64;
        let held = self.position(cx.value());
        let unlisted = self.unlisted_face().0.filter(|_| held.is_none() && cx.value().is_some());
        let numeric = match held.and_then(|held| stops.iter().position(|&stop| stop == held)) {
            Some(at) => Numeric::new(at as f64, range).text(cx.env.resolve(self.entries[stops[at]].label()).as_str()),
            None => match unlisted {
                Some(text) => Numeric::empty(range).text(cx.env.resolve(text).as_str()),
                None => Numeric::empty(range),
            },
        };
        access.numeric(numeric.step(1.0));
    }

    /// The listbox: one option row per entry, a separator row for a separator, the picked entry's row selected. Each row the
    /// keys move the lead to is proposed, and with a preview each row the pointer moves it to; a pick comes back from
    /// the list as Enter or a click on a row and commits.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("list");
        let entries = self.entries;
        let labels: Vec<Label> = entries.iter().map(|entry| cx.env.resolve(entry.label())).collect();
        let name = cx.label();
        let words = cx.words::<PickerWords>();
        let held = self.position(cx.value());
        let (searchable, dress, picture) =
            (self.searchable, self.decorate.as_ref(), self.picture.as_ref().map(|picture| (picture, self.picture_aspect)));
        let kept =
            |at: usize| self.search_kept.as_ref().is_some_and(|kept| entries.get(at).and_then(Entry::value).is_some_and(|value| (kept.0)(value)));
        let (search_label, no_matches, wide) = (self.search_label.clone(), self.no_matches.clone(), self.list_width.is_some());
        let folder = cx.env.folder();
        let mut header_menu = self.header_menu.as_mut();
        let list = cx.id.with("options");
        let overlay = Overlay::dropdown().list(list);
        let overlay = match self.list_width {
            Some(width) if width.is_finite() && width > 0.0 => overlay.min_width(MinWidth::Points(width)),
            _ => overlay,
        };
        let events = ui.show(popup, overlay, |ui, body| {
            let mut query = String::new();
            if searchable || header_menu.is_some() {
                let row = if wide { Row::new().fill_width() } else { Row::new() };
                let _ = row.show(ui, |ui, _| {
                    if searchable {
                        query = search::show_named(ui, body, popup, cx, &name, (search_label.as_ref(), wide), list);
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
            // A query that matches none of the searched options says so above the kept ones.
            let matcher = craft_i18n::matcher::Matcher::new(&folder, query.trim());
            let none_match = !matcher.is_empty()
                && !(0..entries.len()).filter(|&at| entries[at].is_item() && !kept(at)).any(|at| matcher.score(labels[at].as_str()).is_some());
            if let (true, Some(text)) = (none_match, no_matches.as_ref()) {
                Text::new(text.clone()).shade(Shade::Muted).show(ui);
            }
            let selection = held.filter(|_| query.trim().is_empty()).map_or_else(Selection::default, Selection::single);
            // While a query narrows the list the lead starts nowhere, so Down lands on the first match and Enter picks it.
            let lead = if query.trim().is_empty() { LeadOnOpen::First } else { LeadOnOpen::Selected };
            let options = List::options(name.clone()).id(list).empty(words.no_matches.clone()).selection(Bind::given(selection)).lead_on_open(lead);
            // The reveal is a one-shot request: made on the opening pass only, it does not fight the keys after.
            let options = match held.filter(|_| Popup::opening(popup, cx)) {
                Some(at) => options.reveal(at),
                None => options,
            };
            options
                // A kept option matches any query by carrying the query among its texts.
                .filter(
                    &query,
                    |at| if kept(*at) { vec![labels[*at].clone(), Label::from(query.trim().to_owned())] } else { vec![labels[*at].clone()] },
                )
                .show_items(ui, &rows, &mut |at, _| rows.row(*at, held, dress, picture))
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
        // Tab and Shift+Tab choose the row the keys reached and move on; Escape and any other focus move put the value
        // back. A press outside cancels in `update` once the list has closed.
        if matches!(cx.gesture(), Some(GesturePhase::Begin | GesturePhase::Update)) {
            match ui.closing(popup) {
                Some(CloseReason::FocusLeft(FocusMove::Keys)) => cx.commit(),
                Some(CloseReason::FocusLeft(_)) => cx.cancel(CancelReason::Blur),
                Some(CloseReason::Escape) => cx.cancel(CancelReason::Escape),
                _ => {}
            }
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let held = self.position(cx.value());
        let mixed = cx.words::<PickerWords>().mixed.clone();
        let (text, muted) = self.shown(cx.value(), &mixed);
        let text = text.map_or(Label::Empty, |text| cx.env.resolve(text));
        let mut face = SelectFace::new(text).fit(self.fits(cx.resolved.sizing(), cx.env.get::<InBar>().0));
        if let Some(icon) = held.and_then(|at| self.entries[at].icon_spec()) {
            face = face.leading(icon.clone());
        }
        let pictured = held.and_then(|at| self.entries[at].value()).or(cx.value().filter(|_| !muted));
        if let (Some(picture), Some(value)) = (self.picture.as_ref(), pictured) {
            face = face.thumb(self.picture_aspect, move |cx, rect| picture.paint(value, cx, rect)).name(!self.picture_only);
        }
        ui.face(FACE, if muted { face.placeholder() } else { face });
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<T> {
        cx.edit_response()
    }
}
