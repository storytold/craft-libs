//! Buttons and toggles: a dialog's button row in the platform's order, checkboxes and iOS-style
//! toggle switches. The row follows the reading direction ([`direction::row`]), so the same add
//! order paints the platform's mirrored order in a right-to-left interface, and Tab still walks
//! the buttons in creation order — the add order of focusable children is never reversed. A
//! toggle's knob travels from the row's reading start to its reading end as it turns on.

use egui::{Color32, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use crate::direction::{self, HasDirection, is_rtl};

/// What a dialog button does, which decides where the platform's button order puts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonRole {
    /// The default answer (OK, Save, Yes), drawn as the primary button.
    Default,
    /// Another answer that closes the dialog (Don't Save, No).
    Alternate,
    /// Closes the dialog without acting.
    Cancel,
    /// Acts but keeps the dialog open (Apply).
    Apply,
}

impl ButtonRole {
    /// Position from the left in the platform's order. Windows and Linux put the default action
    /// first (OK Cancel Apply, Yes No Cancel); macOS puts it last, in the corner, with Cancel
    /// beside it and the other answers further left (Don't Save, Cancel, Save). The row itself
    /// mirrors with the reading direction (see [`dialog_buttons`]).
    pub fn slot(self, mac: bool) -> u8 {
        match (self, mac) {
            (Self::Default, false) | (Self::Alternate, true) => 0,
            (Self::Alternate, false) | (Self::Cancel, true) => 1,
            (Self::Cancel, false) | (Self::Apply, true) => 2,
            (Self::Apply, false) | (Self::Default, true) => 3,
        }
    }
}

/// One button of a [`dialog_buttons`] row.
#[derive(Clone, Copy)]
pub struct DialogButton<'a> {
    pub role: ButtonRole,
    pub label: &'a str,
    pub min_width: f32,
    pub enabled: bool,
    /// Take keyboard focus this frame (see [`DialogButton::focus`]).
    pub focus: bool,
}

impl<'a> DialogButton<'a> {
    pub fn new(role: ButtonRole, label: &'a str, min_width: f32) -> Self {
        Self { role, label, min_width, enabled: true, focus: false }
    }

    pub fn enabled(self, enabled: bool) -> Self {
        Self { enabled, ..self }
    }

    /// Asks for keyboard focus for this button: from there the arrow keys move between the row's
    /// buttons and Enter or Space presses the focused one.
    pub fn focus(self, focus: bool) -> Self {
        Self { focus, ..self }
    }
}

/// Colors and fonts for [`dialog_buttons`]' primary/secondary buttons; the application owns the
/// theme.
#[derive(Clone, Debug)]
pub struct ButtonStyle {
    pub primary_bg: Color32,
    pub primary_text: Color32,
    pub secondary_bg: Color32,
    pub secondary_text: Color32,
    pub hover: Color32,
    pub focus_ring: Color32,
    pub radius: f32,
    pub height: f32,
    pub font: egui::FontId,
    /// Horizontal padding added to the label's width.
    pub pad_x: f32,
}

impl Default for ButtonStyle {
    fn default() -> Self {
        Self {
            primary_bg: Color32::from_rgb(0x2d, 0x6a, 0xd8),
            primary_text: Color32::WHITE,
            secondary_bg: Color32::from_gray(50),
            secondary_text: Color32::from_gray(220),
            hover: Color32::from_gray(80),
            focus_ring: Color32::from_rgb(0x2d, 0x6a, 0xd8),
            radius: 4.0,
            height: 30.0,
            font: egui::FontId::proportional(13.0),
            pad_x: 28.0,
        }
    }
}

/// The size of a button with `label` under `style`, at least `min_width` wide.
pub fn button_size(ui: &Ui, style: &ButtonStyle, label: &str, min_width: f32) -> Vec2 {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), style.font.clone(), style.secondary_text);
    vec2((galley.size().x + style.pad_x).max(min_width), style.height)
}

fn button_impl(ui: &mut Ui, label: &str, min_width: f32, bg: Color32, fg: Color32, primary: bool, style: &ButtonStyle) -> Response {
    let size = button_size(ui, style, label, min_width);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    if resp.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.0), style.radius + 2.0, Stroke::new(2.0, style.focus_ring), StrokeKind::Outside);
    }
    let fill = if resp.is_pointer_button_down_on() {
        bg.gamma_multiply(0.85)
    } else if resp.hovered() {
        if primary { bg.gamma_multiply(0.93) } else { style.hover }
    } else {
        bg
    };
    ui.painter().rect_filled(rect, style.radius, fill);
    if !primary {
        ui.painter().rect_stroke(rect, style.radius, Stroke::new(1.0, fg), StrokeKind::Inside);
    }
    let galley = ui.painter().layout_no_wrap(label.to_owned(), style.font.clone(), fg);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    resp
}

/// A dialog's button row at the cursor, in the platform's order (see [`ButtonRole`]): every modal
/// draws its buttons through this, so they all agree. The row follows the interface's reading
/// direction ([`direction::row`]): the same add order paints the platform's mirrored order, and
/// Tab still walks the buttons in creation order. Returns the role of the button clicked this
/// frame.
pub fn dialog_buttons(ui: &mut Ui, d: &impl HasDirection, buttons: &[DialogButton], style: &ButtonStyle) -> Option<ButtonRole> {
    let mac = ui.ctx().os() == egui::os::OperatingSystem::Mac;
    let gap = ui.spacing().item_spacing.x;
    let size = buttons.iter().fold(Vec2::ZERO, |acc, b| {
        let s = button_size(ui, style, b.label, b.min_width);
        vec2(acc.x + s.x, acc.y.max(s.y))
    }) + vec2(gap * buttons.len().saturating_sub(1) as f32, 0.0);
    ui.allocate_ui_with_layout(size, direction::row(d), |ui| {
        let mut hit = None;
        for slot in 0..4 {
            for b in buttons.iter().filter(|b| b.role.slot(mac) == slot) {
                let r = ui
                    .add_enabled_ui(b.enabled, |ui| {
                        if b.role == ButtonRole::Default {
                            button_impl(ui, b.label, b.min_width, style.primary_bg, style.primary_text, true, style)
                        } else {
                            button_impl(ui, b.label, b.min_width, style.secondary_bg, style.secondary_text, false, style)
                        }
                    })
                    .inner;
                if b.focus && b.enabled {
                    r.request_focus();
                }
                if r.clicked() {
                    hit = Some(b.role);
                }
            }
        }
        hit
    })
    .inner
}

/// Colors and fonts for [`checkbox`]; the application owns the theme.
#[derive(Clone, Debug)]
pub struct CheckboxStyle {
    pub accent: Color32,
    pub box_fill: Color32,
    pub text: Color32,
    pub outline: Color32,
    pub outline_hover: Color32,
    /// The box's edge length.
    pub box_size: f32,
    pub radius: f32,
    pub font: egui::FontId,
    pub gap: f32,
}

impl Default for CheckboxStyle {
    fn default() -> Self {
        Self {
            accent: Color32::from_rgb(0x2d, 0x6a, 0xd8),
            box_fill: Color32::from_gray(40),
            text: Color32::from_gray(140),
            outline: Color32::from_gray(120),
            outline_hover: Color32::from_gray(200),
            box_size: 14.0,
            radius: 2.0,
            font: egui::FontId::proportional(13.0),
            gap: 6.0,
        }
    }
}

/// The fill of a check box: the accent when `on`, else the field colour. While `pressed`
/// Photoshop shows that a release will act: an unchecked box darkens a little and a checked one
/// turns a little less opaque.
pub fn check_box_fill(on: bool, pressed: bool, style: &CheckboxStyle) -> Color32 {
    match (on, pressed) {
        (true, false) => style.accent,
        (true, true) => style.accent.gamma_multiply(0.75),
        (false, false) => style.box_fill,
        (false, true) => style.box_fill.lerp_to_gamma(Color32::BLACK, 0.18),
    }
}

/// Paint a check box in `rect`: [`check_box_fill`] with a white tick when `on`, else an outline
/// that brightens while `hovered` or `pressed`.
pub fn check_box(p: &egui::Painter, rect: Rect, on: bool, hovered: bool, pressed: bool, style: &CheckboxStyle) {
    p.rect_filled(rect, style.radius, check_box_fill(on, pressed, style));
    if on {
        let a = rect.left_center() + vec2(3.0, 0.5);
        let b = rect.center_bottom() + vec2(-1.0, -3.5);
        let c = rect.right_top() + vec2(-3.0, 3.5);
        let tick = Stroke::new(rect.width() * 0.125, Color32::WHITE);
        p.line_segment([a, b], tick);
        p.line_segment([b, c], tick);
    } else {
        p.rect_stroke(rect, style.radius, Stroke::new(1.5, if hovered || pressed { style.outline_hover } else { style.outline }), StrokeKind::Inside);
    }
}

/// A checkbox with a clickable label, in a row that follows the reading direction
/// ([`direction::row`]): the box leads at the reading start, the label after it. Returns the
/// combined response; when it is clicked `on` has been flipped.
pub fn checkbox(ui: &mut Ui, d: &impl HasDirection, on: &mut bool, label: &str, style: &CheckboxStyle) -> Response {
    let (mut resp, label_resp) = ui
        .allocate_ui_with_layout(vec2(ui.available_size_before_wrap().x, ui.spacing().interact_size.y), direction::row(d), |ui| {
            ui.spacing_mut().item_spacing.x = style.gap;
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(style.box_size), Sense::CLICK);
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            let l = ui
                .add(egui::Label::new(egui::RichText::new(label).font(style.font.clone()).color(style.text)).selectable(false).sense(Sense::click()))
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            // Pressing the label presses the box.
            let pressed = resp.is_pointer_button_down_on() || l.is_pointer_button_down_on();
            check_box(ui.painter(), rect, *on, resp.hovered() || l.hovered(), pressed, style);
            (resp.union(l.clone()), l)
        })
        .inner;
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    // The label is the single named, focusable checkbox; the box is a mouse target.
    label_resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *on, label));
    resp
}

/// Colors and sizes for [`toggle`]; the application owns the theme.
#[derive(Clone, Debug)]
pub struct ToggleStyle {
    pub on: Color32,
    pub off: Color32,
    pub knob: Color32,
    /// The switch's size: width × height.
    pub size: Vec2,
    pub knob_radius: f32,
    pub knob_inset: f32,
    pub text_on: Color32,
    pub text_off: Color32,
    pub font: egui::FontId,
}

impl Default for ToggleStyle {
    fn default() -> Self {
        Self {
            on: Color32::from_rgb(0x2d, 0x6a, 0xd8),
            off: Color32::from_gray(70),
            knob: Color32::WHITE,
            size: vec2(30.0, 17.0),
            knob_radius: 6.5,
            knob_inset: 8.5,
            text_on: Color32::WHITE,
            text_off: Color32::from_gray(140),
            font: egui::FontId::proportional(13.0),
        }
    }
}

/// An iOS-style toggle switch with a clickable label in a reading-direction row. The switch
/// reads off at the row's reading start and on at its reading end: in a right-to-left interface
/// the knob travels left as it turns on. Returns the row's response; when clicked `on` has been
/// flipped.
pub fn toggle(ui: &mut Ui, d: &impl HasDirection, on: &mut bool, label: &str, style: &ToggleStyle) -> Response {
    let mut resp = ui
        .allocate_ui_with_layout(vec2(ui.available_size_before_wrap().x, ui.spacing().interact_size.y), direction::row(d), |ui| {
            let (rect, resp) = ui.allocate_exact_size(style.size, Sense::click());
            let how_on = ui.ctx().animate_bool(resp.id, *on);
            let bg = if *on { style.on } else { style.off };
            ui.painter().rect_filled(rect, style.size.y / 2.0, bg);
            let (off_x, on_x) = if is_rtl(d) {
                (rect.right() - style.knob_inset, rect.left() + style.knob_inset)
            } else {
                (rect.left() + style.knob_inset, rect.right() - style.knob_inset)
            };
            let x = egui::lerp(off_x..=on_x, how_on);
            ui.painter().circle_filled(pos2(x, rect.center().y), style.knob_radius, style.knob);
            ui.label(egui::RichText::new(label).font(style.font.clone()).color(if *on { style.text_on } else { style.text_off }));
            resp
        })
        .inner;
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use egui::{Key, Pos2, RawInput};

    /// Render a `dialog_buttons` row in `dir` and return each button's label rect, sorted by x.
    fn button_rects(dir: Direction) -> Vec<(ButtonRole, Rect)> {
        let ctx = egui::Context::default();
        direction::apply(&ctx, dir);
        let buttons = [
            DialogButton::new(ButtonRole::Default, "Save", 60.0),
            DialogButton::new(ButtonRole::Cancel, "Cancel", 60.0),
            DialogButton::new(ButtonRole::Apply, "Apply", 60.0),
        ];
        let mut r = ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 200.0))), ..Default::default() }, |ui| {
            dialog_buttons(ui, &dir, &buttons, &ButtonStyle::default());
        });
        r.textures_delta.clear();
        let mut found: Vec<(ButtonRole, Rect)> = Vec::new();
        for cs in &r.shapes {
            if let egui::Shape::Text(ts) = &cs.shape {
                let role = match ts.galley.text() {
                    "Save" => Some(ButtonRole::Default),
                    "Cancel" => Some(ButtonRole::Cancel),
                    "Apply" => Some(ButtonRole::Apply),
                    _ => None,
                };
                if let Some(role) = role {
                    found.push((role, cs.shape.visual_bounding_rect()));
                }
            }
        }
        found.sort_by(|a, b| a.1.center().x.partial_cmp(&b.1.center().x).unwrap_or(std::cmp::Ordering::Equal));
        found
    }

    #[test]
    fn dialog_buttons_paint_in_platform_order_and_mirror_as_a_row() {
        // macOS slot order: Cancel, Apply, Default (default last, in the corner); Windows/Linux:
        // Default, Cancel, Apply. The row mirrors in RTL, so left-to-right LTR has Default
        // leftmost and RTL has it rightmost.
        let ltr = button_rects(Direction::Ltr);
        let rtl = button_rects(Direction::Rtl);
        assert_eq!(ltr.len(), 3, "{ltr:?}");
        let mac = cfg!(target_os = "macos");
        let first_ltr = if mac { ButtonRole::Cancel } else { ButtonRole::Default };
        assert_eq!(ltr[0].0, first_ltr, "ltr leftmost: {ltr:?}");
        let first_rtl = if mac { ButtonRole::Default } else { ButtonRole::Apply };
        assert_eq!(rtl[0].0, first_rtl, "rtl leftmost is the mirror of ltr rightmost: {rtl:?}");
        // Mirrored: the rtl row's leftmost centre mirrors the ltr row's rightmost about the row's middle.
        let ltr_mid = (ltr.iter().map(|(_, r)| r.center().x).sum::<f32>()) / 3.0;
        let rtl_mid = (rtl.iter().map(|(_, r)| r.center().x).sum::<f32>()) / 3.0;
        assert!((ltr_mid - rtl_mid).abs() < 200.0, "rows are the same width, mirrored: {ltr_mid} vs {rtl_mid}");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn rtl_dialog_buttons_keep_the_tab_order() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable as _;

        let mut h = Harness::new_ui_state(
            |ui, _s: &mut ()| {
                direction::apply(ui.ctx(), Direction::Rtl);
                let buttons = [
                    DialogButton::new(ButtonRole::Default, "Save", 60.0),
                    DialogButton::new(ButtonRole::Cancel, "Cancel", 60.0),
                    DialogButton::new(ButtonRole::Apply, "Apply", 60.0),
                ];
                dialog_buttons(ui, &Direction::Rtl, &buttons, &ButtonStyle::default());
            },
            (),
        );
        h.run();
        // Tab walks the buttons in creation (add) order, not in painted (mirrored) order.
        let mut order = Vec::new();
        for _ in 0..3 {
            h.key_press(Key::Tab);
            h.run();
            for name in ["Save", "Cancel", "Apply"] {
                if h.get_by_label(name).is_focused() {
                    order.push(name);
                }
            }
        }
        assert_eq!(order, ["Save", "Cancel", "Apply"], "tab order follows creation order: {order:?}");
    }

    #[test]
    fn toggle_knob_travels_from_reading_start_to_reading_end() {
        let rect = Rect::from_min_size(pos2(100.0, 50.0), vec2(30.0, 17.0));
        let style = ToggleStyle::default();
        let (off_l, on_l) = (rect.left() + style.knob_inset, rect.right() - style.knob_inset);
        let (off_r, on_r) = (rect.right() - style.knob_inset, rect.left() + style.knob_inset);
        assert!(off_l < on_l && off_r > on_r);
        // LTR: off at the left, on at the right. RTL: off at the right, on at the left.
        assert!((egui::lerp(off_l..=on_l, 0.0) - (rect.left() + style.knob_inset)).abs() < 1e-4);
        assert!((egui::lerp(off_r..=on_r, 1.0) - (rect.left() + style.knob_inset)).abs() < 1e-4, "rtl on-position is the left edge");
    }
}
