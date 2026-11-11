//! [`SliceBar`]: the members the Slice tools add to the app's options `Bar`.

use super::{SliceCommand, SliceMode, SliceToolSettings, SliceWords};
use craft_behavior::PixelSize;
use craft_ui::bars::{BarItem, BarItems, ConstraintFields, ItemDisplay};
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Entry, NoValue, Outcome, Props as _, Show as _};
use craft_ui::display::Divider;
use craft_ui::runtime::env::{Env, UnitKind};
use egui::{Response, Ui};

/// The Fixed Size unit, the units menu's list, the document's resolution and its size.
#[derive(Clone, Copy)]
struct SizeUnits<'a> {
    unit: UnitKind,
    units: &'a [Entry<UnitKind>],
    pixels_per_inch: f64,
    document: PixelSize,
}

/// What [`SliceBar`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct SliceBarOutput {
    /// The members' response.
    pub response: Response,
    /// The Style, Width, Height or Hide Auto Slices changed: tool settings, no undo step. The settings the bar was
    /// given hold the new values.
    pub settings_changed: bool,
    /// A unit picked from a Fixed Size field's menu; reported only with [`SliceBar::size_units`].
    pub unit_picked: Option<UnitKind>,
}

impl Outcome for SliceBarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The members a Slice tool adds to the options bar.
///
/// - Slice: the Style, Width, swap and Height cells ([`ConstraintFields`]) over [`SliceToolSettings::style`], a rule,
///   Slices From Guides. Fixed Size shows in the units of [`size_units`](Self::size_units).
/// - Select: Promote, Divide…, Delete, a rule, the Hide Auto Slices check box, a rule, Slice Options….
/// - The buttons are the commands of [`SliceCommand`]: the app gates them with [`SliceCommand::enabled`] as command
///   state, and takes them when they run.
/// - Keyboard: each member is a Tab stop in the bar's order.
#[must_use = "show the members with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct SliceBar<'a> {
    mode: SliceMode,
    settings: &'a mut SliceToolSettings,
    size_units: Option<SizeUnits<'a>>,
}

impl<'a> SliceBar<'a> {
    /// The members of `mode` over `settings`, which the Style, Width, Height and Hide Auto Slices write through.
    pub fn new(mode: SliceMode, settings: &'a mut SliceToolSettings) -> Self {
        SliceBar { mode, settings, size_units: None }
    }

    /// Shows Fixed Size in `unit`, with `units` in the units menu (the app's ruler units), converting through the
    /// document's resolution `pixels_per_inch` and its size `document`. Without it Fixed Size is in pixels with no menu.
    pub fn size_units(mut self, unit: UnitKind, units: &'a [Entry<UnitKind>], pixels_per_inch: f64, document: PixelSize) -> Self {
        self.size_units = Some(SizeUnits { unit, units, pixels_per_inch, document });
        self
    }

    /// Adds the members to the bar's row.
    pub fn show(self, ui: &mut Ui) -> SliceBarOutput {
        let SliceBar { mode, settings, size_units } = self;
        let words = Env::of(ui).words::<SliceWords>();
        let text = |commands: &[SliceCommand]| {
            BarItems::new(commands.iter().map(|&command| BarItem::new(command).display(ItemDisplay::Text)).collect()).label(words.commands.clone())
        };
        let settings_changed;
        let mut unit_picked = None;
        let response = match mode {
            SliceMode::Slice => {
                let before = settings.style;
                let mut style = before;
                let mut fields = ConstraintFields::new(Bind::bound(&mut style));
                if let Some(SizeUnits { unit, units, pixels_per_inch, document }) = size_units {
                    fields = fields.size_units(unit, units, pixels_per_inch, document);
                }
                let shown = fields.show(ui);
                settings_changed = shown.changed;
                unit_picked = shown.unit_picked;
                settings.style = style;
                Divider::vertical().show(ui);
                text(&[SliceCommand::FromGuides]).show(ui)
            }
            SliceMode::Select => {
                text(&[SliceCommand::Promote, SliceCommand::Divide, SliceCommand::Delete]).show(ui);
                Divider::vertical().show(ui);
                let hide = Checkbox::new(Bind::bound(&mut settings.hide_auto)).label(words.hide_auto.clone()).show(ui);
                settings_changed = hide.value().is_some();
                Divider::vertical().show(ui);
                text(&[SliceCommand::Options]).show(ui)
            }
        };
        SliceBarOutput { response, settings_changed, unit_picked }
    }
}
