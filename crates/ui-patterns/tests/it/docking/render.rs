//! Docking render maths: divider travel with the shrink chain.

use craft_behavior::Axis;
use craft_behavior::dock::{Node, SplitSize};
use craft_ui::contract::SizeLimits;
use craft_ui::docking::DockSizes;
use craft_ui::docking::tree::{self, FrameRules};
use craft_ui::testing::trial;
use egui::{Pos2, Rect, Vec2};

const WIDTH: f32 = 400.0;
const HEIGHT: f32 = 300.0;
const LEAST_WIDTH: f32 = 60.0;
const HEADER: f32 = 20.0;
const EPS: f32 = 0.01;

const A: usize = 0;
const B: usize = 1;
const C: usize = 2;

fn sizes() -> DockSizes {
    super::sizes(0.0, 0.0, HEADER)
}

fn area(width: f32) -> Rect {
    Rect::from_min_size(Pos2::ZERO, Vec2::new(width, HEIGHT))
}

fn limits(_: &usize) -> SizeLimits {
    SizeLimits::at_least(Vec2::new(LEAST_WIDTH, 0.0))
}

fn widths(root: &Node<usize>, rect: Rect, rules: &FrameRules<'_, usize>) -> Vec<f32> {
    tree::frames(root, rect, &sizes(), rules, false).groups.iter().map(|group| group.rect.width()).collect()
}

/// `A | (B | C)`: three groups side by side, the first divider ahead of a nested split.
fn three_columns() -> Node<usize> {
    let rest = Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![B], 0), Node::tabs(vec![C], 0));
    Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![A], 0), rest)
}

#[trial]
fn a_divider_shrinks_the_groups_beyond_it_in_turn_down_to_their_minimum() {
    let rect = area(WIDTH);
    let rules = FrameRules::new(&limits);
    let mut root = three_columns();
    let first_divider = tree::frames(&root, rect, &sizes(), &rules, false)
        .handles
        .into_iter()
        .find(|handle| handle.path.is_empty())
        .expect("the root split has a handle");
    let sizes = tree::drag_sizes(&root, rect, &sizes(), &rules, &first_divider.path, WIDTH);
    tree::apply_sizes(&mut root, &sizes);

    let after = widths(&root, rect, &rules);
    assert!(
        (after[1] - LEAST_WIDTH).abs() < EPS && (after[2] - LEAST_WIDTH).abs() < EPS,
        "both groups beyond the divider are at their minimum: {after:?}"
    );
    assert!((after.iter().sum::<f32>() - WIDTH).abs() < EPS, "the room they gave went to the group before the divider: {after:?}");
}
