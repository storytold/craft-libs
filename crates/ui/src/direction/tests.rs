use super::*;
use egui::{Pos2, RawInput, Response};

const WIDTH: f32 = 600.0;

fn context(dir: Direction) -> Context {
    let ctx = Context::default();
    apply(&ctx, dir);
    ctx
}

fn frame(ctx: &Context, add: impl FnMut(&mut Ui)) {
    let raw = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(WIDTH, 400.0))), ..Default::default() };
    ctx.run_ui(raw, add).textures_delta.clear();
}

/// Centre x of a panel shown with `panel` and of a label in the space left.
fn panel_and_rest(dir: Direction, panel: fn(&Context, &'static str) -> Panel) -> (f32, f32) {
    let ctx = context(dir);
    let (mut side, mut rest) = (0.0, 0.0);
    for _ in 0..2 {
        frame(&ctx, |ui| {
            side = panel(ui.ctx(), "side").show(ui, |ui| ui.label("side")).response.rect.center().x;
            rest = ui.label("rest").rect.center().x;
        });
    }
    (side, rest)
}

/// Centre x of two labels added in order with `layout`.
fn two_in(dir: Direction, layout: fn(&Ui) -> Layout) -> (f32, f32) {
    let ctx = context(dir);
    let mut xs = (0.0, 0.0);
    frame(&ctx, |ui| {
        let layout = layout(ui);
        ui.with_layout(layout, |ui| {
            let first: Response = ui.label("first");
            let second = ui.label("second");
            xs = (first.rect.center().x, second.rect.center().x);
        });
    });
    xs
}

#[test]
fn direction_is_per_context_and_left_to_right_until_applied() {
    let ctx = Context::default();
    assert_eq!(ctx.direction(), Direction::Ltr);
    apply(&ctx, Direction::Rtl);
    assert!(is_rtl(&ctx));
    assert_eq!(Context::default().direction(), Direction::Ltr);
    apply(&ctx, Direction::Ltr);
    assert!(!is_rtl(&ctx));
}

#[test]
fn setting_overrides_the_language() {
    assert_eq!(Setting::Language.direction("ar"), Direction::Rtl);
    assert_eq!(Setting::Language.direction("he-IL"), Direction::Rtl);
    assert_eq!(Setting::Language.direction("en"), Direction::Ltr);
    assert_eq!(Setting::Rtl.direction("en"), Direction::Rtl);
    assert_eq!(Setting::Ltr.direction("fa"), Direction::Ltr);
    assert_eq!(Setting::parse(" RTL "), Some(Setting::Rtl));
    assert_eq!(Setting::parse("ltr"), Some(Setting::Ltr));
    assert_eq!(Setting::parse("language"), Some(Setting::Language));
    assert_eq!(Setting::parse("ar"), None);
    assert_eq!(Setting::parse(""), None);
}

#[test]
fn start_panel_lands_at_the_right_in_rtl() {
    let (side, rest) = panel_and_rest(Direction::Ltr, start_panel);
    assert!(side < WIDTH / 2.0 && side < rest, "{side} {rest}");
    let (side, rest) = panel_and_rest(Direction::Rtl, start_panel);
    assert!(side > WIDTH / 2.0 && side > rest, "{side} {rest}");
    let (side, _) = panel_and_rest(Direction::Rtl, end_panel);
    assert!(side < WIDTH / 2.0, "{side}");
}

#[test]
fn rows_run_in_reading_order() {
    let (first, second) = two_in(Direction::Ltr, row);
    assert!(first < second);
    let (first, second) = two_in(Direction::Rtl, row);
    assert!(first > second);
    assert!(first > WIDTH / 2.0, "a right-to-left row starts at the right: {first}");
    let (first, _) = two_in(Direction::Rtl, trailing);
    assert!(first < WIDTH / 2.0, "a trailing group starts at the reading end: {first}");
    let (first, _) = two_in(Direction::Ltr, trailing);
    assert!(first > WIDTH / 2.0, "{first}");
}

#[test]
fn nested_rows_follow_and_unmirrored_rows_do_not() {
    let ctx = context(Direction::Rtl);
    let (mut nested, mut fixed) = ((0.0, 0.0), (0.0, 0.0));
    frame(&ctx, |ui| {
        horizontal(ui, |ui| {
            ui.horizontal(|ui| nested = (ui.label("a").rect.center().x, ui.label("b").rect.center().x));
        });
        horizontal(ui, |ui| {
            unmirrored_row(ui, Unmirrored::MediaTransport, |ui| fixed = (ui.label("a").rect.center().x, ui.label("b").rect.center().x));
        });
    });
    assert!(nested.0 > nested.1, "{nested:?}");
    assert!(fixed.0 < fixed.1, "{fixed:?}");
}

#[test]
fn positions_and_popups_mirror() {
    let r = Rect::from_min_max(pos2(10.0, 0.0), pos2(50.0, 10.0));
    let (ltr, rtl) = (Direction::Ltr, Direction::Rtl);
    assert_eq!((start_x(&ltr, r), end_x(&ltr, r), forward(&ltr)), (10.0, 50.0, 1.0));
    assert_eq!((start_x(&rtl, r), end_x(&rtl, r), forward(&rtl)), (50.0, 10.0, -1.0));
    assert_eq!(start_align(&rtl), Align::Max);
    assert_eq!(column(&rtl).horizontal_align(), Align::Max);
    assert_eq!(align2(&rtl, Align2::LEFT_TOP), Align2::RIGHT_TOP);
    assert_eq!(below(&ltr), RectAlign::BOTTOM_START);
    assert_eq!(below(&rtl), RectAlign::BOTTOM_END);
    assert_eq!(above(&rtl), RectAlign::TOP_END);
    assert_eq!(beside(&ltr), RectAlign::RIGHT_START);
    assert_eq!(beside(&rtl), RectAlign::LEFT_START);
}

#[test]
fn arrow_keys_follow_the_visual_direction() {
    let (ltr, rtl) = (Direction::Ltr, Direction::Rtl);
    assert_eq!(step(&ltr, Key::ArrowLeft), Some(Step::Previous));
    assert_eq!(step(&ltr, Key::ArrowRight), Some(Step::Next));
    assert_eq!(step(&rtl, Key::ArrowLeft), Some(Step::Next));
    assert_eq!(step(&rtl, Key::ArrowRight), Some(Step::Previous));
    assert_eq!(step(&rtl, Key::ArrowUp), None);
    for d in [ltr, rtl] {
        for s in [Step::Previous, Step::Next] {
            assert_eq!(step(&d, step_key(&d, s)), Some(s));
        }
    }

    let ctx = context(Direction::Rtl);
    let mut pressed = None;
    let press = egui::Event::Key { key: Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    let raw = RawInput { events: vec![press], ..Default::default() };
    ctx.run_ui(raw, |ui| pressed = pressed_step(ui.ctx())).textures_delta.clear();
    assert_eq!(pressed, Some(Step::Previous));
}

#[test]
fn directional_icons_flip_from_one_list() {
    let (ltr, rtl) = (Direction::Ltr, Direction::Rtl);
    assert_eq!(icon(&ltr, "chevron-left"), Icon { name: "chevron-left", flip: false });
    assert_eq!(icon(&rtl, "chevron-left"), Icon { name: "chevron-right", flip: false });
    assert_eq!(icon(&rtl, "panel-right-close"), Icon { name: "panel-left-close", flip: false });
    assert_eq!(icon(&rtl, "indent-increase"), Icon { name: "indent-increase", flip: true });
    assert_eq!(icon(&ltr, "indent-increase").uv(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)));
    assert_eq!(icon(&rtl, "indent-increase").uv().left(), 1.0);
    for name in KEPT_ICONS.iter().chain(["brush", "layers"].iter()) {
        assert_eq!(icon(&rtl, name), Icon { name, flip: false }, "{name}");
    }

    let mut names: Vec<&str> =
        SWAPPED_ICONS.iter().flat_map(|&(a, b)| [a, b]).chain(FLIPPED_ICONS.iter().copied()).chain(KEPT_ICONS.iter().copied()).collect();
    let count = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), count, "an icon is on one list only");
}

#[test]
fn every_unmirrored_kind_is_listed_once() {
    for (i, kind) in Unmirrored::ALL.into_iter().enumerate() {
        // A new kind fails to compile here until it is placed in `ALL`.
        let position = match kind {
            Unmirrored::Canvas => 0,
            Unmirrored::Image => 1,
            Unmirrored::Ruler => 2,
            Unmirrored::Timeline => 3,
            Unmirrored::Graph => 4,
            Unmirrored::ColourRamp => 5,
            Unmirrored::SliderValue => 6,
            Unmirrored::MediaTransport => 7,
            Unmirrored::Code => 8,
            Unmirrored::Number => 9,
            Unmirrored::Timecode => 10,
        };
        assert_eq!(position, i, "{kind:?}");
    }
}

/// `Style::right_to_left` as egui's `Debug` output shows it: `None` on crates.io egui, which has
/// no such field, so the test builds against both.
fn style_rtl(ctx: &Context, theme: egui::Theme) -> Option<bool> {
    let debug = format!("{:?}", ctx.style_of(theme));
    debug.contains("right_to_left").then(|| debug.contains("right_to_left: true"))
}

#[test]
fn apply_mirrors_egui_chrome_where_egui_can() {
    let ctx = context(Direction::Rtl);
    let patched = style_rtl(&ctx, egui::Theme::Dark).is_some();
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        assert_eq!(style_rtl(&ctx, theme), patched.then_some(true), "{theme:?}");
    }
    apply(&ctx, Direction::Ltr);
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        assert_eq!(style_rtl(&ctx, theme), patched.then_some(false), "{theme:?}");
    }
}
