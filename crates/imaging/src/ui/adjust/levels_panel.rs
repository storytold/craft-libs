//! [`LevelsPanel`]: the Levels adjustment around the [`LevelsEditor`]: the point pickers, the
//! Channel select, Auto and the editor over the shown channel's histogram.

#[cfg(feature = "screen-testing")]
mod stories;

use std::sync::Arc;

use craft_behavior::{EditEvent, EditEvents, Mixed};
use craft_ui::buttons::Button;
use craft_ui::contract::{
    Bind, CommandSet as _, Commands as _, Entry, NoValue, Outcome, Pattern, Props, Remember as _, Show as _, ShowContainer as _, Sizing, clicked,
};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::pickers::Select;
use craft_ui::runtime::commands::CommandRef;
use craft_ui::runtime::env::Env;
use craft_ui::style::TypeRole;
use egui::{Id, Response, Ui};

use super::context::{channel_commands, read_shown, write_shown};
use super::{AdjustContext, AdjustWords, Levels, LevelsClip, LevelsEditor, ToneChannelCommands, ToneChannels, TonePicker, TonePickerRow};
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
/// Top to bottom (PhotoCraft's order):
/// 1. With [`pickers`](Self::pickers), the [`TonePickerRow`]: Set Black, Neutral Gray, White Point.
/// 2. The Channel select ([`ToneChannels`]; an isolated channel is the only entry, named as given)
///    and Auto. Picking a channel shows that channel's levels and histogram; the choice is the
///    panel's own state, reported as [`LevelsPanelOutput::channel`].
/// 3. The [`LevelsEditor`] for the shown channel, over its histogram drawn in the channel's colour;
///    a [`stale`](Self::stale) histogram shows the stale mark.
///
/// - **Auto** sets the shown channel to [`Levels::auto`] of its histogram as one `Set`; without a
///   histogram it is disabled.
/// - **Edits**: the editor's events pass through for the shown channel, mapped onto the per-channel
///   values.
/// - **Keyboard**: Tab visits the pickers, the Channel select, Auto, then the editor.
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
    /// The armed point picker; the picker row shows, first, when it is given.
    #[prop]
    pickers: Option<Bind<'a, Option<TonePicker>>>,
    /// The document canvas the armed picker samples.
    #[prop]
    canvas: Option<Id>,
    /// The shown channel, kept by the host dialog; without it the panel keeps its own.
    #[prop]
    channel: Option<Bind<'a, usize>>,
    picker_enabled: [bool; 3],
    /// The document's mode has no point pickers: all disabled, with the note.
    #[prop(flag)]
    pickers_unavailable: bool,
    /// Dialog or Properties panel.
    #[prop]
    context: AdjustContext,
    /// The commands that pick the Channel select's entries, by position, when the host runs them itself (a Properties
    /// panel): the select shows their keys beside the entries. A dialog's panel runs and shows its own
    /// [`ToneChannelCommands`].
    #[prop]
    channel_keys: Option<&'a [CommandRef]>,
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
            channel: None,
            picker_enabled: [true; 3],
            pickers_unavailable: false,
            context: AdjustContext::dialog(true),
            channel_keys: None,
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
        let Self {
            label,
            mut levels,
            channels,
            histograms,
            stale,
            mut pickers,
            canvas,
            channel: mut channel_bind,
            picker_enabled,
            pickers_unavailable,
            context,
            channel_keys,
            enabled,
        } = self;
        let env = Env::of(ui);
        let words = env.words::<AdjustWords>();
        let os = env.view().shell().os;
        let key_commands = channel_commands(channel_keys, context);
        let entries: Vec<Entry<usize>> = channels.as_slice().iter().enumerate().map(|(at, channel)| Entry::new(at, channel.label(&words))).collect();
        let (mut edit, mut picker, mut clipping) = (EditEvents::new(), None, None);
        let mut shown = 0;
        let stack = Stack::new().label(label).enabled(enabled);
        // Alt+2…Alt+6 pick the Channel select's entry in a dialog; Properties has no such keys.
        let stack = if context.is_dialog() { stack.command_scope(ToneChannelCommands::SCOPE) } else { stack };
        let group = stack.show(ui, |ui, cx| {
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

            let all: Vec<Levels> = match levels.reborrow().read(cx) {
                Some(Mixed::Same(all)) => all,
                _ => Vec::new(),
            };
            let stored = read_shown(&mut channel_bind, cx).unwrap_or_else(|| *cx.keep::<usize>());
            let mut commanded = stored;
            if context.is_dialog() {
                while let Some((command, _)) = cx.take_command::<ToneChannelCommands>() {
                    commanded = channels.entry_of(command).unwrap_or(commanded);
                }
            }
            let channel = commanded.min(channels.len().saturating_sub(1));
            let value = all.get(channel).copied().unwrap_or_else(|| Levels::identity(0.0..=255.0));
            let counts = histograms.get(channel);
            let color = channels.as_slice().get(channel).map_or(ChannelColor::Neutral, |c| c.color());
            let data = counts.map(|counts| HistogramData::tone(counts, color));
            let name = channels.as_slice().get(channel).map(|c| c.label(&words)).unwrap_or(craft_ui::runtime::env::Label::Empty);

            let mut picked = channel;
            let mut auto = false;
            // Each entry shows the key that picks it, as the keymap has it now (Photoshop's Alt+2 to Alt+5).
            let keys: Vec<String> = key_commands
                .iter()
                .map(|command| craft_ui::runtime::commands::chord(ui.ctx(), command).map(|chord| chord.display(os)).unwrap_or_default())
                .collect();
            Row::new().show(ui, |ui, _| {
                // No channels (an action whose target is unresolved): no Channel select.
                if !entries.is_empty() {
                    Text::new(words.channel.clone()).shade(Shade::Muted).role(TypeRole::Label).show(ui);
                    let keys = keys.clone();
                    let select = Select::new(words.channel.clone(), Bind::given(channel), &entries)
                        .sizing(Sizing::Content)
                        .decorate(move |at, row| match keys.get(*at) {
                            Some(key) if !key.is_empty() => row.trailing(key.clone()),
                            _ => row,
                        })
                        .show(ui);
                    if let Some(&at) = select.value() {
                        picked = at;
                    }
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

            // An edit belongs to the channel it was drawn for; a switch shows from the next pass.
            shown = if picked == channel || !edit.is_empty() { channel } else { picked };
            write_shown(&mut channel_bind, cx, stored, shown);
            if channel_bind.is_none() {
                *cx.keep::<usize>() = shown;
            }
        });
        LevelsPanelOutput { response: group.output, edit, channel: shown, picker, clipping }
    }
}
