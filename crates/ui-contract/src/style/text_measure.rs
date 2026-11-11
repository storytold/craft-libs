//! The one resolution of a type role to laid-out text: the font of a span, the paragraph job a run starts from, and
//! the width of a line. `TextRun` lays out through the first two and `StyleCx::text_width` through the third, so the
//! sizes that follow the text agree with the text that is drawn.

use super::tokens::UiTokens;
use crate::fonts::StackFacts;
use crate::runtime::egui::text_layout;
use craft_i18n::Direction;
use craft_theme::TypeRole;
use egui::epaint::text::TextDirection;
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Align, Color32, Context};

/// The format of a span in `role`: its font at the role's size, with the optical size axis where the face has one.
pub(crate) fn span_format(
    tokens: &UiTokens,
    fonts: StackFacts,
    role: TypeRole,
    mono: bool,
    color: Color32,
    italics: bool,
    background: Color32,
) -> TextFormat {
    let mut format = TextFormat { font_id: tokens.font(fonts, role, mono), color, italics, background, ..TextFormat::default() };
    if let Some(size) = tokens.optical_size(fonts, role, mono) {
        format.coords.push(*b"opsz", size);
    }
    format
}

/// An empty job for a paragraph of `direction`. Every row stays at the left edge: egui's own alignment trims the first
/// glyph's offset from an aligned row only, so a run would measure differently by direction.
pub(crate) fn paragraph_job(direction: Direction, ltr: bool, wrap: TextWrapping) -> LayoutJob {
    let rtl = !ltr && direction == Direction::Rtl;
    LayoutJob {
        direction: if rtl { TextDirection::RightToLeft } else { TextDirection::LeftToRight },
        halign: Align::LEFT,
        halign_min_is_start: false,
        wrap,
        ..LayoutJob::default()
    }
}

/// The width of `text` set on one line in the face of `role`.
pub(crate) fn line_width(ctx: &Context, tokens: &UiTokens, fonts: StackFacts, direction: Direction, role: TypeRole, text: &str) -> f32 {
    let wrap = TextWrapping { max_width: f32::INFINITY, max_rows: usize::MAX, break_anywhere: false, ..TextWrapping::default() };
    let mut job = paragraph_job(direction, false, wrap);
    job.append(text, 0.0, span_format(tokens, fonts, role, false, Color32::PLACEHOLDER, false, Color32::TRANSPARENT));
    text_layout::layout(ctx, job).size().x
}
