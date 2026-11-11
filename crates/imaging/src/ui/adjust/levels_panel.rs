//! [`LevelsPanel`]: the Levels adjustment around the [`LevelsEditor`]: the Channel select, Auto,
//! the editor over the shown channel's histogram and the point pickers.

#[cfg(feature = "screen-testing")]
mod stories;

use std::sync::Arc;

use craft_behavior::{EditEvent, EditEvents, Mixed};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Entry, NoValue, Outcome, Pattern, Props, Remember as _, Show as _, ShowContainer as _, clicked};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::Env;
use craft_ui::style::TypeRole;
use egui::{Id, Response, Ui};

use super::{AdjustContext, AdjustWords, Levels, LevelsClip, LevelsEditor, ToneChannels, TonePicker, TonePickerRow};
use crate::ui::histogram::{ChannelColor, HistogramData};

impl Levels {
    /// Photoshop's Auto: the black and white inputs clip 0.1 % of the pixels at each end of
    /// `counts` (256 bins over the levels), gamma back to 1, the outputs kept.
    ///
    /// PhotoCraft's rule, copied exactly: `clip = total / 1000` (integer); from the bottom, add
    /// bins while `lo < 253` and the running sum plus the next bin stays `<= clip`; from the top
    /// likewise while `hi > lo + 2`. The loops alone decide: an all-empty histogram gives 253
    /// and 255.
    pub fn auto(&self, counts: &[u32; 256]) -> Levels {
        // pc: crates/ui-egui/src/dialogs/adjust_editors/levels.rs:42-57 `auto_levels`, :78-85 (gamma
        // to 1).
        let total: u64 = counts.iter().map(|&n| u64::from(n)).sum();
        let clip = total / 1000;
        let (mut sum, mut lo) = (0u64, 0usize);
        while lo < 253 && sum + u64::from(counts[lo]) <= clip {
            sum += u64::from(counts[lo]);
            lo += 1;
        }
        let (mut sum, mut hi) = (0u64, 255usize);
        while hi > lo + 2 && sum + u64::from(counts[hi]) <= clip {
            sum += u64::from(counts[hi]);
            hi -= 1;
        }
        Levels { in_black: lo as f64, gamma: 1.0, in_white: hi as f64, ..*self }
    }
}

/// What [`LevelsPanel::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct LevelsPanelOutput {
    /// The panel's response.
    pub response: Response,
    /// The edit of this pass on the per-channel values: the editor's gestures for the shown channel, and Auto as
    /// one `Set`. A binding that writes through has it applied already.
    pub edit: EditEvents<Vec<Levels>>,
    /// The channel the edit is for, and the one shown: an index into the panel's [`ToneChannels`] (Red is 1 in
    /// RGB). The panel keeps it between passes.
    pub channel: usize,
    /// The point picker the user armed (`Some(Some(p))`) or disarmed (`Some(None)`) this pass.
    pub picker: Option<Option<TonePicker>>,
    /// Which input end an Alt drag moves now ([`LevelsEditorOutput::clipping`](super::LevelsEditorOutput)).
    pub clipping: Option<LevelsClip>,
}

impl Outcome for LevelsPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The Levels adjustment: Image › Adjustments › Levels and a Levels layer's Properties panel
/// (the same call with [`AdjustContext::properties`]).
///
/// Top to bottom (PhotoCraft's and Photoshop's order):
/// 1. The Channel select ([`ToneChannels`]; an isolated channel is the only entry, named as given)
///    and Auto. Picking a channel shows that channel's levels and histogram; the choice is the
///    panel's own state, reported as [`LevelsPanelOutput::channel`].
/// 2. The [`LevelsEditor`] for the shown channel, over its histogram drawn in the channel's colour;
///    a [`stale`](Self::stale) histogram shows the stale mark.
/// 3. With [`pickers`](Self::pickers), the [`TonePickerRow`] below: Set Black, Neutral Gray, White
///    Point.
///
/// - **Auto** sets the shown channel to [`Levels::auto`] of its histogram as one `Set`; without a
///   histogram it is disabled.
/// - **Edits**: the editor's events pass through for the shown channel, mapped onto the per-channel
///   values.
/// - **Keyboard**: Tab visits the Channel select, Auto, the editor, then the pickers.
#[must_use = "show the panel with `show`"]
#[derive(Pattern, Props)]
pub struct LevelsPanel<'a> {
    label: craft_ui::runtime::env::Label,
    levels: Bind<'a, Vec<Levels>>,
    channels: ToneChannels,
    /// The host's histograms, one per channel (same index), 256 bins each. Without them the editor draws no
    /// histogram and Auto is disabled.
    #[prop]
    histograms: &'a [[u32; 256]],
    /// The histograms are being recounted.
    #[prop]
    stale: bool,
    /// The armed point picker; the picker row shows when it is given.
    #[prop]
    pickers: Option<Bind<'a, Option<TonePicker>>>,
    /// The document canvas the armed picker samples.
    #[prop]
    canvas: Option<Id>,
    picker_enabled: [bool; 3],
    /// The document's mode has no point pickers: all disabled, with the note.
    #[prop(flag)]
    pickers_unavailable: bool,
    /// Dialog or Properties panel.
    #[prop]
    context: AdjustContext,
    /// Whether the whole panel takes input.
    #[prop]
    enabled: bool,
}

impl<'a> LevelsPanel<'a> {
    /// The panel named `label` over `levels`, one value per channel of `channels` (same index).
    pub fn new(label: impl Into<craft_ui::runtime::env::Label>, levels: Bind<'a, Vec<Levels>>, channels: ToneChannels) -> Self {
        Self {
            label: label.into(),
            levels,
            channels,
            histograms: &[],
            stale: false,
            pickers: None,
            canvas: None,
            picker_enabled: [true; 3],
            pickers_unavailable: false,
            context: AdjustContext::dialog(true),
            enabled: true,
        }
    }

    /// Enables or disables one picker ([`TonePicker::supported`] for the document's mode).
    pub fn picker_enabled(mut self, picker: TonePicker, enabled: bool) -> Self {
        self.picker_enabled[picker as usize] = enabled;
        self
    }

    /// Draws the panel and reports the shown channel's edit.
    pub fn show(self, ui: &mut Ui) -> LevelsPanelOutput {
        // pc: crates/ui-egui/src/dialogs/adjust_editors/levels.rs:63-85 (Channel select, Auto at the
        // row's end, Auto from the shown channel's histogram), crates/ui-egui/src/dialogs/adjust_dialog.rs:186-216
        // (the picker row in the dialog).
        let Self { label, mut levels, channels, histograms, stale, mut pickers, canvas, picker_enabled, pickers_unavailable, context, enabled } =
            self;
        let words = Env::of(ui).words::<AdjustWords>();
        let entries: Vec<Entry<usize>> = channels.as_slice().iter().enumerate().map(|(at, channel)| Entry::new(at, channel.label(&words))).collect();
        let (mut edit, mut picker, mut clipping) = (EditEvents::new(), None, None);
        let mut shown = 0;
        let group = Stack::new().label(label).enabled(enabled).show(ui, |ui, cx| {
            let all: Vec<Levels> = match levels.reborrow().read(cx) {
                Some(Mixed::Same(all)) => all,
                _ => Vec::new(),
            };
            let channel = (*cx.keep::<usize>()).min(channels.len().saturating_sub(1));
            let value = all.get(channel).copied().unwrap_or_else(|| Levels::identity(0.0..=255.0));
            let counts = histograms.get(channel);
            let color = channels.as_slice().get(channel).map_or(ChannelColor::Neutral, |c| c.color());
            let data = counts.map(|counts| HistogramData::tone(counts, color));
            let name = channels.as_slice().get(channel).map(|c| c.label(&words)).unwrap_or(craft_ui::runtime::env::Label::Empty);

            let mut picked = channel;
            let mut auto = false;
            Row::new().show(ui, |ui, _| {
                Text::new(words.channel.clone()).shade(Shade::Muted).role(TypeRole::Label).show(ui);
                let select = Select::new(words.channel.clone(), Bind::given(channel), &entries).show(ui);
                if let Some(&at) = select.value() {
                    picked = at;
                }
                auto = clicked(&Button::new(words.auto.clone()).enabled(counts.is_some()).show(ui));
            });

            let mut editor = LevelsEditor::new(name, Bind::given(value)).channel(color).stale(stale);
            if let Some(data) = &data {
                editor = editor.histogram(data);
            }
            let shown_edit = editor.show(ui);
            clipping = shown_edit.clipping;
            let mapped = |one: Levels| {
                let mut next = all.clone();
                if let Some(slot) = next.get_mut(channel) {
                    *slot = one;
                }
                next
            };
            let mut events = shown_edit.edit.events.map(mapped);
            if let (true, Some(counts)) = (auto, counts) {
                events = EditEvent::set(Mixed::Same(all.clone()), mapped(value.auto(counts)), Arc::from("Auto"), None).into();
            }
            for event in &events {
                levels.reborrow().write(cx, event);
            }
            edit = events;

            if let Some(armed) = pickers.as_mut() {
                let mut row = TonePickerRow::new(armed.reborrowed());
                if pickers_unavailable {
                    row = row.unavailable();
                }
                if let Some(canvas) = canvas {
                    row = row.canvas(canvas);
                }
                for each in TonePicker::ALL {
                    // A grey document has no Neutral Gray point.
                    let usable = picker_enabled[each as usize] && !(each == TonePicker::Gray && (channels.is_gray() || context.is_gray()));
                    row = row.enabled_picker(each, usable);
                }
                picker = row.show(ui).change;
            }
            // An edit belongs to the channel it was drawn for; a switch shows from the next pass.
            shown = if picked == channel || !edit.is_empty() { channel } else { picked };
            *cx.keep::<usize>() = shown;
        });
        LevelsPanelOutput { response: group.output, edit, channel: shown, picker, clipping }
    }
}
