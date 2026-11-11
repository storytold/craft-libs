//! The section layout of an accordion (`Node::Stack`): every section shows its title row, and only open sections
//! take height for their body. `StackSections` hosts the sections and the dividers between the open ones.

use super::divider::{self, Divider, Drag};
use super::{DockSizes, DockWords, Splitter};
use crate::contract::{
    Access, Common, Component, Container, ContainerProps, Cx, NoValue, Outcome, Pad, PaintUi, PartsCx, Role, Sizing, WordSetCx as _,
};
use crate::runtime::focus::KeyUse;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::divider::{self as push, Segment};
use craft_behavior::{Axis, Metrics, PartKey, Sense};
use egui::{Rect, Response};

const DIVIDER: PartKey = PartKey::new("divider");

/// One section of an accordion as the maths sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct StackSection {
    /// The section shows its body.
    pub open: bool,
    /// The preferred body height; `None` shares the free height.
    pub height: Option<f32>,
    /// The least body height of the section's panel.
    pub min: f32,
    /// The most body height of the section's panel.
    pub max: f32,
}

impl StackSection {
    /// A section whose body takes any height.
    pub const fn new(open: bool) -> Self {
        StackSection { open, height: None, min: 0.0, max: f32::INFINITY }
    }

    /// The section with a preferred body height.
    #[must_use]
    pub const fn height(mut self, height: Option<f32>) -> Self {
        self.height = height;
        self
    }

    /// The section with the body limits of its panel.
    #[must_use]
    pub const fn limits(mut self, min: f32, max: f32) -> Self {
        self.min = min;
        self.max = max;
        self
    }
}

/// The height of each section in `total` points, title row included: a closed section is `header` high, an open one
/// `header` and its body. A body keeps its preferred height within its limits; sections without one share what the
/// others leave. When the preferred heights do not fit, the last open sections give way first, down to their minimum.
pub fn section_heights(sections: &[StackSection], total: f32, header: f32) -> Vec<f32> {
    let room = (total - header * sections.len() as f32).max(0.0);
    let mut bodies: Vec<f32> = sections
        .iter()
        .map(|section| if section.open { section.height.map_or(0.0, |height| height.clamp(section.min, section.max.max(section.min))) } else { 0.0 })
        .collect();
    let free: Vec<usize> = (0..sections.len()).filter(|&at| sections[at].open && sections[at].height.is_none()).collect();
    let fixed: f32 = bodies.iter().sum();
    if !free.is_empty() {
        let mut leftover = (room - fixed).max(0.0);
        let mut pending = free;
        // A section held by its limit leaves its share to the others: settle the held ones, then share what remains.
        while !pending.is_empty() {
            let share = leftover / pending.len() as f32;
            let held: Vec<usize> =
                pending.iter().copied().filter(|&at| share < sections[at].min || share > sections[at].max.max(sections[at].min)).collect();
            if held.is_empty() {
                pending.iter().for_each(|&at| bodies[at] = share);
                break;
            }
            for at in held {
                let section = sections[at];
                bodies[at] = share.clamp(section.min, section.max.max(section.min));
                leftover = (leftover - bodies[at]).max(0.0);
                pending.retain(|&other| other != at);
            }
        }
    }
    let mut excess = bodies.iter().sum::<f32>() - room;
    for at in (0..sections.len()).rev() {
        if excess <= 0.0 {
            break;
        }
        let taken = excess.min((bodies[at] - sections[at].min).max(0.0));
        bodies[at] -= taken;
        excess -= taken;
    }
    sections.iter().zip(bodies).map(|(section, body)| header + if section.open { body } else { 0.0 }).collect()
}

/// The style of the stack dividers: the docking sizes of their reach, keys and sections.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct StackSectionsStyle {
    #[style(metrics)]
    metrics: Metrics,
    sizes: DockSizes,
}

/// What the stack dividers report.
#[derive(Debug)]
#[non_exhaustive]
pub struct StackSectionsOutput {
    /// The dividers' response.
    pub response: Response,
    /// The sections whose body height the user changed this pass, each as its index among the sections with the new
    /// height.
    pub resized: Vec<(usize, f32)>,
}

impl Outcome for StackSectionsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// An accordion laid out in the rect it is shown at (`section_heights` with the strip height of the docking sizes),
/// with a divider between its open sections. The body shows the sections at the rects it computes. A drag or an arrow
/// key on a divider moves the boundary between its two sections and the sections beyond give way in turn; the changed
/// body heights come back in `resized`.
#[derive(crate::contract::Component)]
#[component(container)]
pub struct StackSections {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    sections: Vec<StackSection>,
    resized: Vec<(usize, f32)>,
}

/// Where the dividers of a stack sit.
struct Plan {
    /// The indices of the open sections.
    open: Vec<usize>,
    /// The open sections as a row of segments, for the push rule.
    row: Vec<Segment>,
    /// The divider after each open section but the last: its part index, its position among the open sections and
    /// the divider.
    dividers: Vec<(u64, usize, Divider)>,
}

impl StackSections {
    /// An accordion of `sections`, in order.
    pub fn new(sections: Vec<StackSection>) -> Self {
        let props = ContainerProps::new().sizing(Sizing::Fill).pad(Pad::None);
        StackSections { common: Common::new(), props, sections, resized: Vec::new() }
    }

    /// The dividers of the sections laid out in `rect`.
    fn plan(&self, rect: Rect, sizes: &DockSizes) -> Plan {
        let heights = section_heights(&self.sections, rect.height(), sizes.strip);
        let open: Vec<usize> = (0..self.sections.len()).filter(|at| self.sections[*at].open).collect();
        let row: Vec<Segment> = open.iter().map(|at| Segment::unbounded(heights[*at], sizes.strip + self.sections[*at].min)).collect();
        let dividers = open
            .windows(2)
            .enumerate()
            .map(|(index, pair)| {
                let at = rect.top() + heights.iter().take(pair[0] + 1).sum::<f32>();
                let (lo, hi) = push::travel(&row, index);
                (pair[0] as u64, index, Divider::new(Axis::Y, at, rect, (at + lo, at + hi), sizes.reach))
            })
            .collect();
        Plan { open, row, dividers }
    }

    /// Moves the divider after the `index`-th open section by `delta` points.
    fn move_divider(&mut self, open: &[usize], row: &[Segment], index: usize, delta: f32, strip: f32) {
        let at: f32 = row.iter().take(index + 1).map(|segment| segment.extent).sum();
        for ((section, next), segment) in open.iter().zip(push::resize(row, index, at + delta)).zip(row) {
            if next != segment.extent {
                self.resized.push((*section, next - strip));
            }
        }
    }
}

impl Component for StackSections {
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::SPLITTER;
    type Style = StackSectionsStyle;
    type Output = StackSectionsOutput;

    fn style(cx: &StyleCx<'_>, _: StackSectionsStyleKey) -> StackSectionsStyle {
        StackSectionsStyle { metrics: Metrics::new(), sizes: DockSizes::style(cx) }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let sizes = cx.style.sizes;
        for (key, _, divider) in self.plan(rect, &sizes).dividers {
            divider::declare(cx, rect, DIVIDER, key, &divider);
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let sizes = cx.style.sizes;
        let Plan { open, row, dividers } = self.plan(cx.rect, &sizes);
        for (key, index, divider) in &dividers {
            if let Some(Drag::To(coord)) = divider::pointer(cx, DIVIDER, *key, divider) {
                self.move_divider(&open, &row, *index, coord - divider.at, sizes.strip);
            }
        }
        let keyed: Vec<(u64, Divider)> = dividers.iter().map(|(key, _, divider)| (*key, *divider)).collect();
        if let Some((key, coord)) = divider::keys(cx, DIVIDER, &keyed, sizes.step, sizes.jump)
            && let Some((_, index, divider)) = dividers.iter().find(|(at, ..)| *at == key)
        {
            self.move_divider(&open, &row, *index, coord - divider.at, sizes.strip);
        }
    }

    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        for (key, ..) in self.plan(cx.rect, &cx.style.sizes).dividers {
            ui.face(DIVIDER.at(key), Splitter::new(Axis::Y));
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let sizes = cx.style.sizes;
        let words = cx.words::<DockWords>();
        for (key, _, divider) in self.plan(cx.rect, &sizes).dividers {
            divider::describe(access, DIVIDER, key, &divider, words.resize.as_str(), sizes.step);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> StackSectionsOutput {
        StackSectionsOutput { response: cx.response.clone(), resized: self.resized }
    }
}

impl Container for StackSections {}
