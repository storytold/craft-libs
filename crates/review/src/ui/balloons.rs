//! `CommentBalloons`: comment and revision cards in the margin of the page view, with leaders to
//! their anchors.

use craft_behavior::{Input, Metrics, Proposal};
use craft_pages::ui::PlacedPage;
use craft_ui::contract::{
    Common, Component, Cx, ItemOutput, Outcome, PaintUi, Pattern, Role, SetPosition, Show as _, clicked, measure_item, show_item,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::{ComponentStyle, StyleColor, StyleCx};
use egui::{Pos2, Rect, Response, Ui, vec2};
use kurbo::Point;

use super::comments::{CardActions, CardEvent, CommentEdit, CommentRow, CommentsPanelState, CommentsWords, apply};
use crate::comments::{CommentThread, ThreadId};
use crate::revisions::Revision;

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The most detail lines of a revision a card shows.
const DETAIL_LINES: usize = 8;

/// The most rows of a message a comment card shows, so a long body never pushes the next card out of view.
const BODY_ROWS: usize = 10;

craft_ui::words! {
    /// The words of the comment balloons.
    pub struct BalloonWords {
        add: "balloons.addComment" = "Add a comment";
    }
}

/// One card of the balloons: a comment thread or a revision, on a page at an anchor.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum BalloonItem<'a> {
    /// A comment thread.
    #[non_exhaustive]
    Thread {
        /// The thread.
        thread: &'a CommentThread,
        /// The page it is on.
        page: usize,
        /// The point it is anchored at, in the page's own points (the page's origin top left, y down).
        anchor: Pos2,
    },
    /// A tracked change.
    #[non_exhaustive]
    Revision {
        /// The change.
        revision: &'a Revision,
        /// The page it is on.
        page: usize,
        /// The point it is anchored at, in the page's own points (the page's origin top left, y down).
        anchor: Pos2,
    },
}

impl<'a> BalloonItem<'a> {
    /// A comment `thread` on `page`, anchored at `anchor`.
    pub const fn thread(thread: &'a CommentThread, page: usize, anchor: Pos2) -> Self {
        BalloonItem::Thread { thread, page, anchor }
    }

    /// A tracked change `revision` on `page`, anchored at `anchor`.
    pub const fn revision(revision: &'a Revision, page: usize, anchor: Pos2) -> Self {
        BalloonItem::Revision { revision, page, anchor }
    }
}

impl BalloonItem<'_> {
    fn page(&self) -> usize {
        match *self {
            BalloonItem::Thread { page, .. } | BalloonItem::Revision { page, .. } => page,
        }
    }

    fn anchor(&self) -> Pos2 {
        match *self {
            BalloonItem::Thread { anchor, .. } | BalloonItem::Revision { anchor, .. } => anchor,
        }
    }
}

/// Where the balloons sit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum BalloonPlace {
    /// In a column beside the pages.
    #[default]
    Column,
    /// Floating over the pages.
    Floating,
}

/// Layout and leaders of comment and revision cards. Each card is the panel's own comment row, fed
/// as the panel feeds it; the events the cards report go through the panel's `apply`, so the panel
/// and the balloons share one draft model and one event mapping.
///
/// - Cards are sorted by page, then by anchor height, then by anchor x, and stacked down each page: a
///   card's top is its anchor's, or the bottom of the card before it plus a gap when that is lower.
///   The stacking starts again at each page.
/// - A comment card's message is cut to ten rows, so a long body never pushes the next card out of view.
/// - The selected card shows Reply, Resolve or Reopen, and Delete; a click on a card selects it. A card selected
///   again keeps its own edit draft (one the app could not write back), and an edit takes the keyboard in the pass
///   it opens, so one click on a balloon whose edit the app opens is enough to type.
/// - A dashed leader runs on the page to the page's edge, a solid one on to the card.
#[must_use = "show the balloons with `show`"]
#[derive(Pattern)]
pub struct CommentBalloons<'a> {
    label: Label,
    pages: &'a [PlacedPage],
    items: &'a [BalloonItem<'a>],
    state: &'a mut CommentsPanelState,
    place: BalloonPlace,
    width: Option<f32>,
    now: Option<i64>,
    read_only: bool,
}

impl<'a> CommentBalloons<'a> {
    /// Balloons named `label` for `items` on `pages`, with the card state in `state`.
    pub fn new(label: impl Into<Label>, pages: &'a [PlacedPage], items: &'a [BalloonItem<'a>], state: &'a mut CommentsPanelState) -> Self {
        CommentBalloons { label: label.into(), pages, items, state, place: BalloonPlace::Column, width: None, now: None, read_only: false }
    }

    /// Whether the comments are read only (off by default): every card shows as a locked thread's does, its text
    /// taking no typing, as the comments panel's rows do while comments cannot change.
    pub fn read_only(mut self, on: bool) -> Self {
        self.read_only = on;
        self
    }

    /// Where the balloons sit.
    pub fn place(mut self, place: BalloonPlace) -> Self {
        self.place = place;
        self
    }

    /// The width of a card, in points; the dock column's width by default.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// The time the cards' ages count from.
    pub fn now(mut self, now: Option<i64>) -> Self {
        self.now = now;
        self
    }

    /// Shows the balloons.
    ///
    /// Every card is measured and placed first, so the leaders can be shown beneath the cards; the cards' events
    /// are applied once all are shown.
    pub fn show(self, ui: &mut Ui) -> CommentBalloonsOutput {
        let CommentBalloons { label, pages, items, state, place, width, now, read_only } = self;
        let room = ui.available_rect_before_wrap();
        let env = Env::of(ui);
        let words = env.words::<CommentsWords>();
        let rtl = env.direction.is_rtl();
        let gap = env.tokens.space.gap.row;
        let width = width.unwrap_or(env.tokens.size.dock.column);
        let now = now.unwrap_or(0);

        // The stacking is right only over items in this order, whatever order the caller listed them in.
        let mut order: Vec<(usize, &PlacedPage)> = items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| pages.iter().find(|page| page.page == item.page()).map(|page| (index, page)))
            .collect();
        order.sort_by(|&(a, _), &(b, _)| {
            let (a, b) = (items[a], items[b]);
            a.page().cmp(&b.page()).then(a.anchor().y.total_cmp(&b.anchor().y)).then(a.anchor().x.total_cmp(&b.anchor().x))
        });

        let column = if rtl {
            pages.iter().map(|page| page.rect.left()).fold(f32::INFINITY, f32::min) - gap - width
        } else {
            pages.iter().map(|page| page.rect.right()).fold(f32::NEG_INFINITY, f32::max) + gap
        };
        // A card always lies whole inside the room: without room beside the pages it floats over their edge, so
        // its text is never cut off by the room's clip.
        let x_of = |page: &PlacedPage| {
            let preferred = match place {
                BalloonPlace::Column => column,
                BalloonPlace::Floating if rtl => page.rect.left() - gap - width,
                BalloonPlace::Floating => page.rect.right() + gap,
            };
            preferred.min(room.right() - width).max(room.left())
        };

        let count = order.len();
        // design: allow[contract/memory](the balloons are a function, not a component: the label salts the ids of its cards so two balloon columns never meet, and no contract call scopes ids by hand)
        let shown = ui.push_id(label.as_str(), |ui| {
            let mut stacked: Option<(usize, f32)> = None;
            let mut rows = Vec::with_capacity(count);
            let mut leaders = Vec::with_capacity(count);
            for (at, &(index, page)) in order.iter().enumerate() {
                let item = items[index];
                let anchor = page.to_screen(Point::new(f64::from(item.anchor().x), f64::from(item.anchor().y)));
                let floor = match stacked {
                    Some((placed, bottom)) if placed == page.page => bottom + gap,
                    _ => f32::NEG_INFINITY,
                };
                let (mut row, thread) = card(item, &words, &*state, now, read_only);
                let height = measure_item(ui, &mut row, SetPosition::new(at).of(count), Proposal::Width(width)).y;
                let rect = Rect::from_min_size(Pos2::new(x_of(page), anchor.y.max(floor)), vec2(width, height));
                stacked = Some((page.page, rect.bottom()));
                // A card over the page's edge has no solid stretch: the dashed one runs to the card.
                let (edge, attach) =
                    if rtl { (page.rect.left().max(rect.right()), rect.right()) } else { (page.rect.right().min(rect.left()), rect.left()) };
                leaders.push(Leader { anchor, edge: Pos2::new(edge, anchor.y), card: Pos2::new(attach, anchor.y.clamp(rect.top(), rect.bottom())) });
                rows.push((row, thread, rect));
            }
            // The leaders go first so the cards, shown after, sit above them for hits.
            let response = Leaders::new(leaders).show_at(ui, room);
            let mut events = Vec::new();
            let mut pressed = Vec::new();
            let mut cards = Vec::new();
            for (at, (row, thread, rect)) in rows.into_iter().enumerate() {
                let out = show_item(ui, rect, row, SetPosition::new(at).of(count));
                let (placed, hit) = (Outcome::response(&out).rect, clicked(Outcome::response(&out)));
                events.extend(ItemOutput::<Vec<CardEvent>>::into_event(out).unwrap_or_default());
                if let Some(thread) = thread {
                    if hit {
                        pressed.push(thread);
                    }
                    cards.push((thread.id, placed));
                }
            }
            (response, events, pressed, cards)
        });
        let (response, card_events, pressed, cards) = shown.inner;
        let mut edits = Vec::new();
        for event in card_events {
            apply(event, state, &mut edits);
        }
        for thread in pressed {
            if state.selected.lead != Some(thread.id) {
                state.reveal(thread.id);
                edits.push(CommentEdit::Select(vec![thread.id]));
            }
        }
        CommentBalloonsOutput { response, edits, cards }
    }
}

/// The card of `item` and, for a comment, its thread. A comment's card is the panel's row with the balloon buttons,
/// editable unless the balloons are `read_only` or its thread is locked; a revision's is the row's note form.
///
/// The thread keeps its own lifetime, apart from the borrow of `state` the row holds, so the caller can change `state`
/// once the row is shown.
fn card<'r, 't: 'r>(
    item: BalloonItem<'t>,
    words: &CommentsWords,
    state: &'r CommentsPanelState,
    now: i64,
    read_only: bool,
) -> (CommentRow<'r>, Option<&'t CommentThread>) {
    match item {
        BalloonItem::Thread { thread, .. } => {
            let name =
                thread.messages.first().map_or_else(String::new, |first| words.card_name.fill(&[("author", &first.author), ("text", &first.text)]));
            let selected = state.selected.lead == Some(thread.id);
            let editing = state.editing.as_ref().filter(|(id, ..)| *id == thread.id).map(|(_, index, text)| (*index, text.as_str()));
            let row = CommentRow::card(thread, name)
                .actions(CardActions::Buttons)
                .body_rows(BODY_ROWS)
                .state(selected, !read_only && !thread.locked, now)
                .lead(selected)
                .drafts(editing, if selected { state.reply.as_str() } else { "" });
            (row, Some(thread))
        }
        BalloonItem::Revision { revision, .. } => {
            let details = revision.details.iter().take(DETAIL_LINES).cloned().collect();
            (CommentRow::revision(revision.heading.clone(), details).actions(CardActions::Buttons), None)
        }
    }
}

/// What the balloons report for a pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct CommentBalloonsOutput {
    /// The balloons' response.
    pub response: Response,
    /// What the user asked of the comments, as the panel reports it.
    pub edits: Vec<CommentEdit>,
    /// Each thread's card and the rect it took this pass.
    pub cards: Vec<(ThreadId, Rect)>,
}

impl Outcome for CommentBalloonsOutput {
    type Value = craft_ui::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// One leader: from the anchor on the page to the page's edge (dashed), then on to the card (solid).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Leader {
    anchor: Pos2,
    edge: Pos2,
    card: Pos2,
}

/// The style of the leaders: the divider's colour for the stretch on the page and the border's for the one to the card.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
struct LeadersStyle {
    #[style(metrics)]
    metrics: Metrics,
    on_page: StyleColor,
    to_card: StyleColor,
}

/// The leaders of the balloons, painted over the room the balloons fill.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(sample = Leaders::sample)]
struct Leaders {
    #[common]
    common: Common,
    lines: Vec<Leader>,
}

impl Leaders {
    fn new(lines: Vec<Leader>) -> Self {
        Leaders { common: Common::new(), lines }
    }

    /// Two leaders, for the suite.
    fn sample() -> Self {
        let line = |y: f32| Leader { anchor: Pos2::new(10.0, y), edge: Pos2::new(60.0, y), card: Pos2::new(80.0, y + 10.0) };
        Leaders::new(vec![line(20.0), line(60.0)])
    }
}

impl Component for Leaders {
    const ROLE: Role = Role::GROUP;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    const SHOWS_LABEL: bool = false;
    type Style = LeadersStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: LeadersStyleKey) -> LeadersStyle {
        let lines = &cx.tokens.color.line;
        LeadersStyle { metrics: Metrics::new(), on_page: cx.color(lines.divider), to_card: cx.color(lines.border) }
    }

    /// The leaders are paint only: the cards and the anchors take the pointer and the keys.
    fn sense(&self) -> craft_behavior::Sense {
        craft_behavior::Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let draw = ui.draw();
        for leader in &self.lines {
            draw.dashed(&[leader.anchor, leader.edge], draw.dashes(), StrokeWidth::Thin, cx.style.on_page);
            if leader.edge != leader.card {
                draw.stroke(&PaintShape::Path(vec![leader.edge, leader.card]), StrokeWidth::Thin, cx.style.to_card);
            }
        }
    }

    fn access(&self, access: &mut craft_ui::contract::Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
