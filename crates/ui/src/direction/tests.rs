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

// Dialog, floating-window and title-band chrome (the CL-05 slice).

#[test]
fn float_window_x_pins_to_the_reading_end_and_clamps_both_sides() {
    let canvas = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let (ltr, rtl) = (Direction::Ltr, Direction::Rtl);
    assert_eq!(float_window_x(&ltr, canvas, 200.0, 16.0, 0.0), 800.0 - 200.0 - 16.0);
    assert_eq!(float_window_x(&rtl, canvas, 200.0, 16.0, 0.0), 16.0);
    // The cascade runs toward reading start (clamped at the canvas edge in RTL).
    assert_eq!(float_window_x(&ltr, canvas, 200.0, 16.0, 28.0), 800.0 - 200.0 - 16.0 + 28.0);
    assert_eq!(float_window_x(&rtl, canvas, 200.0, 16.0, 28.0), 8.0);
    // LTR clamps the near (left) side; a huge negative cascade lands on it.
    assert_eq!(float_window_x(&ltr, canvas, 200.0, 16.0, -10000.0), 8.0);
    // RTL clamps both sides.
    assert_eq!(float_window_x(&rtl, canvas, 200.0, 16.0, 10000.0), 8.0);
    assert_eq!(float_window_x(&rtl, canvas, 200.0, 16.0, -10000.0), 800.0 - 8.0 - 200.0);
    // A window wider than the canvas still keeps 8 px on it.
    assert_eq!(float_window_x(&ltr, canvas, 900.0, 16.0, 0.0), 8.0);
    assert_eq!(float_window_x(&rtl, canvas, 900.0, 16.0, 0.0), 8.0);
}

#[test]
fn clamp_window_min_size_is_exact_min_and_keeps_the_anchored_edge() {
    let rect = Rect::from_min_size(pos2(100.0, 50.0), vec2(400.0, 300.0));
    let min = vec2(200.0, 150.0);
    let ltr = clamp_window_min_size(&Direction::Ltr, rect, min);
    assert_eq!(ltr, Rect::from_min_size(pos2(100.0, 50.0), min), "LTR anchors the left edge");
    let rtl = clamp_window_min_size(&Direction::Rtl, rect, min);
    assert_eq!(rtl, Rect::from_min_size(pos2(500.0 - 200.0, 50.0), min), "RTL anchors the right edge");
    assert_eq!(rtl.size(), min);
}

#[test]
fn close_box_hugs_the_band_far_end_and_the_drag_band_excludes_it() {
    let band = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 30.0));
    let title = Rect::from_min_size(pos2(40.0, 5.0), vec2(100.0, 20.0));
    for d in [Direction::Ltr, Direction::Rtl] {
        let band_end = if d == Direction::Rtl { band.left() } else { band.right() };
        let close = close_box_rect(&d, band_end, title.center().y);
        assert_eq!(close.size(), vec2(CLOSE_BOX_SIZE, CLOSE_BOX_SIZE));
        assert!((close.center().y - title.center().y).abs() < 1e-6);
        if d == Direction::Rtl {
            assert_eq!(close.left(), band.left(), "RTL box flush with the band's left end");
        } else {
            assert_eq!(close.right(), band.right(), "LTR box flush with the band's right end");
        }
        let drag = title_drag_band(&d, band, title, Some(close));
        if d == Direction::Rtl {
            assert_eq!(drag.left(), close.right(), "RTL band starts past the box's inner side");
            assert!((drag.right() - (title.right() + 7.0)).abs() < 1e-6);
        } else {
            assert_eq!(drag.right(), close.left(), "LTR band ends before the box's inner side");
            assert!((drag.left() - (title.left() - 7.0)).abs() < 1e-6);
        }
        // Without a close box the band reaches 7 px past the band edge.
        let drag = title_drag_band(&d, band, title, None);
        if d == Direction::Rtl {
            assert!((drag.left() - (band.left() - 7.0)).abs() < 1e-6);
        } else {
            assert!((drag.right() - (band.right() + 7.0)).abs() < 1e-6);
        }
    }
}

#[test]
fn trailing_min_hugs_the_reading_end_and_keeps_slot_order() {
    for d in [Direction::Ltr, Direction::Rtl] {
        let ctx = context(d);
        let mut rects = Vec::new();
        frame(&ctx, |ui| {
            ui.with_layout(trailing_min(&d), |ui| {
                rects.push(ui.button("ok").rect);
                rects.push(ui.button("cancel").rect);
            });
        });
        let group_left = rects.iter().map(|r| r.left()).fold(f32::MAX, f32::min);
        let group_right = rects.iter().map(|r| r.right()).fold(0.0, f32::max);
        if d == Direction::Ltr {
            assert!(group_right > WIDTH - 20.0, "LTR footer hugs the right: {group_right}");
            assert!(rects[0].right() > rects[1].right(), "first slot at the reading end: {rects:?}");
        } else {
            assert!(group_left < 20.0, "RTL footer hugs the left: {group_left}");
            assert!(rects[0].left() < rects[1].left(), "add order unchanged, mirrored by the row: {rects:?}");
        }
    }
}

#[test]
fn dialog_row_is_full_width_and_one_interact_height() {
    for d in [Direction::Ltr, Direction::Rtl] {
        let ctx = context(d);
        let (mut row_frame, mut interact_h) = (Rect::ZERO, 0.0);
        frame(&ctx, |ui| {
            interact_h = ui.spacing().interact_size.y;
            let _ = dialog_row(ui, |ui| {
                let _ = ui.label("row");
                row_frame = ui.max_rect();
            });
        });
        assert!(row_frame.width() > WIDTH * 0.9, "{d:?}: full-width row frame: {row_frame:?}");
        assert!((row_frame.height() - interact_h).abs() < 1.0, "{d:?}: one interact height: {row_frame:?} vs {interact_h}");
    }
}

/// Outer rect and label rect of a window whose body runs through `window_body`, over a few
/// frames so a sizing pass has happened.
fn window_body_rects(d: Direction) -> (Rect, Rect) {
    let ctx = context(d);
    let (mut win, mut label) = (Rect::ZERO, Rect::ZERO);
    for _ in 0..4 {
        frame(&ctx, |ui| {
            if let Some(ir) = egui::Window::new("w").default_width(300.0).show(ui.ctx(), |ui| {
                window_body(ui, |ui| {
                    label = ui.label("a somewhat long label").rect;
                    ui.horizontal(|ui| {
                        let _ = ui.button("ok");
                        let _ = ui.button("cancel");
                    });
                });
            }) {
                win = ir.response.rect;
            }
        });
    }
    (win, label)
}

#[test]
fn window_body_mirrors_once_and_does_not_balloon_in_rtl() {
    let (ltr_win, ltr_label) = window_body_rects(Direction::Ltr);
    let (rtl_win, rtl_label) = window_body_rects(Direction::Rtl);
    assert!(ltr_label.left() < WIDTH / 2.0, "LTR label at the left: {ltr_label:?}");
    assert!(rtl_label.right() > WIDTH / 2.0, "RTL label right-aligned: {rtl_label:?}");
    assert!(
        (rtl_win.width() - ltr_win.width()).abs() < 16.0,
        "the RTL sizing pass must not balloon the width: ltr {} rtl {}",
        ltr_win.width(),
        rtl_win.width()
    );
}

#[test]
fn dialog_content_stacks_title_over_question_in_both_directions() {
    for d in [Direction::Ltr, Direction::Rtl] {
        let ctx = context(d);
        let (mut title, mut question) = (Rect::ZERO, Rect::ZERO);
        for _ in 0..4 {
            frame(&ctx, |ui| {
                let _ = egui::Window::new("d").show(ui.ctx(), |ui| {
                    dialog_content(ui, |ui| {
                        title = ui.label("Discard changes?").rect;
                        question = ui.label("Do you want to discard the document?").rect;
                    });
                });
            });
        }
        assert!(title.bottom() <= question.top() + 0.5, "{d:?}: title and question share a baseline: {title:?} {question:?}");
    }
}

#[test]
fn sizing_pass_ltr_restores_the_direction_afterwards() {
    let ctx = context(Direction::Rtl);
    frame(&ctx, |ui| {
        let _ = sizing_pass_ltr(ui, |ui| ui.label("x"));
        assert_eq!(ui.ctx().direction(), Direction::Rtl, "the scope guard restores the context direction");
        assert!(is_rtl(ui));
    });
}
