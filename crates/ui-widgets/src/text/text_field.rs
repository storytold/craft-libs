//! `TextField<C>`: a text field over a codec, with the search form and the adornment layout.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::adorn::{self, Adornment};
use super::{FieldStyle, completions, lines_inset, text_part, words};
use crate::contract::{
    Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, OverlayUi, PaintUi, PartsCx, Pointer as _, Props as _, Role, Sizing,
    Styled as _, WordSetCx as _,
};
use crate::primitives::{Completer, IconSpec, SubmitKey, TabKey, TextOptions, TextRun, Wrap};
use crate::runtime::env::{Label, Message};
use crate::style::{FieldLook, Span};
use craft_behavior::{Codec, EditEvent, IconId, Mixed, PartKey, PlainText, Proposal, Sense};
use egui::{Id, Rect, Vec2};
use smallvec::SmallVec;
use std::borrow::Cow;
use std::sync::Arc;

/// The clear part of a search field.
const CLEAR: PartKey = PartKey::new("clear");

/// The most characters a single-line field holds unless its caller sets a limit: a field is no place for a document.
const SINGLE_LINE_LIMIT: usize = 1024;

/// The character limit a caller set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Limit {
    /// [`SINGLE_LINE_LIMIT`] on a single-line field, none on a multi-line one.
    #[default]
    Default,
    /// No limit: a path or an address may be longer than the default.
    None,
    /// At most this many characters.
    At(usize),
}

impl Limit {
    /// The limit the text part applies, for a field that is multi-line or not.
    fn effective(self, multiline: bool) -> Option<usize> {
        match self {
            Limit::Default => (!multiline).then_some(SINGLE_LINE_LIMIT),
            Limit::None => None,
            Limit::At(n) => Some(n),
        }
    }
}

/// A text field over a codec: plain text by default.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(overlay, probe(TextField<'static, PlainText>))]
pub struct TextField<'a, C: Codec + Clone + 'static = PlainText> {
    #[common]
    common: Common,
    value: Bind<'a, C::Value>,
    pub(super) codec: C,
    options: TextOptions,
    limit: Limit,
    /// The value of an empty field, for a codec whose values have no natural empty: the start a mixed field's typing
    /// parses against (a mixed field shows no text) and what the clear part of a search field sets.
    #[prop]
    pub(super) empty: Option<C::Value>,
    /// The search form: a clear part while there is text, and Escape clears the text.
    search: bool,
    /// Whether a search field shows its clear button while there is text. Off, the field is a plain integrated one
    /// (a find bar's): Escape still clears it.
    #[prop]
    clear_button: bool,
    /// The field's look: [`FieldLook::Quiet`] is a bar's own field, with its fill and quiet edge at rest
    /// ([`StyleCx::quiet_field`](crate::style::StyleCx::quiet_field)) and the muted placeholder.
    #[prop]
    #[style_key]
    look: FieldLook,
    /// The adornments before the text, in reading order.
    leading: Vec<Adornment>,
    /// The field reserves and paints a chevron at the end, for the host's popup.
    arrow: bool,
    /// The field drives the collection `list`: Up, Down, Page Up, Page Down and Enter go from the field to it, in a
    /// layer or inline, and typed text stays with the field. While the field is empty Home and End go to the
    /// collection too, and for a tree Left and Right open and close its rows; with text they move the caret. `list` is
    /// the id the collection is built with (`Props::id`), so a pattern can name a collection it shows after the field.
    #[prop]
    drives: Option<Id>,
    /// Binds the caret, as a byte offset into the text: a value the app writes places the caret at the character
    /// boundary at or before it, collapses the selection and scrolls the caret into view; the user moving the caret
    /// writes its offset back.
    #[prop]
    caret: Option<Bind<'a, usize>>,
    /// The most lines a multi-line field grows to before its text scrolls inside it.
    max_lines: Option<usize>,
    /// The width the field asks for, in characters ([`width_chars`](Self::width_chars)); `None` asks for the field
    /// length (`size.length.field`).
    #[style_key]
    width_chars: Option<u16>,
}

impl<'a> TextField<'a, PlainText> {
    /// Escape keeps the current text and ends editing by leaving focus: a live filter keeps its query. An open typing
    /// run commits once, without submitting or cancelling, and the same Escape goes on to the enclosing scope. Plain
    /// fields otherwise restore the run's start and search fields clear; neither default changes.
    #[must_use]
    pub fn keeps_on_escape(mut self) -> Self {
        self.options.keep_on_escape = true;
        self.options.clear_on_escape = false;
        self
    }

    /// A plain text field named `label`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, String>) -> Self {
        let mut field = TextField::with_codec(label, value, PlainText);
        field.empty = Some(String::new());
        field
    }

    /// A search field: a search glyph before the text, a clear part after it while there is text, and Escape
    /// clears the text and commits it; Escape on an empty field goes on to the host.
    pub fn search(label: impl Into<Label>, value: Bind<'a, String>) -> Self {
        let mut field = TextField::new(label, value).leading(IconSpec::svg(IconId::new("search")));
        field.options.clear_on_escape = true;
        field.search = true;
        field
    }
}

impl<'a, C: Codec + Clone + 'static> TextField<'a, C> {
    /// A field named `label` that shows `value` through `codec`.
    pub fn with_codec(label: impl Into<Label>, value: Bind<'a, C::Value>, codec: C) -> Self {
        TextField {
            common: Common::new(),
            value,
            codec,
            options: TextOptions::default(),
            limit: Limit::Default,
            empty: None,
            search: false,
            clear_button: true,
            look: FieldLook::Normal,
            leading: Vec::new(),
            arrow: false,
            drives: None,
            caret: None,
            max_lines: None,
            width_chars: None,
        }
        .label(label)
    }

    /// Shown while the text is empty.
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<Label>) -> Self {
        self.options.placeholder = Some(placeholder.into());
        self
    }

    /// An icon before the text, decorative: screen readers skip it.
    #[must_use]
    pub fn leading(mut self, icon: impl Into<IconSpec<'static>>) -> Self {
        self.leading.push(Adornment::glyph(icon));
        self
    }

    /// Fixed text before the text (the `#` before hex digits), in the type role and face set before this call: the user
    /// neither selects nor deletes it, and screen readers skip it.
    #[must_use]
    pub fn prefix(mut self, text: impl Into<Label>) -> Self {
        let prefix = Adornment::text(text, self.options.role);
        self.leading.push(if self.options.mono { prefix.mono() } else { prefix });
        self
    }

    /// A chevron at the end of the field, for a field that opens a popup (a picker): the text does not run under it.
    /// The host senses the click over the end square of the field's rect.
    #[must_use]
    pub fn trailing_arrow(mut self) -> Self {
        self.arrow = true;
        self
    }

    /// The whole text is selected whenever the field takes focus, so typing replaces it. Combine with
    /// `Props::default_stop` and a layer's `Initial::Default`, or a focus request, for a field that opens ready to
    /// type over its text.
    #[must_use]
    pub fn select_on_focus(mut self) -> Self {
        self.options.select_on_focus = true;
        self
    }

    /// A layer that opens on this field leaves its caret where the codec puts it (after the text) instead of selecting
    /// the whole text, so typing adds to it: a card that opens to edit text already written. Off by default.
    #[must_use]
    pub fn keep_caret_on_open(mut self) -> Self {
        self.options.keep_caret_on_open = true;
        self
    }

    /// With `on`, selects the whole text in this pass, so typing replaces it, unless the field is composing text; a
    /// request made during a composition is dropped.
    #[must_use]
    pub fn select_all(mut self, on: bool) -> Self {
        self.options.select_all = on;
        self
    }

    /// Monospace text.
    #[must_use]
    pub fn mono(mut self) -> Self {
        self.options.mono = true;
        self
    }

    /// Characters show as dots; the field exposes no value to assistive technology.
    #[must_use]
    pub fn masked(mut self) -> Self {
        self.options.mask = true;
        self
    }

    /// The field shows `lines` lines and wraps its text; 1 is a single-line field.
    #[must_use]
    pub fn lines(mut self, lines: usize) -> Self {
        self.options.lines = Some(lines);
        self
    }

    /// A multi-line field grows with its text up to `lines` lines, then scrolls inside with the caret in view; never
    /// fewer lines than [`lines`](Self::lines) shows. A field sized `Sizing::Fill` takes its whole room instead.
    #[must_use]
    pub fn max_lines(mut self, lines: usize) -> Self {
        self.max_lines = Some(lines.max(1));
        self
    }

    /// The field asks for the width of `chars` characters instead of the field length (`size.length.field`): `chars`
    /// times the width of a `0` in the field's body face, as CSS's `ch` unit, plus the field's padding. The width
    /// follows the text size and the face, as the field length does. It is the width the field asks for, not a fixed
    /// one: a row that lacks room still narrows it, and a field sized `Sizing::Content` or `Sizing::FillWidth` takes
    /// its own width as before. `0` keeps the field length.
    #[must_use]
    pub fn width_chars(mut self, chars: u16) -> Self {
        self.width_chars = (chars > 0).then_some(chars);
        self
    }

    /// The most characters the text holds; `None` clears the limit. A single-line field holds 1024 characters unless
    /// this is set; a multi-line field has no limit unless this is set.
    #[must_use]
    pub fn limit(mut self, limit: impl Into<Option<usize>>) -> Self {
        self.limit = limit.into().map_or(Limit::None, Limit::At);
        self
    }

    /// Tab inserts a tab; off, Tab moves focus.
    #[must_use]
    pub fn tab_inserts(mut self, on: bool) -> Self {
        self.options.tab = if on { TabKey::Inserts } else { TabKey::Moves };
        self
    }

    /// Tab keeps focus and is reported in the edit response (`EditResponse::tab`): `Some(true)` for Tab, `Some(false)`
    /// for Shift+Tab.
    #[must_use]
    pub fn tab_reports(mut self) -> Self {
        self.options.tab = TabKey::Reports;
        self
    }

    /// The caller hangs a context menu of its own on the field (`Menu::context` over its response): a right click opens
    /// that menu instead of the field's edit menu.
    #[must_use]
    pub fn own_context_menu(mut self) -> Self {
        self.options.own_menu = true;
        self
    }

    /// Enter acts in the field itself (it saves the value or runs a search): the field takes it, and an enclosing
    /// dialog's default button does not. A field without this leaves Enter to that button. In a multi-line field
    /// Enter still adds a line, and Cmd+Enter (Ctrl+Enter off macOS) submits. `EditResponse::submit_mods` holds the
    /// modifiers of the submitting Enter.
    #[must_use]
    pub fn submits(mut self) -> Self {
        self.options.submits = true;
        self
    }

    /// The key that submits a multi-line field (`TextOptions::submit_key`).
    #[must_use]
    pub fn submit_key(mut self, key: SubmitKey) -> Self {
        self.options.submit_key = key;
        self
    }

    /// Completion candidates for the text before the caret. The text adapter owns the list and the keys it takes.
    #[must_use]
    pub fn complete(mut self, completer: impl Completer + 'static) -> Self {
        self.options.complete = Some(Arc::new(completer));
        self
    }

    /// Why a draft that does not parse is no value ("Not a hex color"). While such a draft shows, an enclosing field
    /// row shows it as its status line, or the field shows it as its tooltip where no row does, and assistive
    /// technology reads it as the field's description. A refusal or warning of the binding comes first.
    #[must_use]
    pub fn unparsed(mut self, reason: impl Into<Label>) -> Self {
        self.options.unparsed = Some(Message::new(reason));
        self
    }

    /// Enter accepts the highlighted completion, as Tab does. Without it Enter stays the field's own: it commits the
    /// text as typed.
    #[must_use]
    pub fn complete_on_enter(mut self) -> Self {
        self.options.complete_on_enter = true;
        self
    }

    /// `text` replaces the selection in this pass: at the caret it inserts. Set it for one pass only.
    #[must_use]
    pub fn replace_selection(mut self, text: impl Into<String>) -> Self {
        self.options.replace_selection = Some(text.into());
        self
    }
}

impl<C: Codec + Clone + 'static> TextField<'_, C> {
    /// The clear part shows while the field has text.
    fn clear_shown(&self, value: Option<&C::Value>) -> bool {
        self.search && self.clear_button && value.is_some_and(|value| !self.codec.format(value).is_empty())
    }

    /// The adornments after the text, in reading order: the clear button when `clear` names it, then the arrow.
    fn trailing(&self, clear: Option<Label>, arrow: crate::controls::disclosure::DropdownArrow) -> SmallVec<[Adornment; 2]> {
        let mut trailing = SmallVec::new();
        if let Some(name) = clear {
            trailing.push(Adornment::button(CLEAR, IconSpec::svg(IconId::new("close")), name));
        }
        if self.arrow {
            trailing.push(Adornment::glyph(crate::controls::disclosure::DisclosureGlyph::dropdown_of(arrow)));
        }
        trailing
    }

    /// The clear button's name, from the library words.
    fn clear_name(words: &words::TextWords) -> Label {
        words.clear.clone()
    }
}

impl<C: Codec + Clone + 'static> TextField<'_, C> {
    /// `text` as the editor sets it.
    fn run<'t>(&self, text: impl Into<Cow<'t, str>>) -> TextRun<'t> {
        if self.options.mono { TextRun::spans(vec![Span::new(text).mono()]) } else { TextRun::new(text).role(self.options.role) }
    }

    /// The height of one line of the editor's text.
    fn line(&self, cx: &MeasureCx<'_, Self>) -> f32 {
        cx.layout(&self.run("M")).size().y
    }
}

impl<C: Codec + Clone + 'static> Component for TextField<'_, C> {
    const ROLE: Role = Role::TEXT_INPUT;
    const INPUT: crate::contract::Input = crate::contract::Input::Text;
    type Style = FieldStyle;
    type Output = EditResponse<C::Value>;

    /// The field's shortest width (`size.length.short`) is `measure`'s, so a content-sized field can go without it.
    fn style(cx: &crate::style::StyleCx<'_>, key: TextFieldStyleKey<C>) -> FieldStyle {
        let mut style = FieldStyle::at(cx);
        if let Some(chars) = key.width_chars {
            style = style.held_at(FieldStyle::chars_width(cx, chars));
        }
        style.metrics = style.metrics.min_size(Vec2::new(0.0, style.metrics.min_size.y));
        if key.look == FieldLook::Quiet {
            style.surface = cx.quiet_field();
        }
        style
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    /// The text, and the room the adornments take; a search field keeps the room of its clear button while the text is
    /// empty, so the field does not change width as the button comes and goes.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let sample: Cow<'_, str> = match cx.value() {
            Some(value) => self.codec.sample(value),
            None => Cow::Borrowed(""),
        };
        let text = if sample.is_empty() { self.options.placeholder.as_ref().map_or("", Label::as_str) } else { sample.as_ref() };
        let mut size = cx.field_text(&TextRun::new(text).role(self.options.role));
        // A multi-line field takes the height of its lines, and grows past them with longer text, as the editor does, up
        // to `max_lines`; past that its text scrolls inside it (the text part is its scroll owner). The rows are the
        // field's own text wrapped as the editor wraps it, not the codec's width sample, which a plain field leaves
        // empty.
        if let Some(lines) = self.options.lines.filter(|_| self.options.multiline()) {
            let line = self.line(cx);
            let height = |lines: usize| 2.0 * lines_inset(&cx.style.metrics, line) + line * lines as f32;
            let shown = cx.value().map(|value| self.codec.format(value)).unwrap_or_default();
            let tall = cx.layout(&self.run(shown).wrap(Wrap::Wrap)).size().y;
            let rows = if line > 0.0 { (tall / line).round() as usize } else { 1 };
            size.y = size.y.max(height(lines.max(rows)));
            if let Some(most) = self.max_lines {
                size.y = size.y.min(height(most.max(lines)));
            }
        }
        // At its least a field shows a short part of its text and scrolls the rest.
        if cx.proposal == Proposal::Min {
            size.x = 0.0;
        }
        let clear = (self.search && self.clear_button).then(|| Self::clear_name(&cx.words::<words::TextWords>()));
        self.leading = std::mem::take(&mut self.leading).into_iter().map(|adornment| adornment.measured(cx)).collect();
        size.x += adorn::reserve(cx.style, &self.leading, &self.trailing(clear, cx.env.get()));
        // Short text keeps a field at `size.length.short`, so a column of fields ends at one edge; a content-sized field
        // takes its text and adornments only, as a content-sized number field does.
        if cx.proposal != Proposal::Min && cx.resolved.sizing() != Some(Sizing::Content) {
            let padding = cx.style.metrics.padding;
            size.x = size.x.max(cx.tokens.size.length.short - padding.start - padding.end);
        }
        // A filling multi-line field takes the room it is offered, whatever its text and `max_lines`; its text scrolls
        // inside it.
        if let Proposal::Size(room) = cx.proposal
            && self.options.multiline()
            && cx.resolved.sizing() == Some(Sizing::Fill)
            && room.x.is_finite()
            && room.y.is_finite()
        {
            // The measure gives the content, which step 5 pads: the room less the padding fills it.
            let padding = cx.style.metrics.padding;
            return Vec2::new((room.x - padding.start - padding.end).max(0.0), (room.y - padding.top - padding.bottom).max(0.0));
        }
        size
    }

    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let value = cx.value();
        let clear = self.clear_shown(value).then(|| Self::clear_name(&cx.words::<words::TextWords>()));
        let trailing = self.trailing(clear, cx.env.get());
        self.leading = std::mem::take(&mut self.leading).into_iter().map(|adornment| adornment.measured(cx)).collect();
        let mut placed = adorn::place(cx.content_rect, cx.style, &self.leading, &trailing, cx.flow.is_rtl());
        for (adornment, rect) in trailing.iter().zip(&placed.trailing) {
            if adornment.part() == Some(CLEAR) {
                cx.part(CLEAR, *rect).sense(Sense::CLICK | Sense::KEEP_FOCUS).pointer_only("Escape clears the text");
            }
        }
        if self.options.multiline() {
            let inset = lines_inset(&cx.style.metrics, self.line(cx));
            placed.text = placed.text.shrink2(Vec2::new(0.0, inset.min(placed.text.height() / 2.0)));
        }
        if let Some(start) = value.or(self.empty.as_ref()) {
            let limit = self.limit.effective(self.options.multiline());
            // The bound caret goes to the text part every pass; the part places it only when it differs from the caret
            // its widget has, which this field wrote back last pass (`finish`).
            let caret = self.caret.as_mut().and_then(|caret| match caret.reborrow().get() {
                Some(Mixed::Same(at)) => Some(at),
                _ => None,
            });
            let options = TextOptions { mixed: value.is_none(), limit, caret, look: self.look, ..self.options.clone() };
            cx.text(text_part::<C::Value>(), placed.text, &self.codec, start, &options);
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, C::Value>> {
        Some(self.value.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if let Some(list) = self.drives {
            let empty = cx.value().is_none_or(|value| self.codec.format(value).is_empty());
            crate::runtime::focus::registration::drive(&cx.core.ctx, cx.core.id, list, empty);
        }
        if cx.part_activated(CLEAR).is_some()
            && let Some(empty) = self.empty.clone()
        {
            cx.set(empty);
            // The text part showed the old text earlier in this pass: the pass runs again, so the field shows and reports
            // the empty text in the pass that took the click, not one pass later.
            crate::runtime::frame::discard_pass(&cx.core.ctx, "a search field's clear part emptied it");
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let clear = cx.part_if_declared(CLEAR).is_some().then(|| Self::clear_name(&cx.words::<words::TextWords>()));
        let trailing = self.trailing(clear, cx.env.get());
        let placed = adorn::place(cx.content_rect, cx.style, &self.leading, &trailing, cx.env.direction().is_rtl());
        let color = cx.resolve(&cx.style.glyph.color);
        for (adornment, rect) in self.leading.iter().chain(&trailing).zip(placed.leading.iter().chain(&placed.trailing)) {
            adornment.paint(ui, *rect, color, self.options.role);
        }
    }

    /// The edit menu, and the completion list under the field, over what the text adapter holds.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        crate::menu::text_menu(ui, cx, &self.options);
        if self.options.complete.is_some() {
            completions::show(ui, cx, &text_part::<C::Value>(), self.options.complete_on_enter);
        }
    }

    /// The caret the user moved goes back through its binding: a caret that differs from the bound value after the
    /// pass was moved in the field, as one the app wrote differs before it.
    fn finish(mut self, mut cx: Cx<'_, Self>) -> EditResponse<C::Value> {
        let response = cx.edit_response();
        if let (Some(bind), Some(at)) = (self.caret.as_mut(), response.caret) {
            let before = bind.reborrow().read(&mut cx);
            if before != Some(Mixed::Same(at)) {
                // A caret move is no edit of the document: it names no undo step.
                let event = EditEvent::set(before.unwrap_or(Mixed::Same(at)), at, Arc::from(""), None);
                let _ = bind.reborrow().write(&mut cx, &event);
            }
        }
        response
    }
}
