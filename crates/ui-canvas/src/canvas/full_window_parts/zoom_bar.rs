//! `FullWindowZoomBar`: the zoom strip under a full-window canvas.

use super::view::FullWindowView;
use crate::buttons::IconButton;
use crate::canvas::ViewCommands;
use crate::contract::{Bind, Entry, NoValue, Outcome, Props as _, Show as _, ShowContainer as _};
use crate::dialog::DialogWords;
use crate::display::{Elide, Progress, Shade, Text};
use crate::layout_components::Row;
use crate::pickers::Select;
use crate::runtime::commands::{CommandState, Commands as _};
use crate::runtime::env::{Env, Label};
use crate::style::TypeRole;
use craft_behavior::{EditEvent, IconId, Mixed};
use craft_theme::ControlSize;
use egui::{Response, Ui};
use std::sync::Arc;

/// The select's value for Fit in View.
const FIT: i64 = 0;

/// The menu's zooms in hundredths of a percent: 12.5 %, 25 %, 50 %, 100 %, 200 %, 400 %.
const MENU: [i64; 6] = [1250, 2500, 5000, 10000, 20000, 40000];

/// A zoom in hundredths of a percent, the select's value for it.
fn hundredths(percent: f64) -> i64 {
    (percent * 100.0).round() as i64
}

/// What the zoom bar reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct FullWindowZoomBarOutput {
    /// The strip's response.
    pub response: Response,
}

impl Outcome for FullWindowZoomBarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The strip under a full-window canvas: Zoom Out, Zoom In, a zoom select, then a progress bar while the preview
/// renders, or a note.
///
/// - **View.** The bar reads and writes the bound [`FullWindowView`]: a button, a pick or a view command the dialog
///   scope delivers (`Cmd+=`, `Cmd+-`, `Cmd+0`, `Cmd+1`) writes the next view through the binding.
/// - **Buttons.** Zoom Out and Zoom In are the [`ViewCommands`] zoom commands: label, chord and tooltip come from the
///   command, and each is disabled where a step would not move. The bar belongs inside a [`FullWindowDialog`],
///   which is the command scope; outside one the buttons are disabled.
/// - **Select.** "Zoom": Fit in View, 12.5 %, 25 %, 50 %, 100 %, 200 % and 400 %; a zoom off the menu (stepped to
///   300 %) is listed as itself in its place among them. A fitted view shows Fit in View.
/// - **Progress or note.** While [`progress`](Self::progress) is set, a progress bar (indeterminate for
///   `Some(None)`); otherwise the [`note`](Self::note) in the faint shade, caption type, cut with an ellipsis.
///
/// [`FullWindowDialog`]: crate::dialog::FullWindowDialog
#[derive(crate::contract::Pattern)]
#[must_use = "show the bar with `show`"]
pub struct FullWindowZoomBar<'a> {
    view: Bind<'a, FullWindowView>,
    progress: Option<Option<f32>>,
    note: Option<Label>,
}

impl<'a> FullWindowZoomBar<'a> {
    /// A zoom bar for `view`, the same view the canvas above it shows. A binding with a kept or write-through source
    /// takes the bar's edits; a given one only shows.
    pub fn new(view: Bind<'a, FullWindowView>) -> Self {
        FullWindowZoomBar { view, progress: None, note: None }
    }

    /// The preview's progress: `None` shows the note, `Some(None)` an indeterminate bar, `Some(Some(f))` a bar at `f`
    /// (0 to 1). `FullWindowStatus::progress` gives it.
    pub fn progress(mut self, progress: Option<Option<f32>>) -> Self {
        self.progress = progress;
        self
    }

    /// Text after the select while no progress shows: "Preview 120 ms".
    pub fn note(mut self, note: impl Into<Label>) -> Self {
        self.note = Some(note.into());
        self
    }

    /// Shows the bar.
    pub fn show(self, ui: &mut Ui) -> FullWindowZoomBarOutput {
        let FullWindowZoomBar { mut view, progress, note } = self;
        let env = Env::of(ui);
        let (words, units) = (env.words::<DialogWords>(), env.units());
        let row = Row::new().show(ui, |ui, cx| {
            let mut bind = view.reborrow();
            let shown = bind.read(cx).and_then(|held| held.same().copied()).unwrap_or_default();
            let mut next = shown;
            while let Some((command, _)) = cx.take_command::<ViewCommands>() {
                next.apply(command);
            }
            for (command, dir) in [(ViewCommands::ZoomOut, -1), (ViewCommands::ZoomIn, 1)] {
                cx.publish(command, CommandState::new().enabled(next.can_step(dir)));
            }
            for (icon, command) in [("minus", ViewCommands::ZoomOut), ("plus", ViewCommands::ZoomIn)] {
                IconButton::command(IconId::new(icon), command).size(ControlSize::Sm).show(ui);
            }
            let current = if next.is_fit() { FIT } else { hundredths(next.zoom_percent()) };
            let mut zooms = MENU.to_vec();
            if current != FIT && !zooms.contains(&current) {
                zooms.push(current);
                zooms.sort_unstable();
            }
            let mut entries = vec![Entry::new(FIT, words.full_window_fit_in_view.clone())];
            entries.extend(zooms.into_iter().map(|zoom| Entry::new(zoom, Label::from(units.zoom_percent(zoom as f64 / 100.0)))));
            let picked = Select::new(words.full_window_zoom.clone(), Bind::given(current), &entries).size(ControlSize::Sm).show(ui);
            if let Some(&hundredths) = picked.value().filter(|picked| **picked != current) {
                if hundredths == FIT {
                    next.apply(ViewCommands::FitOnScreen);
                } else {
                    next.set_device_zoom(hundredths as f64 / 10000.0);
                }
            }
            if next != shown {
                let name: Arc<str> = Arc::from(env.view().resolve(&words.full_window_zoom.clone()).as_str());
                let _ = bind.write(cx, &EditEvent::set(Mixed::Same(shown), next, name, None));
            }
            match (progress, &note) {
                (Some(Some(fraction)), _) => {
                    Progress::new(fraction).label(words.full_window_updating.clone()).show(ui);
                }
                (Some(None), _) => {
                    Progress::indeterminate().label(words.full_window_updating.clone()).show(ui);
                }
                (None, Some(note)) if !note.as_str().is_empty() => {
                    Text::new(note.clone()).role(TypeRole::Caption).shade(Shade::Faint).elide(Elide::End).show(ui);
                }
                _ => {}
            }
        });
        FullWindowZoomBarOutput { response: row.output }
    }
}
