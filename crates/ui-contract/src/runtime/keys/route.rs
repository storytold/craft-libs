//! The begin hook's routing: key presses, shortcuts and AccessKit requests to the intents of their owners.

use super::input::chrome_toggle_of;
use super::{Batch, Item, Node, Press, Routed, Shortcut, Shortcuts, Taken, current, with};
use crate::runtime::commands::{self, FirePhase};
use crate::runtime::egui::input::{Action, ActionKind};
use crate::runtime::focus::{self, StopRec};
use crate::runtime::frame::request_chrome_toggle;
use crate::runtime::overlay;
use craft_behavior::focus::key_owner;
use craft_behavior::keys;
use craft_behavior::{EditCommand, Intent, KeyOwner, MoveMods, NavMove, OwnerClass};
use craft_host::target::Os;
use craft_keymap::{Chord, Key, Layout, Mods};
use egui::{Context, Id, Vec2};
use std::collections::HashMap;
use std::sync::Arc;

/// The begin hook's intents: the pass's key events, AccessKit requests and claimed chords, routed to the
/// focused component (or to the component that owns a requested node).
pub(crate) fn begin(ctx: &Context, press_captured: bool) -> bool {
    let os = crate::runtime::windows::os_of(ctx);
    let target = focus::focused_rec(ctx);
    let (batch, nodes, captures, activate) = current(ctx, |stage| {
        stage.routed.clear();
        stage.contexts.clear();
        stage.claimed.clear();
        stage.activated.clear();
        stage.claims.clear();
        stage.last_open_edits = std::mem::take(&mut stage.open_edits);
        stage.last_nodes = std::mem::take(&mut stage.nodes);
        (std::mem::take(&mut stage.batch), stage.last_nodes.clone(), std::mem::take(&mut stage.captures), std::mem::take(&mut stage.activate))
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
    if let Some(rec) = &target {
        for (press, taken) in batch.presses.iter().zip(&mut consumed) {
            *taken = if stale_repeat(&mut stale, press) {
                Taken::Yes
            } else {
                route_press(ctx, os, rec, press, driven.or(field_list), &mut routed, &mut activated)
            };
        }
        for text in &batch.texts {
            let digits;
            let text = if rec.stop.role.digits {
                let (found, rest) = craft_behavior::keys::split_digits(text);
                for digit in found {
                    push(&mut routed, Some(rec.stop.id), Intent::Digit(digit), false);
                }
                digits = rest;
                &digits
            } else {
                text
            };
            if text.is_empty() {
                continue;
            }
            let typed = rec.stop.role.type_ahead && !text.trim().is_empty();
            let to_list = driven.filter(|driven| !driven.text && !text.trim().is_empty());
            if let Some(driven) = to_list {
                push(&mut routed, Some(driven.list), Intent::Text(Arc::from(text.as_str())), false);
            } else if matches!(rec.stop.keys.owner, OwnerClass::Text) || typed {
                routed.entry(Some(rec.stop.id)).or_default().items.push(Item { intent: Intent::Text(Arc::from(text.as_str())), horizontal: false });
            } else if let Some(menu) = focus::menu_scope(ctx, rec).filter(|_| !text.trim().is_empty()) {
                push(&mut routed, Some(menu), Intent::Text(Arc::from(text.as_str())), false);
            }
        }
    } else {
        // With nothing focused, a plain Enter submits to the window, as an unfocused Escape cancels: a tool's pending
        // edit or a dialog's default button takes it.
        for (press, taken) in batch.presses.iter().zip(&mut consumed) {
            if press.key == Key::Enter && press.mods.is_none() && !press.repeat {
                push(&mut routed, None, Intent::Submit, false);
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
    let shortcuts = owned_shortcuts(ctx, target.as_ref(), &batch, &consumed);
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
    for id in &activate {
        if let Some(rec) = focus::stop_rec(ctx, (*id, None)) {
            route_activate(ctx, &rec, &mut routed, &mut activated);
        }
    }
    current(ctx, |stage| {
        stage.routed = routed;
        stage.claimed = claimed;
        stage.activated = activated;
        stage.menu = batch.menu;
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
    escapes.cancels_press
}

pub(super) fn push(routed: &mut HashMap<Option<Id>, Routed>, target: Option<Id>, intent: Intent, horizontal: bool) {
    routed.entry(target).or_default().items.push(Item { intent, horizontal });
}

/// The shortcut events the key owner leaves to the app (9.3). The owner is the focused component's class, or
/// `Blocked` while a modal layer is stacked above the focus; inside a layer its `Overlay.keys` class ranks with
/// the component's. `Menu` and `Recorder` keep every key; a `Widget` or popup keeps the keys
/// it activates with, the keys of its intents and the keys its filter keeps from the key stage; a text field
/// keeps the text its presses typed.
fn owned_shortcuts(ctx: &Context, target: Option<&StopRec>, batch: &Batch, consumed: &[Taken]) -> Shortcuts {
    let layer = overlay::key_layer(ctx, &target.map(|rec| focus::scope_chain(ctx, rec)).unwrap_or_default());
    let own = target.map(|rec| rec.stop.keys.owner);
    let mut present: Vec<OwnerClass> = own.into_iter().collect();
    if let overlay::LayerKeys::Class(class) = layer {
        present.push(class);
    }
    let owner = key_owner(layer == overlay::LayerKeys::Blocked, &present);
    let kept = |keys: &focus::KeyUse, at: usize| {
        let key = batch.presses[at].key;
        match keys.keep {
            focus::KeepKeys::Tab => key == Key::Tab,
            focus::KeepKeys::InlineArrows => matches!(key, Key::Left | Key::Right),
            focus::KeepKeys::BlockArrows => matches!(key, Key::Left | Key::Right | Key::Up | Key::Down),
            _ => false,
        }
    };
    let held_by = |keys: Option<&focus::KeyUse>, at: usize| {
        let Some(keys) = keys else { return false };
        consumed[at] == Taken::Yes || kept(keys, at)
    };
    // A move the widget did not declare it keeps (`KeyUse::keep`) stays a candidate for the scoped commands.
    let only_moved = |keys: Option<&focus::KeyUse>, at: usize| consumed[at] == Taken::Moved && keys.is_some_and(|keys| !kept(keys, at));
    let silent = matches!(owner, KeyOwner::Class(OwnerClass::Menu | OwnerClass::Recorder));
    let dialog = owner == KeyOwner::Class(OwnerClass::Dialog);
    // The focused component keeps its own keys inside a dialog: a text field its typing, a widget its activation.
    let blocked = owner == KeyOwner::Blocked;
    let typing = !blocked && own == Some(OwnerClass::Text);
    let widget = !blocked && matches!(own, Some(OwnerClass::Widget | OwnerClass::Popup | OwnerClass::PassivePopup));
    let keys = if silent {
        Vec::new()
    } else {
        let owner_keys = target.map(|rec| &rec.stop.keys);
        batch
            .shortcuts
            .iter()
            .filter(|shortcut| !(typing && shortcut.typed))
            .filter(|shortcut| !(widget && shortcut.press.is_some_and(|at| held_by(owner_keys, at))))
            .map(|shortcut| Shortcut { moved: widget && shortcut.press.is_some_and(|at| only_moved(owner_keys, at)), ..*shortcut })
            .collect()
    };
    Shortcuts { keys, typing, dialog, layout: Layout::default() }
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
/// Activates the control `id` in the next pass, as Space on it does: a group member goes through its group's root. A
/// click on a label that names the control queues it.
pub(crate) fn queue_activate(ctx: &Context, id: Id) {
    current(ctx, |stage| stage.activate.push(id));
    crate::runtime::frame::request_repaint(ctx);
}

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
        }),
        (None, Some((_, opener))) => {
            let (layer, list) = focus::registration::dropdown_target(ctx, opener)?;
            Some(Driven { layer: Some(layer), list, text: false, ends: true, tree: false })
        }
        (None, None) => None,
    }
}

/// The keys an opener hands to its dropdown's list. Left, Right and Space stay with the opener.
const LIST_KEYS: [Key; 8] = [Key::Up, Key::Down, Key::PageUp, Key::PageDown, Key::Home, Key::End, Key::Enter, Key::Tab];

impl Driven {
    fn of(ctx: &Context, opener: &StopRec) -> Option<Driven> {
        let (layer, list) = focus::registration::dropdown_target(ctx, opener.stop.key())?;
        let text = matches!(opener.stop.keys.owner, OwnerClass::Text);
        Some(Driven { layer: Some(layer), list, text, ends: !text, tree: false })
    }

    /// The intent a claimed chord of the opener becomes for the list.
    fn intent_of(self, chord: &Chord) -> Option<Intent> {
        chord.mods.is_none().then(|| self.intent(chord.key)).flatten()
    }

    /// The list intent of a plain key the focused stop hands on: vertical moves and Page keys, Home and End where
    /// `ends` says, Left and Right of a tree, and Enter as the activation. A text opener's Tab accepts the lead too:
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
            Key::Enter => Some(Intent::Activate),
            Key::Tab if self.text => Some(Intent::Activate),
            _ => None,
        }
    }
}

/// One key press of the focused component: the table's row, else the activation keys, else a nudge. The list keys
/// of the focused stop's `Driven` collection go to it first. Returns whether the component's
/// intents took the press.
fn route_press(
    ctx: &Context,
    os: Os,
    rec: &StopRec,
    press: &Press,
    driven: Option<Driven>,
    routed: &mut HashMap<Option<Id>, Routed>,
    activated: &mut HashMap<Id, Id>,
) -> Taken {
    let stop = &rec.stop;
    let id = Some(stop.id);
    let horizontal = matches!(press.key, Key::Left | Key::Right);
    let plain = press.mods.is_none();
    if let Some(driven) = driven
        && plain
        && let Some(intent) = driven.intent(press.key)
    {
        push(routed, Some(driven.list), intent, false);
        return Taken::Yes;
    }
    if matches!(stop.keys.owner, OwnerClass::Recorder) {
        // A recorder reads every press as a chord, repeats excluded; no intent of the key table applies to it.
        if !press.repeat {
            routed.entry(id).or_default().chords.push(Chord { mods: press.mods, key: press.key });
        }
        return Taken::Yes;
    }
    let mut taken = Taken::Yes;
    match keys::intent(os, press.key, press.mods, stop.role) {
        Some(Intent::Move(mv, mods)) => {
            // Up, Down, Page, Home and End leave a row's children: the collection takes the move. A component stop keeps
            // its declared arrows from the roving group around it; a part stop belongs to the component's own group.
            let vertical = matches!(mv, NavMove::Up | NavMove::Down | NavMove::PageUp | NavMove::PageDown | NavMove::Start | NavMove::End);
            if let Some(owner) = vertical.then(|| focus::leave_row(ctx, rec.stop.key())).flatten() {
                push(routed, Some(owner), Intent::Move(mv, mods), horizontal);
            } else if (stop.part.is_none() && stop.keys.keep.keeps_arrow(mv))
                || focus::roving_move(ctx, rec.stop.key(), mv, mods, horizontal).is_none()
            {
                push(routed, id, Intent::Move(mv, mods), horizontal);
                // Page, Home and End go to the scroll owner around the stop; a stop that acts on its moves keeps its arrows.
                if !stop.role.moves && matches!(press.key, Key::Left | Key::Right | Key::Up | Key::Down) {
                    taken = Taken::Moved;
                }
            }
        }
        Some(Intent::Activate) => route_activate(ctx, rec, routed, activated),
        // A collection that activates on Enter opens its lead with any modifier held (in a new tab or window, say); the
        // component reads the modifiers. Fields and dialogs keep SubmitForce.
        // A chord that a scoped command of the focused chain binds (Cmd+Enter of a panel's Load as Selection) is that
        // command's, not the stop's activation.
        Some(Intent::SubmitForce)
            if press.key == Key::Enter
                && stop.role.space
                && stop.keys.activate.enter()
                && commands::binds_scoped(ctx, &Chord { mods: press.mods, key: press.key }) =>
        {
            return Taken::No;
        }
        Some(Intent::SubmitForce) if press.key == Key::Enter && stop.role.space && stop.keys.activate.enter() => {
            route_activate(ctx, rec, routed, activated)
        }
        Some(Intent::Close) => match driven {
            Some(Driven { layer: Some(layer), .. }) => overlay::request_close(ctx, layer, overlay::CloseReason::Requested),
            _ => push(routed, id, Intent::Close, false),
        },
        Some(intent) => push(routed, id, intent, false),
        // Enter the key table leaves unbound activates a stop that takes Enter with any modifier held; the component
        // reads the modifiers (Alt+Enter adds the way Alt+click does); a chord a scoped command binds is that command's,
        // not an activation. Enter elsewhere submits with Alt and Shift held or not, which the component reads (a
        // polygonal lasso closes straight on Alt+Enter); Enter chords with Cmd or Ctrl the table leaves unbound are the
        // app's.
        None if press.key == Key::Enter
            && stop.keys.activate.enter()
            && (plain || !commands::binds_scoped(ctx, &Chord { mods: press.mods, key: press.key })) =>
        {
            route_activate(ctx, rec, routed, activated)
        }
        None if press.key == Key::Enter && !press.mods.cmd && !press.mods.ctrl => push(routed, id, Intent::Submit, false),
        None if press.key == Key::Space && plain && stop.keys.activate.space() => route_activate(ctx, rec, routed, activated),
        None if press.key == Key::Tab && plain && stop.keys.keep == focus::KeepKeys::Tab => push(routed, id, Intent::Tab, false),
        None if stop.role.nudge => {
            let Some(nudge) = nudge_of(press.key, press.mods) else { return Taken::No };
            routed.entry(id).or_default().nudge = Some(nudge);
        }
        None => return Taken::No,
    }
    taken
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
