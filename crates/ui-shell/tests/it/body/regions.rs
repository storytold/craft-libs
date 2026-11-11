//! The regions of a body: a filling body placed from this pass's header and footer.

use craft_behavior::Sense;
use craft_ui::contract::{BodyCx, Common, Component, Container, ContainerProps, Cx, FooterRegion, HeaderRegion, Role, ShowContainer as _, Sizing};
use craft_ui::style::StyleCx;
use craft_ui::testing::layout::allocate;
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Pos2, Rect, Response, Ui, Vec2};
use std::cell::RefCell;

const AREA: Rect = Rect { min: Pos2::new(10.0, 10.0), max: Pos2::new(410.0, 210.0) };

/// The rects the regions of a `Regioned` took.
#[derive(Clone, Copy, Default)]
struct Took {
    header: Option<Rect>,
    body: Option<Rect>,
    end: Option<Rect>,
    footer: Option<Rect>,
}

/// A container whose header, end region and footer each fill a block of the size given (nothing for `None`).
#[derive(craft_ui::contract::Component)]
#[component(container)]
struct Regioned<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    header: Option<Vec2>,
    end: Vec2,
    footer: Vec2,
    took: &'a RefCell<Took>,
}

impl Component for Regioned<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: RegionedStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Regioned<'_> {
    fn header(&mut self, ui: &mut Ui, _: &mut BodyCx<'_>) {
        self.took.borrow_mut().header = self.header.map(|size| allocate(ui, size));
    }

    fn end(&mut self, ui: &mut Ui, _: &mut BodyCx<'_>) {
        self.took.borrow_mut().end = Some(allocate(ui, self.end));
    }

    fn footer(&mut self, ui: &mut Ui, _: &mut BodyCx<'_>) {
        self.took.borrow_mut().footer = Some(allocate(ui, self.footer));
    }
}

/// Shows a filling `Regioned` at `AREA` with `props` for one frame and returns what its regions took in the frame's
/// first pass, before any discard pass: no region has a size from an earlier pass then.
fn first_pass(props: ContainerProps, header: Option<Vec2>) -> Took {
    let took = RefCell::new(Took::default());
    let passes = RefCell::new(Vec::new());
    let mut frames = Frames::new();
    frames.frame(Vec::new(), |ui| {
        let shown =
            Regioned { common: Common::new(), props: props.clone(), header, end: Vec2::new(80.0, 40.0), footer: Vec2::new(70.0, 16.0), took: &took }
                .show_at(ui, AREA, |_, cx| cx.rect);
        took.borrow_mut().body = shown.inner;
        passes.borrow_mut().push(*took.borrow());
    });
    passes.into_inner().first().copied().expect("the frame ran a pass")
}

// Edges snap to whole pixels, so a rectangle may overhang its neighbour by half a point.
const EDGE_SLACK: f32 = 0.5;

#[trial]
fn a_filling_body_lies_between_its_header_and_its_footer_in_the_first_pass() {
    let props = ContainerProps::new().sizing(Sizing::Fill).header(HeaderRegion::new()).footer(FooterRegion::new());
    let took = first_pass(props, Some(Vec2::new(100.0, 20.0)));
    let (header, body, footer) = (took.header.unwrap(), took.body.unwrap(), took.footer.unwrap());
    assert!(body.min.y >= header.max.y - EDGE_SLACK, "the body starts under the header: {header:?} {body:?}");
    assert!(body.max.y <= footer.min.y + EDGE_SLACK, "the body ends at the footer's top: {body:?} {footer:?}");
    assert!(footer.max.y <= AREA.max.y + EDGE_SLACK, "the footer lies inside the container, at its foot: {footer:?}");
}

/// A row body beside an inline header and before an end region is measured in the line they leave, as it is shown:
/// a wrapping text there shows on one line while the line has room for it, between the header and the end region.
#[trial]
fn a_row_body_between_an_inline_header_and_an_end_region_measures_in_the_line_they_leave() {
    use craft_ui::contract::{BodyLayout, EndRegion, Show as _};
    use craft_ui::display::Text;
    use egui::UiBuilder;
    const WORDS: &str = "File  Edit  Image";
    let props =
        ContainerProps::new().sizing(Sizing::FillWidth).layout(BodyLayout::row()).header(HeaderRegion::new().inline(true)).end(EndRegion::new());
    let took = RefCell::new(Took::default());
    let (text, alone) = (RefCell::new(Rect::NOTHING), RefCell::new(Rect::NOTHING));
    let show = |ui: &mut Ui| {
        ui.scope_builder(UiBuilder::new().max_rect(AREA), |ui| {
            let bar = Regioned {
                common: Common::new(),
                props: props.clone(),
                header: Some(Vec2::new(40.0, 20.0)),
                end: Vec2::new(80.0, 20.0),
                footer: Vec2::ZERO,
                took: &took,
            };
            bar.show(ui, |ui, _| *text.borrow_mut() = Text::new(WORDS).show(ui).rect);
        });
        ui.scope_builder(UiBuilder::new().max_rect(AREA.translate(Vec2::new(0.0, 300.0))), |ui| *alone.borrow_mut() = Text::new(WORDS).show(ui).rect);
    };
    let mut frames = Frames::new();
    frames.idle(3, show);
    frames.frame(Vec::new(), show);
    let (text, alone, took) = (*text.borrow(), *alone.borrow(), *took.borrow());
    let (header, end) = (took.header.unwrap(), took.end.unwrap());
    assert!((text.height() - alone.height()).abs() <= EDGE_SLACK, "the text shows on one line: {text:?}, alone {alone:?}");
    assert!(
        text.min.x >= header.max.x - EDGE_SLACK && text.max.x <= end.min.x + EDGE_SLACK,
        "the text lies between {header:?} and {end:?}: {text:?}"
    );
}

/// The content of a body on a line it shares with an inline header or an end region sits on that line's centre, as the
/// end region's does: a title bar's leading text, its title and its trailing text share one centre line with the
/// caption buttons, and a bulk-edit row's check box and field share one.
#[trial]
fn a_body_on_a_line_it_shares_sits_on_the_lines_centre() {
    use craft_ui::contract::Role;
    use craft_ui::contract::{Bind, Show as _};
    use craft_ui::display::Text;
    use craft_ui::sections::GroupBox;
    use craft_ui::shell::{Caption, CaptionFamily, Captions, Frame, Title, TitleBar};
    use craft_ui::testing::Fixture;
    use craft_ui::text::NumberField;
    let centre = |fixture: &Fixture, label: &str| fixture.by_label(label).unwrap_or_else(|| panic!("{label} is shown")).rect.center().y;
    let mut bar = Fixture::new(|ui| {
        let layout = craft_window::Layout { left: vec![], right: vec![Caption::Minimize, Caption::Maximize, Caption::Close] };
        let frame = Frame::Integrated(Captions { family: CaptionFamily::Windows, layout });
        let _ = TitleBar::new().frame(frame).show_titled(
            ui,
            Title::new("Landscape.psd"),
            |ui, _| {
                Text::new("File  Edit").show(ui);
            },
            |ui, _| {
                Text::new("Share").show(ui);
            },
        );
    });
    bar.idle(4);
    let line = bar.by_role(Role::BUTTON).first().expect("the caption buttons are shown").rect.center().y;
    for label in ["File  Edit", "Landscape.psd", "Share"] {
        assert!((centre(&bar, label) - line).abs() <= EDGE_SLACK, "{label} is centred on the bar's line");
    }
    let mut row = Fixture::new(|ui| {
        let mut apply = true;
        GroupBox::apply("Apply", Bind::bound(&mut apply)).show(ui, |ui, _| {
            NumberField::new("Opacity", Bind::given(Some(80.0))).show(ui);
        });
    });
    row.idle(4);
    let check = row.by_role(Role::CHECK_BOX).first().expect("the check box is shown").rect.center().y;
    assert!((centre(&row, "Opacity") - check).abs() <= EDGE_SLACK, "the field and the check box share a centre line");
}
