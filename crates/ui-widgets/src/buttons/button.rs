//! `Button`: a push button with an optional leading and trailing icon and a busy state.

use super::face::{self, FaceLook, FaceSpec};
use crate::contract::{Common, Component, Cx, LeastWidth, MeasureCx, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props, Role, Styled as _};
use crate::display::{Kbd, Spinner};
use crate::primitives::{HAlign, IconSpec, PaintShape};
use crate::runtime::commands::CommandRef;
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, DataColor, FocusLook, IconLook, StyleColor, StyleCx, SurfaceLook, TextLook, TypeRole};
use crate::text::FieldStyle;
use craft_behavior::{Flow, IconId, Metrics, PartKey, PressPhase, Sense};
use craft_keymap::Mods;
use craft_theme::{
    Rgba, Stateful, States, Variant,
    variant::{Emphasis, Tone},
};
use egui::{Color32, Rect, Response, Vec2};
use std::ops::Deref;

/// The part a loading button shows its spinner in.
const SPINNER: PartKey = PartKey::new("spinner");
/// The part the Search face shows its command's chord in.
const CHORD: PartKey = PartKey::new("chord");

/// The form of a button's face.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ButtonForm {
    /// The ordinary push button.
    #[default]
    Push,
    /// The command-search opener: a text field's face with the search glyph, the placeholder and the command's chord.
    Search,
}

/// The style of a push button.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[style(variants = [Variant::SECONDARY, Variant::PRIMARY, Variant::GHOST, Variant::DANGER, Variant::look(Emphasis::Outline, Tone::Accent)])]
#[non_exhaustive]
pub struct ButtonStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The label.
    #[style(text)]
    text: TextLook,
    /// The leading and trailing icons.
    #[style(icon)]
    icon: IconLook,
    face: FaceLook,
    /// The state overlay a tinted face carries over its fill.
    overlay: Stateful<StyleColor>,
}

impl ButtonStyle {
    /// The style of a push button in `cx`; the split button shares it.
    pub(super) fn of(cx: &StyleCx<'_>) -> ButtonStyle {
        let (text, icon) = face::ink(cx);
        ButtonStyle {
            surface: face::surface(cx, &cx.variant_look()),
            metrics: face::metrics(cx, &cx.variant_look()),
            focus: face::focus(cx),
            text,
            icon,
            face: FaceLook::of(cx),
            overlay: cx.state_layer(),
        }
    }

    /// The style of the command-search opener: a text field's surface, padding and glyph, and the label in the
    /// colour a field's placeholder takes.
    fn search(cx: &StyleCx<'_>) -> ButtonStyle {
        let field = FieldStyle::at(cx);
        let text = &cx.tokens.color.text;
        let (faint, off) = (cx.color(text.faint), cx.color(text.disabled));
        ButtonStyle {
            surface: cx.field(),
            metrics: field.metrics(),
            focus: face::focus(cx),
            text: TextLook::new(TypeRole::Body, Stateful::new(faint).off(States::new(faint, faint, faint, off))),
            icon: field.glyph().clone(),
            face: FaceLook::of(cx),
            overlay: cx.state_layer(),
        }
    }

    /// The style of a button whose face is filled with `tint`: the label and icons read on it.
    fn tinted(cx: &StyleCx<'_>, tint: Rgba) -> ButtonStyle {
        let mut style = ButtonStyle::of(cx);
        let ink = cx.ink_on(DataColor::new(cx.color(tint).egui()));
        style.text = TextLook::new(TypeRole::Label, Stateful::new(ink));
        style.icon = IconLook::new(cx.control().icon, Stateful::new(ink));
        style
    }
}

/// What a button reports: its response, and the click count and modifiers of this pass's activation.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ButtonOutput {
    /// The button's response.
    pub response: Response,
    /// The press count of the click that activated the button this pass: 1 for a single click, 2 for a double click,
    /// and so on (`craft_behavior::press::count_clicks`). An activation by key or AccessKit counts 1; 0 when the
    /// button was not activated.
    pub clicks: u32,
    /// The modifiers held at the activation (with the click for a pointer one); none when the button was not
    /// activated.
    pub mods: Mods,
}

impl ButtonOutput {
    /// The output of a button-like component `cx` that is `activated` this pass.
    pub(super) fn of<C: Component>(cx: &Cx<'_, C>, activated: bool) -> Self {
        let clicks = match (activated, cx.activation(), cx.press().phase) {
            (true, Some(activation), PressPhase::Release { start, click: true, .. }) if !activation.by_key => start.clicks,
            (true, ..) => 1,
            (false, ..) => 0,
        };
        let mods = match (activated, cx.activation()) {
            (true, Some(activation)) => activation.mods,
            _ => Mods::NONE,
        };
        ButtonOutput { response: cx.response.clone(), clicks, mods }
    }

    /// Whether the button was clicked this pass, by pointer or by key, by the framework's click rule. It stands in
    /// for egui's `Response::clicked`, which also needs a click by egui's input options, which are the app's.
    pub fn clicked(&self) -> bool {
        crate::contract::clicked(&self.response)
    }
}

impl Deref for ButtonOutput {
    type Target = Response;

    fn deref(&self) -> &Response {
        &self.response
    }
}

impl Outcome for ButtonOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A push button. A label wider than the room is cut with an ellipsis and shown whole in a tooltip. While `loading`
/// a spinner takes the place of the icon (of the label, without an icon) at the same width, the button ignores
/// activation, keeps focus and reports busy.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct Button<'a> {
    #[common]
    common: Common,
    /// A leading icon; it mirrors under right-to-left when it has a direction.
    #[prop(into)]
    icon: Option<IconSpec<'a>>,
    /// An icon at the reading end: the chevron of a button that opens a menu.
    #[prop(into)]
    trailing_icon: Option<IconSpec<'a>>,
    /// Working: a spinner, no activation, AccessKit busy.
    #[prop]
    loading: bool,
    /// The text the face paints in place of the label, which stays the accessible name: the label with its access
    /// letter marked.
    #[prop(into)]
    painted: Option<Label>,
    /// The face is filled with this colour from data instead of the variant's fill, and the label reads on it; hover
    /// and press show as the theme's state overlay.
    #[style_key]
    #[prop]
    tint: Option<Rgba>,
    /// The least width, from a size role (a dialog's footer buttons); a longer label is wider.
    #[style_key]
    #[prop]
    least_width: Option<LeastWidth>,
    /// Where the icons and the label sit in a button wider than they are: centred unless set (a full-width button
    /// that names an item at the reading start).
    #[prop]
    align: HAlign,
    /// The face: a push button, or the command-search opener.
    #[style_key]
    #[prop]
    form: ButtonForm,
    /// The command whose chord the Search face shows at its end.
    chord: Option<CommandRef>,
}

impl<'a> Button<'a> {
    /// A button showing `label`, which is also its accessible name.
    pub fn new(label: impl Into<Label>) -> Self {
        Button {
            common: Common::new(),
            icon: None,
            trailing_icon: None,
            loading: false,
            painted: None,
            tint: None,
            least_width: None,
            align: HAlign::Center,
            form: ButtonForm::Push,
            chord: None,
        }
        .label(label)
    }

    /// The command-search opener: a search field's face showing `placeholder` and the chord of `command`. The
    /// placeholder is the button's name; cut to fit, it shows whole in the tooltip. A click, Enter or Space fires
    /// `command`.
    pub fn search(command: impl Into<CommandRef>, placeholder: impl Into<Label>) -> Self {
        let command = command.into();
        let mut button = Button::command(command.clone()).label(placeholder).form(ButtonForm::Search);
        button.icon = Some(IconSpec::svg(IconId::new("search")));
        button.align = HAlign::Start;
        button.chord = Some(command);
        button
    }

    /// A button bound to `command`: its label, enabled state and activation follow the command.
    pub fn command(command: impl Into<CommandRef>) -> Self {
        <Button as Props>::command(Button::new(Label::Empty), command)
    }

    /// The room the face's icons and label take: the content less the chord's slot, `chord` wide, and the gap before it.
    fn face_area(content: Rect, rtl: bool, chord: f32, gap: f32) -> Rect {
        if chord > 0.0 { Flow::new(content, rtl).take_end(chord + gap).1.rect() } else { content }
    }

    fn spec<'s>(&'s self, label: &'s str) -> FaceSpec<'s> {
        let label = self.painted.as_ref().map_or(label, Label::as_str);
        FaceSpec {
            label,
            role: (self.form == ButtonForm::Search).then_some(TypeRole::Body),
            lead: self.icon.as_ref(),
            trail: self.trailing_icon.as_ref(),
            loading: self.loading,
            mnemonic: None,
            align: self.align,
        }
    }
}

impl Component for Button<'_> {
    const ROLE: Role = Role::BUTTON;
    const OVERLAYS: bool = false;
    type Style = ButtonStyle;
    type Output = ButtonOutput;

    fn style(cx: &StyleCx<'_>, key: ButtonStyleKey) -> ButtonStyle {
        let mut style = match (key.form, key.tint) {
            (ButtonForm::Search, _) => ButtonStyle::search(cx),
            (_, Some(tint)) => ButtonStyle::tinted(cx, tint),
            (_, None) => ButtonStyle::of(cx),
        };
        if let Some(least) = key.least_width {
            style.metrics.min_size.x = style.metrics.min_size.x.max(least.points(cx.tokens));
        }
        style
    }

    /// The Search face adds the chord's room, which it keeps at its least too: the placeholder is cut first.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let size = face::measure(cx, &self.spec(cx.resolved.label));
        let chord = self.chord.as_ref().map_or(0.0, |command| cx.face(Kbd::command(command.clone())).x);
        if chord > 0.0 { Vec2::new(size.x + cx.style.face.gap + chord, size.y) } else { size }
    }

    fn sense(&self) -> Sense {
        if self.loading { Sense::FOCUS } else { Sense::CLICK | Sense::FOCUS }
    }

    /// The Search face declares the slot of its chord at the reading end, taken before the placeholder's room; a
    /// loading button declares the slot of its spinner. Each paints as the face of its part.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let (content, rtl) = (cx.content_rect, cx.flow.is_rtl());
        let chord = self.chord.as_ref().map_or(0.0, |command| cx.face(Kbd::command(command.clone())).x);
        if chord > 0.0 {
            let (slot, _) = Flow::new(content, rtl).take_end(chord);
            cx.part(CHORD, slot);
        }
        if !self.loading {
            return;
        }
        let area = Self::face_area(content, rtl, chord, cx.style.face.gap);
        let spinner = face::spinner_rect(cx, &self.spec(cx.resolved.label), &cx.style.face, cx.style.icon.side, area, rtl);
        if let Some(rect) = spinner {
            cx.part(SPINNER, rect);
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        cx.join_roving();
    }

    fn activated(&self, cx: &Cx<'_, Self>) -> bool {
        !self.loading && cx.activation().is_some()
    }

    fn flags(&self) -> crate::style::Flags {
        if self.loading { crate::style::Flags::BUSY } else { crate::style::Flags::empty() }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if let Some(tint) = self.tint {
            let shape = PaintShape::Rect(cx.rect, cx.style.surface.radius);
            ui.draw().data_fill(&shape, DataColor::new(Color32::from(craft_theme::Srgba::from(tint))));
            ui.draw().fill(&shape, cx.resolve(&cx.style.overlay));
        }
        let chord = cx.part_if_declared(CHORD).map_or(0.0, |part| part.rect.width());
        let area = Self::face_area(cx.content_rect, cx.flow.is_rtl(), chord, cx.style.face.gap);
        face::paint(ui, cx, (&cx.style.text, &cx.style.icon), &cx.style.face, &self.spec(cx.resolved.label), area);
        if let (Some(command), true) = (&self.chord, chord > 0.0) {
            ui.face(CHORD, Kbd::command(command.clone()));
        }
        if cx.part_if_declared(SPINNER).is_some() {
            ui.face(SPINNER, Spinner::new().size(cx.resolved.size));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> ButtonOutput {
        ButtonOutput::of(&cx, self.activated(&cx))
    }
}
