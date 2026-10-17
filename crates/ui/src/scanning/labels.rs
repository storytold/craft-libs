//! The wording the app supplies. Identity never depends on it.

use super::*;

/// Translated labels supplied by the app; identity never depends on their text.
pub struct Labels<'a> {
    pub scanner: &'a str,
    /// Combo text while nothing is selected.
    pub select_scanner: &'a str,
    pub refresh: &'a str,
    /// Hover text for refresh, e.g. that it lists scanners and does not feed paper.
    pub refresh_hint: &'a str,
    pub source: &'a str,
    pub source_auto: &'a str,
    pub source_feeder: &'a str,
    pub source_glass: &'a str,
    pub color: &'a str,
    pub color_gray: &'a str,
    pub color_color: &'a str,
    pub color_black_and_white: &'a str,
    pub resolution: &'a str,
    /// The unit shown after a resolution, e.g. "dpi".
    pub dpi: &'a str,
    pub pages: &'a str,
    pub all_pages: &'a str,
    pub duplex: &'a str,
    /// Text of the hardware-duplex checkbox, e.g. "Both sides".
    pub duplex_both_sides: &'a str,
    pub usb: &'a str,
    pub network: &'a str,
    pub checking: &'a str,
    pub try_again: &'a str,
    pub preset_text: &'a str,
    pub preset_color_document: &'a str,
    pub preset_black_and_white: &'a str,
    pub preset_photo: &'a str,
    /// "The same printer is also available over USB."
    pub also_usb: &'a str,
    /// "The same printer is also available over the network."
    pub also_network: &'a str,
    /// Shown, short, beside a control the device said it does not have, when `reasons` has nothing
    /// more specific, e.g. "The scanner does not offer this."
    pub unsupported_reason: &'a str,
    /// Shown, quietly, beside an enabled control the device did not confirm, e.g. "Not confirmed by
    /// the scanner." The value is then only a hint.
    pub unconfirmed_note: &'a str,
    /// Why a specific option is unsupported, e.g. "No document feeder."
    pub reasons: Reasons<'a>,
    /// The page-length row (feeder only).
    pub length: LengthLabels<'a>,
    /// Status warnings (see [`Warning`]); they never disable a control.
    pub warning_feeder_empty: &'a str,
    pub warning_cover_open: &'a str,
    pub warning_feeder_ready_but_empty: &'a str,
}

/// App-worded reasons per option for [`Capability::Unsupported`]. `None` or empty falls back to
/// [`Labels::unsupported_reason`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Reasons<'a> {
    /// No automatic document feeder (also the reason for the page count and duplex rows).
    pub feeder: Option<&'a str>,
    pub glass: Option<&'a str>,
    pub duplex: Option<&'a str>,
    pub dpi: Option<&'a str>,
    pub colors: Option<&'a str>,
    pub pages: Option<&'a str>,
}

impl<'a> Labels<'a> {
    pub(super) fn connection(&self, connection: Connection) -> Option<&'a str> {
        match connection {
            Connection::Usb => Some(self.usb),
            Connection::Network => Some(self.network),
            Connection::Unknown => None,
        }
    }

    pub(super) fn source(&self, source: Source) -> &'a str {
        match source {
            Source::Auto => self.source_auto,
            Source::Feeder => self.source_feeder,
            Source::Glass => self.source_glass,
        }
    }

    pub(super) fn color(&self, color: Color) -> &'a str {
        match color {
            Color::Gray => self.color_gray,
            Color::Color => self.color_color,
            Color::BlackAndWhite => self.color_black_and_white,
        }
    }

    pub(super) fn preset(&self, preset: Preset) -> &'a str {
        match preset {
            Preset::DocumentText => self.preset_text,
            Preset::ColorDocument => self.preset_color_document,
            Preset::BlackAndWhite => self.preset_black_and_white,
            Preset::Photo => self.preset_photo,
        }
    }

    /// The specific reason when given and not empty, else the general one.
    pub(super) fn reason(&self, specific: Option<&'a str>) -> &'a str {
        specific.filter(|text| !text.is_empty()).unwrap_or(self.unsupported_reason)
    }

    /// The line under a control: the reason when unsupported, the note when unknown, else none.
    pub(super) fn note_for<T>(&self, capability: &Capability<T>, specific: Option<&'a str>) -> &'a str {
        match capability {
            Capability::Unsupported => self.reason(specific),
            Capability::Unknown => self.unconfirmed_note,
            _ => "",
        }
    }

    pub(super) fn warning(&self, warning: Warning) -> &'a str {
        match warning {
            Warning::FeederEmpty => self.warning_feeder_empty,
            Warning::CoverOpen => self.warning_cover_open,
            Warning::FeederReadyButEmpty => self.warning_feeder_ready_but_empty,
        }
    }

    pub(super) fn note(&self, note: SamePrinter) -> &'a str {
        match note {
            SamePrinter::AlsoUsb => self.also_usb,
            SamePrinter::AlsoNetwork => self.also_network,
        }
    }

    /// Combo entry text: name plus a connection tag where the connection is known.
    pub fn device_label(&self, scanner: &Scanner) -> String {
        match self.connection(scanner.connection) {
            Some(tag) => format!("{}  · {tag}", scanner.name),
            None => scanner.name.clone(),
        }
    }
}
