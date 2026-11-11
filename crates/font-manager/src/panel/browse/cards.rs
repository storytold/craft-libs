//! The family card and the collection of cards the panel lists.

use super::*;

/// What a family card asks the panel for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CardEvent {
    /// Install the family.
    Install,
    /// Download the family for a preview.
    Preview,
    /// Remove the family from the font store.
    Remove,
    /// Try the failed job again.
    Retry,
}

/// What a card shows at its reading end.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CardState {
    /// A job runs: what it does and how far it is.
    Running(Kind, Option<f32>),
    /// The last job failed: what it did, why, and whether Retry applies.
    Failed(Kind, String, bool),
    /// The family is installed and can't be removed here.
    Installed,
    /// The family is in the font store.
    Removable,
    /// A system family.
    System,
    /// Not installed: Install, and Preview until a preview is downloaded.
    #[non_exhaustive]
    Offer {
        /// Whether Preview is offered.
        preview: bool,
    },
}

/// What a family card shows.
#[derive(Clone)]
#[non_exhaustive]
pub struct CardData {
    /// The family name.
    pub name: String,
    /// Styles, categories and source on one line.
    pub meta: String,
    /// The sample text.
    pub sample: String,
    /// The sample size in points.
    pub points: f32,
    /// False for a family from a source the user added.
    pub curated: bool,
    /// What is at the reading end.
    pub state: CardState,
    /// The face the sample is drawn in, and whose catalog holds it.
    pub face: Option<(Owner, FaceInfo, Arc<Catalog>)>,
}

/// The pieces of a card at the reading end, in reading order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum End {
    NotCurated,
    Failed,
    Retry,
    Installed,
    Preview,
    Install,
    Remove,
    RunningText,
    RunningBar,
}

impl End {
    fn event(self) -> Option<CardEvent> {
        match self {
            End::Install => Some(CardEvent::Install),
            End::Preview => Some(CardEvent::Preview),
            End::Remove => Some(CardEvent::Remove),
            End::Retry => Some(CardEvent::Retry),
            _ => None,
        }
    }
}

/// Builds the control for one piece of a card's end and runs `$body` on it as `$c`. A control is built where it is
/// measured and again where it is shown, so both come from this one list.
macro_rules! with_end {
    ($card:expr, $words:expr, $end:expr, |$c:ident| $body:expr) => {
        match $end {
            End::NotCurated => {
                let $c = Badge::new($words.not_curated.clone())
                    .variant(Variant::look(Emphasis::Soft, Tone::Warn))
                    .tooltip($words.not_curated_tip.as_str().to_owned());
                $body
            }
            End::Failed => {
                let $c = $card.failed_text(&$words);
                $body
            }
            End::Retry => {
                let $c = Button::new($words.retry.clone());
                $body
            }
            End::Installed => {
                let $c = Badge::new($words.installed.clone()).variant(Variant::look(Emphasis::Soft, Tone::Good));
                $body
            }
            End::Preview => {
                let $c = Link::new($words.preview.clone()).description($words.preview_family.fill(&[("family", $card.data.name.as_str())]));
                $body
            }
            End::Install => {
                let $c = Button::new($words.install.clone()).description($words.install_family.fill(&[("family", $card.data.name.as_str())]));
                $body
            }
            End::Remove => {
                let $c = Button::new($words.remove.clone())
                    .variant(Variant::DANGER)
                    .description($words.remove_family.fill(&[("family", $card.data.name.as_str())]));
                $body
            }
            End::RunningText => {
                let $c = Text::new(Label::from($card.running_text(&$words))).shade(Shade::Muted);
                $body
            }
            End::RunningBar => {
                let $c = match $card.fraction() {
                    Some(fraction) => Progress::new(fraction),
                    None => Progress::indeterminate(),
                }
                .label(Label::from($card.running_text(&$words)));
                $body
            }
        }
    };
}

/// The style of a family card.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FamilyCardStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The family name.
    #[style(text)]
    name: TextLook,
    /// Styles, categories and source.
    #[style(text)]
    meta: TextLook,
    /// Space round the card's contents.
    pad: f32,
    /// Space between the card's parts.
    gap: f32,
    /// The header's least height.
    line: f32,
}

/// Where the card's parts sit, from the parts step to the paint.
#[derive(Clone, Debug)]
struct Placed {
    name: Rect,
    meta: Option<Rect>,
    end: Vec<(End, Rect)>,
    sample: Rect,
    rule: (Pos2, Pos2),
}

/// The sizes of the card's parts at one width.
struct Plan {
    name: Vec2,
    end: Vec<(End, Vec2)>,
    header: f32,
    sample: Vec2,
}

/// What a family card returns.
#[non_exhaustive]
pub struct CardOut {
    /// The card's response.
    pub response: Response,
    /// What the pressed control asks for.
    pub event: Option<CardEvent>,
}

impl Outcome for CardOut {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl ItemOutput<CardEvent> for CardOut {
    fn into_event(self) -> Option<CardEvent> {
        self.event
    }
}

/// One family: its name and meta line, its state at the reading end and a sample in the face below. The controls at the
/// end and the sample are child components of the card, so a press on a control never selects the card.
#[derive(craft_ui::contract::Component)]
pub struct FamilyCard {
    #[common]
    common: Common,
    data: CardData,
    placed: Option<Placed>,
    event: Option<CardEvent>,
}

impl FamilyCard {
    /// A card showing `data`.
    pub fn new(data: CardData) -> Self {
        let name = data.name.clone();
        FamilyCard { common: Common::new(), data, placed: None, event: None }.label(name)
    }

    fn fraction(&self) -> Option<f32> {
        match self.data.state {
            CardState::Running(_, fraction) => fraction,
            _ => None,
        }
    }

    fn running_text(&self, words: &FontsBrowseWords) -> String {
        let CardState::Running(kind, fraction) = &self.data.state else { return String::new() };
        let what = match kind {
            Kind::Install => words.installing.as_str(),
            Kind::Remove => words.removing.as_str(),
            Kind::Preview => words.downloading.as_str(),
        };
        match fraction {
            Some(fraction) => words.progress.fill(&[("what", what), ("percent", &((fraction * 100.0).round() as u64).to_string())]),
            None => words.working.fill(&[("what", what)]),
        }
    }

    /// The "Couldn't ..." text of a failed job, with the error as its tooltip.
    fn failed_text(&self, words: &FontsBrowseWords) -> Text {
        let (what, error) = match &self.data.state {
            CardState::Failed(kind, error, _) => {
                let what = match kind {
                    Kind::Install => &words.failed_install,
                    Kind::Remove => &words.failed_remove,
                    Kind::Preview => &words.failed_download,
                };
                (what.clone(), error.clone())
            }
            _ => (Label::Empty, String::new()),
        };
        Text::new(what).tone(Tone::Bad).tooltip(error)
    }

    /// The pieces at the reading end, in reading order.
    fn ends(&self) -> Vec<End> {
        let mut ends = Vec::new();
        if !self.data.curated {
            ends.push(End::NotCurated);
        }
        match &self.data.state {
            CardState::Running(..) => ends.extend([End::RunningText, End::RunningBar]),
            CardState::Failed(_, _, retry) => {
                ends.push(End::Failed);
                if *retry {
                    ends.push(End::Retry);
                }
            }
            CardState::Installed => ends.push(End::Installed),
            CardState::Removable => ends.push(End::Remove),
            CardState::System => {}
            CardState::Offer { preview } => {
                if *preview {
                    ends.push(End::Preview);
                }
                ends.push(End::Install);
            }
        }
        ends
    }

    /// The face the sample is drawn in: the card's own, or the interface face when there is none.
    fn face(&self) -> SampleFace<'_> {
        match &self.data.face {
            Some((owner, face, catalog)) => SampleFace::Face { catalog, owner: *owner, face },
            None => SampleFace::Interface,
        }
    }

    fn sample<'a>(&'a self, words: &FontsBrowseWords) -> FontSample<'a> {
        FontSample::new(words.card_sample.clone(), self.data.sample.as_str(), self.face(), self.data.points)
    }

    /// The sizes of the parts at `width`.
    fn plan(&self, cx: &MeasureCx<'_, Self>, width: f32) -> Plan {
        let style = cx.style;
        let words = cx.env.words::<FontsBrowseWords>();
        let name = cx.layout(&TextRun::new(self.data.name.as_str()).role(style.name.role)).size();
        let end: Vec<(End, Vec2)> = self
            .ends()
            .into_iter()
            .map(|end| (end, with_end!(self, words, end, |child| cx.child(("end", end as u8), &mut { child }, Proposal::Ideal))))
            .collect();
        let header = end.iter().map(|(_, size)| size.y).fold(name.y.max(style.line), f32::max);
        let inner = (width - 2.0 * style.pad).max(0.0);
        let mut sample = self.sample(&words);
        let sample = cx.child("sample", &mut sample, Proposal::Width(inner));
        Plan { name, end, header, sample }
    }
}

impl Component for FamilyCard {
    const ROLE: Role = Role::LIST_ITEM;
    type Style = FamilyCardStyle;
    type Output = CardOut;

    fn style(cx: &StyleCx<'_>, _: FamilyCardStyleKey) -> FamilyCardStyle {
        FamilyCardStyle {
            metrics: Metrics::new(),
            name: TextLook::new(TypeRole::BodyStrong, Stateful::new(cx.ink(Ink::OnPanel))),
            meta: TextLook::new(TypeRole::Caption, Stateful::new(cx.color(cx.tokens.color.text.faint))),
            pad: cx.tokens.space.pad.regular,
            gap: cx.tokens.space.gap.inline,
            line: cx.control().height,
        }
    }

    /// The card is named by its family and its meta line, which the layout may elide.
    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.name(Name::Text(format!("{}, {}", self.data.name, self.data.meta)));
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let style = cx.style;
        let width = match cx.room {
            Some(room) => room,
            None => {
                let ideal = self.plan(cx, 0.0);
                2.0 * style.pad + ideal.name.x + ideal.end.iter().map(|(_, size)| size.x + style.gap).sum::<f32>()
            }
        };
        let plan = self.plan(cx, width);
        Vec2::new(width, style.pad + plan.header + style.gap + plan.sample.y + style.pad)
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let rtl = cx.flow.is_rtl();
        let style = cx.style.clone();
        let plan = self.plan(&*cx, rect.width());
        let header = Rect::from_min_size(
            Pos2::new(rect.left() + style.pad, rect.top() + style.pad),
            Vec2::new((rect.width() - 2.0 * style.pad).max(0.0), plan.header),
        );
        let centred = |slot: Rect, size: Vec2| Rect::from_center_size(Pos2::new(slot.center().x, header.center().y), size);
        let mut flow = Flow::new(header, rtl);
        let mut end = Vec::new();
        for (piece, size) in plan.end.iter().rev() {
            let (slot, rest) = flow.take_end(size.x);
            end.push((*piece, centred(slot, *size)));
            flow = rest.take_end(style.gap).1;
        }
        end.reverse();
        let name_width = plan.name.x.min(flow.rect().width());
        let (name_slot, rest) = flow.take_start(name_width);
        let meta_slot = rest.take_start(style.gap).1.rect();
        let meta = (meta_slot.width() > 0.0).then_some(meta_slot);
        let sample_top = header.bottom() + style.gap;
        let sample = Rect::from_min_size(Pos2::new(header.left(), sample_top), Vec2::new(header.width(), plan.sample.y));
        self.placed = Some(Placed {
            name: Rect::from_center_size(Pos2::new(name_slot.center().x, header.center().y), Vec2::new(name_width, plan.name.y)),
            meta,
            end,
            sample,
            rule: (rect.left_bottom(), rect.right_bottom()),
        });
    }

    fn paint(&mut self, ui: &mut craft_ui::contract::PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(placed) = self.placed.as_ref() else { return };
        let style = cx.style;
        let words = cx.env.words::<FontsBrowseWords>();
        {
            let draw = ui.draw();
            draw.text(placed.name, &TextRun::new(self.data.name.as_str()).role(style.name.role).color(cx.resolve(&style.name.color)).elide());
            if let Some(meta) = placed.meta {
                draw.text(meta, &TextRun::new(self.data.meta.as_str()).role(style.meta.role).color(cx.resolve(&style.meta.color)).elide());
            }
            draw.rule(RuleKind::Divider, placed.rule.0, placed.rule.1);
        }
        for (piece, rect) in &placed.end {
            let pressed = with_end!(self, words, *piece, |child| clicked(&ui.child(("end", *piece as u8), *rect, child)));
            if pressed {
                self.event = piece.event();
            }
        }
        let sample = self.sample(&words);
        ui.child("sample", placed.sample, sample);
    }

    fn finish(self, cx: Cx<'_, Self>) -> CardOut {
        CardOut { response: cx.response.clone(), event: self.event }
    }
}

/// The style of the card list: it has no look of its own.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FamilyCardsStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The family cards of one list: one Tab stop, Up and Down move the lead, Enter or a double click activates a card.
#[derive(craft_ui::contract::Component)]
#[component(collection)]
pub struct FamilyCards {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    items: CollectionProps,
}

impl FamilyCards {
    /// A list named `label`.
    pub fn new(label: impl Into<Label>) -> Self {
        FamilyCards {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::Fill),
            items: CollectionProps::new().select(SelectMode::Single),
        }
        .label(label)
    }
}

impl Component for FamilyCards {
    const ROLE: Role = Role::LIST;
    type Style = FamilyCardsStyle;
    type Output = CollectionResponse<usize, CardEvent>;

    fn style(_: &StyleCx<'_>, _: FamilyCardsStyleKey) -> FamilyCardsStyle {
        FamilyCardsStyle::default()
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> Self::Output {
        cx.collection_response()
    }
}

impl Collection for FamilyCards {
    type Key = usize;
    type ItemLayout = SpanLayout;
    type Selection = Selection<usize>;
    type Item<'r> = FamilyCard;
    type ItemEvent = CardEvent;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn item_layout(&self, source: &dyn ItemSource<usize>, width: f32, cx: &StyleCx<'_>) -> SpanLayout {
        SpanLayout::sized(source, width, cx.control().height)
    }
}

/// The cards of a list in display order: each key is a position in the index or in the installed families.
pub struct CardKeys<'a> {
    keys: &'a [usize],
    name: Box<dyn Fn(usize) -> Option<&'a str> + 'a>,
}

impl<'a> CardKeys<'a> {
    /// The cards of `keys`; `name` gives a key's family name for type-ahead.
    pub fn new(keys: &'a [usize], name: impl Fn(usize) -> Option<&'a str> + 'a) -> Self {
        CardKeys { keys, name: Box::new(name) }
    }
}

impl DisplayOrder<usize> for CardKeys<'_> {
    fn len(&self) -> usize {
        self.keys.len()
    }

    fn key(&self, at: usize) -> usize {
        self.keys[at]
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        (self.name)(self.keys[at]).map(Cow::Borrowed)
    }
}

impl ItemSource<usize> for CardKeys<'_> {}

impl CardData {
    /// A card for a key the index no longer holds.
    pub(super) fn empty() -> CardData {
        CardData { name: String::new(), meta: String::new(), sample: String::new(), points: 0.0, curated: true, state: CardState::System, face: None }
    }
}
