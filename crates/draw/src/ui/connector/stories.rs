//! Stories of the connector handles and the connector bar.

use craft_behavior::Mixed;
use craft_behavior::kurbo::Rect;
use craft_ui::bars::{Bar, BarKind};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::{Ui, vec2};

use super::{ConnectionTarget, ConnectorBar, ConnectorEnd, ConnectorHandles, ConnectorItem, ConnectorLine};
use crate::geometry::connector::{ConnectionSide, Routing};
use crate::ui::stroke::{StrokeChoice, StrokeChoiceRow};

fn targets() -> Vec<ConnectionTarget> {
    vec![
        ConnectionTarget::new(1, Rect::new(170.0, 190.0, 250.0, 250.0)).named("Rectangle 1"),
        ConnectionTarget::new(2, Rect::new(390.0, 150.0, 470.0, 210.0)).named("Rectangle 2"),
    ]
}

fn glued(target: u64, side: ConnectionSide) -> ConnectorEnd {
    ConnectorEnd::Glued { target, side, site: None, at: None }
}

/// Shows the handles `handles` builds and shows as the only canvas control of a canvas whose view has the document
/// origin at its top left. The canvas body runs more than once per pass, so `handles` builds them on every run.
fn on_canvas<R: Into<StoryOutput>>(ui: &mut Ui, mut handles: impl FnMut(&mut Ui, egui::Rect) -> R) -> StoryOutput {
    craft_ui::testing::canvas_story::on_canvas(ui, |ui, at, _| handles(ui, at))
}

#[story(ConnectorHandles<'static>, covers = [Default], drive = Drive::Drag(Grip::part(super::handles::END.at(0)).toward(vec2(0.0, 1.0))))]
fn connector_glued_at_both_ends(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let targets = targets();
    let item = data.value(
        "item",
        ConnectorItem {
            id: 9,
            from: ConnectorEnd::Glued { target: 1, side: ConnectionSide::Right, site: None, at: Some([1.0, 0.5]) },
            to: glued(2, ConnectionSide::Left),
            routing: Routing::Elbow,
            path: None,
        },
    );
    on_canvas(ui, |ui, at| ConnectorHandles::new("Connector", Bind::bound(&mut *item), &targets).show_at(ui, at))
}

fn caps() -> Vec<StrokeChoice> {
    vec![StrokeChoice::new("none", "None"), StrokeChoice::new("arrow", "Arrow"), StrokeChoice::new("dot", "Dot")]
}

#[story(ConnectorBar<'static>, covers = [Default], drive = Drive::Never)]
fn connector_bar_for_one_connector(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let choices = caps();
    Bar::new("Options", BarKind::Options)
        .show(ui, |ui, _| {
            let start = StrokeChoiceRow::new("start", "Start", &choices).selected(Mixed::Same("none"));
            let end = StrokeChoiceRow::new("end", "End", &choices).selected(Mixed::Same("arrow"));
            let _ = ConnectorBar::new(Mixed::Same(Routing::Elbow), Mixed::Same(ConnectorLine::default())).caps(start, end).show(ui);
        })
        .output
        .into()
}

#[story(ConnectorBar<'static>, covers = [Mixed], drive = Drive::Never)]
fn connector_bar_for_a_mixed_selection(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let choices = caps();
    Bar::new("Options", BarKind::Options)
        .show(ui, |ui, _| {
            let start = StrokeChoiceRow::new("start", "Start", &choices).selected(Mixed::Mixed);
            let end = StrokeChoiceRow::new("end", "End", &choices).selected(Mixed::Same("arrow"));
            let _ = ConnectorBar::new(Mixed::Mixed, Mixed::Mixed).caps(start, end).show(ui);
        })
        .output
        .into()
}
