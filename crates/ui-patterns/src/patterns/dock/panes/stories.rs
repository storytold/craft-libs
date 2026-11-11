//! Stories of `PaneArea` and `PaneFrame`.

use super::{PaneArea, PaneContent, PaneFrame, PaneSpec, PaneState};
use crate::contract::{BodyCx, PaneEdge, Show as _, ShowCollection as _, ShowContainer as _};
use crate::display::Text;
use crate::list::{List, ListRow};
use crate::runtime::env::Env;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::keyed;
use egui::{Ui, Vec2};

const LAYERS: usize = 40;

fn layer_names() -> Vec<String> {
    (1..=LAYERS).map(|n| format!("Layer {n}")).collect()
}

/// A list of layers: a body that is one collection.
fn layer_list(ui: &mut Ui, names: &[String]) {
    let source = keyed(names, |name: &String| name.clone());
    List::<String>::new("Layers").show_items(ui, &source, &mut |key, _| ListRow::new(key.clone()));
}

/// Two panes: the first on the start edge with a scrolling body, the second on the bottom edge with a list.
struct Panes {
    float: Vec2,
    side: f32,
    layers: Vec<String>,
}

const LIST_PANE: u32 = 1;

impl PaneContent<u32> for Panes {
    fn spec(&self, pane: &u32) -> PaneSpec {
        let edge = if *pane == 0 { PaneEdge::Start } else { PaneEdge::Bottom };
        let mut spec = PaneSpec::new(format!("Pane {pane}"), PaneState::docked(edge, self.side), self.float);
        spec.collection_body = *pane == LIST_PANE;
        spec
    }

    fn body(&mut self, pane: &u32, ui: &mut Ui, _: &mut BodyCx<'_>) {
        if *pane == LIST_PANE {
            layer_list(ui, &self.layers);
        } else {
            Text::new(format!("Contents of pane {pane}")).show(ui);
        }
    }
}

#[story(PaneArea<'static, u32>, covers = [], drive = Drive::Never)]
fn pane_area(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let size = &Env::of(ui).tokens.size;
    let mut content = Panes { float: Vec2::new(size.float.width, size.float.height), side: size.length.field, layers: layer_names() };
    let open = [0_u32, 1];
    PaneArea::new("Panes", &open, &mut content)
        .show(ui, |ui, _| {
            Text::new("Document").show(ui);
        })
        .into()
}

fn spec(ui: &Ui, title: &str, collection_body: bool) -> PaneSpec {
    let size = &Env::of(ui).tokens.size;
    let mut spec =
        PaneSpec::new(title.to_owned(), PaneState::docked(PaneEdge::Start, size.length.field), Vec2::new(size.float.width, size.float.height));
    spec.collection_body = collection_body;
    spec
}

#[story(PaneFrame<'static>, covers = [Default], drive = Drive::Never)]
fn pane_frame_scrolling_body(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let spec = spec(ui, "Styles", false);
    PaneFrame::new(&spec, PaneEdge::Start, spec.default_state.size)
        .show(ui, |ui, _| {
            for n in 1..=LAYERS {
                Text::new(format!("Style {n}")).show(ui);
            }
        })
        .output
        .into()
}

#[story(PaneFrame<'static>, covers = [], drive = Drive::Never)]
fn pane_frame_collection_body(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let spec = spec(ui, "Layers", true);
    let names = layer_names();
    PaneFrame::new(&spec, PaneEdge::Start, spec.default_state.size).show(ui, |ui, _| layer_list(ui, &names)).output.into()
}
