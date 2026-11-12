//! The begin hook's routing: key presses, shortcuts and AccessKit requests to the intents of their owners.

use super::input::chrome_toggle_of;
use super::{Batch, Item, MenuKey, Node, Press, Routed, Shortcuts, Taken, current, with};
use crate::runtime::commands::{self, FirePhase};
use crate::runtime::egui::input::{Action, ActionKind};
use crate::runtime::focus::{self, StopRec};
use crate::runtime::frame::request_chrome_toggle;
use crate::runtime::overlay;
use craft_behavior::focus::{EscapeRouting, key_owner};
use craft_behavior::keys;
use craft_behavior::{EditCommand, Intent, KeyOwner, MoveMods, NavMove, OwnerClass};
use craft_host::target::Os;
use craft_keymap::{Chord, Key, Layout, Mods};
use egui::{Context, Id, Vec2};
use std::collections::HashMap;
use std::sync::Arc;

/// The begin hook's intents: the pass's key events, AccessKit requests and claimed chords, routed to the
/// focused component (or to the component that owns a requested node). Returns how the pass's Escapes split
/// between the captured press and the components.
pub(crate) fn begin(ctx: &Context, press_captured: bool) -> EscapeRouting {
    let os = crate::runtime::windows::os_of(ctx);
    let target = focus::focused_rec(ctx);
    let (batch, nodes, captures) = current(ctx, |stage| {
        stage.routed.clear();
        stage.contexts.clear();
        stage.claimed.clear();
        stage.activated.clear();
        stage.claims.clear();
        stage.last_open_edits = std::mem::take(&mut stage.open_edits);
        stage.last_nodes = std::mem::take(&mut stage.nodes);
        (std::mem::take(&mut stage.batch), stage.last_nodes.clone(), std::mem::take(&mut stage.captures))
    });
    let modal = overlay::any_open(ctx, overlay::OpenFilter::Modal);
    let stale_repeat = |stale: &mut bool, press: &Press| {
        if press.key != Key::Enter || !press.mods.is_none() {
            return false;
        }
        *stale &= press.repeat;
        *stale
    };
    // An Enter that began before the modal layer opened (the field's Enter that raised it, and its repeats)
    // answers nothing in the layer; only a fresh press does.
    let mut stale = current(ctx, |stage| {
        stage.enter_stale = modal && (stage.enter_stale || (!stage.modal_open && stage.enter_down));
        stage.enter_stale
    });
    let mut routed: HashMap<Option<Id>, Routed> = HashMap::new();
    let mut activated = HashMap::new();
    let mut consumed = vec![Taken::No; batch.presses.len()];
    let driven = target.as_ref().and_then(|rec| Driven::of(ctx, rec));
    let field_list = target.as_ref().filter(|_| driven.is_none()).and_then(|rec| list_of_field(ctx, rec));
    // Whether a component took each text of the pass, as typing, type-ahead or digits.
    let mut taken_texts = vec![false; batch.texts.len()];
    if let Some(rec) = &target {
        for (press, taken) in batch.presses.iter().zip(&mut consumed) {
            *taken = if stale_repeat(&mut stale, press) {
                Taken::Yes
            } else {
                let route = press_route(ctx, os, rec, press, driven.or(field_list));
                apply_press(ctx, rec, press, route, &mut routed, &mut activated)
            };
        }
        for (text, taken) in batch.texts.iter().zip(&mut taken_texts) {
            let route = text_route(ctx, rec, driven, text);
            *taken = !route.is_empty();
            for (to, intent) in route {
                push(&mut routed, Some(to), intent, false);
            }
        }
    } else {
        // With nothing focused, a plain Enter submits to the window, as an unfocused Escape cancels: a tool's pending
        // edit or a dialog's default button takes it. The arrows nudge the window's canvas content (a tool or the
        // session control on a canvas), unless a modal layer is open.
        for (press, taken) in batch.presses.iter().zip(&mut consumed) {
            if press.key == Key::Enter && press.mods.is_none() && !press.repeat {
                push(&mut routed, None, Intent::Submit, false);
                *taken = Taken::Yes;
            } else if !modal && let Some(nudge) = nudge_of(press.key, press.mods) {
                routed.entry(None).or_default().nudge = Some(nudge);
                *taken = Taken::Yes;
            }
        }
    }
    let focused = target.as_ref().map(|rec| rec.stop.id);
    // The web sends a chord's key press and its clipboard event: the command reaches the component once.
    let mut clips = batch.clips.clone();
    clips.dedup();
    for command in &clips {
        push(&mut routed, focused, Intent::Edit(*command), false);
    }
    let queued = current(ctx, |stage| std::mem::take(&mut stage.queued_edits));
    for command in queued {
        push(&mut routed, focused, Intent::Edit(command), false);
    }
    let (shortcuts, held) = owned_shortcuts(ctx, target.as_ref(), &batch, &consumed, &taken_texts);
    withdraw_held(ctx, target.as_ref(), &batch, &held);
    let pasted = batch.paste.as_deref().zip(focused).map(|(text, id)| (id, Arc::<str>::from(text)));
    let escapes = craft_behavior::focus::route_escapes(batch.escapes, press_captured);
    for _ in 0..escapes.to_components {
        // A row's child that claims Escape keeps it; any other returns focus to its row.
        let claims_escape = target.as_ref().is_some_and(|rec| {
            rec.stop.keys.claims.iter().any(|chord| chord.key == Key::Escape) || current(ctx, |stage| stage.last_open_edits.contains(&rec.stop.id))
        });
        if !claims_escape && target.as_ref().is_some_and(|rec| focus::leave_row(ctx, rec.stop.key()).is_some()) {
            continue;
        }
        push(&mut routed, focused, Intent::Cancel, false);
    }
    // A text opener's list keys arrive as claimed chords, withheld from its text part: they go to the list.
    let mut claimed = Vec::new();
    for chord in &batch.claimed {
        match driven.and_then(|driven| driven.intent_of(chord)) {
            Some(intent) => push(&mut routed, driven.map(|driven| driven.list), intent, false),
            None => claimed.extend(captures.iter().find(|(_, held)| held == chord).map(|(owner, _)| *owner).or(focused).map(|id| (id, *chord))),
        }
    }
    for request in &batch.access {
        route_access(ctx, request, &nodes, &mut routed, &mut activated);
    }
    current(ctx, |stage| {
        stage.routed = routed;
        stage.claimed = claimed;
        stage.activated = activated;
        stage.menu = batch.menu;
        if std::mem::take(&mut stage.fired_menu) {
            stage.menu.push(MenuKey::Focus);
        }
        stage.shortcuts = shortcuts;
        if !stage.shortcuts.keys.is_empty() {
            stage.shortcuts.layout = stage.layout.clone();
        }
        stage.pasted = pasted;
        stage.enter_stale = stale;
        stage.modal_open = modal;
        if let Some(down) = batch.enter_down {
            stage.enter_down = down;
        }
        if let (Some(_), Some(rec)) = (driven.filter(|driven| driven.text), &target) {
            stage.claims.extend(
                LIST_KEYS
                    .iter()
                    .filter(|key| driven.is_some_and(|driven| driven.intent(**key).is_some()))
                    .map(|key| (rec.stop.id, Chord::plain(*key))),
            );
        }
    });
    escapes
}

pub(super) fn push(routed: &mut HashMap<Option<Id>, Routed>, target: Option<Id>, intent: Intent, horizontal: bool) {
    routed.entry(target).or_default().items.push(Item { intent, horizontal });
}

/// Who owns the keys at the stop `target`, whose scope chain is `scopes`: its own class and the class of the layer
/// it is in, or `Blocked` under a modal layer. With `below`, only the layers under the layer `below` count.
pub(crate) fn owner_at(ctx: &Context, target: Option<&StopRec>, scopes: &[Id], below: Option<Id>) -> KeyOwner {
    let layer = overlay::key_layer(ctx, scopes, below);
    let mut present: Vec<OwnerClass> = target.map(|rec| rec.stop.keys.owner).into_iter().collect();
    if let overlay::LayerKeys::Class(class) = layer {
        present.push(class);
    }
    key_owner(layer == overlay::LayerKeys::Blocked, &present)
}

/// The shortcut events the key owner leaves to the app (9.3), and the presses the focused component holds
/// (`holds_press`). The owner is the focused component's class, or `Blocked` while a modal layer is stacked above the
/// focus; inside a layer its `Overlay.keys` class ranks with the component's. `Menu` and `Recorder` keep every key; a
/// text field keeps the text its presses typed. `consumed` is what routing did with each press and `taken_texts` whether
/// a component took each text of the pass.
fn owned_shortcuts(ctx: &Context, target: Option<&StopRec>, batch: &Batch, consumed: &[Taken], taken_texts: &[bool]) -> (Shortcuts, Vec<bool>) {
    let scopes = target.map(|rec| focus::scope_chain(ctx, rec)).unwrap_or_default();
    let owner = owner_at(ctx, target, &scopes, None);
    let own = target.map(|rec| rec.stop.keys.owner);
    let silent = keeps_every_key(owner);
    let dialog = owner == KeyOwner::Class(OwnerClass::Dialog);
    // The focused component keeps its own keys inside a dialog: a text field its typing, a widget its activation.
    let typing = owner != KeyOwner::Blocked && own == Some(OwnerClass::Text);
    let held: Vec<bool> = batch
        .presses
        .iter()
        .zip(consumed)
        .map(|(press, taken)| holds_press(owner, target, press, *taken == Taken::Yes, press.text.is_some_and(|at| taken_texts[at])))
        .collect();
    let keys = if silent {
        Vec::new()
    } else {
        batch
            .shortcuts
            .iter()
            .filter(|shortcut| !(typing && shortcut.typed))
            .filter(|shortcut| !shortcut.press.is_some_and(|at| held[at]))
            .copied()
            .collect()
    };
    (Shortcuts { keys, typing, dialog, layout: Layout::default() }, held)
}

/// Whether the focused stop `target`, whose keys `owner` owns, holds `press`: the press is its own and reaches neither
/// egui's input, where the app reads its own keys, nor the app's shortcuts. A widget or popup holds the presses its
/// routing `taken`, the keys its filter keeps from the key stage, and the presses whose text it took (`typed`: the
/// type-ahead of a list). A menu or a recorder holds every press.
fn holds_press(owner: KeyOwner, target: Option<&StopRec>, press: &Press, taken: bool, typed: bool) -> bool {
    let Some(keys) = target.map(|rec| &rec.stop.keys) else { return false };
    if keeps_every_key(owner) {
        return true;
    }
    let widget = owner != KeyOwner::Blocked && matches!(keys.owner, OwnerClass::Widget | OwnerClass::Popup | OwnerClass::PassivePopup);
    let kept = match keys.keep {
        focus::KeepKeys::Tab => press.key == Key::Tab,
        focus::KeepKeys::InlineArrows => matches!(press.key, Key::Left | Key::Right),
        focus::KeepKeys::BlockArrows => matches!(press.key, Key::Left | Key::Right | Key::Up | Key::Down),
        _ => false,
    };
    widget && (taken || typed || kept)
}

/// `Menu` and `Recorder` keep every key (9.3): no press of theirs reaches egui's input or the app's shortcuts.
fn keeps_every_key(owner: KeyOwner) -> bool {
    matches!(owner, KeyOwner::Class(OwnerClass::Menu | OwnerClass::Recorder))
}

/// Where the focused stop `rec` sends the typed `text`, and as what: a digits stop takes its digits, and the rest goes
/// to the list `driven` drives, to a text or type-ahead stop, or to the menu around the stop. Empty when no component
/// takes the text. Reads state only, so a query can ask it.
fn text_route(ctx: &Context, rec: &StopRec, driven: Option<Driven>, text: &str) -> Vec<(Id, Intent)> {
    let mut route = Vec::new();
    let rest;
    let text = if rec.stop.role.digits {
        let (found, left) = keys::split_digits(text);
        route.extend(found.into_iter().map(|digit| (rec.stop.id, Intent::Digit(digit))));
        rest = left;
        rest.as_str()
    } else {
        text
    };
    if text.is_empty() {
        return route;
    }
    let letters = !text.trim().is_empty();
    let to = if let Some(driven) = driven.filter(|driven| !driven.text && letters) {
        Some(driven.list)
    } else if matches!(rec.stop.keys.owner, OwnerClass::Text) || (rec.stop.role.type_ahead && letters) {
        Some(rec.stop.id)
    } else if letters {
        focus::menu_scope(ctx, rec)
    } else {
        None
    };
    route.extend(to.map(|to| (to, Intent::Text(Arc::from(text)))));
    route
}

/// The text a press of `chord` types: the character of a letter, digit or symbol key pressed with no Cmd, Ctrl or Alt
/// held (`Chord::is_plain`, the chords typing would trigger).
fn text_of(chord: Chord) -> Option<&'static str> {
    let key = chord.key;
    (chord.is_plain() && (key.is_letter() || key.is_digit() || key.is_symbol())).then(|| key.name())
}

/// Whether the focused component of the running viewport holds a press of `chord`, as routing decides for the press:
/// it claims the chord, its routing takes the press or the text the press types (a list's type-ahead), or a menu or
/// recorder has the keys. No app command bound to the chord runs while the component has the focus, and the press
/// reaches egui's input only for the app's own widget (`Stop::host`), which reads its keys there.
pub fn focus_holds(ctx: &Context, chord: Chord) -> bool {
    let Some(rec) = focus::focused_rec(ctx) else { return false };
    let viewport = crate::runtime::egui::pass::current_viewport(ctx);
    let record = focus::record(ctx, viewport);
    if current(ctx, |stage| super::input::focused_claims(&record, stage)).contains(&chord) {
        return true;
    }
    let press = Press { key: chord.key, mods: chord.mods, repeat: false, in_egui: None, release: None, text: None };
    let driven = Driven::of(ctx, &rec);
    let route = press_route(ctx, crate::runtime::windows::os_of(ctx), &rec, &press, driven.or_else(|| list_of_field(ctx, &rec)));
    let taken = !matches!(route, PressRoute::Free);
    let typed = text_of(chord).is_some_and(|text| !text_route(ctx, &rec, driven, text).is_empty());
    let owner = owner_at(ctx, Some(&rec), &focus::scope_chain(ctx, &rec), None);
    holds_press(owner, Some(&rec), &press, taken, typed)
}

/// Takes the presses the focused component holds out of the running pass's egui input, where the app reads its own
/// keys: one press, one action. A held press whose release has not come yet leaves its release to the strip. The
/// app's own widget (`Stop::host`) reads its keys from egui, so its presses stay.
fn withdraw_held(ctx: &Context, target: Option<&StopRec>, batch: &Batch, held: &[bool]) {
    if target.is_some_and(|rec| rec.stop.host) {
        return;
    }
    let presses: Vec<&Press> =
        batch.presses.iter().zip(held).filter(|(press, held)| **held && press.in_egui.is_some()).map(|(press, _)| press).collect();
    if presses.is_empty() {
        return;
    }
    let places: Vec<usize> = presses.iter().filter_map(|press| press.in_egui).collect();
    let open = crate::runtime::egui::input::withdraw_presses(ctx, &places);
    current(ctx, |stage| {
        for key in presses.iter().zip(open).filter(|(_, open)| *open).filter_map(|(press, _)| press.release) {
            if !stage.consumed.contains(&key) {
                stage.consumed.push(key);
            }
        }
    });
}

/// The keyboard layout of the running viewport.
pub(crate) fn layout_of(ctx: &Context) -> Layout {
    current(ctx, |stage| stage.layout.clone())
}

/// Takes the shortcut events the key stage left to the app this pass, for the dispatcher at H7.
///
/// A press of a chord a chrome command is rebound to fires that command here: it toggles the running viewport's panels
/// and goes no further.
pub(crate) fn take_shortcuts(ctx: &Context) -> Shortcuts {
    let mut shortcuts = current(ctx, |stage| std::mem::take(&mut stage.shortcuts));
    let viewport = crate::runtime::egui::pass::current_viewport(ctx);
    shortcuts.keys.retain(|shortcut| {
        let toggle = (shortcut.phase == FirePhase::Press).then(|| chrome_toggle_of(ctx, &shortcut.input)).flatten();
        toggle.inspect(|toggle| request_chrome_toggle(ctx, viewport, *toggle)).is_none()
    });
    shortcuts
}

/// Asks the focused component of the running viewport to take `command` as `Intent::Edit` in the next pass.
/// `false` when it does not list the command. A paste asks the platform for the clipboard text, which arrives as
/// the paste event that becomes the intent. Focus is read as it is now: a menu pick returns focus in its pass
/// before its command fires, and the command goes where focus returned.
pub(crate) fn queue_edit(ctx: &Context, command: EditCommand) -> bool {
    let viewport = crate::runtime::egui::pass::current_viewport(ctx);
    if !focus::focused_rec(ctx).is_some_and(|rec| rec.stop.keys.edits.contains(command)) {
        return false;
    }
    if command == EditCommand::Paste {
        crate::runtime::egui::clipboard::request_paste(ctx);
    } else {
        with(ctx, viewport, |stage| stage.queued_edits.push(command));
        crate::runtime::egui::repaint::request_of(ctx, viewport);
    }
    true
}

/// The text of the paste the focused component `id` takes this pass.
pub(crate) fn pasted_text(ctx: &Context, id: Id) -> Option<Arc<str>> {
    current(ctx, |stage| stage.pasted.as_ref().filter(|(owner, _)| *owner == id).map(|(_, text)| Arc::clone(text)))
}

/// The collection that takes the list keys of the focused stop: the collection of an open `KeepAnchor` dropdown whose
/// opener has the key focus, or the collection a text field drives.
#[derive(Clone, Copy)]
struct Driven {
    /// The dropdown layer the keys belong to, which Escape closes; `None` for a field outside any dropdown layer.
    layer: Option<Id>,
    /// The collection that takes the keys.
    list: Id,
    /// The focused stop is a text opener: its typing and caret keys stay with it, and Tab goes to the list.
    text: bool,
    /// Home and End go to the list; a text stop keeps them for its caret while it holds text.
    ends: bool,
    /// Left and Right go to the list too (a tree): a text stop keeps them for its caret while it holds text.
    tree: bool,
    /// Enter goes to the list; off, a text opener keeps it (its layer's `FocusPolicy::list_enter`).
    enter: bool,
}

/// The collection a text stop inside an open `KeepAnchor` dropdown layer (a search field) or a text field that drives
/// one hands its list keys to. The layer is the innermost `KeepAnchor` scope above the stop; without a driven
/// collection the list is found as for the layer's opener, and Home and End go to it. Typed text stays with the
/// field, so the result's `text` is false.
fn list_of_field(ctx: &Context, rec: &StopRec) -> Option<Driven> {
    if !matches!(rec.stop.keys.owner, OwnerClass::Text) {
        return None;
    }
    let layer = focus::scope_chain(ctx, rec).into_iter().find_map(|scope| {
        focus::with(ctx, |state| {
            let record = state.last_scopes.get(&scope)?;
            (record.policy.initial == focus::Initial::KeepAnchor).then_some((scope, record.opener?))
        })
    });
    match (focus::registration::driven(ctx, rec.stop.id), layer) {
        (Some(drive), layer) => Some(Driven {
            layer: layer.map(|(layer, _)| layer),
            list: drive.list,
            text: false,
            ends: drive.empty,
            tree: drive.empty && focus::registration::is_tree(ctx, drive.list),
            enter: true,
        }),
        (None, Some((_, opener))) => {
            let (layer, list, _) = focus::registration::dropdown_target(ctx, opener)?;
            Some(Driven { layer: Some(layer), list, text: false, ends: true, tree: false, enter: true })
        }
        (None, None) => None,
    }
}

/// The keys an opener hands to its dropdown's list. Left, Right and Space stay with the opener.
const LIST_KEYS: [Key; 8] = [Key::Up, Key::Down, Key::PageUp, Key::PageDown, Key::Home, Key::End, Key::Enter, Key::Tab];

impl Driven {
    fn of(ctx: &Context, opener: &StopRec) -> Option<Driven> {
        let (layer, list, policy) = focus::registration::dropdown_target(ctx, opener.stop.key())?;
        let text = matches!(opener.stop.keys.owner, OwnerClass::Text);
        Some(Driven { layer: Some(layer), list, text, ends: !text, tree: false, enter: !text || policy.list_enter })
    }

    /// The intent a claimed chord of the opener becomes for the list.
    fn intent_of(self, chord: &Chord) -> Option<Intent> {
        chord.mods.is_none().then(|| self.intent(chord.key)).flatten()
    }

    /// The list intent of a plain key the focused stop hands on: vertical moves and Page keys, Home and End where
    /// `ends` says, Left and Right of a tree, and Enter as the activation where `enter` says. A text opener's Tab accepts the lead too:
    /// there Tab is not the way to the next stop while the list is open.
    fn intent(self, key: Key) -> Option<Intent> {
        let moved = |mv| Some(Intent::Move(mv, MoveMods::PLAIN));
        match key {
            Key::Up => moved(NavMove::Up),
            Key::Down => moved(NavMove::Down),
            Key::PageUp => moved(NavMove::PageUp),
            Key::PageDown => moved(NavMove::PageDown),
            Key::Home if self.ends => moved(NavMove::Start),
            Key::End if self.ends => moved(NavMove::End),
            Key::Left if self.tree => moved(NavMove::Back),
            Key::Right if self.tree => moved(NavMove::Forward),
            Key::Enter if self.enter => Some(Intent::Activate),
            Key::Tab if self.text => Some(Intent::Activate),
            _ => None,
        }
    }
}

/// Where one key press of the focused component goes, as `press_route` decides it; `apply_press` acts on it.
enum PressRoute {
    /// Nothing takes it: the press is a shortcut candidate.
    Free,
    /// The intent of a list key, for the focused stop's `Driven` collection.
    List(Id, Intent),
    /// The press as a chord, for a recorder.
    Chord,
    /// A move: out of a row to its collection, along the roving group around the stop, or to the stop itself.
    Move(NavMove, MoveMods),
    /// The stop's activation.
    Activate,
    /// A close of the dropdown layer whose opener has the focus.
    Close(Id),
    /// An intent of the stop.
    Intent(Intent),
    /// A nudge of the stop's content.
    Nudge((Vec2, Mods)),
}

/// Where one key press of the focused component goes: the table's row, else the activation keys, else a nudge. The
/// list keys of the focused stop's `Driven` collection go to it first. Reads state only, so a query can ask it.
fn press_route(ctx: &Context, os: Os, rec: &StopRec, press: &Press, driven: Option<Driven>) -> PressRoute {
    let stop = &rec.stop;
    let plain = press.mods.is_none();
    if let Some(driven) = driven
        && plain
        && let Some(intent) = driven.intent(press.key)
    {
        return PressRoute::List(driven.list, intent);
    }
    if matches!(stop.keys.owner, OwnerClass::Recorder) {
        // A recorder reads every press as a chord; no intent of the key table applies to it.
        return PressRoute::Chord;
    }
    let chord = Chord { mods: press.mods, key: press.key };
    match keys::intent(os, press.key, press.mods, stop.role) {
        Some(Intent::Move(mv, mods)) => {
            // Up, Down, Page, Home and End leave a row's children: the collection takes the move. A component stop keeps
            // its declared arrows from the roving group around it; a part stop belongs to the component's own group.
            let leaves_row = is_vertical(mv) && focus::is_row_child(ctx, stop);
            let kept = stop.part.is_none() && stop.keys.keep.keeps_arrow(mv);
            // A stop that does not act on its moves leaves a move key a scoped command around it binds to that
            // command, ahead of its roving group and the scroll owner.
            if !leaves_row && !kept && !stop.role.moves && commands::binds_scoped(ctx, &chord) {
                PressRoute::Free
            } else {
                PressRoute::Move(mv, mods)
            }
        }
        Some(Intent::Activate) => PressRoute::Activate,
        // A collection that activates on Enter opens its lead with any modifier held (in a new tab or window, say); the
        // component reads the modifiers. Fields and dialogs keep SubmitForce.
        // A chord that a scoped command of the focused chain binds (Cmd+Enter of a panel's Load as Selection) is that
        // command's, not the stop's activation.
        Some(Intent::SubmitForce) if press.key == Key::Enter && stop.role.space && stop.keys.activate.enter() => {
            if commands::binds_scoped(ctx, &chord) { PressRoute::Free } else { PressRoute::Activate }
        }
        Some(Intent::Close) => match driven {
            Some(Driven { layer: Some(layer), .. }) => PressRoute::Close(layer),
            _ => PressRoute::Intent(Intent::Close),
        },
        Some(intent) => PressRoute::Intent(intent),
        // Enter the key table leaves unbound activates a stop that takes Enter with any modifier held; the component
        // reads the modifiers (Alt+Enter adds the way Alt+click does); a chord a scoped command binds is that command's,
        // not an activation. Enter elsewhere submits with Alt and Shift held or not, which the component reads (a
        // polygonal lasso closes straight on Alt+Enter); Enter chords with Cmd or Ctrl the table leaves unbound are the
        // app's.
        None if press.key == Key::Enter && stop.keys.activate.enter() && (plain || !commands::binds_scoped(ctx, &chord)) => PressRoute::Activate,
        None if press.key == Key::Enter && !press.mods.cmd && !press.mods.ctrl => PressRoute::Intent(Intent::Submit),
        None if press.key == Key::Space && plain && stop.keys.activate.space() => PressRoute::Activate,
        None if press.key == Key::Tab && plain && stop.keys.keep == focus::KeepKeys::Tab => PressRoute::Intent(Intent::Tab),
        None if stop.role.nudge => nudge_of(press.key, press.mods).map_or(PressRoute::Free, PressRoute::Nudge),
        None => PressRoute::Free,
    }
}

fn is_vertical(mv: NavMove) -> bool {
    matches!(mv, NavMove::Up | NavMove::Down | NavMove::PageUp | NavMove::PageDown | NavMove::Start | NavMove::End)
}

/// Acts on the route `press_route` gave `press` of the focused stop `rec`. Returns whether the component's intents
/// took the press.
fn apply_press(
    ctx: &Context,
    rec: &StopRec,
    press: &Press,
    route: PressRoute,
    routed: &mut HashMap<Option<Id>, Routed>,
    activated: &mut HashMap<Id, Id>,
) -> Taken {
    let stop = &rec.stop;
    let id = Some(stop.id);
    match route {
        PressRoute::Free => return Taken::No,
        PressRoute::List(list, intent) => push(routed, Some(list), intent, false),
        // Repeats are no chords.
        PressRoute::Chord if press.repeat => {}
        PressRoute::Chord => routed.entry(id).or_default().chords.push(Chord { mods: press.mods, key: press.key }),
        PressRoute::Move(mv, mods) => {
            let horizontal = matches!(press.key, Key::Left | Key::Right);
            let kept = stop.part.is_none() && stop.keys.keep.keeps_arrow(mv);
            if let Some(owner) = is_vertical(mv).then(|| focus::leave_row(ctx, stop.key())).flatten() {
                push(routed, Some(owner), Intent::Move(mv, mods), horizontal);
            } else if kept || focus::roving_move(ctx, stop.key(), mv, mods, horizontal).is_none() {
                // Page, Home and End go to the scroll owner around the stop.
                push(routed, id, Intent::Move(mv, mods), horizontal);
            }
        }
        PressRoute::Activate => route_activate(ctx, rec, routed, activated),
        PressRoute::Close(layer) => overlay::request_close(ctx, layer, overlay::CloseReason::Requested),
        PressRoute::Intent(intent) => push(routed, id, intent, false),
        PressRoute::Nudge(nudge) => routed.entry(id).or_default().nudge = Some(nudge),
    }
    Taken::Yes
}

/// `Activate` of a group member goes to the group's root, which reads `activated_member`. A child or part of a
/// collection row activates itself.
fn route_activate(ctx: &Context, rec: &StopRec, routed: &mut HashMap<Option<Id>, Routed>, activated: &mut HashMap<Id, Id>) {
    let target = if rec.stop.group.is_some() && !focus::is_row_child(ctx, &rec.stop) {
        activated.insert(rec.stop.root, rec.stop.id);
        rec.stop.root
    } else {
        rec.stop.id
    };
    push(routed, Some(target), Intent::Activate, false);
}

/// An arrow key as a unit direction with the modifiers held.
fn nudge_of(key: Key, mods: Mods) -> Option<(Vec2, Mods)> {
    let direction = match key {
        Key::Left => -Vec2::X,
        Key::Right => Vec2::X,
        Key::Up => -Vec2::Y,
        Key::Down => Vec2::Y,
        _ => return None,
    };
    Some((direction, mods))
}

/// Routes an AccessKit action request to the owner of its target node, whatever the focus (9.2 rule 2).
fn route_access(
    ctx: &Context,
    request: &Action,
    nodes: &HashMap<u64, Node>,
    routed: &mut HashMap<Option<Id>, Routed>,
    activated: &mut HashMap<Id, Id>,
) {
    let Some(node) = nodes.get(&request.node()).copied() else { return };
    let owner = Some(node.owner);
    if node.focusable {
        let target = node.part.map_or(focus::FocusTarget::Id(node.owner), |part| focus::FocusTarget::Part(node.owner, part));
        focus::request_now(ctx, target);
    }
    match request.kind() {
        ActionKind::Focus if node.focusable => {}
        ActionKind::Expand | ActionKind::Collapse if node.expanded.is_some() => {
            if node.expanded == Some(matches!(request.kind(), ActionKind::Collapse)) {
                // A sub-node with no stop of its own (a chevron) names its part: the component decides what it opens.
                if node.part.is_some() && !node.focusable {
                    push(routed, owner, Intent::Access(request.request(node.part)), false);
                    return;
                }
                match focus::stop_rec(ctx, (node.owner, node.part)).filter(|rec| rec.stop.group.is_some()) {
                    Some(rec) => route_activate(ctx, &rec, routed, activated),
                    None => push(routed, owner, Intent::Activate, false),
                }
            }
        }
        ActionKind::Click if node.part.is_some() && !node.focusable => push(routed, owner, Intent::Access(request.request(node.part)), false),
        ActionKind::Click => match focus::stop_rec(ctx, (node.owner, node.part)).filter(|rec| rec.stop.group.is_some()) {
            Some(rec) => route_activate(ctx, &rec, routed, activated),
            None => push(routed, owner, Intent::Activate, false),
        },
        ActionKind::Increment | ActionKind::Decrement if node.part.is_some() && !node.focusable => {
            push(routed, owner, Intent::Access(request.request(node.part)), false)
        }
        ActionKind::Increment => push(routed, owner, Intent::Move(NavMove::Forward, MoveMods::PLAIN), false),
        ActionKind::Decrement => push(routed, owner, Intent::Move(NavMove::Back, MoveMods::PLAIN), false),
        ActionKind::ShowContextMenu => push(routed, owner, Intent::ContextMenu, false),
        ActionKind::SetValue(value) => routed.entry(owner).or_default().set_value = Some((node.part, value)),
        ActionKind::Focus | ActionKind::Expand | ActionKind::Collapse | ActionKind::Other => {
            push(routed, owner, Intent::Access(request.request(node.part)), false)
        }
    }
}

/// The member whose `Activate` the group `root` reads this pass.
pub(crate) fn activated_member(ctx: &Context, root: Id) -> Option<Id> {
    current(ctx, |stage| stage.activated.get(&root).copied())
}
