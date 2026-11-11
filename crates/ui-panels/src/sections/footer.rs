//! `FooterBar` (16.4): the container that fills a container's footer region.

use super::words::SectionWords;
use crate::buttons::{Button, IconButton};
use crate::contract::{
    Access, BodyLayout, Common, Component, Container, ContainerProps, Cx, DropMark, DropOperation, DropTarget, NoValue, Outcome, Pad, Pointer as _,
    PopupKind, Props as _, Role, RowAlign, Show as _, ShowContainer as _, Sizing, TooltipSpec, WordSetCx as _,
};
use crate::primitives::IconSpec;
use crate::runtime::access::Name;
use crate::runtime::env::{Env, EnvValue, Label};
use crate::runtime::focus;
use crate::runtime::keys;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{Metrics, NavShape, OwnerClass, Roving, Sense};
use craft_host::target::Os;
use craft_keymap::{Chord, Key};
use craft_theme::{
    Variant,
    variant::{Emphasis, Tone},
};
use egui::{Context, Pos2, Rect, Response, Ui};
use std::any::Any;
use std::fmt;
use std::sync::Arc;

#[cfg(feature = "testing")]
mod stories;

/// The style of a footer bar: no look of its own.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FooterBarStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// A footer region's content: its body is the caller's closure, and its members are one roving group.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub struct FooterBar {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    content: Content,
    /// Whether an Enter that the focused part left untaken, pressed with the focus inside the open layer, activates the
    /// default button (the first enabled `Accept` one) like a click on it.
    #[prop(name = "enter_activates")]
    enter: bool,
}

/// The payload a dropped-on action reports, type-erased so actions of different payload types share one footer.
type Dropped = Arc<dyn Any + Send + Sync>;

/// What showing an action's drop slot reports: the button's response, the payload dropped on it and whether an
/// accepted drag is over it.
struct DropShown {
    response: Response,
    dropped: Option<Dropped>,
    over: bool,
}

/// Shows an action's drop slot around its button.
type DropShow = dyn Fn(&mut Ui, &FooterAction) -> DropShown + Send + Sync;

/// An icon action of a panel footer: an `IconButton` named by its label.
#[derive(Clone)]
pub struct FooterAction {
    icon: IconSpec<'static>,
    label: Label,
    enabled: bool,
    pressed: bool,
    tone: Option<Tone>,
    popup: Option<PopupKind>,
    expanded: Option<bool>,
    takes_drops_disabled: bool,
    drop_hint: Option<Label>,
    drops: Option<Arc<DropShow>>,
}

impl fmt::Debug for FooterAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FooterAction")
            .field("icon", &self.icon)
            .field("label", &self.label)
            .field("enabled", &self.enabled)
            .field("pressed", &self.pressed)
            .field("tone", &self.tone)
            .field("popup", &self.popup)
            .field("expanded", &self.expanded)
            .field("takes_drops_disabled", &self.takes_drops_disabled)
            .field("drop_hint", &self.drop_hint)
            .field("accepts_drops", &self.drops.is_some())
            .finish()
    }
}

impl FooterAction {
    /// An action showing `icon`, named `label`.
    pub fn new(icon: impl Into<IconSpec<'static>>, label: impl Into<Label>) -> Self {
        FooterAction {
            icon: icon.into(),
            label: label.into(),
            enabled: true,
            pressed: false,
            tone: None,
            popup: None,
            expanded: None,
            takes_drops_disabled: false,
            drop_hint: None,
            drops: None,
        }
    }

    /// Whether a click or the keyboard can activate the action. A disabled action takes no drop unless
    /// [`takes_drops_disabled`](FooterAction::takes_drops_disabled) says so.
    #[must_use]
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    /// Whether the action shows the host's state, as the on look: Button Mode while the panel is in it, Record
    /// while recording. A click is reported as usual; the host changes its state or ignores the click.
    #[must_use]
    pub fn pressed(mut self, on: bool) -> Self {
        self.pressed = on;
        self
    }

    /// Tints the icon in the tone's status colour for an alert state (Record while recording is `Bad`).
    #[must_use]
    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = Some(tone);
        self
    }

    /// The kind of popup the action opens: the action reports it, and a mark shows at its end-bottom corner.
    #[must_use]
    pub fn popup(mut self, kind: PopupKind) -> Self {
        self.popup = Some(kind);
        self
    }

    /// Whether the popup the action opens is open, as the host knows it.
    #[must_use]
    pub fn expanded(mut self, open: bool) -> Self {
        self.expanded = Some(open);
        self
    }

    /// Whether a disabled action takes drops: Delete with nothing selected still deletes a dragged row. The action
    /// shows and reports enabled while an accepted drag is over it.
    #[must_use]
    pub fn takes_drops_disabled(mut self, on: bool) -> Self {
        self.takes_drops_disabled = on;
        self
    }

    /// The hint [`FooterOutput::drop_hint`] reports while an accepted drag is over the action.
    #[must_use]
    pub fn drop_hint(mut self, hint: impl Into<Label>) -> Self {
        self.drop_hint = Some(hint.into());
        self
    }

    /// Lets drags carrying a `P` that `accepts` takes drop on the action. The action shows the drop outline while
    /// such a drag is over it, and the drop comes back in [`FooterOutput::dropped`]. `accepts` may capture: a caller
    /// checks that the drag comes from its own list.
    #[must_use]
    pub fn accepts_drops<P: Clone + Send + Sync + 'static>(mut self, accepts: impl Fn(&P) -> bool + Send + Sync + 'static) -> Self {
        let accepts: Arc<dyn Fn(&P) -> bool + Send + Sync> = Arc::new(accepts);
        self.drops = Some(Arc::new(move |ui, action| {
            let accepts = Arc::clone(&accepts);
            let slot = DropAction::new(move |payload: &P| accepts(payload)).enabled(action.enabled || action.takes_drops_disabled);
            let shown = slot.show(ui, |ui, _| action.icon_button(DropAction::<P>::accepting(ui)).show(ui).response);
            let response = shown.inner.unwrap_or_else(|| shown.output.response.clone());
            DropShown { response, dropped: shown.output.dropped.map(|payload| Arc::new(payload) as Dropped), over: shown.output.over }
        }));
        self
    }

    /// The button, enabled when the action is or when `accepting` lifts it for an accepted drag over it.
    fn icon_button(&self, accepting: bool) -> IconButton<'static> {
        let mut button = IconButton::new(self.icon.clone()).label(self.label.clone()).enabled(self.enabled || accepting).selected(self.pressed);
        if let Some(tone) = self.tone {
            button = button.variant(Variant::look(Emphasis::Ghost, tone));
        }
        if let Some(kind) = self.popup {
            button = button.popup(kind);
        }
        if let Some(open) = self.expanded {
            button = button.expanded(open);
        }
        button
    }
}

/// A drop slot around one footer action: it takes the drags its predicate accepts over the action's rect.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct DropActionStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What a drop slot reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct DropActionOutput<P> {
    /// The slot's response.
    pub response: Response,
    /// The payload dropped on the slot this pass.
    pub dropped: Option<P>,
    /// Whether a drag the slot accepts is over it.
    pub over: bool,
}

impl<P> Outcome for DropActionOutput<P> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Whether an accepted drag is over the enclosing drop slot, which the slot's body reads to show its child enabled.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Accepting(bool);

impl EnvValue for Accepting {
    fn root() -> Self {
        Accepting(false)
    }
}

/// A container that is as large as its one child and takes drops of payload type `P` over that child. A slot that is
/// not enabled takes nothing. A footer action that takes drops while disabled enables its slot and leaves its own
/// button disabled, so a disabled parent still disables both.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container, probe(DropAction<u8>))]
pub struct DropAction<P: Clone + Send + Sync + 'static> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    accepts: Arc<dyn Fn(&P) -> bool + Send + Sync>,
    over: bool,
    dropped: Option<P>,
}

impl<P: Clone + Send + Sync + 'static> DropAction<P> {
    /// A slot that takes the payloads `accepts` takes.
    pub fn new(accepts: impl Fn(&P) -> bool + Send + Sync + 'static) -> Self {
        let props = ContainerProps::new().pad(Pad::None).sizing(Sizing::Content);
        DropAction { common: Common::new(), props, accepts: Arc::new(accepts), over: false, dropped: None }
    }

    /// Whether a drag the enclosing slot accepts is over it: a body shows its child enabled then, as the slot takes
    /// the drop whatever the child's own state.
    pub fn accepting(ui: &Ui) -> bool {
        Env::of(ui).get::<Accepting>().0
    }
}

/// Where a drop on the slot lands: the slot itself.
struct OnSlot<'a, P> {
    rect: Rect,
    live: bool,
    accepts: &'a (dyn Fn(&P) -> bool + Send + Sync),
}

impl<P: Clone + Send + Sync + 'static> DropTarget<P> for OnSlot<'_, P> {
    type Drop = P;

    fn accept(&self, payload: &P, _: Pos2, _: DropOperation) -> Option<P> {
        (self.live && (self.accepts)(payload)).then(|| payload.clone())
    }

    fn mark(&self, _: &P) -> DropMark {
        DropMark::Into(self.rect)
    }
}

impl<P: Clone + Send + Sync + 'static> Component for DropAction<P> {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = DropActionStyle;
    type Output = DropActionOutput<P>;

    fn style(_: &StyleCx<'_>, _: DropActionStyleKey<P>) -> DropActionStyle {
        DropActionStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let live = cx.resolved.enabled;
        // Decided before the child shows, so its look follows the drag in the same pass.
        self.over = live && cx.dragging::<P>().is_some_and(|(payload, at)| cx.rect.contains(at) && (self.accepts)(&payload));
        let target = OnSlot { rect: cx.rect, live, accepts: &*self.accepts };
        self.dropped = cx.drop_target(&target);
    }

    fn content_env(&self, _: &Cx<'_, Self>, values: &mut crate::runtime::env::AppValues) {
        values.set(Accepting(self.over));
    }

    fn finish(self, cx: Cx<'_, Self>) -> DropActionOutput<P> {
        DropActionOutput { response: cx.response.clone(), dropped: self.dropped, over: self.over }
    }
}

impl<P: Clone + Send + Sync + 'static> Container for DropAction<P> {}

/// What a footer reports: the action or button clicked, the drop an action took, and each action's response.
#[derive(Debug)]
#[non_exhaustive]
pub struct FooterOutput {
    /// The footer's response.
    pub response: Response,
    /// The index, in the order given, of the action or button activated this pass.
    pub picked: Option<usize>,
    /// The drop an action took this pass.
    pub dropped: Option<FooterDrop>,
    /// The hint of the action an accepted drag is over, when it has one.
    pub drop_hint: Option<Label>,
    /// The response of each action, in the order given; a caller opens a menu from one with `Menu::opened_by`.
    pub responses: Vec<Response>,
    /// The modifiers held at the pick; none when nothing was picked.
    pub mods: craft_keymap::Mods,
}

impl Outcome for FooterOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A payload dropped on a footer action.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FooterDrop {
    /// The index of the action it was dropped on, in the order given.
    pub index: usize,
    payload: Dropped,
}

impl FooterDrop {
    /// The payload, when it is a `P`.
    pub fn payload<P: 'static>(&self) -> Option<&P> {
        self.payload.downcast_ref::<P>()
    }
}

/// What a dialog button means; it decides the button's look and its place in the row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DialogRole {
    /// The default outcome: the primary look, last in the row (first on Windows).
    Accept,
    /// An outcome that destroys something: the danger look.
    Destructive,
    /// Leaves without an outcome.
    Cancel,
    /// Any other outcome.
    Other,
}

impl DialogRole {
    /// The position in the row: later roles stand nearer the end. Windows puts the accepting button first and the
    /// others after it; macOS and Linux put it last.
    fn rank(self, windows: bool) -> u8 {
        let rank = match self {
            DialogRole::Other => 0,
            DialogRole::Destructive => 1,
            DialogRole::Cancel => 2,
            DialogRole::Accept => 3,
        };
        if windows { 3 - rank } else { rank }
    }
}

/// A button of a dialog footer.
#[derive(Clone, Debug)]
pub struct FooterButton {
    label: Label,
    role: DialogRole,
    letter: Option<char>,
    enabled: bool,
    disabled_reason: Option<Label>,
    tooltip: Option<Label>,
    stays_open: bool,
}

impl FooterButton {
    /// The meaning of the button.
    pub const fn role(&self) -> DialogRole {
        self.role
    }

    /// The button's label, resolved by the environment that shows it.
    pub const fn label(&self) -> &Label {
        &self.label
    }

    /// A button named `label` with the meaning `role`.
    pub fn new(label: impl Into<Label>, role: DialogRole) -> Self {
        FooterButton { label: label.into(), role, letter: None, enabled: true, disabled_reason: None, tooltip: None, stays_open: false }
    }

    /// The letter that activates the button while its dialog holds focus and no text part owns the keys. The
    /// button reports it as its `access_key`.
    #[must_use]
    pub const fn letter(mut self, letter: char) -> Self {
        self.letter = Some(letter);
        self
    }

    /// Whether the button can be activated; a disabled button ignores its letter too.
    #[must_use]
    pub const fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    /// What the button tells while it is enabled: its tooltip. While it is disabled, its reason shows instead.
    #[must_use]
    pub fn tooltip(mut self, tooltip: impl Into<Label>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Why the button is disabled: the tooltip it shows while it is, over `tooltip`.
    #[must_use]
    pub fn disabled_reason(mut self, reason: impl Into<Label>) -> Self {
        self.disabled_reason = Some(reason.into());
        self
    }

    /// Whether a click leaves the dialog open (an Apply): the pick is reported and nothing closes. `Cancel` always
    /// closes.
    #[must_use]
    pub const fn stays_open(mut self, on: bool) -> Self {
        self.stays_open = on;
        self
    }

    /// Whether a click on the button closes its dialog.
    pub(crate) fn closes(&self) -> bool {
        self.role == DialogRole::Cancel || !self.stays_open
    }

    /// Sets whether the button can be activated, for a rule that runs while the footer is shown
    /// (`Dialog::settle_buttons`).
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
    }

    /// Whether the button can be activated.
    pub(crate) const fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Whether a bare letter reaches the footer being shown: focus is inside the focus scope the footer sits in (the
/// dialog's, whether it is a layer or a full window), on a component that does not own text keys.
fn letters_reach_footer(ctx: &Context) -> bool {
    let Some(scope) = focus::open_scope(ctx) else { return false };
    focus::focus_in_scope(ctx, scope) == Some(true) && focus::focused_rec(ctx).is_some_and(|rec| rec.stop.keys.owner == OwnerClass::Widget)
}

/// Whether an Enter reaches the default button of the footer being shown: focus is inside the focus scope the footer
/// sits in (the dialog's, whether it is a layer or a full window), or nothing is focused and that dialog is the top
/// modal.
fn enter_reaches_footer(ctx: &Context) -> bool {
    focus::open_scope(ctx).is_some_and(|scope| match focus::focus_in_scope(ctx, scope) {
        Some(inside) => inside,
        None => crate::runtime::overlay::is_top_modal(ctx, scope),
    })
}

#[derive(Default)]
enum Content {
    #[default]
    Body,
    Actions(Vec<FooterAction>),
    Buttons(Vec<FooterButton>),
}

/// The indexes of `roles` in display order: by role rank, and in the given order within a rank.
pub(crate) fn display_order(roles: &[DialogRole], windows: bool) -> Vec<usize> {
    let mut order: Vec<usize> = (0..roles.len()).collect();
    order.sort_by_key(|&at| roles[at].rank(windows));
    order
}

/// The labels of one prompt's buttons with their letters in the same style: `(Y)es` when every label starts with
/// its letter (case-insensitive), otherwise the letter is appended to every label (`Ja (Y)`, `Nein (N)`), so one
/// prompt never mixes the two. An answer without a letter keeps its text. macOS shows plain labels: the letters
/// still work.
pub fn accelerator_labels(answers: &[(&str, Option<char>)], os: Os) -> Vec<String> {
    let starts_with = |label: &str, letter: char| label.chars().next().is_some_and(|first| first.to_uppercase().eq(letter.to_uppercase()));
    let inline = answers.iter().all(|(label, letter)| letter.is_none_or(|letter| starts_with(label, letter)));
    answers
        .iter()
        .map(|(label, letter)| match letter {
            Some(letter) if os != Os::Mac => match label.chars().next() {
                Some(first) if inline => format!("({first}){}", &label[first.len_utf8()..]),
                _ => format!("{label} ({})", letter.to_uppercase()),
            },
            _ => (*label).to_owned(),
        })
        .collect()
}

impl FooterBar {
    /// A footer bar whose content is its body. It spans its region, so its content sits at the region's end edge; in a
    /// container sized from its content it takes its content's width there and never widens the container.
    pub fn body() -> Self {
        FooterBar {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::FillWidth).roving(Roving::new(NavShape::Flat)).layout(BodyLayout::row().align(RowAlign::End)),
            content: Content::Body,
            enter: false,
        }
    }

    /// A footer bar of icon actions, in the given order. Show it with `show_footer`.
    pub fn actions(actions: impl IntoIterator<Item = FooterAction>) -> Self {
        FooterBar { content: Content::Actions(actions.into_iter().collect()), ..FooterBar::body() }
    }

    /// A footer bar of dialog buttons, in the order of the platform's dialog convention. Show it with
    /// `show_footer`.
    pub fn buttons(buttons: impl IntoIterator<Item = FooterButton>) -> Self {
        FooterBar { content: Content::Buttons(buttons.into_iter().collect()), ..FooterBar::body() }
    }

    /// Shows the bar with the actions or buttons it was made with and reports what happened to them.
    pub fn show_footer(mut self, ui: &mut Ui) -> FooterOutput {
        let content = std::mem::take(&mut self.content);
        let enter = self.enter;
        let shown = self.show(ui, |ui, _| match &content {
            Content::Body => Shown::default(),
            Content::Actions(actions) => {
                let mut shown = Shown::default();
                for (at, action) in actions.iter().enumerate() {
                    let response = match &action.drops {
                        Some(drops) => {
                            let slot = drops(ui, action);
                            if let Some(payload) = slot.dropped {
                                shown.dropped = Some(FooterDrop { index: at, payload });
                            }
                            if slot.over {
                                shown.drop_hint = action.drop_hint.clone().or(shown.drop_hint.take());
                            }
                            slot.response
                        }
                        None => action.icon_button(false).show(ui).response,
                    };
                    if crate::contract::clicked(&response) {
                        shown.picked = Some(at);
                    }
                    shown.responses.push(response);
                }
                shown
            }
            Content::Buttons(buttons) => {
                let roles: Vec<DialogRole> = buttons.iter().map(|button| button.role).collect();
                let mut shown = Shown::default();
                let (ctx, owner) = (ui.ctx().clone(), ui.scope_id());
                let lettered = letters_reach_footer(&ctx);
                // Labels are resolved in the scope's language before the letters decorate them.
                let env = Env::of(ui);
                let resolved: Vec<Label> = buttons.iter().map(|button| env.view().resolve(&button.label)).collect();
                let lettering: Vec<(&str, Option<char>)> =
                    resolved.iter().zip(buttons).map(|(label, button)| (label.as_str(), button.letter)).collect();
                let labels = accelerator_labels(&lettering, crate::runtime::windows::os_of(&ctx));
                // Initial focus lands on the first enabled Accept button on every OS.
                let default_at = buttons.iter().position(|button| button.role == DialogRole::Accept && button.enabled);
                for at in display_order(&roles, crate::runtime::windows::os_of(ui.ctx()) == Os::Windows) {
                    let button = &buttons[at];
                    let variant = match button.role {
                        DialogRole::Accept => Variant::PRIMARY,
                        DialogRole::Destructive => Variant::DANGER,
                        DialogRole::Cancel | DialogRole::Other => Variant::SECONDARY,
                    };
                    let mut widget = Button::new(button.label.clone()).painted(labels[at].clone()).variant(variant).enabled(button.enabled);
                    let tip = if button.enabled { button.tooltip.clone() } else { button.disabled_reason.clone().or_else(|| button.tooltip.clone()) };
                    if let Some(tip) = tip {
                        widget = widget.tooltip(TooltipSpec::new(tip.clone()).disabled_text(tip));
                    }
                    if let Some(letter) = button.letter {
                        widget = widget.access_key(letter.to_ascii_uppercase());
                    }
                    if default_at == Some(at) {
                        widget = widget.default_stop();
                    }
                    if crate::contract::clicked(&widget.show(ui)) {
                        shown.picked = Some(at);
                    }
                    let chord = button.letter.and_then(Key::from_char).map(Chord::plain);
                    if let Some(chord) = chord.filter(|_| lettered && button.enabled && !ui.is_sizing_pass()) {
                        keys::capture(&ctx, owner, chord);
                        if keys::take_claimed(&ctx, owner, chord) {
                            shown.picked = Some(at);
                        }
                    }
                }
                if enter && shown.picked.is_none() && !ui.is_sizing_pass() {
                    shown.picked = default_at.filter(|_| enter_reaches_footer(&ctx) && keys::take_submit(&ctx));
                }
                shown
            }
        });
        let inner = shown.inner.unwrap_or_default();
        let mods = if inner.picked.is_some() { crate::runtime::pointer::frame(ui.ctx()).mods } else { craft_keymap::Mods::NONE };
        FooterOutput {
            response: shown.output,
            picked: inner.picked,
            dropped: inner.dropped,
            drop_hint: inner.drop_hint,
            responses: inner.responses,
            mods,
        }
    }
}

/// What the body of a footer collected in one pass.
#[derive(Default)]
struct Shown {
    picked: Option<usize>,
    dropped: Option<FooterDrop>,
    drop_hint: Option<Label>,
    responses: Vec<Response>,
}

impl Component for FooterBar {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = FooterBarStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: FooterBarStyleKey) -> FooterBarStyle {
        FooterBarStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    /// A footer bar the caller did not label is named by the words of the family.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if cx.resolved.label.is_empty() {
            access.name(Name::Text(cx.words::<SectionWords>().footer.as_str().to_owned()));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for FooterBar {}
