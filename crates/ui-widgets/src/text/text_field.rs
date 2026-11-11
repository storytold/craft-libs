//! `TextField<C>`: a text field over a codec, with the search form and the adornment layout.

#[cfg(feature = "testing")]
mod stories;

use super::adorn::{self, Adornment};
use super::{FieldStyle, completions, text_part, words};
use crate::contract::{
    Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, OverlayUi, PaintUi, PartsCx, Pointer as _, Props as _, Role,
    Styled as _, WordSetCx as _,
};
use crate::primitives::{Completer, IconSpec, TextOptions, TextRun};
use crate::runtime::env::Label;
use craft_behavior::{Codec, IconId, PartKey, PlainText, Proposal, Sense};
use egui::{Id, Rect, Vec2};
use smallvec::SmallVec;
use std::borrow::Cow;
use std::sync::Arc;

/// The clear part of a search field.
const CLEAR: PartKey = PartKey::new("clear");

/// A text field over a codec: plain text by default.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(overlay, probe(TextField<'static, PlainText>))]
pub struct TextField<'a, C: Codec + Clone + 'static = PlainText> {
    #[common]
    common: Common,
    value: Bind<'a, C::Value>,
    codec: C,
    options: TextOptions,
    /// The value of an empty field, for a codec whose values have no natural empty: the start a mixed field's typing
    /// parses against (a mixed field shows no text) and what the clear part of a search field sets.
    #[prop]
    pub(super) empty: Option<C::Value>,
    /// The search form: a clear part while there is text, and Escape clears the text.
    search: bool,
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
}

impl<'a> TextField<'a, PlainText> {
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
            empty: None,
            search: false,
            leading: Vec::new(),
            arrow: false,
            drives: None,
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

    /// The most characters the text holds.
    #[must_use]
    pub fn limit(mut self, limit: usize) -> Self {
        self.options.limit = Some(limit);
        self
    }

    /// Tab inserts a tab.
    #[must_use]
    pub fn tab_inserts(mut self, on: bool) -> Self {
        self.options.tab_inserts = on;
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

    /// Completion candidates for the text before the caret. The text adapter owns the list and the keys it takes.
    #[must_use]
    pub fn complete(mut self, completer: impl Completer + 'static) -> Self {
        self.options.complete = Some(Arc::new(completer));
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
        self.search && value.is_some_and(|value| !self.codec.format(value).is_empty())
    }

    /// The adornments after the text, in reading order: the clear button when `clear` names it, then the arrow.
    fn trailing(&self, clear: Option<Label>) -> SmallVec<[Adornment; 2]> {
        let mut trailing = SmallVec::new();
        if let Some(name) = clear {
            trailing.push(Adornment::button(CLEAR, IconSpec::svg(IconId::new("close")), name));
        }
        if self.arrow {
            trailing.push(Adornment::glyph(crate::controls::disclosure::DisclosureGlyph::dropdown()));
        }
        trailing
    }

    /// The clear button's name, from the library words.
    fn clear_name(words: &words::TextWords) -> Label {
        words.clear.clone()
    }
}

impl<C: Codec + Clone + 'static> Component for TextField<'_, C> {
    const ROLE: Role = Role::TEXT_INPUT;
    const INPUT: crate::contract::Input = crate::contract::Input::Text;
    type Style = FieldStyle;
    type Output = EditResponse<C::Value>;

    fn style(cx: &crate::style::StyleCx<'_>, _: TextFieldStyleKey<C>) -> FieldStyle {
        FieldStyle::at(cx)
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
        let mut size = cx.layout(&TextRun::new(text).role(self.options.role)).size();
        // A multi-line field takes the height of its lines, and grows past them with longer text, as the editor does.
        if let Some(lines) = self.options.lines.filter(|_| self.options.multiline()) {
            let line = cx.layout(&TextRun::new("M").role(self.options.role)).size().y;
            size.y = size.y.max(cx.style.metrics().min_size.y + line * (lines - 1) as f32);
        }
        // At its least a field shows a short part of its text and scrolls the rest.
        if cx.proposal == Proposal::Min {
            size.x = 0.0;
        }
        let clear = self.search.then(|| Self::clear_name(&cx.words::<words::TextWords>()));
        size.x += adorn::reserve(cx.style, &self.leading, &self.trailing(clear));
        size
    }

    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let value = cx.value();
        let clear = self.clear_shown(value).then(|| Self::clear_name(&cx.words::<words::TextWords>()));
        let trailing = self.trailing(clear);
        let placed = adorn::place(cx.content_rect, cx.style, &self.leading, &trailing, cx.flow.is_rtl());
        for (adornment, rect) in trailing.iter().zip(&placed.trailing) {
            if adornment.part() == Some(CLEAR) {
                cx.part(CLEAR, *rect).sense(Sense::CLICK | Sense::KEEP_FOCUS).pointer_only("Escape clears the text");
            }
        }
        if let Some(start) = value.or(self.empty.as_ref()) {
            let options = TextOptions { mixed: value.is_none(), ..self.options.clone() };
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
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let clear = cx.part_if_declared(CLEAR).is_some().then(|| Self::clear_name(&cx.words::<words::TextWords>()));
        let trailing = self.trailing(clear);
        let placed = adorn::place(cx.content_rect, cx.style, &self.leading, &trailing, cx.env.direction().is_rtl());
        let color = cx.resolve(&cx.style.glyph.color);
        for (adornment, rect) in self.leading.iter().chain(&trailing).zip(placed.leading.iter().chain(&placed.trailing)) {
            adornment.paint(ui, *rect, color);
        }
    }

    /// The completion list under the field, over what the text adapter holds.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        if self.options.complete.is_some() {
            completions::show(ui, cx, &text_part::<C::Value>());
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<C::Value> {
        cx.edit_response()
    }
}
