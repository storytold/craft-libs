//! Building `TextEdit` and its state: the one caller of egui's text editing.

use craft_theme::TypeRole;
use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::text_edit::TextEditState;
use egui::{Color32, Context, EventFilter, FontId, Id, Rect, Ui};
use std::ops::Range;

/// One text editing widget.
pub(crate) struct Edit<'a> {
    pub id: Id,
    pub rect: Rect,
    /// The buffer; `None` shows `text` read-only.
    pub text: &'a mut String,
    pub read_only: bool,
    /// Whether the widget senses focus and input; a disabled one declares neither to AccessKit.
    pub enabled: bool,
    /// The widget takes key focus, which a disabled one does while its reason shows on focus.
    pub focusable: bool,
    /// The widget takes the pointer: presses, drags and clicks. A part whose component scrubs by drag leaves the pointer
    /// to the component until the field is in edit; the part stays a focus stop.
    pub pointer: bool,
    pub multiline: bool,
    pub password: bool,
    pub hint: Option<&'a str>,
    pub muted_hint: bool,
    pub mono: bool,
    pub role: TypeRole,
    pub ink: Color32,
    pub filter: EventFilter,
    pub char_limit: Option<usize>,
    /// The bytes of `text` that are IME preedit, underlined.
    pub preedit: Option<Range<usize>>,
    pub rtl: bool,
    /// The text sits in the middle of the rect.
    pub centred: bool,
    /// The text sits at the end edge of the rect (unless `centred`).
    pub end_aligned: bool,
    /// Whether the caret is drawn in this pass.
    pub caret_visible: bool,
    /// The width of the preedit underline.
    pub underline: f32,
    /// The text laid out already, as a run is painted: the widget shows this galley, which never scrolls, so the text
    /// is not clipped to `rect`.
    pub galley: Option<std::sync::Arc<egui::Galley>>,
    /// The screen y of the baseline a single line without a laid-out galley sits on: its control's line's, which a
    /// text in another face (mono digits) shares. `None` centres the line's row on the rect's centre line.
    pub baseline: Option<f32>,
    /// How far a multi-line text taller than `rect` is scrolled up inside it, in points; the text part owns the offset
    /// (`text_adapter`), and a single line or a laid-out galley ignores it.
    pub scroll: f32,
    /// The modifiers of the Enter chord that breaks the line of a multi-line text. egui inserts the line break itself
    /// before the adapter sees the Enter, so the adapter passes the chord its submit key leaves free: Shift under a plain
    /// Enter submit, none under Cmd or Ctrl+Enter.
    pub line_break: egui::Modifiers,
}

/// What the widget did in one pass.
pub(crate) struct Shown {
    pub changed: bool,
    pub focused: bool,
    pub lost_focus: bool,
    /// Enter was pressed while the widget held focus.
    pub entered: bool,
    /// A primary click on the widget ended this pass.
    pub clicked: bool,
    pub caret: Option<Rect>,
    pub galley: std::sync::Arc<egui::Galley>,
    /// Where the galley sits on screen.
    pub galley_pos: egui::Pos2,
    /// The height the widget's text takes, margins included: more than the rect's when a multi-line text scrolls.
    pub content: f32,
}

/// Runs the widget at `edit.rect`, under `env`, the env of the `Ui` the caller shows the field in.
pub(crate) fn show(ui: &mut Ui, env: &crate::runtime::env::Env, edit: Edit<'_>) -> Shown {
    let Edit {
        id,
        rect,
        text,
        read_only,
        enabled,
        focusable,
        pointer,
        multiline,
        password,
        hint,
        muted_hint,
        mono,
        role,
        ink,
        filter,
        char_limit,
        preedit,
        rtl,
        centred,
        end_aligned,
        caret_visible,
        underline,
        galley,
        baseline,
        scroll,
        line_break,
    } = edit;
    // The same font and optical size as a label of the same role: one text path.
    let tokens = &env.tokens;
    // Behind selected text and the selected text itself: the field's roles.
    let selection = (tokens.color.accent.soft, tokens.color.text.normal);
    let facts = crate::fonts::stack_facts(ui.ctx());
    let font_id = tokens.font(facts, role, mono);
    let optical_size = tokens.optical_size(facts, role, mono);
    let had_focus = ui.memory(|memory| memory.has_focus(id));
    let entered = had_focus && ui.input(|input| input.key_pressed(egui::Key::Enter));
    let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
        if let Some(galley) = &galley {
            return galley.clone();
        }
        // egui masks only in its own layouter: a custom one lays out what it is given, so the dots are made here. The
        // preedit range counts bytes of the real text, so a masked field shows no preedit underline.
        let masked;
        let text = if password {
            masked = "\u{2022}".repeat(text.as_str().chars().count());
            masked.as_str()
        } else {
            text.as_str()
        };
        let mut job = LayoutJob::default();
        job.wrap.max_width = if multiline { wrap_width } else { f32::INFINITY };
        let mut plain = TextFormat::simple(font_id.clone(), ink);
        if let Some(size) = optical_size {
            plain.coords.push(*b"opsz", size);
        }
        match preedit
            .as_ref()
            .filter(|_| !password)
            .filter(|range| range.end <= text.len() && text.is_char_boundary(range.start) && text.is_char_boundary(range.end))
        {
            Some(range) => {
                job.append(&text[..range.start], 0.0, plain.clone());
                job.append(&text[range.clone()], 0.0, TextFormat { underline: egui::Stroke::new(underline, ink), ..plain.clone() });
                job.append(&text[range.end..], 0.0, plain);
            }
            None => job.append(text, 0.0, plain),
        }
        #[cfg(feature = "testing")]
        crate::runtime::measure::count_text_layout(ui.ctx());
        ui.fonts_mut(|fonts| crate::runtime::egui::text_layout::layout_with(fonts, job))
    };
    let hint_text = hint.map(|hint| egui::RichText::new(hint).font(FontId::proportional(font_id.size)));
    // A single-line edit runs in a rect one text row tall, or the part's height when that is taller, placed so the
    // text's baseline is `baseline`, else centred on the part's centre line. egui centres the row in the rect, whatever
    // row it allocates from the rect's top.
    let rect = if multiline || galley.is_some() {
        rect
    } else {
        let probe = {
            let mut format = TextFormat::simple(font_id.clone(), ink);
            if let Some(size) = optical_size {
                format.coords.push(*b"opsz", size);
            }
            let mut job = LayoutJob::default();
            job.append("x", 0.0, format);
            super::text_layout::layout(ui.ctx(), job)
        };
        let row = probe.size().y;
        let first_baseline = probe.rows.first().and_then(|placed| placed.row.glyphs.first().map(|glyph| placed.pos.y + glyph.pos.y));
        let centre = match baseline.zip(first_baseline) {
            Some((baseline, first)) => baseline - first + row / 2.0,
            None => rect.center().y,
        };
        Rect::from_center_size(egui::pos2(rect.center().x, centre), egui::vec2(rect.width(), rect.height().max(row)))
    };
    // The component's root took its place in the flow already: the editor runs in a child that takes none, so the
    // parent's cursor stays past the root, not past the text part inside it.
    // A scrolled multi-line text runs moved up by its offset and shows only inside `rect` (the clip below), so the
    // widget's own hit test, caret and selection follow the text where it is drawn.
    let placed = if multiline && galley.is_none() && scroll > 0.0 { rect.translate(egui::vec2(0.0, -scroll)) } else { rect };
    let output = {
        let ui = &mut super::alloc::child(ui, egui::UiBuilder::new().max_rect(placed));
        super::style::set_text_edit_unfloored(ui);
        // The text shows only inside the text part, however wide the line is.
        if galley.is_none() {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        }
        if !enabled && !focusable {
            super::style::disable_without_fade(ui);
        }
        // The placeholder is the faint text colour, or the muted one on a fill the faint one does not read on, and the
        // disabled one when the field is off.
        let text_colors = &tokens.color.text;
        let hint_color = match (enabled, muted_hint) {
            (false, _) => text_colors.disabled,
            (true, true) => text_colors.muted,
            (true, false) => text_colors.faint,
        };
        super::style::set_hint_color(ui, crate::style::rgba_color(hint_color));
        super::style::set_text_cursor(ui, caret_visible);
        super::style::set_text_selection(ui, crate::style::rgba_color(selection.0), crate::style::rgba_color(selection.1));
        let mut read = text.as_str();
        macro_rules! build {
            ($buffer:expr) => {{
                let mut edit = if multiline { egui::TextEdit::multiline($buffer) } else { egui::TextEdit::singleline($buffer) };
                edit = edit
                    .id(id)
                    // egui sizes the text row by this font: the text's own, not egui's default.
                    .font(font_id.clone())
                    .interactive(focusable && pointer)
                    .frame(egui::Frame::NONE)
                    .desired_width(rect.width())
                    .min_size(rect.size())
                    // The part's rect sets the height; egui's default of four rows would grow past it.
                    .desired_rows(1)
                    .password(password)
                    .event_filter(filter)
                    .return_key(if multiline { Some(egui::KeyboardShortcut::new(line_break, egui::Key::Enter)) } else { None })
                    .layouter(&mut layouter)
                    // A laid-out galley sits at the rect's corner, as a painted run does.
                    .align(match (centred, rtl) {
                        _ if galley.is_some() => egui::Align2::LEFT_TOP,
                        (centred, rtl) => {
                            let across = match (centred, end_aligned == rtl) {
                                (true, _) => egui::Align::Center,
                                (false, true) => egui::Align::Min,
                                (false, false) => egui::Align::Max,
                            };
                            // A multi-line text starts at the part's top, however many of its lines the text fills.
                            egui::Align2([across, if multiline { egui::Align::Min } else { egui::Align::Center }])
                        }
                    });
                if let Some(hint) = hint_text.clone() {
                    edit = edit.hint_text(hint);
                }
                if let Some(limit) = char_limit {
                    edit = edit.char_limit(limit);
                }
                edit.show(ui)
            }};
        }
        let shown = if read_only { build!(&mut read) } else { build!(&mut *text) };
        // egui gives a widget of a disabled `Ui` no focus stop and drops its focus, and so does one that is not
        // interactive; a disabled part whose reason shows on focus, and a part that leaves the pointer to its
        // component, keep both (as `response::widget` does for a root).
        if focusable && (!ui.is_enabled() || !pointer) {
            let layer = ui.layer_id();
            ui.ctx().memory_mut(|memory| {
                memory.interested_in_focus(id, layer);
                if had_focus {
                    super::focus::keep_focus(memory, id);
                }
            });
        }
        shown
    };
    // A multi-line text taller than its part scrolls inside it: the part is the view, and the text, which the clip
    // above cuts to the view, the content (S-L186: the text adapter owns the offset).
    if multiline && galley.is_none() && output.response.rect.height() > rect.height() {
        let text_rect = egui::Rect::from_min_size(output.galley_pos, output.galley.rect.size());
        let content = placed.union(output.response.rect).union(text_rect);
        super::widgets::note_scroll_region(ui.ctx(), rect.intersect(ui.clip_rect()), content);
    }
    // The widget's key focus, whether or not the window has the system's: switching to another app is no focus move,
    // so the typing run, caret and selection carry on (egui's `Response::has_focus` is false while the window is in
    // the background).
    let focused = ui.memory(|memory| memory.has_focus(id));
    let caret = output.cursor_range.map(|range| {
        let at = output.galley.pos_from_cursor(range.primary);
        at.translate(output.galley_pos.to_vec2())
    });
    Shown {
        changed: output.response.changed(),
        focused,
        lost_focus: output.response.lost_focus() || (had_focus && !focused),
        entered,
        clicked: output.response.clicked(),
        caret,
        content: output.response.rect.height(),
        galley: output.galley,
        galley_pos: output.galley_pos,
    }
}

/// The selection of the widget as character indices, `(primary, secondary)`.
pub(crate) fn selection(ctx: &Context, id: Id) -> Option<(usize, usize)> {
    TextEditState::load(ctx, id).and_then(|state| state.cursor.char_range()).map(|range| (range.primary.index.0, range.secondary.index.0))
}

/// Sets the selection: `anchor` and `caret` are character indices.
pub(crate) fn select(ctx: &Context, id: Id, anchor: usize, caret: usize) {
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange { primary: CCursor::new(caret), secondary: CCursor::new(anchor), h_pos: None }));
    state.store(ctx, id);
}

/// Whether the widget's field has an undo step and a redo step, as `(undo, redo)`, against its current text `text`
/// (egui's undoer compares its steps with the text, which the widget's state does not hold). It reads the undoer and
/// steps nothing, so it agrees with what [`undo`] would do now.
pub(crate) fn history(ctx: &Context, id: Id, text: &str) -> (bool, bool) {
    let Some(state) = TextEditState::load(ctx, id) else { return (false, false) };
    let now = (state.cursor.char_range().unwrap_or_default(), text.to_owned());
    let undoer = state.undoer();
    (undoer.has_undo(&now), undoer.has_redo(&now))
}

/// The undo steps of the widget: `Some(text)` when `step` changed it.
pub(crate) fn undo(ctx: &Context, id: Id, current: &str, redo: bool) -> Option<String> {
    let mut state = TextEditState::load(ctx, id)?;
    let range = state.cursor.char_range().unwrap_or_default();
    let now = (range, current.to_owned());
    let mut undoer = state.undoer();
    let step = if redo { undoer.redo(&now).cloned() } else { undoer.undo(&now).cloned() };
    state.set_undoer(undoer);
    let (range, text) = step?;
    state.cursor.set_char_range(Some(range));
    state.store(ctx, id);
    Some(text)
}
