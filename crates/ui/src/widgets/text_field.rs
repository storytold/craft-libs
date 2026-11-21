//! Text-edit helpers: a bounded single-line field in a shared box style, and owned right-to-left
//! layouters for edits that do not go through it. In a right-to-left interface the text aligns at
//! the reading start and the galley's base direction is pinned right-to-left, so Arabic or Hebrew
//! content shapes from the reading start whatever script the text begins with; monospaced
//! code-like input (hex fields) stays left-to-right in both directions. Left-to-right everything
//! is egui's plain edit, unchanged.

use std::sync::Arc;

use egui::{Color32, FontId, Frame, Id, Rect, Response, Sense, Stroke, StrokeKind, TextBuffer, Ui, Vec2, vec2};

use crate::direction::{HasDirection, is_rtl};

/// The most characters a [`text_field`] holds unless its caller sets a limit: far beyond any
/// search, name or hex value, yet a pasted multi-gigabyte string is cut at the field instead of
/// being laid out every frame.
pub const TEXT_FIELD_LIMIT: usize = 1024;

/// The most characters one typed, pasted or IME-committed text event carries into the app
/// ([`cap_text_events`]): a megabyte of text, more than any field is pasted.
pub const INPUT_EVENT_LIMIT: usize = 1 << 20;

/// Cuts every text, paste and IME event to [`INPUT_EVENT_LIMIT`] characters (at a character
/// boundary) before any widget sees it, so no text field — whatever its own limit — has to lay
/// out an 8 GB paste.
pub fn cap_text_events(events: &mut [egui::Event]) {
    fn cut(text: &mut String) -> bool {
        let Some((end, _)) = text.char_indices().nth(INPUT_EVENT_LIMIT) else { return false };
        text.truncate(end);
        true
    }
    for event in events {
        match event {
            egui::Event::Text(text) | egui::Event::Paste(text) | egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                cut(text);
            }
            // The highlighted range indexes the old text; past the cut it would point nowhere.
            egui::Event::Ime(egui::ImeEvent::Preedit { text, active_range_chars }) => {
                let truncated = cut(text);
                if truncated && active_range_chars.as_ref().is_some_and(|r| r.end > INPUT_EVENT_LIMIT) {
                    *active_range_chars = None;
                }
            }
            _ => {}
        }
    }
}

/// Height of a [`text_field`]'s box.
pub const TEXT_FIELD_H: f32 = 24.0;

/// Inner padding of a [`text_field`]'s box: 6 pt either side, 2 pt above and below.
const TEXT_FIELD_PAD: Vec2 = vec2(6.0, 2.0);

/// An owned right-to-left layouter for a single-line edit: the galley's base direction is pinned
/// [`egui::epaint::text::TextDirection::RightToLeft`], so Arabic or Hebrew content shapes from
/// the reading start whatever script the text begins with (a Latin- or digit-first value would
/// otherwise infer a left-to-right paragraph). The line anchors at its end (`halign = Max`),
/// which is what a [`egui::Align2::RIGHT_CENTER`] edit hands egui's default layouter; line height
/// and language match the default too, so only the direction changes.
pub fn rtl_singleline_layouter(font: FontId) -> impl FnMut(&Ui, &dyn TextBuffer, f32) -> Arc<egui::Galley> {
    move |ui: &Ui, text: &dyn TextBuffer, _wrap_width: f32| {
        let mut job = egui::text::LayoutJob::simple_singleline(text.as_str().to_owned(), font.clone(), ui.visuals().text_color());
        job.direction = egui::epaint::text::TextDirection::RightToLeft;
        job.halign = egui::Align::Max;
        job.language = ui.language();
        job.keep_trailing_whitespace = true;
        let line_height = ui.fonts_mut(|f| f.row_height(&font)) + ui.spacing().extra_text_line_spacing;
        for section in &mut job.sections {
            section.format.line_height = Some(line_height);
        }
        ui.fonts_mut(|f| f.layout_job(job))
    }
}

/// A raw single-line edit for inputs that do not go through [`text_field`]: in a right-to-left
/// interface it aligns at the reading start and installs `layouter` (build it with
/// [`rtl_singleline_layouter`]) so the text's base direction is right-to-left as well. The
/// layouter must outlive the returned editor, hence the parameter. In a left-to-right interface
/// the plain edit comes back unchanged.
pub fn rtl_singleline<'t>(
    d: &impl HasDirection,
    text: &'t mut String,
    layouter: &'t mut dyn FnMut(&Ui, &dyn TextBuffer, f32) -> Arc<egui::Galley>,
) -> egui::TextEdit<'t> {
    let edit = egui::TextEdit::singleline(text);
    if !is_rtl(d) {
        return edit;
    }
    edit.align(egui::Align2::RIGHT_CENTER).layouter(layouter)
}

/// A single-line edit in the body font, `width` wide, `hint` shown while empty; in a right-to-left
/// interface the text reads from the reading start via [`rtl_singleline`]. Left-to-right the plain
/// edit comes back unchanged.
pub fn rtl_singleline_edit(ui: &mut Ui, d: &impl HasDirection, text: &mut String, width: f32, hint: &str) -> Response {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let mut layouter = rtl_singleline_layouter(font);
    ui.add(rtl_singleline(d, text, &mut layouter).desired_width(width).hint_text(hint.to_owned()))
}

/// [`rtl_singleline`] with `configure` applied to the editor (an accessible name, say).
pub fn rtl_singleline_add(
    ui: &mut Ui,
    d: &impl HasDirection,
    text: &mut String,
    configure: impl FnOnce(egui::TextEdit<'_>) -> egui::TextEdit<'_>,
) -> Response {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let mut layouter = rtl_singleline_layouter(font);
    ui.add(configure(rtl_singleline(d, text, &mut layouter)))
}

/// An owned right-to-left layouter for a multiline edit: the galley's base direction is pinned
/// right-to-left and every line anchors at its end, wrapping at the given width.
pub fn rtl_multiline_layouter(font: FontId) -> impl FnMut(&Ui, &dyn TextBuffer, f32) -> Arc<egui::Galley> {
    move |ui: &Ui, text: &dyn TextBuffer, wrap_width: f32| {
        let mut job = egui::text::LayoutJob::simple(text.as_str().to_owned(), font.clone(), ui.visuals().text_color(), wrap_width);
        job.direction = egui::epaint::text::TextDirection::RightToLeft;
        job.halign = egui::Align::Max;
        job.keep_trailing_whitespace = true;
        ui.fonts_mut(|f| f.layout_job(job))
    }
}

/// A multiline edit `rows` rows tall and `width` wide; in a right-to-left interface it aligns to
/// the reading end with the base direction pinned by [`rtl_multiline_layouter`]. Left-to-right the
/// plain edit comes back unchanged.
pub fn rtl_multiline_edit(ui: &mut Ui, d: &impl HasDirection, text: &mut String, rows: usize, width: f32) -> Response {
    let edit = egui::TextEdit::multiline(text).desired_rows(rows).desired_width(width);
    if !is_rtl(d) {
        return ui.add(edit);
    }
    let mut layouter = rtl_multiline_layouter(FontId::default());
    ui.add(edit.align(egui::Align2::RIGHT_TOP).layouter(&mut layouter))
}

/// Colors and fonts for a [`text_field`]; the application owns the theme.
#[derive(Clone, Debug)]
pub struct TextFieldStyle {
    pub fill: Color32,
    pub border: Color32,
    pub radius: f32,
    pub text: Color32,
    pub selection: Color32,
    /// The proportional (non-mono) font.
    pub font: FontId,
    /// The monospaced font for code-like values.
    pub mono_font: FontId,
}

impl Default for TextFieldStyle {
    fn default() -> Self {
        Self {
            fill: Color32::from_gray(40),
            border: Color32::from_gray(70),
            radius: 3.0,
            text: Color32::WHITE,
            selection: Color32::from_rgb(0x2d, 0x6a, 0xd8),
            font: FontId::proportional(12.5),
            mono_font: FontId::monospace(12.0),
        }
    }
}

/// A single-line text field in a shared box style: a rounded box `width` wide with consistent
/// inner padding and no focus outline. `id` carries the field's text and focus across frames;
/// `hint` shows while it is empty; `mono` picks the monospaced face for hex and other code-like
/// values. Returns the editor's response and the field's box.
///
/// Hint and value align at the field's reading start — the right edge in a right-to-left
/// interface; monospaced code-like input stays left-to-right in both directions. Under
/// right-to-left the edit also pins the galley's base direction right-to-left, so Arabic shapes
/// from the reading start instead of inferring left-to-right from its first characters. The value
/// is limited to `char_limit` characters (or [`TEXT_FIELD_LIMIT`]); a value already past the
/// limit is shown and edited as its first `limit` characters, so it is never laid out whole.
#[allow(clippy::too_many_arguments)]
pub fn text_field(
    ui: &mut Ui,
    d: &impl HasDirection,
    id: Id,
    text: &mut String,
    hint: &str,
    width: f32,
    mono: bool,
    char_limit: Option<usize>,
    style: &TextFieldStyle,
) -> (Response, Rect) {
    let (rect, _) = ui.allocate_exact_size(vec2(width.max(16.0), TEXT_FIELD_H), Sense::hover());
    ui.painter().rect_filled(rect, style.radius, style.fill);
    ui.painter().rect_stroke(rect, style.radius, Stroke::new(1.0, style.border), StrokeKind::Inside);
    let font = if mono { style.mono_font.clone() } else { style.font.clone() };
    let inner = rect.shrink2(TEXT_FIELD_PAD);
    let limit = char_limit.unwrap_or(TEXT_FIELD_LIMIT);
    // A value already past the limit (restored from a state file, set by automation) is shown and
    // edited as its first `limit` characters, so it is never laid out whole; the caller's string
    // changes only when that copy is edited.
    let mut bounded = text.char_indices().nth(limit).map(|(end, _)| text[..end].to_string());
    let shown = match bounded.as_mut() {
        Some(b) => b,
        None => &mut *text,
    };
    let rtl = !mono && is_rtl(d);
    let align = if rtl { egui::Align2::RIGHT_CENTER } else { egui::Align2::LEFT_CENTER };
    let mut rtl_layouter = rtl.then(|| rtl_singleline_layouter(font.clone()));
    // TextEdit draws its text at the top of its rect; fill the box and centre the line in it, so
    // the text sits on the field's middle like a value field's.
    let mut edit = egui::TextEdit::singleline(shown)
        .id(id)
        .hint_text(hint.to_owned())
        .frame(Frame::NONE)
        .font(font)
        .min_size(inner.size())
        .align(align)
        .char_limit(limit);
    if let Some(layouter) = rtl_layouter.as_mut() {
        edit = edit.layouter(layouter);
    }
    // A child Ui keeps the field's selection colour local: `Visuals::selection` also fills
    // selected list rows and slider trails, which must keep the theme's.
    let layout = egui::Layout::centered_and_justified(egui::Direction::TopDown);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(layout));
    // The default 24 pt interact height would make the edit taller than the box's inside.
    child.spacing_mut().interact_size.y = inner.height();
    child.style_mut().visuals.selection.bg_fill = style.selection;
    let response = child.add(edit);
    if let Some(b) = bounded
        && response.changed()
    {
        *text = b;
    }
    (response, rect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::{self, Direction};
    use egui::{Context, Pos2, RawInput};

    fn events_with_text(chars: usize) -> Vec<egui::Event> {
        vec![egui::Event::Text("x".repeat(chars))]
    }

    fn text_len(e: &egui::Event) -> usize {
        match e {
            egui::Event::Text(t) | egui::Event::Paste(t) | egui::Event::Ime(egui::ImeEvent::Commit(t)) => t.chars().count(),
            _ => panic!("not a committed text event: {e:?}"),
        }
    }

    #[test]
    fn cap_text_events_cuts_at_a_char_boundary_and_invalidates_ime_ranges() {
        // Exactly at the limit: untouched.
        let mut events = events_with_text(INPUT_EVENT_LIMIT);
        cap_text_events(&mut events);
        assert_eq!(text_len(&events[0]), INPUT_EVENT_LIMIT);
        // One past: cut at the boundary.
        let mut events = events_with_text(INPUT_EVENT_LIMIT + 10);
        cap_text_events(&mut events);
        assert_eq!(text_len(&events[0]), INPUT_EVENT_LIMIT);
        // Multibyte chars: the cut lands on a char boundary, not mid-codepoint.
        let mut events = vec![egui::Event::Text("é".repeat(INPUT_EVENT_LIMIT + 1))];
        cap_text_events(&mut events);
        assert_eq!(text_len(&events[0]), INPUT_EVENT_LIMIT);
        // An IME preedit whose active range runs past the cut loses the range.
        let mut events = vec![egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "y".repeat(INPUT_EVENT_LIMIT + 5),
            active_range_chars: Some(INPUT_EVENT_LIMIT + 1..INPUT_EVENT_LIMIT + 3),
        })];
        cap_text_events(&mut events);
        match &events[0] {
            egui::Event::Ime(egui::ImeEvent::Preedit { text, active_range_chars }) => {
                assert_eq!(text.chars().count(), INPUT_EVENT_LIMIT);
                assert!(active_range_chars.is_none(), "the range past the cut is invalidated");
            }
            e => panic!("not a preedit: {e:?}"),
        }
        // A range inside the cut survives.
        let mut events = vec![egui::Event::Ime(egui::ImeEvent::Preedit { text: "y".repeat(INPUT_EVENT_LIMIT + 5), active_range_chars: Some(2..4) })];
        cap_text_events(&mut events);
        match &events[0] {
            egui::Event::Ime(egui::ImeEvent::Preedit { active_range_chars, .. }) => assert!(active_range_chars.is_some()),
            e => panic!("not a preedit: {e:?}"),
        }
        // Non-text events pass through untouched.
        let mut events = vec![egui::Event::PointerMoved(egui::pos2(1.0, 2.0))];
        cap_text_events(&mut events);
        assert!(matches!(&events[0], egui::Event::PointerMoved(_)));
    }

    #[test]
    fn the_rtl_singleline_layouter_pins_the_base_direction() {
        let ctx = Context::default();
        direction::apply(&ctx, Direction::Rtl);
        let buf = "abc".to_string();
        ctx.run_ui(RawInput::default(), |ui| {
            let mut l = rtl_singleline_layouter(FontId::default());
            let g = l(ui, &buf, 100.0);
            assert_eq!(g.job.direction, egui::epaint::text::TextDirection::RightToLeft);
            assert_eq!(g.job.halign, egui::Align::Max);
        })
        .textures_delta
        .clear();
    }

    /// Render a `text_field` in `dir` and return the value galley's job direction, when drawn.
    fn value_direction(dir: Direction, mono: bool) -> Option<egui::epaint::text::TextDirection> {
        let ctx = Context::default();
        direction::apply(&ctx, dir);
        let mut text = "abc123".to_string();
        let mut out = ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 100.0))), ..Default::default() }, |ui| {
            text_field(ui, &dir, Id::unique("f"), &mut text, "hint", 200.0, mono, None, &TextFieldStyle::default());
        });
        let mut found = None;
        for shape in &out.shapes {
            if let egui::Shape::Text(ts) = &shape.shape
                && ts.galley.text().contains("abc123")
            {
                found = Some(ts.galley.job.direction);
            }
        }
        out.textures_delta.clear();
        found
    }

    /// The field pins the value's base direction right-to-left in a right-to-left interface, but
    /// monospaced code-like input stays on egui's auto-detected direction in both directions.
    #[test]
    fn text_field_pins_the_value_direction_in_rtl_but_not_for_mono() {
        use egui::epaint::text::TextDirection as TD;
        assert_eq!(value_direction(Direction::Ltr, false), Some(TD::Auto), "ltr keeps egui's auto direction");
        assert_eq!(value_direction(Direction::Rtl, false), Some(TD::RightToLeft), "rtl pins right-to-left");
        assert_eq!(value_direction(Direction::Rtl, true), Some(TD::Auto), "mono stays physical (auto) even in rtl");
        assert_eq!(value_direction(Direction::Ltr, true), Some(TD::Auto));
    }

    /// A value already past the limit is shown truncated to the limit, never laid out whole.
    #[test]
    fn an_over_limit_value_is_shown_bounded() {
        let ctx = Context::default();
        direction::apply(&ctx, Direction::Ltr);
        let mut text = "z".repeat(TEXT_FIELD_LIMIT + 500);
        let mut out = ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 100.0))), ..Default::default() }, |ui| {
            let (resp, _rect) =
                text_field(ui, &Direction::Ltr, Id::unique("f"), &mut text, "hint", 200.0, false, Some(8), &TextFieldStyle::default());
            assert!(!resp.changed(), "showing the bounded copy is not a change");
        });
        let mut shown = None;
        for shape in &out.shapes {
            if let egui::Shape::Text(ts) = &shape.shape
                && ts.galley.text().starts_with('z')
            {
                shown = Some(ts.galley.text().to_string());
            }
        }
        out.textures_delta.clear();
        assert_eq!(shown.as_deref(), Some("zzzzzzzz"), "only the first `limit` characters are laid out: {shown:?}");
        assert_eq!(text.chars().count(), TEXT_FIELD_LIMIT + 500, "the caller's string is untouched until edited");
    }
}
