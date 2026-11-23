//! `Checkbox`: a box with a check mark, a dash while the value is mixed. Its text is the label step's (a `Field`'s,
//! `PaintUi::labelled`'s, or the one `show` runs for a labelled instance); the control draws only the box.

use crate::contract::{Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, PaintUi, Pointer as _, Role, Styled as _};
use crate::primitives::{StrokeWidth, glyph};
use crate::style::{ComponentStyle, FocusLook, StyleColor, StyleCx, SurfaceKind, SurfaceLook};
use craft_behavior::{Metrics, Sense};
use craft_theme::Stateful;
use egui::{Rect, Vec2};

/// The style of a check box.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct CheckboxStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The box.
    well: SurfaceLook,
    /// The check mark and the dash.
    mark: Stateful<StyleColor>,
    side: f32,
}

/// The side of a quiet check box, against the icon side.
const QUIET_SIDE: f32 = 0.6;
/// The corner radius of a quiet check box's well, in points: a small box keeps small corners.
const QUIET_RADIUS: u8 = 2;

/// A check box over a bool; a `Mixed` binding shows the mixed state and a click makes it checked.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct Checkbox<'a> {
    #[common]
    common: Common,
    value: Bind<'a, bool>,
    /// A small, dark box with a plain mark, for a check that sits over a picture (a thumbnail tile's).
    #[style_key]
    #[prop]
    quiet: bool,
}

impl<'a> Checkbox<'a> {
    /// A check box of `value`.
    pub fn new(value: Bind<'a, bool>) -> Self {
        Checkbox { common: Common::new(), value, quiet: false }
    }
}

impl Component for Checkbox<'_> {
    const ROLE: Role = Role::CHECK_BOX;
    const PARTS: bool = false;
    const LABELLED_AFTER: bool = true;
    const OVERLAYS: bool = false;
    type Style = CheckboxStyle;
    type Output = EditResponse<bool>;

    fn style(cx: &StyleCx<'_>, key: CheckboxStyleKey) -> CheckboxStyle {
        let control = cx.control();
        if key.quiet {
            return CheckboxStyle {
                metrics: Metrics::new().min_size(Vec2::new(0.0, control.icon * QUIET_SIDE)),
                focus: cx.focus_look(),
                well: super::quiet_indicator(cx, egui::CornerRadius::same(QUIET_RADIUS)),
                mark: super::quiet_mark(cx),
                side: control.icon * QUIET_SIDE,
            };
        }
        CheckboxStyle {
            // In a ribbon, the look's row height (`RibbonLook::row`), so check boxes stand at its buttons' pitch.
            metrics: Metrics::new().min_size(Vec2::new(0.0, crate::buttons::row_height(cx, control.height))),
            focus: cx.focus_look(),
            well: super::indicator(cx, cx.surface(SurfaceKind::Well).radius),
            mark: super::mark(cx),
            side: control.icon,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.icon())
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::DRAG | Sense::FOCUS
    }

    fn bind(&mut self) -> Option<BindMut<'_, bool>> {
        Some(self.value.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let on = cx.value().copied();
        if let Some(swept) = cx.swept_toggle(on) {
            if on != Some(swept) {
                cx.set(swept);
            }
        } else if cx.activation().is_some() {
            cx.set(!on.unwrap_or(false));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let square = Rect::from_center_size(cx.rect.center(), Vec2::splat(cx.style.side));
        let draw = ui.draw();
        draw.surface_look(square, &cx.style.well, cx.state());
        if cx.mixed {
            draw.stroke(&glyph::dash(square), StrokeWidth::Medium, cx.resolve(&cx.style.mark));
        } else if cx.value().copied().unwrap_or(false) {
            draw.stroke(&glyph::check(square), StrokeWidth::Medium, cx.resolve(&cx.style.mark));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<bool> {
        cx.edit_response()
    }
}

#[cfg(feature = "suite")]
pub(crate) mod stories;
