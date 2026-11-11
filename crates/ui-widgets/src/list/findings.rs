//! `FindingsList`: a list of findings, each a `ListRow` with a severity icon, its text and an optional detail line,
//! and `FindingsSummary`, the line of counts above it. The list is the contract's collection.

use super::view_words::FindingsWords;
use super::words::ListWords;
use super::{ListRow, RowEvent};
use crate::contract::{
    Bind, BindMut, BodyCx, Collection, CollectionProps, CollectionResponse, Common, Component, ContainerProps, Cx, MeasureCx, PaintUi, Props as _,
    Role, SelectMode, Show as _, Sizing, Styled as _, WordSetCx as _,
};
use crate::notices::{Notice, NoticeLayout};
use crate::primitives::{IconSpec, TextRun};
use crate::style::{ComponentStyle, IconLook, Stateful, StyleCx, SurfaceKind, TextLook, TypeRole};
use craft_behavior::{DisplayOrder, Flow, IconId, Insets, ItemSource, Metrics, Selection, Sense, SpanLayout};
use craft_i18n::Format;
use craft_theme::variant::Tone;
use egui::{Rect, Response, Ui, Vec2};
use std::borrow::Cow;

/// How serious a finding is. The glyph's shape differs per severity as well as its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Severity {
    /// For information.
    Info,
    /// Worth fixing.
    Warning,
    /// Must be fixed.
    Error,
}

impl Severity {
    /// The icon of the severity: its shape differs per severity as well as its meaning.
    fn icon(self) -> IconId {
        match self {
            Severity::Info => IconId::new("info"),
            Severity::Warning => IconId::new("warning"),
            Severity::Error => IconId::new("error"),
        }
    }

    /// The tone of the severity's icon.
    fn tone(self) -> Tone {
        match self {
            Severity::Info => Tone::Info,
            Severity::Warning => Tone::Warn,
            Severity::Error => Tone::Bad,
        }
    }
}

/// One finding: how serious it is, what it says and, optionally, where.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Finding {
    /// The app's id for the finding; it keeps the selection while findings come and go.
    pub id: u64,
    /// How serious the finding is.
    pub severity: Severity,
    /// What it says.
    pub text: String,
    /// Where it is, as a second line under the text ("page 3").
    pub detail: Option<String>,
}

impl Finding {
    /// The finding `id` of `severity` that says `text`.
    pub fn new(id: u64, severity: Severity, text: impl Into<String>) -> Self {
        Finding { id, severity, text: text.into(), detail: None }
    }

    /// Where the finding is.
    #[must_use]
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// The finding as a list row: its severity icon, text and detail line. A screen reader reads the text, the
    /// detail and the severity word.
    pub fn row(&self, words: &FindingsWords) -> ListRow<'static> {
        let level = match self.severity {
            Severity::Info => words.info.as_str(),
            Severity::Warning => words.warning.as_str(),
            Severity::Error => words.error.as_str(),
        };
        let name = match &self.detail {
            Some(detail) => format!("{}, {detail}, {level}", self.text),
            None => format!("{}, {level}", self.text),
        };
        let row = ListRow::new(self.text.clone()).icon(IconSpec::svg(self.severity.icon())).tone(self.severity.tone()).accessible_name(name);
        match &self.detail {
            Some(detail) => row.detail(detail.clone()),
            None => row,
        }
    }
}

/// The line of counts over a list of findings: "No errors", or "1 error, 2 warnings".
pub fn summary_text(words: &FindingsWords, format: &Format, findings: &[Finding]) -> String {
    if findings.is_empty() {
        return words.no_errors.as_str().to_owned();
    }
    let errors = findings.iter().filter(|finding| finding.severity == Severity::Error).count();
    let warnings = findings.iter().filter(|finding| finding.severity == Severity::Warning).count();
    let count = |text: &craft_i18n::Count, n: usize| text.text(n as u64, &format.integer(n as u64));
    words.counts.fill(&[("errors", &count(&words.errors, errors)), ("warnings", &count(&words.warnings, warnings))])
}

/// The findings of a slice as an item source, keyed by `Finding::id`.
#[derive(Clone, Copy, Debug)]
pub struct Findings<'a>(&'a [Finding]);

impl<'a> Findings<'a> {
    /// The findings of `findings`.
    pub const fn new(findings: &'a [Finding]) -> Self {
        Findings(findings)
    }
}

impl DisplayOrder<u64> for Findings<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> u64 {
        self.0[at].id
    }

    fn index_of(&self, key: &u64) -> Option<usize> {
        self.0.iter().position(|finding| finding.id == *key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|finding| Cow::Borrowed(finding.text.as_str()))
    }
}

impl ItemSource<u64> for Findings<'_> {}

/// The style of the list: it has no look of its own beyond its container's surface.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FindingsListStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The list of findings: one selected finding, Enter activates, and an empty list says so.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(collection)]
pub struct FindingsList<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    items: CollectionProps,
    /// The selection the caller owns.
    #[prop]
    selection: Option<Bind<'a, Selection<u64>>>,
}

impl<'a> FindingsList<'a> {
    /// A findings list named `label`.
    pub fn new(label: impl Into<crate::runtime::env::Label>) -> Self {
        FindingsList {
            common: Common::new(),
            props: ContainerProps::new().surface(SurfaceKind::Sunken).sizing(Sizing::Fill),
            items: CollectionProps::new().select(SelectMode::Single),
            selection: None,
        }
        .label(label)
    }
}

impl Component for FindingsList<'_> {
    const ROLE: Role = Role::LIST;
    const PARTS: bool = false;
    type Style = FindingsListStyle;
    type Output = CollectionResponse<u64, RowEvent>;

    fn style(_: &StyleCx<'_>, _: FindingsListStyleKey) -> FindingsListStyle {
        FindingsListStyle::default()
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> Self::Output {
        cx.collection_response()
    }
}

impl Collection for FindingsList<'_> {
    type Key = u64;
    type ItemLayout = SpanLayout;
    type Selection = Selection<u64>;
    type Item<'r> = ListRow<'r>;
    type ItemEvent = RowEvent;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn selection(&mut self) -> Option<BindMut<'_, Selection<u64>>> {
        self.selection.as_mut().map(Bind::reborrow)
    }

    fn item_layout(&self, source: &dyn ItemSource<u64>, width: f32, cx: &StyleCx<'_>) -> SpanLayout {
        SpanLayout::sized(source, width, cx.control().height)
    }

    fn empty_height(&self, _width: f32, _ui: &Ui, cx: &StyleCx<'_>) -> f32 {
        cx.control().height
    }

    fn empty(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        let words = cx.words::<ListWords>();
        Notice::new(Tone::Neutral, words.no_findings.clone()).layout(NoticeLayout::Centred).show_at(ui, cx.rect);
    }
}

/// The style of the summary line.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FindingsSummaryStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(text)]
    text: TextLook,
    /// The icon when there are no errors.
    #[style(icon)]
    good: IconLook,
    /// The icon when there are errors.
    #[style(icon)]
    bad: IconLook,
}

/// The line over a list of findings: an icon that says whether there are errors, then the counts.
#[derive(crate::contract::Component)]
pub struct FindingsSummary<'a> {
    #[common]
    common: Common,
    findings: &'a [Finding],
}

impl<'a> FindingsSummary<'a> {
    /// The summary of `findings`.
    pub fn new(findings: &'a [Finding]) -> Self {
        FindingsSummary { common: Common::new(), findings }
    }

    fn text(&self, words: &FindingsWords, format: &Format) -> String {
        summary_text(words, format, self.findings)
    }

    fn has_errors(&self) -> bool {
        self.findings.iter().any(|finding| finding.severity == Severity::Error)
    }

    fn icon(&self) -> IconId {
        if self.has_errors() { Severity::Error.icon() } else { IconId::new("success") }
    }
}

impl Component for FindingsSummary<'_> {
    const ROLE: Role = Role::LABEL;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = FindingsSummaryStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: FindingsSummaryStyleKey) -> FindingsSummaryStyle {
        let control = cx.control();
        FindingsSummaryStyle {
            metrics: Metrics::new().padding(Insets::horizontal(control.pad_x)).gap(control.gap),
            text: TextLook::new(TypeRole::Label, Stateful::new(cx.color(cx.tokens.color.text.normal))),
            good: IconLook::new(control.icon, Stateful::new(cx.tone(Tone::Good))),
            bad: IconLook::new(control.icon, Stateful::new(cx.tone(Tone::Bad))),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let words = cx.words::<FindingsWords>();
        let text = cx.layout(&TextRun::new(self.text(&words, cx.format()))).size();
        Vec2::new(text.x + cx.icon() + cx.gap(), text.y.max(cx.icon()))
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let rtl = cx.env.direction().is_rtl();
        let (glyph, rest) = Flow::new(cx.content_rect, rtl).take_start(cx.style.good.side);
        let (_, rest) = rest.take_start(cx.style.metrics.gap);
        let draw = ui.draw();
        let look = if self.has_errors() { &cx.style.bad } else { &cx.style.good };
        draw.icon(Rect::from_center_size(glyph.center(), Vec2::splat(look.side)), &IconSpec::svg(self.icon()).color(cx.resolve(&look.color)));
        let words = cx.words::<FindingsWords>();
        draw.text(rest.rect(), &TextRun::new(self.text(&words, cx.format())).color(cx.resolve(&cx.style.text.color)).elide());
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
