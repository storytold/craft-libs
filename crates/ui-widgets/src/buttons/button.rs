//! `Button`: a push button with an optional leading and trailing icon and a busy state.

use super::face::{self, FaceLook, FaceSpec};
use crate::contract::{
    Access, Common, Component, Cx, LeastWidth, MeasureCx, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, PopupKind, Props, Role, Styled as _, WordSetCx as _,
};
use crate::display::{ChordWrite, DisplayWords, Kbd, Spinner};
use crate::primitives::{HAlign, IconSpec, PaintShape};
use crate::runtime::commands::{CommandRef, ResolvedCommand};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, DataColor, FocusLook, IconLook, StyleColor, StyleCx, SurfaceLook, TextLook, TypeRole};
use craft_behavior::{Metrics, PartKey, PointerButton, PressPhase, Sense};
use craft_host::target::Os;
use craft_keymap::{Mods, Trigger};
use craft_theme::{
    Rgba, Stateful, Variant,
    variant::{Emphasis, Tone},
};
use egui::{Color32, Rect, Response, Vec2};
use std::ops::Deref;
use std::sync::Arc;

/// The part a loading button shows its spinner in.
const SPINNER: PartKey = PartKey::new("spinner");

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

    /// The style of a button whose face is filled with `tint`: the label and icons read on it.
    fn tinted(cx: &StyleCx<'_>, tint: Rgba) -> ButtonStyle {
        let mut style = ButtonStyle::of(cx);
        let ink = cx.ink_on(DataColor::new(cx.color(tint).egui()));
        style.text = TextLook::new(TypeRole::Label, Stateful::new(ink));
        style.icon = IconLook::new(cx.control().icon, Stateful::new(ink));
        style.face.muted = Stateful::new(ink);
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
    /// True in every pass from the pointer press on the button to its release or cancel, also after the pointer leaves
    /// the button or the window. A key or AccessKit activation never sets it.
    pub held: bool,
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
        let held = cx.press().phase.button_down() && cx.press().phase.start().is_some_and(|start| start.button == PointerButton::Primary);
        ButtonOutput { response: cx.response.clone(), clicks, mods, held }
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

/// Whether a command button shows its command's chord.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ButtonChord {
    /// No chord.
    #[default]
    Hidden,
    /// The command's chord after the label, in the `Kbd` notation; a command with no chord shows none. The chord is
    /// dropped first when room is short.
    Trailing,
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
    /// A muted caption-role second line, wrapped under the icon and label. The face grows to fit it. It is the
    /// accessible description unless `description` is set.
    #[prop(into)]
    detail: Option<Label>,
    /// Whether a command button shows its command's chord after the label.
    #[prop]
    chord: ButtonChord,
    /// The kind of popup the button opens: AccessKit reports it.
    #[prop(into)]
    popup: Option<PopupKind>,
    /// Whether the popup the button opens is open, as the host knows it.
    #[prop(into)]
    expanded: Option<bool>,
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
            detail: None,
            chord: ButtonChord::Hidden,
            popup: None,
            expanded: None,
        }
        .label(label)
    }

    /// A button bound to `command`: its label, enabled state and activation follow the command.
    pub fn command(command: impl Into<CommandRef>) -> Self {
        <Button as Props>::command(Button::new(Label::Empty), command)
    }

    /// The face of the button, with `chord` after the label.
    fn spec<'s>(&'s self, label: &'s str, chord: Option<&'s str>) -> FaceSpec<'s> {
        let label = self.painted.as_ref().map_or(label, Label::as_str);
        FaceSpec {
            label,
            role: None,
            lead: self.icon.as_ref(),
            trail: self.trailing_icon.as_ref(),
            loading: self.loading,
            mnemonic: None,
            align: self.align,
            chord,
            detail: self.detail.as_ref().map(Label::as_str),
        }
    }

    /// The chord a `Trailing` button shows: the bound `command`'s, written as `Kbd` writes it with the scope's key
    /// names (`words`, asked only then). `None` for a hidden chord and for a command with none.
    fn chord_text(&self, command: Option<&ResolvedCommand>, os: Os, words: impl FnOnce() -> Arc<DisplayWords>) -> Option<String> {
        let chord = command.filter(|_| self.chord == ButtonChord::Trailing)?.chord?;
        Some(Kbd::text(&Trigger::from(chord), os, &words(), ChordWrite::default()))
    }
}

impl Component for Button<'_> {
    const ROLE: Role = Role::BUTTON;
    const OVERLAYS: bool = false;
    type Style = ButtonStyle;
    type Output = ButtonOutput;

    fn style(cx: &StyleCx<'_>, key: ButtonStyleKey) -> ButtonStyle {
        let mut style = match key.tint {
            Some(tint) => ButtonStyle::tinted(cx, tint),
            None => ButtonStyle::of(cx),
        };
        if let Some(least) = key.least_width {
            style.metrics.min_size.x = style.metrics.min_size.x.max(least.points(cx.tokens));
        }
        style
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let chord = self.chord_text(cx.command(), cx.env.shell().os, || cx.words::<DisplayWords>());
        face::measure(cx, &self.spec(cx.resolved.label, chord.as_deref()))
    }

    fn sense(&self) -> Sense {
        if self.loading { Sense::FOCUS } else { Sense::CLICK | Sense::FOCUS }
    }

    /// A loading button declares the slot of its spinner, which paints as the face of that part.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        if !self.loading {
            return;
        }
        let (area, rtl) = (cx.content_rect, cx.flow.is_rtl());
        let spinner = face::spinner_rect(cx, &self.spec(cx.resolved.label, None), &cx.style.face, cx.style.icon.side, area, rtl);
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

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let detail = self.detail.as_ref().map(Label::as_str).filter(|detail| !detail.is_empty());
        if let Some(detail) = detail
            && cx.resolved.description.is_none()
        {
            access.description(detail);
        }
        if let Some(kind) = self.popup {
            access.has_popup(kind);
        }
        if let Some(open) = self.expanded {
            access.expanded(open);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if let Some(tint) = self.tint {
            let shape = PaintShape::Rect(cx.rect, cx.style.surface.radius);
            ui.draw().data_fill(&shape, DataColor::new(Color32::from(craft_theme::Srgba::from(tint))));
            ui.draw().fill(&shape, cx.resolve(&cx.style.overlay));
        }
        let chord = self.chord_text(cx.command.as_deref(), cx.env.shell().os, || cx.words::<DisplayWords>());
        face::paint(ui, cx, (&cx.style.text, &cx.style.icon), &cx.style.face, &self.spec(cx.resolved.label, chord.as_deref()), cx.content_rect);
        if cx.part_if_declared(SPINNER).is_some() {
            ui.face(SPINNER, Spinner::new().size(cx.resolved.size));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> ButtonOutput {
        ButtonOutput::of(&cx, self.activated(&cx))
    }
}
