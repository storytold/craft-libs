//! `StatusBadge`: a link's status as a glyph and as words, never colour alone.

use craft_behavior::IconId;
use craft_docpath::Refused;
use craft_docpath::links::LinkStatus;
use craft_ui::primitives::{Glyph, IconSpec};
use craft_ui::runtime::env::Label;
use craft_ui::style::{TypeRole, UiTokens, rgba_color};
use egui::{Align2, Color32, FontFamily, FontId, Painter, Pos2, Rect, Shape, Stroke, vec2};

use super::LinkListWords;

/// The disc's radius as a share of the badge square (6.5 in 16).
const DISC: f32 = 0.40625;
/// The triangle's half width and apex height as a share of the square (7 in 16).
const TRIANGLE_REACH: f32 = 0.4375;
/// The triangle's base below the centre as a share of the square (6 in 16).
const TRIANGLE_BASE: f32 = 0.375;
/// The `!` below the centre as a share of the square (1.5 in 16).
const BANG_DROP: f32 = 0.09375;
/// The ban sign's ring radius as a share of the disc's.
const BAN_RING: f32 = 0.55;

/// A status badge drawn from the theme: a solid shape in the status colour with its mark in `text.on_status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StatusGlyph {
    /// A `status.bad.solid` disc with `?`.
    Missing,
    /// A `status.bad.solid` disc with `×`.
    Unreadable,
    /// A `status.bad.solid` disc with the ban sign.
    Refused,
    /// A `status.warn.solid` triangle with `!`.
    Modified,
}

static MISSING: StatusGlyph = StatusGlyph::Missing;
static UNREADABLE: StatusGlyph = StatusGlyph::Unreadable;
static REFUSED: StatusGlyph = StatusGlyph::Refused;
static MODIFIED: StatusGlyph = StatusGlyph::Modified;

impl Glyph for StatusGlyph {
    fn paint(&self, painter: &Painter, tokens: &UiTokens, rect: Rect, _: Color32, _: bool) {
        let side = rect.width().min(rect.height());
        let center = rect.center();
        let status = &tokens.color.status;
        let mark = rgba_color(tokens.color.text.on_status);
        match self {
            StatusGlyph::Modified => {
                let (reach, base) = (side * TRIANGLE_REACH, side * TRIANGLE_BASE);
                let points = vec![center + vec2(0.0, -reach), center + vec2(reach, base), center + vec2(-reach, base)];
                painter.add(Shape::convex_polygon(points, rgba_color(status.warn.solid), Stroke::NONE));
                text(painter, tokens, center + vec2(0.0, side * BANG_DROP), "!", mark);
            }
            StatusGlyph::Missing | StatusGlyph::Unreadable | StatusGlyph::Refused => {
                let radius = side * DISC;
                painter.circle_filled(center, radius, rgba_color(status.bad.solid));
                match self {
                    StatusGlyph::Missing => text(painter, tokens, center, "?", mark),
                    StatusGlyph::Unreadable => text(painter, tokens, center, "\u{d7}", mark),
                    _ => {
                        let ring = radius * BAN_RING;
                        let stroke = Stroke::new(tokens.stroke.medium, mark);
                        painter.circle_stroke(center, ring, stroke);
                        let across = vec2(ring, -ring) * std::f32::consts::FRAC_1_SQRT_2;
                        painter.line_segment([center - across, center + across], stroke);
                    }
                }
            }
        }
    }
}

/// `mark` centred at `at`, in the caption size.
fn text(painter: &Painter, tokens: &UiTokens, at: Pos2, mark: &str, color: Color32) {
    let size = tokens.typography.get(TypeRole::Caption).size;
    painter.text(at, Align2::CENTER_CENTER, mark, FontId::new(size, FontFamily::Proportional), color);
}

/// A link's status for a row's end and a candidate's mark.
///
/// The glyph is one drawing per status, in the status colours: Missing a `status.bad.solid` disc with `?`,
/// Unreadable the disc with `×`, Refused the disc with the ban sign, Modified a `status.warn.solid` triangle with `!`
/// (the marks in `text.on_status`); Embedded the picture mark; Ok has none. The words carry the same status for the
/// row's name and the glyph's tooltip.
pub struct StatusBadge;

impl StatusBadge {
    /// The glyph of `status`; `None` for Ok.
    pub fn icon(status: LinkStatus) -> Option<IconSpec<'static>> {
        let glyph: &'static StatusGlyph = match status {
            LinkStatus::Missing => &MISSING,
            LinkStatus::Unreadable => &UNREADABLE,
            LinkStatus::Refused(_) => &REFUSED,
            LinkStatus::Modified => &MODIFIED,
            LinkStatus::Embedded => return Some(IconSpec::svg(IconId::new("image"))),
            _ => return None,
        };
        Some(IconSpec::glyph(glyph))
    }

    /// The status in words ("Missing", "Modified", "Unreadable", "Not followed: the path is on another computer",
    /// "Embedded"); empty for Ok.
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
