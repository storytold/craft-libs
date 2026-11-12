//! The look of a row: ListRowStyle, the fill and ink rules of its flags.

use super::placement::Sizes;
use crate::style::{ComponentStyle, Flags, IconLook, StyleColor, StyleCx, SurfaceLook, TextLook};
use craft_behavior::Metrics;
use craft_theme::{Contrast, Rgba, Stateful, States, contrast_ratio, identifiable_outline, over};
use egui::CornerRadius;

/// Least contrast ratio of a selected fill on the panel for it to identify the row alone: standard, high.
const OUTLINE_MIN: (f64, f64) = (3.0, 4.5);

/// The look of a row.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ListRowStyle {
    #[style(surface)]
    pub(super) surface: SurfaceLook,
    #[style(metrics)]
    pub(super) metrics: Metrics,
    /// The label.
    #[style(text)]
    pub(super) text: TextLook,
    /// The row's icon.
    #[style(icon)]
    pub(super) icon: IconLook,
    /// The row's trailing icons.
    #[style(icon)]
    pub(super) trailing_icon: IconLook,
    /// The glyph of a toggle cell and of the link between thumbnails: quiet at rest, the normal ink under the pointer.
    #[style(icon)]
    pub(super) toggle: IconLook,
    /// The detail line.
    #[style(text)]
    pub(super) detail: TextLook,
    /// A text cell.
    #[style(text)]
    pub(super) cell: TextLook,
    /// A text cell of a column that reads muted.
    #[style(text)]
    pub(super) cell_muted: TextLook,
    pub(super) thumb_edge: StyleColor,
    pub(super) thumb_radius: CornerRadius,
    pub(super) sizes: Sizes,
    pub(super) dividers: bool,
}

/// Which rules a row's fill and ink follow.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RowForm {
    /// An item of a list.
    List,
    /// A row of a menu, or an option that runs a command: the lead row is the command highlight.
    Command,
    /// An option of a listbox: the chosen row keeps the selected fill, the lead row the hover fill.
    Option,
}

/// What the row fill depends on.
#[derive(Clone, Copy)]
pub(super) struct FillKey {
    pub(super) striped: bool,
    pub(super) layer: bool,
    pub(super) form: RowForm,
}

/// The fill of a row: a stripe at rest, the hover and pressed tints over it, the selection's
/// fills, and the lead row's hover tint (selected hover on a selected row). An emphasized row that is not selected
/// keeps its hover tint.
pub(super) fn fill_of(cx: &StyleCx<'_>, key: FillKey) -> Stateful<StyleColor> {
    let state = &cx.tokens.color.state;
    let clear = Rgba::TRANSPARENT;
    let rest = if key.striped { state.stripe } else { clear };
    let hover = state.hover;
    let c = |rgba: Rgba| cx.color(rgba);
    // `over` keeps the alpha of what is under it, so over a clear rest the tint is the token itself.
    let tinted = if rest.a() == 0 { hover } else { over(hover, rest) };
    let pressed = if rest.a() == 0 { state.pressed } else { over(state.pressed, rest) };
    let selected_hover = if key.layer { state.selected } else { state.selected_hover };
    let mut fill = Stateful::new(c(rest)).off(States::new(c(rest), c(tinted), c(pressed), c(rest))).on(States::new(
        c(state.selected),
        c(selected_hover),
        c(selected_hover),
        c(state.selected),
    ));
    if key.form != RowForm::List {
        // A command row and an option are active by their lead alone: the pointer moves the lead, so the pointer
        // resting on a row adds nothing, and a chosen option keeps the selected fill.
        fill = fill.off(States::new(c(rest), c(rest), c(rest), c(rest)));
    }
    if key.form == RowForm::Option {
        fill = fill.on(States::new(c(state.selected), c(state.selected), c(state.selected), c(state.selected)));
    }
    for rule in flag_rules(key.form) {
        let rgba = match rule.fill {
            RuleFill::Tinted => tinted,
            RuleFill::Selected => state.selected,
            RuleFill::SelectedHover => selected_hover,
            RuleFill::Accent => cx.tokens.color.accent.solid,
            RuleFill::Hover => hover,
        };
        fill = fill.when(rule.flags, c(rgba));
    }
    fill
}

/// The outline of a selected layer row: only where the selected fill is a tint over the panel, and in high contrast
/// where the fill alone does not show on the panel; otherwise transparent.
pub(super) fn selected_outline(cx: &StyleCx<'_>) -> Rgba {
    let colors = &cx.tokens.color;
    let high = cx.contrast() == Contrast::High;
    let panel = colors.surface.panel;
    let selected = colors.state.selected;
    let min = if high { OUTLINE_MIN.1 } else { OUTLINE_MIN.0 };
    let shows = contrast_ratio(over(selected, panel), panel) >= min;
    let tint = selected.0[3] < 255;
    if tint || (high && !shows) { identifiable_outline(colors.line.control_edge, selected, &[panel], high) } else { Rgba::TRANSPARENT }
}

/// Which fill a flag rule paints.
#[derive(Clone, Copy)]
enum RuleFill {
    Tinted,
    Selected,
    SelectedHover,
    Accent,
    /// The hover token itself: translucent, so it tints the surface under a row of a menu.
    Hover,
}

/// The ink paired with a rule's fill.
#[derive(Clone, Copy)]
enum RuleInk {
    Base,
    OnSelected,
    OnAccent,
}

/// A flag rule of a row: the fill it paints and whether its ink is the selection's paired ink.
struct FlagRule {
    flags: Flags,
    fill: RuleFill,
    ink: RuleInk,
}

/// The flag rules of a row in priority order. Fill and ink both derive from this list, so an ink always pairs with
/// the fill under it: the lead row on a selected row paints the selected hover fill with `text.on_selected`, a
/// selected row shows its selection whether or not it is emphasized, an emphasized row that is not selected keeps its
/// hover-like fill and base ink, and the lead row alone keeps the hover tint. The command
/// form of a menu paints the lead row with `accent.solid` and `text.on_accent`. The option form paints the chosen row
/// with `state.selected` and `text.on_selected`, lead or not, and the lead row alone with `state.hover`.
fn flag_rules(form: RowForm) -> Vec<FlagRule> {
    let emphasized = FlagRule { flags: Flags::EMPHASIZED, fill: RuleFill::Tinted, ink: RuleInk::Base };
    match form {
        RowForm::Command => {
            let emphasized = FlagRule { fill: RuleFill::Hover, ..emphasized };
            return vec![emphasized, FlagRule { flags: Flags::LEAD, fill: RuleFill::Accent, ink: RuleInk::OnAccent }];
        }
        RowForm::Option => {
            return vec![
                FlagRule { flags: Flags::SELECTED, fill: RuleFill::Selected, ink: RuleInk::OnSelected },
                FlagRule { flags: Flags::LEAD, fill: RuleFill::Hover, ink: RuleInk::Base },
            ];
        }
        RowForm::List => {}
    }
    vec![
        FlagRule { flags: Flags::LEAD | Flags::SELECTED, fill: RuleFill::SelectedHover, ink: RuleInk::OnSelected },
        FlagRule { flags: Flags::EMPHASIZED | Flags::SELECTED, fill: RuleFill::Selected, ink: RuleInk::OnSelected },
        emphasized,
        FlagRule { flags: Flags::LEAD, fill: RuleFill::Tinted, ink: RuleInk::Base },
    ]
}

/// The row looks of the collection items, on a `StyleCx`: lists, menus and popup lists share them.
pub trait RowStyleCx {
    /// The fill of a row of a menu or popup list: the active (lead) row is a command highlight in `accent.solid`, and a
    /// row with an open submenu (emphasized) keeps the hover tint.
    fn command_row_surface(&self) -> Stateful<StyleColor>;

    /// The surface of a face of a menu: the command highlight on square corners, as the old menu rows and bar titles
    /// drew it. A face selects it with `SurfaceLook`'s fill from here and `command_row_ink` for its text and icons.
    fn command_row_look(&self) -> SurfaceLook;

    /// The ink of a collection item's text or icon: `base`, the theme's paired `text.on_selected` on the selection
    /// fills, and `text.disabled` on a disabled item.
    fn row_ink(&self, base: Rgba) -> Stateful<StyleColor>;

    /// The ink of a menu or popup list row: `base`, `text.on_accent` on the active row, and `text.disabled` on a
    /// disabled row.
    fn command_row_ink(&self, base: Rgba) -> Stateful<StyleColor>;
}

impl RowStyleCx for StyleCx<'_> {
    fn command_row_surface(&self) -> Stateful<StyleColor> {
        fill_of(self, FillKey { striped: false, layer: false, form: RowForm::Command })
    }

    fn command_row_look(&self) -> SurfaceLook {
        SurfaceLook::new(self.command_row_surface()).radius(egui::CornerRadius::ZERO)
    }

    fn row_ink(&self, base: Rgba) -> Stateful<StyleColor> {
        row_ink_of(self, self.color(base), RowForm::List)
    }

    fn command_row_ink(&self, base: Rgba) -> Stateful<StyleColor> {
        row_ink_of(self, self.color(base), RowForm::Command)
    }
}

/// The ink of a row's toggle and link glyphs: `row_ink_of` on `text.muted`, whose off states are the quiet ink's, so
/// the glyph reads in `text.normal` under the pointer and while pressed. The flag rules keep every fill's paired ink.
pub(super) fn quiet_row_ink(cx: &StyleCx<'_>, form: RowForm) -> Stateful<StyleColor> {
    let mut ink = row_ink_of(cx, cx.color(cx.tokens.color.text.muted), form);
    ink.off = cx.quiet_ink().off;
    ink
}

/// `row_ink` for a base that is already a style colour (a tone). A disabled row's ink is `text.disabled`.
pub(super) fn row_ink_of(cx: &StyleCx<'_>, base: StyleColor, form: RowForm) -> Stateful<StyleColor> {
    let on_selected = cx.color(cx.tokens.color.text.on_selected);
    let on_accent = cx.color(cx.tokens.color.text.on_accent);
    let disabled = cx.color(cx.tokens.color.text.disabled);
    let mut ink = Stateful::new(base).off(States::new(base, base, base, disabled)).on(States::new(on_selected, on_selected, on_selected, disabled));
    for rule in flag_rules(form) {
        ink = ink.when(
            rule.flags,
            match rule.ink {
                RuleInk::Base => base,
                RuleInk::OnSelected => on_selected,
                RuleInk::OnAccent => on_accent,
            },
        );
    }
    ink
}
