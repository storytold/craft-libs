//! `LinkedFields`: number fields joined by a link toggle that keeps their values in proportion (width and height) or
//! equal (four sides).

#[cfg(feature = "testing")]
mod stories;

use super::NumberField;
use crate::buttons::ToggleButton;
use crate::contract::{
    Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, Outcome, PaintUi, Props as _, Remember as _, Role, SideLabel,
    WordSetCx as _,
};
use crate::primitives::TextRun;
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{EditEvents, Flow, IconId, Metrics, Mixed, Proposal, Sense};
use craft_i18n::Format;
use egui::{Rect, Response, Vec2, pos2};
use std::ops::{Deref, RangeInclusive};
use std::sync::Arc;

craft_ui::words! {
    /// The words of the link toggle.
    pub struct LinkedWords {
        keep_ratio: "linkedFields.keepRatio" = "Maintain aspect ratio";
        same_values: "linkedFields.sameValues" = "Make all settings the same";
        /// A field with no caption: `{group}` is the group's label and `{n}` the field's place, counted from 1.
        field_name: template "linkedFields.fieldName" = "{group} {n}";
    }
}

/// What the link keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LinkMode {
    /// A value typed into one field scales the others by the same factor: width and height.
    #[default]
    Ratio,
    /// A value typed into one field is copied to the others: four sides, X and Y offsets.
    Equal,
}

impl LinkMode {
    /// The link toggle's tooltip and accessible name for this mode.
    pub fn label(self, words: &LinkedWords) -> Label {
        match self {
            LinkMode::Ratio => words.keep_ratio.clone(),
            LinkMode::Equal => words.same_values.clone(),
        }
    }
}

/// How the fields and the link are placed in the room offered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LinkedLayout {
    /// The fields in one row, the link after the first.
    #[default]
    Row,
    /// The fields stacked, the link beside them, centred on their height.
    Column,
    /// Two columns of fields (the first half, the second half) with the link between them, centred on their height.
    Columns,
}

/// The chain toggle as a `ToggleButton` face: an unlinked chain while off, a linked chain while on, named by `label`
/// (`LinkMode::label`). `LinkedFields` shows it between its fields; a transform bar or form that keeps its own fields
/// shows the same toggle beside them and applies `follow_ratio` or `follow_equal` to its edits.
pub fn link_toggle<'a>(label: impl Into<Label>, linked: Bind<'a, bool>) -> ToggleButton<'a> {
    ToggleButton::new(label, linked).icon(IconId::new("unlink")).on_icon(IconId::new("link")).icon_only()
}

/// The style of linked fields: the gap between the fields and the link.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct LinkedFieldsStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What linked fields remember between passes: the values a gesture began with, so every value it reports is built
/// from them, and the link state of the last passes, to see the link turn on.
#[derive(Clone, Default)]
struct Memo<const N: usize> {
    base: Option<Parts<N>>,
    /// The link state the last pass ended with, as the link child showed it.
    seen: Option<bool>,
    /// What `seen` held when the last pass began, to see the link turn on.
    before: Option<bool>,
}

/// The value of linked fields: each field's number, shared or mixed on its own.
type Parts<const N: usize> = [Mixed<Option<f64>>; N];

/// The output of linked fields: the values' edit, and the link's edit when the toggle was clicked.
#[derive(Debug)]
#[non_exhaustive]
pub struct LinkedOutput<const N: usize> {
    /// The edit of the values: every field's value after the link rule, one event per gesture.
    pub edit: EditResponse<Parts<N>>,
    /// The link toggle's edits.
    pub link: EditEvents<bool>,
}

impl<const N: usize> Outcome for LinkedOutput<N> {
    type Value = Parts<N>;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.edit.story_facts()
    }
}

impl<const N: usize> Deref for LinkedOutput<N> {
    type Target = EditResponse<Parts<N>>;

    fn deref(&self) -> &Self::Target {
        &self.edit
    }
}

/// `N` number fields joined by a link toggle. An edit of one field moves the others with it while linked: in
/// `LinkMode::Ratio` by its factor, kept inside each field's range; in `LinkMode::Equal` to the same value. The value
/// is one `[Option<f64>; N]`, so a linked edit is one gesture and one event. Every value a gesture reports is built from
/// the values at its start, so a scrub or a value typed digit by digit keeps the starting ratio. Unlinked, a field
/// moves alone.
///
/// Turning the link on in `LinkMode::Equal` sets every field to the first one's value in one step; in
/// `LinkMode::Ratio` it changes no value. A field that is blank or mixed takes no part in a ratio. A field is blank
/// only while its own part is mixed (or empty); an edit reports the fields it changes as shared and the others as
/// mixed, so on a selection where only the height differs, typing a width commits `[Same(width), Mixed]` and the
/// app writes the width alone to every target.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(probe(LinkedFields<'static, 2>))]
pub struct LinkedFields<'a, const N: usize> {
    #[common]
    common: Common,
    values: Bind<'a, Parts<N>>,
    link: Bind<'a, bool>,
    /// The link child's event of this pass and the state it showed, kept from `paint` to `finish`.
    link_event: EditEvents<bool>,
    link_shown: Option<bool>,
    /// What the link keeps.
    #[prop]
    mode: LinkMode,
    /// How the fields and the link are placed.
    #[prop]
    layout: LinkedLayout,
    captions: Option<[Label; N]>,
    /// Each field's own range.
    #[prop]
    ranges: Option<[RangeInclusive<f64>; N]>,
}

impl<'a, const N: usize> LinkedFields<'a, N> {
    /// Fields named after `label`, showing `values`, joined by a link toggle that edits `link`.
    pub fn new(label: impl Into<Label>, values: Bind<'a, Parts<N>>, link: Bind<'a, bool>) -> Self {
        LinkedFields {
            common: Common::new(),
            values,
            link,
            link_event: EditEvents::new(),
            link_shown: None,
            mode: LinkMode::Ratio,
            layout: LinkedLayout::Row,
            captions: None,
            ranges: None,
        }
        .label(label)
    }

    /// Shows a caption beside each field that also scrubs it, and names the field. Without captions a field is named
    /// by the group's label and its position.
    #[must_use]
    pub fn captions(mut self, captions: [impl Into<Label>; N]) -> Self {
        self.captions = Some(captions.map(Into::into));
        self
    }

    /// Every field's range.
    #[must_use]
    pub fn range(mut self, range: RangeInclusive<f64>) -> Self {
        self.ranges = Some(std::array::from_fn(|_| range.clone()));
        self
    }

    fn range_of(&self, i: usize) -> RangeInclusive<f64> {
        self.ranges.as_ref().map_or(f64::NEG_INFINITY..=f64::INFINITY, |ranges| ranges[i].clone())
    }

    fn name(&self, i: usize, group: &Label, words: &LinkedWords, format: &Format) -> Label {
        match &self.captions {
            Some(captions) => captions[i].clone(),
            None => Label::from(words.field_name.fill(&[("group", group.as_str()), ("n", format.integer(i as u64 + 1).as_str())])),
        }
    }

    fn field(&self, i: usize, group: &Label, words: &LinkedWords, format: &Format, value: Mixed<Option<f64>>) -> NumberField<'static> {
        NumberField::new(self.name(i, group, words, format), Bind::given(value)).range(self.range_of(i))
    }
}

impl<const N: usize> Component for LinkedFields<'_, N> {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = LinkedFieldsStyle;
    type Output = LinkedOutput<N>;

    fn style(cx: &StyleCx<'_>, _: LinkedFieldsStyleKey<N>) -> LinkedFieldsStyle {
        LinkedFieldsStyle { metrics: Metrics::new().gap(cx.control().gap) }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let shown = shown_of(cx.value());
        let group = cx.label();
        let words = cx.words::<LinkedWords>();
        let gap = cx.gap();
        let colon = cx.layout(&TextRun::new(crate::contract::colon_of(&cx.words::<crate::runtime::words::CommonWords>()))).size().x;
        let mut cell = Vec2::ZERO;
        for (i, value) in shown.iter().enumerate() {
            let mut field = self.field(i, &group, &words, cx.format(), *value);
            let size = cx.child(("field", i), &mut field, Proposal::Ideal);
            let caption = match &self.captions {
                Some(captions) => cx.layout(&TextRun::new(captions[i].as_str())).size().x + colon + gap,
                None => 0.0,
            };
            cell = cell.max(Vec2::new(size.x + caption, size.y));
        }
        let link = cx.child("link", &mut link_toggle("", Bind::given(false)), Proposal::Ideal);
        let cell = Vec2::new(cell.x, cell.y.max(link.y));
        let side = cell.y;
        let n = N as f32;
        match self.layout {
            LinkedLayout::Row => Vec2::new(n * cell.x + side + n * gap, cell.y),
            LinkedLayout::Column => Vec2::new(cell.x + gap + side, n * cell.y + (n - 1.0).max(0.0) * gap),
            LinkedLayout::Columns => {
                let rows = N.div_ceil(2) as f32;
                Vec2::new(2.0 * cell.x + 2.0 * gap + side, rows * cell.y + (rows - 1.0).max(0.0) * gap)
            }
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, Parts<N>>> {
        Some(self.values.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let shown = cx.value().copied();
        let running = cx.gesture().is_some();
        let memo = cx.memory::<Memo<N>>();
        if !running {
            memo.base = Some(shown_of(shown.as_ref()));
        }
        let was = std::mem::replace(&mut memo.before, memo.seen);
        let turned_on = memo.seen == Some(true) && was == Some(false);
        if let (true, LinkMode::Equal, Some(values)) = (turned_on, self.mode, shown)
            && let Mixed::Same(Some(first)) = values[0]
            && values.iter().any(|value| *value != Mixed::Same(Some(first)))
        {
            cx.set([Mixed::Same(Some(first)); N]);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let shown = shown_of(cx.value());
        let base = cx.recall::<Memo<N>>().base.unwrap_or(shown);
        let group = cx.label();
        let words = cx.words::<LinkedWords>();
        let ranges: [RangeInclusive<f64>; N] = std::array::from_fn(|i| self.range_of(i));
        let mode = self.mode;
        let (cells, link_cell) = arrange(self.layout, N, cx.rect.size(), cx.style.metrics.gap);
        let flow = Flow::new(cx.rect, cx.env.direction().is_rtl());
        let place = |logical: Rect| flow.to_physical(logical.translate(cx.rect.min.to_vec2()));

        // The fields are shown first, so the Tab order is the fields, then the link. The link's own pipeline reads,
        // applies and records its edit; the fields follow the state it shows after the pass, so their edits are
        // adopted once the link has been shown.
        let children: Vec<_> = cells
            .into_iter()
            .enumerate()
            .map(|(i, cell)| {
                let field = self.field(i, &group, &words, cx.format(), shown[i]);
                match &self.captions {
                    Some(captions) => {
                        ui.labelled(("field", i), place(cell), captions[i].clone(), SideLabel::new().scrub(true), field.scrub_by_label())
                    }
                    None => ui.child(("field", i), place(cell), field),
                }
            })
            .collect();
        let link = ui.child("link", place(link_cell), link_toggle(mode.label(&words), self.link.reborrowed()));
        let is_on = |shown: &Mixed<Arc<bool>>| matches!(shown, Mixed::Same(on) if **on);
        let linked = link.shown.as_ref().is_some_and(is_on);
        self.link_shown = link.shown.as_ref().map(is_on);
        self.link_event = link.events;
        for (i, child) in children.iter().enumerate() {
            cx.adopt(child, &base, |value, mine| *mine = follow(mode, linked, base, i, *value, &ranges));
        }
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> LinkedOutput<N> {
        cx.memory::<Memo<N>>().seen = self.link_shown;
        LinkedOutput { edit: cx.edit_response(), link: self.link_event }
    }
}

/// The rects of the `n` fields and of the link in a room of `size`, in logical coordinates from its start edge and top.
fn arrange(layout: LinkedLayout, n: usize, size: Vec2, gap: f32) -> (Vec<Rect>, Rect) {
    let rect = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pos2(x, y), Vec2::new(w, h));
    let count = n.max(1) as f32;
    match layout {
        LinkedLayout::Row => {
            let side = size.y;
            let width = ((size.x - side - count * gap) / count).max(0.0);
            let mut cells = Vec::with_capacity(n);
            let mut link = rect(0.0, 0.0, side, side);
            let mut x = 0.0;
            for i in 0..n {
                cells.push(rect(x, 0.0, width, size.y));
                x += width + gap;
                if i == 0 {
                    link = rect(x, 0.0, side, size.y);
                    x += side + gap;
                }
            }
            (cells, link)
        }
        LinkedLayout::Column => {
            let height = ((size.y - (count - 1.0) * gap) / count).max(0.0);
            let width = (size.x - gap - height).max(0.0);
            let cells = (0..n).map(|i| rect(0.0, i as f32 * (height + gap), width, height)).collect();
            (cells, rect(width + gap, (size.y - height) / 2.0, height, height))
        }
        LinkedLayout::Columns => {
            let first = n.div_ceil(2);
            let rows = first.max(1) as f32;
            let height = ((size.y - (rows - 1.0) * gap) / rows).max(0.0);
            let width = ((size.x - 2.0 * gap - height) / 2.0).max(0.0);
            let second = width + gap + height + gap;
            let cells = (0..n)
                .map(|i| {
                    if i < first {
                        rect(0.0, i as f32 * (height + gap), width, height)
                    } else {
                        rect(second, (i - first) as f32 * (height + gap), width, height)
                    }
                })
                .collect();
            (cells, rect(width + gap, (size.y - height) / 2.0, height, height))
        }
    }
}

/// The value of the fields when the binding shows none: every part mixed.
fn shown_of<const N: usize>(shown: Option<&Parts<N>>) -> Parts<N> {
    shown.copied().unwrap_or([Mixed::Mixed; N])
}

/// The parts after field `changed` became `new`, under the link: `base` are the parts the gesture began with. The
/// edit's mixed parts are the ones it leaves alone. Unlinked, or with a blank `new`, only the field changes. A blank
/// or mixed field takes no part in a ratio, and a mixed field has no number to scale; an equal link sets every field.
fn follow<const N: usize>(
    mode: LinkMode,
    linked: bool,
    base: Parts<N>,
    changed: usize,
    new: Option<f64>,
    ranges: &[RangeInclusive<f64>; N],
) -> Parts<N> {
    let mut alone = base;
    alone[changed] = Mixed::Same(new);
    let (true, Some(new)) = (linked, new) else { return alone };
    let numbers = base.map(|part| part.same().copied().flatten().unwrap_or(0.0));
    match mode {
        LinkMode::Ratio => {
            let Mixed::Same(Some(_)) = base[changed] else { return alone };
            let scaled = follow_ratio(numbers, changed, new, ranges);
            std::array::from_fn(|i| if i == changed || matches!(base[i], Mixed::Same(Some(_))) { Mixed::Same(Some(scaled[i])) } else { base[i] })
        }
        LinkMode::Equal => {
            let equal = follow_equal(numbers, changed, new);
            std::array::from_fn(|i| Mixed::Same(Some(equal[i].max(*ranges[i].start()).min(*ranges[i].end()))))
        }
    }
}

/// The values after field `changed` became `new` under a ratio link: every value scaled by `new / values[changed]`,
/// the factor lowered so no value leaves its range in `ranges`. An old value of zero scales nothing: only `changed`
/// moves. A gesture passes the values at its start, so the ratio never drifts.
pub fn follow_ratio<const N: usize>(values: [f64; N], changed: usize, new: f64, ranges: &[RangeInclusive<f64>; N]) -> [f64; N] {
    let old = values[changed];
    if old == 0.0 || !old.is_finite() || !new.is_finite() {
        let mut out = values;
        out[changed] = new;
        return out;
    }
    let wanted = new / old;
    let mut factor = wanted;
    for (value, range) in values.iter().zip(ranges) {
        if *value == 0.0 {
            continue;
        }
        let (a, b) = (range.start() / value, range.end() / value);
        let (low, high) = if *value > 0.0 { (a, b) } else { (b, a) };
        factor = factor.max(low).min(high);
    }
    // Multiplying before dividing keeps `100 * 200 / 500`-style results free of rounding noise.
    let mut out = values.map(|value| if factor == wanted { value * new / old } else { value * factor });
    out[changed] = if factor == wanted { new } else { old * factor };
    out
}

/// The values after field `changed` became `new` under an equal link: all `new`.
pub fn follow_equal<const N: usize>(values: [f64; N], _changed: usize, new: f64) -> [f64; N] {
    values.map(|_| new)
}
