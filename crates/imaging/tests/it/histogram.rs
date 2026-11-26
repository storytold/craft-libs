//! The histogram: how tall the bins are drawn, how overlapping channels combine, the selection band and what the
//! plot reports for the pointer, the keys and the stale mark.

use craft_imaging::ui::histogram::paint::{rgb_heights, rgb_mesh, tone_heights};
use craft_imaging::ui::histogram::{Blend, HistogramScale};
use craft_ui::testing::trial;
use egui::{Color32, Mesh, Pos2, Rect, Vec2, pos2};

const PLOT: Vec2 = Vec2::new(256.0, 100.0);
const TOLERANCE: f32 = 1e-5;

fn rect() -> Rect {
    Rect::from_min_size(pos2(10.0, 20.0), PLOT)
}

fn flat(n: usize, v: f32) -> Vec<f32> {
    vec![v; n]
}

/// The colour of the mesh's triangle containing `point`.
fn color_at(mesh: &Mesh, point: Pos2) -> Option<Color32> {
    mesh.indices.as_chunks::<3>().0.iter().find_map(|t| {
        let [a, b, c] = [0, 1, 2].map(|i| mesh.vertices[t[i] as usize].pos);
        let cross = |u: Vec2, v: Vec2| u.x * v.y - u.y * v.x;
        if cross(b - a, c - a).abs() < 1e-5 {
            return None;
        }
        let sides = [cross(b - a, point - a), cross(c - b, point - b), cross(a - c, point - c)];
        (sides.iter().all(|&s| s >= -1e-5) || sides.iter().all(|&s| s <= 1e-5)).then_some(mesh.vertices[t[0] as usize].color)
    })
}

#[trial]
fn the_typical_scale_runs_spikes_off_the_top() {
    let mut bins = flat(256, 100.0);
    bins[128] = 100_000.0;
    let heights = tone_heights(&bins, HistogramScale::Typical);
    assert!((heights[50] - 100.0 / 150.0).abs() < TOLERANCE, "{}", heights[50]);
    assert_eq!(heights[128], 1.0);
    assert!(tone_heights(&bins, HistogramScale::Peak)[50] < 0.01, "Peak keeps the spike on the plot");
    let mut channel = flat(256, 100.0);
    channel[100] = 50_000.0;
    let rgb = rgb_heights(&[channel.clone(), channel.clone(), channel], HistogramScale::Typical);
    assert!((rgb[0][30] - 2.0 / 3.0).abs() < 0.01 && rgb[0][100] == 1.0, "{}", rgb[0][30]);
}

#[trial]
fn crossing_channels_keep_the_overlap_colour_under_both() {
    let rect = rect();
    let colors = [Color32::from_rgb(255, 0, 0), Color32::from_rgb(0, 255, 0), Color32::from_rgb(0, 0, 255)];
    let mesh = rgb_mesh(rect, &[vec![0.9, 0.9, 0.1, 0.1], vec![0.1, 0.1, 0.9, 0.9], flat(4, 0.0)], &colors, Blend::Add);
    for x in [0.1, 0.3, 0.5, 0.7, 0.9] {
        let point = pos2(rect.left() + x * rect.width(), rect.bottom() - 0.05 * rect.height());
        assert_eq!(color_at(&mesh, point), Some(Color32::from_rgb(255, 255, 0)), "x = {x}");
    }
}
