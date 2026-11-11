//! Stories of the stroke editor and its button: the popover and panel sections, an open path, dashes, a mixed
//! selection and the app's choice rows.

use super::{
    ChoicePainter, LineCap, LineJoin, StrokeAlign, StrokeButton, StrokeChoice, StrokeChoiceRow, StrokeEditor, StrokeMixed, StrokeSections,
    StrokeStyle,
};
use craft_behavior::Mixed;
use craft_ui::contract::{Bind, Role, Show as _, Validity};
use craft_ui::primitives::ContentPainterCx;
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::{Pos2, Rect, Shape, Stroke, Ui};

/// The arrowhead's size as a share of the preview's height.
const HEAD_SHARE: f32 = 0.35;
/// The arrowhead's length in head sizes.
const HEAD_LENGTH: f32 = 2.0;
/// The width profile's half thickness as a share of the preview's height.
const PROFILE_SHARE: f32 = 0.3;
/// The number of segments of a profile outline.
const PROFILE_STEPS: usize = 16;
/// The line through an arrowhead preview, as a share of the preview's height.
const HEAD_LINE_SHARE: f32 = 0.1;

const START: &str = "arrow-start";
const END: &str = "arrow-end";
const PROFILE: &str = "profile";

/// A style that differs from the default in every field the editor edits, for a mixed selection.
fn other_than(base: &StrokeStyle) -> StrokeStyle {
    StrokeStyle {
        align: StrokeAlign::Inside,
        cap: LineCap::Square,
        join: LineJoin::Bevel,
        miter_limit: base.miter_limit / 2.0,
        dashes: StrokeStyle::DASHED.to_vec(),
        dash_offset: 3.0,
        opacity: base.opacity / 2.0,
        ..base.clone()
    }
}

#[story(StrokeEditor<'static>, covers = [], drive = Drive::Never)]
fn stroke_editor_popover(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let style = data.value("popover", StrokeStyle::default());
    StrokeEditor::new("Stroke", Bind::bound(style)).sections(StrokeSections::POPOVER).show(ui).into()
}

#[story(StrokeEditor<'static>, covers = [], drive = Drive::Never)]
fn stroke_editor_panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let style = data.value("panel", StrokeStyle::default());
    StrokeEditor::new("Stroke", Bind::bound(style)).sections(StrokeSections::PANEL).show(ui).into()
}

#[story(StrokeEditor<'static>, covers = [], drive = Drive::Never)]
fn stroke_editor_open_path(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let style = data.value("open", StrokeStyle::default());
    StrokeEditor::new("Stroke", Bind::bound(style)).sections(StrokeSections::POPOVER).closed(false).show(ui).into()
}

#[story(StrokeEditor<'static>, covers = [], drive = Drive::Never)]
fn stroke_editor_dashed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let style = data.value("dashed", StrokeStyle::default().with_line_style(super::LineStyle::Dashed));
    StrokeEditor::new("Stroke", Bind::bound(style)).sections(StrokeSections::POPOVER).show(ui).into()
}

#[story(StrokeEditor<'static>, covers = [Mixed], drive = Drive::Never)]
fn stroke_editor_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (style, mixed) = data.value("mixed", {
        let base = StrokeStyle::default();
        StrokeMixed::of([&base, &other_than(&base)]).expect("two styles")
    });
    StrokeEditor::new("Stroke", Bind::bound(style)).sections(StrokeSections::POPOVER).mixed(mixed).show(ui).into()
}

/// The arrowhead `id` at one end of the line through `rect`.
fn paint_head(cx: &ContentPainterCx, rect: Rect, id: &str, at_end: bool) {
    let ink = cx.ink();
    let line = rect.height() * HEAD_LINE_SHARE;
    let y = rect.center().y;
    let size = rect.height() * HEAD_SHARE;
    let direction = if at_end { 1.0 } else { -1.0 };
    let tip = Pos2::new(if at_end { rect.right() } else { rect.left() }, y);
    let back = |d: f32| Pos2::new(tip.x - direction * d, y);
    cx.painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], Stroke::new(line, ink));
    match id {
        "arrow" => {
            let base = back(size * HEAD_LENGTH);
            cx.painter.add(Shape::convex_polygon(vec![tip, Pos2::new(base.x, y - size), Pos2::new(base.x, y + size)], ink, Stroke::NONE));
        }
        "circle" => {
            cx.painter.circle_filled(back(size), size, ink);
        }
        "square" => {
            cx.painter.rect_filled(Rect::from_center_size(back(size), egui::Vec2::splat(size * HEAD_LENGTH)), 0.0, ink);
        }
        "bar" => {
            cx.painter.line_segment([Pos2::new(tip.x, y - size), Pos2::new(tip.x, y + size)], Stroke::new(line, ink));
        }
        _ => {}
    }
}

/// The width profile `id` along the line through `rect`: half the line's thickness at each point.
fn paint_profile(cx: &ContentPainterCx, rect: Rect, id: &str) {
    let half = rect.height() * PROFILE_SHARE;
    let at = |t: f32| match id {
        "taper" => 1.0 - t,
        "bulge" => (t * std::f32::consts::PI).sin(),
        _ => 0.5,
    };
    let point = |t: f32, side: f32| Pos2::new(rect.left() + rect.width() * t, rect.center().y + side * half * at(t));
    let steps = (0..=PROFILE_STEPS).map(|i| i as f32 / PROFILE_STEPS as f32);
    let mut outline: Vec<Pos2> = steps.clone().map(|t| point(t, -1.0)).collect();
    outline.extend(steps.rev().map(|t| point(t, 1.0)));
    cx.painter.add(Shape::convex_polygon(outline, cx.ink(), Stroke::NONE));
}

/// What the choice-row story keeps: the picks of the three rows.
#[derive(Clone, Debug, PartialEq)]
struct Picks {
    style: StrokeStyle,
    start: String,
    end: String,
    profile: String,
}

#[story(StrokeEditor<'static>, covers = [], drive = Drive::Never)]
fn stroke_editor_choice_rows(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let picks = data.value("rows", Picks { style: StrokeStyle::default(), start: "none".into(), end: "arrow".into(), profile: "uniform".into() });
    let heads = [
        StrokeChoice::new("none", "None"),
        StrokeChoice::new("arrow", "Arrow"),
        StrokeChoice::new("circle", "Circle"),
        StrokeChoice::new("square", "Square"),
        StrokeChoice::new("bar", "Bar"),
    ];
    let profiles = [StrokeChoice::new("uniform", "Uniform"), StrokeChoice::new("taper", "Taper"), StrokeChoice::new("bulge", "Bulge")];
    let rows = [
        StrokeChoiceRow::new(START, "Start", &heads)
            .selected(Mixed::Same(picks.start.as_str()))
            .paint(ChoicePainter::new(|cx, rect, choice| paint_head(cx, rect, &choice.id, false))),
        StrokeChoiceRow::new(END, "End", &heads)
            .selected(Mixed::Same(picks.end.as_str()))
            .paint(ChoicePainter::new(|cx, rect, choice| paint_head(cx, rect, &choice.id, true))),
        StrokeChoiceRow::new(PROFILE, "Profile", &profiles)
            .selected(Mixed::Same(picks.profile.as_str()))
            .paint(ChoicePainter::new(|cx, rect, choice| paint_profile(cx, rect, &choice.id))),
    ];
    let out = StrokeEditor::new("Stroke", Bind::bound(&mut picks.style)).rows(&rows).show(ui);
    let picked = out.choice.clone();
    let story: StoryOutput = out.into();
    drop(rows);
    if let Some((row, choice)) = picked {
        match row.as_str() {
            START => picks.start = choice,
            END => picks.end = choice,
            _ => picks.profile = choice,
        }
    }
    story
}

#[story(StrokeButton<'static>, covers = [Default], drive = Drive::Never)]
fn stroke_button(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (style, open) = data.value("button", (StrokeStyle::default(), false));
    StrokeButton::new("Stroke Options", Bind::bound(style), Bind::bound(open)).show(ui).into()
}

#[story(StrokeButton<'static>, covers = [Mixed], drive = Drive::Never)]
fn stroke_button_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (style, open) = data.value("button mixed", (Mixed::<StrokeStyle>::Mixed, false));
    StrokeButton::new("Stroke Options", Bind::bound_mixed(style), Bind::bound(open)).show(ui).into()
}

#[story(StrokeButton<'static>, covers = [Invalid], drive = Drive::Never)]
fn stroke_button_refused(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (style, open) = data.value("button refused", (StrokeStyle::default().with_line_style(super::LineStyle::Dashed), false));
    let refuse_dashes = |style: &StrokeStyle| if style.dashes.is_empty() { Validity::Ok } else { Validity::Refuse(Message::new("Solid lines only")) };
    StrokeButton::new("Stroke Options", Bind::bound(style).check(refuse_dashes), Bind::bound(open)).show(ui).into()
}

/// Opens the popover with a click on the button, then picks the Dashed preset.
fn pick_dashed(fixture: &mut Fixture) {
    if let Some(button) = fixture.by_role(Role::BUTTON).first().copied() {
        fixture.click(&button);
        fixture.idle(3);
    }
    if let Some(preset) = fixture.by_label("Dashed") {
        fixture.click(&preset);
        fixture.idle(2);
    }
}

#[story(StrokeButton<'static>, covers = [Open], drive = Drive::Custom(pick_dashed))]
fn stroke_button_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let style = data.value("button open", StrokeStyle::default());
    StrokeButton::new("Stroke Options", Bind::bound(style), Bind::given(false)).show(ui).into()
}
