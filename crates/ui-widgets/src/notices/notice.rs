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
use crate::style::{ComponentStyle, IconLook, Insets, StyleColor, StyleCx, SurfaceKind, SurfaceLook, TextLook, TypeRole, UiTokens};
use craft_behavior::{Flow, IconId, Intent, Metrics, NavShape, PartKey, Proposal, Roving, Sense};
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
    /// The inline layout on a floating surface, as wide as its text up to the card width: a toast.
    Card,
    /// Icon, title, message and actions in a column centred in the room, at most the reading width wide: an empty
    /// state. The first action is the primary button and the others secondary, all as wide as the widest, side by side
    /// or stacked when they do not fit.
    Centred,
}

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
    default: bool,
}

impl NoticeAction {
    /// An action named `label`; the notice reports it as `NoticeOutput.action`.
    pub fn new(label: impl Into<Label>) -> Self {
        NoticeAction { label: label.into(), command: None, default: false }
    }

    /// An action that shows `command`'s label and fires it when taken; the notice reports it as well.
    pub fn command(command: impl Into<CommandRef>) -> Self {
        NoticeAction { label: Label::Empty, command: Some(command.into()), default: false }
    }

    /// Makes this the notice's default action: a layer with `Initial::Default` focuses it when it opens, so Enter
    /// takes it. When several actions are marked, the last one is the default.
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
    /// The width a card wraps its text at when the room is open.
    card_width: f32,
    /// The widest column of the centred layout.
    reading_width: f32,
}

/// The widest column of a centred notice, in body text sizes: about 460 at the 12 point body, the empty states'
/// column in the source apps.
const READING_IN_BODY_SIZES: f32 = 38.5;

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
    /// The host's leaves under the actions of a centred notice, and their sizes in this pass.
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
    dismiss: Option<Vec2>,
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

    /// Hosts `component` under the actions of a centred notice, after the leaves added before it; its output goes to
    /// `output` in the pass it is shown. Other layouts show no hosted leaves.
    #[must_use]
    pub fn content<C: Show + 'a>(mut self, component: C, output: impl FnOnce(C::Output) + 'a) -> Self {
        self.content.push(Box::new(Hosting::new(component, output)));
        self
    }

    /// Measures the hosted leaves for this pass; only a centred notice shows them.
    fn measure_content(&mut self, cx: &MeasureCx<'_, Self>) {
        let centred = !self.row();
        self.content_sizes = if centred {
            self.content.iter_mut().enumerate().map(|(at, item)| item.measure(cx, (CONTENT, at), Proposal::Ideal)).collect()
        } else {
            Vec::new()
        };
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
                Some(command) => cx.resolve_command(command.clone()).map_or(Label::Empty, |resolved| Label::from(resolved.label)),
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

    /// The face of an inline or card notice's action.
    fn link(role: TypeRole, label: Label) -> Link {
        Link::new(label).role(role)
    }

    /// The sizes of the actions' faces; a centred notice's buttons all take the widest one's width.
    fn action_sizes(&self, cx: &MeasureCx<'_, Self>) -> Vec<Vec2> {
        let labels = self.action_labels(cx).into_iter().enumerate();
        if self.row() {
            return labels.map(|(_, label)| cx.face(Self::link(cx.style.message.role, label))).collect();
        }
        let sizes: Vec<Vec2> = labels.map(|(at, label)| cx.face(Self::button(at, label))).collect();
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
        let column = if width.is_finite() { (width - icon_width - dismiss_width).max(0.0) } else { f32::INFINITY };
        let run = |text: &str, role: TypeRole| cx.layout_at(&self.run(text, role), column).size();
        let title_role = self.title_look(cx.style).role;
        let mut title = texts.title.as_deref().map(|title| run(title, title_role));
        let brand = title.zip(texts.byline.as_deref()).map(|(name, byline)| {
            let byline = run(byline, cx.style.byline.role);
            let beside = name.x + gap + byline.x <= column;
            (name, byline, beside)
        });
        if let Some((name, byline, beside)) = brand {
            title = Some(if beside { Vec2::new(name.x + gap + byline.x, name.y) } else { Vec2::new(name.x.max(byline.x), name.y + gap + byline.y) });
        }
        let message = if texts.message.is_empty() { Vec2::ZERO } else { run(texts.message, cx.style.message.role) };
        let bullet = if texts.lines.is_empty() { 0.0 } else { cx.layout(&TextRun::new(BULLET).role(cx.style.line.role)).size().x + gap };
        let line_column = (column - bullet).max(0.0);
        let line_sizes: Vec<Vec2> = texts.lines.iter().map(|line| cx.layout_at(&self.run(line, cx.style.line.role), line_column).size()).collect();
        let more = texts.more.as_deref().map(|more| run(more, cx.style.more.role));
        let lines_width = line_sizes.iter().map(|size| size.x + bullet).fold(more.map_or(0.0, |size| size.x), f32::max);
        let lines_height = line_sizes.iter().map(|size| size.y).sum::<f32>() + more.map_or(0.0, |size| size.y);
        let actions_width = actions.iter().map(|size| size.x).sum::<f32>() + gap * actions.len().saturating_sub(1) as f32;
        let stacked = !row && column.is_finite() && actions_width > column;
        let actions_height = if stacked {
            actions.iter().map(|size| size.y).sum::<f32>() + gap * actions.len().saturating_sub(1) as f32
        } else {
            actions.iter().map(|size| size.y).fold(0.0, f32::max)
        };
        let content_width = self.content_sizes.iter().map(|size| size.x).fold(0.0, f32::max);
        let text_width = title.map_or(0.0, |size| size.x).max(message.x).max(actions_width).max(lines_width).max(content_width);
        let text_height = title.map_or(0.0, |size| size.y) + message.y + lines_height;
        let with_actions = text_height + if actions.is_empty() { 0.0 } else { gap + actions_height };
        let (needs, height) = if row {
            (icon_width + text_width + dismiss_width, with_actions.max(icon.unwrap_or(0.0)).max(dismiss.map_or(0.0, |size| size.y)))
        } else {
            let parts = [icon, title.map(|size| size.y), Some(message.y), (!actions.is_empty()).then_some(actions_height)];
            let shown: Vec<f32> = parts.into_iter().flatten().chain(self.content_sizes.iter().map(|size| size.y)).collect();
            (text_width.max(icon.unwrap_or(0.0)), shown.iter().sum::<f32>() + gap * shown.len().saturating_sub(1) as f32)
        };
        let lines = line_sizes.iter().map(|size| size.y).collect();
        Plan { icon, title, brand, message, bullet, lines, more: more.map(|size| size.y), actions, dismiss, stacked, needs, height }
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
        let (title, message, detail, surface, padding) = match key.layout {
            NoticeLayout::Inline => (
                TextLook::new(TypeRole::BodyStrong, stateful(tone)),
                TextLook::new(TypeRole::Caption, stateful(tone)),
                TextLook::new(TypeRole::Caption, stateful(muted)),
                SurfaceLook::new(Stateful::new(cx.color(Rgba::TRANSPARENT))),
                Insets::uniform(0.0),
            ),
            NoticeLayout::Card => {
                let fill = StyleColor(cx.surface(SurfaceKind::Floating).fill);
                (
                    TextLook::new(TypeRole::BodyStrong, stateful(normal)),
                    TextLook::new(TypeRole::Body, stateful(normal)),
                    TextLook::new(TypeRole::Body, stateful(normal)),
                    SurfaceLook::new(Stateful::new(fill)).kind(SurfaceKind::Floating).radius(UiTokens::corner(cx.tokens.shape.radius.surface)),
                    Insets::uniform(cx.tokens.space.pad.roomy),
                )
            }
            NoticeLayout::Centred => (
                TextLook::new(TypeRole::Title, stateful(normal)),
                TextLook::new(TypeRole::Body, stateful(muted)),
                TextLook::new(TypeRole::Body, stateful(muted)),
                SurfaceLook::new(Stateful::new(cx.color(Rgba::TRANSPARENT))),
                Insets::horizontal(control.pad_x),
            ),
        };
        let brand = TextLook::new(TypeRole::Display, stateful(normal));
        let byline = TextLook::new(TypeRole::Label, stateful(faint));
        let line = TextLook::new(message.role, stateful(muted));
        let more = TextLook::new(message.role, stateful(faint));
        NoticeStyle {
            surface,
            metrics: Metrics::new().padding(padding).gap(control.gap),
            title,
            brand,
            byline,
            byline_lift: cx.tokens.space.gap.stack,
            message,
            detail,
            line,
            more,
            icon: IconLook::new(control.icon, stateful(tone)),
            card_width: cx.tokens.size.length.field,
            reading_width: cx.tokens.typography.get(TypeRole::Body).size * READING_IN_BODY_SIZES,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        self.measure_content(cx);
        let texts = self.texts(&cx.env, &cx.words::<NoticeWords>(), cx.format(), cx.resolved.label);
        let natural = self.plan(cx, &texts, f32::INFINITY).needs;
        let width = match (self.layout, cx.room) {
            (NoticeLayout::Card, Some(room)) => natural.min(cx.style.card_width).min(room),
            (NoticeLayout::Card, None) => natural.min(cx.style.card_width),
            (NoticeLayout::Centred, None) => natural.min(cx.style.reading_width),
            (_, Some(room)) => room,
            (_, None) => natural,
        };
        Vec2::new(width, self.plan(cx, &texts, self.wrap_width(cx, width)).height)
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        self.measure_content(cx);
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
            let mut flow = Flow::new(rect, rtl);
            if let Some(side) = plan.icon {
                let (slot, rest) = flow.take_start(side);
                placed.icon = Some(Rect::from_min_size(slot.min, Vec2::splat(side)));
                flow = rest.take_start(gap).1;
            }
            if let Some(size) = plan.dismiss {
                let (slot, rest) = flow.take_end(size.x);
                placed.dismiss = Some(Rect::from_min_size(slot.min, size));
                flow = rest.take_end(gap).1;
            }
            let column = flow.rect();
            let mut y = column.top();
            if let Some(size) = plan.title {
                placed.title = Some(Rect::from_min_size(Pos2::new(column.left(), y), Vec2::new(column.width(), size.y)));
                y += size.y;
            }
            if !texts.message.is_empty() {
                placed.message = Some(Rect::from_min_size(Pos2::new(column.left(), y), Vec2::new(column.width(), plan.message.y)));
                y += plan.message.y;
            }
            for height in &plan.lines {
                let band = Flow::new(Rect::from_min_size(Pos2::new(column.left(), y), Vec2::new(column.width(), *height)), rtl);
                let (bullet, text) = band.take_start(plan.bullet);
                placed.lines.push((bullet, text.rect()));
                y += height;
            }
            if let Some(height) = plan.more {
                placed.more = Some(Rect::from_min_size(Pos2::new(column.left(), y), Vec2::new(column.width(), height)));
                y += height;
            }
            y += gap;
            let mut line = Flow::new(Rect::from_min_size(Pos2::new(column.left(), y), Vec2::new(column.width(), (rect.bottom() - y).max(0.0))), rtl);
            for size in &plan.actions {
                let (slot, rest) = line.take_start(size.x);
                placed.actions.push(Rect::from_min_size(slot.min, *size));
                line = rest.take_start(gap).1;
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
        let default = self.actions.iter().rposition(|action| action.default);
        for (at, rect) in placed.actions.iter().enumerate() {
            let part = cx.part(ACTION.at(at as u64), *rect).sense(Sense::CLICK | Sense::FOCUS).focusable();
            if default == Some(at) {
                part.default_stop();
            }
        }
        if let Some(rect) = placed.dismiss {
            cx.part(DISMISS, rect).sense(Sense::CLICK | Sense::FOCUS).focusable();
        }
        if !self.footer_actions && (!placed.actions.is_empty() || placed.dismiss.is_some()) {
            cx.roving(Roving::new(NavShape::Flat));
        }
        self.placed = Some(placed);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        for (at, action) in self.actions.iter().enumerate() {
            if let Some(source) = cx.part_activated(ACTION.at(at as u64)) {
                self.taken = Some(at);
                if let Some(command) = &action.command {
                    cx.fire(command.clone(), source);
                }
            }
        }
        if cx.part_activated(DISMISS).is_some() || (self.dismiss && cx.core.intents.take(Intent::Cancel)) {
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
        for (at, label) in placed.labels.iter().cloned().enumerate() {
            let part = ACTION.at(at as u64);
            if self.row() {
                ui.face(part, Self::link(cx.style.message.role, label));
            } else {
                ui.face(part, Self::button(at, label));
            }
        }
        if placed.dismiss.is_some() {
            ui.face(DISMISS, IconButton::new(IconId::new(DISMISS_ICON)).label(words.dismiss.clone()));
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

/// The icon that marks `tone`; the neutral tone has none.
fn tone_icon(tone: Tone) -> Option<&'static str> {
    match tone {
        Tone::Info | Tone::Accent => Some("info"),
        Tone::Good => Some("success"),
        Tone::Warn => Some("warning"),
        Tone::Bad => Some("error"),
        _ => None,
    }
}
