//! `TransformOverlay`: the open session's box on the canvas, which shows a [`TransformBox`](super::TransformBox)
//! and turns its reports into [`TransformEdit`]s, and `TransformTool`: the session's keys while the canvas has the
//! keyboard.

#[cfg(feature = "screen-testing")]
mod stories;

use std::sync::Arc;

use craft_behavior::geometry::SnapTargets;
use craft_behavior::geometry::quad::{BoxPart, BoxRules};
use craft_behavior::kurbo::Vec2 as DocVec;
use craft_behavior::{CancelReason, EditEvent, Mixed};
use craft_keymap::Mods;
use craft_ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::contract::Bind;
use craft_ui::contract::Show as _;
use craft_ui::menu::Menu;
use craft_ui::primitives::Draw;
use craft_ui::runtime::env::{Env, Label};
use egui::{Rect, Ui};

use super::{TransformBox, TransformChoice, TransformEdit, TransformMenuWords, TransformPhase, TransformSession, TransformSettings, TransformWarp};

/// The distance an arrow key moves the whole box, in document units; Shift moves it by `NUDGE_FAR`.
const NUDGE: f64 = 1.0;
const NUDGE_FAR: f64 = 10.0;

/// What the overlay reported this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct TransformOverlayOutput<W> {
    /// The edits in the order they happened; the app applies each with [`TransformSession::apply`].
    pub edits: Vec<TransformEdit<W>>,
    /// Enter, a click outside or a double click inside: the app calls [`TransformSession::commit`] after the edits.
    pub apply: bool,
    /// Escape with no drag running: the app calls [`TransformSession::discard`] after the edits.
    pub discard: bool,
    /// The keyboard's picked part while the box has focus.
    pub picked: Option<BoxPart>,
}

/// The open [`TransformSession`]'s box on the canvas.
///
/// - Box phase: shows a [`TransformBox`] on the session's box with the session's
///   [rules](TransformSession::box_rules) (or the app's [`rules`](Self::rules)), its reference point hidden unless
///   the session [shows it](TransformSession::reference_point), and reports every box event as
///   [`TransformEdit::Box`]: a drag as Begin, Update and Commit, Escape during a drag as Cancel, an arrow key as
///   Set. The box's apply (Enter, a click outside, a double click inside) is
///   [`TransformOverlayOutput::apply`]; its discard (Escape with no drag) is
///   [`discard`](TransformOverlayOutput::discard). The app answers them with [`TransformSession::commit`] and
///   [`discard`](TransformSession::discard) after the edits.
/// - A secondary click on the canvas opens the session's mode menu; a pick reports as the edit it stands for.
/// - Warp phase: draws nothing, senses nothing and claims nothing; the app's warp widgets own the canvas.
/// - Keyboard, AccessKit and right-to-left are the box's. Undo and Redo are the app's Edit commands.
#[derive(craft_ui::contract::Pattern)]
#[pattern(probe(TransformOverlay<'static, ()>))]
pub struct TransformOverlay<'a, W> {
    label: Label,
    session: &'a TransformSession<W>,
    settings: &'a TransformSettings,
    rules: Option<BoxRules>,
    pixel_grid: bool,
    snap: Option<&'a SnapTargets>,
    mods: Option<Mods>,
}

impl<'a, W: TransformWarp> TransformOverlay<'a, W> {
    /// The overlay of `session` named `label`, with the box rules `settings` give.
    pub fn new(label: impl Into<Label>, session: &'a TransformSession<W>, settings: &'a TransformSettings) -> Self {
        Self { label: label.into(), session, settings, rules: None, pixel_grid: false, snap: None, mods: None }
    }

    /// The app's own box rules in place of [`TransformSession::box_rules`]: a 45° rotate step, a Move tool's rules.
    /// The session's mode and what it allows still win.
    #[must_use]
    pub fn rules(mut self, rules: BoxRules) -> Self {
        self.rules = Some(rules);
        self
    }

    /// Rounds dragged boxes to whole document units (Snap Transforms to Pixel Grid; the app's rule for shape layers
    /// and paths).
    #[must_use]
    pub fn pixel_grid(mut self, on: bool) -> Self {
        self.pixel_grid = on;
        self
    }

    /// The app's snapping of a dragged point.
    #[must_use]
    pub fn snap(mut self, snap: Option<&'a SnapTargets>) -> Self {
        self.snap = snap;
        self
    }

    /// The modifiers every test reads in place of the pointer's (a touch host's sticky modifiers).
    #[must_use]
    pub fn mods(mut self, mods: Option<Mods>) -> Self {
        self.mods = mods;
        self
    }

    /// Shows the box at `rect`, the canvas body's rect (Box phase); in the Warp phase returns an empty output.
    pub fn show_at(self, ui: &mut Ui, rect: Rect) -> TransformOverlayOutput<W> {
        let session = self.session;
        let mut out = TransformOverlayOutput { edits: Vec::new(), apply: false, discard: false, picked: None };
        if session.phase() == TransformPhase::Warp {
            return out;
        }
        let mut rules = self.rules.unwrap_or_else(|| session.box_rules(self.settings));
        // The session's mode and what it allows win over the app's rules.
        rules.mode = session.current_mode();
        rules.allow = session.allows();
        let mut tbox = TransformBox::new(self.label.clone(), Bind::given(*session.state()), session.rect(), session.skew_v())
            .rules(rules)
            .reference_point(session.reference_point())
            .pixel_grid(self.pixel_grid);
        if let Some(snap) = self.snap {
            tbox = tbox.snap(snap);
        }
        if let Some(mods) = self.mods {
            tbox = tbox.mods(mods);
        }
        let shown = tbox.show_at(ui, rect);
        out.edits.extend(shown.edit.events.iter().cloned().map(TransformEdit::Box));
        out.apply = shown.apply;
        out.discard = shown.discard;
        out.picked = shown.picked;
        let words = Env::of(ui).words::<TransformMenuWords>();
        let entries = session.menu(&words);
        if let Some(choice) = Menu::context(self.label, &shown.edit.response).pick(ui, &entries).picked {
            out.edits.push(match choice {
                TransformChoice::Mode(mode) => TransformEdit::Mode(mode),
                TransformChoice::Warp => TransformEdit::WarpRequested,
                TransformChoice::Preset(preset) => TransformEdit::Preset(preset),
            });
        }
        out
    }
}

/// The canvas tool while a session is open, so Free Transform's keys work while the canvas has the keyboard and the
/// box does not: the arrows move the whole box, Enter applies and Escape discards. Presses are the box's parts', so
/// the tool ignores them.
pub struct TransformTool<'a, W> {
    session: &'a TransformSession<W>,
    edits: Vec<TransformEdit<W>>,
    apply: bool,
    discard: bool,
}

impl<'a, W: TransformWarp> TransformTool<'a, W> {
    /// The tool of `session`.
    pub fn new(session: &'a TransformSession<W>) -> Self {
        Self { session, edits: Vec::new(), apply: false, discard: false }
    }

    /// What the keys asked for since the last call: the edits to apply in order, then whether to apply the session
    /// and whether to discard it.
    pub fn take_edits(&mut self) -> (Vec<TransformEdit<W>>, bool, bool) {
        (std::mem::take(&mut self.edits), std::mem::take(&mut self.apply), std::mem::take(&mut self.discard))
    }
}

impl<W: TransformWarp> CanvasTool for TransformTool<'_, W> {
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Ignored
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        match key {
            ToolKey::Nudge { direction, mods, .. } => {
                let Some(space) = cx.space().filter(|_| self.session.phase() == TransformPhase::Box) else { return false };
                let doc = space.view.unmap_vec(*direction);
                let length = doc.hypot();
                if length <= 0.0 {
                    return false;
                }
                let step = if mods.shift { NUDGE_FAR } else { NUDGE };
                let by: DocVec = doc * (step / length);
                let before = *self.session.state();
                let event = EditEvent::set(Mixed::Same(before), before.moved(by), Arc::from(""), None);
                self.edits.push(TransformEdit::Box(event));
                true
            }
            ToolKey::Submit => {
                self.apply = true;
                true
            }
            ToolKey::Cancel => {
                self.discard = true;
                true
            }
            _ => false,
        }
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Transform fields of the options bar")
    }
}
