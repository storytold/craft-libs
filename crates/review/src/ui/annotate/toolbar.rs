//! The annotation toolbar: [`AnnotationToolbar`] and its style controls.

use craft_behavior::Mixed;
use craft_ui::bars::{Bar, BarKind};
use craft_ui::choice::{RadioGroup, Swatch};
use craft_ui::contract::{Bind, BodyCx, NoValue, Outcome, Props as _, Remember as _, Show as _, ShowContainer as _};
use craft_ui::layout_components::Row;
use craft_ui::popover::Popover;
use craft_ui::rails::{FlyoutButton, LastUsed, RailTool, SlotId, ToolSlot, remembered, shown_tool};
use craft_ui::runtime::env::Env;
use craft_ui::text::NumberField;
use egui::{Response, Ui};

use super::tool::srgba;
use super::{AnnotateCommand, AnnotateTool, AnnotateWords, PALETTE};
use crate::annotate::{AnnotationGroup, AnnotationKind, AnnotationTool};

/// The widths the toolbar's width field accepts, in page units.
const WIDTH_RANGE: std::ops::RangeInclusive<f64> = 1.0..=12.0;
/// The most decimals the width field shows.
const WIDTH_DECIMALS: usize = 1;

/// What [`AnnotationToolbar::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct AnnotationToolbarOutput {
    /// The bar's response.
    pub response: Response,
    /// The tool, the colour or the width changed.
    pub changed: bool,
}

impl Outcome for AnnotationToolbarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Whether the colour palette popover is open.
#[derive(Clone, Copy, Debug, Default)]
struct PaletteOpen(bool);

/// Acrobat's quick bar: Select, then the three groups (Comment, Highlight, Draw), each a button showing its
/// last-used tool with a flyout of the group's tools, then the colour well and the line width of the tool's new
/// annotations ([`AnnotateTool::color`], [`AnnotateTool::width`]); both are disabled for tools that draw nothing
/// (Select, Eraser) and the width for markups. The colour well opens a palette popover.
#[derive(craft_ui::contract::Pattern)]
pub struct AnnotationToolbar<'a> {
    tool: &'a mut AnnotateTool,
}

impl<'a> AnnotationToolbar<'a> {
    /// A toolbar for `tool`, whose `tool`, `color`, `highlight_color` and `width` it edits.
    pub fn new(tool: &'a mut AnnotateTool) -> Self {
        Self { tool }
    }

    /// Shows the bar.
    pub fn show(self, ui: &mut Ui) -> AnnotationToolbarOutput {
        let words = Env::of(ui).words::<AnnotateWords>();
        let annotate = self.tool;
        let before = (annotate.tool, annotate.color, annotate.highlight_color, annotate.width);
        let select = [RailTool::new(AnnotationTool::Select, AnnotateCommand::Select)];
        let slots: Vec<ToolSlot<AnnotationTool>> = AnnotationGroup::ALL
            .iter()
            .enumerate()
            .map(|(n, group)| ToolSlot::new(n as SlotId, group.tools().iter().map(|&tool| RailTool::new(tool, AnnotateCommand::of(tool))).collect()))
            .collect();
        let bar = Bar::new(words.toolbar.clone(), BarKind::Tool).show(ui, |ui, body| {
            let last = body.memory::<LastUsed<AnnotationTool>>().clone();
            let current = annotate.tool;
            let out =
                FlyoutButton::new(&select, &AnnotationTool::Select).label(words.select.clone()).selected(current == AnnotationTool::Select).show(ui);
            if let Some(tool) = out.select {
                annotate.tool = tool;
            }
            for (slot, group) in slots.iter().zip(AnnotationGroup::ALL) {
                let tools: Vec<&RailTool<AnnotationTool>> = slot.tools.iter().collect();
                let shown = shown_tool(&current, slot.id, &tools, &last, |_| true).map_or(current, |tool| tool.id);
                let out = FlyoutButton::new(&slot.tools, &shown)
                    .label(words.group(group).to_owned())
                    .selected(slot.tools.iter().any(|tool| tool.id == current))
                    .show(ui);
                if let Some(tool) = out.select {
                    annotate.tool = tool;
                }
            }
            if let Some(next) = remembered(&slots, &annotate.tool, &last) {
                *body.memory::<LastUsed<AnnotationTool>>() = next;
            }
            style_controls(ui, body, annotate, &words);
        });
        let changed = before != (annotate.tool, annotate.color, annotate.highlight_color, annotate.width);
        AnnotationToolbarOutput { response: bar.output.response, changed }
    }
}

/// The colour well with its palette popover, and the line width field, for the current tool.
fn style_controls(ui: &mut Ui, body: &mut BodyCx<'_>, annotate: &mut AnnotateTool, words: &AnnotateWords) {
    let draws = matches!(annotate.tool, AnnotationTool::Draw(_));
    let highlight = annotate.tool == AnnotationTool::Draw(AnnotationKind::Highlight);
    let markup = matches!(annotate.tool, AnnotationTool::Draw(kind) if kind.is_markup());
    let current = if highlight { annotate.highlight_color } else { annotate.color };

    let well = Swatch::new(words.color.clone(), srgba(current)).enabled(draws).show(ui);
    let mut open = body.memory::<PaletteOpen>().0;
    if well.clicked {
        open = !open;
    }
    let shown: Mixed<u8> = PALETTE.iter().position(|&color| color == current).map_or(Mixed::Mixed, |at| Mixed::Same(at as u8));
    let mut pick = shown;
    Popover::new(words.palette.clone(), Bind::bound(&mut open)).opened_by(&well.response).show(ui, |ui, _| {
        RadioGroup::new(words.palette.clone(), Bind::bound_mixed(&mut pick)).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                for (n, color) in PALETTE.into_iter().enumerate() {
                    Swatch::new(words.palette_color(n).to_owned(), srgba(color)).member(n as u8).show(ui);
                }
            });
        });
    });
    if pick != shown
        && let Mixed::Same(at) = pick
    {
        let color = PALETTE[usize::from(at)];
        if highlight {
            annotate.highlight_color = color;
        } else {
            annotate.color = color;
        }
        open = false;
    }
    body.memory::<PaletteOpen>().0 = open;

    let mut width = Some(annotate.width);
    NumberField::new(words.width.clone(), Bind::bound(&mut width))
        .range(WIDTH_RANGE)
        .step(1.0)
        .commit_decimals(WIDTH_DECIMALS)
        .enabled(draws && !markup)
        .show(ui);
    if let Some(value) = width {
        annotate.width = value.clamp(*WIDTH_RANGE.start(), *WIDTH_RANGE.end());
    }
}
