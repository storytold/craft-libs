//! The error checking tag beside a cell.

use craft_behavior::{IconId, Metrics, Sense};
use craft_theme::Variant;
use craft_theme::variant::Tone;
use craft_ui::contract::{
    Access, Common, Component, Cx, MeasureCx, NoValue, Outcome, Overlay, OverlayUi, Overlays as _, PaintUi, PopupKind, Props as _, Role, Show as _,
    ShowContainer as _, Styled as _, Trigger,
};
use craft_ui::display::Divider;
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::env::WordSet as _;
use craft_ui::style::{ComponentStyle, FocusLook, IconLook, StyleCx, SurfaceKind, SurfaceLook};
use egui::{Rect, Response, Vec2};

#[cfg(feature = "screen-testing")]
mod stories;

craft_ui::words! {
    /// The tag's accessible name, English by default (Excel's).
    #[non_exhaustive]
    pub struct ErrorTagWords {
        options: "errorTag.options" = "Error checking options";
    }
}

/// The style of the error tag.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[style(variants = [Variant::SECONDARY])]
#[non_exhaustive]
pub struct ErrorTagStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The warning diamond.
    #[style(icon)]
    glyph: IconLook,
    /// The tag's side. It does not follow the sheet's zoom.
    side: f32,
}

/// What the tag did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct ErrorTagOutput {
    /// The tag's response.
    pub response: Response,
    /// The fix picked this pass, by index into the fixes.
    pub fix: Option<usize>,
}

impl Outcome for ErrorTagOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Excel's error checking tag: a square with the warning-diamond icon in the warn tone and the problem as its
/// tooltip. A click opens a menu with the problem as its title and the fixes below it. The tag is a Tab stop named
/// [`ErrorTagWords::options`]; Enter, Space or Alt+Down open the menu, and a pick reports its index in
/// [`ErrorTagOutput::fix`]. The tag measures itself at a named icon size and does not follow the sheet's zoom; where
/// it sits beside the cell is the grid's.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(overlay)]
pub struct ErrorTag<'a> {
    #[common]
    common: Common,
    problem: Label,
    fixes: &'a [Label],
    fix: Option<usize>,
}

impl<'a> ErrorTag<'a> {
    /// A tag with `problem` as tooltip and menu title and `fixes` as the menu entries.
    pub fn new(problem: impl Into<Label>, fixes: &'a [Label]) -> Self {
        let problem = problem.into();
        ErrorTag { common: Common::new(), problem: problem.clone(), fixes, fix: None }.label(Label::from(ErrorTagWords::WORDS[0])).tooltip(problem)
    }
}

impl Component for ErrorTag<'_> {
    const ROLE: Role = Role::BUTTON;
    const SHOWS_LABEL: bool = false;
    const PARTS: bool = false;
    type Style = ErrorTagStyle;
    type Output = ErrorTagOutput;

    fn style(cx: &StyleCx<'_>, _: ErrorTagStyleKey) -> ErrorTagStyle {
        let parts = cx.variant_look();
        let mut surface = SurfaceLook::new(parts.fill.clone()).radius(parts.radius).kind(SurfaceKind::Raised).kind_down(SurfaceKind::Pressed);
        if let Some(edge) = parts.edge {
            surface = surface.border(edge, cx.tokens.stroke.thin);
        }
        let warn = cx.tone(Tone::Warn);
        ErrorTagStyle {
            surface,
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            glyph: IconLook::new(cx.control().icon, craft_theme::Stateful::new(warn)),
            side: cx.tokens.size.icon.md,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.side)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        cx.popup("menu").trigger(Trigger::Click, cx);
    }

    /// The menu: the problem as a disabled title, a rule, then one row per fix.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("menu");
        let (problem, fixes) = (self.problem.clone(), self.fixes);
        let ran = ui.show(popup, Overlay::menu(), |ui, _| {
            Menu::inline(problem.clone())
                .show(ui, |ui, _| {
                    MenuItem::new(problem.clone()).enabled(false).show(ui);
                    Divider::horizontal().show(ui);
                    let mut ran = None;
                    for (at, fix) in fixes.iter().enumerate() {
                        if MenuItem::new(fix.clone()).show(ui).ran {
                            ran = Some(at);
                        }
                    }
                    ran
                })
                .inner
                .flatten()
        });
        if let Some(at) = ran.flatten() {
            self.fix = Some(at);
            popup.close(cx);
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.has_popup(PopupKind::Menu);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let glyph = Rect::from_center_size(cx.rect.center(), Vec2::splat(cx.style.glyph.side));
        ui.draw().icon(glyph, &IconSpec::svg(IconId::new("warning_diamond")).color(cx.resolve(&cx.style.glyph.color)));
    }

    fn finish(self, cx: Cx<'_, Self>) -> ErrorTagOutput {
        ErrorTagOutput { response: cx.response.clone(), fix: self.fix }
    }
}
