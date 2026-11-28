//! `Notice` (20.3): a message in a tone with an icon, an optional title, actions and a dismiss button. One leaf
//! covers the inline notice, the toast card and the empty state; the layout is a style key. Actions and the
//! dismiss button are parts that wear the faces of `Link`, `Button` and `IconButton`.

use super::words::NoticeWords;
use crate::buttons::{Button, IconButton, Link};
use crate::contract::{
    Access, Common, Component, Cx, Hosted, Hosting, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx, Politeness, Props as _, Role, Show,
    Styled as _, WordSetCx as _,
};
use crate::primitives::{HAlign, IconSpec, TextRun, Wrap};
use crate::runtime::commands::CommandRef;
use crate::runtime::env::{EnvView, Label};
use crate::style::{ComponentStyle, FocusLook, IconLook, Insets, StyleColor, StyleCx, SurfaceKind, SurfaceLook, TextLook, TypeRole, UiTokens};
use craft_behavior::{Flow, IconId, Metrics, NavShape, PartKey, Proposal, Roving, Sense};
use craft_i18n::Format;
use craft_theme::{Rgba, Stateful, States, Variant, variant::Tone};
use egui::{Align, Pos2, Rect, Response, Vec2};

/// Where a notice sits and how much room it takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NoticeLayout {
    /// The icon at the reading start and the text beside it, across the width offered, with no surface.
    #[default]
    Inline,
    /// A full-width status surface with button actions, above a document or workspace.
    Bar,
    /// The inline layout on a floating surface, the card width wide where the room allows: a toast. A card of a status
    /// tone draws its edge and title in the tone's colour.
    Card,
    /// The card on the inverse surface (`surface.inverse`, with `text.on_inverse`): light on a dark theme, dark on a
    /// light one. Toasts use it. The size and layout are the card's; a status tone draws its icon and edge in the
    /// tone's colour.
    Toast,
    /// Icon, title, message and actions in a column centred in the room, at most the reading width wide: an empty
    /// state. The first action is the primary button and the others secondary, all as wide as the widest, side by side
    /// or stacked when they do not fit.
    Centred,
}

/// How many large icons wide the picture of a centred notice (an empty state) is.
const EMPTY_ICON_IN_LARGE: f32 = 1.5;

/// The icon of a notice.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum NoticeIcon<'a> {
    /// The tone's icon; the neutral tone has none.
    #[default]
    Tone,
    /// No icon.
    None,
    /// This icon, in the tone's colour unless the spec names another.
    Custom(IconSpec<'a>),
}

/// An action of a notice: a label, or a command that supplies it and runs when the action is taken.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct NoticeAction {
    label: Label,
    command: Option<CommandRef>,
    enabled: bool,
    default: bool,
}

impl NoticeAction {
    /// An action named `label`; the notice reports it as `NoticeOutput.action`.
    pub fn new(label: impl Into<Label>) -> Self {
        NoticeAction { label: label.into(), command: None, enabled: true, default: false }
    }

    /// An action that shows `command`'s label and fires it when taken; the notice reports it as well.
    pub fn command(command: impl Into<CommandRef>) -> Self {
        NoticeAction { label: Label::Empty, command: Some(command.into()), enabled: true, default: false }
    }

    /// Sets whether the action can be focused or taken. Disabled actions remain visible.
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Makes this the notice's default action: a layer with `Initial::Default` focuses it when it opens, so Enter
    /// takes it. When several enabled actions are marked, the last one is the default.
    #[must_use]
    pub fn default_action(mut self) -> Self {
        self.default = true;
        self
    }
}

/// What a notice reports: its response, the action taken this pass and whether dismiss was pressed.
#[derive(Debug)]
#[non_exhaustive]
pub struct NoticeOutput {
    /// The notice's response; its hover tells a toast host to hold the timer.
    pub response: Response,
    /// The index of the action taken this pass.
    pub action: Option<usize>,
    /// The dismiss button was pressed this pass.
    pub dismissed: bool,
}

impl Outcome for NoticeOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

const ACTION: PartKey = PartKey::new("action");
const DISMISS: PartKey = PartKey::new("dismiss");
/// The title's band; the centred layout names it as a heading.
const TITLE: PartKey = PartKey::new("title");
/// The title's text and the message's: selection parts.
const TITLE_TEXT: PartKey = PartKey::new("title_text");
/// The byline beside a brand title: a selection part.
const BYLINE: PartKey = PartKey::new("byline");
const MESSAGE: PartKey = PartKey::new("message");
/// A bullet line's text, and the line that stands for the lines left out: selection parts.
const LINE: PartKey = PartKey::new("line");
const MORE: PartKey = PartKey::new("more");
/// The key of a hosted leaf's slot.
const CONTENT: &str = "content";
/// Most bullet lines a notice shows; the rest become one "…and n more" line.
const MAX_LINES: usize = 8;
/// The mark before each bullet line.
const BULLET: &str = "\u{2022}";
/// The heading level of the centred title: the empty state's top-level heading.
const TITLE_LEVEL: usize = 1;
const DISMISS_ICON: &str = "close";

/// The style of a notice.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct NoticeStyle {
    /// A floating surface in the card layout, none otherwise.
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The title's role and colour.
    #[style(text)]
    title: TextLook,
    /// A brand title's role and colour.
    #[style(text)]
    brand: TextLook,
    /// The byline beside a brand title.
    #[style(text)]
    byline: TextLook,
    /// How far a byline beside a brand title sits above the title's bottom edge.
    byline_lift: f32,
    /// Between the rows of an inline or card notice: its head, each bullet line, the "…and n more" line and the
    /// actions (at least the gap before the actions).
    stack: f32,
    /// The message's role and colour.
    #[style(text)]
    message: TextLook,
    /// The message's role and colour under a title.
    #[style(text)]
    detail: TextLook,
    /// The bullet lines' role and colour.
    #[style(text)]
    line: TextLook,
    /// The colour of the line that stands for the bullet lines left out.
    #[style(text)]
    more: TextLook,
    /// The icon's side and colour.
    #[style(icon)]
    icon: IconLook,
    /// The widest column of the centred layout.
    reading_width: f32,
}

/// The widest column of a centred notice, in body text sizes: about 460 at the 12 point body, the empty states'
/// column in the source apps.
const READING_IN_BODY_SIZES: f32 = 38.5;

/// The width of every card of a toast stack, padding included, where the room allows, in points, in every density.
const CARD_WIDTH: f32 = 360.0;

/// A message in a tone. The tone picks the icon and the colours; the neutral tone reads muted and has no icon. The
/// message is the label; a title above it is optional. A centred notice can host the app's own leaves under its
/// actions (a drop hint, "No recent files"). Actions read as links (as buttons in the centred layout) and
/// are reported in the output; with `dismiss` a close button is the last part. Bullet lines under the message (an
/// inline or card notice's list of details) show up to eight, then one "…and n more" line. With `announce` set, the notice is a
/// live region: its title and message are announced when it appears and whenever they change. The title and message wrap and can be selected and copied; Tab passes over them.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct Notice<'a> {
    #[common]
    common: Common,
    /// The tone: the icon and the colours.
    #[style_key]
    tone: Tone,
    /// The layout.
    #[style_key]
    #[prop]
    layout: NoticeLayout,
    /// A title above the message.
    #[prop(into)]
    title: Option<Label>,
    /// Draws the title of a centred notice as the app's brand: the title in the display role with this byline after it,
    /// or under it when both don't fit. The title stays the heading.
    #[prop(into)]
    brand: Option<Label>,
    /// The icon.
    #[prop]
    icon: NoticeIcon<'a>,
    /// Shows a dismiss button.
    #[prop(flag)]
    dismiss: bool,
    /// The actions are separate Tab stops, like the buttons of a dialog footer, instead of one roving group: the
    /// arrow keys are left to the surrounding scope.
    #[prop(flag)]
    footer_actions: bool,
    /// Makes the notice a live region of this politeness: its title and message are announced when it appears and
    /// whenever they change.
    #[prop]
    announce: Option<Politeness>,
    actions: Vec<NoticeAction>,
    lines: Vec<Label>,
    /// The host's leaves: under the actions of a centred notice, between the bullet lines and the actions of an inline
    /// or card notice; and their sizes in this pass.
    content: Vec<Box<dyn for<'r> Hosted<Notice<'r>> + 'a>>,
    content_sizes: Vec<Vec2>,
    placed: Option<Placed>,
    taken: Option<usize>,
    dismissed: bool,
}

/// Where each element sits, decided with the final size in `parts`.
#[derive(Clone, Default)]
struct Placed {
    icon: Option<Rect>,
    title: Option<Rect>,
    byline: Option<Rect>,
    message: Option<Rect>,
    /// Each shown line's bullet and text, then the "…and n more" line.
    lines: Vec<(Rect, Rect)>,
    more: Option<Rect>,
    actions: Vec<Rect>,
    content: Vec<Rect>,
    /// The actions' labels, with commands resolved.
    labels: Vec<Label>,
    dismiss: Option<Rect>,
}

/// Sizes of the elements for one width.
struct Plan {
    icon: Option<f32>,
    /// The title's block: a brand title and its byline together.
    title: Option<Vec2>,
    /// A brand title's size, its byline's, and whether the byline sits beside the title.
    brand: Option<(Vec2, Vec2, bool)>,
    message: Vec2,
    /// The bullet's width, each shown line's height and the "…and n more" line's height.
    bullet: f32,
    lines: Vec<f32>,
    more: Option<f32>,
    actions: Vec<Vec2>,
    /// The action index range and height of each wrapped row in a row layout; empty for the centred layout.
    action_rows: Vec<(std::ops::Range<usize>, f32)>,
    inline_actions: bool,
    dismiss: Option<Vec2>,
    /// The height of the head line in a row layout, where the icon, the first line of the text and the dismiss button
    /// share one centre line; and how far the text sits below the head's top to put its first line there.
    line: f32,
    text_offset: f32,
    /// The actions sit one under the other because they do not fit side by side.
    stacked: bool,
    /// The width the elements need at this width: the natural width when it is unbounded.
    needs: f32,
    height: f32,
}

/// The texts of a notice as resolved for this pass.
struct Texts<'t> {
    title: Option<String>,
    /// The byline of a centred brand title.
    byline: Option<String>,
    message: &'t str,
    /// The shown bullet lines, then the "…and n more" text when lines were left out.
    lines: Vec<String>,
    more: Option<String>,
}

impl<'a> Notice<'a> {
    /// A notice of `tone` saying `message`.
    pub fn new(tone: Tone, message: impl Into<Label>) -> Self {
        Notice {
            common: Common::new(),
            tone,
            layout: NoticeLayout::Inline,
            title: None,
            brand: None,
            icon: NoticeIcon::Tone,
            dismiss: false,
            footer_actions: false,
            announce: None,
            actions: Vec::new(),
            lines: Vec::new(),
            content: Vec::new(),
            content_sizes: Vec::new(),
            placed: None,
            taken: None,
            dismissed: false,
        }
        .label(message)
    }

    /// Adds an action after the message, in reading order.
    #[must_use]
    pub fn action(mut self, action: NoticeAction) -> Self {
        self.actions.push(action);
        self
    }

    /// Bullet lines under the message, in an inline or card notice: up to eight show, then one "…and n more" line.
    #[must_use]
    pub fn lines<L: Into<Label>>(mut self, lines: impl IntoIterator<Item = L>) -> Self {
        self.lines = lines.into_iter().map(Into::into).collect();
        self
    }

    /// Hosts `component` after the leaves added before it: under the actions of a centred notice, and in an inline or
    /// card notice under the message and its bullet lines, before the actions, across the whole width. Its output goes
    /// to `output` in the pass it is shown.
    #[must_use]
    pub fn content<C: Show + 'a>(mut self, component: C, output: impl FnOnce(C::Output) + 'a) -> Self {
        self.content.push(Box::new(Hosting::new(component, output)));
        self
    }

    /// Measures the hosted leaves for this pass.
    fn measure_content(&mut self, cx: &MeasureCx<'_, Self>) {
        self.content_sizes = self.content.iter_mut().enumerate().map(|(at, item)| item.measure(cx, (CONTENT, at), Proposal::Ideal)).collect();
    }

    fn texts<'t>(&self, env: &EnvView<'_>, words: &NoticeWords, format: &Format, message: &'t str) -> Texts<'t> {
        let shown = if self.row() { self.lines.len().min(MAX_LINES) } else { 0 };
        let lines = self.lines[..shown].iter().map(|line| env.resolve(line).as_str().to_owned()).collect();
        let left = if self.row() { self.lines.len() - shown } else { 0 };
        let more = (left > 0).then(|| words.more.fill(&[("n", format.integer(left as u64).as_str())]));
        let byline = self.brand.as_ref().filter(|_| !self.row()).map(|byline| env.resolve(byline).as_str().to_owned());
        Texts { title: self.title.as_ref().map(|title| env.resolve(title).as_str().to_owned()), byline, message, lines, more }
    }

    fn icon_spec(&self) -> Option<IconSpec<'a>> {
        match &self.icon {
            NoticeIcon::Tone => tone_icon(self.tone).map(|id| IconSpec::svg(IconId::new(id))),
            NoticeIcon::Custom(spec) => Some(spec.clone()),
            _ => None,
        }
    }

    fn action_labels(&self, cx: &MeasureCx<'_, Self>) -> Vec<Label> {
        self.actions
            .iter()
            .map(|action| match &action.command {
                Some(command) => cx.resolve_command(command.clone()).map_or(Label::Empty, |resolved| Label::from(resolved.label.clone())),
                None => action.label.clone(),
            })
            .collect()
    }

    /// A text of the notice: the centred layout centres its lines.
    fn run(&self, text: &str, role: TypeRole) -> TextRun<'static> {
        let run = TextRun::new(text.to_owned()).role(role).wrap(Wrap::Wrap);
        if self.layout == NoticeLayout::Centred { run.align(HAlign::Center) } else { run }
    }

    /// The title's look: the brand look for a centred brand title.
    fn title_look<'s>(&self, style: &'s NoticeStyle) -> &'s TextLook {
        if self.brand.is_some() && !self.row() { &style.brand } else { &style.title }
    }

    /// The width the elements wrap at when the notice is `width` wide.
    fn wrap_width(&self, cx: &MeasureCx<'_, Self>, width: f32) -> f32 {
        if self.layout == NoticeLayout::Centred { width.min(cx.style.reading_width) } else { width }
    }

    fn row(&self) -> bool {
        self.layout != NoticeLayout::Centred
    }

    /// The face of a centred notice's action `at`: the first is the primary button.
    fn button(at: usize, label: Label) -> Button<'static> {
        Button::new(label).variant(if at == 0 { Variant::PRIMARY } else { Variant::SECONDARY })
    }

    /// The face of an inline or card notice's action; a toast's reads in the card's text color.
    fn link(&self, role: TypeRole, label: Label) -> Link {
        let link = Link::new(label).role(role);
        if self.layout == NoticeLayout::Toast { link.on_inverse() } else { link }
    }

    /// The sizes of the actions' faces; a centred notice's buttons all take the widest one's width.
    fn action_sizes(&self, cx: &MeasureCx<'_, Self>) -> Vec<Vec2> {
        let labels = self.action_labels(cx);
        if self.row() && self.layout != NoticeLayout::Bar {
            return labels.into_iter().map(|label| cx.face(self.link(cx.style.message.role, label))).collect();
        }
        let sizes: Vec<Vec2> = labels.into_iter().enumerate().map(|(at, label)| cx.face(Self::button(at, label))).collect();
        if self.layout == NoticeLayout::Bar {
            return sizes;
        }
        let widest = sizes.iter().map(|size| size.x).fold(0.0, f32::max);
        sizes.into_iter().map(|size| Vec2::new(widest, size.y)).collect()
    }

    /// Sizes every element for `width`, which is infinite to find the natural width.
    fn plan(&self, cx: &MeasureCx<'_, Self>, texts: &Texts<'_>, width: f32) -> Plan {
        let gap = cx.gap();
        let words = cx.words::<NoticeWords>();
        let row = self.row();
        let icon = self.icon_spec().map(|_| cx.style.icon.side);
        let dismiss = (self.dismiss && row).then(|| cx.face(IconButton::new(IconId::new(DISMISS_ICON)).label(words.dismiss.clone())));
        let actions = self.action_sizes(cx);
        let icon_width = if row { icon.map_or(0.0, |side| side + gap) } else { 0.0 };
        let dismiss_width = dismiss.map_or(0.0, |size| size.x + gap);
        let actions_width = actions.iter().map(|size| size.x).sum::<f32>() + gap * actions.len().saturating_sub(1) as f32;
        let remaining = width - icon_width - dismiss_width - actions_width - gap;
        let inline_actions = matches!(self.layout, NoticeLayout::Inline | NoticeLayout::Bar)
            && !actions.is_empty()
            && (!width.is_finite() || remaining >= actions_width);
        let action_width = if inline_actions { actions_width + gap } else { 0.0 };
        let column = if width.is_finite() { (width - icon_width - dismiss_width - action_width).max(0.0) } else { f32::INFINITY };
        let run = |text: &str, role: TypeRole| cx.layout_at(&self.run(text, role), column).size();
        let title_role = self.title_look(cx.style).role;
        let title_layout = texts.title.as_deref().map(|title| cx.layout_at(&self.run(title, title_role), column));
        let mut title = title_layout.as_ref().map(|layout| layout.size());
        let brand = title.zip(texts.byline.as_deref()).map(|(name, byline)| {
            let byline = run(byline, cx.style.byline.role);
            let beside = name.x + gap + byline.x <= column;
            (name, byline, beside)
        });
        if let Some((name, byline, beside)) = brand {
            title = Some(if beside { Vec2::new(name.x + gap + byline.x, name.y) } else { Vec2::new(name.x.max(byline.x), name.y + gap + byline.y) });
        }
        let message_role = if self.title.is_some() { cx.style.detail.role } else { cx.style.message.role };
        let message_layout = (!texts.message.is_empty()).then(|| cx.layout_at(&self.run(texts.message, message_role), column));
        let message = message_layout.as_ref().map_or(Vec2::ZERO, |layout| layout.size());
        let bullet = if texts.lines.is_empty() { 0.0 } else { cx.layout(&TextRun::new(BULLET).role(cx.style.line.role)).size().x + gap };
        // The bullet lines run under the icon and the dismiss button as well, across the whole width.
        let line_column = if width.is_finite() { (width - bullet).max(0.0) } else { f32::INFINITY };
        let line_sizes: Vec<Vec2> = texts.lines.iter().map(|line| cx.layout_at(&self.run(line, cx.style.line.role), line_column).size()).collect();
        let more = texts.more.as_deref().map(|more| cx.layout_at(&self.run(more, cx.style.more.role), width).size());
        let stack = cx.style.stack;
        let content_width = self.content_sizes.iter().map(|size| size.x).fold(0.0, f32::max);
        // In a row layout the lines, the hosted leaves and the actions run from the content's start edge, under the icon.
        let below_width = if row { actions_width.max(content_width) } else { 0.0 };
        let lines_width = line_sizes.iter().map(|size| size.x + bullet).fold(more.map_or(below_width, |size| size.x.max(below_width)), f32::max);
        let lines_height = line_sizes.iter().chain(more.iter()).map(|size| stack + size.y).sum::<f32>();
        let leaves_height = self.content_sizes.iter().map(|size| stack + size.y).sum::<f32>();
        let action_rows = if row && !inline_actions { wrap_actions(&actions, width, gap) } else { Vec::new() };
        let stacked = !row && column.is_finite() && actions_width > column;
        let actions_height = if row {
            action_rows.iter().map(|(_, height)| *height).sum::<f32>() + gap * action_rows.len().saturating_sub(1) as f32
        } else if stacked {
            actions.iter().map(|size| size.y).sum::<f32>() + gap * actions.len().saturating_sub(1) as f32
        } else {
            actions.iter().map(|size| size.y).fold(0.0, f32::max)
        };
        let column_width = if row { 0.0 } else { actions_width.max(content_width) };
        let text_width = title.map_or(0.0, |size| size.x).max(message.x).max(column_width);
        // The head line: the icon, the first line of the text and the dismiss button share its centre line.
        let first = title_layout.as_ref().or(message_layout.as_ref()).and_then(|layout| layout.galley.rows.first()).map_or(0.0, |row| row.row.size.y);
        let action_line = if inline_actions { actions.iter().map(|size| size.y).fold(0.0, f32::max) } else { 0.0 };
        let line = icon.unwrap_or(0.0).max(dismiss.map_or(0.0, |size| size.y)).max(first).max(action_line);
        let text_offset = (line - first) / 2.0;
        // The head includes the complete wrapped message; details and stacked actions follow it.
        let head = (text_offset + title.map_or(0.0, |size| size.y) + message.y).max(line);
        let with_actions =
            head + lines_height + leaves_height + if actions.is_empty() || inline_actions { 0.0 } else { stack.max(gap) + actions_height };
        let (needs, height) = if row {
            ((icon_width + text_width + dismiss_width + action_width).max(lines_width), with_actions)
        } else {
            let parts = [icon, title.map(|size| size.y), Some(message.y), (!actions.is_empty()).then_some(actions_height)];
            let shown: Vec<f32> = parts.into_iter().flatten().chain(self.content_sizes.iter().map(|size| size.y)).collect();
            (text_width.max(icon.unwrap_or(0.0)), shown.iter().sum::<f32>() + gap * shown.len().saturating_sub(1) as f32)
        };
        let lines = line_sizes.iter().map(|size| size.y).collect();
        Plan {
            icon,
            title,
            brand,
            message,
            bullet,
            lines,
            more: more.map(|size| size.y),
            actions,
            action_rows,
            inline_actions,
            dismiss,
            line,
            text_offset,
            stacked,
            needs,
            height,
        }
    }
}

impl Component for Notice<'_> {
    const ROLE: Role = Role::STATUS;
    const ALT_ROLES: &'static [Role] = &[Role::ALERT, Role::LABEL];
    const OVERLAYS: bool = false;
    type Style = NoticeStyle;
    type Output = NoticeOutput;

    fn style(cx: &StyleCx<'_>, key: NoticeStyleKey) -> NoticeStyle {
        let text = &cx.tokens.color.text;
        let disabled = cx.color(text.disabled);
        let stateful = |color: StyleColor| Stateful::new(color).off(States::new(color, color, color, disabled));
        let tone = cx.tone(key.tone);
        let (normal, muted) = (cx.color(text.normal), cx.color(text.muted));
        let control = cx.control();
        let faint = cx.color(text.faint);
        let (title, message, detail, surface, padding, stack) = match key.layout {
            NoticeLayout::Inline => (
                TextLook::new(TypeRole::BodyStrong, stateful(tone)),
                TextLook::new(TypeRole::Caption, stateful(tone)),
                TextLook::new(TypeRole::Caption, stateful(muted)),
                SurfaceLook::new(Stateful::new(cx.color(Rgba::TRANSPARENT))),
                Insets::uniform(0.0),
                0.0,
            ),
            NoticeLayout::Bar => {
                let colors = &cx.tokens.color;
                let ink = if key.tone == Tone::Neutral { normal } else { cx.color(key.tone.on_soft(cx.tokens)) };
                let fill = match key.tone {
                    Tone::Info => colors.status.info.bg,
                    Tone::Good => colors.status.good.bg,
                    Tone::Warn => colors.status.warn.bg,
                    Tone::Bad => colors.status.bad.bg,
                    Tone::Accent => colors.accent.soft,
                    _ => colors.surface.panel,
                };
                (
                    TextLook::new(TypeRole::BodyStrong, stateful(ink)),
                    TextLook::new(TypeRole::Body, stateful(ink)),
                    TextLook::new(TypeRole::Body, stateful(ink)),
                    SurfaceLook::new(Stateful::new(cx.color(fill))),
                    Insets::uniform(cx.tokens.space.pad.control),
                    cx.tokens.space.gap.stack,
                )
            }
            NoticeLayout::Card | NoticeLayout::Toast => {
                let toast = key.layout == NoticeLayout::Toast;
                let colors = &cx.tokens.color;
                let fill = if toast { cx.color(colors.surface.inverse) } else { StyleColor(cx.surface(SurfaceKind::Floating).fill) };
                let normal = if toast { cx.color(colors.text.on_inverse) } else { normal };
                let mut surface =
                    SurfaceLook::new(Stateful::new(fill)).kind(SurfaceKind::Floating).radius(UiTokens::corner(cx.tokens.shape.radius.surface));
                // A card of a status tone carries the tone on its edge and title; a neutral card keeps the surface's edge.
                let title = if key.tone == Tone::Neutral {
                    normal
                } else {
                    surface = surface.border(stateful(tone), cx.tokens.stroke.thin).border_replaces_edge();
                    // Tone text is tuned for the ordinary surfaces, not the inverse one.
                    if toast { normal } else { tone }
                };
                (
                    TextLook::new(TypeRole::BodyStrong, stateful(title)),
                    TextLook::new(TypeRole::Body, stateful(normal)),
                    TextLook::new(TypeRole::Body, stateful(normal)),
                    surface,
                    Insets::uniform(cx.tokens.space.pad.regular),
                    cx.tokens.space.gap.stack,
                )
            }
            NoticeLayout::Centred => (
                TextLook::new(TypeRole::Title, stateful(normal)),
                TextLook::new(TypeRole::Body, stateful(muted)),
                TextLook::new(TypeRole::Body, stateful(muted)),
                SurfaceLook::new(Stateful::new(cx.color(Rgba::TRANSPARENT))),
                Insets::horizontal(control.pad_x),
                0.0,
            ),
        };
        let bar_ink = if key.layout == NoticeLayout::Bar { Some(message.color.clone()) } else { None };
        let brand = TextLook::new(TypeRole::Display, bar_ink.clone().unwrap_or_else(|| stateful(normal)));
        let byline = TextLook::new(TypeRole::Label, bar_ink.clone().unwrap_or_else(|| stateful(faint)));
        let line = TextLook::new(message.role, bar_ink.clone().unwrap_or_else(|| stateful(muted)));
        let more = TextLook::new(message.role, bar_ink.unwrap_or_else(|| stateful(faint)));
        NoticeStyle {
            surface,
            metrics: Metrics::new().padding(padding).gap(control.gap),
            focus: cx.focus_look(),
            title,
            brand,
            byline,
            byline_lift: cx.tokens.space.gap.stack,
            stack,
            message,
            detail,
            line,
            more,
            icon: if key.layout == NoticeLayout::Centred {
                // An empty state's picture: large and quiet above its message.
                IconLook::new(cx.tokens.size.icon.lg * EMPTY_ICON_IN_LARGE, stateful(muted))
            } else {
                IconLook::new(control.icon, stateful(tone))
            },
            reading_width: cx.tokens.typography.get(TypeRole::Body).size * READING_IN_BODY_SIZES,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        self.measure_content(cx);
        let texts = self.texts(&cx.env, &cx.words::<NoticeWords>(), cx.format(), cx.resolved.label);
        let natural = self.plan(cx, &texts, f32::INFINITY).needs;
        let width = match (self.layout, cx.room) {
            // The card is as wide as its text, up to the card width (padding included) where the room allows. A card
            // that hosts content (a progress bar) has no text width of its own and takes the card width.
            (NoticeLayout::Card | NoticeLayout::Toast, room) => {
                let padding = cx.style.metrics.padding;
                let cap = CARD_WIDTH - padding.start - padding.end;
                let width = if self.content_sizes.is_empty() { natural.min(cap) } else { cap };
                room.map_or(width, |room| width.min(room))
            }
            (NoticeLayout::Centred, None) => natural.min(cx.style.reading_width),
            (_, Some(room)) => room,
            // A row notice offered no room (a unit of a row, a column sized by its content) takes its line, at most
            // the column's room: in a container that sets a line length, its paragraphs' wrap width.
            (_, None) => cx.column_room().map_or(natural, |column| natural.min(column)),
        };
        Vec2::new(width, self.plan(cx, &texts, self.wrap_width(cx, width)).height)
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn parts(&mut self, _rect: Rect, cx: &mut PartsCx<'_, Self>) {
        self.measure_content(cx);
        // The elements sit inside the padding: a card's content is inset from its surface's edge.
        let rect = cx.content_rect;
        let rtl = cx.flow.is_rtl();
        let gap = cx.gap();
        let texts = self.texts(&cx.env, &cx.words::<NoticeWords>(), cx.format(), cx.resolved.label);
        let rect = if self.layout == NoticeLayout::Centred {
            let column = Vec2::new(self.wrap_width(cx, rect.width()), rect.height());
            Flow::new(rect, rtl).align(column, Align::Center, Align::Min)
        } else {
            rect
        };
        let plan = self.plan(cx, &texts, rect.width());
        let mut placed = Placed { labels: self.action_labels(cx), ..Placed::default() };
        if self.row() {
            let stack = cx.style.stack;
            // Each item of the head line is centred on it.
            let on_line = |height: f32| rect.top() + (plan.line - height) / 2.0;
            let mut flow = Flow::new(rect, rtl);
            if let Some(side) = plan.icon {
                let (slot, rest) = flow.take_start(side);
                placed.icon = Some(Rect::from_min_size(Pos2::new(slot.min.x, on_line(side)), Vec2::splat(side)));
                flow = rest.take_start(gap).1;
            }
            if let Some(size) = plan.dismiss {
                let (slot, rest) = flow.take_end(size.x);
                placed.dismiss = Some(Rect::from_min_size(Pos2::new(slot.min.x, on_line(size.y)), size));
                flow = rest.take_end(gap).1;
            }
            if plan.inline_actions {
                for size in plan.actions.iter().rev() {
                    let (slot, rest) = flow.take_end(size.x);
                    placed.actions.push(Rect::from_min_size(Pos2::new(slot.min.x, on_line(size.y)), *size));
                    flow = rest.take_end(gap).1;
                }
                placed.actions.reverse();
            }
            let column = flow.rect();
            let mut y = column.top() + plan.text_offset;
            if let Some(size) = plan.title {
                placed.title = Some(Rect::from_min_size(Pos2::new(column.left(), y), Vec2::new(column.width(), size.y)));
                y += size.y;
            }
            if !texts.message.is_empty() {
                placed.message = Some(Rect::from_min_size(Pos2::new(column.left(), y), Vec2::new(column.width(), plan.message.y)));
                y += plan.message.y;
            }
            y = y.max(rect.top() + plan.line);
            for height in &plan.lines {
                y += stack;
                let band = Flow::new(Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), *height)), rtl);
                let (bullet, text) = band.take_start(plan.bullet);
                placed.lines.push((bullet, text.rect()));
                y += height;
            }
            if let Some(height) = plan.more {
                y += stack;
                placed.more = Some(Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), height)));
                y += height;
            }
            for size in &self.content_sizes {
                y += stack;
                placed.content.push(Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), size.y)));
                y += size.y;
            }
            if !plan.action_rows.is_empty() {
                place_action_rows(&plan, rect, y + stack.max(gap), rtl, gap, &mut placed.actions);
            }
        } else {
            let mut y = rect.top() + ((rect.height() - plan.height) / 2.0).max(0.0);
            let mut put = |size: Vec2| {
                let size = Vec2::new(size.x.min(rect.width()), size.y);
                let band = Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), size.y));
                y += size.y + gap;
                Flow::new(band, rtl).align(size, Align::Center, Align::Min)
            };
            placed.icon = plan.icon.map(|side| put(Vec2::splat(side)));
            placed.title = plan.title.map(&mut put);
            if let (Some(block), Some((name, byline, beside))) = (placed.title, plan.brand) {
                let (title, rest) = Flow::new(block, rtl).take_start(name.x.min(block.width()));
                if beside {
                    let byline_rect = rest.take_start(gap).1.rect();
                    let bottom = block.bottom() - cx.style.byline_lift;
                    placed.title = Some(Rect::from_min_size(title.min, name));
                    placed.byline =
                        Some(Rect::from_min_max(Pos2::new(byline_rect.left(), bottom - byline.y), Pos2::new(byline_rect.right(), bottom)));
                } else {
                    let band = |top: f32, size: Vec2| {
                        Flow::new(Rect::from_min_size(Pos2::new(block.left(), top), Vec2::new(block.width(), size.y)), rtl).align(
                            size,
                            Align::Center,
                            Align::Min,
                        )
                    };
                    placed.title = Some(band(block.top(), name));
                    placed.byline = Some(band(block.top() + name.y + gap, byline));
                }
            }
            placed.message = Some(put(plan.message));
            if plan.stacked {
                placed.actions = plan.actions.iter().map(|size| put(*size)).collect();
            } else if !plan.actions.is_empty() {
                let total = plan.actions.iter().map(|size| size.x).sum::<f32>() + gap * (plan.actions.len() - 1) as f32;
                let height = plan.actions.iter().map(|size| size.y).fold(0.0, f32::max);
                let mut line = Flow::new(put(Vec2::new(total, height)), rtl);
                for size in &plan.actions {
                    let (slot, rest) = line.take_start(size.x);
                    placed.actions.push(Rect::from_min_size(slot.min, *size));
                    line = rest.take_start(gap).1;
                }
            }
            placed.content = self.content_sizes.iter().map(|size| put(*size)).collect();
        }
        if self.layout == NoticeLayout::Centred
            && let Some(title) = placed.title
        {
            cx.part(TITLE, title).sense(Sense::HOVER);
        }
        if let Some(title) = texts.title.as_deref().filter(|_| placed.title.is_some()) {
            cx.selectable_text(TITLE_TEXT, &self.run(title, self.title_look(cx.style).role));
        }
        if let Some(byline) = texts.byline.as_deref().filter(|_| placed.byline.is_some()) {
            cx.selectable_text(BYLINE, &self.run(byline, cx.style.byline.role));
        }
        if placed.message.is_some() {
            let role = if self.title.is_some() { cx.style.detail.role } else { cx.style.message.role };
            cx.selectable_text(MESSAGE, &self.run(texts.message, role));
        }
        for (at, line) in texts.lines.iter().enumerate() {
            cx.selectable_text(LINE.at(at as u64), &self.run(line, cx.style.line.role));
        }
        if let Some(more) = &texts.more {
            cx.selectable_text(MORE, &self.run(more, cx.style.more.role));
        }
        let default = self.actions.iter().rposition(|action| action.enabled && action.default);
        for (at, (action, rect)) in self.actions.iter().zip(&placed.actions).enumerate() {
            let part = cx.part(ACTION.at(at as u64), *rect);
            if action.enabled {
                let part = part.sense(Sense::CLICK | Sense::FOCUS).focusable();
                if default == Some(at) {
                    part.default_stop();
                }
            }
        }
        if let Some(rect) = placed.dismiss {
            cx.part(DISMISS, rect).sense(Sense::CLICK | Sense::FOCUS).focusable();
        }
        if !self.footer_actions && (self.actions.iter().any(|action| action.enabled) || placed.dismiss.is_some()) {
            cx.roving(Roving::new(NavShape::Flat));
        }
        self.placed = Some(placed);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        for (at, action) in self.actions.iter().enumerate() {
            if action.enabled
                && let Some(source) = cx.part_activated(ACTION.at(at as u64))
            {
                self.taken = Some(at);
                if let Some(command) = &action.command {
                    cx.fire(command.clone(), source);
                }
            }
        }
        if cx.part_activated(DISMISS).is_some() {
            self.dismissed = true;
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        match (self.announce, self.tone) {
            (None, _) => {
                access.role(Role::LABEL);
            }
            (Some(politeness), Tone::Bad) => {
                access.role(Role::ALERT).live(politeness);
            }
            (Some(politeness), _) => {
                access.live(politeness);
            }
        }
        let texts = self.texts(&cx.env, &cx.words::<NoticeWords>(), cx.format(), cx.resolved.label);
        if !texts.lines.is_empty() || texts.message.is_empty() {
            let parts = self.title.iter().map(|title| cx.env.resolve(title).as_str().to_owned());
            let message = (!texts.message.is_empty()).then(|| texts.message.to_owned());
            access.name(Name::Joined(parts.chain(message).chain(texts.lines.clone()).chain(texts.more.clone()).collect()));
        } else if let Some(title) = self.title.as_ref().map(|title| cx.env.resolve(title).as_str().to_owned()) {
            if self.layout == NoticeLayout::Centred {
                access.name(Name::Text(cx.resolved.label.to_owned()));
                let node = access.node(TITLE, Role::HEADING);
                node.level(TITLE_LEVEL);
                node.name = Name::Text(title);
            } else {
                access.name(Name::Joined(vec![title, cx.resolved.label.to_owned()]));
            }
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(placed) = self.placed.clone() else { return };
        let words = cx.words::<NoticeWords>();
        for (at, (item, rect)) in std::mem::take(&mut self.content).into_iter().zip(&placed.content).enumerate() {
            item.show(ui, (CONTENT, at), *rect);
        }
        for (at, (action, label)) in self.actions.iter().zip(placed.labels.iter().cloned()).enumerate() {
            let part = ACTION.at(at as u64);
            if self.row() && self.layout != NoticeLayout::Bar {
                ui.face(part, self.link(cx.style.message.role, label).enabled(action.enabled));
            } else {
                ui.face(part, Self::button(at, label).enabled(action.enabled));
            }
        }
        if placed.dismiss.is_some() {
            // A toast's card is the inverse surface: its close glyph takes the card's text colour, not the panel ink.
            let mut icon = IconSpec::svg(IconId::new(DISMISS_ICON));
            if self.layout == NoticeLayout::Toast {
                icon = icon.color(cx.style.message.color.off.rest);
            }
            ui.face(DISMISS, IconButton::new(icon).label(words.dismiss.clone()));
        }
        if let (Some(rect), Some(mut spec)) = (placed.icon, self.icon_spec()) {
            if spec.color.is_none() {
                spec.color = Some(cx.resolve(&cx.style.icon.color));
            }
            ui.draw().icon(rect, &spec);
        }
        if let (Some(rect), Some(title)) = (placed.title, self.title.as_ref()) {
            let look = self.title_look(cx.style);
            let run = self.run(cx.env.resolve(title).as_str(), look.role);
            ui.selectable_text(TITLE_TEXT, rect, &run.color(cx.resolve(&look.color)));
        }
        if let (Some(rect), Some(byline)) = (placed.byline, self.brand.as_ref().filter(|_| !self.row())) {
            let look = &cx.style.byline;
            let run = self.run(cx.env.resolve(byline).as_str(), look.role);
            ui.selectable_text(BYLINE, rect, &run.color(cx.resolve(&look.color)));
        }
        let look = if self.title.is_some() { &cx.style.detail } else { &cx.style.message };
        if let Some(message) = placed.message {
            let run = self.run(cx.resolved.label, look.role);
            ui.selectable_text(MESSAGE, message, &run.color(cx.resolve(&look.color)));
        }
        let texts = self.texts(&cx.env, &words, cx.format(), cx.resolved.label);
        let (line, more) = (&cx.style.line, &cx.style.more);
        for (at, ((bullet, rect), text)) in placed.lines.iter().zip(&texts.lines).enumerate() {
            let ink = cx.resolve(&line.color);
            ui.draw().text(*bullet, &TextRun::new(BULLET).role(line.role).color(ink));
            ui.selectable_text(LINE.at(at as u64), *rect, &self.run(text, line.role).color(ink));
        }
        if let (Some(rect), Some(text)) = (placed.more, &texts.more) {
            ui.selectable_text(MORE, rect, &self.run(text, more.role).color(cx.resolve(&more.color)));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> NoticeOutput {
        NoticeOutput { response: cx.response.clone(), action: self.taken, dismissed: self.dismissed }
    }
}

/// Packs `actions` into rows at most `width` wide with `gap` between neighbours, keeping at least one action on every
/// row so a single wide action never loops; infinite `width` keeps all actions on one row. Returns each row's index
/// range and tallest height.
fn wrap_actions(actions: &[Vec2], width: f32, gap: f32) -> Vec<(std::ops::Range<usize>, f32)> {
    if actions.is_empty() {
        return Vec::new();
    }
    let mut rows = Vec::new();
    let (mut start, mut row_width, mut row_height) = (0_usize, 0.0_f32, 0.0_f32);
    for (at, size) in actions.iter().enumerate() {
        let next = if at > start { row_width + gap + size.x } else { size.x };
        if at > start && width.is_finite() && next > width {
            rows.push((start..at, row_height));
            start = at;
            row_width = size.x;
            row_height = size.y;
        } else {
            row_width = next;
            row_height = row_height.max(size.y);
        }
    }
    rows.push((start..actions.len(), row_height));
    rows
}

/// The icon that marks `tone`; the neutral tone has none. A list row's status cell marks its tone with it too.
pub(crate) fn tone_icon(tone: Tone) -> Option<&'static str> {
    match tone {
        Tone::Info | Tone::Accent => Some("info"),
        Tone::Good => Some("success"),
        Tone::Warn => Some("warning"),
        Tone::Bad => Some("error"),
        _ => None,
    }
}

/// Places a row notice's action rows across `rect` from `top` down, each action at its width and centred on its row.
fn place_action_rows(plan: &Plan, rect: Rect, top: f32, rtl: bool, gap: f32, actions: &mut Vec<Rect>) {
    let mut y = top;
    for (range, row_height) in &plan.action_rows {
        let mut line = Flow::new(Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), *row_height)), rtl);
        for size in plan.actions.get(range.clone()).unwrap_or_default() {
            let width = size.x.min(line.rect().width().max(0.0));
            let (slot, rest) = line.take_start(width);
            let top = y + (*row_height - size.y) / 2.0;
            actions.push(Rect::from_min_size(Pos2::new(slot.min.x, top), Vec2::new(width, size.y)));
            line = rest.take_start(gap).1;
        }
        y += row_height + gap;
    }
}
