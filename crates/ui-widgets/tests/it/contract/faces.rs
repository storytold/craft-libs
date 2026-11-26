//! Faces take their ink from the composite that paints them: its text colour, or the ink it names for one face.

use craft_behavior::{Metrics, PartKey, Sense};
use craft_theme::TypeRole;
use craft_ui::choice::Rating;
use craft_ui::contract::{Bind, Common, Component, Cx, PaintUi, PartsCx, Props as _, Role, Show as _, Styled as _};
use craft_ui::style::{ComponentStyle, Stateful, StyleColor, StyleCx, TextLook};
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, trial};
use egui::{Color32, Rect, Response, Shape, pos2, vec2};
use std::cell::Cell;

const HOSTED: PartKey = PartKey::new("hosted");
const GIVEN: PartKey = PartKey::new("given");

#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
struct TwoFacesStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(text)]
    name: TextLook,
    given: Stateful<StyleColor>,
}

/// A composite that shows the same rating as the face of two parts: the start half with its own text colour, the end
/// half with the ink it names, which it reports in `given`.
#[derive(Component)]
struct TwoFaces<'a> {
    #[common]
    common: Common,
    given: &'a Cell<Color32>,
}

impl Component for TwoFaces<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = TwoFacesStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: TwoFacesStyleKey) -> TwoFacesStyle {
        let colors = &cx.tokens.color;
        TwoFacesStyle {
            metrics: Metrics::new().min_size(vec2(200.0, 40.0)),
            name: TextLook::new(TypeRole::Body, Stateful::new(cx.color(colors.text.normal))),
            given: Stateful::new(cx.color(colors.status.bad.fg)),
        }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let (start, end) = rect.split_left_right_at_fraction(0.5);
        let _ = cx.part(HOSTED, start);
        let _ = cx.part(GIVEN, end);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let stars = || Rating::new(Bind::given(5u8), 5).compact(true);
        ui.face(HOSTED, stars());
        ui.face_in(GIVEN, stars(), &cx.style.given);
        self.given.set(cx.resolve(&cx.style.given).egui());
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

#[trial]
fn a_face_shown_with_face_in_reads_the_ink_it_is_given() {
    let mut frames = Frames::new();
    let named = Cell::new(Color32::TRANSPARENT);
    let (mut rect, mut shapes) = (Rect::NOTHING, Vec::new());
    for _ in 0..3 {
        shapes = frames.frame_shapes(|ui| {
            let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 40.0));
            rect = in_region(ui, region, |ui| TwoFaces { common: Common::new(), given: &named }.id_salt("faces").show(ui).rect);
        });
    }
    let middle = rect.center().x;
    let tints = |start: bool| -> Vec<Color32> {
        let mut tints: Vec<Color32> = shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::Mesh(mesh) if (mesh.calc_bounds().center().x < middle) == start => Some(mesh.vertices.iter().map(|vertex| vertex.color)),
                _ => None,
            })
            .flatten()
            .filter(|color| color.a() > 0)
            .collect();
        tints.dedup();
        tints
    };
    let (hosted, given) = (tints(true), tints(false));
    assert!(!hosted.is_empty() && !given.is_empty(), "both faces paint their stars");
    assert!(given.iter().all(|tint| *tint == named.get()), "the face shown with face_in paints in the ink it is given: {given:?}");
    assert!(!hosted.contains(&named.get()), "the face shown with face paints in the composite's text colour: {hosted:?}");
}
