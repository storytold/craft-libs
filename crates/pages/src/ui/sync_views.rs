//! `SyncedViews`: two page views side by side that follow each other's zoom, layout, rotation, current page and
//! scroll, as PdfCraft's Compare Files shows the old file left and the new one right. The half the user last pressed
//! leads (the new file at first). The pair draws the leading half first; the other takes the leader's zoom, layout
//! and scroll before it is drawn, in the same pass. The halves and the divider are craft-ui's [`SplitViews`] with
//! [`SplitLayout::TwoUpVertical`]: pane 0 is the left half, pane 1 the right half and main view.

use super::view::{PageViewOutput, PageViewState};
use craft_ui::canvas::{SplitAt, SplitLayout, SplitViews};
use craft_ui::contract::{Bind, EditResponse, Outcome, ShowContainer as _};
use craft_ui::runtime::env::Label;
use egui::{Rect, Response, Ui};

/// Which half leads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Lead {
    /// The left half.
    Left,
    /// The right half, the main view of the split.
    #[default]
    Right,
}

/// The pair's state, kept by the app.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct SyncedViewsState {
    /// The half that leads: the one the user last pressed.
    pub lead: Lead,
    /// Where the divider lies.
    pub at: SplitAt,
    /// The leader's offset when the follower last copied it.
    last_offset: Option<(f64, f64)>,
    /// The leader's current page when the follower last copied it.
    last_current: Option<usize>,
}

/// What the pair reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct SyncedViewsOutput {
    /// The divider's edits and the pair's response.
    pub edit: EditResponse<SplitAt>,
    /// The two halves' views, left then right; `None` for a half the split did not draw.
    pub views: [Option<PageViewOutput>; 2],
}

impl Outcome for SyncedViewsOutput {
    type Value = SplitAt;

    fn response(&self) -> &Response {
        &self.edit.response
    }
}

/// The pair, built per pass: the app draws each half with its own [`PageView`](super::view::PageView) inside the
/// closure, which gets the half's rect, its state and which half it is; the pair keeps the states together.
///
/// ```ignore
/// SyncedViews::new("Compare", &mut pair).halves(ui, (&mut old_state, &mut new_state), |ui, rect, state, half| {
///     let (label, pages): (&str, &dyn PageSource) = match half {
///         Lead::Left => ("Old", &old_pages),
///         Lead::Right => ("New", &new_pages),
///     };
///     // Each half runs the Hand tool in its own canvas body.
///     let mut reply = None;
///     let out = PageView::new(label, pages, &mut *state, paper).show_at(ui, rect, |ui, body| {
///         if let Some(pass) = body.pass {
///             reply = Some(ToolHost::new().run(ui, pass, &mut hand));
///         }
///     });
///     if let Some(view) = reply.and_then(|reply| reply.view) {
///         out.adopt(state, &view);
///     }
///     out
/// });
/// ```
pub struct SyncedViews<'a> {
    label: Label,
    state: &'a mut SyncedViewsState,
}

impl<'a> SyncedViews<'a> {
    /// A pair named `label`, with its state kept in `state`.
    pub fn new(label: impl Into<Label>, state: &'a mut SyncedViewsState) -> Self {
        SyncedViews { label: label.into(), state }
    }

    /// Splits the room in two halves (`SplitViews`, its divider and gap) and runs `half` for each, the leader first.
    // pdfcraft: crates/ui-egui/src/workflows/compare_ui.rs:344 `compare_area`.
    pub fn halves(
        self,
        ui: &mut Ui,
        states: (&mut PageViewState, &mut PageViewState),
        mut half: impl FnMut(&mut Ui, Rect, &mut PageViewState, Lead) -> PageViewOutput,
    ) -> SyncedViewsOutput {
        let SyncedViews { label, state } = self;
        let (left, right) = states;
        let lead = state.lead;
        let SyncedViewsState { at, last_offset, last_current, .. } = state;
        let (leader, follower) = match lead {
            Lead::Left => (left, right),
            Lead::Right => (right, left),
        };
        // `SplitLayout::TwoUpVertical` numbers its panes in reading order: pane 0 is the left half.
        let active = match lead {
            Lead::Left => 0,
            Lead::Right => 1,
        };
        let mut views: [Option<PageViewOutput>; 2] = [None, None];
        let divider = at.clone();
        let layout = SplitLayout::TwoUpVertical;
        let mut split = SplitViews::new(label, layout, Bind::bound(at)).active(active).show(ui, |ui, body| {
            // The follower is a copy per run, so the body writes nothing kept; `settle` stores the last copy.
            let mut followed = None;
            let rects = divider.placed(layout, 2, body.rect, body.env.direction().is_rtl());
            for pane in [active, 1 - active] {
                let Some(&rect) = rects.get(pane) else { continue };
                let this = if pane == 0 { Lead::Left } else { Lead::Right };
                views[pane] = Some(if this == lead {
                    half(ui, rect, leader, this)
                } else {
                    let mut copy = follower.clone();
                    let (mut offset, mut current) = (*last_offset, *last_current);
                    follow(leader, &mut copy, &mut offset, &mut current);
                    let output = half(ui, rect, &mut copy, this);
                    followed = Some((copy, offset, current));
                    output
                });
            }
            followed
        });
        // Once per pass, after the real run: the follower and what it followed from are stored.
        split.settle(|_, followed| {
            if let Some((copy, offset, current)) = followed.take() {
                *follower = copy;
                *last_offset = offset;
                *last_current = current;
            }
        });
        if let Some(pane) = split.output.activated {
            state.lead = if pane == 0 { Lead::Left } else { Lead::Right };
        }
        SyncedViewsOutput { edit: split.output.edit, views }
    }
}

/// The follower takes the leader's zoom, layout and rotation, its scroll offset once the leader has moved since the
/// last copy, and, when one page shows at a time, the leader's current page once it has changed (a continuous view's
/// current page follows from the offset). Returns whether anything changed.
// pdfcraft: crates/ui-egui/src/workflows/compare_ui.rs:362 `compare_area`.
fn follow(leader: &PageViewState, follower: &mut PageViewState, last_offset: &mut Option<(f64, f64)>, last_current: &mut Option<usize>) -> bool {
    let before = follower.clone();
    follower.zoom = leader.zoom;
    follower.fit = leader.fit;
    follower.display = leader.display;
    follower.continuous = leader.continuous;
    follower.cover = leader.cover;
    follower.page_gaps = leader.page_gaps;
    follower.hide_white_space = leader.hide_white_space;
    follower.rotation = leader.rotation;
    if *last_offset != Some(leader.offset) {
        follower.offset = leader.offset;
        *last_offset = Some(leader.offset);
    }
    if *last_current != Some(leader.current) {
        if !leader.continuous {
            follower.go_to(leader.current);
        }
        *last_current = Some(leader.current);
    }
    *follower != before
}
