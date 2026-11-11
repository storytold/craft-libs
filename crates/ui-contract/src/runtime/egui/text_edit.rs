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
    /// Whether the caret is drawn in this pass.
    pub caret_visible: bool,
    /// The width of the preedit underline.
    pub underline: f32,
}

/// What the widget did in one pass.
pub(crate) struct Shown {
    pub changed: bool,
    pub focused: bool,
    pub lost_focus: bool,
    /// Enter was pressed while the widget held focus.
    pub entered: bool,
    pub caret: Option<Rect>,
    pub galley: std::sync::Arc<egui::Galley>,
    /// Where the galley sits on screen.
    pub galley_pos: egui::Pos2,
}

impl Shown {
    /// The character index a screen position falls at.
    pub fn char_at(&self, pos: egui::Pos2) -> usize {
        self.galley.cursor_from_pos(pos - self.galley_pos).index.0
    }
}

/// Runs the widget at `edit.rect`.
pub(crate) fn show(ui: &mut Ui, edit: Edit<'_>) -> Shown {
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
        mono,
        role,
        ink,
        filter,
        char_limit,
        preedit,
        rtl,
        centred,
        caret_visible,
        underline,
    } = edit;
    // The same font and optical size as a label of the same role: one text path.
    let env = crate::runtime::env::Env::of(ui);
    let tokens = &env.tokens;
    // Behind selected text and the selected text itself: the field's roles.
    let selection = (tokens.color.accent.soft, tokens.color.text.normal);
    let facts = crate::fonts::stack_facts(ui.ctx());
    let font_id = tokens.font(facts, role, mono);
    let optical_size = tokens.optical_size(facts, role, mono);
    let had_focus = ui.memory(|memory| memory.has_focus(id));
    let entered = had_focus && ui.input(|input| input.key_pressed(egui::Key::Enter));
    let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
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
        ui.fonts_mut(|fonts| crate::runtime::egui::text_layout::layout_with(fonts, job))
    };
    let hint_text = hint.map(|hint| egui::RichText::new(hint).font(FontId::proportional(font_id.size)));
    let output = ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        // The text shows only inside the text part, however wide the line is.
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        if !enabled && !focusable {
            super::style::disable_without_fade(ui);
        }
        // The placeholder is the faint text colour, the disabled one when the field is off.
        let text_colors = &crate::runtime::env::Env::of(ui).tokens.color.text;
        super::style::set_hint_color(ui, crate::style::rgba_color(if enabled { text_colors.faint } else { text_colors.disabled }));
        super::style::set_text_cursor(ui, caret_visible);
        super::style::set_text_selection(ui, crate::style::rgba_color(selection.0), crate::style::rgba_color(selection.1));
        let mut read = text.as_str();
        macro_rules! build {
            ($buffer:expr) => {{
                let mut edit = if multiline { egui::TextEdit::multiline($buffer) } else { egui::TextEdit::singleline($buffer) };
                edit = edit
                    .id(id)
                    .interactive(focusable && pointer)
                    .frame(egui::Frame::NONE)
                    .desired_width(rect.width())
                    .min_size(rect.size())
                    // The part's rect sets the height; egui's default of four rows would grow past it.
                    .desired_rows(1)
                    .password(password)
                    .event_filter(filter)
                    .return_key(if multiline { Some(egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::Enter)) } else { None })
                    .layouter(&mut layouter)
                    .align(match (centred, rtl) {
                        (true, _) => egui::Align2::CENTER_CENTER,
                        (false, true) => egui::Align2::RIGHT_CENTER,
                        (false, false) => egui::Align2::LEFT_CENTER,
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
                    memory.request_focus(id);
                }
            });
        }
        shown
    });
    let output = output.inner;
    let focused = output.response.has_focus();
    let caret = output.cursor_range.map(|range| {
        let at = output.galley.pos_from_cursor(range.primary);
        at.translate(output.galley_pos.to_vec2())
    });
    Shown {
        changed: output.response.changed(),
        focused,
        lost_focus: output.response.lost_focus() || (had_focus && !focused),
        entered,
        caret,
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
