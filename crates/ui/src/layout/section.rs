//! Group related settings without taking ownership of a dialog or its contents.

use egui::{Frame, InnerResponse, Label, Stroke, Ui, WidgetText};

const MAX_WIDTH: f32 = 1_000_000.0;
const MAX_DECORATION: f32 = 64.0;

/// A caller-styled frame with an optional heading above its contents.
///
/// This extracts the grouping boundary of WordCraft's dialog sections (#4038), not its theme,
/// fonts, indentation or window policy. Pass localized [`WidgetText`] (including [`egui::RichText`]
/// for emphasis) and an application-supplied [`Frame`]. The contents retain ordinary egui focus,
/// clipping, disabled-state and widget-ID behavior. Use `Ui::push_id` around repeated sections
/// when the application needs identity independent of their position.
///
/// Reusing one frame for nested sections keeps their fill consistent: this component neither
/// adds another tint nor writes temporary global style or nesting state.
pub struct Section {
    frame: Frame,
    heading: Option<WidgetText>,
    heading_gap: f32,
    fill_width: bool,
}

impl Section {
    pub fn new(frame: Frame) -> Self {
        Self { frame, heading: None, heading_gap: 2.0, fill_width: true }
    }

    /// Put a heading above the contents. An empty heading takes no row or gap.
    ///
    /// Typography is caller-owned, for example `RichText::new(title).strong()`.
    pub fn heading(mut self, heading: impl Into<WidgetText>) -> Self {
        let heading = heading.into();
        self.heading = (!heading.text().is_empty()).then_some(heading);
        self
    }

    /// Space after a nonempty heading, in logical points. Invalid or negative values become zero;
    /// excessive values are capped at 64 points. This does not add space outside the frame.
    pub fn heading_gap(mut self, points: f32) -> Self {
        self.heading_gap = decoration(points);
        self
    }

    /// Fill the available width (the default), or wrap the contents when false.
    /// Unbounded parent widths do not force an unbounded minimum width.
    pub fn fill_width(mut self, fill: bool) -> Self {
        self.fill_width = fill;
        self
    }

    /// Lay out the heading and caller's controls, returning the caller's value and frame response.
    ///
    /// The callback owns its state and actions; the section does not dispatch commands, create
    /// a modal window, indent controls or change global visuals. egui clamps narrow frame content
    /// rectangles. This component additionally bounds forced widths and sanitizes the frame's
    /// floating-point stroke width (invalid values become zero, maximum 64 points).
    pub fn show<R>(self, ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
        let mut frame = self.frame;
        let width = decoration(frame.stroke.width);
        frame.stroke = if width == 0.0 { Stroke::NONE } else { Stroke::new(width, frame.stroke.color) };
        frame.show(ui, |ui| {
            // A frame inherits its parent's layout. Keep the section's heading above its body
            // even when the caller places several sections side by side in a horizontal row.
            ui.vertical(|ui| {
                if self.fill_width {
                    let width = ui.available_width();
                    if width.is_finite() && width > 0.0 {
                        ui.set_width(width.min(MAX_WIDTH));
                    }
                }
                if let Some(heading) = self.heading {
                    ui.add(Label::new(heading));
                    ui.add_space(self.heading_gap);
                }
                add_contents(ui)
            })
            .inner
        })
    }
}

fn decoration(value: f32) -> f32 {
    if value.is_finite() { value.clamp(0.0, MAX_DECORATION) } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Color32, Context, CornerRadius, Event, Margin, Modifiers, PointerButton, RawInput, RichText, Vec2};
    use egui_kittest::kittest::Queryable;

    fn style() -> Frame {
        Frame::new()
            .fill(Color32::from_gray(30))
            .stroke(Stroke::new(1.0, Color32::GRAY))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::same(8))
    }

    #[test]
    fn heading_stays_above_controls_in_horizontal_parents() {
        let mut h = egui_kittest::Harness::builder().with_size(Vec2::new(500.0, 300.0)).build_ui(|ui| {
            ui.horizontal(|ui| {
                Section::new(style()).heading("Display").fill_width(false).show(ui, |ui| {
                    let _ = ui.button("Apply");
                });
                ui.label("Neighbour");
            });
        });
        h.run_steps(2);
        assert!(h.get_by_label("Apply").rect().top() >= h.get_by_label("Display").rect().bottom());
    }

    #[test]
    fn heading_and_content_are_accessible_and_return_the_callers_value() {
        let mut h = egui_kittest::Harness::builder().with_size(Vec2::new(500.0, 300.0)).build_ui_state(
            |ui, clicked: &mut bool| {
                let result = Section::new(style()).heading(RichText::new("Display").strong()).show(ui, |ui| {
                    if ui.button("Apply").clicked() {
                        *clicked = true;
                    }
                    42
                });
                assert_eq!(result.inner, 42);
            },
            false,
        );
        h.run_steps(2);
        h.get_by_label("Display");
        h.get_by_label("Apply").click();
        h.run_steps(2);
        assert!(*h.state());
    }

    #[test]
    fn parent_disabled_state_prevents_content_activation() {
        let mut h = egui_kittest::Harness::builder().with_size(Vec2::new(500.0, 300.0)).build_ui_state(
            |ui, state: &mut (bool, usize)| {
                ui.add_enabled_ui(state.0, |ui| {
                    Section::new(style()).heading("Display").show(ui, |ui| {
                        if ui.button("Apply").clicked() {
                            state.1 += 1;
                        }
                    });
                });
            },
            (true, 0),
        );
        h.run_steps(2);
        let pos = h.get_by_label("Apply").rect().center();
        h.state_mut().0 = false;
        h.run_steps(2);
        h.event(Event::PointerMoved(pos));
        h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.step();
        h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
        assert_eq!(h.state().1, 0);
    }

    #[test]
    fn nested_sections_do_not_change_theme_and_invalid_decoration_stays_finite() {
        let ctx = Context::default();
        let original = ctx.global_style().visuals.clone();
        for value in [f32::NAN, f32::INFINITY, -1.0, f32::MAX] {
            let mut frame = style();
            frame.stroke.width = value;
            let mut out = ctx.run_ui(RawInput::default(), |ui| {
                Section::new(frame).heading("").heading_gap(value).show(ui, |ui| {
                    let response = Section::new(frame).heading("Nested").heading_gap(value).show(ui, |ui| {
                        ui.label("Contents");
                    });
                    assert!(response.response.rect.is_finite());
                });
            });
            out.textures_delta.clear();
            let _ = ctx.tessellate(out.shapes, out.pixels_per_point);
            assert_eq!(ctx.global_style().visuals.panel_fill, original.panel_fill);
            assert_eq!(ctx.global_style().visuals.window_fill, original.window_fill);
            assert_eq!(ctx.global_style().visuals.window_stroke, original.window_stroke);
        }
    }
}
