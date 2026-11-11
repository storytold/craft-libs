//! The collapse bar of a dock column: the chevron, a part of the column's run that collapses or expands it, and the
//! band beside it that moves the column and opens its menu.

use crate::buttons::IconButton;
use crate::buttons::icon_button::IconButtonStyleKey;
use crate::contract::overlay::Trigger;
use crate::contract::{Access, BodyCx, Name, PaintUi, Props as _, Role, Show as _, ShowContainer as _, WordSetCx as _};
use crate::docking::group::GroupGrip;
use crate::docking::{DockWords, PanelSpec};
use crate::menu::{GroupItem, Menu, MenuWords, PanelMenu, PanelRow};
use crate::primitives::{Glyph, IconSpec};
use crate::runtime::env::Label;
use craft_behavior::PartKey;
use craft_behavior::dock::{Column, DockEdge};
use craft_theme::variant::Tone;
use craft_theme::{ControlSize, CustomVariant, Emphasis, States, Tokens, Variant, VariantLook};
use egui::{Color32, Pos2, Rect, Ui};

/// The chevron of a column's bar: a click collapses an expanded column to icons and expands an iconic one.
pub(in crate::docking) const CHEVRON: PartKey = PartKey::new("chevron");

/// What the column bar recorded while the body ran, for the container to read when it finishes.
#[derive(Default)]
pub(in crate::docking) struct BarRecord {
    /// The user asked to collapse (`true`) or expand (`false`) the column.
    pub(in crate::docking) iconic: Option<bool>,
    /// The dock row picked in the column menu.
    pub(in crate::docking) picked: Option<GroupItem>,
}

/// A double chevron's arms, as a share of the icon box's side: the glyph the collapse control draws is small beside
/// the bar's other icons.
const ARM: f32 = 0.17;

/// How far each chevron of the pair reaches along the arrow, and the distance between the two, as shares of an arm.
const REACH: f32 = 0.8;
const PITCH: f32 = 1.4;

/// The look of the collapse control: ghost, with the muted text until the pointer is over it, and no fill under the
/// pointer.
fn collapse_look(tokens: &Tokens) -> VariantLook {
    let ghost = VariantLook::of(Emphasis::Ghost, Tone::Neutral, tokens);
    let mut text = ghost.text;
    text.rest = tokens.color.text.muted;
    VariantLook::new(States::new(ghost.fill.rest, ghost.fill.rest, ghost.fill.rest, ghost.fill.rest), text)
}

const COLLAPSE: CustomVariant = CustomVariant::new::<IconButtonStyleKey>("craft_ui.dock-collapse", collapse_look);

/// The glyph of a column's collapse control: a double chevron that points where the column goes, towards the window
/// edge when it collapses to icons and away from it when it expands.
struct DoubleChevron {
    /// Whether it points to the right of the screen.
    right: bool,
    /// The stroke width.
    width: f32,
}

impl Glyph for DoubleChevron {
    fn paint(&self, painter: &egui::Painter, _: &crate::style::UiTokens, rect: Rect, color: Color32, _: bool) {
        let arm = rect.width().min(rect.height()) * ARM;
        let flip = if self.right { 1.0 } else { -1.0 };
        let centre = rect.center();
        let stroke = egui::Stroke::new(self.width, color);
        for step in [-0.5, 0.5] {
            let x = centre.x + flip * arm * PITCH * step;
            let points = [(-0.5, -1.0), (0.5, 0.0), (-0.5, 1.0)].map(|(dx, dy)| Pos2::new(x + flip * arm * REACH * dx, centre.y + arm * dy)).to_vec();
            painter.add(egui::Shape::line(points, stroke));
        }
    }
}

/// Whether the column's collapse control points to the right of the screen: towards the window edge when it collapses
/// to icons and away from it when it expands.
fn points_right(edge: DockEdge, expanded: bool, rtl: bool) -> bool {
    let at_left = (edge == DockEdge::Start) != rtl;
    at_left != expanded
}

/// The chevron's slot at the bar's outer end, the window edge the dock is on, as high and wide as the bar is high; and
/// the band in the rest of the bar.
pub(in crate::docking) fn bar_slots(bar: Rect, edge: DockEdge, rtl: bool) -> (Rect, Rect) {
    let side = bar.height().min(bar.width());
    if (edge == DockEdge::End) != rtl {
        let split = bar.right() - side;
        (Rect::from_x_y_ranges(split..=bar.right(), bar.y_range()), Rect::from_x_y_ranges(bar.left()..=split, bar.y_range()))
    } else {
        let split = bar.left() + side;
        (Rect::from_x_y_ranges(bar.left()..=split, bar.y_range()), Rect::from_x_y_ranges(split..=bar.right(), bar.y_range()))
    }
}

/// The name of the chevron of an expanded or an iconic column.
pub(in crate::docking) fn chevron_name(words: &DockWords, expanded: bool) -> Label {
    Label::from(if expanded { words.collapse_column.as_str() } else { words.expand_column.as_str() }.to_owned())
}

/// Paints the chevron part: a small ghost button with the double chevron pointing where the column goes.
pub(in crate::docking) fn paint_chevron(ui: &mut PaintUi<'_>, edge: DockEdge, expanded: bool, rtl: bool, name: Label) {
    let chevron = DoubleChevron { right: points_right(edge, expanded, rtl), width: ui.env.tokens.stroke.thin };
    ui.face(CHEVRON, IconButton::new(IconSpec::glyph(&chevron)).size(ControlSize::Sm).variant(Variant::Custom(COLLAPSE)).label(name));
}

/// The chevron's node.
pub(in crate::docking) fn describe_chevron(access: &mut Access, name: &Label) {
    access.node(CHEVRON, Role::BUTTON).name = Name::Text(name.as_str().to_owned());
}

/// Shows the band of the column bar `bar` and the column menu: a drag on `grip` moves the column, a double click
/// collapses or expands it, and a context request opens the column menu. The chevron at the bar's outer end is the
/// column's own part. Returns what the user did.
pub(in crate::docking) fn show_bar<P: Clone + Send + Sync + 'static>(
    ui: &mut Ui,
    body: &mut BodyCx<'_>,
    bar: Rect,
    expanded: bool,
    auto_collapse: bool,
    edge: DockEdge,
    grip: GroupGrip<P>,
) -> BarRecord {
    let words = body.words::<DockWords>();
    let menu_words = body.words::<MenuWords>();
    let rtl = crate::runtime::env::Env::of(ui).direction.is_rtl();
    let toggle = chevron_name(&words, expanded);
    let (_, band) = bar_slots(bar, edge, rtl);
    let mut record = BarRecord::default();
    if band.width() <= 0.0 {
        return record;
    }
    let held = grip.show_at(ui, band);
    if held.double_pressed {
        record.iconic = Some(expanded);
    }
    let rows = vec![
        PanelRow::Item(crate::contract::Entry::new(GroupItem::CollapseToIcons, toggle)),
        PanelRow::Item(
            crate::contract::Entry::new(GroupItem::AutoCollapse, Label::from(menu_words.panel_auto_collapse.as_str().to_owned()))
                .checked(auto_collapse),
        ),
    ];
    let picked = Menu::new(Label::from(words.panels.as_str().to_owned()))
        .opened_by(&held.response, Trigger::Context)
        .show(ui, |ui, _| PanelMenu::new(Vec::new(), rows.clone()).show(ui).picked);
    record.picked = picked.inner.flatten();
    record
}

/// The name of a column's bar: the labels of its panels.
pub(in crate::docking) fn bar_name<P>(column: &Column<P>, spec: &dyn Fn(&P) -> PanelSpec, template: &crate::runtime::env::WordTemplate) -> String {
    template.fill(&[("name", panel_names(column, spec).as_str())])
}

/// The labels of a column's panels, in order.
pub(super) fn panel_names<P>(column: &Column<P>, spec: &dyn Fn(&P) -> PanelSpec) -> String {
    let names: Vec<String> = column.groups.iter().flat_map(|group| group.panels.iter()).map(|panel| spec(panel).label.as_str().to_owned()).collect();
    names.join(", ")
}
