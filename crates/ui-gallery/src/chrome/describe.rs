//! Words for the facts the decision record shows: a reader sees what a component is, not how the code spells it.

use craft_behavior::Input;
use craft_ui::contract::ShapeKind;
use craft_ui::runtime::access::Role;
use craft_ui::runtime::focus::{ActivateKeys, KeepKeys, KeyUse};

/// What the component shows: no body, a body from the caller, or an item per entry of a source.
pub(super) fn shape(shape: ShapeKind) -> &'static str {
    match shape {
        ShapeKind::Leaf => "Leaf, no body",
        ShapeKind::Body => "Body from the caller",
        ShapeKind::Items => "An item per entry of a source",
        _ => "Another shape",
    }
}

/// How input reaches the component.
pub(super) fn input(input: Input) -> &'static str {
    match input {
        Input::Pointer => "The pointer acts on it",
        Input::Text => "A text field",
        Input::Display => "Shows something, takes no input",
        Input::Editor => "An editor with its own caret",
        _ => "Another kind of input",
    }
}

/// What the component is to assistive technology. AccessKit names its roles by their variant, so the words are that
/// name spaced out.
pub(super) fn role(role: Role) -> String {
    match role {
        Role::None => "No node".to_owned(),
        Role::Merged => "Merged into its container's node".to_owned(),
        Role::Node(role) => {
            let name = format!("{role:?}");
            let mut words = String::with_capacity(name.len() + 4);
            for (at, letter) in name.chars().enumerate() {
                if at > 0 && letter.is_uppercase() {
                    words.push(' ');
                }
                words.push(if at > 0 { letter.to_ascii_lowercase() } else { letter });
            }
            words
        }
        _ => "Another role".to_owned(),
    }
}

/// The keys the focused component keeps, as a sentence.
pub(super) fn keys(keys: &KeyUse) -> String {
    let activates = match keys.activate {
        ActivateKeys::Neither => "no activation key",
        ActivateKeys::Enter => "Enter activates",
        ActivateKeys::Space => "Space activates",
        ActivateKeys::Both => "Enter and Space activate",
        _ => "other activation keys",
    };
    let kept = match keys.keep {
        KeepKeys::None => None,
        KeepKeys::Tab => Some("keeps Tab"),
        KeepKeys::InlineArrows => Some("keeps Left and Right"),
        KeepKeys::BlockArrows => Some("keeps all four arrows"),
        _ => Some("keeps other keys"),
    };
    let flags = [(keys.value, "arrows step a value"), (keys.moves, "takes arrow moves"), (keys.nudge, "arrows nudge its content")];
    let mut parts = vec![activates.to_owned()];
    parts.extend(kept.map(str::to_owned));
    parts.extend(flags.into_iter().filter(|(set, _)| *set).map(|(_, words)| words.to_owned()));
    if !keys.claims.is_empty() {
        parts.push(format!("claims {} shortcuts", keys.claims.len()));
    }
    parts.join(", ")
}
