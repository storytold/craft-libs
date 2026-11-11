//! The editor's own commands: Photoshop's type formatting keys and the commit chord. The editor reads them with
//! `take_command`, so the keymap rebinds them like any command; the editor reads no raw keys.

use super::{CharToggle, StyleKey};

/// How far a leading, baseline or tracking step moves, in points or thousandths of an em.
const LEADING_STEP: f32 = 2.0;
const BASELINE_STEP: f32 = 2.0;
const TRACKING_STEP: f32 = 20.0;
/// The factor of a step taken with the command modifier added.
const FAR: f32 = 5.0;

/// The commands of a type session. Each runs on the editor that holds the keyboard; the host decides what a
/// formatting command means for its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui::contract::CommandSet)]
#[command(prefix = "textEditor", scope = "textEditor")]
#[non_exhaustive]
pub enum TextEditorCommand {
    /// Commit the session and keep the text.
    #[command(label = "Commit Text", default = "Cmd+Enter")]
    Commit,
    /// Toggle All Caps on the selection.
    #[command(label = "All Caps", default = "Cmd+Shift+K")]
    AllCaps,
    /// Toggle Underline on the selection.
    #[command(label = "Underline", default = "Cmd+Shift+U")]
    Underline,
    /// Toggle Faux Bold on the selection.
    #[command(label = "Faux Bold", default = "Cmd+Shift+B")]
    FauxBold,
    /// Toggle Faux Italic on the selection.
    #[command(label = "Faux Italic", default = "Cmd+Shift+I")]
    FauxItalic,
    /// Toggle Strikethrough on the selection.
    #[command(label = "Strikethrough", default = "Cmd+Shift+/")]
    Strikethrough,
    /// Hyphenate the selected paragraphs.
    #[command(label = "Hyphenate", default = "Cmd+Alt+Shift+H")]
    Hyphenate,
    /// Reduce the leading.
    #[command(label = "Reduce Leading", default = "Alt+Up")]
    LeadingLess,
    /// Increase the leading.
    #[command(label = "Increase Leading", default = "Alt+Down")]
    LeadingMore,
    /// Reduce the leading in large steps.
    #[command(label = "Reduce Leading More", default = "Cmd+Alt+Up")]
    LeadingLessFar,
    /// Increase the leading in large steps.
    #[command(label = "Increase Leading More", default = "Cmd+Alt+Down")]
    LeadingMoreFar,
    /// Raise the baseline.
    #[command(label = "Raise Baseline", default = "Alt+Shift+Up")]
    BaselineUp,
    /// Lower the baseline.
    #[command(label = "Lower Baseline", default = "Alt+Shift+Down")]
    BaselineDown,
    /// Raise the baseline in large steps.
    #[command(label = "Raise Baseline More", default = "Cmd+Alt+Shift+Up")]
    BaselineUpFar,
    /// Lower the baseline in large steps.
    #[command(label = "Lower Baseline More", default = "Cmd+Alt+Shift+Down")]
    BaselineDownFar,
    /// Tighten the tracking or kerning.
    #[command(label = "Tighten Tracking", default = "Alt+Left")]
    TrackingLess,
    /// Loosen the tracking or kerning.
    #[command(label = "Loosen Tracking", default = "Alt+Right")]
    TrackingMore,
    /// Tighten the tracking or kerning in large steps.
    #[command(label = "Tighten Tracking More", default = "Cmd+Alt+Left")]
    TrackingLessFar,
    /// Loosen the tracking or kerning in large steps.
    #[command(label = "Loosen Tracking More", default = "Cmd+Alt+Right")]
    TrackingMoreFar,
}

impl TextEditorCommand {
    /// The formatting key the command stands for; `None` for `Commit`. Tracking at a caret is kerning.
    pub(super) fn style_key(self, selection_is_empty: bool) -> Option<StyleKey> {
        use TextEditorCommand as C;
        let tracking = |by: f32| if selection_is_empty { StyleKey::Kern(by) } else { StyleKey::Tracking(by) };
        Some(match self {
            C::Commit => return None,
            C::AllCaps => StyleKey::Toggle(CharToggle::AllCaps),
            C::Underline => StyleKey::Toggle(CharToggle::Underline),
            C::FauxBold => StyleKey::Toggle(CharToggle::FauxBold),
            C::FauxItalic => StyleKey::Toggle(CharToggle::FauxItalic),
            C::Strikethrough => StyleKey::Toggle(CharToggle::Strikethrough),
            C::Hyphenate => StyleKey::Hyphenate,
            C::LeadingLess => StyleKey::Leading(-LEADING_STEP),
            C::LeadingMore => StyleKey::Leading(LEADING_STEP),
            C::LeadingLessFar => StyleKey::Leading(-LEADING_STEP * FAR),
            C::LeadingMoreFar => StyleKey::Leading(LEADING_STEP * FAR),
            C::BaselineUp => StyleKey::BaselineShift(BASELINE_STEP),
            C::BaselineDown => StyleKey::BaselineShift(-BASELINE_STEP),
            C::BaselineUpFar => StyleKey::BaselineShift(BASELINE_STEP * FAR),
            C::BaselineDownFar => StyleKey::BaselineShift(-BASELINE_STEP * FAR),
            C::TrackingLess => tracking(-TRACKING_STEP),
            C::TrackingMore => tracking(TRACKING_STEP),
            C::TrackingLessFar => tracking(-TRACKING_STEP * FAR),
            C::TrackingMoreFar => tracking(TRACKING_STEP * FAR),
        })
    }

    /// Whether the command is a tracking step that moves the caret by word when the host has no pair to kern:
    /// the direction is forward for a loosen step.
    pub(super) fn word_move(self) -> Option<bool> {
        match self {
            TextEditorCommand::TrackingLess | TextEditorCommand::TrackingLessFar => Some(false),
            TextEditorCommand::TrackingMore | TextEditorCommand::TrackingMoreFar => Some(true),
            _ => None,
        }
    }
}
