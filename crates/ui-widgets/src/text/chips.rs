//! `ChipField`: a list of text chips in lines, each removable, with a draft text part after the last chip.

#[cfg(feature = "testing")]
mod stories;

use super::{FieldStyle, text_part};
use crate::buttons::IconButton;
use crate::contract::{
    Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, PaintUi, PartsCx, Pointer as _, Props as _, Role,
    WordSetCx as _,
};
use crate::display::Badge;
use crate::primitives::TextOptions;
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx, SurfaceLook};
use craft_behavior::{Caret, Codec, Flow, Granularity, IconId, Intent, Metrics, NavShape, NotAValue, PartKey, Proposal, Roving, Sense};
use craft_theme::Variant;
use egui::{Rect, Vec2, pos2, vec2};
use std::borrow::Cow;

/// The characters that end a chip in typed or pasted text.
const SEPARATORS: [char; 3] = [',', ';', '\n'];

/// A chip's body: the chip's text on a pill.
const CHIP: PartKey = PartKey::new("chip");
/// A chip's remove part, at the end of the chip.
const REMOVE: PartKey = PartKey::new("remove");

/// The keyboard path of the remove parts.
const REMOVE_BY_KEY: &str = "Delete or Backspace on the focused chip removes it";

craft_ui::words! {
    /// The words of the chip field.
    pub struct ChipWords {
        /// A chip's remove part: `{name}` the chip's text.
        remove: template "chipField.remove" = "Remove \u{201c}{name}\u{201d}";
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

/// A field of removable chips and a draft text part. Enter or leaving the field commits the draft as chips; the
/// value is the list of chip texts.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct ChipField<'a> {
    #[common]
    common: Common,
    value: Bind<'a, Vec<String>>,
    options: TextOptions,
    /// Shown while the field has no chips and the draft is empty.
    #[prop(into)]
    placeholder: Option<Label>,
}

impl<'a> ChipField<'a> {
    /// A chip field named `label` over `value`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, Vec<String>>) -> Self {
        let options = TextOptions { clear_on_escape: true, commit_on: &[',', ';'], ..TextOptions::default() };
        ChipField { common: Common::new(), value, options, placeholder: None }.label(label)
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

/// The chips measured as faces, and where they sit.
struct Arranged {
    /// The width of each chip's body face.
    bodies: Vec<f32>,
    /// A chip's height, and the side of its remove part.
    side: f32,
    /// A line's height: a control, or a chip when that is taller.
    line: f32,
    gap: f32,
    placement: Placement,
}

impl Arranged {
    fn height(&self) -> f32 {
        let lines = self.placement.lines;
        lines as f32 * self.line + (lines - 1) as f32 * self.gap
    }
}

/// Measures the chips and flows them into `room` (`None`: one line).
fn arrange(cx: &MeasureCx<'_, ChipField<'_>>, chips: &[String], room: Option<f32>) -> Arranged {
    let metrics = &cx.style.metrics;
    let sizes: Vec<Vec2> =
        chips.iter().enumerate().map(|(at, chip)| cx.child(("chip", at), &mut Badge::new(chip.clone()), Proposal::Ideal)).collect();
    let side = sizes.iter().map(|size| size.y).fold(0.0, f32::max);
    let gap = cx.gap();
    let draft_min = cx.style.draft_min;
    let widths: Vec<f32> = sizes.iter().map(|size| size.x + side).collect();
    let room = room.unwrap_or_else(|| widths.iter().sum::<f32>() + gap * widths.len() as f32 + draft_min);
    Arranged {
        bodies: sizes.iter().map(|size| size.x).collect(),
        side,
        line: metrics.min_size.y.max(side),
        gap,
        placement: place(&widths, room, gap, draft_min),
    }
}

impl Component for ChipField<'_> {
    const ROLE: Role = Role::TEXT_INPUT;
    const INPUT: crate::contract::Input = crate::contract::Input::Text;
    const OVERLAYS: bool = false;
    type Style = ChipFieldStyle;
    type Output = EditResponse<Vec<String>>;

    fn style(cx: &StyleCx<'_>, _: ChipFieldStyleKey) -> ChipFieldStyle {
        let field = FieldStyle::at(cx);
        ChipFieldStyle { surface: field.surface, metrics: field.metrics.gap(cx.control().gap), draft_min: cx.tokens.size.length.short }
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let chips = cx.value().map_or(&[][..], Vec::as_slice);
        let arranged = arrange(cx, chips, cx.room);
        let (_, start, width) = arranged.placement.draft;
        Vec2::new(cx.room.unwrap_or(start + width), arranged.height())
    }

    /// The chips and their remove parts, then the draft; the chip bodies are one roving group.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(chips) = cx.value() else { return };
        let content = cx.content_rect;
        let arranged = arrange(cx, chips, Some(content.width()));
        let rtl = cx.flow.is_rtl();
        let line_rect = |line: usize| {
            let top = content.min.y + line as f32 * (arranged.line + arranged.gap);
            Rect::from_min_size(pos2(content.min.x, top), vec2(content.width(), arranged.line))
        };
        for (at, &(line, start)) in arranged.placement.cells.iter().enumerate() {
            let cell = Flow::new(line_rect(line), rtl).span(start, arranged.placement.widths[at]);
            let cell = Rect::from_center_size(cell.center(), vec2(cell.width(), arranged.side));
            let (body, remove) = Flow::new(cell, rtl).take_start(arranged.bodies[at].min(cell.width()));
            let chip = cx.part(CHIP.at(at as u64), body).sense(Sense::CLICK | Sense::FOCUS).focusable();
            // The chip next to the draft is the group's arrival: Shift+Tab from the draft lands on it.
            if at + 1 == chips.len() {
                chip.selected();
            }
            cx.part(REMOVE.at(at as u64), remove.rect()).sense(Sense::CLICK).over().pointer_only(REMOVE_BY_KEY);
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
    /// focuses the last chip. Delete in an empty draft does nothing.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(chips) = cx.value().cloned() else { return };
        let draft_empty = cx.text(&text_part::<Vec<String>>()).is_none_or(|text| text.draft.as_deref().is_none_or(str::is_empty));
        let on_chip = cx.active_part().filter(|part| part.key == CHIP).and_then(|part| part.index);
        let in_draft = cx.is_focused() && cx.active_part().is_none();
        let backspace = Intent::delete(false, Granularity::Unit);
        let delete = Intent::delete(true, Granularity::Unit);
        let mut removed = (0..chips.len()).find(|&at| {
            // Guard: a commit asks for a new pass, so no chip is undeclared; one that is stays out of this pass.
            cx.part_activated(REMOVE.at(at as u64)).is_some()
        });
        if removed.is_none() {
            if let Some(at) = on_chip {
                if cx.intents().take_exact(&backspace) || cx.intents().take_exact(&delete) {
                    removed = Some(at as usize);
                }
            } else if in_draft && draft_empty && !chips.is_empty() && cx.intents().take_exact(&backspace) {
                cx.focus_part(CHIP.at(chips.len() as u64 - 1));
            }
        }
        if let Some(at) = removed.filter(|&at| at < chips.len()) {
            // Next pass the chip after the removed one sits at `at`, where focus already is; with none, the draft.
            if on_chip.is_some() && at + 1 == chips.len() {
                cx.focus_own();
            }
            let mut rest = chips;
            rest.remove(at);
            cx.set(rest);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(chips) = cx.value() else { return };
        for (at, chip) in chips.iter().enumerate() {
            // Guard: a commit asks for a new pass, so no chip is undeclared; one that is stays out of this pass.
            if cx.part_if_declared(CHIP.at(at as u64)).is_none() {
                continue;
            }
            ui.face(CHIP.at(at as u64), Badge::new(chip.clone()).variant(Variant::SECONDARY));
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

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<Vec<String>> {
        cx.edit_response()
    }
}
