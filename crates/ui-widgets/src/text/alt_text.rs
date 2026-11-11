//! `AltTextField`: an object's description for people who cannot see it, with a Mark as decorative check.

#[cfg(feature = "testing")]
mod stories;

use super::{FieldStyle, text_part};
use crate::choice::Checkbox;
use crate::contract::{Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, PaintUi, PartsCx, Props as _, Role, WordSetCx as _};
use crate::primitives::{TextOptions, TextRun};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, Flags, StyleCx, SurfaceLook};
use craft_behavior::{Codec, Metrics, Mixed, NotAValue, PartKey, Proposal, Sense};
use egui::{Rect, Vec2, pos2, vec2};
use std::borrow::Cow;

/// The description box shows this many lines.
const LINES: usize = 4;

/// The check that marks the object decorative.
const DECORATIVE: PartKey = PartKey::new("decorative");

craft_ui::words! {
    /// The words of the alt text field.
    pub struct AltTextWords {
        /// The box's placeholder.
        alt_text: "altText.altText" = "Alt text";
        decorative: "altText.decorative" = "Mark as decorative";
        /// Shown in place of the description while the selection's descriptions differ.
        mixed: "altText.mixed" = "Several descriptions";
    }
}

/// The alt text of one object or of a selection: its description and whether it is decorative (read by no one).
/// Each part is mixed on its own: a selection whose descriptions differ but whose objects are all decorative has a
/// mixed text and a shared flag. An edit's `Same` parts are the ones that change on every target; its `Mixed` parts
/// stay as each target has them.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AltText {
    /// The description; empty when none is set.
    pub text: Mixed<String>,
    /// `true` when the object is decorative and needs no description.
    pub decorative: Mixed<bool>,
}

impl Default for AltText {
    fn default() -> Self {
        AltText::new("")
    }
}

impl AltText {
    /// A description of `text`, not decorative.
    pub fn new(text: impl Into<String>) -> Self {
        AltText { text: Mixed::Same(text.into()), decorative: Mixed::Same(false) }
    }

    /// The given parts, each shared or mixed.
    pub fn parts(text: Mixed<String>, decorative: Mixed<bool>) -> Self {
        AltText { text, decorative }
    }

    /// Whether the object is decorative.
    #[must_use]
    pub fn decorative(mut self, decorative: bool) -> Self {
        self.decorative = Mixed::Same(decorative);
        self
    }

    /// Whether every object is decorative.
    fn all_decorative(&self) -> bool {
        self.decorative == Mixed::Same(true)
    }
}

/// The description box's text form: the description as typed, with the decorative flag kept.
#[derive(Clone, Copy, Debug)]
struct AltCodec;

impl Codec for AltCodec {
    type Value = AltText;
    const DRAFTS_ARE_VALUES: bool = true;

    fn format(&self, value: &AltText) -> String {
        value.text.same().cloned().unwrap_or_default()
    }

    fn parse(&self, text: &str, start: &AltText) -> Result<AltText, NotAValue> {
        Ok(AltText { text: Mixed::Same(text.to_owned()), decorative: start.decorative })
    }

    fn sample(&self, _: &AltText) -> Cow<'_, str> {
        Cow::Borrowed("")
    }
}

/// The style of the alt text field: the field look with a gap between the box and the check.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct AltTextFieldStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
}

/// An alt text editor: a multi-line description box and a Mark as decorative check. While the check is on, the box
/// keeps its text and does not edit it. The field's label is the prompt that names the box.
#[derive(crate::contract::Component)]
pub struct AltTextField<'a> {
    #[common]
    common: Common,
    value: Bind<'a, AltText>,
}

impl<'a> AltTextField<'a> {
    /// An alt text field named `label` over `value`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, AltText>) -> Self {
        AltTextField { common: Common::new(), value }.label(label)
    }
}

/// The check as a face over `on`.
fn check_face(on: Mixed<bool>, words: &AltTextWords) -> Checkbox<'static> {
    Checkbox::new(Bind::given(on)).label(words.decorative.clone())
}

/// The shown value: the binding's, or every part mixed while the binding is.
fn shown_of(shown: Option<&AltText>) -> AltText {
    shown.cloned().unwrap_or_else(|| AltText::parts(Mixed::Mixed, Mixed::Mixed))
}

/// The height of the description box and the size of the check.
fn sizes(cx: &MeasureCx<'_, AltTextField<'_>>) -> (f32, Vec2) {
    let words = cx.words::<AltTextWords>();
    let line = cx.layout(&TextRun::new("M")).size().y;
    (line * LINES as f32, cx.child("decorative", &mut check_face(Mixed::Same(false), &words), Proposal::Ideal))
}

impl Component for AltTextField<'_> {
    const ROLE: Role = Role::TEXT_INPUT;
    const INPUT: crate::contract::Input = crate::contract::Input::Text;
    const OVERLAYS: bool = false;
    type Style = AltTextFieldStyle;
    type Output = EditResponse<AltText>;

    fn style(cx: &StyleCx<'_>, _: AltTextFieldStyleKey) -> AltTextFieldStyle {
        let field = FieldStyle::at(cx);
        AltTextFieldStyle { surface: field.surface, metrics: field.metrics.gap(cx.control().gap) }
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let (text, check) = sizes(cx);
        let width = cx.room.unwrap_or(check.x);
        Vec2::new(width, text + cx.gap() + check.y)
    }

    /// The description box (a text part, disabled while every object is decorative, showing the mixed word as its
    /// placeholder while the descriptions differ) and the check, whose part carries the checked and mixed flags of
    /// its own value.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let content = cx.content_rect;
        let (text, check) = sizes(cx);
        let words = cx.words::<AltTextWords>();
        let area = Rect::from_min_size(content.min, vec2(content.width(), text));
        let check_at = Rect::from_min_size(pos2(content.min.x, area.max.y + cx.gap()), vec2(content.width(), check.y));
        let value = shown_of(cx.value());
        let placeholder = if value.text.same().is_some() { words.alt_text.clone() } else { words.mixed.clone() };
        let options = TextOptions { lines: Some(LINES), placeholder: Some(placeholder), enabled: !value.all_decorative(), ..TextOptions::default() };
        cx.text(text_part::<AltText>(), area, &AltCodec, &value, &options);
        let mut flags = Flags::empty();
        flags.set(Flags::CHECKED, value.all_decorative());
        flags.set(Flags::MIXED, value.decorative == Mixed::Mixed);
        cx.part(DECORATIVE, check_at).sense(Sense::CLICK | Sense::FOCUS).focusable().flags(flags);
    }

    fn bind(&mut self) -> Option<BindMut<'_, AltText>> {
        Some(self.value.reborrow())
    }

    /// A click or key activation on the check flips the flag, and on a mixed check sets every target decorative. The
    /// text part of the edit is the shown one, so a mixed text stays as each target has it.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let value = shown_of(cx.value());
        if cx.part_activated(DECORATIVE).is_some() {
            cx.set(AltText { decorative: Mixed::Same(!value.all_decorative()), ..value });
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let words = cx.words::<AltTextWords>();
        ui.face(DECORATIVE, check_face(shown_of(cx.value()).decorative, &words));
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<AltText> {
        cx.edit_response()
    }
}
