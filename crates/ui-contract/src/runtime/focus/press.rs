//! Focus on press (spec 8.3): the press is classified at the begin hook from last pass's shapes, and the
//! classified focus move applies at the end hook.

use super::nav::declared_canvas;
use super::{Act, State, StopKey};
use crate::runtime::egui::focus as egui_focus;
use crate::runtime::{pointer, sampler};
use craft_behavior::PartRef;
use craft_behavior::Winner;
use craft_behavior::focus::{PressFocus, classify_press};
use egui::{Context, Id};

/// What won a primary press at the begin hook.
pub(crate) struct PressAt {
    winner: Option<Winner<Id>>,
    /// The arena root the winner belongs to.
    root: Option<Id>,
    /// The egui widget that takes the press.
    target: Option<Id>,
    /// Every part under the press point on its layer, sensing or not, from last pass's shapes: `(owner, part)`.
    under: Vec<(Id, Option<PartRef>)>,
    /// The press landed in a popup layer opened above the layer of the widget that holds egui focus.
    over_focus: bool,
}

/// Records that the part `part` of `owner` is declared `holds_drafts`, for the press classification of the next pass.
pub(crate) fn note_holds_drafts(ctx: &Context, owner: Id, part: PartRef) {
    super::with(ctx, |state| state.press.holding.push((owner, part)));
}

/// The text stop whose draft the press of this pass left unapplied, when one is.
pub(crate) fn held_draft(ctx: &Context) -> Option<Id> {
    super::with(ctx, |state| state.press.held_draft)
}

pub(crate) fn press_at(ctx: &Context) -> Option<PressAt> {
    // A press the modal's canvas took moves no key focus: the dialog keeps it.
    egui_focus::primary_press_origin(ctx).filter(|pos| !sampler::admitted_press(ctx, *pos)).map(|pos| {
        let targets = pointer::pass_pointer(ctx).press_targets;
        let (winner, root) = pointer::press_winner(ctx, pos, targets).unzip();
        let under = pointer::shapes_at(ctx, pos);
        let over_focus = super::popup_above_focus(ctx, pos);
        PressAt { winner, root, target: targets.click, under, over_focus }
    })
}

#[derive(Clone, Debug)]
struct Plan {
    kind: PressFocus,
    owner: Option<Id>,
    /// The arena root of the owner: the widget egui focus falls back to when the owner has no stop.
    root: Option<Id>,
    part: Option<PartRef>,
    pane: Option<Id>,
    was_text: bool,
}

#[derive(Default)]
pub(crate) struct PressState {
    plan: Option<Plan>,
    pub(super) commit_now: Option<Id>,
    /// The widget a Tab or Shift+Tab moved focus off this pass, and whether the move was forward.
    pub(super) tabbed_from: Option<(Id, bool)>,
    /// Tab, Shift+Tab or F6 moved focus this pass.
    pub(super) by_keys: bool,
    focus_at_begin: Option<Id>,
    /// The parts declared `holds_drafts` in the running pass, `(owner, part)`.
    holding: Vec<(Id, PartRef)>,
    /// The parts declared `holds_drafts` in the last pass, whose shapes the press is classified on.
    last_holding: Vec<(Id, PartRef)>,
    /// The text stop whose draft this pass's press left unapplied.
    held_draft: Option<Id>,
}

impl PressState {
    pub(super) fn rotate(&mut self, focused: Option<Id>) {
        self.plan = None;
        self.commit_now = None;
        self.tabbed_from = None;
        self.by_keys = false;
        self.focus_at_begin = focused;
        self.last_holding = std::mem::take(&mut self.holding);
        self.held_draft = None;
    }

    /// Whether a part under the press is one last pass declared `holds_drafts`.
    fn holds(&self, under: &[(Id, Option<PartRef>)]) -> bool {
        under.iter().any(|(owner, part)| part.is_some_and(|part| self.last_holding.contains(&(*owner, part))))
    }
}

impl State {
    /// Classifies this pass's primary press by the shape that wins it on the top layer, and marks a focused
    /// text part to commit its draft when the press leaves it. A focused text stop whose keys hold drafts keeps focus
    /// and its draft when any part under the press declared `holds_drafts`, whatever shape wins, or when the press
    /// lands in a popup opened above it; the stop is then the held draft.
    pub(super) fn classify_press(&mut self, at: &PressAt) {
        let winner = at.winner.as_ref();
        // A press on the focused stop is its own: it keeps the focus and the draft. The stop a press is on is the one it
        // would take focus for, read against the framework's record: egui focus sits on an arena's root while a member
        // of its body holds the stop, so a press on the root is not on the member. A press no stop's shape wins is on
        // the focused stop when egui gives it to the widget that holds egui focus (an app's own widget).
        let on_focused = match winner.and_then(|winner| self.press_stop(winner.owner, winner.part)) {
            Some(pressed) => self.stop_of_focus().is_some_and(|focused| focused.stop.key() == pressed.stop.key()),
            None => at.target.is_some() && at.target == self.focus_now,
        };
        let text_rec = self.stop_of_focus().filter(|stop| stop.stop.text);
        let text_stop = text_rec.as_ref().map(|stop| stop.stop.id);
        let held = !on_focused && text_rec.as_ref().is_some_and(|stop| stop.stop.keys.holds_drafts) && (at.over_focus || self.press.holds(&at.under));
        self.press.held_draft = text_stop.filter(|_| held);
        // A press no shape wins that egui gives to the widget of a press-only stop (a selection part's text, which egui
        // selects in) takes focus for that stop, as a press on any stop's shape does.
        let widget_stop = at.target.filter(|_| winner.is_none()).and_then(|target| self.press_only_stop(target));
        let kind = if on_focused || held {
            PressFocus::Keep
        } else if widget_stop.is_some() {
            PressFocus::Take
        } else {
            classify_press(winner.map(|winner| winner.sense))
        };
        let (owner, part) = match (winner, widget_stop) {
            (Some(winner), _) => (Some(winner.owner), winner.part),
            (None, Some(rec)) => (Some(rec.stop.id), rec.stop.part),
            (None, None) => (None, None),
        };
        let was_text = text_stop.is_some() && kind.commits_draft();
        // A press that takes focus for another component commits too: the page that component switches may remove the
        // field in the same pass, so a later focus loss would never reach it.
        let leaves = was_text || (kind == PressFocus::Take && owner != text_stop);
        self.press.commit_now = text_stop.filter(|_| leaves);
        let pane = self.stop_of_focus().filter(|_| was_text).and_then(|rec| self.pane_of(&rec));
        self.press.plan = Some(Plan { kind, owner, root: at.root, part, pane, was_text });
    }

    /// The stop this pass's press takes focus for, which the end hook moves focus to.
    pub(super) fn pressed_stop(&self) -> Option<StopKey> {
        let plan = self.press.plan.as_ref().filter(|plan| plan.kind == PressFocus::Take)?;
        let owner = plan.owner?;
        self.press_stop(owner, plan.part).map(|rec| rec.stop.key())
    }

    /// The stop a press on `part` of `owner` takes focus for: the part's own stop, else the owner's.
    fn press_stop(&self, owner: Id, part: Option<PartRef>) -> Option<super::StopRec> {
        self.find_stop((owner, part)).or_else(|| self.find_stop((owner, None)))
    }

    /// The press-only stop whose egui widget is `widget`.
    fn press_only_stop(&self, widget: Id) -> Option<super::StopRec> {
        self.stops.iter().chain(&self.last_stops).find(|rec| rec.stop.press_only && rec.stop.root == widget).cloned()
    }

    /// The press's focus move at the end hook; `focused` is egui's focus now. A press a component takes moves focus to
    /// the component, whatever has it. A press nothing takes ends the focus of a component only: it leaves the app's
    /// document canvas focused (`declared_canvas`), and when `focused` is an app's own widget (`on_app_widget`),
    /// egui's own surrender rule, by the app's `surrender_focus_on`, decides whether a press elsewhere takes it.
    pub(super) fn apply_press(&mut self, focused: Option<Id>, on_app_widget: bool) -> Vec<Act> {
        let Some(plan) = self.press.plan.take() else { return Vec::new() };
        match plan.kind {
            PressFocus::Take => self.take_focus(&plan),
            PressFocus::Release if plan.was_text => match plan.pane {
                Some(pane) => {
                    self.focused.remove(&focused.unwrap_or(pane));
                    vec![Act::Focus(pane, Default::default())]
                }
                // A dialog is its own pane: focus stays inside it, so Enter after a click on one of its buttons still
                // reaches the default button.
                None if self.in_trapping_scope() => Vec::new(),
                None => focused.map(Act::Surrender).into_iter().collect(),
            },
            PressFocus::Unclaimed
                if focused == self.press.focus_at_begin
                    && !on_app_widget
                    && !focused.is_some_and(|id| declared_canvas(&self.stops, id))
                    && !self.in_trapping_scope() =>
            {
                focused.map(Act::Surrender).into_iter().collect()
            }
            _ => Vec::new(),
        }
    }

    /// Whether the focused stop sits in a scope that traps focus (a dialog or modal): an unclaimed press, such as one on
    /// its scrim, leaves focus inside it.
    fn in_trapping_scope(&self) -> bool {
        let mut at = self.stop_of_focus().and_then(|rec| rec.scope);
        while let Some(id) = at {
            let Some(scope) = self.scopes.get(&id).or_else(|| self.last_scopes.get(&id)) else { return false };
            if scope.policy.trap {
                return true;
            }
            at = scope.parent;
        }
        false
    }

    fn take_focus(&mut self, plan: &Plan) -> Vec<Act> {
        let Some(owner) = plan.owner else { return Vec::new() };
        match self.press_stop(owner, plan.part) {
            Some(stop) => self.focus_stop_in_place(stop),
            None => vec![Act::Focus(plan.root.unwrap_or(owner), Default::default())],
        }
    }
}
