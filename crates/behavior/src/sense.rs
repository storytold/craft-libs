//! The library's sense set, for a root or a part.

use bitflags::bitflags;

bitflags! {
    /// What a root or a part senses. The egui adapter maps the root's sense to egui's.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct Sense: u16 {
        /// Hover only.
        const HOVER = 1;
        /// A press released within slop activates.
        const CLICK = 2;
        /// A press that moves past slop drags.
        const DRAG = 4;
        /// A Tab stop.
        const FOCUS = 8;
        /// The whole wheel: horizontal and vertical scroll and zoom go here.
        const WHEEL = 16;
        /// A press on this shape leaves key focus and any draft where they are.
        const KEEP_FOCUS = 32;
        /// A press on this shape moves key focus to its component, and to the part when the part is focusable.
        const TAKE_FOCUS = 64;
        /// The horizontal wheel share, taken without focus or arming; the split rule passes the vertical share on.
        const WHEEL_X = 128;
        /// A middle press that ends within slop is a click the component reads from its press; it never activates.
        /// Without this bit a middle press only captures for a drag.
        const MIDDLE_CLICK = 256;
        /// The vertical wheel share, taken without the other two; the split rule passes the horizontal share and zoom on.
        const WHEEL_Y = 512;
        /// The zoom share (Ctrl or Cmd with the wheel, pinch), taken without scroll; the split rule passes both scroll shares on.
        const WHEEL_ZOOM = 1024;
        /// A secondary press captures the part and its release is reported to it, so a part that senses only a click
        /// still gets its context click. Without this bit a secondary press only captures for a drag.
        const CONTEXT_CLICK = 2048;
        /// A container root learns of a primary press on its own surface: inside its rect, on its layer, where no
        /// shape takes the press. The root takes no press and no focus by it, so it is no click target.
        const SURFACE = 4096;
        /// A part of an item of a collection: an unmodified primary click on it also picks the item, as a click on the
        /// item's own area does. The part reports its own click as well.
        const PICKS_ITEM = 8192;
    }
}

impl Sense {
    /// The bits that make a shape a press candidate.
    pub const PRESS: Sense = Sense::CLICK.union(Sense::DRAG);

    /// Whether a press can start on a shape with this sense.
    pub const fn takes_press(self) -> bool {
        self.intersects(Sense::PRESS)
    }
}
