//! Windows that each cover one monitor, to show a capture of that monitor over it (a screen colour pick): where such
//! a window goes, and which physical desktop pixel a point in it shows.
//!
//! The window itself opens as a borderless, always-on-top window that is shown after its first paint, through the
//! window host of the UI crate; this module has the geometry, which needs no window.

/// One monitor's area in physical pixels of the virtual desktop, with its physical pixels per point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonitorArea {
    /// The top-left pixel in the virtual desktop.
    pub origin: [i32; 2],
    /// The width and height in pixels.
    pub size: [u32; 2],
    /// Physical pixels per point of the monitor.
    pub scale: f32,
}

impl MonitorArea {
    /// The monitor at `origin` of `size` pixels, at `scale` pixels per point.
    pub const fn new(origin: [i32; 2], size: [u32; 2], scale: f32) -> Self {
        MonitorArea { origin, size, scale }
    }

    /// The outer rect of a borderless window that covers the monitor exactly, in points of the monitor's own scale
    /// (`[x, y, width, height]`): the space a window on that monitor reports its rects in.
    pub fn cover(&self) -> [f32; 4] {
        let scale = if self.scale.is_finite() && self.scale > 0.0 { self.scale } else { 1.0 };
        [self.origin[0] as f32 / scale, self.origin[1] as f32 / scale, self.size[0] as f32 / scale, self.size[1] as f32 / scale]
    }

    /// The physical desktop pixel shown at `at`, a point in a view of `shown` points (width, height) that shows the
    /// whole monitor stretched over it. `None` outside the view.
    ///
    /// The view's own size gives the ratio, not the monitor's scale, so the pixel is right wherever the window
    /// system put the window and whatever scale it gave it.
    pub fn pixel_at(&self, shown: [f32; 2], at: [f32; 2]) -> Option<[i32; 2]> {
        if !(shown[0] > 0.0 && shown[1] > 0.0) || !(0.0..shown[0]).contains(&at[0]) || !(0.0..shown[1]).contains(&at[1]) {
            return None;
        }
        let axis = |n: usize| {
            let pixel = (at[n] / shown[n] * self.size[n] as f32).floor() as i64;
            self.origin[n] as i64 + pixel.clamp(0, i64::from(self.size[n]).saturating_sub(1))
        };
        Some([axis(0), axis(1)].map(|v| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32))
    }
}

#[cfg(test)]
mod tests {
    use super::MonitorArea;

    /// A monitor at scale 2 right of one at scale 1 is covered in its own points, and a point in a view of any size
    /// lands on the same pixel: guards a pick that reads the pixel at the right coordinates on the wrong monitor, or
    /// at half or double the distance on a HiDPI one.
    #[test]
    fn a_point_in_the_cover_is_the_pixel_of_its_monitor_at_any_scale() {
        let right = MonitorArea::new([1920, 0], [2880, 1800], 2.0);
        assert_eq!(right.cover(), [960.0, 0.0, 1440.0, 900.0]);
        let middle = [1920 + 1440, 900];
        assert_eq!(right.pixel_at([1440.0, 900.0], [720.0, 450.0]), Some(middle), "a view in the monitor's points");
        assert_eq!(right.pixel_at([2880.0, 1800.0], [1440.0, 900.0]), Some(middle), "a view in its pixels");
        assert_eq!(right.pixel_at([1440.0, 900.0], [1440.0, 10.0]), None, "past the right edge");
    }
}
