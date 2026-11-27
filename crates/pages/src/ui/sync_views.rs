//! `SyncedViews`: two page views side by side that follow each other's zoom, layout, rotation, current page and
//! scroll, as PdfCraft's Compare Files shows the old file left and the new one right. The half the user last pressed
//! leads (the new file at first). The pair draws the leading half first; the other takes the leader's zoom, layout
//! and scroll before it is drawn, in the same pass. The halves and the divider are craft-ui's [`SplitViews`] with
//! [`SplitLayout::TwoUpVertical`]: pane 0 is the left half, pane 1 the right half and main view.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::view::{PageViewOutput, PageViewState};
use craft_behavior::Proposal;
use craft_ui::canvas::{SplitAt, SplitLayout, SplitViews};
use craft_ui::contract::{Bind, EditResponse, Outcome, Pad, Show as _, ShowContainer as _};
use craft_ui::display::{Elide, Shade, Text};
use craft_ui::layout_components::Stack;
use craft_ui::runtime::env::Label;
use craft_ui::style::{SurfaceKind, TypeRole};
use egui::{Rect, Response, Ui, pos2};

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
/// closure, which gets the half's rect, its state and which half it is, and returns the half's page view output
/// (`None` for a half with no pages to show: a closed or empty document); the pair keeps the states together.
///
/// ```ignore
/// SyncedViews::new("Compare", &mut pair).halves(ui, (&mut old_state, &mut new_state), |ui, rect, state, half| {
///     let (label, pages): (&str, &dyn PageSource) = match half {
///         Lead::Left => ("Old", &old_pages),
///         Lead::Right => ("New", &new_pages),
///     };
///     let out = PageView::new(label, pages, &mut *state, paper).show_at_with(ui, rect, &mut hand,
///         |ui, body, hand| body.pass.and_then(|pass| ToolHost::new().run(ui, pass, hand).view),
///         |_, _, _| ());
///     Some(out)
/// });
/// ```
#[derive(craft_ui::contract::Pattern)]
pub struct SyncedViews<'a> {
    label: Label,
    state: &'a mut SyncedViewsState,
    captions: Option<(Label, Label)>,
}

impl<'a> SyncedViews<'a> {
    /// A pair named `label`, with its state kept in `state`.
    pub fn new(label: impl Into<Label>, state: &'a mut SyncedViewsState) -> Self {
        SyncedViews { label: label.into(), state, captions: None }
    }

    /// A caption line over each half: `left` over the left half and `right` over the right.
    #[must_use]
    pub fn captions(mut self, left: impl Into<Label>, right: impl Into<Label>) -> Self {
        self.captions = Some((left.into(), right.into()));
        self
    }

    /// Splits the room in two halves (`SplitViews`, its divider on the boundary of the two panes) and runs `half` for each, the leader first.
    /// With [`captions`](Self::captions) each half shows its caption band at its top and `half` gets the rect under it.
    // pdfcraft: crates/ui-egui/src/workflows/compare_ui.rs:344 `compare_area`.
    pub fn halves(
        self,
        ui: &mut Ui,
        states: (&mut PageViewState, &mut PageViewState),
        mut half: impl FnMut(&mut Ui, Rect, &mut PageViewState, Lead) -> Option<PageViewOutput>,
    ) -> SyncedViewsOutput {
        let SyncedViews { label, state, captions } = self;
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
            // The band is the framework's `Stack` (panel surface, `Pad::Control`) around a caption `Text`, so its padding
            // and line height come from the components' own styles and the pattern reads no tokens.
            // Both captions share one type role, so one measurement gives both bands' height.
            let band = captions.as_ref().map(|(left, _)| caption_band().measure_in(ui, Proposal::Ideal, |ui, _| caption(left).show(ui)).y);
            for pane in [active, 1 - active] {
                let Some(&rect) = rects.get(pane) else { continue };
                let this = if pane == 0 { Lead::Left } else { Lead::Right };
                // The band is cut inside the pane, so the divider and the active edge keep the full height.
                let rect = match (&captions, band) {
                    (Some((left, right)), Some(height)) => {
                        let (band, view) = cut_band(rect, height);
                        let text = if this == Lead::Left { left } else { right };
                        caption_band().show_at(ui, band, |ui, _| caption(text).show(ui));
                        view
                    }
                    _ => rect,
                };
                views[pane] = if this == lead {
                    half(ui, rect, leader, this)
                } else {
                    let mut copy = follower.clone();
                    let (mut offset, mut current) = (*last_offset, *last_current);
                    follow(leader, &mut copy, &mut offset, &mut current);
                    let output = half(ui, rect, &mut copy, this);
                    followed = Some((copy, offset, current));
                    output
                };
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

/// The band a caption sits in: the panel surface with control padding, as wide as its half.
fn caption_band() -> Stack<'static> {
    Stack::new().surface(SurfaceKind::Panel).pad(Pad::Control).fill_width()
}

/// A half's caption: one muted caption line, cut at the end when the half is narrow.
fn caption(text: &Label) -> Text {
    Text::new(text.clone()).role(TypeRole::Caption).shade(Shade::Muted).elide(Elide::End)
}

/// Cuts `rect` into the band of `height` at its top and the rest under it; a pane shorter than the band is all band.
fn cut_band(rect: Rect, height: f32) -> (Rect, Rect) {
    let edge = (rect.min.y + height).min(rect.max.y);
    (Rect::from_min_max(rect.min, pos2(rect.max.x, edge)), Rect::from_min_max(pos2(rect.min.x, edge), rect.max))
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
