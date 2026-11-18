//! `Keytips`: the Alt and F10 key hints of the ribbon.

use std::marker::PhantomData;
use std::sync::Arc;

use craft_keymap::CommandRef;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::inspect;
use egui::Ui;

/// Alt and F10 keytips with one- and two-letter badges, placed at the rect of each element of the
/// given inspected-region records whose `command` matches, from this pass's records. Suppressed in
/// text fields and modals; fires with `FireSource::Keytip`.
#[must_use = "show the keytips with `show`"]
pub struct Keytips<'a> {
    stub: PhantomData<&'a ()>,
}

impl<'a> Keytips<'a> {
    /// Keytips that react to the keys when `enabled`.
    pub fn new(enabled: bool) -> Self {
        let _ = enabled;
        todo!("rb.R14")
    }

    /// The ribbon's tabs, each with its name and its badge.
    pub fn tabs(self, tabs: &'a [(Label, &'a str)]) -> Self {
        let _ = (self, tabs);
        todo!("rb.R14")
    }

    /// The controls that get a badge.
    pub fn controls(self, controls: &'a [KeytipControl<'a>]) -> Self {
        let _ = (self, controls);
        todo!("rb.R14")
    }

    /// Fixed badge sequences and the command each fires.
    pub fn sequences(self, sequences: &'a [(&'a str, CommandRef)]) -> Self {
        let _ = (self, sequences);
        todo!("rb.R14")
    }

    /// The commands the digit badges fire, in order.
    pub fn digits(self, digits: &'a [CommandRef]) -> Self {
        let _ = (self, digits);
        todo!("rb.R14")
    }

    /// Letters no badge may use.
    pub fn reserved(self, reserved: &'a [char]) -> Self {
        let _ = (self, reserved);
        todo!("rb.R14")
    }

    /// The inspected-region records whose controls the badges are placed at.
    pub fn places(self, regions: &'a [Arc<inspect::Recorded>]) -> Self {
        let _ = (self, regions);
        todo!("rb.R14")
    }

    /// Shows the badges and reads the keys.
    pub fn show(self, ui: &mut Ui) -> KeytipsOutput {
        let _ = (self, ui);
        todo!("rb.R14")
    }
}

/// A control that gets a keytip.
pub struct KeytipControl<'a> {
    /// The control's command.
    pub command: CommandRef,
    /// The text the badge letters come from.
    pub label: &'a str,
    /// The letters the app fixes for the badge, when it does.
    pub letters: Option<&'a str>,
}

/// What the keytips report for a pass.
pub struct KeytipsOutput {
    /// The tab the keys chose, by its badge.
    pub tab: Option<String>,
    /// The command the keys fired.
    pub fired: Option<CommandRef>,
    /// The badge letters typed so far.
    pub pending: Option<String>,
    /// Whether the keytips are showing.
    pub active: bool,
}
