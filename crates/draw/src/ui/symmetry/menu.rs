//! `SymmetryMenu`: the brush options bar's painting symmetry button and its menu.

use super::model::SymmetryMode;
use craft_behavior::IconId;
use craft_ui::buttons::IconButton;
use craft_ui::contract::{Entry, Outcome, PopupKind, Props as _, Show as _, Trigger};
use craft_ui::menu::Menu;
use craft_ui::runtime::env::{Env, Label};
use egui::{Response, Ui};

craft_ui::words! {
    /// The words of painting symmetry: the menu, the transform's bar and its accessible names.
    pub struct SymmetryWords {
        button: "symmetry.button" = "Set painting symmetry options";
        off: "symmetry.off" = "Symmetry Off";
        vertical: "symmetry.vertical" = "Vertical";
        horizontal: "symmetry.horizontal" = "Horizontal";
        dual: "symmetry.dual" = "Dual Axis";
        diagonal: "symmetry.diagonal" = "Diagonal";
        work_path: "symmetry.workPath" = "Work Path";
        transform: "symmetry.transform" = "Transform Symmetry";
        commit: "symmetry.commit" = "Commit transform";
        cancel: "symmetry.cancel" = "Cancel transform";
        group: "symmetry.group" = "Symmetry";
        center: "symmetry.center" = "Center";
        rotation_handle: "symmetry.rotationHandle" = "Rotation handle";
    }
}

/// The symmetry in force, as the menu marks it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SymmetryChoice {
    /// No symmetry.
    Off,
    /// A straight symmetry of this mode.
    Mode(SymmetryMode),
    /// Symmetry from the path keyed as [`SymmetryPathRow::key`].
    Path(String),
}

/// A path the symmetry can come from (the saved paths, the Work Path, the layer's path), in the app's order.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SymmetryPathRow {
    /// The app's key ("work", "layer", or the saved path's name).
    pub key: String,
    /// The row's text: the path's name, or [`SymmetryWords`]' Work Path.
    pub label: String,
}

impl SymmetryPathRow {
    /// A path keyed `key` and shown as `label`.
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self { key: key.into(), label: label.into() }
    }
}

/// What the user picked in the menu.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SymmetryCommand {
    /// Symmetry Off.
    Off,
    /// A straight mode at the document's centre (the app's command sets it).
    Mode(SymmetryMode),
    /// Symmetry from the path keyed so.
    FromPath(String),
    /// Transform Symmetry: the app opens a [`SymmetryTransform`](super::SymmetryTransform).
    Transform,
}

/// What [`SymmetryMenu`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct SymmetryMenuOutput {
    /// The button's response.
    pub response: Response,
    /// The row picked this pass.
    pub command: Option<SymmetryCommand>,
}

impl Outcome for SymmetryMenuOutput {
    type Value = craft_ui::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The painting symmetry button: an icon button named "Set painting symmetry options", pressed while a symmetry is
/// on, with the current mode's or path's name as its description. A press opens a [`Menu`]: Symmetry Off (checked
/// while off, disabled then), a separator, Vertical, Horizontal, Dual Axis and Diagonal with their glyphs, a separator
/// and one row per path (checked when it is the source) when there are paths, a separator, then Transform Symmetry
/// (enabled only with [`can_transform`](Self::can_transform)).
#[must_use = "show the button with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct SymmetryMenu<'a> {
    label: Label,
    current: &'a SymmetryChoice,
    paths: &'a [SymmetryPathRow],
    can_transform: bool,
}

impl<'a> SymmetryMenu<'a> {
    /// The button named `label` for the symmetry `current`.
    pub fn new(label: impl Into<Label>, current: &'a SymmetryChoice) -> Self {
        Self { label: label.into(), current, paths: &[], can_transform: false }
    }

    /// The paths a symmetry can come from.
    pub fn paths(mut self, rows: &'a [SymmetryPathRow]) -> Self {
        self.paths = rows;
        self
    }

    /// Whether Transform Symmetry is enabled: a straight symmetry is on and a painting tool is picked with nothing
    /// else being edited. The app decides.
    pub fn can_transform(mut self, can: bool) -> Self {
        self.can_transform = can;
        self
    }

    /// The button, and the menu while it is open.
    pub fn show(self, ui: &mut Ui) -> SymmetryMenuOutput {
        let words = Env::of(ui).words::<SymmetryWords>();
        let current = self.current;
        let is_off = matches!(current, SymmetryChoice::Off);

        let mut entries: Vec<Entry<SymmetryCommand>> = vec![Entry::new(SymmetryCommand::Off, words.off.clone()).checked(is_off).enabled(!is_off)];
        entries.push(Entry::separator());
        for mode in SymmetryMode::ALL {
            let (label, icon) = match mode {
                SymmetryMode::Vertical => (&words.vertical, "symmetry_vertical"),
                SymmetryMode::Horizontal => (&words.horizontal, "symmetry_horizontal"),
                SymmetryMode::Dual => (&words.dual, "symmetry_dual"),
                SymmetryMode::Diagonal => (&words.diagonal, "symmetry_diagonal"),
            };
            let chosen = *current == SymmetryChoice::Mode(mode);
            entries.push(Entry::new(SymmetryCommand::Mode(mode), label.clone()).icon(IconId::new(icon)).checked(chosen));
        }
        if !self.paths.is_empty() {
            entries.push(Entry::separator());
        }
        for row in self.paths {
            let chosen = matches!(current, SymmetryChoice::Path(key) if *key == row.key);
            entries.push(Entry::new(SymmetryCommand::FromPath(row.key.clone()), Label::from(row.label.clone())).checked(chosen));
        }
        entries.push(Entry::separator());
        entries.push(Entry::new(SymmetryCommand::Transform, words.transform.clone()).enabled(self.can_transform));

        let value = match current {
            SymmetryChoice::Off => None,
            SymmetryChoice::Mode(mode) => Some(match mode {
                SymmetryMode::Vertical => words.vertical.as_str().to_owned(),
                SymmetryMode::Horizontal => words.horizontal.as_str().to_owned(),
                SymmetryMode::Dual => words.dual.as_str().to_owned(),
                SymmetryMode::Diagonal => words.diagonal.as_str().to_owned(),
            }),
            SymmetryChoice::Path(key) => Some(self.paths.iter().find(|row| row.key == *key).map_or_else(|| key.clone(), |row| row.label.clone())),
        };
        let mut button = IconButton::new(IconId::new("painting_symmetry")).label(words.button.clone()).selected(!is_off).popup(PopupKind::Menu);
        if let Some(value) = value {
            button = button.description(value);
        }
        let button = button.id_salt("button").show(ui).response;
        let picked = Menu::new(self.label).values().opened_by(&button, Trigger::Press).pick(ui, &entries);
        SymmetryMenuOutput { response: button, command: picked.picked }
    }
}
