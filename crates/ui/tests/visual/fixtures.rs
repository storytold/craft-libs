//! Independent, font-free reference painting for the shared controls' public visual contracts.
//! Reference paths never call craft-ui or derive their expected rectangles from its responses.

use craft_ui::buttons::{ButtonVisuals, IconButton, IconButtonStyle};
use craft_ui::layout::{ResizeHandle, ResizeHandleStyle, SplitAxis, SplitSize, split_rect};
use craft_ui::tabs::Tab;
use egui::{Align, Color32, CornerRadius, Id, Layout, Painter, Rect, Response, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2};

pub const CANVAS: Vec2 = vec2(128.0, 72.0);
pub const GAP: f32 = 8.0;

#[derive(Clone, Copy, Debug)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    pub fn name(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn egui(self) -> egui::Theme {
        match self {
            Self::Light => egui::Theme::Light,
            Self::Dark => egui::Theme::Dark,
        }
    }

    pub fn surface(self) -> Color32 {
        match self {
            Self::Light => Color32::from_rgb(248, 250, 252),
            Self::Dark => Color32::from_rgb(17, 24, 39),
        }
    }

    pub fn palette(self) -> Palette {
        let rgb = |[r, g, b]: [u8; 3]| Color32::from_rgb(r, g, b);
        let colors = match self {
            Self::Light => [
                [226, 232, 240],
                [186, 230, 253],
                [125, 211, 252],
                [37, 99, 235],
                [148, 163, 184],
                [15, 23, 42],
                [12, 74, 110],
                [8, 47, 73],
                [220, 38, 38],
            ],
            Self::Dark => [
                [31, 41, 55],
                [55, 65, 81],
                [75, 85, 99],
                [67, 56, 202],
                [100, 116, 139],
                [226, 232, 240],
                [243, 244, 246],
                [255, 255, 255],
                [56, 189, 248],
            ],
        };
        Palette {
            normal: rgb(colors[0]),
            hovered: rgb(colors[1]),
            pressed: rgb(colors[2]),
            selected: rgb(colors[3]),
            border: rgb(colors[4]),
            icon: rgb(colors[5]),
            hover_icon: rgb(colors[6]),
            press_icon: rgb(colors[7]),
            focus: rgb(colors[8]),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub normal: Color32,
    pub hovered: Color32,
    pub pressed: Color32,
    pub selected: Color32,
    pub border: Color32,
    pub icon: Color32,
    pub hover_icon: Color32,
    pub press_icon: Color32,
    pub focus: Color32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Normal,
    Hover,
    Press,
    Selected,
    SelectedPress,
    Disabled,
    Focus,
}

impl State {
    pub const ALL: [Self; 7] = [Self::Normal, Self::Hover, Self::Press, Self::Selected, Self::SelectedPress, Self::Disabled, Self::Focus];

    pub fn selected(self) -> bool {
        matches!(self, Self::Selected | Self::SelectedPress)
    }

    pub fn pressed(self) -> bool {
        matches!(self, Self::Press | Self::SelectedPress)
    }

    pub fn pointer(self) -> bool {
        matches!(self, Self::Hover | Self::Press | Self::SelectedPress | Self::Disabled)
    }

    pub fn enabled(self) -> bool {
        self != Self::Disabled
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Component {
    Button,
    SingleShapeButton,
    Tab,
    HorizontalSplit,
    VerticalSplit,
}

impl Component {
    pub const ALL: [Self; 5] = [Self::Button, Self::SingleShapeButton, Self::Tab, Self::HorizontalSplit, Self::VerticalSplit];

    pub fn is_split(self) -> bool {
        matches!(self, Self::HorizontalSplit | Self::VerticalSplit)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Case {
    pub component: Component,
    pub state: State,
    pub theme: Theme,
    pub scale: f32,
    pub compact: bool,
}

impl Case {
    pub fn name(self) -> String {
        format!("{:?}-{:?}-{}-{}-{}x", self.component, self.state, self.theme.name(), if self.compact { "compact" } else { "normal" }, self.scale)
    }

    pub fn size(self) -> Vec2 {
        match (self.component, self.compact) {
            (Component::Tab, false) => vec2(44.0, 28.0),
            (Component::Tab, true) => vec2(22.0, 12.0),
            (_, false) => vec2(36.0, 28.0),
            (_, true) => vec2(18.0, 12.0),
        }
    }

    pub fn control_rect(self) -> Rect {
        if self.component.is_split() { reference_split(self).1 } else { Rect::from_min_size(pos2(12.0, 12.0), self.size()) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mutation {
    None,
    Color,
    Spacing,
    Stroke,
}

pub fn button_style(theme: Theme) -> IconButtonStyle {
    let p = theme.palette();
    let visual = |fill, icon| ButtonVisuals { fill, icon, stroke: Stroke::new(1.0, p.border) };
    IconButtonStyle {
        normal: visual(p.normal, p.icon),
        hovered: visual(p.hovered, p.hover_icon),
        pressed: Some(visual(p.pressed, p.press_icon)),
        selected: visual(p.selected, Color32::WHITE),
        corner_radius: CornerRadius::same(3),
        focus_stroke: Stroke::new(1.0, p.focus),
    }
}

pub fn icon(painter: &Painter, rect: Rect, color: Color32, compact: bool) {
    let half = if compact { vec2(3.0, 2.0) } else { vec2(6.0, 4.0) };
    painter.line_segment([rect.center() - half, rect.center() + half], Stroke::new(1.0, color));
    painter.line_segment([rect.center() + vec2(-half.x, half.y), rect.center() + vec2(half.x, -half.y)], Stroke::new(1.0, color));
}

fn tab_body(ui: &Ui, response: &Response, selected: bool, palette: Palette, compact: bool) {
    let (fill, tint) = if selected {
        (palette.selected, Color32::WHITE)
    } else if response.is_pointer_button_down_on() {
        (palette.pressed, palette.press_icon)
    } else if response.hovered() {
        (palette.hovered, palette.hover_icon)
    } else {
        (palette.normal, palette.icon)
    };
    ui.painter().rect_filled(response.rect, 0.0, fill);
    ui.painter().rect_stroke(response.rect, 0.0, Stroke::new(1.0, palette.border), StrokeKind::Inside);
    icon(ui.painter(), response.rect, tint, compact);
}

/// Actual production components, driven through egui responses and app-supplied fixture styles.
pub fn actual(ui: &mut Ui, case: Case, mutation: Mutation) -> Vec<Response> {
    let p = case.theme.palette();
    if case.component.is_split() {
        let axis = if matches!(case.component, Component::HorizontalSplit) { SplitAxis::Horizontal } else { SplitAxis::Vertical };
        let area = Rect::from_min_size(pos2(12.0, 12.0), if case.compact { vec2(52.0, 20.0) } else { vec2(104.0, 44.0) });
        let split =
            split_rect(area, axis, SplitSize::Ratio(if case.compact { 0.5 } else { 0.375 }), if case.compact { 3.0 } else { 4.0 }, 4.0).unwrap();
        ui.painter().rect_filled(split.first, 0.0, p.normal);
        ui.painter().rect_filled(split.second, 0.0, p.hovered);
        let response = ui
            .add_enabled_ui(case.state.enabled(), |ui| {
                ResizeHandle::new(
                    Id::unique("visual-divider"),
                    axis,
                    split.divider,
                    "Resize panes",
                    ResizeHandleStyle { hovered: p.hovered, dragged: p.pressed, focused: p.focus },
                )
                .hit_margin(0.0)
                .show(ui)
                .response
            })
            .inner;
        return vec![response];
    }
    let spacing = GAP + if mutation == Mutation::Spacing { 1.0 } else { 0.0 };
    let mut style = button_style(case.theme);
    if mutation == Mutation::Color {
        style.normal.fill = Color32::from_rgb(200, 60, 130);
    }
    if mutation == Mutation::Stroke {
        style.normal.stroke.width = 2.0;
    }
    let mut responses = Vec::new();
    ui.scope_builder(
        UiBuilder::new().max_rect(Rect::from_min_size(pos2(12.0, 12.0), CANVAS - vec2(24.0, 24.0))).layout(Layout::left_to_right(Align::Min)),
        |ui| {
            ui.spacing_mut().item_spacing.x = spacing;
            ui.add_enabled_ui(case.state.enabled(), |ui| {
                for index in 0..2 {
                    let selected = index == 0 && case.state.selected();
                    let response = if matches!(case.component, Component::Tab) {
                        let rect = Rect::from_min_size(pos2(12.0 + index as f32 * (case.size().x + spacing), 12.0), case.size());
                        Tab::new(Id::unique(("visual-tab", index)), "Tab", selected)
                            .focus_stroke(style.focus_stroke)
                            .show_at(ui, rect, |ui, r| tab_body(ui, r, selected, p, case.compact))
                    } else {
                        let button = IconButton::new("Action", case.size(), &style).selected(selected);
                        let button = if matches!(case.component, Component::SingleShapeButton) { button.single_shape_frame() } else { button };
                        button.show(ui, |ui, rect, tint| icon(ui.painter(), rect, tint, case.compact))
                    };
                    responses.push(response);
                }
            });
        },
    );
    responses
}

/// Fixed reference geometry, written independently of split_rect's implementation.
fn reference_split(case: Case) -> (Rect, Rect, Rect) {
    let rect = |x, y, w, h| Rect::from_min_size(pos2(x, y), vec2(w, h));
    match (case.component, case.compact) {
        (Component::HorizontalSplit, false) => (rect(12.0, 12.0, 37.5, 44.0), rect(49.5, 12.0, 4.0, 44.0), rect(53.5, 12.0, 62.5, 44.0)),
        (Component::HorizontalSplit, true) => (rect(12.0, 12.0, 24.5, 20.0), rect(36.5, 12.0, 3.0, 20.0), rect(39.5, 12.0, 24.5, 20.0)),
        (Component::VerticalSplit, false) => (rect(12.0, 12.0, 104.0, 15.0), rect(12.0, 27.0, 104.0, 4.0), rect(12.0, 31.0, 104.0, 25.0)),
        (Component::VerticalSplit, true) => (rect(12.0, 12.0, 52.0, 8.5), rect(12.0, 20.5, 52.0, 3.0), rect(12.0, 23.5, 52.0, 8.5)),
        _ => panic!("split fixture required"),
    }
}

/// Baseline painting uses only egui shapes, literal fixture geometry, and the specified state.
pub fn reference(ui: &Ui, case: Case) {
    let p = case.theme.palette();
    if case.component.is_split() {
        let (first, divider, second) = reference_split(case);
        ui.painter().rect_filled(first, 0.0, p.normal);
        ui.painter().rect_filled(second, 0.0, p.hovered);
        let fill = match case.state {
            State::Hover => Some(p.hovered),
            State::Press => Some(p.pressed),
            State::Focus => Some(p.focus),
            _ => None,
        };
        if let Some(fill) = fill {
            ui.painter().rect_filled(divider, 0.0, fill);
        }
        return;
    }
    let mut painter = ui.painter().clone();
    if case.state == State::Disabled {
        // The fixture's known disabled alpha. Do not ask the actual widget how it was painted.
        painter.multiply_opacity(0.4);
    }
    for index in 0..2 {
        let state = if index == 0 { case.state } else { State::Normal };
        let (fill, tint) = match state {
            State::Selected | State::SelectedPress => (p.selected, Color32::WHITE),
            State::Press => (p.pressed, p.press_icon),
            State::Hover => (p.hovered, p.hover_icon),
            _ => (p.normal, p.icon),
        };
        let rect = Rect::from_min_size(pos2(12.0 + index as f32 * (case.size().x + GAP), 12.0), case.size());
        let radius = if matches!(case.component, Component::Tab) { 0 } else { 3 };
        if matches!(case.component, Component::SingleShapeButton) {
            painter.rect(rect, radius, fill, Stroke::new(1.0, p.border), StrokeKind::Inside);
        } else {
            painter.rect_filled(rect, radius, fill);
            painter.rect_stroke(rect, radius, Stroke::new(1.0, p.border), StrokeKind::Inside);
        }
        icon(&painter, rect, tint, case.compact);
        if state == State::Focus {
            painter.rect_stroke(rect.shrink(2.0), radius, Stroke::new(1.0, p.focus), StrokeKind::Inside);
        }
    }
}
