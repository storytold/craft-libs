//! The docked and floating scope of the workspace.

use std::sync::Arc;

use craft_behavior::{EditEvent, EditEvents, Mixed};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, BodyCx, EditResponse, Pattern, Props, SaveKey, Show as _, ShowContainer as _, clicked};
use craft_ui::dialog::Dialog;
use craft_ui::layout_components::Stack;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::sections::HeaderAction;
use egui::Ui;

use super::controls::{LIGHT, Slider, section};
use super::words::RawDevelopWords;
use crate::raw::RawDevelopSettings;
use crate::ui::scopes::{ScopeFrame, ScopePanel, ScopePanelOutput, ScopePanelState, ScopeWords, ToneZone, ZoneScale, ZoneValues};

/// What the docked and the floating scope report: the scope panel's events and the tone-zone edits of the bound
/// settings.
#[derive(Debug)]
#[non_exhaustive]
pub struct RawScopeStripOutput {
    /// The scope panel's events; `None` while the panel is not shown (a collapsed strip, a closed window).
    pub panel: Option<ScopePanelOutput>,
    /// The edits of the settings' tone zones.
    pub edit: EditResponse<RawDevelopSettings>,
}

/// The slider row a tone zone edits: the zone's key is the settings field's name.
fn zone_slider(key: &str) -> Option<&'static Slider> {
    LIGHT.iter().find(|row| row.key == key)
}

/// The zone's value as the tooltip and the screen reader word it.
fn zone_text(row: &Slider, settings: &RawDevelopSettings) -> String {
    let value = (row.get)(settings);
    if row.step < 1.0 { format!("{value:.2}") } else { format!("{value:.0}") }
}

/// How a zone's slider row moves: its range and step, its default as the reset, and a drag across the plot's width
/// that spans twice the range's end (PhotoCraft's tone-zone drag).
fn zone_scale(row: &Slider) -> ZoneScale {
    let limit = row.max as f32;
    ZoneScale::new(row.min as f32..=limit, limit * 2.0, row.step as f32, (row.get)(&RawDevelopSettings::default()))
}

/// The values of the zones' fields in `settings`, in the zones' order.
fn zone_values(settings: &RawDevelopSettings) -> ZoneValues {
    ZoneValues(ToneZone::CAMERA_RAW.iter().map(|zone| zone_slider(zone.key).map_or(0.0, |row| (row.get)(settings))).collect())
}

/// `base` with each zone's field set to its value in `values`.
fn with_zones(base: &RawDevelopSettings, values: &ZoneValues) -> RawDevelopSettings {
    let mut settings = base.clone();
    for (zone, value) in ToneZone::CAMERA_RAW.iter().zip(&values.0) {
        if let Some(row) = zone_slider(zone.key) {
            (row.set)(&mut settings, *value);
        }
    }
    settings
}

/// The scope panel with Camera Raw's tone zones over the bound settings, and the zone edits written back to them.
fn zone_panel(
    ui: &mut Ui,
    cx: &mut BodyCx<'_>,
    state: &mut ScopePanelState,
    frame: Option<&ScopeFrame<'_>>,
    settings: &mut Bind<'_, RawDevelopSettings>,
    has_selection: bool,
) -> RawScopeStripOutput {
    let base = settings.reborrow().read(cx).and_then(|held| held.same().cloned());
    let rows: Vec<Option<&Slider>> = ToneZone::CAMERA_RAW.iter().map(|zone| zone_slider(zone.key)).collect();
    // A zone without a row keeps a range of one value, so it never moves.
    let scales: Vec<ZoneScale> = rows.iter().map(|row| row.map_or(ZoneScale::new(0.0..=0.0, 0.0, 0.0, 0.0), zone_scale)).collect();
    let texts: Vec<Label> = rows
        .iter()
        .map(|row| {
            let text = base.as_ref().zip(*row).map(|(settings, row)| zone_text(row, settings));
            Label::from(text.unwrap_or_default())
        })
        .collect();
    let values = base.as_ref().map_or(Mixed::Mixed, |base| Mixed::Same(zone_values(base)));
    let panel = ScopePanel::new(state);
    // Without a frame the panel draws nothing: the host shows its own "Calculating..." line.
    let panel = match frame {
        Some(frame) => panel.frame(frame),
        None => panel,
    };
    let panel = panel.zones(&ToneZone::CAMERA_RAW, &scales, &texts, Bind::given(values)).has_selection(has_selection).show(ui);
    // The zones' values in the settings around them; nothing edits a mixed value.
    let events = base.as_ref().map(|base| panel.zone.clone().map(|values| with_zones(base, &values))).unwrap_or_default();
    for event in &events {
        settings.reborrow().write(cx, event);
    }
    let edit = EditResponse::lifted(panel.response.clone(), events, None);
    RawScopeStripOutput { panel: Some(panel), edit }
}

/// The output of a scope whose panel is not shown: no panel and no edit, on the response of what holds it.
fn unshown(response: &egui::Response) -> RawScopeStripOutput {
    RawScopeStripOutput { panel: None, edit: EditResponse::lifted(response.clone(), EditEvents::new(), None) }
}

/// The docked scope above the controls: a `ScopePanel` with Camera Raw's tone zones and typical histogram scale, its
/// context menu and a Float action in the section header.
#[must_use = "show the strip with `show`"]
#[derive(Pattern, Props)]
pub struct RawScopeStrip<'a> {
    state: &'a mut ScopePanelState,
    frame: Option<&'a ScopeFrame<'a>>,
    settings: Bind<'a, RawDevelopSettings>,
    /// Whether the scope floats; its Float action sets it.
    #[prop]
    floating: Option<Bind<'a, bool>>,
    /// The picture has a selection: the scope menu offers Show Selected Region.
    #[prop]
    has_selection: bool,
}

impl<'a> RawScopeStrip<'a> {
    /// The strip over `state` and the host's `frame`, showing the zone values of `settings`.
    pub fn new(state: &'a mut ScopePanelState, frame: Option<&'a ScopeFrame<'a>>, settings: Bind<'a, RawDevelopSettings>) -> Self {
        RawScopeStrip { state, frame, settings, floating: None, has_selection: false }
    }

    /// Draws the scope.
    pub fn show(self, ui: &mut Ui) -> RawScopeStripOutput {
        let Self { state, frame, mut settings, mut floating, has_selection } = self;
        let env = Env::of(ui);
        let words = env.words::<RawDevelopWords>();
        let title = env.words::<ScopeWords>().histogram.clone();
        let shown = Stack::new().show(ui, |ui, root| {
            let section = section(&title, "scope", true)
                .actions(floating.is_some().then_some(HeaderAction::Float))
                .show(ui, |ui, cx| zone_panel(ui, cx, &mut *state, frame, &mut settings, has_selection));
            if section.output.action == Some(HeaderAction::Float)
                && let Some(floating) = floating.as_mut()
            {
                let event = EditEvent::set(Mixed::Same(false), true, Arc::from(words.float.as_str()), None);
                floating.reborrow().write(root, &event);
            }
            section.inner
        });
        match shown.inner.flatten() {
            Some(out) => out,
            None => unshown(&shown.output),
        }
    }
}

/// The floating scope: a modeless child dialog of the workspace, resizable and remembered; its Dock action clears
/// `floating`.
#[must_use = "show the scope with `show`"]
#[derive(Pattern, Props)]
pub struct RawFloatingScope<'a> {
    state: &'a mut ScopePanelState,
    frame: Option<&'a ScopeFrame<'a>>,
    floating: Bind<'a, bool>,
    settings: Bind<'a, RawDevelopSettings>,
    /// The picture has a selection: the scope menu offers Show Selected Region.
    #[prop]
    has_selection: bool,
}

impl<'a> RawFloatingScope<'a> {
    /// The floating scope over `state` and the host's `frame`, showing the zone values of `settings`.
    pub fn new(
        state: &'a mut ScopePanelState,
        frame: Option<&'a ScopeFrame<'a>>,
        floating: Bind<'a, bool>,
        settings: Bind<'a, RawDevelopSettings>,
    ) -> Self {
        RawFloatingScope { state, frame, floating, settings, has_selection: false }
    }

    /// Draws the scope. The window's close action and Dock both write `false` to `floating` (the dialog's open state is
    /// the binding).
    pub fn show(self, ui: &mut Ui) -> RawScopeStripOutput {
        let Self { state, frame, floating, mut settings, has_selection } = self;
        let env = Env::of(ui);
        let words = env.words::<RawDevelopWords>();
        let title = env.words::<ScopeWords>().histogram.clone();
        let shown =
            Dialog::new(title).modeless().resizable(true).remember(SaveKey::new("craft_imaging.raw_develop.scope_window")).opened(floating).show(
                ui,
                |ui, cx| {
                    let out = zone_panel(ui, cx, &mut *state, frame, &mut settings, has_selection);
                    if clicked(&Button::new(words.dock.clone()).id_salt("dock").show(ui)) {
                        cx.request_close();
                    }
                    out
                },
            );
        match shown.inner {
            Some(out) => out,
            None => unshown(&shown.output.response),
        }
    }
}
