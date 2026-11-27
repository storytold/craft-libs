//! `TransformOverlay`: the open session's box on the canvas, which shows a [`TransformBox`](super::TransformBox)
//! and turns its reports into [`TransformEdit`]s, and `TransformTool`: the canvas tool of the open session.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use craft_behavior::CancelReason;
use craft_behavior::geometry::SnapLine;
use craft_behavior::geometry::quad::{BoxPart, BoxRules};
use craft_keymap::Mods;
use craft_ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::contract::Show as _;
use craft_ui::contract::{Bind, Entry, IdSource, Props as _};
use craft_ui::menu::Menu;
use craft_ui::primitives::Draw;
use craft_ui::runtime::env::{Env, Label};
use egui::{AsIdSalt, Id, IdSalt, Rect, Ui};

use super::{
    ApplyBy, RotateZone, TransformBox, TransformChoice, TransformEdit, TransformMenuWords, TransformPhase, TransformReach, TransformSession,
    TransformSettings, TransformWarp,
};

/// What the overlay reported this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct TransformOverlayOutput<W> {
    /// The edits in the order they happened; the app applies each with [`TransformSession::apply`].
    pub edits: Vec<TransformEdit<W>>,
    /// Enter, a click outside or a double click inside: the app calls [`TransformSession::commit`] after the edits.
    pub apply: Option<ApplyBy>,
    /// Escape with no drag running: the app calls [`TransformSession::discard`] after the edits.
    pub discard: bool,
    /// The keyboard's picked part while the box has focus.
    pub picked: Option<BoxPart>,
    /// The part the box's open press holds, from the press to its release or cancel: a host that keeps state for a
    /// press (a capture) starts it at the press, before the drag begins.
    pub held: Option<BoxPart>,
    /// The snap lines of the point being dragged, from the box.
    pub snapped: Vec<SnapLine>,
    /// The value of the app's menu row that was picked.
    pub menu_row: Option<String>,
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
/// - [`TransformReach::MoveOnly`] offers only the move reach outside the frame: no handle, side, inside, outside, key
///   or menu, and the box is drawn only while its drag runs. [`TransformReach::Shown`] draws the box and takes
///   nothing: every press and key is the host's.
/// - Warp phase: draws nothing, senses nothing and claims nothing; the app's warp widgets own the canvas.
/// - Keyboard, AccessKit and right-to-left are the box's. Undo and Redo are the app's Edit commands.
#[derive(craft_ui::contract::Pattern)]
#[pattern(probe(TransformOverlay<'static, ()>))]
pub struct TransformOverlay<'a, W> {
    label: Label,
    session: &'a TransformSession<W>,
    settings: &'a TransformSettings,
    rules: Option<BoxRules>,
    mods: Option<Mods>,
    menu_rows: &'a [Entry<String>],
    reach: TransformReach,
    rotate_zone: RotateZone,
    id: IdSource,
}

impl<'a, W: TransformWarp> TransformOverlay<'a, W> {
    /// The overlay of `session` named `label`, with the box rules `settings` give.
    pub fn new(label: impl Into<Label>, session: &'a TransformSession<W>, settings: &'a TransformSettings) -> Self {
        Self {
            label: label.into(),
            session,
            settings,
            rules: None,
            mods: None,
            menu_rows: &[],
            reach: TransformReach::Box,
            rotate_zone: RotateZone::Outside,
            id: IdSource::Auto,
        }
    }

    /// Salts the box's id with `salt`, as [`Props::id_salt`](craft_ui::contract::Props::id_salt) does: a host that
    /// shows a [`TransformBox`] before the session opens gives both the same id, so a drag carries over.
    #[must_use]
    pub fn id_salt(mut self, salt: impl AsIdSalt) -> Self {
        self.id = IdSource::Salt(IdSalt::new(salt));
        self
    }

    /// Uses `id` as the box's id, as [`Props::id`](craft_ui::contract::Props::id) does.
    #[must_use]
    pub fn id(mut self, id: Id) -> Self {
        self.id = IdSource::Id(id);
        self
    }

    /// The app's rows after Warp in the box menu; a pick is reported as [`TransformOverlayOutput::menu_row`].
    #[must_use]
    pub fn menu_rows(mut self, rows: &'a [Entry<String>]) -> Self {
        self.menu_rows = rows;
        self
    }

    /// What of the box takes the pointer and the keys: the whole box (the default), or only the move reach outside
    /// the frame ([`TransformReach::MoveOnly`]), where a plain press moves the box and reaches
    /// [`TransformOverlayOutput::edits`] as a [`TransformEdit::Box`]; every other press is the host's, and the box has
    /// no menu. [`TransformReach::Shown`] only draws the box: every press and key is the host's.
    #[must_use]
    pub fn reach(mut self, reach: TransformReach) -> Self {
        self.reach = reach;
        self
    }

    /// Where a press outside the box rotates it: anywhere outside (the default, Free Transform), or only near its
    /// sides ([`RotateZone::Border`]).
    #[must_use]
    pub fn rotate_zone(mut self, zone: RotateZone) -> Self {
        self.rotate_zone = zone;
        self
    }

    /// The app's own box rules in place of [`TransformSession::box_rules`]: a 45° rotate step, a Move tool's rules.
    /// The session's mode and what it allows still win.
    #[must_use]
    pub fn rules(mut self, rules: BoxRules) -> Self {
        self.rules = Some(rules);
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
        let mut out =
            TransformOverlayOutput { edits: Vec::new(), apply: None, discard: false, picked: None, held: None, snapped: Vec::new(), menu_row: None };
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
            .reach(self.reach)
            .rotate_zone(self.rotate_zone)
            .own_context_menu(self.reach == TransformReach::Box)
            .id_source(self.id);
        if let Some(mods) = self.mods {
            tbox = tbox.mods(mods);
        }
        let shown = tbox.show_at(ui, rect);
        out.edits.extend(shown.edit.events.iter().cloned().map(TransformEdit::Box));
        out.apply = shown.apply;
        out.discard = shown.discard;
        out.picked = shown.picked;
        out.held = shown.held;
        out.snapped = shown.snapped;
        if self.reach != TransformReach::Box {
            return out;
        }
        let words = Env::of(ui).words::<TransformMenuWords>();
        let entries = splice_rows(session.menu(&words), self.menu_rows);
        if let Some(choice) = Menu::context(self.label, &shown.edit.response).pick(ui, &entries).picked {
            match choice {
                TransformChoice::Mode(mode) => out.edits.push(TransformEdit::Mode(mode)),
                TransformChoice::Warp => out.edits.push(TransformEdit::WarpRequested),
                TransformChoice::Preset(preset) => out.edits.push(TransformEdit::Preset(preset)),
                TransformChoice::Row(i) => out.menu_row = self.menu_rows.get(i).and_then(|row| row.value().cloned()),
            }
        }
        out
    }
}

/// `entries` (the session's menu) with the app's `rows` after the Warp entry, set off by a separator; the presets
/// group that follows starts with its own separator. Row `i` picks as [`TransformChoice::Row`]`(i)`.
fn splice_rows(mut entries: Vec<Entry<TransformChoice>>, rows: &[Entry<String>]) -> Vec<Entry<TransformChoice>> {
    if rows.is_empty() {
        return entries;
    }
    let after_warp = entries.iter().position(|entry| entry.value() == Some(&TransformChoice::Warp)).map_or(entries.len(), |at| at + 1);
    let app_rows = std::iter::once(Entry::separator()).chain(rows.iter().enumerate().map(|(i, row)| {
        if row.is_separator() {
            return Entry::separator();
        }
        if row.is_heading() {
            return Entry::heading(row.label().clone());
        }
        let entry = Entry::new(TransformChoice::Row(i), row.label().clone()).checked(row.is_checked()).enabled(row.is_enabled());
        match row.why_disabled() {
            Some(reason) => entry.disabled_reason(reason.clone()),
            None => entry,
        }
    }));
    entries.splice(after_warp..after_warp, app_rows);
    entries
}

/// The canvas tool while a session is open. Presses are the box's parts', so the tool ignores them. In the Box phase
/// the box holds the canvas's keys (Enter, Escape and the arrows reach it first, `KeyUse::session`); in the Warp
/// phase, where no box is shown, Enter applies and Escape discards through the tool. The snap lines the overlay
/// reported in this pass's canvas body reach the host through the tool, which draws and reports them.
pub struct TransformTool<'a, W> {
    session: &'a TransformSession<W>,
    snapped: Vec<SnapLine>,
    apply: bool,
    discard: bool,
}

impl<'a, W: TransformWarp> TransformTool<'a, W> {
    /// The tool of `session` for this pass. `snapped` is what [`TransformOverlay::show_at`] reported in this pass's
    /// canvas body ([`TransformOverlayOutput::snapped`]); empty when the body showed no box (the Warp phase).
    pub fn new(session: &'a TransformSession<W>, snapped: Vec<SnapLine>) -> Self {
        Self { session, snapped, apply: false, discard: false }
    }

    /// What the keys asked for since the last call: whether to apply the session and whether to discard it.
    pub fn take_keys(&mut self) -> (bool, bool) {
        (std::mem::take(&mut self.apply), std::mem::take(&mut self.discard))
    }
}

impl<W: TransformWarp> CanvasTool for TransformTool<'_, W> {
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        cx.add_snapped(self.snapped.iter().cloned());
    }

    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Ignored
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn key(&mut self, key: &ToolKey, _: &mut ToolContext<'_>) -> bool {
        if self.session.phase() != TransformPhase::Warp {
            return false;
        }
        match key {
            ToolKey::Submit => self.apply = true,
            ToolKey::Cancel => self.discard = true,
            _ => return false,
        }
        true
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Transform fields of the options bar")
    }
}
