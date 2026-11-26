//! `SplitButton`: an action button whose end part opens a menu of variants or presets.

use super::face::{self, Arrangement, FaceLook, FaceSpec};
use crate::contract::{
    Access, Bind, BodyCx, Common, Component, Cx, Entry, Focus as _, Hover, MeasureCx, Name, Outcome, OverlayUi, Overlays as _, PaintUi, PartsCx,
    Pointer as _, Props, Role, ShowContainer as _, Styled as _, WordSetCx as _,
};
use crate::menu::{self, Menu};
use crate::primitives::{IconSpec, PaintShape, RuleKind};
use crate::runtime::commands::CommandRef;
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, Corners, FocusLook, IconLook, RuleLook, RuleWidth, Stateful, StyleColor, StyleCx, SurfaceLook, TextLook};
use craft_behavior::{Flow, HitLevel, HitPriority, Metrics, NavMove, PartKey, Sense, Value};
use craft_theme::{
    States, Variant,
    variant::{Emphasis, Tone},
};
use egui::{CornerRadius, Pos2, Rect, Response, Ui, Vec2};

const ARROW: PartKey = PartKey::new("arrow");

/// The style of a split button: one surface under both parts, each part's hover and pressed fill over it, and a
/// divider between them.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[style(variants = [Variant::SECONDARY, Variant::PRIMARY, Variant::GHOST, Variant::DANGER, Variant::look(Emphasis::Outline, Tone::Accent), super::FILLED])]
#[non_exhaustive]
pub struct SplitButtonStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The label.
    #[style(text)]
    text: TextLook,
    /// The leading icon.
    #[style(icon)]
    icon: IconLook,
    face: FaceLook,
    /// What each part adds over the surface: clear at rest, the variant's hover and pressed fills.
    part_fill: Stateful<StyleColor>,
    /// The chevron: the variant's secondary-glyph ink, muted on a neutral button without fill.
    chevron: Stateful<StyleColor>,
    /// The line between the parts, in the variant's edge colour.
    divider: Stateful<StyleColor>,
    /// The divider rule's width.
    divider_width: RuleWidth,
    /// The divider's gap from the top and bottom edges.
    divider_inset: f32,
    radius: CornerRadius,
}

/// What a split button reports: whether its action runs, and the entry picked from the menu this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct SplitOutput<T> {
    /// The button's response, which reports the clicks on the arrow as the main part's.
    pub response: Response,
    /// The action runs in this pass: a click on the main part, or an activation by key or AccessKit. Never a click on
    /// the arrow.
    pub activated: bool,
    /// The entry the user picked.
    pub chosen: Option<T>,
    /// The pointer's stay over the main part: hovered, entered, left and clicked under the pointer.
    pub hover: Hover,
}

impl<T: Value> Outcome for SplitOutput<T> {
    type Value = crate::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// What a split button's arrow opens.
#[non_exhaustive]
pub enum SplitMenu<'a, T> {
    /// A menu of rows, one per entry.
    Entries(&'a [Entry<T>]),
    /// A menu whose content the caller draws.
    Body(MenuBody<'a>),
}

/// The caller's drawing of a split button's menu.
pub type MenuBody<'a> = Box<dyn FnMut(&mut Ui, &mut BodyCx<'_>) + 'a>;

/// A button whose end opens a menu. A press on the arrow part opens the menu below the button; a press on the main
/// part runs the action and never opens it. The button is one Tab stop: Space and Enter run the action, Alt+Down and
/// Down open the menu with its first row active, Escape closes it with focus back on the button.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(overlay, probe(SplitButton<'static, u8>))]
pub struct SplitButton<'a, T: Value + 'static> {
    #[common]
    common: Common,
    menu: SplitMenu<'a, T>,
    /// The arrow's accessible name and tooltip ("Tracing Presets"); the label alone names it without one.
    #[prop(into)]
    arrow_label: Option<Label>,
    /// A leading icon on the main part.
    #[prop(into)]
    icon: Option<IconSpec<'a>>,
    /// How the icon and label are laid out: beside each other with the arrow part at the end, or the icon over the
    /// label with the arrow part (the label and its chevron) under the icon.
    #[style_key]
    #[prop]
    arrangement: Arrangement,
    chosen: Option<T>,
}

impl<'a, T: Value + 'static> SplitButton<'a, T> {
    /// A split button showing `label` whose arrow opens `entries`.
    pub fn new(label: impl Into<Label>, entries: &'a [Entry<T>]) -> Self {
        SplitButton::with_menu(SplitMenu::Entries(entries)).label(label)
    }

    /// A split button showing `label` whose arrow opens a menu with `body` as its content. A body reports no
    /// `chosen` entry: its rows run their own commands.
    pub fn body(label: impl Into<Label>, body: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'a) -> Self {
        SplitButton::with_menu(SplitMenu::Body(Box::new(body))).label(label)
    }

    /// A split button bound to `command`: its label, enabled state and main part follow the command, which a click
    /// on the main part fires; its arrow opens `menu`.
    pub fn command(command: impl Into<CommandRef>, menu: SplitMenu<'a, T>) -> Self {
        <SplitButton<'a, T> as Props>::command(SplitButton::with_menu(menu).label(Label::Empty), command)
    }

    fn with_menu(menu: SplitMenu<'a, T>) -> Self {
        SplitButton { common: Common::new(), menu, arrow_label: None, icon: None, arrangement: Arrangement::default(), chosen: None }
    }

    /// The face: a stacked face shows the chevron after its label, so the arrow part holds both.
    fn spec<'s>(&'s self, label: &'s str, chevron: &'s IconSpec<'s>) -> FaceSpec<'s> {
        FaceSpec {
            arrangement: self.arrangement,
            label,
            role: None,
            lead: self.icon.as_ref(),
            picture: None,
            trail: (self.arrangement == Arrangement::Stacked).then_some(chevron),
            loading: false,
            mnemonic: None,
            align: crate::primitives::HAlign::Center,
            chord: None,
            detail: None,
        }
    }
}

/// The chevron the arrow part shows.
fn chevron(look: &FaceLook) -> IconSpec<'static> {
    if look.arrow.is_some() { IconSpec::glyph(&RIBBON_ARROW) } else { crate::controls::disclosure::DisclosureGlyph::menu_button() }
}

/// The ribbon's menu arrow where it sets one (`RibbonLook::arrow`): the small filled triangle, pointing down.
const RIBBON_ARROW: crate::controls::disclosure::DisclosureGlyph = crate::controls::disclosure::DisclosureGlyph::new(1.0);

impl<T: Value + 'static> Component for SplitButton<'_, T> {
    const ROLE: Role = Role::BUTTON;
    const KEYS: crate::runtime::focus::KeyUse = crate::runtime::focus::KeyUse::of(crate::contract::Input::Pointer).with_moves();
    type Style = SplitButtonStyle;
    type Output = SplitOutput<T>;

    fn default_variant(env: &crate::runtime::env::Env) -> Option<Variant> {
        super::push_default(env)
    }

    fn style(cx: &StyleCx<'_>, key: SplitButtonStyleKey<T>) -> SplitButtonStyle {
        let cx = &super::placed(cx);
        let parts = cx.variant_look();
        let metrics = face::metrics_of(cx, &parts, key.arrangement);
        let mut surface = face::surface(cx, &parts);
        let (fill, clear) = (parts.fill.off, cx.color(craft_theme::Rgba::TRANSPARENT));
        surface.fill = Stateful::new(fill.rest).off(States::new(fill.rest, fill.rest, fill.rest, fill.disabled));
        let divider = parts.edge.unwrap_or_else(|| Stateful::new(clear));
        let chevron = parts.muted;
        let (text, icon) = face::ink(cx);
        SplitButtonStyle {
            surface,
            metrics,
            focus: face::focus(cx),
            text,
            icon,
            face: FaceLook::of(cx),
            part_fill: Stateful::new(clear).off(States::new(clear, fill.hover, fill.pressed, clear)),
            chevron,
            divider,
            divider_width: cx.rule(RuleKind::Divider).width,
            divider_inset: cx.tokens.space.gap.inline,
            radius: parts.radius,
        }
    }

    /// Inline: the face and the arrow part, which sits in the end padding. Stacked: the face, whose chevron follows
    /// the label inside the arrow part.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let arrow = face::arrow_room(cx.env.get::<super::ControlPlace>(), cx.env.get::<super::RibbonLook>());
        let chevron = chevron(&FaceLook { arrow, ..cx.style.face.clone() });
        let face = face::measure(cx, &self.spec(cx.resolved.label, &chevron));
        match self.arrangement {
            Arrangement::Stacked => face,
            _ => Vec2::new(face.x + arrow.unwrap_or_else(|| face::measured_icon(cx)), face.y),
        }
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let arrow = match self.arrangement {
            Arrangement::Stacked => {
                let chevron = chevron(&cx.style.face);
                let (look, side) = (&cx.style.face, cx.style.face.arrow.unwrap_or(cx.style.icon.side));
                let y = face::stacked_divide(cx, &self.spec(cx.resolved.label, &chevron), look, side, cx.content_rect);
                Rect::from_min_max(egui::pos2(rect.left(), y.clamp(rect.top(), rect.bottom())), rect.max)
            }
            _ => Flow::new(rect, cx.flow.is_rtl()).take_end(cx.style.face.arrow.unwrap_or(cx.style.icon.side)).0,
        };
        let part = cx
            .part(ARROW, arrow)
            .sense(Sense::CLICK)
            .priority(HitPriority::new(HitLevel::Handle, 0))
            .pointer_only("Alt+Down and Down open the menu")
            .toggles("menu");
        if let Some(label) = &self.arrow_label {
            part.tooltip(label.clone());
        }
    }

    /// The arrow opens the menu: a click on it never runs the action, nor fires the bound command.
    fn activated(&self, cx: &Cx<'_, Self>) -> bool {
        cx.activation().is_some() && cx.part_activated(ARROW).is_none()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        cx.join_roving();
        let menu = cx.popup("menu");
        if !menu.is_open(cx) {
            let mut asked = false;
            while let Some((step, _)) = cx.intents().take_move() {
                asked |= matches!(step, NavMove::Down);
            }
            if asked {
                menu.open(cx);
            }
        }
    }

    /// The menu: one `MenuItem` row per entry, a rule for a separator, or the caller's body. Running an entry's row
    /// reports its value and closes the menu; a disabled entry takes neither focus-run nor a pick. A body's rows run
    /// their own commands, which close the layer as any row's run does; the body's `request_close` (a palette's pick)
    /// closes the layer too.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup("menu");
        let name = cx.label();
        let entries = match &mut self.menu {
            SplitMenu::Entries(entries) => *entries,
            SplitMenu::Body(body) => {
                // The body's `request_close` (a swatch pick) and a row's run close the inline menu; it is open while
                // the popup is, so its close is the popup's. Its own open slot would stay closed and leave the layer
                // open around an empty body. Its focus scope ignores Escape, so the binding takes no Escape from the
                // layer's own rule.
                let closed = ui.show(popup, menu::layer(false), |ui, _| {
                    Menu::inline(name.clone()).open(Bind::given(true)).show(ui, |ui, cx| body(ui, cx)).output.closed
                });
                if closed.flatten().is_some() {
                    popup.close(cx);
                }
                return;
            }
        };
        let ran = ui.show(popup, menu::layer(false), |ui, _| {
            Menu::inline(name.clone()).values().show(ui, |ui, _| menu::entry_rows(ui, entries)).inner.flatten()
        });
        if let Some(value) = ran.flatten().and_then(|at| entries.get(at)).and_then(Entry::value) {
            self.chosen = Some(value.clone());
            popup.close(cx);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let name = match &self.arrow_label {
            Some(label) => cx.env.resolve(label).as_str().to_owned(),
            None => cx.resolved.label.to_owned(),
        };
        access.node(ARROW, Role::BUTTON).name = Name::Text(name);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if self.arrangement == Arrangement::Stacked {
            self.paint_stacked(ui, cx);
            return;
        }
        let (rtl, direction) = (cx.flow.is_rtl(), cx.env.direction());
        let arrow_side = cx.style.face.arrow.unwrap_or(cx.style.icon.side);
        let (arrow, main) = Flow::new(cx.rect, rtl).take_end(arrow_side);
        let main = main.rect();
        let main_fill = cx.root_state().resolve(&cx.style.part_fill);
        let draw = ui.draw();
        draw.fill(&PaintShape::Rect(main, Corners::Start.apply(cx.style.radius, direction)), main_fill);
        draw.fill(&PaintShape::Rect(arrow, Corners::End.apply(cx.style.radius, direction)), cx.resolve_part(ARROW, &cx.style.part_fill));
        let area = Flow::new(main, rtl).inset(cx.style.metrics.padding);
        let chevron = chevron(&cx.style.face).color(cx.resolve(&cx.style.chevron));
        face::paint(ui, cx, (&cx.style.text, &cx.style.icon), &cx.style.face, &self.spec(cx.resolved.label, &chevron), area);
        let draw = ui.draw();
        draw.icon(Rect::from_center_size(arrow.center(), Vec2::splat(arrow_side)), &chevron);
        let x = if rtl { arrow.right() } else { arrow.left() };
        let inset = cx.style.divider_inset;
        let divider = RuleLook { color: cx.resolve(&cx.style.divider).egui(), width: cx.style.divider_width };
        draw.rule_with(&divider, Pos2::new(x, cx.rect.top() + inset), Pos2::new(x, cx.rect.bottom() - inset));
    }

    fn finish(self, cx: Cx<'_, Self>) -> SplitOutput<T> {
        SplitOutput { activated: self.activated(&cx), response: cx.response.clone(), chosen: self.chosen, hover: cx.hover() }
    }
}

impl<T: Value + 'static> SplitButton<'_, T> {
    /// Paints a stacked split button: the main part (the icon) over the arrow part (the label and its chevron), each
    /// with its own state fill, and the divider across the button where they meet. The arrow part is the one `parts`
    /// declared; a face, which declares none, shows the whole button as the arrow part.
    fn paint_stacked(&self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let rect = cx.rect;
        let arrow = cx.part_if_declared(ARROW).map_or(rect, |part| part.rect);
        let main = Rect::from_min_max(rect.min, Pos2::new(rect.right(), arrow.top().max(rect.top())));
        let radius = cx.style.radius;
        let draw = ui.draw();
        draw.fill(&PaintShape::Rect(main, CornerRadius { sw: 0, se: 0, ..radius }), cx.root_state().resolve(&cx.style.part_fill));
        draw.fill(&PaintShape::Rect(arrow, CornerRadius { nw: 0, ne: 0, ..radius }), cx.resolve_part(ARROW, &cx.style.part_fill));
        let chevron = chevron(&cx.style.face).color(cx.resolve(&cx.style.chevron));
        face::paint(ui, cx, (&cx.style.text, &cx.style.icon), &cx.style.face, &self.spec(cx.resolved.label, &chevron), cx.content_rect);
        if main.height() > 0.0 {
            let inset = cx.style.divider_inset;
            let divider = RuleLook { color: cx.resolve(&cx.style.divider).egui(), width: cx.style.divider_width };
            ui.draw().rule_with(&divider, Pos2::new(rect.left() + inset, arrow.top()), Pos2::new(rect.right() - inset, arrow.top()));
        }
    }
}
