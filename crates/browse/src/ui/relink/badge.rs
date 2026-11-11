//! `StatusBadge`: a link's status as a glyph and as words, never colour alone.

use craft_behavior::IconId;
use craft_docpath::Refused;
use craft_docpath::links::LinkStatus;
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::Label;

use super::LinkListWords;

/// A link's status for a row's end and a candidate's mark.
///
/// The glyph is one drawing per status: Missing and Unreadable the error mark, Modified the warning
/// triangle, Refused the ban sign, Embedded the picture mark; Ok has none. The words carry the same
/// status for the row's name and the glyph's tooltip.
pub struct StatusBadge;

impl StatusBadge {
    /// The glyph of `status`; `None` for Ok.
    pub fn icon(status: LinkStatus) -> Option<IconSpec<'static>> {
        let name = match status {
            LinkStatus::Missing | LinkStatus::Unreadable => "error",
            LinkStatus::Modified => "warning",
            LinkStatus::Refused(_) => "ban",
            LinkStatus::Embedded => "image",
            _ => return None,
        };
        Some(IconSpec::svg(IconId::new(name)))
    }

    /// The status in words ("Missing", "Modified", "Unreadable", "Not followed: the path is on
    /// another computer", "Embedded"); empty for Ok.
    pub fn words(status: LinkStatus, words: &LinkListWords) -> Label {
        match status {
            LinkStatus::Missing => words.missing.clone(),
            LinkStatus::Modified => words.modified.clone(),
            LinkStatus::Unreadable => words.unreadable.clone(),
            LinkStatus::Embedded => words.embedded.clone(),
            LinkStatus::Refused(why) => {
                let reason = match why {
                    Refused::Network => &words.refused_network,
                    Refused::Device => &words.refused_device,
                    Refused::Url => &words.refused_url,
                    Refused::Invalid => &words.refused_invalid,
                };
                Label::from(words.refused.fill(&[("reason", reason.as_str())]))
            }
            _ => Label::Empty,
        }
    }
}
