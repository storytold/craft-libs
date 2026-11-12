//! The editor's own commands: Photoshop's type formatting keys and the commit chord. The editor reads them with
//! `take_command`, so the keymap rebinds them like any command; the editor reads no raw keys.

use craft_behavior::{Granularity, MoveMods, NavMove};

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

    /// The command the same key means in text that runs vertically: the arrows follow the flow (Up is Left, Down
    /// Right, Left Down, Right Up), so the leading and tracking steps trade places. Other commands and horizontal text
    /// unchanged.
    pub(super) fn along_flow(self, vertical: bool) -> Self {
        use TextEditorCommand as C;
        if !vertical {
            return self;
        }
        match self {
            C::LeadingLess => C::TrackingLess,
            C::LeadingMore => C::TrackingMore,
            C::LeadingLessFar => C::TrackingLessFar,
            C::LeadingMoreFar => C::TrackingMoreFar,
            C::TrackingLess => C::LeadingMore,
            C::TrackingMore => C::LeadingLess,
            C::TrackingLessFar => C::LeadingMoreFar,
            C::TrackingMoreFar => C::LeadingLessFar,
            other => other,
        }
    }

    /// The caret move along the text that the command's arrow key makes when the host takes no formatting step:
    /// leading a line up or down (with the command modifier to the text's start or end), baseline the same while
    /// extending the selection, tracking a word back or forward. `None` for the commands without an arrow.
    pub(super) fn fallback(self) -> Option<(NavMove, MoveMods)> {
        use TextEditorCommand as C;
        let line = |extend: bool, by: Granularity| MoveMods::new(extend, false, by);
        Some(match self {
            C::LeadingLess => (NavMove::Up, line(false, Granularity::Unit)),
            C::LeadingMore => (NavMove::Down, line(false, Granularity::Unit)),
            C::LeadingLessFar => (NavMove::Up, line(false, Granularity::Document)),
            C::LeadingMoreFar => (NavMove::Down, line(false, Granularity::Document)),
            C::BaselineUp => (NavMove::Up, line(true, Granularity::Unit)),
            C::BaselineDown => (NavMove::Down, line(true, Granularity::Unit)),
            C::BaselineUpFar => (NavMove::Up, line(true, Granularity::Document)),
            C::BaselineDownFar => (NavMove::Down, line(true, Granularity::Document)),
            C::TrackingLess | C::TrackingLessFar => (NavMove::Back, line(false, Granularity::Word)),
            C::TrackingMore | C::TrackingMoreFar => (NavMove::Forward, line(false, Granularity::Word)),
            _ => return None,
        })
    }
}
