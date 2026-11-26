//! The intents routed to a component and what it leaves untaken: Submit, Cancel, Escape and context requests.

use super::route::push;
use super::{Nudge, Source, Stage, current, with};
use crate::runtime::focus::{self};
use craft_behavior::focus::{EscapeFacts, EscapeStep, escape_step};
use craft_behavior::{AccessRequest, CancelRule, EditCommand, EditorInput, Intent, MoveMods, NavMove, OwnerClass, PartRef};
use craft_keymap::{Chord, Key, Mods};
use egui::{Context, Id, Vec2};
use std::sync::Arc;

/// An owned copy of the intents routed to one id this pass; read through `cx.intents()`.
#[derive(Clone, Debug, Default)]
pub struct Intents {
    items: Vec<(Intent, bool)>,
    /// The press or text each of `items` came from (`keys::Item::source`); read by index, so an item pushed without
    /// one has none.
    sources: Vec<Option<Source>>,
    took_arrows: Vec<NavMove>,
    nudge: Option<Nudge>,
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

    /// Leaves an intent this pass's taker took to the enclosing scope, in the same pass: the next taker gets it. No
    /// input is queued or copied.
    pub(crate) fn pass_on(&mut self, intent: Intent) {
        if let Some((_, taken)) = self.items.iter_mut().find(|(item, taken)| *taken && item.same_kind(&intent)) {
            *taken = false;
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

    /// Takes the next move, whichever it is: for a component that acts on every move. A move taken is the component's,
    /// so the Page Up, Page Down, Home and End it takes never reach the scroll owner around it (a page view under a
    /// focused page part); a component that acts on some moves takes those with [`take_move_where`](Self::take_move_where)
    /// and leaves the rest.
    pub fn take_move(&mut self) -> Option<(NavMove, MoveMods)> {
        self.take_move_where(|_, _| true)
    }

    /// Takes the next move that `wanted` accepts; the others stay for a later taker, or for the scroll owner around a
    /// focused component when they scroll (9.2 rule 5).
    pub fn take_move_where(&mut self, wanted: impl Fn(NavMove, MoveMods) -> bool) -> Option<(NavMove, MoveMods)> {
        let at = self.items.iter().position(|(item, taken)| !taken && matches!(item, Intent::Move(mv, mods) if wanted(*mv, *mods)))?;
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
        self.nudge.take().map(|nudge| (nudge.direction, nudge.mods))
    }

    /// The keys that were down when the arrow of the nudge not yet taken went down, the arrow not among them; empty
    /// without one. Unlike `keys::held`, which is the state after the frame's input, it keeps the order of the frame's
    /// events: a key let go after the arrow in the same frame was held with it, and one pressed after it was not.
    pub fn nudge_held(&self) -> &[Key] {
        self.nudge.as_ref().map_or(&[], |nudge| &nudge.held)
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
    /// newline with its Shift, Tab and Shift+Tab, and Escape as a cancel. An arrow move the editor's `keep` does not
    /// keep stays for the component's navigation (18.4), and with `KeepKeys::None` so do Home, End and the page keys:
    /// the editor is a part of a grid that navigates until a cell opens. With it, the place among the frame's events of
    /// the press or text it came from, `None` for an intent of no one event.
    pub(crate) fn take_editor_input(&mut self, keep: focus::KeepKeys) -> Option<(EditorInput, Option<usize>)> {
        let kept = |item: &Intent| match item {
            Intent::Move(NavMove::Up | NavMove::Down, _) => matches!(keep, focus::KeepKeys::BlockArrows | focus::KeepKeys::Tab),
            Intent::Move(NavMove::Back | NavMove::Forward, _) => keep != focus::KeepKeys::None,
            Intent::Move(NavMove::Start | NavMove::End | NavMove::PageUp | NavMove::PageDown, _) => keep != focus::KeepKeys::None,
            _ => true,
        };
        let at = self.items.iter().position(|(item, taken)| !taken && kept(item) && editor_input_of(item, false).is_some())?;
        self.items[at].1 = true;
        let source = self.sources.get(at).copied().flatten();
        editor_input_of(&self.items[at].0, source.is_some_and(|source| source.shift)).map(|input| (input, source.map(|source| source.place)))
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

/// The editor input of `intent`, from a press with Shift held when `shift`.
fn editor_input_of(intent: &Intent, shift: bool) -> Option<EditorInput> {
    match intent {
        Intent::Text(text) => Some(EditorInput::Text(Arc::clone(text))),
        Intent::Move(mv, mods) => Some(EditorInput::Move(*mv, *mods)),
        Intent::Delete { forward, by, shift, .. } => Some(EditorInput::delete_with_shift(*forward, *by, *shift)),
        Intent::Edit(command) => Some(EditorInput::Edit(*command)),
        Intent::Submit => Some(EditorInput::newline(shift)),
        Intent::Tab => Some(EditorInput::Tab),
        Intent::BackTab => Some(EditorInput::BackTab),
        Intent::Cancel => Some(EditorInput::Cancel),
        _ => None,
    }
}

/// The intents routed to `id` this pass, with physical horizontal moves read in the component's direction.
#[doc(hidden)]
pub fn intents_of(ctx: &Context, id: Option<Id>, rtl: bool) -> Intents {
    let routed = current(ctx, |stage| {
        stage.intents_read = true;
        stage.routed.get(&id).cloned()
    })
    .unwrap_or_default();
    let flip = |mv| match (rtl, mv) {
        (true, NavMove::Back) => NavMove::Forward,
        (true, NavMove::Forward) => NavMove::Back,
        (_, mv) => mv,
    };
    let sources = routed.items.iter().map(|item| item.source).collect();
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
        sources,
        took_arrows: Vec::new(),
        nudge: routed.nudge,
        chords: routed.chords,
        set_value: routed.set_value,
        whole_reset: routed.whole_reset,
    }
}

/// Offers the window's intents and the focused framework stop's unused Escape to `take`, and writes back what it
/// left untaken. `take` returns whether it took any.
///
/// The window's intents are the keys the runtime routes to no component this pass: an Escape (`Intent::Cancel`)
/// or a plain Enter (`Intent::Submit`) while no framework stop is focused, and the arrows' nudge; with a framework
/// stop focused, its unused Escape, unless it is a text field. A component or a
/// scope that runs earlier in the pass takes them first (an open dialog takes the Escape that closes it, a canvas
/// session the one that ends it), so an app that hosts the runtime reads them last in its UI to see what no
/// component took. `rtl` reads horizontal moves in the window's direction.
pub fn with_window_intents(ctx: &Context, rtl: bool, take: impl FnOnce(&mut Intents) -> bool) -> bool {
    offer_window_intents(ctx, rtl, take)
}

/// Offers window intents to a canvas while the canvas or one of its controls has focus, or nothing has focus.
/// Other focused controls get the opportunity to use their Escape before the window reads it.
pub fn with_canvas_window_intents(ctx: &Context, rtl: bool, take: impl FnOnce(&mut Intents) -> bool) -> bool {
    if focus::focused_rec(ctx).is_some_and(|rec| !rec.stop.keys.chrome_key && !focus::is_canvas_member(ctx, &rec)) {
        return false;
    }
    offer_window_intents(ctx, rtl, take)
}

fn offer_window_intents(ctx: &Context, rtl: bool, take: impl FnOnce(&mut Intents) -> bool) -> bool {
    // A focused text field keeps the Escape it does not use, so typing in a palette's field never closes the
    // palette; the document's editor and a canvas control lend it as any other control does.
    let focused = focus::focused_rec(ctx)
        .filter(|rec| rec.stop.keys.owner != OwnerClass::Text || rec.stop.keys.is_document_editor() || focus::is_canvas_member(ctx, rec))
        .map(|rec| rec.stop.id);
    let mut window = intents_of(ctx, None, rtl);
    // The focused stop's leftover Escape stays in its slot until taken.
    let lent_cancel =
        focused.filter(|id| !window.items.iter().any(|(item, _)| *item == Intent::Cancel) && holds(ctx, Some(*id), &Intent::Cancel)).map(|id| {
            window.items.push((Intent::Cancel, false));
            (id, window.items.len() - 1)
        });
    let taken = take(&mut window);
    if let Some((id, at)) = lent_cancel {
        let (_, cancel_taken) = window.items.remove(at);
        if cancel_taken && !crate::runtime::measure::active(ctx) {
            current(ctx, |stage| take_from(stage, Some(id), &Intent::Cancel));
        }
    }
    if taken {
        settle(ctx, None, &window);
    }
    taken
}

/// Whether an intent of the kind of `intent` is routed to `id` and not taken this pass.
fn holds(ctx: &Context, id: Option<Id>, intent: &Intent) -> bool {
    current(ctx, |stage| stage.routed.get(&id).is_some_and(|routed| routed.items.iter().any(|item| item.intent.same_kind(intent))))
}

/// Removes one intent of the kind of `intent` routed to `id`; whether there was one.
fn take_from(stage: &mut Stage, id: Option<Id>, intent: &Intent) -> bool {
    let Some(routed) = stage.routed.get_mut(&id) else { return false };
    match routed.items.iter().position(|item| item.intent.same_kind(intent)) {
        Some(at) => {
            routed.items.remove(at);
            true
        }
        None => false,
    }
}

/// The keys an arena's session control borrowed this pass (`KeyUse::session`), with the ids they were routed to.
#[derive(Debug, Default)]
pub(crate) struct Lent {
    items: Vec<(Option<Id>, Intent)>,
    nudge: Option<Option<Id>>,
}

/// Lends `to`, an arena member that holds the arena's session, the Submit, Cancel and nudge routed to its arena root
/// `arena` and, while nothing in the window has focus, the window's. The member reads them through `intents_of`;
/// `give_back` returns what it leaves, so the arena's tool and the scopes around it still see those keys.
pub(crate) fn lend(ctx: &Context, arena: Id, to: Id) -> Lent {
    let mut lent = Lent::default();
    if crate::runtime::measure::active(ctx) {
        return lent;
    }
    let window = focus::focused_rec(ctx).is_none();
    current(ctx, |stage| {
        let sources = [Some(Some(arena)), window.then_some(None)];
        let (mut moved, mut nudge) = (Vec::new(), None);
        for from in sources.into_iter().flatten() {
            let Some(routed) = stage.routed.get_mut(&from) else { continue };
            routed.items.retain(|item| {
                let lends = matches!(item.intent, Intent::Submit | Intent::Cancel);
                if lends {
                    moved.push(item.clone());
                    lent.items.push((from, item.intent.clone()));
                }
                !lends
            });
            if nudge.is_none()
                && let Some(taken) = routed.nudge.take()
            {
                nudge = Some(taken);
                lent.nudge = Some(from);
            }
        }
        let own = stage.routed.entry(Some(to)).or_default();
        own.items.extend(moved);
        if nudge.is_some() {
            own.nudge = nudge;
        }
    });
    lent
}

/// Returns to their sources the keys of `lent` that `to` left untaken (after `settle`).
pub(crate) fn give_back(ctx: &Context, to: Id, lent: Lent) {
    if lent.items.is_empty() && lent.nudge.is_none() {
        return;
    }
    current(ctx, |stage| {
        let Some(own) = stage.routed.get_mut(&Some(to)) else { return };
        let mut back = Vec::new();
        let lent_items = lent.items.len();
        for (from, intent) in lent.items {
            if let Some(at) = own.items.iter().position(|item| item.intent == intent) {
                back.push((from, own.items.remove(at)));
            }
        }
        let nudge = lent.nudge.and_then(|from| own.nudge.take().map(|nudge| (from, nudge)));
        let taken = lent_items - back.len() + usize::from(lent.nudge.is_some() && nudge.is_none());
        stage.lent_taken += taken as u64;
        for (from, item) in back {
            stage.routed.entry(from).or_default().items.push(item);
        }
        if let Some((from, nudge)) = nudge {
            stage.routed.entry(from).or_default().nudge = Some(nudge);
        }
    });
}

/// How many lent keys session members have taken so far in the current viewport: a check of keyboard parity reads
/// it before and after its input, since a lent key is the session control's, not the focused component's.
#[cfg(feature = "suite")]
pub(crate) fn lent_taken(ctx: &Context) -> u64 {
    current(ctx, |stage| stage.lent_taken)
}

/// Takes the window's untaken `Cancel` (Escape) while nothing has focus.
pub(crate) fn take_window_cancel(ctx: &Context) -> bool {
    with_canvas_window_intents(ctx, false, |window| window.take(Intent::Cancel))
}

/// The end hook's hand-on: an Escape that nothing in the running window took, not even a reader of its intents, is the
/// Cancel of the window it was opened from, where its spec says so (`WindowSpec::hands_on_cancel`). That window reads it
/// in its next pass, which is requested. A popup or a modal open in the running window keeps the Escapes it leaves.
pub(crate) fn hand_on_cancel(ctx: &Context) {
    let Some(opener) = crate::runtime::windows::cancel_to(ctx) else { return };
    if crate::runtime::overlay::popup_or_modal_open(ctx) {
        return;
    }
    let mut left = 0;
    with_window_intents(ctx, false, |window| {
        while window.take(Intent::Cancel) {
            left += 1;
        }
        left > 0
    });
    if left > 0 {
        let mods = current(ctx, |stage| stage.escape_mods).unwrap_or(Mods::NONE);
        with(ctx, opener, |stage| stage.handed_cancels.extend(std::iter::repeat_n(mods, left)));
        crate::runtime::egui::repaint::request_of(ctx, opener);
    }
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
    current(ctx, |stage| take_from(stage, focused, intent))
}

/// Takes every Escape of this pass that no component has taken, wherever the runtime routed it: the untaken `Cancel`
/// of every stop and of the window, and every Escape chord held for a claim or a capture. For a dialog opened in this
/// pass, the top layer from then on: the stop the keys were routed to need not hold focus any more, since the press
/// that opened the dialog may have moved it.
pub(crate) fn take_every_escape(ctx: &Context) -> bool {
    if crate::runtime::measure::active(ctx) {
        return false;
    }
    current(ctx, |stage| {
        let mut taken = false;
        for routed in stage.routed.values_mut() {
            let before = routed.items.len();
            routed.items.retain(|item| !item.intent.same_kind(&Intent::Cancel));
            taken |= routed.items.len() != before;
        }
        let before = stage.claimed.len();
        stage.claimed.retain(|(_, chord)| chord.key != Key::Escape);
        taken || stage.claimed.len() != before
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

/// Records that `id` asked for a context menu in this pass, to open at `rect`, at its part `part` when the request
/// was made at one: a secondary-button release on it, or a `ContextMenu` intent it did not take (9.2 rule 3).
pub(crate) fn request_context(ctx: &Context, id: Id, rect: egui::Rect, part: Option<PartRef>) {
    current(ctx, |stage| {
        if !stage.contexts.iter().any(|(owner, ..)| *owner == id) {
            stage.contexts.push((id, rect, part));
        }
    });
}

/// The rect `id`'s context request of this pass opens at, when it made one. A `Trigger::Context` opener reads it.
pub(crate) fn context_request(ctx: &Context, id: Id) -> Option<egui::Rect> {
    current(ctx, |stage| stage.contexts.iter().find(|(owner, ..)| *owner == id).map(|(_, rect, _)| *rect))
}

/// Every context request of this pass so far, in the order they were made: the component and the rect it opens at.
pub(crate) fn context_requests(ctx: &Context) -> Vec<(Id, egui::Rect)> {
    current(ctx, |stage| stage.contexts.iter().map(|(owner, rect, _)| (*owner, *rect)).collect())
}

/// The part of `id` its context request of this pass was made at, when it made one at a part.
pub(crate) fn context_part(ctx: &Context, id: Id) -> Option<PartRef> {
    current(ctx, |stage| stage.contexts.iter().find(|(owner, ..)| *owner == id).and_then(|(.., part)| *part))
}
