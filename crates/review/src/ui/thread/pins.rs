//! Comment pins on a canvas: clustering, the pin bubble and [`CommentPins`].

use craft_behavior::{IconId, Metrics, PressPhase, Proposal, Sense};
use craft_ui::contract::{
    Common, Component, Cx, MeasureCx, NoValue, Outcome, PaintUi, Pattern, Pointer as _, Props as _, Role, Show as _, ShowContainer as _, Trigger,
};
use craft_ui::layout_components::Stack;
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::primitives::{HAlign, IconSpec, PaintShape, StrokeWidth, TextRun};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::{ComponentStyle, DataColor, StyleColor, StyleCx, TypeRole};
use egui::{Color32, CornerRadius, Pos2, Rect, Response, Ui, Vec2};

use crate::comments::ThreadId;
use crate::ui::avatar::{paint_avatar, paint_face};

use super::ThreadWords;

/// The words of the thread's text a pin's accessible name carries.
const PIN_NAME_WORDS: usize = 8;

/// A pin or a cluster of pins on screen.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PinCluster {
    /// Centre of the pin or cluster, in screen points.
    pub at: Pos2,
    /// The threads it stands for.
    pub threads: Vec<ThreadId>,
}

/// Groups of `points` (screen): in order, each point joins the first group whose centre (the mean
/// of its points so far) is closer than `radius`, else starts one. Indices per group.
pub fn cluster(points: &[Pos2], radius: f32) -> Vec<Vec<usize>> {
    let mut groups: Vec<(Vec<usize>, Vec2)> = Vec::new();
    for (index, point) in points.iter().enumerate() {
        let near = groups.iter_mut().find(|(members, sum)| (*sum / members.len() as f32).to_pos2().distance(*point) < radius);
        match near {
            Some((members, sum)) => {
                members.push(index);
                *sum += point.to_vec2();
            }
            None => groups.push((vec![index], point.to_vec2())),
        }
    }
    groups.into_iter().map(|(members, _)| members).collect()
}

/// One thread's pin: where it is (screen), who started it and how it looks.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CommentPin<'a> {
    /// The thread this pin belongs to.
    pub id: ThreadId,
    /// The pin's spot in screen coordinates.
    pub point: Pos2,
    /// The author of the thread's first message.
    pub author: &'a str,
    /// The thread's first message, for the pin's accessible name (its first words).
    pub text: &'a str,
    /// The author's colour (straight RGB), as the people bar shows them.
    pub color: [u8; 3],
    /// The viewer hasn't read the thread: a dot on the pin.
    pub unread: bool,
    /// The thread is resolved: a grey avatar and a check.
    pub resolved: bool,
}

impl<'a> CommentPin<'a> {
    /// A read, unresolved pin of the thread `id` at `point`, started by `author` in their colour `color`; `text` is
    /// the thread's first message.
    pub fn new(id: ThreadId, point: Pos2, author: &'a str, text: &'a str, color: [u8; 3]) -> Self {
        CommentPin { id, point, author, text, color, unread: false, resolved: false }
    }

    /// Whether the viewer hasn't read the thread.
    #[must_use]
    pub fn unread(mut self, unread: bool) -> Self {
        self.unread = unread;
        self
    }

    /// Whether the thread is resolved.
    #[must_use]
    pub fn resolved(mut self, resolved: bool) -> Self {
        self.resolved = resolved;
        self
    }
}

/// A pin dragged to a new screen point; the app maps it to its anchor.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PinMove {
    /// The thread that was dragged.
    pub thread: ThreadId,
    /// Where it was dropped, in screen points.
    pub to: Pos2,
}

/// What a pin bubble shows.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Face {
    Pin { author: String, color: [u8; 3], unread: bool, resolved: bool, open: bool, mine: bool },
    Cluster { count: usize },
}

/// What a pin bubble returns.
#[non_exhaustive]
pub(crate) struct PinOut {
    response: Response,
    activated: bool,
    dropped: Option<Pos2>,
}

impl Outcome for PinOut {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The style of a pin bubble.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(crate) struct PinStyle {
    #[style(metrics)]
    metrics: Metrics,
    fill: StyleColor,
    fill_hot: StyleColor,
    edge: StyleColor,
    dot: StyleColor,
    cluster: StyleColor,
    on_strong: StyleColor,
    resolved: StyleColor,
    resolved_ink: StyleColor,
    side: f32,
    inset: f32,
    mark_side: f32,
}

/// A speech-bubble pin or a cluster of them: round on three corners, square at the spot (bottom-left).
#[derive(craft_ui::contract::Component)]
pub(crate) struct PinBubble {
    #[common]
    common: Common,
    face: Face,
    line: f32,
    activated: bool,
    dropped: Option<Pos2>,
}

impl PinBubble {
    /// A bubble of `face`, named `name`.
    pub(super) fn new(face: Face, name: String) -> Self {
        PinBubble { common: Common::new(), face, line: 0.0, activated: false, dropped: None }.label(name)
    }

    fn mine(&self) -> bool {
        matches!(self.face, Face::Pin { mine: true, .. })
    }

    /// How far the pointer has dragged your own pin from where it was pressed.
    fn lift(&self, cx: &Cx<'_, Self>) -> Vec2 {
        match cx.press().phase {
            PressPhase::DragStart { start, pos, .. } | PressPhase::Drag { start, pos, .. } if self.mine() => pos - start.origin,
            _ => Vec2::ZERO,
        }
    }
}

impl Component for PinBubble {
    const ROLE: Role = Role::BUTTON;
    type Style = PinStyle;
    type Output = PinOut;

    fn style(cx: &StyleCx<'_>, _: PinBubbleStyleKey) -> PinStyle {
        let color = &cx.tokens.color;
        PinStyle {
            metrics: Metrics::new(),
            fill: cx.color(color.surface.raised),
            fill_hot: cx.color(color.accent.soft),
            edge: cx.color(color.line.border),
            dot: cx.color(color.accent.strong),
            cluster: cx.color(color.accent.strong),
            on_strong: cx.color(color.text.on_strong),
            resolved: cx.color(color.surface.sunken),
            resolved_ink: cx.color(color.text.muted),
            side: cx.control().height,
            inset: cx.tokens.space.gap.inline,
            mark_side: cx.tokens.size.icon.sm,
        }
    }

    fn sense(&self) -> Sense {
        if self.mine() { Sense::CLICK | Sense::DRAG | Sense::FOCUS } else { Sense::CLICK | Sense::FOCUS }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let line = cx.layout(&TextRun::new("0").role(TypeRole::Caption)).size().y;
        self.line = line;
        let width = match self.face {
            Face::Cluster { count } => {
                let digits = cx.layout(&TextRun::new(count.to_string()).role(TypeRole::Caption)).size().x;
                cx.style.side.max(digits + 2.0 * cx.style.inset)
            }
            _ => cx.style.side,
        };
        Vec2::new(width, cx.style.side)
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.activated = cx.activation().is_some();
        if let PressPhase::Release { start, pos, click: false, .. } = cx.press().phase
            && self.mine()
        {
            self.dropped = Some(cx.rect.left_bottom() + (pos - start.origin));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let st = cx.style;
        let rect = cx.rect.translate(self.lift(cx));
        let radius = CornerRadius { sw: 0, ..CornerRadius::same((rect.height() / 2.0) as u8) };
        let hot = cx.response.hovered() || cx.response.has_focus() || matches!(self.face, Face::Pin { open: true, .. });
        let shape = PaintShape::Rect(rect, radius);
        let draw = ui.draw();
        draw.fill(&shape, if hot { st.fill_hot } else { st.fill });
        draw.stroke(&shape, StrokeWidth::Thin, st.edge);
        let inner = rect.shrink(st.inset);
        match &self.face {
            Face::Pin { author, color, unread, resolved, .. } => {
                if *resolved {
                    paint_face(&draw, inner, author, st.resolved, st.resolved_ink);
                } else {
                    let [r, g, b] = *color;
                    paint_avatar(ui, inner, author, DataColor::new(Color32::from_rgb(r, g, b)));
                }
                if *unread {
                    let centre = Pos2::new(rect.right() - st.inset, rect.top() + st.inset);
                    draw.icon(Rect::from_center_size(centre, Vec2::splat(st.mark_side)), &IconSpec::disc().color(st.dot));
                }
                if *resolved {
                    let check = Rect::from_center_size(Pos2::new(rect.right() - st.inset, rect.bottom() - st.inset), Vec2::splat(st.mark_side));
                    draw.icon(check, &IconSpec::svg(IconId::new("check")).color(st.dot));
                }
            }
            Face::Cluster { count } => {
                draw.fill(&PaintShape::Rect(inner, CornerRadius::same((inner.height() / 2.0) as u8)), st.cluster);
                let run = TextRun::new(count.to_string()).role(TypeRole::Caption).color(st.on_strong).align(HAlign::Center);
                let at = Rect::from_min_size(Pos2::new(inner.left(), inner.center().y - self.line / 2.0), Vec2::new(inner.width(), self.line));
                draw.text(at, &run);
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> PinOut {
        PinOut { response: cx.response.clone(), activated: self.activated, dropped: self.dropped }
    }
}

/// Comment pins over a canvas: a pin per thread (the author's avatar in the author's colour in a speech
/// bubble with a square bottom-left corner, grey with a check once resolved, a dot while unread), pins
/// within one pin's width of each other gathered into a cluster showing the count. A click or Enter on a
/// pin opens its thread, on the open pin closes it, on a cluster lists its threads. Your own pin can be
/// dragged to another spot, reported as a [`PinMove`] on release. Each pin is a focus stop in reading order.
#[must_use = "show the pins with `show`"]
#[derive(Pattern)]
pub struct CommentPins<'a> {
    label: Label,
    pins: &'a [CommentPin<'a>],
    me: &'a str,
    open: Option<ThreadId>,
}

impl<'a> CommentPins<'a> {
    /// The pins named `label`, one per thread of `pins`; `me` is the viewer's name.
    pub fn new(label: impl Into<Label>, pins: &'a [CommentPin<'a>], me: &'a str) -> Self {
        CommentPins { label: label.into(), pins, me, open: None }
    }

    /// The thread whose popover is open (its pin is drawn raised).
    pub fn open(mut self, thread: Option<ThreadId>) -> Self {
        self.open = thread;
        self
    }
}

/// What the pins did this pass.
#[must_use]
#[non_exhaustive]
pub struct CommentPinsOutput {
    /// The pins' response.
    pub response: Response,
    /// The thread whose pin or cluster entry was clicked while its popover was closed.
    pub opened: Option<ThreadId>,
    /// The thread whose open pin was clicked: its popover closes.
    pub closed: Option<ThreadId>,
    /// A drag of your own pin, reported when it is dropped.
    pub moved: Option<PinMove>,
}

impl Outcome for CommentPinsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl CommentPins<'_> {
    /// Shows the pins and reports clicks and drags.
    pub fn show(self, ui: &mut Ui) -> CommentPinsOutput {
        let CommentPins { label, pins, me, open } = self;
        let words = Env::of(ui).words::<ThreadWords>();
        let side = PinBubble::new(Face::Cluster { count: 1 }, String::new()).measure_in(ui, Proposal::Ideal).y;
        let points: Vec<Pos2> = pins.iter().map(|pin| pin.point).collect();
        let mut spots: Vec<PinCluster> = cluster(&points, side)
            .into_iter()
            .map(|members| {
                let sum = members.iter().fold(Vec2::ZERO, |sum, &i| sum + points[i].to_vec2());
                PinCluster { at: (sum / members.len() as f32).to_pos2(), threads: members.iter().map(|&i| pins[i].id).collect() }
            })
            .collect();
        spots.sort_by(|a, b| a.at.y.total_cmp(&b.at.y).then(a.at.x.total_cmp(&b.at.x)));
        let (mut opened, mut closed, mut moved) = (None, None, None);
        let shown = Stack::new().label(label).show(ui, |ui, _| {
            for spot in &spots {
                let one = match spot.threads.as_slice() {
                    [thread] => pins.iter().find(|pin| pin.id == *thread),
                    _ => None,
                };
                if let Some(pin) = one {
                    let words_of = pin.text.split_whitespace().take(PIN_NAME_WORDS).collect::<Vec<_>>().join(" ");
                    let name = if words_of.is_empty() {
                        words.pin.fill(&[("author", pin.author)])
                    } else {
                        words.pin_text.fill(&[("author", pin.author), ("words", &words_of)])
                    };
                    let face = Face::Pin {
                        author: pin.author.to_owned(),
                        color: pin.color,
                        unread: pin.unread,
                        resolved: pin.resolved,
                        open: open == Some(pin.id),
                        mine: pin.author == me,
                    };
                    let rect = Rect::from_min_max(Pos2::new(spot.at.x, spot.at.y - side), Pos2::new(spot.at.x + side, spot.at.y));
                    let bubble = PinBubble::new(face, name).id_salt(("pin", pin.id.0)).show_at(ui, rect);
                    if bubble.activated {
                        if open == Some(pin.id) {
                            closed = Some(pin.id);
                        } else {
                            opened = Some(pin.id);
                        }
                    }
                    if let Some(to) = bubble.dropped {
                        moved = Some(PinMove { thread: pin.id, to });
                    }
                } else {
                    let name = words.cluster.fill(&[("n", &spot.threads.len().to_string())]);
                    let face = Face::Cluster { count: spot.threads.len() };
                    let width = PinBubble::new(face.clone(), String::new()).measure_in(ui, Proposal::Ideal).x;
                    let rect = Rect::from_min_max(Pos2::new(spot.at.x, spot.at.y - side), Pos2::new(spot.at.x + width, spot.at.y));
                    let first = spot.threads.first().map_or(0, |thread| thread.0);
                    let bubble = PinBubble::new(face, name.clone()).id_salt(("cluster", first)).show_at(ui, rect);
                    let mut chosen = None;
                    Menu::new(name).opened_by(&bubble.response, Trigger::Click).show(ui, |ui, _| {
                        for thread in spot.threads.iter().filter_map(|thread| pins.iter().find(|pin| pin.id == *thread)) {
                            if MenuItem::new(words.pin.fill(&[("author", thread.author)])).show(ui).ran {
                                chosen = Some(thread.id);
                            }
                        }
                    });
                    if chosen.is_some() {
                        opened = chosen;
                    }
                }
            }
        });
        CommentPinsOutput { response: shown.output, opened, closed, moved }
    }
}
