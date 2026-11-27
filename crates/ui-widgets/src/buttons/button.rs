//! `Button`: a push button with an optional leading icon or picture, a trailing icon and a busy state.

use super::face::{self, Arrangement, FaceLook, FaceSpec};
use crate::contract::{
    Access, Common, Component, Cx, Hover, LeastWidth, MeasureCx, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, PopupKind, Props, Role,
    Styled as _, WordSetCx as _,
};
use crate::display::{ChordWrite, DisplayWords, Kbd, Picture, Spinner};
use crate::list::ThumbAspect;
use crate::primitives::{HAlign, IconSpec, PaintShape, Wrap};
use crate::runtime::commands::{CommandRef, ResolvedCommand};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, DataColor, FocusLook, IconLook, StyleColor, StyleCx, SurfaceLook, TextLook, TypeRole};
use craft_behavior::{Insets, Metrics, PartKey, PointerButton, PressPhase, Proposal, Sense};
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

/// The part a button with a picture shows it in, in the leading icon's place.
const PICTURE: PartKey = PartKey::new("picture");

/// A side menu row's band under the pointer: its ink at about a tenth over the brand surface.
const SIDE_MENU_HOVER_ALPHA: u8 = 26;
/// The band of the page shown and of a press: its ink at about a sixth, a step above the hover's.
const SIDE_MENU_SHOWN_ALPHA: u8 = 46;
/// A disabled page's name and icon: the ink at a little under half.
const SIDE_MENU_DISABLED_ALPHA: u8 = 110;

/// The style of a push button.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[style(variants = [Variant::SECONDARY, Variant::PRIMARY, Variant::GHOST, Variant::DANGER, Variant::look(Emphasis::Outline, Tone::Accent), super::FILLED])]
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
        style.icon = IconLook::new(super::small_icon(cx, cx.control().icon), Stateful::new(ink));
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
    /// The pointer's stay over the button: hovered, entered, left and clicked under the pointer.
    pub hover: Hover,
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
        ButtonOutput { response: cx.response.clone(), clicks, mods, held, hover: cx.hover() }
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
/// a spinner takes the place of the icon or picture (of the label, without either) at the same width, the button
/// ignores activation, keeps focus and reports busy.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct Button<'a> {
    #[common]
    common: Common,
    /// A leading icon; it mirrors under right-to-left when it has a direction.
    icon: Option<IconSpec<'a>>,
    /// A rendered picture in the leading icon's place, with the width of its box as a multiple of its height.
    picture: Option<(ThumbAspect, Picture<'a>)>,
    /// An icon at the reading end: the chevron of a button that opens a menu.
    #[prop(into)]
    trailing_icon: Option<IconSpec<'a>>,
    /// Working: a spinner, no activation, AccessKit busy.
    #[prop]
    loading: bool,
    /// Text the button keeps room for: the face is as wide as the wider of its label and this text, so a button whose
    /// label changes (a zoom percent) keeps one width. The label is placed as usual.
    #[prop(into)]
    room_for: Option<Label>,
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
    /// A muted caption under the icon and label of an inline button. It is the accessible description unless
    /// `description` is set.
    #[prop(into)]
    detail: Option<Label>,
    /// Whether the detail wraps in layouts that measure the button at its assigned width. By default the detail
    /// uses its natural width.
    #[prop]
    wrap_detail: bool,
    /// Whether a command button shows its command's chord after the label.
    #[prop]
    chord: ButtonChord,
    /// The kind of popup the button opens: AccessKit reports it.
    #[prop(into)]
    popup: Option<PopupKind>,
    /// Whether the popup the button opens is open, as the host knows it.
    #[prop(into)]
    expanded: Option<bool>,
    /// How the icon and label are laid out: beside each other, or the icon over the label.
    #[style_key]
    #[prop]
    arrangement: Arrangement,
}

impl<'a> Button<'a> {
    /// A button showing `label`, which is also its accessible name.
    pub fn new(label: impl Into<Label>) -> Self {
        Button {
            common: Common::new(),
            icon: None,
            picture: None,
            trailing_icon: None,
            loading: false,
            room_for: None,
            painted: None,
            tint: None,
            least_width: None,
            align: HAlign::Center,
            detail: None,
            wrap_detail: false,
            chord: ButtonChord::Hidden,
            popup: None,
            expanded: None,
            arrangement: Arrangement::default(),
        }
        .label(label)
    }

    /// A button bound to `command`: its label, enabled state and activation follow the command.
    pub fn command(command: impl Into<CommandRef>) -> Self {
        <Button as Props>::command(Button::new(Label::Empty), command)
    }

    /// A leading icon; it mirrors under right-to-left when it has a direction. The last of `icon` and `picture`
    /// called wins.
    pub fn icon(mut self, icon: impl Into<IconSpec<'a>>) -> Self {
        self.icon = Some(icon.into());
        self.picture = None;
        self
    }

    /// The button shows `picture` in the leading icon's place, as high as the icon and `aspect` times as wide, as its
    /// own component in that box (a rendered equation, a stored image). The picture is decoration: press, focus, hit
    /// area, keytip and accessible name stay the button's. The last of `icon` and `picture` called wins.
    pub fn picture(mut self, aspect: ThumbAspect, picture: Picture<'a>) -> Self {
        self.picture = Some((aspect, picture));
        self.icon = None;
        self
    }

    /// The face as it shows `label` in `style`, with `chord` after it: the painted text in its place when set.
    fn spec<'s>(&'s self, label: &'s str, style: &ButtonStyle, chord: Option<&'s str>) -> FaceSpec<'s> {
        self.face_of(self.painted.as_ref().map_or(label, Label::as_str), style, chord)
    }

    /// The face showing `label` as it is, with `chord` after it: in the label role, unless `style` names another (a
    /// side menu's row).
    fn face_of<'s>(&'s self, label: &'s str, style: &ButtonStyle, chord: Option<&'s str>) -> FaceSpec<'s> {
        let detail_wrap = if self.wrap_detail { Wrap::Wrap } else { Wrap::Line };
        FaceSpec {
            arrangement: self.arrangement,
            label,
            role: (style.text.role != TypeRole::Label).then_some(style.text.role),
            lead: self.icon.as_ref(),
            picture: self.picture.as_ref().map(|(aspect, _)| *aspect),
            trail: self.trailing_icon.as_ref(),
            loading: self.loading,
            mnemonic: None,
            align: self.align,
            chord,
            detail: self.detail.as_ref().map(|detail| (detail.as_str(), detail_wrap)),
        }
    }

    /// The chord a `Trailing` button shows: the bound `command`'s, written as `Kbd` writes it for the scope's OS and
    /// with its key names (`keys`, asked only then, so a button with no chord reads neither). `None` for a hidden chord
    /// and for a command with none.
    fn chord_text(&self, command: Option<&ResolvedCommand>, keys: impl FnOnce() -> (Os, Arc<DisplayWords>)) -> Option<String> {
        let chord = command.filter(|_| self.chord == ButtonChord::Trailing)?.chord?;
        let (os, words) = keys();
        Some(Kbd::text(&Trigger::from(chord), os, &words, ChordWrite::default()))
    }
}

/// A side menu stands on the brand surface: its names and icons read in the ink of strong fills, a disabled page's
/// dimmed, and its bands are that ink laid thinly over the brand colour (hover lighter than the page shown), not the
/// panel's state fills, which painted a light band with dark text over the rail. The row of the page shown (`on`)
/// keeps its band, so the menu marks where the page is. Returns the ink and the band fill, for every control of the
/// place (`Button`, `IconButton`).
pub(super) fn side_menu_ink(cx: &StyleCx<'_>) -> (craft_theme::Stateful<StyleColor>, craft_theme::Stateful<StyleColor>) {
    let ink = cx.tokens.color.text.on_strong;
    let (rest, dim) = (cx.color(ink), cx.color(ink.with_alpha(SIDE_MENU_DISABLED_ALPHA)));
    let text = craft_theme::Stateful::new(rest).off(craft_theme::States::new(rest, rest, rest, dim));
    let (clear, hover, shown) =
        (cx.color(Rgba::TRANSPARENT), cx.color(ink.with_alpha(SIDE_MENU_HOVER_ALPHA)), cx.color(ink.with_alpha(SIDE_MENU_SHOWN_ALPHA)));
    let fill = craft_theme::Stateful::new(clear)
        .off(craft_theme::States::new(clear, hover, shown, clear))
        .on(craft_theme::States::new(shown, shown, shown, shown));
    (text, fill)
}

impl Component for Button<'_> {
    const ROLE: Role = Role::BUTTON;
    const OVERLAYS: bool = false;
    type Style = ButtonStyle;
    type Output = ButtonOutput;

    fn default_variant(env: &crate::runtime::env::Env) -> Option<Variant> {
        super::push_default(env)
    }

    fn style(cx: &StyleCx<'_>, key: ButtonStyleKey) -> ButtonStyle {
        let cx = &super::placed(cx);
        let mut style = match key.tint {
            Some(tint) => ButtonStyle::tinted(cx, tint),
            None => ButtonStyle::of(cx),
        };
        if key.arrangement == Arrangement::Stacked {
            style.metrics = face::stacked_metrics(cx, &cx.variant_look());
        }
        if let Some(least) = key.least_width {
            style.metrics.min_size.x = style.metrics.min_size.x.max(least.points(cx.tokens));
            // A dialog's footer button stands a dialog button high.
            if matches!(least, LeastWidth::DialogButton | LeastWidth::DialogSideButton) && key.arrangement == Arrangement::Inline {
                style.metrics.min_size.y = cx.dialog_button_height();
            }
        }
        if cx.env::<super::ControlPlace>() == super::ControlPlace::CallToAction {
            style.metrics.min_size.y = style.metrics.min_size.y.max(cx.tokens.size.page.action);
            style.text.role = TypeRole::BodyStrong;
        }
        if cx.env::<super::ControlPlace>() == super::ControlPlace::PageAction {
            let page = &cx.tokens.size.page;
            let padding = &mut style.metrics.padding;
            style.metrics.min_size.y = style.metrics.min_size.y.max(page.button);
            (padding.start, padding.end) = (padding.start.max(page.button_side), padding.end.max(page.button_side));
        }
        // A side menu's row: the large body text, padded as a list row of places, not as a control.
        if cx.env::<super::ControlPlace>() == super::ControlPlace::SideMenu && key.arrangement == Arrangement::Inline {
            let pad = &cx.tokens.space.pad;
            style.text.role = TypeRole::BodyLarge;
            style.metrics.padding = Insets::new(pad.roomy, pad.roomy, pad.segment, pad.segment);
            let (text, fill) = side_menu_ink(cx);
            style.text.color = text.clone();
            // A place's icon is the large icon, not a control's standard one, set off from its name by a section gap: a
            // list of places reads by its icons (WordCraft's File rail: 18 pt icons, nearest token 20 pt, 12 pt before 14 pt names).
            style.icon = IconLook::new(style.icon.side.max(cx.tokens.size.icon.lg), text);
            // The face paints by its gap and the measure by the metrics', so the two stay one.
            style.face.gap = style.face.gap.max(cx.tokens.space.gap.section);
            style.metrics.gap = style.face.gap;
            style.surface.fill = fill;
        }
        style
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let chord = self.chord_text(cx.command(), || (cx.env.shell().os, cx.words::<DisplayWords>()));
        let spec = self.spec(cx.resolved.label, cx.style, chord.as_deref());
        let shown = face::measure(cx, &spec);
        let mut size = match &self.room_for {
            Some(room) => shown.max(face::measure(cx, &self.face_of(room.as_str(), cx.style, chord.as_deref()))),
            None => shown,
        };
        if cx.proposal == Proposal::Min
            && self.arrangement == Arrangement::Inline
            && let Some(detail) = spec.detail_run().filter(|run| run.wrap == Wrap::Wrap)
        {
            // The title reservation and style floor can widen the face before its caption wraps.
            let metrics = cx.style.metrics;
            let floor = metrics.least_width.unwrap_or(metrics.min_size.x) - metrics.padding.start - metrics.padding.end;
            let width = cx.room.filter(|room| room.is_finite()).unwrap_or_else(|| size.x.max(floor));
            size.y += cx.layout_at(&detail, width).size().y - cx.least(&detail).size().y;
        }
        size
    }

    fn sense(&self) -> Sense {
        if self.loading { Sense::FOCUS } else { Sense::CLICK | Sense::FOCUS }
    }

    /// A loading button declares the slot of its spinner, which paints as the face of that part; any other button with
    /// a picture declares the picture's box, where `paint` shows it.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let (area, rtl) = (cx.content_rect, cx.flow.is_rtl());
        if self.loading {
            // A busy face shows no chord.
            let spinner = face::spinner_rect(cx, &self.spec(cx.resolved.label, cx.style, None), &cx.style.face, cx.style.icon.side, area, rtl);
            if let Some(rect) = spinner {
                cx.part(SPINNER, rect);
            }
        } else if self.picture.is_some() {
            let chord = self.chord_text(cx.command(), || (cx.env.shell().os, cx.words::<DisplayWords>()));
            let lead = face::lead_rect(cx, &self.spec(cx.resolved.label, cx.style, chord.as_deref()), &cx.style.face, cx.style.icon.side, area, rtl);
            if let Some(rect) = lead {
                cx.part(PICTURE, rect);
            }
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
        let chord = self.chord_text(cx.command.as_deref(), || (cx.env.shell().os, cx.words::<DisplayWords>()));
        face::paint(
            ui,
            cx,
            (&cx.style.text, &cx.style.icon),
            &cx.style.face,
            &self.spec(cx.resolved.label, cx.style, chord.as_deref()),
            cx.content_rect,
        );
        if cx.part_if_declared(SPINNER).is_some() {
            ui.face(SPINNER, Spinner::new().size(cx.resolved.size));
        }
        if let (Some(part), Some((_, picture))) = (cx.part_if_declared(PICTURE), self.picture.take()) {
            // The button's name names the picture: shown unlabelled, it is no access node and senses nothing.
            let _ = ui.child("picture", part.rect, picture.label(Label::Empty));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> ButtonOutput {
        ButtonOutput::of(&cx, self.activated(&cx))
    }
}
