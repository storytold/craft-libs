//! Pure focus rules (spec 9.3, 9.4, 8.3): key ownership, focus after removal, the Escape order and the
//! focus-on-press kinds. The runtime's focus service feeds them facts and applies the result.

use crate::sense::Sense;

/// What a focus scope is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScopeKind {
    /// A window.
    Window,
    /// A pane, an F6 stop.
    Pane,
    /// A dialog.
    Dialog,
    /// A popup.
    Popup,
    /// A menu.
    Menu,
    /// A group of controls.
    Group,
}

/// What Escape does in a scope that has no more specific taker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CancelRule {
    /// Close the scope.
    Close,
    /// Clear the scope's content first, close on the next Escape.
    ClearThenClose,
    /// Do nothing.
    Ignore,
}

/// A key class: the input of key ownership.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OwnerClass {
    /// A control that takes its own keys.
    Widget,
    /// A text run.
    Text,
    /// A shortcut recorder.
    Recorder,
    /// The app's shortcuts.
    App,
    /// A popup that takes keys.
    Popup,
    /// A popup that leaves keys to what is below.
    PassivePopup,
    /// A menu: arrows move through its entries and Escape closes the innermost.
    Menu,
    /// A dialog: Tab stays inside, Enter submits, Escape cancels.
    Dialog,
}

/// Who owns the keys of one pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeyOwner {
    /// A modal layer is above the focus: no component owns keys.
    Blocked,
    /// The highest class present.
    Class(OwnerClass),
}

/// Key ownership, in the order blocked, `Menu`, `Recorder`, `Dialog`, `Text`, `Popup` and `PassivePopup`,
/// `Widget`, `App`. `present` lists the classes that have a claim this pass, in any order.
pub fn key_owner(blocked: bool, present: &[OwnerClass]) -> KeyOwner {
    if blocked {
        return KeyOwner::Blocked;
    }
    const ORDER: [&[OwnerClass]; 7] = [
        &[OwnerClass::Menu],
        &[OwnerClass::Recorder],
        &[OwnerClass::Dialog],
        &[OwnerClass::Text],
        &[OwnerClass::Popup, OwnerClass::PassivePopup],
        &[OwnerClass::Widget],
        &[OwnerClass::App],
    ];
    ORDER
        .iter()
        .find_map(|rank| rank.iter().find(|class| present.contains(class)))
        .map_or(KeyOwner::Class(OwnerClass::App), |class| KeyOwner::Class(*class))
}

/// The stop that takes focus when `focused` is no longer registered: the one after it in last pass's order
/// that is still registered, else the one before. `None` leaves the scope's initial stop to decide.
pub fn after_removal<'a, T: PartialEq>(last_order: &'a [T], focused: &T, registered: impl Fn(&T) -> bool) -> Option<&'a T> {
    let at = last_order.iter().position(|stop| stop == focused)?;
    let after = last_order[at + 1..].iter().find(|stop| registered(stop));
    after.or_else(|| last_order[..at].iter().rev().find(|stop| registered(stop)))
}

/// What the Escape of one pass reaches first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EscapeStep {
    /// A captured press is cancelled; nothing else sees this Escape.
    CancelPress,
    /// The focused component has something to undo.
    Component,
    /// Focus returns from a row's child to the row.
    BackToRow,
    /// The scope clears its content and stays open.
    ClearScope,
    /// The scope closes.
    CloseScope,
    /// Nothing takes it.
    Ignore,
}

/// The facts the Escape order reads.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct EscapeFacts {
    /// A press is captured on any egui layer.
    pub press_captured: bool,
    /// The focused component has an open draft, a gesture or its own open popup.
    pub component_undoable: bool,
    /// Focus is on a row's child.
    pub in_row_child: bool,
    /// The scope has content to clear.
    pub scope_clearable: bool,
}

impl EscapeFacts {
    /// No fact set.
    pub const fn new() -> Self {
        EscapeFacts { press_captured: false, component_undoable: false, in_row_child: false, scope_clearable: false }
    }

    /// A press is captured.
    pub const fn press_captured(mut self, on: bool) -> Self {
        self.press_captured = on;
        self
    }

    /// Focus is on a row's child.
    pub const fn in_row_child(mut self, on: bool) -> Self {
        self.in_row_child = on;
        self
    }

    /// The scope has content to clear.
    pub const fn scope_clearable(mut self, on: bool) -> Self {
        self.scope_clearable = on;
        self
    }
}

/// The order of Escape for the top egui layer that is not exiting: a captured press, the focused component,
/// a row's child, then the scope's rule.
pub fn escape_step(facts: EscapeFacts, rule: CancelRule) -> EscapeStep {
    if facts.press_captured {
        EscapeStep::CancelPress
    } else if facts.component_undoable {
        EscapeStep::Component
    } else if facts.in_row_child {
        EscapeStep::BackToRow
    } else {
        match rule {
            CancelRule::Close => EscapeStep::CloseScope,
            CancelRule::ClearThenClose if facts.scope_clearable => EscapeStep::ClearScope,
            CancelRule::ClearThenClose => EscapeStep::CloseScope,
            CancelRule::Ignore => EscapeStep::Ignore,
        }
    }
}

/// How the Escapes that arrived in one pass split between the captured press and the components.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EscapeRouting {
    /// The first Escape cancels the captured press and reaches no component.
    pub cancels_press: bool,
    /// The Escapes components see as `Cancel`.
    pub to_components: usize,
}

/// Routes `escapes` Escapes of one pass: with a press captured, the first one is the press's (`escape_step`'s
/// first case) and the rest go to the components, in the order they arrived.
pub fn route_escapes(escapes: usize, press_captured: bool) -> EscapeRouting {
    let first = escapes > 0 && escape_step(EscapeFacts::new().press_captured(press_captured), CancelRule::Ignore) == EscapeStep::CancelPress;
    EscapeRouting { cancels_press: first, to_components: escapes - usize::from(first) }
}

/// How a primary press moves key focus (8.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PressFocus {
    /// Kind 1: focus moves to the winning shape's component.
    Take,
    /// Kind 2: focus and any draft stay.
    Keep,
    /// Kind 3: the control acts; a focused text part commits and focus moves to its enclosing pane.
    Release,
    /// Kind 4: no shape claims the press; a focused text part commits and focus is surrendered.
    Unclaimed,
}

impl PressFocus {
    /// Whether a focused text part with a draft commits it in the press pass.
    pub const fn commits_draft(self) -> bool {
        matches!(self, PressFocus::Release | PressFocus::Unclaimed)
    }
}

/// The kind of a press from the sense of the shape that wins it, `None` when no shape senses it. A shape that
/// senses both `TAKE_FOCUS` and `KEEP_FOCUS` takes focus.
pub fn classify_press(winner: Option<Sense>) -> PressFocus {
    match winner {
        None => PressFocus::Unclaimed,
        Some(sense) if sense.contains(Sense::TAKE_FOCUS) => PressFocus::Take,
        Some(sense) if sense.contains(Sense::KEEP_FOCUS) => PressFocus::Keep,
        Some(_) => PressFocus::Release,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ownership_follows_the_documented_order() {
        use OwnerClass::*;
        assert_eq!(key_owner(true, &[Menu]), KeyOwner::Blocked);
        assert_eq!(key_owner(false, &[App, Widget, Text, Dialog, Recorder, Menu]), KeyOwner::Class(Menu));
        assert_eq!(key_owner(false, &[App, Widget, Text, Dialog]), KeyOwner::Class(Dialog));
        assert_eq!(key_owner(false, &[App, Widget, PassivePopup, Text]), KeyOwner::Class(Text));
        assert_eq!(key_owner(false, &[App, Widget, PassivePopup]), KeyOwner::Class(PassivePopup));
        assert_eq!(key_owner(false, &[App, Widget]), KeyOwner::Class(Widget));
        assert_eq!(key_owner(false, &[]), KeyOwner::Class(App));
    }
}
