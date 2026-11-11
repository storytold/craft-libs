//! `BreadcrumbBar`: a path as clickable segments, the current one last. A leaf whose crumbs are parts with plain-text
//! faces in one part roving group. The leading crumbs that do not fit stand behind an ellipsis part that opens a menu
//! of them.

use super::words::BarWords;
use crate::contract::{
    Access, Common, Component, Cx, Entry, MeasureCx, Name, Outcome, OverlayUi, Overlays as _, PaintUi, PartsCx, Props as _, Role, Styled as _,
    WordSetCx as _,
};
use crate::display::{Elide, Shade, Text};
use crate::menu::Menu;
use crate::primitives::IconSpec;
use crate::runtime::env::Label;
use crate::runtime::focus::{ActivateKeys, KeyUse};
use crate::style::{ComponentStyle, FocusLook, IconLook, Stateful, StyleCx, TypeRole};
use craft_behavior::fit::{FitItem, FitKeep, fit_items};
use craft_behavior::{Axis, Flow, IconId, Metrics, NavShape, PartKey, PartRef, Roving, Sense, Value};
use egui::{Rect, Response, Vec2};

#[cfg(feature = "testing")]
mod stories;

const HIDDEN: PartKey = PartKey::new("hidden");
const CRUMB: PartKey = PartKey::new("crumb");

/// What separates crumbs in the whole path a tooltip shows.
const PATH_JOINT: &str = " \u{203a} ";

/// The style of a breadcrumb bar.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct BreadcrumbBarStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The chevron between crumbs.
    #[style(icon)]
    separator: IconLook,
    /// On each side of a separator and of the ellipsis.
    gap: f32,
    /// A separator with its gaps on both sides: the room between neighbouring crumbs.
    joint: f32,
    /// The width of the ellipsis part beyond its glyph.
    ellipsis_pad: f32,
    /// The most room an ancestor crumb asks for; a longer name elides.
    crumb_cap: f32,
}

/// What a breadcrumb bar reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct BreadcrumbOutput<K> {
    /// The bar's response.
    pub response: Response,
    /// The crumb opened this pass. The last crumb, where the user is, never reports.
    pub picked: Option<K>,
}

impl<K> Outcome for BreadcrumbOutput<K> {
    type Value = crate::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Where the parts of one pass sit.
#[derive(Default)]
struct Laid {
    /// How many leading crumbs stand behind the ellipsis.
    hidden: usize,
    ellipsis: Option<Rect>,
    /// The shown crumbs' cells, whether each is cut short, in order.
    cells: Vec<(Rect, bool)>,
    separators: Vec<Rect>,
}

/// A path of crumbs, the root first. Each crumb but the last is a link that reports its value in the output.
#[derive(crate::contract::Component)]
#[component(overlay, probe(BreadcrumbBar<'static, u8>))]
pub struct BreadcrumbBar<'a, K: Value + 'static> {
    #[common]
    common: Common,
    crumbs: &'a [Entry<K>],
    picked: Option<K>,
    laid: Laid,
    whole: Label,
}

impl<'a, K: Value + 'static> BreadcrumbBar<'a, K> {
    /// A bar named `label` over `crumbs`, the root first.
    pub fn new(label: impl Into<Label>, crumbs: &'a [Entry<K>]) -> Self {
        debug_assert!(crumbs.iter().all(|crumb| !crumb.is_separator()), "a `BreadcrumbBar` has no separators");
        BreadcrumbBar { common: Common::new(), crumbs, picked: None, laid: Laid::default(), whole: Label::Empty }.label(label)
    }

    fn part(at: usize) -> PartRef {
        CRUMB.at(at as u64)
    }

    fn is_last(&self, at: usize) -> bool {
        at + 1 == self.crumbs.len()
    }

    /// The face crumb `at` wears: plain text for an ancestor, strong text for the current crumb.
    fn crumb_face(&self, at: usize, label: &Label) -> CrumbFace {
        if self.is_last(at) {
            CrumbFace::Current(Text::new(label.clone()).role(TypeRole::BodyStrong).elide(Elide::End))
        } else {
            CrumbFace::Ancestor(Text::new(label.clone()).elide(Elide::Path))
        }
    }

    /// The size crumb `at` shows whole.
    fn natural(&self, at: usize, cx: &MeasureCx<'_, Self>) -> Vec2 {
        match self.crumb_face(at, self.crumbs[at].label()) {
            CrumbFace::Current(text) => cx.face(text),
            CrumbFace::Ancestor(link) => cx.face(link),
        }
    }

    /// The width crumb `at` asks for: its natural width, an ancestor's capped so a long name elides.
    fn wanted(&self, at: usize, natural: f32, cap: f32) -> f32 {
        if self.is_last(at) { natural } else { natural.min(cap) }
    }
}

enum CrumbFace {
    Ancestor(Text),
    Current(Text),
}

/// Widths for crumbs that want `natural` points each in `room` points: the shortest keep what they want and the rest
/// share what is left equally.
fn fair_widths(natural: &[f32], room: f32) -> Vec<f32> {
    let mut order: Vec<usize> = (0..natural.len()).collect();
    order.sort_by(|&a, &b| natural[a].total_cmp(&natural[b]));
    let mut widths = vec![0.0; natural.len()];
    let mut left = room;
    for (taken, &at) in order.iter().enumerate() {
        let share = left / (natural.len() - taken) as f32;
        widths[at] = natural[at].min(share).floor();
        left -= widths[at];
    }
    widths
}

/// The segments of a file path as crumbs, each valued by the path that opens it, spelled as in `path`: its own
/// separators (`/` or `\`), a leading `/` or a UNC `\\server\share` kept in the values, empty parts skipped. A bare
/// drive (`C:`) as the first crumb gets its separator back in its value, since on its own it means the current folder
/// on that drive.
pub fn path_crumbs(path: &str) -> Vec<Entry<String>> {
    let mut out: Vec<Entry<String>> = Vec::new();
    let mut start = 0;
    for (at, c) in path.char_indices().chain(std::iter::once((path.len(), '/'))) {
        if c != '/' && c != '\\' {
            continue;
        }
        if let Some(part) = path.get(start..at).filter(|part| !part.is_empty()) {
            let mut key = path.get(..at).unwrap_or(path).to_owned();
            if out.is_empty() && part.ends_with(':') && key == part {
                key.push(if path.contains('\\') { '\\' } else { '/' });
            }
            out.push(Entry::new(key, part.to_owned()));
        }
        start = at + c.len_utf8();
    }
    out
}

impl<K: Value + 'static> Component for BreadcrumbBar<'_, K> {
    const ROLE: Role = Role::NAVIGATION;
    const KEYS: KeyUse = KeyUse::of(crate::contract::Input::Pointer).with_activate(ActivateKeys::Both);
    type Style = BreadcrumbBarStyle;
    type Output = BreadcrumbOutput<K>;

    fn style(cx: &StyleCx<'_>, _: BreadcrumbBarStyleKey<K>) -> BreadcrumbBarStyle {
        let icon = cx.tokens.size.icon.sm;
        let gap = cx.tokens.space.gap.tight;
        let muted = cx.color(cx.tokens.color.text.muted);
        BreadcrumbBarStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            separator: IconLook::new(icon, Stateful::new(muted)),
            gap,
            joint: icon + 2.0 * gap,
            ellipsis_pad: 2.0 * gap,
            crumb_cap: cx.tokens.size.length.field,
        }
    }

    /// The width of the whole path with every crumb and separator, at most the room the parent offers: a path that
    /// does not fit overflows behind the ellipsis instead of widening the bar past its room.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let faces: Vec<Vec2> = (0..self.crumbs.len()).map(|at| self.natural(at, cx)).collect();
        let crumbs: f32 = faces.iter().map(|face| face.x).sum();
        let height = faces.iter().map(|face| face.y).fold(cx.face(Text::new("x")).y, f32::max);
        let joints = self.crumbs.len().saturating_sub(1) as f32 * cx.style.joint;
        let whole = crumbs + joints;
        Vec2::new(cx.room.map_or(whole, |room| whole.min(room)), height)
    }

    fn fills(&self) -> Option<Axis> {
        Some(Axis::X)
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let (gap, chevron_side) = (cx.style.gap, cx.style.separator.side);
        let total = self.crumbs.len();
        let labels: Vec<String> = self.crumbs.iter().map(|crumb| cx.env.resolve(crumb.label()).as_str().to_owned()).collect();
        self.whole = Label::from(labels.join(PATH_JOINT));
        let ellipsis_width = cx.face(Text::new("\u{2026}")).x + cx.style.ellipsis_pad;
        let full: Vec<f32> = (0..total).map(|at| self.natural(at, cx).x).collect();
        let wanted: Vec<f32> = (0..total).map(|at| self.wanted(at, full[at], cx.style.crumb_cap)).collect();
        // The fit runs from the current crumb back, so the root end overflows first.
        let backwards: Vec<FitItem> =
            (0..total).rev().map(|at| FitItem::new(wanted[at], 0, if self.is_last(at) { FitKeep::Pinned } else { FitKeep::Free })).collect();
        let hidden = fit_items(&backwards, rect.width(), cx.style.joint, cx.style.joint, ellipsis_width).overflow.len();
        let shown = total - hidden;
        let mut laid = Laid { hidden, ..Laid::default() };
        let mut flow = Flow::new(rect, cx.flow.is_rtl());
        if hidden > 0 {
            let (cell, rest) = flow.take_start(ellipsis_width);
            laid.ellipsis = Some(cell);
            let (_, rest) = rest.take_start(gap);
            let (chevron, rest) = rest.take_start(chevron_side);
            laid.separators.push(chevron);
            flow = rest.take_start(gap).1;
        }
        let room = (flow.rect().width() - shown.saturating_sub(1) as f32 * cx.style.joint).max(0.0);
        let widths = fair_widths(&wanted[hidden..], room);
        for (at, width) in (hidden..total).zip(widths) {
            let (cell, rest) = flow.take_start(width);
            laid.cells.push((cell, width < full[at]));
            flow = rest;
            if !self.is_last(at) {
                let (_, rest) = flow.take_start(gap);
                let (chevron, rest) = rest.take_start(chevron_side);
                laid.separators.push(chevron);
                flow = rest.take_start(gap).1;
            }
        }
        if let Some(cell) = laid.ellipsis {
            cx.part(HIDDEN, cell).sense(Sense::CLICK | Sense::FOCUS).focusable().tooltip(self.whole.clone()).toggles("hidden");
        }
        for (at, (cell, cut)) in (hidden..total).zip(laid.cells.iter().copied()) {
            let part = cx.part(Self::part(at), cell);
            if self.is_last(at) {
                let part = part.sense(Sense::HOVER);
                if cut {
                    part.tooltip(self.crumbs[at].label().clone());
                }
            } else {
                let tip = if cut { self.crumbs[at].label().clone() } else { self.whole.clone() };
                part.sense(Sense::CLICK | Sense::FOCUS).focusable().tooltip(tip);
            }
        }
        self.laid = laid;
        cx.roving(Roving::new(NavShape::Flat));
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        for at in self.laid.hidden..self.crumbs.len().saturating_sub(1) {
            if cx.part_activated(Self::part(at)).is_some() {
                self.picked = self.crumbs[at].value().cloned();
            }
        }
    }

    /// The menu of the hidden crumbs, the root first: one row per crumb.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        if self.laid.ellipsis.is_none() {
            return;
        }
        let popup = cx.popup("hidden");
        let hidden = &self.crumbs[..self.laid.hidden];
        let name = cx.words::<BarWords>().breadcrumb_hidden.clone();
        let picked = ui.show(popup, crate::menu::layer(false), |ui, _| Menu::inline(name.clone()).pick(ui, hidden).picked).flatten();
        if picked.is_some() {
            self.picked = picked;
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        for (at, crumb) in self.crumbs.iter().enumerate().skip(self.laid.hidden) {
            let current = self.is_last(at);
            let node = access.node(Self::part(at), if current { Role::LABEL } else { Role::LINK });
            node.name = Name::Text(cx.env.resolve(crumb.label()).as_str().to_owned());
            if current {
                node.current(true);
            }
        }
        if self.laid.ellipsis.is_some() {
            access.node(HIDDEN, Role::BUTTON).name = Name::Text(cx.words::<BarWords>().breadcrumb_hidden.as_str().to_owned());
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if self.laid.ellipsis.is_some() {
            ui.face(HIDDEN, Text::new("\u{2026}").shade(Shade::Muted));
        }
        for (at, _) in (self.laid.hidden..self.crumbs.len()).zip(&self.laid.cells) {
            match self.crumb_face(at, self.crumbs[at].label()) {
                CrumbFace::Current(text) => ui.face(Self::part(at), text),
                CrumbFace::Ancestor(link) => ui.face(Self::part(at), link),
            }
        }
        let side = cx.style.separator.side;
        let color = cx.resolve(&cx.style.separator.color);
        for slot in &self.laid.separators {
            let glyph = Rect::from_center_size(slot.center(), Vec2::splat(side));
            ui.draw().icon(glyph, &IconSpec::svg(IconId::new("chevron_right")).color(color));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> BreadcrumbOutput<K> {
        BreadcrumbOutput { response: cx.response.clone(), picked: self.picked }
    }
}
