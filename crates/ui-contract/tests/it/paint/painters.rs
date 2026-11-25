//! The pure painters: focus rings, scroll bars, handles and the face id.

use craft_ui::testing::trial;
use egui::{pos2, vec2};

const EPS: f32 = 1e-3;

#[trial]
fn ellipse_points_lie_on_the_ellipse_whatever_the_conjugate_axes() {
    let center = pos2(50.0, 40.0);
    // A circle of radius 30 turned by 30 degrees and squeezed to a skew: the axes are conjugate, not orthogonal.
    let axes = [vec2(30.0, 0.0), vec2(15.0, 20.0)];
    let points = craft_ui::primitives::ellipse(center, axes);
    assert!(points.len() >= 16);
    // Solve p - center = a * axes[0] + b * axes[1]; the points are where a^2 + b^2 = 1.
    let det = axes[0].x * axes[1].y - axes[0].y * axes[1].x;
    for p in points {
        let d = p - center;
        let a = (d.x * axes[1].y - d.y * axes[1].x) / det;
        let b = (axes[0].x * d.y - axes[0].y * d.x) / det;
        assert!((a * a + b * b - 1.0).abs() < EPS, "{p:?} is off the ellipse");
    }
}

#[trial]
fn a_concave_polygon_fills_its_area_once_in_either_winding() {
    use egui::{Color32, Pos2};

    let cross = |a: Pos2, b: Pos2, c: Pos2| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    let area = |ring: &[Pos2]| (0..ring.len()).map(|i| cross(Pos2::ZERO, ring[i], ring[(i + 1) % ring.len()])).sum::<f32>().abs() / 2.0;
    // Even-odd ray cast to the right.
    let inside = |ring: &[Pos2], p: Pos2| {
        (0..ring.len()).fold(false, |odd, i| {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) { !odd } else { odd }
        })
    };
    let ell = vec![pos2(0.0, 0.0), pos2(40.0, 0.0), pos2(40.0, 10.0), pos2(10.0, 10.0), pos2(10.0, 30.0), pos2(0.0, 30.0)];
    // A doughnut sector over 2 rad: the outer arc out, the inner arc back.
    let on = |radius: f32, i: usize| {
        let angle = 2.0 * i as f32 / 40.0;
        pos2(60.0 + radius * angle.sin(), 60.0 - radius * angle.cos())
    };
    let sector: Vec<Pos2> = (0..=40).map(|i| on(50.0, i)).chain((0..=40).rev().map(|i| on(25.0, i))).collect();

    for shape in [ell, sector] {
        for ring in [shape.clone(), shape.iter().rev().copied().collect()] {
            let mesh = craft_ui::primitives::fill_polygon(&ring, Color32::WHITE, 0.0);
            let triangles: Vec<[Pos2; 3]> = mesh.indices.chunks(3).map(|t| [0, 1, 2].map(|k| mesh.vertices[t[k] as usize].pos)).collect();
            let turns: Vec<f32> = triangles.iter().map(|&[a, b, c]| cross(a, b, c)).collect();
            assert!(turns.iter().all(|t| *t > 0.0) || turns.iter().all(|t| *t < 0.0), "every triangle turns the same way");
            let covered: f32 = turns.iter().map(|t| t.abs() / 2.0).sum();
            let whole = area(&ring);
            assert!((covered - whole).abs() < whole * 1e-3, "the triangles cover {covered}, the polygon {whole}");
            for [a, b, c] in triangles {
                let centroid = pos2((a.x + b.x + c.x) / 3.0, (a.y + b.y + c.y) / 3.0);
                assert!(inside(&ring, centroid), "a triangle around {centroid:?} lies outside the polygon");
            }
        }
    }
}

#[trial]
fn a_rounded_ramp_keeps_inside_its_corners_and_a_square_one_fills_its_rect() {
    use craft_behavior::Axis;
    use craft_ui::style::DataColor;
    use egui::{Color32, CornerRadius, Rect, Shape};

    let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(60.0, 20.0));
    let stops = [(0.0, DataColor::new(Color32::RED)), (0.5, DataColor::new(Color32::GREEN)), (1.0, DataColor::new(Color32::BLUE))];
    let points = |radius: CornerRadius, axis: Axis| -> Vec<egui::Pos2> {
        craft_ui::primitives::ramp(rect, radius, &stops, axis)
            .into_iter()
            .flat_map(|shape| match shape {
                Shape::Mesh(mesh) => mesh.vertices.iter().map(|v| v.pos).collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect()
    };

    let square = points(CornerRadius::ZERO, Axis::X);
    for corner in [rect.left_top(), rect.right_top(), rect.left_bottom(), rect.right_bottom()] {
        assert!(square.iter().any(|p| p.distance(corner) < EPS), "a square ramp reaches {corner:?}");
    }

    let r = 6.0;
    for axis in [Axis::X, Axis::Y] {
        let rounded = points(CornerRadius::same(6), axis);
        assert!(rounded.len() > square.len(), "the rounded edge has samples along its corners");
        for p in rounded {
            assert!(rect.expand(EPS).contains(p), "{p:?} is outside the rect");
            // Inside a corner square, a point lies within the arc's circle.
            let cx = p.x.clamp(rect.left() + r, rect.right() - r);
            let cy = p.y.clamp(rect.top() + r, rect.bottom() - r);
            assert!(p.distance(pos2(cx, cy)) <= r + EPS, "{p:?} sticks out past a rounded corner along {axis:?}");
        }
    }
}

#[trial]
fn an_edge_rule_lies_on_the_pixels_just_inside_its_side_at_any_scale() {
    use craft_ui::primitives::{RectSide, Rule};
    use craft_ui::style::{RuleLook, RuleWidth};
    use egui::{Color32, Rect, Shape};

    for ppp in [1.0, 1.5, 2.0] {
        // Edges on physical pixel boundaries, as laid-out rows and cells have them.
        let rect = Rect::from_min_max(pos2(30.0, 45.0), pos2(150.0, 90.0)) / ppp;
        for width in [RuleWidth::Hairline, RuleWidth::Points(2.0)] {
            let look = RuleLook { color: Color32::WHITE, width };
            for side in [RectSide::Top, RectSide::Bottom, RectSide::Left, RectSide::Right] {
                let shapes = Rule::edge(&look, rect, side, ppp);
                let [Shape::LineSegment { points: [from, to], stroke }] = shapes.as_slice() else {
                    panic!("one line segment for {side:?}");
                };
                let half = stroke.width / 2.0;
                let horizontal = matches!(side, RectSide::Top | RectSide::Bottom);
                let (at, low, high) = if horizontal { (from.y, rect.top(), rect.bottom()) } else { (from.x, rect.left(), rect.right()) };
                assert!(at - half >= low - EPS && at + half <= high + EPS, "{side:?} at {ppp}x: the band {at}±{half} leaves the rect");
                let edge = match side {
                    RectSide::Top => at - half - low,
                    RectSide::Left => at - half - low,
                    _ => high - (at + half),
                };
                assert!(edge.abs() < EPS, "{side:?} at {ppp}x: the band does not touch its side");
                let along = if horizontal { (from.x, to.x, rect.left(), rect.right()) } else { (from.y, to.y, rect.top(), rect.bottom()) };
                assert!((along.0 - along.2).abs() < EPS && (along.1 - along.3).abs() < EPS, "{side:?} runs the whole side");
            }
        }
    }
}

/// A swath is painted in the theme's text-selection fill, line and dots alike.
#[trial]
fn a_swath_takes_the_text_selection_fill() {
    use craft_theme::{Contrast, Mode};
    use egui::Shape;

    let tokens = craft_ui::testing::style::tokens(Mode::Dark, Contrast::Standard);
    let want = craft_ui::style::rgba_color(tokens.color.state.text_selection);
    let shapes = craft_ui::primitives::swath(&tokens, &[pos2(10.0, 10.0), pos2(50.0, 20.0)], 8.0);
    assert_eq!(shapes.len(), 3, "a line and a dot at each of its two points: {shapes:?}");
    for shape in &shapes {
        match shape {
            Shape::Path(path) => assert_eq!(path.stroke.color, egui::epaint::ColorMode::Solid(want)),
            Shape::Circle(dot) => assert_eq!(dot.fill, want),
            other => panic!("unexpected shape {other:?}"),
        }
    }
}

#[trial]
fn a_dotted_rule_is_whole_pixel_dots_at_one_pitch_between_its_ends() {
    use craft_ui::primitives::Rule;
    use craft_ui::style::{RuleLook, RuleWidth};
    use egui::{Color32, Rect, Shape};

    let whole = |v: f32| (v - v.round()).abs() < EPS;
    for ppp in [1.0, 1.5, 2.0] {
        for width in [RuleWidth::Hairline, RuleWidth::Points(1.0), RuleWidth::Points(2.0)] {
            let look = RuleLook { color: Color32::WHITE, width };
            // Ends off the pixel grid, level and upright.
            for (from, to) in [(pos2(10.3, 20.6), pos2(41.1, 20.6)), (pos2(7.7, 3.2), pos2(7.7, 29.9))] {
                let shapes = Rule::dotted(&look, from, to, 3.0, ppp);
                let [Shape::Mesh(mesh)] = shapes.as_slice() else { panic!("the dots are one mesh") };
                let dots: Vec<Rect> =
                    mesh.vertices.chunks(4).map(|quad| Rect::from_points(&quad.iter().map(|v| v.pos).collect::<Vec<_>>())).collect();
                assert!(dots.len() > 2, "{width:?} at {ppp}x: several dots");
                let level = from.y == to.y;
                let across = width.points(ppp);
                let span = if level {
                    Rect::from_two_pos(from, to).expand2(egui::vec2(0.0, across))
                } else {
                    Rect::from_two_pos(from, to).expand2(egui::vec2(across, 0.0))
                };
                for dot in &dots {
                    let px = *dot * ppp;
                    assert!([px.min.x, px.min.y, px.max.x, px.max.y].into_iter().all(whole), "{width:?} at {ppp}x: {px:?} is not whole pixels");
                    assert!((dot.width() - dot.height()).abs() < EPS, "{width:?} at {ppp}x: a dot is square");
                    assert!(span.expand(EPS).contains_rect(*dot), "{width:?} at {ppp}x: {dot:?} leaves the line {from:?}..{to:?}");
                }
                let starts: Vec<f32> = dots.iter().map(|dot| if level { dot.min.x } else { dot.min.y }).collect();
                let pitch = starts[1] - starts[0];
                assert!(pitch > dots[0].width(), "{width:?} at {ppp}x: dots are apart");
                assert!(starts.windows(2).all(|w| (w[1] - w[0] - pitch).abs() < EPS), "{width:?} at {ppp}x: one pitch: {starts:?}");
            }
        }
    }
}

#[trial]
fn ticks_stand_across_a_level_rule_and_lie_across_an_upright_one() {
    use craft_ui::primitives::Rule;
    use craft_ui::style::{RuleLook, RuleWidth};
    use egui::{Color32, Rect, Shape};

    let whole = |v: f32| (v - v.round()).abs() < EPS;
    for ppp in [1.0, 1.25, 1.5, 2.0] {
        let look = RuleLook { color: Color32::WHITE, width: RuleWidth::Points(1.0) };
        let side = look.width.points(ppp);
        let long = (4.0 * ppp).round() / ppp;
        for (from, to) in [(pos2(10.3, 20.6), pos2(31.1, 20.6)), (pos2(7.7, 3.2), pos2(7.7, 24.9))] {
            let shapes = Rule::ticks(&look, from, to, 1.0, 4.0, ppp);
            let [Shape::Mesh(mesh)] = shapes.as_slice() else { panic!("the ticks are one mesh") };
            let ticks: Vec<Rect> = mesh.vertices.chunks(4).map(|quad| Rect::from_points(&quad.iter().map(|v| v.pos).collect::<Vec<_>>())).collect();
            assert!(ticks.len() > 2, "at {ppp}x: several ticks");
            let level = from.y == to.y;
            for tick in &ticks {
                let px = *tick * ppp;
                assert!([px.min.x, px.min.y, px.max.x, px.max.y].into_iter().all(whole), "at {ppp}x: {px:?} is not whole pixels");
                let (along, across) = if level { (tick.width(), tick.height()) } else { (tick.height(), tick.width()) };
                assert!((along - side).abs() < EPS && (across - long).abs() < EPS, "at {ppp}x: {tick:?} is {side} along, {long} across");
            }
            let starts: Vec<f32> = ticks.iter().map(|tick| if level { tick.min.x } else { tick.min.y }).collect();
            assert!(starts.windows(2).all(|w| w[1] - w[0] > side + EPS), "at {ppp}x: ticks are apart: {starts:?}");
        }
    }
}

#[trial]
fn a_dotted_rule_is_the_ticks_as_long_as_the_line_is_wide() {
    use craft_ui::primitives::Rule;
    use craft_ui::style::{RuleLook, RuleWidth};
    use egui::Color32;

    for ppp in [1.0, 1.5, 2.0] {
        let look = RuleLook { color: Color32::WHITE, width: RuleWidth::Points(1.0) };
        let (from, to) = (pos2(10.3, 20.6), pos2(41.1, 20.6));
        let across = look.width.points(ppp);
        assert_eq!(Rule::dotted(&look, from, to, 2.0, ppp), Rule::ticks(&look, from, to, 2.0, across, ppp), "at {ppp}x");
    }
}

#[trial]
fn ant_edges_are_whole_device_pixels_at_a_fractional_scale() {
    use craft_theme::{Contrast, Mode};
    use egui::{Rect, Shape};
    use std::collections::BTreeSet;

    let tokens = craft_ui::testing::style::tokens(Mode::Dark, Contrast::Standard);
    let ppp = 1.5;
    // Corners off the device pixel grid, as a zoomed selection puts them.
    let corners = [pos2(10.3, 10.7), pos2(60.2, 10.7), pos2(60.2, 40.1), pos2(10.3, 40.1)];
    let clip = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0));
    let shapes = craft_ui::primitives::marching_ants(&tokens, &corners, true, 0.0, clip, ppp);
    let [Shape::Mesh(mesh)] = shapes.as_slice() else { panic!("an axis-aligned outline is one mesh, got {shapes:?}") };
    // Every quad in device pixels: (x0, y0, x1, y1) and its colour; the first quad of each edge is its light band.
    let quads: Vec<([f32; 4], egui::Color32)> = mesh
        .vertices
        .chunks(4)
        .map(|quad| {
            let rect = Rect::from_points(&quad.iter().map(|vertex| vertex.pos).collect::<Vec<_>>());
            ([rect.min.x * ppp, rect.min.y * ppp, rect.max.x * ppp, rect.max.y * ppp], quad[0].color)
        })
        .collect();
    for (quad, _) in &quads {
        assert!(quad.iter().all(|v| (v - v.round()).abs() < EPS), "{quad:?} lies on whole device pixels");
    }
    let light = quads[0].1;
    let bands: Vec<[f32; 4]> = quads.iter().filter(|(_, color)| *color == light).map(|(quad, _)| *quad).collect();
    assert_eq!(bands.len(), 4, "one band per edge");
    let thickness: Vec<f32> = bands.iter().map(|b| (b[2] - b[0]).min(b[3] - b[1]).round()).collect();
    assert!(
        thickness.iter().all(|t| *t == thickness[0] && *t >= 2.0),
        "every edge is as thick as the others, two device pixels at 1.5x: {thickness:?}"
    );

    // The top band's two pixel rows: each row's dashes start one band width earlier than the row above, so a dash ends
    // on a slant.
    let top = bands.iter().copied().min_by(|a, b| a[1].total_cmp(&b[1])).expect("a top band");
    let width = thickness[0] as i64;
    // The corners belong to the side bands too: only the columns strictly between them count.
    let (left, right) = (top[0].round() as i64, top[2].round() as i64 - width);
    let starts = |row: f32| -> BTreeSet<i64> {
        quads
            .iter()
            .filter(|(quad, color)| *color != light && quad[1].round() == row && quad[3] - quad[1] < 1.5)
            .map(|(quad, _)| quad[0].round() as i64)
            .filter(|s| *s > left && *s < right)
            .collect()
    };
    let (upper, lower) = (starts(top[1].round()), starts(top[1].round() + 1.0));
    assert!(!upper.is_empty(), "the top band has dashes");
    for s in upper.iter().filter(|s| **s - width > left) {
        assert!(lower.contains(&(s - width)), "the dash at {s} continues {width} pixels earlier one row down: {lower:?}");
    }
}

/// A docked scroll bar (the canvas's) has the same thumb across the track at rest and under the pointer, on a
/// `surface.chrome` track.
#[trial]
fn a_docked_scroll_bar_keeps_its_width_at_rest_and_under_the_pointer() {
    use craft_behavior::{Axis, ScrollSpan};
    use craft_theme::{Contrast, ControlSize, Mode, Variant};
    use craft_ui::primitives::ScrollBar;
    use craft_ui::style::{Flags, ScrollBarKind, State, StateMix, StateSet};
    use egui::{Rect, Shape};

    let tokens = craft_ui::testing::style::tokens(Mode::Dark, Contrast::Standard);
    let look = craft_ui::testing::style::with_cx(&egui::Context::default(), &tokens, Variant::SECONDARY, ControlSize::Md, |cx| {
        cx.scroll_bar_of(ScrollBarKind::Docked)
    });
    let span = ScrollSpan::new(0.0..400.0, 100.0..300.0);
    for (axis, rtl) in [(Axis::Y, false), (Axis::Y, true), (Axis::X, false)] {
        let track = match axis {
            Axis::X => Rect::from_min_max(pos2(0.0, 100.0), pos2(200.0, 100.0 + look.thickness)),
            _ => Rect::from_min_max(pos2(100.0, 0.0), pos2(100.0 + look.thickness, 200.0)),
        };
        let thumb = |state: State, mix: StateMix| {
            let shapes = ScrollBar::shapes(&look, track, axis, rtl, &span, &StateSet::new(state, Flags::empty(), mix));
            match shapes.last() {
                Some(Shape::Rect(shape)) => shape.rect,
                other => panic!("the thumb is a rect: {other:?}"),
            }
        };
        let (rest, hover) = (thumb(State::Rest, StateMix::REST), thumb(State::Hover, StateMix::new(1.0, 0.0, 0.0)));
        assert_eq!(rest, hover, "{axis:?} rtl {rtl}: the thumb is the same at rest and under the pointer");
        let across = |rect: Rect| if axis == Axis::X { rect.height() } else { rect.width() };
        assert!((across(rest) - across(track)).abs() < EPS, "{axis:?}: the thumb fills the track");
        let fill = |state: State, mix: StateMix| match ScrollBar::shapes(&look, track, axis, rtl, &span, &StateSet::new(state, Flags::empty(), mix))
            .as_slice()
        {
            [Shape::Rect(track), Shape::Rect(thumb)] => (track.fill, thumb.fill),
            other => panic!("a track and a thumb: {other:?}"),
        };
        let (track_fill, thumb_fill) = fill(State::Rest, StateMix::REST);
        let (_, hover_fill) = fill(State::Hover, StateMix::new(1.0, 0.0, 0.0));
        assert_ne!(thumb_fill, track_fill, "{axis:?}: the resting thumb shows on its track");
        assert_ne!(thumb_fill, hover_fill, "{axis:?}: the thumb takes a stronger colour under the pointer");
    }
}

/// Every scroll bar rests as a thin line along its track's outer edge and shows its thumb across the whole track under
/// the pointer, at the same place along the bar.
#[trial]
fn a_resting_scroll_bar_is_a_thin_line_at_its_outer_edge_that_widens_under_the_pointer() {
    use craft_behavior::{Axis, ScrollSpan};
    use craft_theme::{Contrast, ControlSize, Mode, Variant};
    use craft_ui::primitives::ScrollBar;
    use craft_ui::style::{Flags, State, StateMix, StateSet};
    use egui::{Rect, Shape};

    let tokens = craft_ui::testing::style::tokens(Mode::Dark, Contrast::Standard);
    let look = craft_ui::testing::style::with_cx(&egui::Context::default(), &tokens, Variant::SECONDARY, ControlSize::Md, |cx| cx.scroll_bar());
    let span = ScrollSpan::new(0.0..400.0, 100.0..300.0);
    for (axis, rtl) in [(Axis::Y, false), (Axis::Y, true), (Axis::X, false)] {
        let track = match axis {
            Axis::X => Rect::from_min_max(pos2(0.0, 100.0), pos2(200.0, 100.0 + look.thickness)),
            _ => Rect::from_min_max(pos2(100.0, 0.0), pos2(100.0 + look.thickness, 200.0)),
        };
        let thumb = |state: State, mix: StateMix| {
            let shapes = ScrollBar::shapes(&look, track, axis, rtl, &span, &StateSet::new(state, Flags::empty(), mix));
            match shapes.last() {
                Some(Shape::Rect(shape)) => shape.rect,
                other => panic!("the thumb is a rect: {other:?}"),
            }
        };
        let (rest, hover) = (thumb(State::Rest, StateMix::REST), thumb(State::Hover, StateMix::new(1.0, 0.0, 0.0)));
        let across = |rect: Rect| if axis == Axis::X { rect.height() } else { rect.width() };
        assert!(across(rest) < across(hover), "{axis:?}: the resting thumb is thinner: {rest:?} and {hover:?}");
        assert!((across(hover) - across(track)).abs() < EPS, "{axis:?}: under the pointer the thumb fills the track");
        let outer = match (axis, rtl) {
            (Axis::X, _) => (rest.bottom() - track.bottom()).abs(),
            (_, true) => (rest.left() - track.left()).abs(),
            _ => (rest.right() - track.right()).abs(),
        };
        assert!(outer < EPS, "{axis:?} rtl {rtl}: the resting line lies along the outer edge: {rest:?} in {track:?}");
        let along = |rect: Rect| if axis == Axis::X { rect.x_range() } else { rect.y_range() };
        assert_eq!(along(rest), along(hover), "{axis:?}: the thumb keeps its place along the bar");
    }
}
