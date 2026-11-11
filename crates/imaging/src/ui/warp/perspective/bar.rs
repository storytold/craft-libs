//! [`PerspectiveBar`]: Perspective Warp's options bar.

use craft_ui::bars::{Bar, BarKind};
use craft_ui::buttons::Button;
use craft_ui::choice::Segmented;
use craft_ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, clicked};
use craft_ui::display::{Divider, Text};
use craft_ui::runtime::env::Env;
use egui::Ui;

use super::{PerspectiveEdit, PerspectiveSession, PlaneMode, Straighten};

craft_ui::words! {
    /// The words of [`PerspectiveBar`] (PhotoCraft's Perspective Warp bar).
    pub struct PerspectiveBarWords {
        name: "perspectiveBar.name" = "Perspective Warp options";
        layout: "perspectiveBar.layout" = "Layout";
        warp: "perspectiveBar.warp" = "Warp";
        mode: "perspectiveBar.mode" = "Mode";
        planes: count "perspectiveBar.planes" = "{n} plane" | "{n} planes";
        remove_planes: "perspectiveBar.removePlanes" = "Remove Planes";
        straighten: "perspectiveBar.straighten" = "Straighten";
        straighten_tip: "perspectiveBar.straightenTip" = "Automatically straighten near-vertical line segments";
        level: "perspectiveBar.level" = "Level";
        level_tip: "perspectiveBar.levelTip" = "Automatically level near-horizontal line segments";
        both: "perspectiveBar.both" = "Both";
        both_tip: "perspectiveBar.bothTip" = "Automatically straighten and level";
        commit: "perspectiveBar.commit" = "Commit Perspective Warp";
        cancel: "perspectiveBar.cancel" = "Cancel Perspective Warp";
    }
}

/// Perspective Warp's options bar, in Tab order: the Layout and Warp tabs (Warp disabled with no plane); in Layout the
/// plane count ("2 planes") and Remove Planes; in Warp Straighten, Level and Both with their tooltips; then cancel and
/// commit at the reading end. The bar never changes the session: it reports edits for [`PerspectiveSession::apply`].
#[derive(craft_ui::contract::Pattern)]
pub struct PerspectiveBar<'a> {
    session: &'a PerspectiveSession,
}

/// What the bar reported this pass.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct PerspectiveBarOutput {
    /// The edits in order; the app applies each with [`PerspectiveSession::apply`].
    pub edits: Vec<PerspectiveEdit>,
    /// Commit pressed.
    pub apply: bool,
    /// Cancel pressed.
    pub discard: bool,
}

impl<'a> PerspectiveBar<'a> {
    /// The bar of `session`.
    pub fn new(session: &'a PerspectiveSession) -> Self {
        Self { session }
    }

    /// Shows the bar.
    pub fn show(self, ui: &mut Ui) -> PerspectiveBarOutput {
        // pc: crates/ui-egui/src/tools/perspective_ui.rs:310 (`options_bar`).
        let env = Env::of(ui);
        let words = env.words::<PerspectiveBarWords>();
        let mut edits = Vec::new();
        let shown = Bar::new(words.name.clone(), BarKind::Options)
            .commit_cancel(words.commit.clone(), words.cancel.clone())
            .show(ui, |ui, _| body(ui, self.session, &words, &env, &mut edits));
        PerspectiveBarOutput { edits, apply: shown.committed, discard: shown.cancelled }
    }
}

fn body(ui: &mut Ui, session: &PerspectiveSession, words: &PerspectiveBarWords, env: &Env, edits: &mut Vec<PerspectiveEdit>) {
    let planes = session.planes().len();
    let modes = [Entry::new(PlaneMode::Layout, words.layout.clone()), Entry::new(PlaneMode::Warp, words.warp.clone()).enabled(planes > 0)];
    let tabs = Segmented::new(words.mode.clone(), Bind::given(session.mode()), &modes).show(ui);
    if let Some(&mode) = tabs.value().filter(|&&mode| mode != session.mode()) {
        edits.push(PerspectiveEdit::Mode(mode));
    }
    Divider::vertical().show(ui);
    match session.mode() {
        PlaneMode::Warp => {
            for (label, tip, how) in [
                (&words.straighten, &words.straighten_tip, Straighten::Vertical),
                (&words.level, &words.level_tip, Straighten::Horizontal),
                (&words.both, &words.both_tip, Straighten::Both),
            ] {
                if clicked(&Button::new(label.clone()).tooltip(tip.clone()).show(ui)) {
                    edits.push(PerspectiveEdit::Straighten(how));
                }
            }
        }
        PlaneMode::Layout => {
            let count = words.planes.text(planes as u64, &env.format().integer(planes as u64));
            Text::new(count).show(ui);
            let remove = Button::new(words.remove_planes.clone()).enabled(planes > 0).show(ui);
            if clicked(&remove) {
                edits.push(PerspectiveEdit::replace(session.planes(), Vec::new(), words.remove_planes.as_str()));
            }
        }
    }
}
