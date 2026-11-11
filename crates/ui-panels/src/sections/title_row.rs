//! `TitleRow` (16.4): the leaf that fills a container's header region. Its lead and actions are parts that wear
//! the faces of other components, so they have those components' look, content paint and AccessKit role.

use super::words::SectionWords;
use crate::buttons::IconButton;
use crate::choice::Checkbox;
use crate::contract::overlay::{Anchor, Trigger};
use crate::contract::{
    Access, Bind, BindMut, BodyCx, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, Outcome, PaintUi, PartsCx, Pointer as _, Props as _,
    Role, ShowContainer as _, Styled as _, WordSetCx as _,
};
use crate::controls::disclosure::Disclosure;
use crate::menu::Menu;
use crate::primitives::{IconSpec, Span, TextRun};
use crate::runtime::commands::CommandRef;
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, IconLook, Ink, StyleCx, TextLook, TypeRole};
use craft_behavior::{CursorIcon, Flow, IconId, Metrics, Mixed, NavShape, PartKey, Roving, Sense};
use craft_theme::{ControlSize, Stateful, States};
use egui::{Id, Rect, Response, Ui, Vec2};
use std::borrow::Cow;

#[cfg(feature = "testing")]
mod stories;

/// How much weight a header carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HeaderStrength {
    /// A control-height row with label text.
    Normal,
    /// A panel or dialog title.
    Strong,
    /// A section heading of a panel: emphasised body text.
    Section,
    /// A large heading: the heading role.
    Heading,
    /// A quiet heading: the caption role, faint, in capitals.
    Caption,
}

/// What a title row's action parts ask the container to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HeaderAction {
    /// Close the container: the container calls `cx.request_close()`.
    Close,
    /// Show the container's menu from its `overlay` hook.
    Menu,
    /// Float the container out of its dock.
    Float,
}

/// An icon command at the end of a header: an icon button bound to a command, so its accessible name, enabled state and
/// activation follow the command. The header's own actions (close, menu, float) come after the commands.
#[derive(Clone, Debug)]
pub struct HeaderCommand {
    icon: IconId,
    command: CommandRef,
    size: Option<ControlSize>,
}

impl HeaderCommand {
    /// The icon `icon` that runs `command`.
    pub fn new(icon: IconId, command: impl Into<CommandRef>) -> Self {
        HeaderCommand { icon, command: command.into(), size: None }
    }

    /// The control size of the button, in place of the row's.
    #[must_use]
    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = Some(size);
        self
    }
}

/// What `TitleRow` reports: the lead's value edits and the action taken.
#[derive(Debug)]
#[non_exhaustive]
pub struct TitleOutput {
    /// The lead's edits and the row's response.
    pub edit: EditResponse<bool>,
    /// The action part taken this pass.
    pub action: Option<HeaderAction>,
    /// The rect of the `HeaderAction::Menu` action part, in the row's layer space, for a container that hangs its
    /// menu under the button. `None` when the row has no menu action.
    pub menu_rect: Option<Rect>,
}

type MenuRowsFn<'a> = Box<dyn FnMut(&mut Ui, &mut BodyCx<'_>) + 'a>;

/// The rows of the menu a container hangs from its header row.
#[doc(hidden)]
pub struct MenuRows<'a>(MenuRowsFn<'a>);

impl<'a> MenuRows<'a> {
    #[doc(hidden)]
    pub fn new(rows: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'a) -> Self {
        MenuRows(Box::new(rows))
    }

    /// Hangs the menu `name` from the header row `shown`. It opens from the row's Menu action, under that action's
    /// part, and from the context request (right click, Menu key, Shift+F10). Returns the row's id when the Menu action
    /// was taken this pass: the container toggles the popup of that opener in its `finish`.
    #[doc(hidden)]
    pub fn hang(&mut self, ui: &mut Ui, shown: &TitleOutput, name: impl Into<Label>) -> Option<Id> {
        let mut menu = Menu::new(name).opened_by(&shown.edit.response, Trigger::Manual);
        if let Some(button) = shown.menu_rect {
            menu = menu.anchor(Anchor::Rect(button));
        }
        menu.opened_by(&shown.edit.response, Trigger::Context).show(ui, |ui, menu| (self.0)(ui, menu));
        (shown.action == Some(HeaderAction::Menu)).then_some(shown.edit.response.id)
    }
}

impl Outcome for TitleOutput {
    type Value = bool;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.edit.story_facts()
    }
}

enum Lead<'a> {
    None,
    Disclosure(Bind<'a, bool>),
    Check(Bind<'a, bool>),
}

const LEAD: PartKey = PartKey::new("lead");
const ACTION: PartKey = PartKey::new("action");
const COMMAND: PartKey = PartKey::new("command");
/// The title band: a click on it toggles a disclosure lead, and it carries the tooltip of an elided title.
const TITLE: PartKey = PartKey::new("title");

/// The style of a title row.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TitleRowStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The title's type role and colour.
    #[style(text)]
    text: TextLook,
    /// The icon before the title.
    #[style(icon)]
    icon: IconLook,
    /// The width of a disclosure lead's slot: `size.icon.sm`, a small glyph beside the title.
    disclosure: f32,
    /// The side of an action part: the control height, which is below a tall header's height.
    action: f32,
}

/// An optional icon, a title with an optional subtitle, a lead (a disclosure bound to the container's open state, or
/// a check box bound to its gate) and action parts. A row too narrow for the title and subtitle together drops the
/// subtitle before it elides the title; an elided title shows whole in a tooltip. A click on the title band toggles a
/// disclosure lead, as a click on the lead does.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct TitleRow<'a> {
    #[common]
    common: Common,
    /// The header's weight.
    #[style_key]
    #[prop]
    strength: HeaderStrength,
    /// The heading level AccessKit reports.
    #[prop]
    level: usize,
    /// Text after the title.
    #[prop(into)]
    subtitle: Option<Label>,
    /// An icon before the title, painted in the style's icon colour unless the spec names another.
    #[prop]
    icon: Option<IconSpec<'a>>,
    /// Whether the row takes the whole width offered, so its actions sit at the end edge; off, it is as wide as its
    /// parts.
    #[prop]
    fill: bool,
    /// The accessible name of the disclosure lead, for a row whose title is empty (a column bar names its toggle by
    /// the action it does). It defaults to the title.
    #[prop(into)]
    lead_name: Option<Label>,
    /// Whether the title band is the grab handle of a reorderable container: it shows the grab cursor and, under
    /// the pointer, a handle mark at its end.
    #[prop]
    handle: bool,
    lead: Lead<'a>,
    /// Whether the gate's value is mixed: the check box lead shows the mixed face.
    #[prop]
    gate_mixed: bool,
    actions: Vec<HeaderAction>,
    commands: Vec<HeaderCommand>,
    taken: Option<HeaderAction>,
    menu_rect: Option<Rect>,
    placed: Option<Placed>,
}

/// Where the icon, title and subtitle sit in the row, decided with the final width in `parts`.
#[derive(Clone, Copy)]
struct Placed {
    icon: Option<Rect>,
    handle: Option<Rect>,
    title: Rect,
    subtitle: Option<Rect>,
}

impl<'a> TitleRow<'a> {
    /// A title row for `title`.
    pub fn new(title: impl Into<Label>) -> Self {
        let common = Common::new();
        TitleRow {
            common,
            strength: HeaderStrength::Normal,
            level: 2,
            subtitle: None,
            icon: None,
            fill: true,
            lead_name: None,
            handle: false,
            lead: Lead::None,
            gate_mixed: false,
            actions: Vec::new(),
            commands: Vec::new(),
            taken: None,
            menu_rect: None,
            placed: None,
        }
        .label(title)
    }

    /// A check box lead bound to the container's gate.
    #[must_use]
    pub fn gate(mut self, gate: Bind<'a, bool>) -> Self {
        self.lead = Lead::Check(gate);
        self
    }

    /// A disclosure lead bound to the container's open state.
    #[must_use]
    pub fn disclosure(mut self, open: Bind<'a, bool>) -> Self {
        self.lead = Lead::Disclosure(open);
        self
    }

    /// The action parts at the end of the row, in reading order.
    #[must_use]
    pub fn actions(mut self, actions: impl IntoIterator<Item = HeaderAction>) -> Self {
        self.actions = actions.into_iter().collect();
        self
    }

    /// Icon commands before the action parts, in reading order.
    #[must_use]
    pub fn commands(mut self, commands: impl IntoIterator<Item = HeaderCommand>) -> Self {
        self.commands = commands.into_iter().collect();
        self
    }

    fn has_lead(&self) -> bool {
        !matches!(self.lead, Lead::None)
    }

    /// The width of the lead's slot: a disclosure's glyph takes the icon side, a check the row's height.
    fn lead_side(&self, row: f32, icon: f32) -> f32 {
        if matches!(self.lead, Lead::Disclosure(_)) { icon } else { row }
    }

    /// The title as it shows: a caption reads in capitals.
    fn title_text<'t>(&self, label: &'t str) -> Cow<'t, str> {
        if self.strength == HeaderStrength::Caption { Cow::Owned(label.to_uppercase()) } else { Cow::Borrowed(label) }
    }

    /// Divides the room left between the lead and the actions: the icon, then the title, then the subtitle when
    /// both texts fit whole.
    fn place(&self, mut band: Flow, cx: &PartsCx<'_, Self>) -> Placed {
        // A disclosure's glyph sits a gap before the title, as a title's own icon does; the gap is part of the title band,
        // so a click beside the glyph toggles too.
        if matches!(self.lead, Lead::Disclosure(_)) {
            band = band.take_start(cx.gap()).1;
        }
        let side = band.rect().height();
        let mut handle = None;
        if self.handle {
            let (slot, rest) = band.take_end(side);
            let (_, rest) = rest.take_end(cx.gap());
            handle = Some(slot);
            band = rest;
        }
        let mut icon = None;
        if self.icon.is_some() {
            let (slot, rest) = band.take_start(side);
            let (_, rest) = rest.take_start(cx.gap());
            icon = Some(slot);
            band = rest;
        }
        let role = cx.style.text.role;
        let subtitle = self.subtitle.as_ref().and_then(|subtitle| {
            let title = cx.layout_at(&TextRun::new(self.title_text(cx.resolved.label)).role(role), f32::INFINITY).galley.size().x;
            let width = cx.layout_at(&subtitle_run(subtitle.as_str(), role), f32::INFINITY).galley.size().x;
            (title + cx.gap() + width <= band.rect().width()).then_some(title)
        });
        match subtitle {
            Some(title) => {
                let (title, rest) = band.take_start(title);
                let (_, rest) = rest.take_start(cx.gap());
                Placed { icon, handle, title, subtitle: Some(rest.rect()) }
            }
            None => Placed { icon, handle, title: band.rect(), subtitle: None },
        }
    }
}

impl Component for TitleRow<'_> {
    const ROLE: Role = Role::HEADING;
    type Style = TitleRowStyle;
    type Output = TitleOutput;

    fn style(cx: &StyleCx<'_>, key: TitleRowStyleKey) -> TitleRowStyle {
        let text = &cx.tokens.color.text;
        let (height, role, shade) = match key.strength {
            HeaderStrength::Normal => (cx.control().height, TypeRole::Label, text.normal),
            HeaderStrength::Strong => (cx.tokens.size.panel_header, TypeRole::Title, text.normal),
            HeaderStrength::Section => (cx.tokens.size.panel_header, TypeRole::BodyStrong, text.normal),
            HeaderStrength::Heading => (cx.tokens.size.panel_header, TypeRole::Heading, text.normal),
            HeaderStrength::Caption => (cx.control().height, TypeRole::Caption, text.faint),
        };
        let (shade, ink, disabled) = (cx.color(shade), cx.ink(Ink::OnPanel), cx.color(text.disabled));
        TitleRowStyle {
            metrics: Metrics::new().min_size(Vec2::new(0.0, height)).gap(cx.control().gap),
            text: TextLook::new(role, Stateful::new(shade).off(States::new(shade, shade, shade, disabled))),
            icon: IconLook::new(cx.control().icon, Stateful::new(ink).off(States::new(ink, ink, ink, disabled))),
            disclosure: cx.tokens.size.icon.sm,
            action: cx.control().height,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let role = cx.style.text.role;
        let title = cx.layout(&TextRun::new(self.title_text(cx.resolved.label)).role(role)).galley.size();
        let subtitle = self.subtitle.as_ref().map(|subtitle| cx.layout(&subtitle_run(subtitle.as_str(), role)).galley.size().x);
        let parts =
            usize::from(self.has_lead()) + usize::from(self.icon.is_some()) + usize::from(self.handle) + self.commands.len() + self.actions.len();
        // The parts are square on the row's height, which the height token can raise above the text's.
        let side = title.y.max(cx.style.metrics.min_size.y);
        let text = title.x + subtitle.map_or(0.0, |width| width + cx.gap());
        let lead = if self.has_lead() { self.lead_side(side, cx.style.disclosure) - side } else { 0.0 };
        let natural = text + side * parts as f32 + cx.gap() * parts as f32 + lead;
        // A row takes the room it is offered, a filling row all of it, and never more: the title elides to what is left.
        let width = cx.room.map_or(natural, |room| if self.fill { room } else { natural.min(room) });
        Vec2::new(width, title.y)
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    /// Lead and actions are focusable parts of one roving group.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let side = rect.height();
        let mut flow = Flow::new(rect, cx.flow.is_rtl());
        if self.has_lead() {
            let (lead, rest) = flow.take_start(self.lead_side(side, cx.style.disclosure));
            cx.part(LEAD, lead).sense(Sense::CLICK | Sense::FOCUS).focusable();
            flow = rest;
        }
        // The last part sits at the end edge, so the group reads commands then actions in the order given; stops register
        // in that order.
        let commands = self.commands.len();
        let mut rects = Vec::new();
        for _ in 0..commands + self.actions.len() {
            let (part, rest) = flow.take_end(cx.style.action.min(side));
            rects.push(part);
            flow = rest;
        }
        self.menu_rect = None;
        for (at, rect) in rects.into_iter().rev().enumerate() {
            let key = if at < commands {
                COMMAND.at(at as u64)
            } else {
                let action = at - commands;
                if self.actions[action] == HeaderAction::Menu {
                    self.menu_rect = Some(rect);
                }
                ACTION.at(action as u64)
            };
            cx.part(key, rect).sense(Sense::CLICK | Sense::FOCUS).focusable();
        }
        cx.roving(Roving::new(NavShape::Flat));
        let band = flow.rect();
        let placed = self.place(flow, cx);
        let title = TextRun::new(self.title_text(cx.resolved.label)).role(cx.style.text.role).elide();
        let elided = cx.layout_at(&title, placed.title.width()).elided;
        let tooltip = cx.resolved.label.to_owned();
        let part = if matches!(self.lead, Lead::Disclosure(_)) {
            cx.part(TITLE, band).sense(Sense::CLICK).keyboard_via(LEAD)
        } else {
            cx.part(TITLE, band).sense(Sense::HOVER)
        };
        let part = if self.handle { part.cursor(CursorIcon::Grab).drag_cursor(CursorIcon::Grabbing) } else { part };
        if elided {
            part.tooltip(tooltip);
        }
        self.placed = Some(placed);
    }

    fn bind(&mut self) -> Option<BindMut<'_, bool>> {
        match &mut self.lead {
            Lead::None => None,
            Lead::Disclosure(bind) | Lead::Check(bind) => Some(bind.reborrow()),
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let row_toggles = matches!(self.lead, Lead::Disclosure(_)) && cx.part_activated(TITLE).is_some();
        if self.has_lead() && (cx.part_activated(LEAD).is_some() || row_toggles) {
            let now = cx.value().copied().unwrap_or(false);
            cx.set(!now);
        }
        for (at, action) in self.actions.iter().enumerate() {
            if cx.part_activated(ACTION.at(at as u64)).is_some() {
                self.taken = Some(*action);
            }
        }
        for (at, header) in self.commands.iter().enumerate() {
            if let Some(source) = cx.part_activated(COMMAND.at(at as u64)) {
                cx.fire(header.command.clone(), source);
            }
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let words = cx.words::<SectionWords>();
        let value = cx.value().copied().unwrap_or(false);
        match &self.lead {
            Lead::None => {}
            Lead::Disclosure(_) => {
                let name = self.lead_name.as_ref().map_or_else(|| cx.resolved.label.to_owned(), |name| name.as_str().to_owned());
                ui.face(LEAD, Disclosure::new(Bind::given(value)).node(false).label(name));
            }
            Lead::Check(_) => {
                let shown = if self.gate_mixed { Mixed::Mixed } else { Mixed::Same(value) };
                ui.face(LEAD, Checkbox::new(Bind::given(shown)).label(words.gate.clone()));
            }
        }
        for (at, header) in self.commands.iter().enumerate() {
            let button = IconButton::command(header.icon.clone(), header.command.clone());
            ui.face(COMMAND.at(at as u64), if let Some(size) = header.size { button.size(size) } else { button });
        }
        for (at, action) in self.actions.iter().enumerate() {
            let (icon, label) = match action {
                HeaderAction::Close => ("close", words.close.clone()),
                HeaderAction::Menu => ("more", words.menu.clone()),
                HeaderAction::Float => ("float", words.float.clone()),
            };
            ui.face(ACTION.at(at as u64), IconButton::new(IconId::new(icon)).size(ControlSize::Sm).label(label));
        }
        let Some(placed) = self.placed else { return };
        let role = cx.style.text.role;
        if let Some(rect) = placed.handle
            && cx.part(TITLE).hovered
        {
            let mut mark = IconSpec::svg(IconId::new("move"));
            mark.color = Some(cx.resolve(&cx.style.icon.color));
            ui.draw().icon(rect, &mark);
        }
        if let (Some(rect), Some(icon)) = (placed.icon, &mut self.icon) {
            if icon.color.is_none() {
                icon.color = Some(cx.resolve(&cx.style.icon.color));
            }
            ui.draw().icon(rect, icon);
        }
        let mut title = TextRun::new(self.title_text(cx.resolved.label)).role(role).elide();
        if self.strength == HeaderStrength::Caption {
            title = title.color(cx.resolve(&cx.style.text.color));
        }
        ui.draw().text_on_centre_line(placed.title, &title);
        if let (Some(rect), Some(subtitle)) = (placed.subtitle, &self.subtitle) {
            ui.draw().text_on_centre_line(rect, &subtitle_run(subtitle.as_str(), role));
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.level(self.level);
    }

    fn finish(self, cx: Cx<'_, Self>) -> TitleOutput {
        TitleOutput { edit: cx.edit_response(), action: self.taken, menu_rect: self.menu_rect }
    }
}

fn subtitle_run(subtitle: &str, role: TypeRole) -> TextRun<'_> {
    TextRun::spans(vec![Span::new(subtitle).muted()]).role(role)
}
