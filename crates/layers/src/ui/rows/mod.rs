//! `BlendRows`: Blend mode, Opacity, Lock and Fill as one block.

#[cfg(feature = "screen-testing")]
mod stories;

use craft_behavior::{EditEvent, EditEvents, IconId};
use craft_theme::ControlSize;
use craft_ui::buttons::ToggleButton;
use craft_ui::contract::{Bind, Entry, NoValue, Outcome, Props as _, RowAlign, Show as _, ShowContainer as _, SideLabel, TooltipSpec};
use craft_ui::display::{Shade, Text};
use craft_ui::form::Field;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::text::NumberField;
use egui::{Response, Ui};

use super::tree::{LayerProperties, LockKind};

/// The rows sit in a panel, so their controls take the small size.
const SIZE: ControlSize = ControlSize::Sm;

/// An edit of the active layer's properties, which the app applies to every selected layer (the
/// Background left out of a multi-layer change, as PhotoCraft).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LayerPropertyEdit {
    /// The blend mode, by the app's key. While the list is open the hovered mode is proposed
    /// (`Update`, which the app previews on the canvas) and closing the list without a pick is a
    /// `Cancel`; a pick in the list commits, and an arrow key or a wheel notch over the closed
    /// select is a `Set`.
    Blend(EditEvent<String>),
    /// Opacity, 0 to 100: a scrub or slider drag is a gesture (`Begin`, `Update`, `Commit` or
    /// `Cancel`, Escape restores), typing, an arrow step and a wheel step are `Set`.
    Opacity(EditEvent<f64>),
    /// Fill, 0 to 100, as Opacity.
    Fill(EditEvent<f64>),
    /// A lock button: the lock `kind` asks to be `on`.
    #[non_exhaustive]
    Lock {
        /// The lock.
        kind: LockKind,
        /// The state asked for.
        on: bool,
    },
}

craft_ui::words! {
    /// The words of the blend, opacity, lock and fill rows.
    pub struct BlendRowsWords {
        /// The blend select's name.
        blend_mode: "blendRows.blendMode" = "Blend Mode";
        /// The caption and name of the Opacity field.
        opacity: "blendRows.opacity" = "Opacity";
        /// The caption and name of the Fill field.
        fill: "blendRows.fill" = "Fill";
        /// The caption before the lock buttons.
        lock: "blendRows.lock" = "Lock";
        /// The lock buttons' tooltips and names, in [`LockKind::ALL`] order.
        lock_transparency: "blendRows.lockTransparency" = "Lock transparent pixels";
        lock_pixels: "blendRows.lockPixels" = "Lock image pixels";
        lock_position: "blendRows.lockPosition" = "Lock position";
        lock_artboard: "blendRows.lockArtboard" = "Prevent auto-nesting in and out of Artboards and Frames";
        lock_all: "blendRows.lockAll" = "Lock all";
        /// Why Lock transparent pixels is disabled while Lock image pixels is on.
        transparency_included: "blendRows.lockTransparencyIncluded" = "Included in Lock image pixels";
        /// Why a lock is disabled while Lock all is on.
        all_included: "blendRows.lockAllIncluded" = "Included in Lock all";
        /// Why the controls are disabled with no layer.
        no_layer: "blendRows.noLayer" = "No layer is selected";
        /// Why the blend mode, Opacity and Fill are disabled on the Background.
        background: "blendRows.background" = "The Background layer has no blend mode, opacity or fill";
    }
}

/// Blend mode, Opacity, Lock and Fill as one block: the two rows above the Layers panel's list,
/// usable alone (a Properties panel).
///
/// - **Row 1**: the blend [`Select`] with `blend_modes` and their separators, then Opacity as a
///   percent field with its pop-up slider. A wheel notch over the closed select steps one mode;
///   hovering a mode in the open list proposes it (`LayerPropertyEdit::Blend` with an `Update`).
/// - **Row 2** (with [`locks`](Self::locks)): "Lock", five toggle buttons in [`LockKind::ALL`]
///   order, then Fill as Opacity's field. Lock image pixels disables Lock transparent pixels. Lock
///   all shows the other four on and disabled.
/// - **Caption**: a drag on the "Opacity" or "Fill" caption scrubs its field.
/// - **States**: with no properties the rows show Normal, 100 %, unlocked, all disabled with a
///   reason. With `editable` false (the Background) the select, Opacity and Fill are disabled with
///   a reason; the locks stay.
/// - **Values**: the app passes its value back each pass; a field shows the value it is given.
/// - **Keyboard**: each control is a Tab stop; the fields take arrows (with Shift by 10) and
///   typing; Escape during a scrub cancels it.
#[must_use = "show the rows with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct BlendRows<'a> {
    properties: Option<&'a LayerProperties>,
    blend_modes: &'a [Entry<String>],
    /// Whether the Lock and Fill row shows; the default is true.
    #[prop]
    locks: bool,
}

/// What [`BlendRows::show`] returns.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the edits"]
pub struct BlendRowsOutput {
    /// The block.
    pub response: Response,
    /// The edits of this pass, in order.
    pub events: Vec<LayerPropertyEdit>,
}

impl Outcome for BlendRowsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl<'a> BlendRows<'a> {
    /// The rows for the active layer's `properties` (`None`: no active layer), offering
    /// `blend_modes`.
    pub fn new(properties: Option<&'a LayerProperties>, blend_modes: &'a [Entry<String>]) -> Self {
        Self { properties, blend_modes, locks: true }
    }

    /// Draws the rows over the available width.
    pub fn show(self, ui: &mut Ui) -> BlendRowsOutput {
        let Self { properties, blend_modes, locks } = self;
        let words = Env::of(ui).words::<BlendRowsWords>();
        let fallback = LayerProperties::new("normal");
        let live = properties.is_some();
        let props = properties.unwrap_or(&fallback);
        let editable = live && props.editable;
        let no_layer = words.no_layer.clone();
        let unavailable = if live { words.background.clone() } else { no_layer.clone() };

        let mut events = Vec::new();
        let block = Stack::new().fill_width().show(ui, |ui, _| {
            // Opacity ends the blend row and Fill ends the lock row, as in Photoshop.
            Row::new().fill_width().align(RowAlign::SpaceBetween).show(ui, |ui, _| {
                let blend_name = words.blend_mode.clone();
                let picked = Select::new(blend_name.clone(), Bind::given(props.blend_key.clone()), blend_modes)
                    .preview()
                    .size(SIZE)
                    .enabled(editable)
                    .tooltip(TooltipSpec::new(blend_name).disabled_text(unavailable.clone()))
                    .show(ui);
                events.extend(picked.events.into_iter().map(LayerPropertyEdit::Blend));
                let caption = words.opacity.clone();
                events.extend(percent_field(ui, &caption, props.opacity, editable, &unavailable).into_iter().map(LayerPropertyEdit::Opacity));
            });
            if !locks {
                return;
            }
            Row::new().fill_width().align(RowAlign::SpaceBetween).show(ui, |ui, _| {
                Row::new().show(ui, |ui, _| {
                    Text::new(words.lock.clone()).shade(Shade::Muted).show(ui);
                    let names = [&words.lock_transparency, &words.lock_pixels, &words.lock_position, &words.lock_artboard, &words.lock_all];
                    let icons = ["grid", "brush", "move", "scan", "lock"];
                    for ((kind, name), icon) in LockKind::ALL.into_iter().zip(names).zip(icons) {
                        let held = props.locks;
                        let included = kind != LockKind::All && held.all;
                        let reason = if included {
                            Some(&words.all_included)
                        } else if kind == LockKind::Transparency && held.pixels {
                            Some(&words.transparency_included)
                        } else {
                            None
                        };
                        let name = name.clone();
                        let button = ToggleButton::new(name.clone(), Bind::given(held.get(kind) || included))
                            .icon(IconId::new(icon))
                            .icon_only()
                            .size(SIZE)
                            .enabled(live && reason.is_none())
                            .tooltip(TooltipSpec::new(name).disabled_text(reason.map_or_else(|| no_layer.clone(), |reason| reason.clone())))
                            .show(ui);
                        if let Some(on) = button.value() {
                            events.push(LayerPropertyEdit::Lock { kind, on: *on });
                        }
                    }
                });
                let caption = words.fill.clone();
                events.extend(percent_field(ui, &caption, props.fill, editable, &unavailable).into_iter().map(LayerPropertyEdit::Fill));
            });
        });
        BlendRowsOutput { response: block.output, events }
    }
}

/// A 0 to 100 % field with its pop-up slider and a caption that scrubs it; returns its edits.
fn percent_field(ui: &mut Ui, caption: &Label, value: f64, enabled: bool, reason: &Label) -> EditEvents<f64> {
    let mut edit = EditEvents::new();
    Field::new(caption.clone()).side(SideLabel::new().scrub(true)).show(ui, |ui, _| {
        let field = NumberField::new(caption.clone(), Bind::given(Some(value)))
            .range(0.0..=100.0)
            .unit_kind(UnitKind::Percent)
            .popup_slider(0.0..=100.0)
            .scrub_by_label()
            .size(SIZE)
            .enabled(enabled)
            .tooltip(TooltipSpec::new(caption.clone()).disabled_text(reason.clone()))
            .show(ui);
        edit = field.edit.events.map(|typed| typed.unwrap_or(value));
    });
    edit
}
