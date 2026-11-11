//! The intents routed to a component and what it leaves untaken: Submit, Cancel, Escape and context requests.

use super::current;
use super::route::push;
use crate::runtime::focus::{self};
use craft_behavior::focus::{EscapeFacts, EscapeStep, escape_step};
use craft_behavior::{AccessRequest, CancelRule, EditCommand, EditorInput, Intent, MoveMods, NavMove, PartRef};
use craft_keymap::{Chord, Mods};
use egui::{Context, Id, Vec2};
use std::sync::Arc;

/// An owned copy of the intents routed to one id this pass; read through `cx.intents()`.
#[derive(Clone, Debug, Default)]
pub struct Intents {
    items: Vec<(Intent, bool)>,
    took_arrows: Vec<NavMove>,
    nudge: Option<(Vec2, Mods)>,
    chords: Vec<Chord>,
    set_value: Option<(Option<PartRef>, f64)>,
    whole_reset: bool,
}

impl Intents {
    /// Queues an intent for this pass, as the key stage does for its own.
    pub(crate) fn push(&mut self, intent: Intent) {
        self.items.push((intent, false));
    }

    /// Takes the next typed digit.
    pub fn take_digit(&mut self) -> Option<u8> {
        let at = self.items.iter().position(|(item, taken)| !taken && matches!(item, Intent::Digit(_)))?;
        self.items[at].1 = true;
        match self.items[at].0 {
            Intent::Digit(digit) => Some(digit),
            _ => None,
        }
    }

    /// Whether an AccessKit click names some part of the component; reads without taking.
    pub(crate) fn any_access_click(&self) -> bool {
        self.items.iter().any(|(item, taken)| {
            !taken && matches!(item, Intent::Access(request) if request.target.is_some() && request.action == accesskit::Action::Click)
        })
    }

    /// Whether an AccessKit click names `part`; reads without taking.
    pub(crate) fn access_click(&self, part: PartRef) -> bool {
        self.items.iter().any(|(item, taken)| {
            !taken && matches!(item, Intent::Access(request) if request.target == Some(part) && request.action == accesskit::Action::Click)
        })
    }

    /// Takes a `Reset`, once per pass: `Some(true)` when the label step sent it (the whole binding resets),
    /// `Some(false)` when a key sent it (it resets the focused part when that part has its own reset).
    pub(crate) fn take_reset(&mut self) -> Option<bool> {
        self.take(Intent::Reset).then_some(self.whole_reset)
    }

    /// Takes an intent of the kind of `intent`, once per pass: the first taker gets it.
    pub fn take(&mut self, intent: Intent) -> bool {
        match self.items.iter().position(|(item, taken)| !taken && item.same_kind(&intent)) {
            Some(at) => {
                self.items[at].1 = true;
                true
            }
            None => false,
        }
    }

    /// Takes an intent equal to `intent` in every field, once per pass: `Intent::delete(false, Granularity::Grapheme)`
    /// takes Backspace and leaves Delete, which `take` would also take.
    pub fn take_exact(&mut self, intent: &Intent) -> bool {
        match self.items.iter().position(|(item, taken)| !taken && item == intent) {
            Some(at) => {
                self.items[at].1 = true;
                true
            }
            None => false,
        }
    }

    /// Takes the next move.
    pub fn take_move(&mut self) -> Option<(NavMove, MoveMods)> {
        let at = self.items.iter().position(|(item, taken)| !taken && matches!(item, Intent::Move(..)))?;
        self.items[at].1 = true;
        match self.items[at].0 {
            Intent::Move(mv, mods) => {
                if matches!(mv, NavMove::Back | NavMove::Forward | NavMove::Up | NavMove::Down) {
                    self.took_arrows.push(mv);
                }
                Some((mv, mods))
            }
            _ => None,
        }
    }

    /// The arrow moves `take_move` took this pass: the run step checks that the component holds those keys.
    pub(crate) fn taken_arrow_moves(&self) -> &[NavMove] {
        &self.took_arrows
    }

    /// Takes the next move that scrolls (page up or down, start, end): what a focused component that does not
    /// take it leaves to the scroll owner around it (9.2 rule 5).
    pub(crate) fn take_scroll_move(&mut self) -> Option<NavMove> {
        let at = self.items.iter().position(|(item, taken)| {
            !taken && matches!(item, Intent::Move(NavMove::PageUp | NavMove::PageDown | NavMove::Start | NavMove::End, _))
        })?;
        self.items[at].1 = true;
        match self.items[at].0 {
            Intent::Move(mv, _) => Some(mv),
            _ => None,
        }
    }

    /// Takes the next AccessKit request that asks a scroll region to scroll, or with `into_view` to show its target.
    /// A component that scrolls reads its own requests through it.
    pub fn take_scroll_access(&mut self, into_view: bool) -> Option<AccessRequest> {
        use craft_behavior::AccessAction as Action;
        let at = self.items.iter().position(|(item, taken)| {
            !taken
                && matches!(
                    item,
                    Intent::Access(request)
                        if request.target.is_none()
                            && (request.action == Action::ScrollIntoView) == into_view
                            && matches!(
                                request.action,
                                Action::ScrollUp | Action::ScrollDown | Action::ScrollLeft | Action::ScrollRight | Action::ScrollIntoView | Action::SetScrollOffset
                            )
                )
        })?;
        self.items[at].1 = true;
        match &self.items[at].0 {
            Intent::Access(request) => Some(request.clone()),
            _ => None,
        }
    }

    /// Takes the next edit command.
    pub fn take_edit(&mut self) -> Option<EditCommand> {
        let at = self.items.iter().position(|(item, taken)| !taken && matches!(item, Intent::Edit(_)))?;
        self.items[at].1 = true;
        match self.items[at].0 {
            Intent::Edit(command) => Some(command),
            _ => None,
        }
    }

    /// Takes the next typed text.
    pub fn take_text(&mut self) -> Option<Arc<str>> {
        let at = self.items.iter().position(|(item, taken)| !taken && matches!(item, Intent::Text(_)))?;
        self.items[at].1 = true;
        match &self.items[at].0 {
            Intent::Text(text) => Some(Arc::clone(text)),
            _ => None,
        }
    }

    /// Takes the physical arrow nudge for canvas content: a unit direction and the modifiers.
    pub fn take_nudge(&mut self) -> Option<(Vec2, Mods)> {
        self.nudge.take()
    }

    /// Takes the chords this pass's presses spell, for a component whose key use is `KeyUse::RECORDER`; empty for any
    /// other component. A recorder gets no other key intents, and no command runs on its presses.
    pub fn take_chords(&mut self) -> Vec<Chord> {
        std::mem::take(&mut self.chords)
    }

    /// Takes the AccessKit `SetValue` request aimed at the component's own node.
    pub fn take_set_value(&mut self) -> Option<f64> {
        self.take_set_value_of(None)
    }

    /// Takes the AccessKit `SetValue` request aimed at the sub-node of `part`.
    pub fn take_set_value_at(&mut self, part: impl Into<PartRef>) -> Option<f64> {
        self.take_set_value_of(Some(part.into()))
    }

    fn take_set_value_of(&mut self, part: Option<PartRef>) -> Option<f64> {
        match self.set_value {
            Some((target, value)) if target == part => {
                self.set_value = None;
                Some(value)
            }
            _ => None,
        }
    }

    /// Whether an intent of the kind of `intent` is routed and not taken.
    pub fn untaken(&self, intent: &Intent) -> bool {
        self.items.iter().any(|(item, taken)| !taken && item.same_kind(intent))
    }

    /// Takes the next intent an editor reads as input: typed text, a move, a delete, an edit command, Enter as a
    /// newline, Tab, and Escape as a cancel. An arrow move the editor's `keep` does not keep stays for the
    /// component's navigation (18.4), and with `KeepKeys::None` so do Home, End and the page keys: the editor is
    /// a part of a grid that navigates until a cell opens.
    pub(crate) fn take_editor_input(&mut self, keep: focus::KeepKeys) -> Option<EditorInput> {
        let kept = |item: &Intent| match item {
            Intent::Move(NavMove::Up | NavMove::Down, _) => matches!(keep, focus::KeepKeys::BlockArrows | focus::KeepKeys::Tab),
            Intent::Move(NavMove::Back | NavMove::Forward, _) => keep != focus::KeepKeys::None,
            Intent::Move(NavMove::Start | NavMove::End | NavMove::PageUp | NavMove::PageDown, _) => keep != focus::KeepKeys::None,
            _ => true,
        };
        let at = self.items.iter().position(|(item, taken)| !taken && kept(item) && editor_input_of(item).is_some())?;
        self.items[at].1 = true;
        editor_input_of(&self.items[at].0)
    }

    /// Takes the next `Intent::Access` request that `wanted` accepts.
    #[doc(hidden)]
    pub fn take_access_where(&mut self, wanted: impl Fn(&AccessRequest) -> bool) -> Option<AccessRequest> {
        let at = self.items.iter().position(|(item, taken)| !taken && matches!(item, Intent::Access(request) if wanted(request)))?;
        self.items[at].1 = true;
        match &self.items[at].0 {
            Intent::Access(request) => Some(request.clone()),
            _ => None,
        }
    }

    /// Takes the next `Intent::Access` request.
    pub fn take_access(&mut self) -> Option<AccessRequest> {
        let at = self.items.iter().position(|(item, taken)| !taken && matches!(item, Intent::Access(_)))?;
        self.items[at].1 = true;
        match &self.items[at].0 {
            Intent::Access(request) => Some(request.clone()),
            _ => None,
        }
    }
}

fn editor_input_of(intent: &Intent) -> Option<EditorInput> {
    match intent {
        Intent::Text(text) => Some(EditorInput::Text(Arc::clone(text))),
        Intent::Move(mv, mods) => Some(EditorInput::Move(*mv, *mods)),
        Intent::Delete { forward, by, .. } => Some(EditorInput::delete(*forward, *by)),
        Intent::Edit(command) => Some(EditorInput::Edit(*command)),
        Intent::Submit => Some(EditorInput::Newline),
        Intent::Tab => Some(EditorInput::Tab),
        Intent::Cancel => Some(EditorInput::Cancel),
        _ => None,
    }
}

/// The intents routed to `id` this pass, with physical horizontal moves read in the component's direction.
#[doc(hidden)]
pub fn intents_of(ctx: &Context, id: Option<Id>, rtl: bool) -> Intents {
    let routed = current(ctx, |stage| stage.routed.get(&id).cloned()).unwrap_or_default();
    let flip = |mv| match (rtl, mv) {
        (true, NavMove::Back) => NavMove::Forward,
        (true, NavMove::Forward) => NavMove::Back,
        (_, mv) => mv,
    };
    let items = routed
        .items
        .into_iter()
        .map(|item| match item.intent {
            Intent::Move(mv, mods) if item.horizontal => (Intent::Move(flip(mv), mods), false),
            intent => (intent, false),
        })
        .collect();
    Intents {
        items,
        took_arrows: Vec::new(),
        nudge: routed.nudge,
        chords: routed.chords,
        set_value: routed.set_value,
        whole_reset: routed.whole_reset,
    }
}

/// Offers the window's unfocused intents to `take` while nothing in the window has focus, and writes back what it
/// left untaken. `take` returns whether it took any. `false` when something is focused or nothing was taken.
#[doc(hidden)]
pub fn with_window_intents(ctx: &Context, rtl: bool, take: impl FnOnce(&mut Intents) -> bool) -> bool {
    if focus::focused_rec(ctx).is_some() {
        return false;
    }
    let mut window = intents_of(ctx, None, rtl);
    let taken = take(&mut window);
    if taken {
        settle(ctx, None, &window);
    }
    taken
}

/// Takes the window's untaken `Cancel` (Escape) while nothing has focus.
pub(crate) fn take_window_cancel(ctx: &Context) -> bool {
    with_window_intents(ctx, false, |window| window.take(Intent::Cancel))
}

/// Writes back what a component left untaken, so a scope's `Submit` and `Cancel` rules see it.
#[doc(hidden)]
pub fn settle(ctx: &Context, id: Option<Id>, intents: &Intents) {
    if crate::runtime::measure::active(ctx) {
        return;
    }
    current(ctx, |stage| {
        let Some(routed) = stage.routed.get_mut(&id) else { return };
        let mut taken = intents.items.iter().map(|(_, taken)| *taken);
        routed.items.retain(|_| !taken.next().unwrap_or(false));
        if intents.nudge.is_none() {
            routed.nudge = None;
        }
        if intents.chords.is_empty() {
            routed.chords.clear();
        }
        if intents.set_value.is_none() {
            routed.set_value = None;
        }
    });
}

/// Routes a `Reset` of the whole binding to the component `target` for the rest of this pass: the label step sends
/// it for a double click on the label and for the reset dot.
pub(crate) fn route_whole_reset(ctx: &Context, target: Id) {
    current(ctx, |stage| {
        push(&mut stage.routed, Some(target), Intent::Reset, false);
        stage.routed.entry(Some(target)).or_default().whole_reset = true;
    });
}

/// Takes an untaken `Submit` for the innermost scope with a default action.
#[doc(hidden)]
pub fn take_submit(ctx: &Context) -> bool {
    take_untaken(ctx, &Intent::Submit)
}

/// Takes an untaken `Cancel` (Escape) for the focused stop, without deciding what the scope does with it.
pub(crate) fn take_cancel(ctx: &Context) -> bool {
    take_untaken(ctx, &Intent::Cancel)
}

fn take_untaken(ctx: &Context, intent: &Intent) -> bool {
    let focused = focus::focused_rec(ctx).map(|rec| rec.stop.id);
    // One-shot takes deliver in the real run only (runtime/measure.rs `take_queued`).
    if crate::runtime::measure::active(ctx) {
        return false;
    }
    current(ctx, |stage| {
        let Some(routed) = stage.routed.get_mut(&focused) else { return false };
        match routed.items.iter().position(|item| item.intent.same_kind(intent)) {
            Some(at) => {
                routed.items.remove(at);
                true
            }
            None => false,
        }
    })
}

/// What an untaken Escape does in a scope: `None` when no Escape arrived or a component took it. One scope
/// takes each Escape.
pub(crate) fn scope_escape(ctx: &Context, rule: CancelRule, press_captured: bool, scope_clearable: bool) -> Option<EscapeStep> {
    if !take_cancel(ctx) {
        return None;
    }
    let facts = EscapeFacts::new().press_captured(press_captured).in_row_child(focus::in_row_child(ctx)).scope_clearable(scope_clearable);
    Some(escape_step(facts, rule))
}

/// Records that `id` asked for a context menu in this pass, to open at `rect`: a secondary-button release on it, or
/// a `ContextMenu` intent it did not take (9.2 rule 3).
pub(crate) fn request_context(ctx: &Context, id: Id, rect: egui::Rect) {
    current(ctx, |stage| {
        if !stage.contexts.iter().any(|(owner, _)| *owner == id) {
            stage.contexts.push((id, rect));
        }
    });
}

/// The rect `id`'s context request of this pass opens at, when it made one. A `Trigger::Context` opener reads it.
pub(crate) fn context_request(ctx: &Context, id: Id) -> Option<egui::Rect> {
    current(ctx, |stage| stage.contexts.iter().find(|(owner, _)| *owner == id).map(|(_, rect)| *rect))
}
