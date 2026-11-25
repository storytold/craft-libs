//! The begin hook's routing: key presses, shortcuts and AccessKit requests to the intents of their owners.

use super::input::chrome_toggle_of;
use super::{Batch, EscapePress, Item, MenuKey, Node, Nudge, Press, Routed, Shortcuts, Source, Taken, current, with};
use crate::runtime::commands::{self, ClipContent, FirePhase};
use crate::runtime::egui::input::{Action, ActionKind};
use crate::runtime::focus::{self, StopRec};
use crate::runtime::frame::request_chrome_toggle;
use crate::runtime::overlay;
use craft_behavior::focus::{EscapeRouting, key_owner};
use craft_behavior::keys;
use craft_behavior::{EditCommand, Intent, KeyOwner, MoveMods, NavMove, OwnerClass};
use craft_host::target::Os;
use craft_keymap::{Chord, Key, Layout, Mods};
use egui::{Context, Id, IdMap, Vec2};
use std::collections::HashMap;
use std::sync::Arc;

/// The begin hook's intents: the pass's key events, AccessKit requests and claimed chords, routed to the
/// focused component (or to the component that owns a requested node). Returns how the pass's Escapes split
/// between the captured press and the components.
pub(crate) fn begin(ctx: &Context, press_captured: bool) -> EscapeRouting {
    let os = crate::runtime::windows::os_of(ctx);
    let target = focus::focused_rec(ctx);
    let (batch, nodes, captures, held_claims, alt_claims, fired_claims, hover_deletes) = current(ctx, |stage| {
        stage.routed.clear();
        stage.intents_read = false;
        stage.contexts.clear();
        stage.claimed.clear();
        stage.activated.clear();
        let held_claims = std::mem::take(&mut stage.claims);
        stage.last_claims.clone_from(&held_claims);
        stage.last_open_edits = std::mem::take(&mut stage.open_edits);
        stage.last_nodes = std::mem::take(&mut stage.nodes);
        let alt_claims = std::mem::take(&mut stage.alt_claims);
        let fired = std::mem::take(&mut stage.fired_claims);
        (
            std::mem::take(&mut stage.batch),
            stage.last_nodes.clone(),
            std::mem::take(&mut stage.captures),
            held_claims,
            alt_claims,
            fired,
            std::mem::take(&mut stage.hover_deletes),
        )
    });
    let alt_tapped = alt_claims.first().copied().filter(|_| batch.alt_tap);
    let modal = overlay::any_open(ctx, overlay::OpenFilter::Modal);
    let hover_delete = if !modal
        && !ctx.text_edit_focused()
        && target.as_ref().is_none_or(|rec| !rec.stop.text && !matches!(rec.stop.keys.owner, OwnerClass::Text | OwnerClass::Recorder))
    {
        crate::runtime::pointer::with_hover(ctx, crate::runtime::egui::pass::current_viewport(ctx), |under, captured| {
            if captured {
                return None;
            }
            let under = under?;
            let owner = under.press.click.or(under.press.drag).or(under.hover_top).map(|(_, owner)| owner)?;
            hover_deletes.contains(&owner).then_some(owner)
        })
    } else {
        None
    };
    let canvas = if target.is_some() { None } else { unfocused_canvas(ctx) };
    // A key down when a modal layer opens or closes (the field's Enter that raised an alert, and its repeats) answers
    // nothing in either layer: it is held over until its release or a fresh press. `down` already holds this pass's
    // presses, so a key pressed fresh in this pass is not held over.
    let held_over = current(ctx, |stage| {
        let fresh = |key: Key| {
            batch.presses.iter().any(|press| press.release == Some(key) && !press.repeat)
                || (key == Key::Escape && batch.escapes.iter().any(|escape| !escape.repeat))
        };
        if modal != stage.modal_open {
            stage.held_over = stage.down.clone();
        }
        stage.held_over.retain(|key| stage.down.contains(key) && !fresh(*key));
        stage.held_over.clone()
    });
    let held_press = |press: &Press| press.repeat && press.release.is_some_and(|key| held_over.contains(&key));
    let mut routed: HashMap<Option<Id>, Routed> = HashMap::new();
    let mut activated = IdMap::default();
    let mut consumed = vec![Taken::No; batch.presses.len()];
    let driven = target.as_ref().and_then(|rec| Driven::of(ctx, rec));
    let field_list = target.as_ref().filter(|_| driven.is_none()).and_then(|rec| list_of_field(ctx, rec));
    // Whether a component took each text of the pass, as typing, type-ahead or digits; and whether the framework took
    // it from egui's input, so no egui text widget reads it in this pass (`withdraw_taken`).
    let mut taken_texts = vec![false; batch.texts.len()];
    // A held-over repeat's text is withdrawn as a claim's is: no field types it.
    let mut withdrawn_texts: Vec<usize> = batch
        .texts
        .iter()
        .enumerate()
        .filter(|(_, typed)| typed.claimed)
        .map(|(at, _)| at)
        .chain(batch.presses.iter().filter(|press| held_press(press)).filter_map(|press| press.text))
        .collect();
    if let Some(rec) = &target {
        for (press, taken) in batch.presses.iter().zip(&mut consumed).filter(|(press, _)| !press.canvas) {
            *taken = if let Some(id) = hover_delete.filter(|id| Some(*id) != target.as_ref().map(|rec| rec.stop.id))
                && press.mods.is_none()
                && matches!(press.key, Key::Delete | Key::Backspace)
            {
                push(&mut routed, Some(id), Intent::delete(press.key == Key::Delete, craft_behavior::Granularity::Unit), false);
                Taken::Hover
            } else if held_press(press) {
                Taken::Yes
            } else {
                let route = press_route(ctx, os, rec, press, driven.or(field_list));
                apply_press(ctx, rec, press, route, &mut routed, &mut activated)
            };
            stamp(&mut routed, Source { place: press.place, shift: press.mods.shift });
        }
        // The focused stop reads its typing from egui's input when it is a text field (its egui text widget) or an app's
        // own widget; any other taker reads the text as an intent only, and the text leaves egui's input.
        let reads_egui = rec.stop.host || rec.stop.keys.owner == OwnerClass::Text;
        for (at, (typed, taken)) in batch.texts.iter().zip(&mut taken_texts).enumerate() {
            // A claimed text (a claim's, or an Alt shortcut's that the keymap binds) is withdrawn and is no component's
            // typing: its press is not the focused stop's to hold, so the shortcut it comes with still reaches the
            // key stage from a text field or a list, as Alt+F7 or Alt+Q does from a word processor's page.
            if typed.claimed {
                continue;
            }
            // A held-over repeat's text is withdrawn and taken with its press.
            if withdrawn_texts.contains(&at) {
                *taken = true;
                continue;
            }
            let route = text_route(rec, driven, &typed.text);
            *taken = !route.is_empty();
            if route.iter().any(|(to, _)| !(reads_egui && *to == rec.stop.id)) {
                withdrawn_texts.push(at);
            }
            for (to, intent) in route {
                push(&mut routed, Some(to), intent, false);
            }
            stamp(&mut routed, Source { place: typed.place, shift: false });
        }
    } else {
        // With nothing focused, a plain Enter submits to the window, as an unfocused Escape cancels: a tool's pending
        // edit or a dialog's default button takes it. The arrows nudge the window's canvas content (a tool or the
        // session control on a canvas), unless a modal layer is open. The window's document canvas is its key target
        // (`focus::host_canvas`): Page Up and Down, Home and End page it as they do while it has the focus. What the
        // canvas takes is its alone, as while it has the focus: the app's shortcuts do not run with it too. It takes the
        // arrows with the modifiers it acts on (`KeyUse::nudge_mods`); the others are the app's, as is a page key bound to
        // a command that runs now (`page_key_runs`).
        let declared = |mods: Mods| canvas.as_ref().is_none_or(|canvas| canvas.stop.keys.nudge_mods.contains(mods));
        for (press, taken) in batch.presses.iter().zip(&mut consumed).filter(|(press, _)| !press.canvas) {
            if let Some(id) = hover_delete
                && press.mods.is_none()
                && matches!(press.key, Key::Delete | Key::Backspace)
            {
                push(&mut routed, Some(id), Intent::delete(press.key == Key::Delete, craft_behavior::Granularity::Unit), false);
                *taken = Taken::Hover;
            } else if press.key == Key::Enter && press.mods.is_none() && !press.repeat {
                push(&mut routed, None, Intent::Submit, false);
                *taken = Taken::Yes;
            } else if !modal
                && declared(press.mods)
                && let Some(nudge) = nudge_of(press)
            {
                routed.entry(None).or_default().nudge = Some(nudge);
                *taken = if canvas.is_some() { Taken::Canvas } else { Taken::Yes };
            } else if let Some(canvas) = &canvas
                && let Some(Intent::Move(mv @ (NavMove::PageUp | NavMove::PageDown | NavMove::Start | NavMove::End), mods)) =
                    keys::intent(os, press.key, press.mods, canvas.stop.role)
                && !page_key_runs(ctx, canvas, press)
            {
                push(&mut routed, Some(canvas.stop.id), Intent::Move(mv, mods), false);
                *taken = Taken::Canvas;
            }
        }
    }
    let focused = target.as_ref().map(|rec| rec.stop.id);
    let root = target.as_ref().filter(|rec| rec.stop.root != rec.stop.id && !rec.stop.text).map(|rec| rec.stop.root);
    let record = focus::record(ctx, crate::runtime::egui::pass::current_viewport(ctx));
    let (menu, menu_bar) = (record.menu, record.menu_bar);
    // The web sends a chord's key press and its clipboard event: the command reaches the component once, in the
    // press's place among the frame's events, so an editor reads a paste in order with the keys and texts around it.
    let mut clips = batch.clips.clone();
    clips.dedup_by_key(|(command, _)| *command);
    for (command, place) in clips {
        let source = Some(Source { place, shift: false });
        routed.entry(focused).or_default().items.push(Item { intent: Intent::Edit(command), horizontal: false, source });
    }
    let queued = current(ctx, |stage| std::mem::take(&mut stage.queued_edits));
    for command in queued {
        push(&mut routed, focused, Intent::Edit(command), false);
    }
    let over: Vec<bool> = batch.presses.iter().map(held_press).collect();
    let (shortcuts, held) = owned_shortcuts(ctx, target.as_ref(), &batch, &consumed, &taken_texts, &over);
    // The app's own widget, focused or the window's canvas, reads its keys from egui.
    let host = target.as_ref().or(canvas.as_ref()).is_some_and(|rec| rec.stop.host);
    withdraw_taken(ctx, host, &batch, &held, &withdrawn_texts);
    let pasted = batch.paste.clone().zip(focused).map(|(content, id)| (id, content));
    // PLAN: a repeat Tab of a held-over key is not gated; `take_nav` reads `batch.tabs` before this hook runs (owner: focus).
    // One Escape press cancels once: its repeats cancel nothing more, in the layer it closed or in what that uncovered.
    let kept: Vec<&EscapePress> = batch.escapes.iter().filter(|escape| !escape.repeat).collect();
    let escapes = craft_behavior::focus::route_escapes(kept.len(), press_captured);
    let mut cancel_mods = None;
    // With a captured press the first kept Escape is the press's; the others are the components' cancels.
    for escape in kept.iter().skip(usize::from(escapes.cancels_press)) {
        // A row's child that claims Escape keeps it; any other returns focus to its row.
        let claims_escape = target.as_ref().is_some_and(|rec| {
            rec.stop.keys.claims.iter().any(|chord| chord.key == Key::Escape) || current(ctx, |stage| stage.last_open_edits.contains(&rec.stop.id))
        });
        if !claims_escape && target.as_ref().is_some_and(|rec| focus::leave_row(ctx, rec.stop.key()).is_some()) {
            continue;
        }
        push(&mut routed, focused, Intent::Cancel, false);
        cancel_mods = Some(escape.mods);
    }
    // The Escapes the windows opened from this one handed on are the window's. A modal that opened meanwhile blocks the
    // window they were pressed in, so they reach nothing.
    let handed = current(ctx, |stage| std::mem::take(&mut stage.handed_cancels));
    if !handed.is_empty() && overlay::app_modal_viewport(ctx).is_none() {
        for mods in handed {
            push(&mut routed, None, Intent::Cancel, false);
            cancel_mods = Some(mods);
        }
        // A view that reads what is sent (`Input::Sent`) runs for them as for a key of the window's own.
        current(ctx, |stage| stage.sent += 1);
    }
    // A text opener's list keys arrive as claimed chords, withheld from its text part: they go to the list. The chords
    // fired commands stood for go to the owners of their claims.
    let mut claimed = fired_claims;
    for chord in &batch.claimed {
        match driven.and_then(|driven| driven.intent_of(chord)) {
            Some(intent) => push(&mut routed, driven.map(|driven| driven.list), intent, false),
            None => {
                // A capture's owner, else the claim's: the focused component's own, then its arena root's, then its
                // menu level's, then the registered menu bar's (keytips hold their letters with no focus), then the
                // document canvas's with nothing focused: a state-dependent claim it holds, else one it declares
                // (`KeyUse::claims`).
                let held = |owner: Option<Id>| held_claims.iter().find(|(id, held)| Some(*id) == owner && held == chord).map(|(id, _)| *id);
                let owner = captures.iter().find(|(_, held)| held == chord).map(|(owner, _)| *owner);
                claimed.extend(
                    owner
                        .or_else(|| held(focused))
                        .or_else(|| held(root))
                        .or_else(|| held(menu))
                        .or_else(|| held(menu_bar))
                        .or_else(|| held(record.canvas))
                        .or(focused)
                        .or_else(|| canvas.as_ref().map(|canvas| canvas.stop.id))
                        .map(|id| (id, *chord)),
                );
            }
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
        stage.modal_open = modal;
        stage.escape_mods = cancel_mods;
        stage.alt_tapped = alt_tapped;
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

/// The Alt and Shift held at the press of the pass's last Escape that became a `Cancel`; `None` when the pass had none.
pub fn escape_mods(ctx: &Context) -> Option<Mods> {
    current(ctx, |stage| stage.escape_mods)
}

/// The window's document canvas as the key target of the running viewport while nothing has the focus: none while a
/// modal layer is open. It takes the window's nudges and page keys, and a fired command whose chord it claims, as it
/// does while it has the focus.
pub(super) fn unfocused_canvas(ctx: &Context) -> Option<StopRec> {
    if overlay::any_open(ctx, overlay::OpenFilter::Modal) { None } else { focus::document_canvas(ctx) }
}

/// Rewrites the indices of `mods`, which count the entries of a list, to count only the entries `kept` says stay.
fn remap_mods(mods: &[(usize, Mods)], kept: &[bool]) -> Vec<(usize, Mods)> {
    mods.iter().map(|(at, held)| (kept.get(..*at).unwrap_or(kept).iter().filter(|stays| **stays).count(), *held)).collect()
}

/// Whether `press`, a page key (Page Up or Down, Home, End) of the window's document canvas `canvas`, focused or the
/// unfocused window's key target, runs the command the window's keymap binds it to (`commands::runs_now`): that app
/// shortcut goes ahead of the canvas's page scroll (Q370), as the keymap ran it before the canvas paged. A canvas's
/// editor part keeps its page keys as its input.
fn page_key_runs(ctx: &Context, canvas: &StopRec, press: &Press) -> bool {
    let keys = &canvas.stop.keys;
    keys.chrome_key
        && !keys.is_document_editor()
        && matches!(press.key, Key::PageUp | Key::PageDown | Key::Home | Key::End)
        && commands::runs_now(ctx, crate::runtime::egui::pass::current_viewport(ctx), &Chord { mods: press.mods, key: press.key })
}

pub(super) fn push(routed: &mut HashMap<Option<Id>, Routed>, target: Option<Id>, intent: Intent, horizontal: bool) {
    routed.entry(target).or_default().items.push(Item { intent, horizontal, source: None });
}

/// Gives the intents routed so far without a source the press or text `source` they came from: routing stamps each
/// event's intents right after it pushed them, so only that event's are without one.
fn stamp(routed: &mut HashMap<Option<Id>, Routed>, source: Source) {
    for item in routed.values_mut().flat_map(|routed| routed.items.iter_mut()) {
        item.source.get_or_insert(source);
    }
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

/// A modal dialog's Cancel discards its edits together, so a field's draft does not hold Escape there. A modeless
/// window's edits belong to the app, and its field must cancel its own draft first.
pub(crate) fn focus_in_modal_dialog(ctx: &Context) -> bool {
    focus::focused_rec(ctx).is_some_and(|rec| {
        let scopes = focus::scope_chain(ctx, &rec);
        overlay::key_layer(ctx, &scopes, None) == overlay::LayerKeys::Class(OwnerClass::Dialog)
            && scopes.iter().find_map(|id| overlay::entry(ctx, *id)).is_some_and(|entry| entry.rank.is_modal())
    })
}

/// The shortcut events the key owner leaves to the app (9.3), and the presses the focused component holds
/// (`holds_press`). The owner is the focused component's class, or `Blocked` while a modal layer is stacked above the
/// focus; inside a layer its `Overlay.keys` class ranks with the component's. A `Recorder` keeps every key and a `Menu`
/// its navigation keys (`keeps_key`); a text field keeps the text its presses typed. `consumed` is what
/// routing did with each press, `taken_texts` whether a component took each text of the pass, and `over` whether each
/// press is a repeat of a key held over a modal layer's opening or closing, which runs no shortcut either.
fn owned_shortcuts(
    ctx: &Context,
    target: Option<&StopRec>,
    batch: &Batch,
    consumed: &[Taken],
    taken_texts: &[bool],
    over: &[bool],
) -> (Shortcuts, Vec<bool>) {
    let scopes = target.map(|rec| focus::scope_chain(ctx, rec)).unwrap_or_default();
    let owner = owner_at(ctx, target, &scopes, None);
    let own = target.map(|rec| rec.stop.keys.owner);
    let dialog = owner == KeyOwner::Class(OwnerClass::Dialog);
    // The focused component keeps its own keys inside a dialog: a text field its typing, a widget its activation.
    let typing = owner != KeyOwner::Blocked && own == Some(OwnerClass::Text);
    let command_keys = typing && target.is_some_and(|rec| rec.stop.keys.command_keys);
    let held: Vec<bool> = batch
        .presses
        .iter()
        .zip(consumed)
        // The canvas's Space is never the focused stop's; a press the unfocused window's canvas took is the canvas's.
        .map(|(press, taken)| {
            // Text an input method committed for the press goes to the text stop that has the IME.
            let typed = press.text.is_some_and(|at| taken_texts[at]) || (press.ime && target.is_some_and(|rec| rec.stop.text));
            !press.canvas && (matches!(*taken, Taken::Canvas | Taken::Hover) || holds_press(owner, target, press, *taken == Taken::Yes, typed))
        })
        .collect();
    // A shortcut without a press of its own (a release, or the clipboard event that stands for ⌘C) follows the owner by
    // its modifiers, as its press would.
    let kept: Vec<bool> = batch
        .shortcuts
        .iter()
        .map(|shortcut| {
            let key = shortcut.input.typed.or(shortcut.input.physical);
            !keeps_key(owner, key, shortcut.input.mods) && !shortcut.press.is_some_and(|at| held[at] || over[at])
        })
        .collect();
    let keys = batch.shortcuts.iter().zip(&kept).filter(|(_, stays)| **stays).map(|(shortcut, _)| *shortcut).collect();
    let mods = remap_mods(&batch.mods, &kept);
    (
        Shortcuts {
            keys,
            typing,
            command_keys,
            dialog,
            layout: Layout::default(),
            mods,
            focus_lost: batch.focus_lost,
            pasted: batch.app_paste.clone(),
        },
        held,
    )
}

/// Whether the focused stop `target`, whose keys `owner` owns, holds `press`: the press is its own and reaches neither
/// egui's input, where the app reads its own keys, nor the app's shortcuts. A widget or popup holds the presses its
/// routing `taken`, the keys its filter keeps from the key stage, and the presses whose text it took (`typed`: the
/// type-ahead of a list). A text field holds the presses whose text it took, its typing; the egui text widget behind it
/// reads the text, not the press, and the editing keys, which stay in egui's input. A recorder holds every press and a
/// menu its navigation keys (`keeps_key`), except that a text field in a menu (a menu's search field) keeps the text
/// field's rule: the editing chords it lists reach it as edits before routing (`Strip::press`). The letters a menu's
/// rows' mnemonics name are the menu's claims, held before routing.
fn holds_press(owner: KeyOwner, target: Option<&StopRec>, press: &Press, taken: bool, typed: bool) -> bool {
    let Some(keys) = target.map(|rec| &rec.stop.keys) else { return false };
    let text = keys.owner == OwnerClass::Text;
    if keeps_key(owner, Some(press.key), press.mods) && !text {
        return true;
    }
    let open = owner != KeyOwner::Blocked;
    let widget = open && matches!(keys.owner, OwnerClass::Widget | OwnerClass::Popup | OwnerClass::PassivePopup);
    let kept = match keys.keep {
        focus::KeepKeys::Tab => press.key == Key::Tab,
        focus::KeepKeys::InlineArrows => matches!(press.key, Key::Left | Key::Right),
        focus::KeepKeys::BlockArrows => matches!(press.key, Key::Left | Key::Right | Key::Up | Key::Down),
        _ => false,
    };
    (widget && (taken || typed || kept)) || (open && text && typed)
}

/// Whether the key owner `owner` keeps a press of `key` with `mods` held from egui's input and the app's shortcuts
/// (9.3). A `Recorder` keeps every key. A `Menu` keeps its navigation keys, Escape, Enter and the arrows, without Cmd or
/// Ctrl; its rows' mnemonic letters are its claims. Any other key reaches the app: a tool key picks its tool under an
/// open menu, and running a command from a key closes the open popups (`overlay::close_popups`), so ⌘⌥P under an open
/// menu works as it does with the menu closed.
fn keeps_key(owner: KeyOwner, key: Option<Key>, mods: Mods) -> bool {
    match owner {
        KeyOwner::Class(OwnerClass::Recorder) => true,
        KeyOwner::Class(OwnerClass::Menu) => {
            !(mods.cmd || mods.ctrl) && key.is_some_and(|key| matches!(key, Key::Escape | Key::Enter | Key::Left | Key::Right | Key::Up | Key::Down))
        }
        _ => false,
    }
}

/// Where the focused stop `rec` sends the typed `text`, and as what: a digits stop takes its digits, and the rest goes
/// to the list `driven` drives, or to a text or type-ahead stop. Empty when no component takes the text.
fn text_route(rec: &StopRec, driven: Option<Driven>, text: &str) -> Vec<(Id, Intent)> {
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
    } else {
        None
    };
    route.extend(to.map(|to| (to, Intent::Text(Arc::from(text)))));
    route
}

/// The text a press of `chord` types: a space, or the character of a letter, digit or symbol key, pressed with no Cmd,
/// Ctrl or Alt held (`Chord::is_plain`, the chords typing would trigger).
fn text_of(chord: Chord) -> Option<&'static str> {
    let key = chord.key;
    if !chord.is_plain() {
        return None;
    }
    if key == Key::Space { Some(" ") } else { (key.is_letter() || key.is_digit() || key.is_symbol()).then(|| key.name()) }
}

/// Whether the focused component of the running viewport holds a press of `chord`, as routing decides for the press:
/// it or its menu claims the chord, its routing takes the press or the text the press types (a list's type-ahead), or a
/// menu or recorder has the keys, or the chord is a bare Tab that moves focus from it rather than toggling the panels
/// (`tab_toggles_now`). No app command bound to the chord runs while the component has the focus, and the press reaches
/// egui's input only for the app's own widget (`Stop::host`), which reads its keys there.
pub fn focus_holds(ctx: &Context, chord: Chord) -> bool {
    let Some(rec) = focus::focused_rec(ctx) else { return false };
    let viewport = crate::runtime::egui::pass::current_viewport(ctx);
    let record = focus::record(ctx, viewport);
    if current(ctx, |stage| super::input::focused_claims(&record, stage)).contains(&chord) {
        return true;
    }
    let press = Press {
        key: chord.key,
        mods: chord.mods,
        repeat: false,
        in_egui: None,
        release: None,
        text: None,
        canvas: false,
        place: 0,
        held: Vec::new(),
        ime: false,
    };
    let driven = Driven::of(ctx, &rec);
    let route = press_route(ctx, crate::runtime::windows::os_of(ctx), &rec, &press, driven.or_else(|| list_of_field(ctx, &rec)));
    let taken = !matches!(route, PressRoute::Free);
    let typed = text_of(chord).is_some_and(|text| !text_route(&rec, driven, text).is_empty());
    let owner = owner_at(ctx, Some(&rec), &focus::scope_chain(ctx, &rec), None);
    let bare_tab = chord.key == Key::Tab && !chord.mods.cmd && !chord.mods.ctrl && !chord.mods.alt;
    let traverses = bare_tab && !(super::input::tab_toggles_now(ctx) && super::input::is_chrome_chord(ctx, &chord));
    holds_press(owner, Some(&rec), &press, taken, typed) || traverses
}

/// Takes what the framework handed to one taker out of the running pass's egui input, where the app and egui's text
/// widgets read their keys: one press, one action, and one text, one reader. `held` marks the presses the focused
/// component (or the unfocused window's canvas) holds; the app's own widget (`Stop::host`, `host`) reads its keys from
/// egui, so its presses stay. `texts` are the places in `Batch::texts` of the texts a claim or a framework taker took.
/// A held press whose release has not come yet leaves its release to the strip.
fn withdraw_taken(ctx: &Context, host: bool, batch: &Batch, held: &[bool], texts: &[usize]) {
    let presses: Vec<&Press> =
        batch.presses.iter().zip(held).filter(|(press, held)| !host && **held && press.in_egui.is_some()).map(|(press, _)| press).collect();
    if presses.is_empty() && texts.is_empty() {
        return;
    }
    let places: Vec<usize> = presses.iter().filter_map(|press| press.in_egui).collect();
    let text_places: Vec<usize> = texts.iter().filter_map(|at| batch.texts.get(*at).and_then(|typed| typed.in_egui)).collect();
    let open = crate::runtime::egui::input::withdraw(ctx, &places, &text_places);
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
    let kept: Vec<bool> = shortcuts
        .keys
        .iter()
        .map(|shortcut| {
            let toggle = (shortcut.phase == FirePhase::Press).then(|| chrome_toggle_of(ctx, &shortcut.input)).flatten();
            toggle.inspect(|toggle| request_chrome_toggle(ctx, viewport, *toggle)).is_none()
        })
        .collect();
    shortcuts.mods = remap_mods(&shortcuts.mods, &kept);
    let mut stays = kept.iter().copied();
    shortcuts.keys.retain(|_| stays.next().unwrap_or(true));
    shortcuts
}

/// Asks the focused component of the running viewport to take `command` as `Intent::Edit`: in this pass when no
/// component has read its intents in it yet (a system menu pick the host dispatches before the UI), else in the next
/// pass (a pick in the window's own menus or bars, which run in the UI).
/// `false` when it does not list the command, or is the app's own widget (`Stop::host`), whose edits are the app's. A paste asks the platform for the clipboard text, which arrives as
/// the paste event that becomes the intent. Focus is read as it is now: a menu pick returns focus in its pass
/// before its command fires, and the command goes where focus returned.
pub(crate) fn queue_edit(ctx: &Context, command: EditCommand) -> bool {
    let viewport = crate::runtime::egui::pass::current_viewport(ctx);
    let Some(id) = focus::focused_rec(ctx).filter(|rec| !rec.stop.host && rec.stop.keys.edits.contains(command)).map(|rec| rec.stop.id) else {
        return false;
    };
    if command == EditCommand::Paste {
        crate::runtime::egui::clipboard::request_paste(ctx);
    } else if with(ctx, viewport, |stage| {
        if !stage.intents_read {
            push(&mut stage.routed, Some(id), Intent::Edit(command), false);
        } else {
            stage.queued_edits.push(command);
        }
        stage.intents_read
    }) {
        crate::runtime::egui::repaint::request_of(ctx, viewport);
    }
    true
}

/// The paste the focused component `id` takes this pass.
pub(crate) fn pasted(ctx: &Context, id: Id) -> Option<ClipContent> {
    current(ctx, |stage| stage.pasted.as_ref().filter(|(owner, _)| *owner == id).map(|(_, content)| content.clone()))
}

/// The text of the paste the focused component `id` takes this pass.
pub(crate) fn pasted_text(ctx: &Context, id: Id) -> Option<Arc<str>> {
    pasted(ctx, id).and_then(|content| content.text)
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
    Nudge(Nudge),
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
            let caret = caret_move(mv, stop.keys.owner);
            let leaves_row = !caret && is_vertical(mv) && focus::is_row_child(ctx, stop);
            let kept = caret || (stop.part.is_none() && stop.keys.keep.keeps_arrow(mv));
            // A stop that does not act on its moves leaves a move key a scoped command around it binds to that
            // command, ahead of its roving group and the scroll owner; the document canvas leaves a bound page key.
            if (!leaves_row && !kept && !stop.role.moves && commands::binds_scoped(ctx, &chord)) || page_key_runs(ctx, rec, press) {
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
        // A document canvas's editor reads Shift+Tab itself (a word processor's outdent, the previous slot of an
        // equation). No other stop that keeps Tab gets it as an intent.
        None if press.key == Key::Tab && press.mods == Mods::SHIFT && stop.keys.keep == focus::KeepKeys::Tab && stop.keys.is_document_editor() => {
            PressRoute::Intent(Intent::BackTab)
        }
        None if stop.role.nudge => nudge_of(press).map_or(PressRoute::Free, PressRoute::Nudge),
        None => PressRoute::Free,
    }
}

fn is_vertical(mv: NavMove) -> bool {
    matches!(mv, NavMove::Up | NavMove::Down | NavMove::PageUp | NavMove::PageDown | NavMove::Start | NavMove::End)
}

/// Whether the move `mv` of a stop with the key class `owner` is its caret's: Home and End of a text field (a row's
/// rename editor, a search field in a toolbar), which neither its row nor a roving group around it takes.
fn caret_move(mv: NavMove, owner: OwnerClass) -> bool {
    matches!(mv, NavMove::Start | NavMove::End) && owner == OwnerClass::Text
}

/// Acts on the route `press_route` gave `press` of the focused stop `rec`. Returns whether the component's intents
/// took the press.
fn apply_press(
    ctx: &Context,
    rec: &StopRec,
    press: &Press,
    route: PressRoute,
    routed: &mut HashMap<Option<Id>, Routed>,
    activated: &mut IdMap<Id>,
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
            let caret = caret_move(mv, stop.keys.owner);
            let kept = caret || (stop.part.is_none() && stop.keys.keep.keeps_arrow(mv));
            if let Some(owner) = (!caret && is_vertical(mv)).then(|| focus::leave_row(ctx, stop.key())).flatten() {
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
fn route_activate(ctx: &Context, rec: &StopRec, routed: &mut HashMap<Option<Id>, Routed>, activated: &mut IdMap<Id>) {
    let target = if rec.stop.group.is_some() && !focus::is_row_child(ctx, &rec.stop) {
        activated.insert(rec.stop.root, rec.stop.id);
        rec.stop.root
    } else {
        rec.stop.id
    };
    push(routed, Some(target), Intent::Activate, false);
}

/// An arrow press as a nudge: a unit direction, with the modifiers and the keys held at the press.
fn nudge_of(press: &Press) -> Option<Nudge> {
    let direction = match press.key {
        Key::Left => -Vec2::X,
        Key::Right => Vec2::X,
        Key::Up => -Vec2::Y,
        Key::Down => Vec2::Y,
        _ => return None,
    };
    Some(Nudge { direction, mods: press.mods, held: press.held.clone() })
}

/// Routes an AccessKit action request to the owner of its target node, whatever the focus (9.2 rule 2).
fn route_access(ctx: &Context, request: &Action, nodes: &HashMap<u64, Node>, routed: &mut HashMap<Option<Id>, Routed>, activated: &mut IdMap<Id>) {
    let Some(node) = nodes.get(&request.node()).copied() else { return };
    let owner = Some(node.owner);
    // A request that only scrolls (a screen reader's Scroll Into View) shows the node and leaves focus where it is.
    if node.focusable && !request.scrolls() {
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

/// The components the running pass's intents are routed to (keys, AccessKit requests, edits): a memoized view that
/// holds one runs, so the component takes them.
pub(crate) fn routed_to(ctx: &Context) -> Vec<Id> {
    let (mut routed, menu_keys) = current(ctx, |stage| (stage.routed.keys().flatten().copied().collect::<Vec<_>>(), !stage.menu.is_empty()));
    // The menu keys of the pass go to the menu bar the last pass registered.
    if menu_keys {
        routed.extend(focus::menu_bar(ctx, crate::runtime::egui::pass::current_viewport(ctx)));
    }
    routed
}

/// The member whose `Activate` the group `root` reads this pass.
pub(crate) fn activated_member(ctx: &Context, root: Id) -> Option<Id> {
    current(ctx, |stage| stage.activated.get(&root).copied())
}
