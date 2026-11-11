//! [`WarpFields`]: the Warp group of a transform's options bar: Split tools, Grid menu, warp style,
//! orientation and Bend, H and V.

use craft_behavior::{EditEvent, IconId};
use craft_ui::buttons::ToggleButton;
use craft_ui::contract::{Bind, Entry, FieldLayout, Props as _, Show as _, ShowContainer as _, Sizing};
use craft_ui::display::Divider;
use craft_ui::form::Field;
use craft_ui::layout_components::Row;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, UnitKind};
use craft_ui::text::NumberField;
use egui::Ui;

use super::model::{WarpMesh, WarpSplit};

craft_ui::words! {
    /// The words of [`WarpFields`] (PhotoCraft's Warp options bar).
    pub struct WarpFieldsWords {
        split: "warpFields.split" = "Split";
        split_crosswise: "warpFields.splitCrosswise" = "Split Warp Crosswise";
        split_vertically: "warpFields.splitVertically" = "Split Warp Vertically";
        split_horizontally: "warpFields.splitHorizontally" = "Split Warp Horizontally";
        grid: "warpFields.grid" = "Grid";
        custom: "warpFields.custom" = "Custom";
        default_grid: "warpFields.defaultGrid" = "Default";
        three: "warpFields.three" = "3 x 3";
        four: "warpFields.four" = "4 x 4";
        five: "warpFields.five" = "5 x 5";
        warp: "warpFields.warp" = "Warp";
        orientation: "warpFields.orientation" = "Change warp orientation";
        bend: "warpFields.bend" = "Bend";
        horizontal: "warpFields.horizontal" = "Horizontal distortion";
        vertical: "warpFields.vertical" = "Vertical distortion";
    }
}

/// How far a knot may lie from an even one and still read as even (PhotoCraft `warp_grid_id`).
const EVEN_KNOT: f64 = 1e-4;

/// The range of Bend, H and V in percent.
const DISTORTION_RANGE: std::ops::RangeInclusive<f64> = -100.0..=100.0;

/// One warp style of the style select, as the app lists them (Photoshop's 15 styles, None and
/// Custom; the styles are the app's warp model).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct WarpStyleEntry {
    /// The app's id ("arc", "custom").
    pub key: String,
    /// The translated name ("Arc").
    pub label: String,
    /// A preset style: the orientation and Bend, H and V act on it.
    pub preset: bool,
    /// The Custom style (the mesh edited by hand): hidden over a type layer.
    pub custom: bool,
}

impl WarpStyleEntry {
    /// A style `key` shown as `label`, neither a preset nor Custom.
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self { key: key.into(), label: label.into(), preset: false, custom: false }
    }

    /// Whether the orientation and Bend, H and V act on the style.
    #[must_use]
    pub fn preset(mut self, preset: bool) -> Self {
        self.preset = preset;
        self
    }

    /// Whether this is the Custom style.
    #[must_use]
    pub fn custom(mut self, custom: bool) -> Self {
        self.custom = custom;
        self
    }
}

/// The Grid menu's reading of a mesh (PhotoCraft `warp_grid_id`): Default is one patch, the
/// numbered entries uniform n × n patches, anything split by hand Custom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WarpGridChoice {
    /// A mesh split by hand.
    Custom,
    /// One patch.
    #[default]
    Default,
    /// 3 × 3 even patches.
    Three,
    /// 4 × 4 even patches.
    Four,
    /// 5 × 5 even patches.
    Five,
}

impl WarpGridChoice {
    /// What `mesh` reads as: as many columns as rows with even knots (within 1e-4), 1, 3, 4 or 5
    /// of them; otherwise Custom.
    pub fn of(mesh: &WarpMesh) -> WarpGridChoice {
        // pc: crates/ui-egui/src/tools/transform_tool/options_bar.rs:480 (`warp_grid_id`).
        let columns = mesh.us.len().saturating_sub(1);
        let rows = mesh.vs.len().saturating_sub(1);
        let even = |knots: &[f64]| {
            let n = knots.len().saturating_sub(1);
            n > 0 && knots.iter().enumerate().all(|(i, v)| (v - i as f64 / n as f64).abs() < EVEN_KNOT)
        };
        if columns != rows || !even(&mesh.us) || !even(&mesh.vs) {
            return WarpGridChoice::Custom;
        }
        match columns {
            1 => WarpGridChoice::Default,
            3 => WarpGridChoice::Three,
            4 => WarpGridChoice::Four,
            5 => WarpGridChoice::Five,
            _ => WarpGridChoice::Custom,
        }
    }

    /// The patches per side a pick asks for: Default 1, Three 3, Four 4, Five 5, Custom none.
    pub fn size(self) -> Option<usize> {
        // pc: crates/ui-egui/src/tools/transform_tool/options_bar.rs:443.
        match self {
            WarpGridChoice::Custom => None,
            WarpGridChoice::Default => Some(1),
            WarpGridChoice::Three => Some(3),
            WarpGridChoice::Four => Some(4),
            WarpGridChoice::Five => Some(5),
        }
    }
}

/// What the group shows: the app's warp as the fields read it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct WarpFieldValues {
    /// The current style's [`WarpStyleEntry::key`].
    pub style: String,
    /// Vertical orientation (the orientation button pressed).
    pub vertical: bool,
    /// Bend in percent, −100 to 100.
    pub bend: f64,
    /// Horizontal distortion in percent, −100 to 100.
    pub horizontal: f64,
    /// Vertical distortion in percent, −100 to 100.
    pub vertical_distortion: f64,
    /// The Grid menu's reading of the mesh.
    pub grid: WarpGridChoice,
}

impl WarpFieldValues {
    /// The values of a warp in style `style`, horizontal, with no bend or distortion and the default grid.
    pub fn new(style: impl Into<String>) -> Self {
        Self { style: style.into(), vertical: false, bend: 0.0, horizontal: 0.0, vertical_distortion: 0.0, grid: WarpGridChoice::Default }
    }
}

/// What the user changed in the group. The app applies it to its warp and reports the result to
/// the session (`TransformEdit::Warp`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum WarpFieldsEdit {
    /// A Split tool armed or disarmed; `None` disarms. View state, not a warp change.
    Split(Option<WarpSplit>),
    /// A Grid pick: an even n × n mesh over the box (Custom is never picked, only read).
    Grid(usize),
    /// A style pick, by [`WarpStyleEntry::key`]. PhotoCraft's rule for the app: Custom keeps the
    /// current surface as a one-patch mesh, None resets to the box, a preset keeps the bend when
    /// coming from a preset and takes 50 otherwise.
    Style(String),
    /// The orientation button.
    #[non_exhaustive]
    Orientation {
        /// The new orientation is vertical.
        vertical: bool,
    },
    /// The Bend field: a scrub or typing run, or a Set.
    Bend(EditEvent<f64>),
    /// The H field.
    Horizontal(EditEvent<f64>),
    /// The V field.
    Vertical(EditEvent<f64>),
}

impl WarpFieldsEdit {
    /// The orientation button's edit: the new orientation is `vertical`.
    pub fn orientation(vertical: bool) -> Self {
        Self::Orientation { vertical }
    }
}

/// What the group reported this pass.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct WarpFieldsOutput {
    /// The edits in the order they happened.
    pub edits: Vec<WarpFieldsEdit>,
}

/// The Warp group of a transform's options bar: the app shows it in the transform's Warp phase and
/// reads the edits from the output.
///
/// - In order: "Split" and three toggle buttons (Crosswise, Vertically, Horizontally; the armed
///   one pressed, a press on it disarms), a divider, "Grid" and a select (Custom, Default,
///   3 x 3, 4 x 4, 5 x 5), a divider, "Warp" and the style select, the orientation toggle
///   (pressed when vertical), and Bend, H, V number fields (−100 to 100 %).
/// - [`on_type`](Self::on_type): Split and Grid draw disabled and Custom is not offered (PhotoCraft
///   #3004).
/// - The orientation and the three fields are always there and enabled only on a preset style.
/// - The group reports and never changes the warp.
#[must_use = "show the group with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct WarpFields<'a> {
    styles: &'a [WarpStyleEntry],
    values: WarpFieldValues,
    /// The armed Split tool, shown pressed.
    #[prop]
    split: Option<WarpSplit>,
    /// The warp is on a type layer: Split and Grid disabled, no Custom.
    #[prop(flag)]
    on_type: bool,
}

impl<'a> WarpFields<'a> {
    /// The group for the app's `styles` showing `values`.
    pub fn new(styles: &'a [WarpStyleEntry], values: WarpFieldValues) -> Self {
        Self { styles, values, split: None, on_type: false }
    }

    /// Shows the group in the flow of `ui` and reports its edits.
    pub fn show(self, ui: &mut Ui) -> WarpFieldsOutput {
        let Self { styles, values, split, on_type } = self;
        let words = Env::of(ui).words::<WarpFieldsWords>();
        let caption = FieldLayout::Beside;
        let mut edits = Vec::new();
        Row::new().show(ui, |ui, _| {
            Row::new().enabled(!on_type).show(ui, |ui, _| {
                Field::new(words.split.clone()).layout(caption).sizing(Sizing::Content).show(ui, |ui, _| {
                    Row::new().show(ui, |ui, _| {
                        let tools = [
                            (WarpSplit::Cross, "split_cross", &words.split_crosswise),
                            (WarpSplit::Vertical, "split_vertical", &words.split_vertically),
                            (WarpSplit::Horizontal, "split_horizontal", &words.split_horizontally),
                        ];
                        for (tool, icon, name) in tools {
                            let armed = split == Some(tool);
                            let button = ToggleButton::new(name.clone(), Bind::given(armed)).icon(IconId::new(icon)).icon_only().show(ui);
                            if button.value().is_some() {
                                edits.push(WarpFieldsEdit::Split(if armed { None } else { Some(tool) }));
                            }
                        }
                    });
                });
                Divider::vertical().show(ui);
                Field::new(words.grid.clone()).layout(caption).sizing(Sizing::Content).show(ui, |ui, _| {
                    let entries = [
                        Entry::new(WarpGridChoice::Custom, words.custom.clone()),
                        Entry::new(WarpGridChoice::Default, words.default_grid.clone()),
                        Entry::new(WarpGridChoice::Three, words.three.clone()),
                        Entry::new(WarpGridChoice::Four, words.four.clone()),
                        Entry::new(WarpGridChoice::Five, words.five.clone()),
                    ];
                    let pick = Select::new(words.grid.clone(), Bind::given(values.grid), &entries).show(ui);
                    if let Some(&picked) = pick.value()
                        && picked != values.grid
                        && let Some(size) = picked.size()
                    {
                        edits.push(WarpFieldsEdit::Grid(size));
                    }
                });
            });
            Divider::vertical().show(ui);
            Field::new(words.warp.clone()).layout(caption).sizing(Sizing::Content).show(ui, |ui, _| {
                let entries: Vec<Entry<String>> = styles
                    .iter()
                    .filter(|entry| !(on_type && entry.custom))
                    .map(|entry| Entry::new(entry.key.clone(), entry.label.clone()))
                    .collect();
                let pick = Select::new(words.warp.clone(), Bind::given(values.style.clone()), &entries).show(ui);
                if let Some(picked) = pick.value()
                    && *picked != values.style
                {
                    edits.push(WarpFieldsEdit::Style(picked.clone()));
                }
            });
            // The orientation and the fields are always there and act only on a preset style.
            let preset = styles.iter().any(|entry| entry.key == values.style && entry.preset);
            Row::new().enabled(preset).show(ui, |ui, _| {
                let button = ToggleButton::new(words.orientation.clone(), Bind::given(values.vertical))
                    .icon(IconId::new("swap_horizontal"))
                    .icon_only()
                    .show(ui);
                if button.value().is_some() {
                    edits.push(WarpFieldsEdit::orientation(!values.vertical));
                }
                type FieldEdit = fn(EditEvent<f64>) -> WarpFieldsEdit;
                let fields: [(_, f64, FieldEdit); 3] = [
                    (&words.bend, values.bend, WarpFieldsEdit::Bend),
                    (&words.horizontal, values.horizontal, WarpFieldsEdit::Horizontal),
                    (&words.vertical, values.vertical_distortion, WarpFieldsEdit::Vertical),
                ];
                for (name, value, edit) in fields {
                    Field::new(name.clone()).layout(caption).sizing(Sizing::Content).show(ui, |ui, _| {
                        let field =
                            NumberField::new(name.clone(), Bind::given(Some(value))).range(DISTORTION_RANGE).unit_kind(UnitKind::Percent).show(ui);
                        for event in field.edit.events.iter().cloned() {
                            edits.push(edit(event.map(|typed| typed.unwrap_or(value))));
                        }
                    });
                }
            });
        });
        WarpFieldsOutput { edits }
    }
}
