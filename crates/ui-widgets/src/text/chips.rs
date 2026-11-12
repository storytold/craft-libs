//! `ChipField`: a list of text chips in lines, each removable, with a draft text part after the last chip, "+N more"
//! where a line limit stops the list, and Clear All.

#[cfg(feature = "testing")]
mod stories;

use super::{FieldStyle, text_part};
use crate::buttons::IconButton;
use crate::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Entry, Focus as _, MeasureCx, Name, Outcome, OverlayUi, Overlays as _,
    PaintUi, PartsCx, Pointer as _, Props as _, Role, WordSetCx as _,
};
use crate::display::Badge;
use crate::menu::Menu;
use crate::primitives::TextOptions;
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx, SurfaceLook};
use craft_behavior::{Caret, Codec, Flow, Granularity, IconId, Intent, Metrics, NavShape, NotAValue, PartKey, Proposal, Roving, Sense};
use craft_theme::Variant;
use egui::{Rect, Response, Vec2, pos2, vec2};
use std::borrow::Cow;

/// The characters that end a chip in typed or pasted text.
const SEPARATORS: [char; 3] = [',', ';', '\n'];

/// A chip: the chip's text on a pill, its remove part at the pill's end.
const CHIP: PartKey = PartKey::new("chip");
/// A chip's remove part: the square at the end of the chip's pill.
const REMOVE: PartKey = PartKey::new("remove");
/// The button after the shown chips that counts the hidden ones and opens their menu.
const MORE: PartKey = PartKey::new("more");
/// The button that removes every chip.
const CLEAR: PartKey = PartKey::new("clear");

/// The keyboard path of the remove parts.
const REMOVE_BY_KEY: &str = "Delete or Backspace on the focused chip removes it";

craft_ui::words! {
    /// The words of the chip field.
    pub struct ChipWords {
        /// A chip's remove part: `{name}` the chip's text.
        remove: template "chipField.remove" = "Remove \u{201c}{name}\u{201d}";
        /// The button for the chips past the line limit: `{n}` how many.
        more: count "chipField.more" = "+{n} more" | "+{n} more";
        /// The button that removes every chip.
        clear_all: "chipField.clearAll" = "Clear All";
        /// A partial chip's accessible name (the asterisk is only drawn): `{name}` the chip's text.
        partial: template "chipField.partial" = "{name}, some of the selection";
        /// The name of the menu of the chips past the line limit.
        hidden: "chipField.hidden" = "More chips";
    }
}

/// The text form of a chip list while typing: the text part shows the draft only, and committing it adds the chips
/// the draft names after the ones the field has.
///
/// A draft names one chip per piece between separators (comma, semicolon, new line), trimmed; a piece the list
/// already holds, and an empty piece, add nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ChipCodec;

impl ChipCodec {
    /// The chips a draft names, in order.
    fn pieces(text: &str) -> impl Iterator<Item = &str> {
        text.split(|c| SEPARATORS.contains(&c)).map(str::trim).filter(|piece| !piece.is_empty())
    }
}

impl Codec for ChipCodec {
    type Value = Vec<String>;

    fn format(&self, _: &Vec<String>) -> String {
        String::new()
    }

    fn parse(&self, text: &str, start: &Vec<String>) -> Result<Vec<String>, NotAValue> {
        let mut chips = start.clone();
        for piece in ChipCodec::pieces(text) {
            if !chips.iter().any(|chip| chip == piece) {
                chips.push(piece.to_owned());
            }
        }
        Ok(chips)
    }

    fn normalise(&self, _: &str, _: &Vec<String>) -> String {
        String::new()
    }

    fn caret_on_focus(&self, _: &str) -> Caret {
        Caret::End
    }

    fn sample(&self, _: &Vec<String>) -> Cow<'_, str> {
        Cow::Borrowed("")
    }
}

/// The style of a chip field: the field look with a gap between chips and between lines.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ChipFieldStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    /// The least width of the draft, so a full line of chips still leaves room to type.
    draft_min: f32,
}

/// Names the chips only some of the selection carries, by their text.
pub struct PartialChips<'a>(Box<dyn Fn(&str) -> bool + 'a>);

impl<'a> PartialChips<'a> {
    /// The chips `partial` holds true for are partial.
    pub fn new(partial: impl Fn(&str) -> bool + 'a) -> Self {
        PartialChips(Box::new(partial))
    }
}

/// A field of removable chips and a draft text part. Enter or leaving the field commits the draft as chips; the
/// value is the list of chip texts.
///
/// A click on a chip, or Enter or Space on the focused one, reports it (`ChipFieldOutput::activated`); a secondary
/// click on a chip, or the Menu key or Shift+F10 on the focused one, asks for its menu (`context_menu`), which the
/// host opens with `Trigger::Context` on the field's response. A chip cut to its line shows its whole text as its
/// tooltip.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(overlay)]
pub struct ChipField<'a> {
    #[common]
    common: Common,
    value: Bind<'a, Vec<String>>,
    options: TextOptions,
    /// Shown while the field has no chips and the draft is empty.
    #[prop(into)]
    placeholder: Option<Label>,
    /// The chips only some of the selection carries: drawn muted with an asterisk after the text.
    #[prop(func = "Fn(&str) -> bool")]
    partial: Option<PartialChips<'a>>,
    /// At most this many lines, the draft's included; the chips that don't fit go behind a "+N more" button at the
    /// end of the last line, whose menu lists them, and choosing one there removes it.
    #[prop]
    max_lines: Option<usize>,
    /// A Clear All button after the chips removes every chip.
    #[prop(flag)]
    clear_all: bool,
    /// How many chips show this pass, the rest behind "+N more"; set by `parts`.
    shown: Option<usize>,
    /// The chip activated this pass.
    activated: Option<usize>,
}

impl<'a> ChipField<'a> {
    /// A chip field named `label` over `value`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, Vec<String>>) -> Self {
        let options = TextOptions { clear_on_escape: true, commit_on: &[',', ';'], ..TextOptions::default() };
        ChipField {
            common: Common::new(),
            value,
            options,
            placeholder: None,
            partial: None,
            max_lines: None,
            clear_all: false,
            shown: None,
            activated: None,
        }
        .label(label)
    }

    fn is_partial(&self, chip: &str) -> bool {
        self.partial.as_ref().is_some_and(|partial| (partial.0)(chip))
    }

    /// The face of chip `chip`: its text, with an asterisk after a partial one's and muted then, on a pill whose end
    /// holds the remove part.
    fn face(&self, chip: &str) -> Badge {
        let face = Badge::new(if self.is_partial(chip) { format!("{chip}*") } else { chip.to_owned() }).variant(Variant::SECONDARY).end_mark();
        if self.is_partial(chip) { face.muted() } else { face }
    }
}

/// What a chip field reports: the edit of its value, and the chip activated or asked for its menu this pass.
#[non_exhaustive]
pub struct ChipFieldOutput {
    /// The edit response of the chip list.
    pub edit: EditResponse<Vec<String>>,
    /// The chip clicked or activated with Enter or Space.
    pub activated: Option<usize>,
    /// The chip whose context menu was asked for; the host's `Trigger::Context` menu on the field opens at it.
    pub context_menu: Option<usize>,
}

impl std::ops::Deref for ChipFieldOutput {
    type Target = EditResponse<Vec<String>>;

    fn deref(&self) -> &Self::Target {
        &self.edit
    }
}

impl Outcome for ChipFieldOutput {
    type Value = Vec<String>;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// Where the chips and the draft sit: line and distance from the start edge, for widths in reading order.
struct Placement {
    widths: Vec<f32>,
    cells: Vec<(usize, f32)>,
    /// Line, start offset and width of the draft.
    draft: (usize, f32, f32),
    lines: usize,
}

/// Flows chips `widths` wide into lines `room` wide, `gap` apart: a chip that does not fit what is left of a line
/// starts the next, and a chip wider than a line is cut to the line. The draft takes what is left of the last line
/// when that is `draft_min` wide, else a line of its own.
fn place(widths: &[f32], room: f32, gap: f32, draft_min: f32) -> Placement {
    let mut cells = Vec::with_capacity(widths.len());
    let mut clamped = Vec::with_capacity(widths.len());
    let (mut line, mut at) = (0, 0.0_f32);
    for &width in widths {
        let width = width.min(room);
        if at > 0.0 && at + width > room {
            line += 1;
            at = 0.0;
        }
        cells.push((line, at));
        clamped.push(width);
        at += width + gap;
    }
    let left = room - at;
    let draft = match () {
        () if widths.is_empty() => (0, 0.0, room),
        () if left >= draft_min => (line, at, left),
        () => (line + 1, 0.0, room),
    };
    Placement { widths: clamped, cells, draft, lines: draft.0 + 1 }
}

/// The chips measured as faces, and where they and the buttons sit.
struct Arranged {
    /// The width each chip's face wants, its remove part included.
    wants: Vec<f32>,
    /// A chip's height, and the side of its remove part.
    side: f32,
    /// A line's height: a control, or a chip when that is taller.
    line: f32,
    gap: f32,
    /// How many chips show; "+N more" counts the rest.
    shown: usize,
    /// Clear All follows the chips.
    clear: bool,
    /// The cells in reading order: the shown chips, "+N more" when chips are hidden, Clear All; then the draft.
    placement: Placement,
}

impl Arranged {
    fn height(&self) -> f32 {
        let lines = self.placement.lines;
        lines as f32 * self.line + (lines - 1) as f32 * self.gap
    }

    /// The cell of "+N more", when chips are hidden.
    fn more_cell(&self, chips: usize) -> Option<usize> {
        (self.shown < chips).then_some(self.shown)
    }

    /// The cell of Clear All, when it shows.
    fn clear_cell(&self, chips: usize) -> Option<usize> {
        self.clear.then_some(self.shown + usize::from(self.shown < chips))
    }
}

/// Measures the chips and flows them into `room` (`None`: one line). With a line limit, as many chips show as leave
/// room for "+N more" in the last line, the draft's line counted.
fn arrange(cx: &MeasureCx<'_, ChipField<'_>>, field: &ChipField<'_>, chips: &[String], room: Option<f32>) -> Arranged {
    let metrics = &cx.style.metrics;
    let sizes: Vec<Vec2> = chips.iter().enumerate().map(|(at, chip)| cx.child(("chip", at), &mut field.face(chip), Proposal::Ideal)).collect();
    let words = cx.words::<ChipWords>();
    let button = |key: &'static str, text: String| cx.child(key, &mut Badge::new(text), Proposal::Ideal).x;
    // The widest count names every chip, so the button keeps its width whatever it counts.
    let more = field.max_lines.map(|_| button("more", more_text(&words, chips.len(), &cx.format().integer(chips.len() as u64))));
    let clear = (field.clear_all && !chips.is_empty()).then(|| button("clear", words.clear_all.as_str().to_owned()));
    let side = sizes.iter().map(|size| size.y).fold(0.0, f32::max);
    let gap = cx.gap();
    let draft_min = cx.style.draft_min;
    let wants: Vec<f32> = sizes.iter().map(|size| size.x).collect();
    let room = room.unwrap_or_else(|| wants.iter().chain(&clear).sum::<f32>() + gap * (wants.len() + 1) as f32 + draft_min);
    let layout = |shown: usize| {
        let mut cells = wants[..shown].to_vec();
        cells.extend(more.filter(|_| shown < chips.len()));
        cells.extend(clear);
        place(&cells, room, gap, draft_min)
    };
    let mut shown = chips.len();
    let mut placement = layout(shown);
    if let Some(limit) = field.max_lines.map(|limit| limit.max(1)).filter(|&limit| placement.lines > limit) {
        // The chips that fit `limit` lines, less as many as the button needs room for.
        let alone = place(&wants, room, gap, draft_min);
        shown = alone.cells.iter().take_while(|(line, _)| *line < limit).count();
        loop {
            placement = layout(shown);
            if placement.lines <= limit || shown == 0 {
                break;
            }
            shown -= 1;
        }
    }
    Arranged { wants, side, line: metrics.min_size.y.max(side), gap, shown, clear: clear.is_some(), placement }
}

/// "+N more" for `hidden` chips, `integer` the count in the installed number format.
fn more_text(words: &ChipWords, hidden: usize, integer: &str) -> String {
    words.more.text(hidden as u64, integer)
}

impl Component for ChipField<'_> {
    const ROLE: Role = Role::TEXT_INPUT;
    const INPUT: crate::contract::Input = crate::contract::Input::Text;
    type Style = ChipFieldStyle;
    type Output = ChipFieldOutput;

    fn style(cx: &StyleCx<'_>, _: ChipFieldStyleKey) -> ChipFieldStyle {
        let field = FieldStyle::at(cx);
        ChipFieldStyle { surface: field.surface, metrics: field.metrics.gap(cx.control().gap), draft_min: cx.tokens.size.length.short }
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let chips = cx.value().map_or(&[][..], Vec::as_slice);
        let arranged = arrange(cx, self, chips, cx.room);
        let (_, start, width) = arranged.placement.draft;
        Vec2::new(cx.room.unwrap_or(start + width), arranged.height())
    }

    /// The shown chips and their remove parts, "+N more", Clear All, then the draft; the chip bodies and the buttons
    /// are one roving group.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(chips) = cx.value() else { return };
        let content = cx.content_rect;
        let arranged = arrange(cx, self, chips, Some(content.width()));
        self.shown = Some(arranged.shown);
        let rtl = cx.flow.is_rtl();
        let line_rect = |line: usize| {
            let top = content.min.y + line as f32 * (arranged.line + arranged.gap);
            Rect::from_min_size(pos2(content.min.x, top), vec2(content.width(), arranged.line))
        };
        let cell_rect = |at: usize| {
            let (line, start) = arranged.placement.cells[at];
            let cell = Flow::new(line_rect(line), rtl).span(start, arranged.placement.widths[at]);
            Rect::from_center_size(cell.center(), vec2(cell.width(), arranged.side))
        };
        for (at, chip) in chips.iter().enumerate().take(arranged.shown) {
            let cell = cell_rect(at);
            let (remove, _) = Flow::new(cell, rtl).take_end(arranged.side.min(cell.width()));
            let part = cx.part(CHIP.at(at as u64), cell).sense(Sense::CLICK | Sense::FOCUS).focusable();
            // A chip cut to its line shows its whole text as its tooltip.
            let part = if arranged.placement.widths[at] < arranged.wants[at] { part.tooltip(Label::from(chip.clone())) } else { part };
            // The chip next to the draft is the group's arrival: Shift+Tab from the draft lands on it.
            if at + 1 == arranged.shown {
                part.selected();
            }
            cx.part(REMOVE.at(at as u64), remove).sense(Sense::CLICK).over().pointer_only(REMOVE_BY_KEY);
        }
        if let Some(at) = arranged.more_cell(chips.len()) {
            cx.part(MORE, cell_rect(at)).sense(Sense::CLICK | Sense::FOCUS).focusable().toggles("more");
        }
        if let Some(at) = arranged.clear_cell(chips.len()) {
            cx.part(CLEAR, cell_rect(at)).sense(Sense::CLICK | Sense::FOCUS).focusable();
        }
        let (line, start, width) = arranged.placement.draft;
        let draft = Flow::new(line_rect(line), rtl).span(start, width);
        let mut options = self.options.clone();
        options.placeholder = self.placeholder.clone().filter(|_| chips.is_empty());
        cx.text(text_part::<Vec<String>>(), draft, &ChipCodec, chips, &options);
        if !chips.is_empty() {
            cx.roving(Roving::new(NavShape::Flat));
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, Vec<String>>> {
        Some(self.value.reborrow())
    }

    /// A remove part's click, or Delete or Backspace on a focused chip, removes the chip; Backspace in an empty draft
    /// focuses the last shown chip. Delete in an empty draft does nothing. Clear All removes every chip. A click on a
    /// chip, or Enter or Space on the focused one, is reported.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        cx.popup("more");
        let Some(chips) = cx.value().cloned() else { return };
        let shown = self.shown.unwrap_or(chips.len()).min(chips.len());
        if cx.part_activated(CLEAR).is_some() {
            cx.set(Vec::new());
            return;
        }
        self.activated = (0..shown).find(|&at| cx.part_activated(CHIP.at(at as u64)).is_some());
        let draft_empty = cx.text(&text_part::<Vec<String>>()).is_none_or(|text| text.draft.as_deref().is_none_or(str::is_empty));
        let on_chip = cx.active_part().filter(|part| part.key == CHIP).and_then(|part| part.index);
        let in_draft = cx.is_focused() && cx.active_part().is_none();
        let backspace = Intent::delete(false, Granularity::Unit);
        let delete = Intent::delete(true, Granularity::Unit);
        let mut removed = (0..shown).find(|&at| {
            // Guard: a commit asks for a new pass, so no chip is undeclared; one that is stays out of this pass.
            cx.part_activated(REMOVE.at(at as u64)).is_some()
        });
        if removed.is_none() {
            if let Some(at) = on_chip {
                if cx.intents().take_exact(&backspace) || cx.intents().take_exact(&delete) {
                    removed = Some(at as usize);
                }
            } else if in_draft && draft_empty && shown > 0 && cx.intents().take_exact(&backspace) {
                cx.focus_part(CHIP.at(shown as u64 - 1));
            }
        }
        if let Some(at) = removed.filter(|&at| at < shown) {
            // Next pass the chip after the removed one sits at `at`, where focus already is; with none, the draft.
            if on_chip.is_some() && at + 1 == shown {
                cx.focus_own();
            }
            let mut rest = chips;
            rest.remove(at);
            cx.set(rest);
        }
    }

    /// The menu of the hidden chips: one row per chip, its whole text; choosing one removes it.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        crate::menu::text_menu(ui, cx, &self.options);
        let popup = cx.popup("more");
        let chips = cx.value().cloned().unwrap_or_default();
        let shown = self.shown.unwrap_or(chips.len()).min(chips.len());
        if shown == chips.len() {
            if popup.is_open(cx) {
                popup.close(cx);
            }
            return;
        }
        let hidden: Vec<Entry<usize>> = (shown..chips.len()).map(|at| Entry::new(at, Label::from(chips[at].clone()))).collect();
        let name = cx.words::<ChipWords>().hidden.clone();
        let picked = ui.show(popup, crate::menu::layer(false), |ui, _| Menu::inline(name.clone()).pick(ui, &hidden).picked).flatten();
        if let Some(at) = picked {
            let mut rest = chips;
            rest.remove(at);
            cx.set(rest);
        }
    }

    /// Each shown chip is a list item named by its text, a partial one with the partial word; the buttons are named
    /// by their text.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let Some(chips) = cx.value() else { return };
        let words = cx.words::<ChipWords>();
        let shown = self.shown.unwrap_or(chips.len()).min(chips.len());
        for (at, chip) in chips.iter().enumerate().take(shown) {
            let name = if self.is_partial(chip) { words.partial.fill(&[("name", chip.as_str())]) } else { chip.clone() };
            access.node(CHIP.at(at as u64), Role::LIST_ITEM).name = Name::Text(name);
        }
        if shown < chips.len() {
            let hidden = chips.len() - shown;
            access.node(MORE, Role::BUTTON).name = Name::Text(more_text(&words, hidden, &cx.format().integer(hidden as u64)));
        }
        if cx.part_if_declared(CLEAR).is_some() {
            access.node(CLEAR, Role::BUTTON).name = Name::Text(words.clear_all.as_str().to_owned());
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(chips) = cx.value() else { return };
        for (at, chip) in chips.iter().enumerate() {
            // Guard: a commit asks for a new pass, so no chip is undeclared; one that is stays out of this pass.
            if cx.part_if_declared(CHIP.at(at as u64)).is_none() {
                continue;
            }
            ui.face(CHIP.at(at as u64), self.face(chip));
        }
        let words = cx.words::<ChipWords>();
        if cx.part_if_declared(MORE).is_some() {
            let hidden = chips.len() - self.shown.unwrap_or(chips.len()).min(chips.len());
            ui.face(MORE, Badge::new(more_text(&words, hidden, &cx.format().integer(hidden as u64))).variant(Variant::SECONDARY));
        }
        if cx.part_if_declared(CLEAR).is_some() {
            ui.face(CLEAR, Badge::new(words.clear_all.clone()).variant(Variant::SECONDARY));
        }
    }

    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(chips) = cx.value() else { return };
        let words = cx.words::<ChipWords>();
        for (at, chip) in chips.iter().enumerate() {
            if cx.part_if_declared(REMOVE.at(at as u64)).is_none() {
                continue;
            }
            let name = words.remove.fill(&[("name", chip.as_str())]);
            ui.face(REMOVE.at(at as u64), IconButton::new(IconId::new("close")).label(name).focusable(false));
        }
    }

    /// The chip the context request of this pass was made at is the one whose menu was asked for.
    fn finish(self, cx: Cx<'_, Self>) -> ChipFieldOutput {
        let context_menu = cx.context_part().filter(|part| part.key == CHIP).and_then(|part| part.index).map(|at| at as usize);
        ChipFieldOutput { activated: self.activated, context_menu, edit: cx.edit_response() }
    }
}
