//! `MenuIconRow`: one stop in a menu that holds a row of icons.

use super::item::{RowInput, close_on_pick};
use super::level::{MenuLevel, Row};
use crate::contract::{
    Access, Common, Component, Cx, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props as _, Role, SetPosition, Styled as _,
};
use crate::primitives::IconSpec;
use crate::runtime::env::Label;
use crate::runtime::focus::FocusTarget;
use crate::runtime::frame;
use crate::style::{ComponentStyle, FocusLook, IconLook, StyleCx, SurfaceLook};
use craft_behavior::{Arrows, Flow, Insets, Metrics, NavShape, PartKey, PartRef, Roving, Sense};
use egui::{Context, Id, Rect, Response, Vec2};

/// The part of each icon cell, indexed by the icon's place in the row.
const ICON: PartKey = PartKey::new("icon");

/// One icon of a `MenuIconRow`.
pub struct MenuIcon<'a> {
    icon: IconSpec<'a>,
    label: Label,
    enabled: bool,
}

impl<'a> MenuIcon<'a> {
    /// An icon `icon` named `label`.
    pub fn new(icon: impl Into<IconSpec<'a>>, label: impl Into<Label>) -> Self {
        MenuIcon { icon: icon.into(), label: label.into(), enabled: true }
    }

    /// Whether the icon can be picked.
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// The style of a menu icon row: square icon cells in the row's content, which starts where a menu row's does.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct MenuIconRowStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The glyphs: their side, and their colour per state.
    #[style(icon)]
    glyph: IconLook,
    /// The side of an icon's square cell: the control height.
    cell: f32,
    /// Between neighbouring cells.
    gap: f32,
}

/// A menu row of icons that is one stop: Left and Right rove among the icons, Up and Down leave the row to the
/// neighbouring menu rows as from any row. Picking an icon runs it and closes the menu chain by the menu item rule.
///
/// - The icons are parts in a horizontal roving group that joins the menu's vertical one as a single member, so the
///   menu steps over the row once and the arrows along the row stop at its first and last icon.
/// - A disabled icon is no stop and takes no press; it paints disabled and keeps its tooltip.
/// - The row is one row of its menu level: the pointer or the keyboard on it makes it the active row, and its
///   mnemonic letter moves focus to its first enabled icon.
#[derive(crate::contract::Component)]
pub struct MenuIconRow<'a> {
    #[common]
    common: Common,
    icons: &'a [MenuIcon<'a>],
    picked: Option<usize>,
    /// The row took a key activation it does not run: the first key in a menu the pointer opened (`RowInput::of`).
    withheld: bool,
}

impl<'a> MenuIconRow<'a> {
    /// A row named `label` of `icons`.
    pub fn new(label: impl Into<Label>, icons: &'a [MenuIcon<'a>]) -> Self {
        MenuIconRow { common: Common::new(), icons, picked: None, withheld: false }.label(label)
    }

    fn part(at: usize) -> PartRef {
        ICON.at(at as u64)
    }

    /// The row can run: it is enabled and one of its icons is.
    fn runs(&self, enabled: bool) -> bool {
        enabled && self.icons.iter().any(|icon| icon.enabled)
    }

    fn register(&self, ctx: &Context, level: &MenuLevel, id: Id, label: &str, enabled: bool) {
        level.register(ctx.cumulative_pass_nr(), Row { id, text: label.to_owned(), own: None, enabled: self.runs(enabled) });
    }
}

impl Component for MenuIconRow<'_> {
    const ROLE: Role = Role::TOOLBAR;
    const OVERLAYS: bool = false;
    const SHOWS_LABEL: bool = false;
    type Style = MenuIconRowStyle;
    type Output = MenuIconRowOutput;

    fn style(cx: &StyleCx<'_>, _: MenuIconRowStyleKey) -> MenuIconRowStyle {
        let tokens = cx.tokens;
        let control = cx.control();
        // The cells start `space.pad.regular` in from the popup's own padding, as a menu row's content does, and are
        // as tall as a row.
        let mut metrics = control.metrics();
        metrics.padding = Insets::new(tokens.space.pad.regular, tokens.space.pad.regular, 0.0, 0.0);
        // The cells show the face of an icon button: its hover plate, glyph side and ink.
        let face = crate::buttons::face::icon_face(cx);
        MenuIconRowStyle {
            surface: face.surface,
            metrics,
            focus: cx.focus_look(),
            glyph: face.glyph,
            cell: control.height,
            gap: tokens.space.gap.item,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let level = cx.env.get::<MenuLevel>();
        // The measuring run of the menu body registers the rows, so the real run's letters come from this pass's rows.
        self.register(cx.ui.ctx(), &level, cx.id, cx.resolved.label, cx.resolved.enabled);
        let count = self.icons.len() as f32;
        let width = cx.style.cell * count + cx.style.gap * (count - 1.0).max(0.0);
        Vec2::new(cx.room.map_or(width, |room| width.min(room)), cx.style.cell)
    }

    fn fills(&self) -> Option<craft_behavior::Axis> {
        Some(craft_behavior::Axis::X)
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let (cell, gap) = (cx.style.cell, cx.style.gap);
        let mut flow = Flow::new(cx.content_rect, cx.flow.is_rtl());
        for (at, icon) in self.icons.iter().enumerate() {
            let (slot, rest) = flow.take_start(cell);
            flow = rest.take_start(gap).1;
            let part = cx.part(Self::part(at), slot).tooltip(icon.label.clone());
            if icon.enabled {
                part.sense(Sense::CLICK | Sense::FOCUS).focusable().state_layer();
            } else {
                part.inactive(true);
            }
        }
        cx.roving(Roving::new(NavShape::Flat).arrows(Arrows::Horizontal));
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        cx.join_roving();
        let ctx = cx.core.ctx.clone();
        let level = cx.env.get::<MenuLevel>();
        let enabled = cx.resolved.enabled;
        self.register(&ctx, &level, cx.id, cx.resolved.label, enabled);
        let runs = self.runs(enabled);
        let input = RowInput::of(cx, &level);
        let (moving, hovered) = (input.moving, input.hovered);
        // As a menu item: the pointer moving over the row, or the keyboard landing on it, makes it the active row. The
        // first key in a menu the pointer opened picks no icon of a row the pointer did not light: `RowInput::of` runs
        // the lit row instead, or closes the menu.
        input.activate(&ctx, cx.id, &level, runs);
        self.withheld = input.withheld;
        // The pointer on the row closes another row's open submenu, unless it is on its way into it.
        if hovered && moving && !level.aimed(cx.id) && level.clear_submenu() {
            frame::request_repaint(&ctx);
        }
        // The row's letter names no icon to run: it moves focus to the first icon that can run.
        if level.take_run(cx.id)
            && runs
            && let Some(first) = self.icons.iter().position(|icon| icon.enabled)
        {
            crate::runtime::focus::request(&ctx, FocusTarget::Part(cx.id, Self::part(first)));
        }
        let icons = self.icons;
        self.picked = icons
            .iter()
            .enumerate()
            .filter(|(_, icon)| runs && !input.withheld && icon.enabled)
            .map(|(at, _)| at)
            .find(|at| cx.part_activated(Self::part(*at)).is_some());
        if self.picked.is_some() {
            close_on_pick(&ctx);
        }
    }

    /// A key activation the row withheld does not activate it.
    fn activated(&self, cx: &Cx<'_, Self>) -> bool {
        !self.withheld && cx.activation().is_some()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let side = cx.style.glyph.side;
        let draw = ui.draw();
        for (at, icon) in self.icons.iter().enumerate() {
            let part = Self::part(at);
            let Some(cell) = cx.part_if_declared(part).map(|result| result.rect) else { continue };
            let ink = cx.resolve_part(part, &cx.style.glyph.color);
            let spec = if icon.icon.color.is_some() { icon.icon.clone() } else { icon.icon.clone().color(ink) };
            draw.icon(Rect::from_center_size(cell.center(), Vec2::splat(side)), &spec);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        for (at, icon) in self.icons.iter().enumerate() {
            let node = access.node(Self::part(at), Role::BUTTON);
            node.name = Name::Text(cx.env.resolve(&icon.label).as_str().to_owned());
            node.position = Some(SetPosition::new(at).of(self.icons.len()));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> MenuIconRowOutput {
        MenuIconRowOutput { response: cx.response.clone(), picked: self.picked }
    }
}

/// What a `MenuIconRow` returns.
#[non_exhaustive]
pub struct MenuIconRowOutput {
    /// The row's response.
    pub response: Response,
    /// The index of the icon picked in this pass.
    pub picked: Option<usize>,
}

impl Outcome for MenuIconRowOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        crate::testing::StoryOutput { id: Some(self.response.id), clicked: self.picked.is_some(), ..crate::testing::StoryOutput::default() }
    }
}
