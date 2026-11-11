//! What the two ends of the bar take: the caption groups of an integrated frame and the room the macOS traffic
//! lights reserve. The groups are laid out once by [`CaptionPlan`], whose slots run flush with the window's edges.

use crate::shell::{CaptionFamily, CaptionPlan, CaptionSlot, Frame};
use craft_behavior::Arrangement;
use egui::{Pos2, Rect, Vec2, pos2, vec2};

/// One end of the bar: the room reserved before its buttons, then the buttons in reading order.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct End {
    /// The room the traffic lights take, ahead of the buttons.
    pub(super) inset: f32,
    /// The buttons' slots in reading order, each as wide as its hit area.
    pub(super) captions: Vec<CaptionSlot>,
}

impl End {
    fn inset(inset: f32) -> End {
        End { inset, captions: Vec::new() }
    }
}

/// Both ends of a bar, by reading direction: the start end is at the left edge in a left-to-right window.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Ends {
    pub(super) family: CaptionFamily,
    pub(super) start: End,
    pub(super) end: End,
}

impl Ends {
    fn bare() -> Ends {
        Ends { family: CaptionFamily::Generic, start: End::default(), end: End::default() }
    }

    /// The ends of a bar in `frame`. A window in full screen (`shown` false) has none. The macOS lights and the
    /// Windows captions stay where the platform puts them: the lights on the physical left, the Windows captions
    /// mirrored, the Linux layouts as the user set them (see [`CaptionPlan::new`]).
    pub(super) fn of(frame: &Frame, shown: bool, rtl: bool) -> Ends {
        if !shown {
            return Ends::bare();
        }
        match frame {
            Frame::Inset(inset) if rtl => Ends { end: End::inset(inset.leading), ..Ends::bare() },
            Frame::Inset(inset) => Ends { start: End::inset(inset.leading), ..Ends::bare() },
            Frame::Integrated(captions) => {
                let plan = CaptionPlan::new(captions, rtl);
                let total = plan.left_width() + plan.right_width();
                let slots = plan.place(Rect::from_min_max(Pos2::ZERO, pos2(total, 1.0)), 0.0, total);
                let (left, right) = slots.split_at(plan.left().len());
                // The group at the physical right edge starts the line in a right-to-left window and reads leftward.
                let (mut start, mut end) = if rtl { (right.to_vec(), left.to_vec()) } else { (left.to_vec(), right.to_vec()) };
                if rtl {
                    start.reverse();
                    end.reverse();
                }
                Ends { family: captions.family, start: End { inset: 0.0, captions: start }, end: End { inset: 0.0, captions: end } }
            }
            _ => Ends::bare(),
        }
    }
}

/// Places one end's buttons side by side from the start edge of their room, each as wide as its hit area and as
/// high as the bar. The room is the one the end's region gives, so the end takes exactly its width.
pub(super) struct Strip {
    inset: f32,
    widths: Vec<f32>,
    height: f32,
}

impl Strip {
    pub(super) fn new(end: &End, height: f32) -> Strip {
        Strip { inset: end.inset, widths: end.captions.iter().map(|slot| slot.rect.width()).collect(), height }
    }
}

impl Arrangement for Strip {
    fn place(&self, room: Rect, _: &[Vec2], out: &mut Vec<Rect>) -> Vec2 {
        let mut x = self.inset;
        for width in &self.widths {
            out.push(Rect::from_min_size(room.min + vec2(x, 0.0), vec2(*width, self.height)));
            x += width;
        }
        vec2(x, self.height)
    }
}
