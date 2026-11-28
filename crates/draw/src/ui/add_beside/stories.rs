//! Stories of the "+" buttons: an artboard on a pasteboard with the buttons around it, one side kept clear by a
//! neighbour's rect, and the connected-shape form.

use super::{AddBesideHandles, AddBesideWords, AddSide};
use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Rect as DocRect, Vec2};
use craft_ui::canvas::{Canvas, PageFrame, Pasteboard};
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Rect, Ui};

craft_ui::words! {
    /// The status line of the "+" button stories.
    #[non_exhaustive]
    pub(crate) struct AddBesideStoryWords {
        /// A button was clicked.
        clicked: template "addBesideStory.clicked" = "Clicked: {side}";
        /// A button was clicked with Alt: add with contents.
        copy: template "addBesideStory.copy" = "Clicked: {side}, with a copy";
        /// The button under the pointer or focused.
        hovered: template "addBesideStory.hovered" = "Pointing at: {side}";
        /// No button clicked so far.
        none: "addBesideStory.none" = "No button pressed";
    }
}

/// The artboard the buttons surround, in document units: in the middle of the story document.
fn artboard() -> DocRect {
    DocRect::new(220.0, 130.0, 420.0, 270.0)
}

/// The gap between the artboard and the board a button adds, in document units.
const GAP: f64 = 40.0;

/// The board each side's button would add: the artboard's size, `GAP` beyond that side.
fn ghosts() -> [(AddSide, DocRect); 4] {
    let board = artboard();
    AddSide::ALL.map(|side| {
        let by = match side {
            AddSide::Top => Vec2::new(0.0, -(board.height() + GAP)),
            AddSide::Right => Vec2::new(board.width() + GAP, 0.0),
            AddSide::Bottom => Vec2::new(0.0, board.height() + GAP),
            _ => Vec2::new(-(board.width() + GAP), 0.0),
        };
        (side, board + by)
    })
}

/// The view of the canvas the story shows: the story document at 100 %, centred.
fn view(ui: &Ui, rect: Rect) -> CanvasView {
    craft_ui::testing::canvas_story::StoryDocument::STANDARD.view(rect, ui.pixels_per_point(), 100.0)
}

/// The name of the button on `side`.
fn side_name(words: &AddBesideWords, side: AddSide) -> &str {
    match side {
        AddSide::Top => words.above.as_str(),
        AddSide::Right => words.right.as_str(),
        AddSide::Bottom => words.below.as_str(),
        _ => words.left.as_str(),
    }
}

/// Shows the artboard with `handles` over it on a canvas. The readout names the button pointed at, else the last
/// click.
fn on_canvas<'a>(ui: &mut Ui, data: &mut StoryData, rect: Rect, handles: impl Fn(Rect) -> AddBesideHandles<'a>) -> StoryOutput {
    let env = Env::of(ui);
    let (words, sides) = (env.words::<AddBesideStoryWords>(), env.words::<AddBesideWords>());
    let board = [PageFrame::new(artboard()).name("Artboard 1").active(true)];
    let view = view(ui, rect);
    let shown = Canvas::new("Document").view(view).show_at(ui, rect, |ui, body| {
        craft_ui::testing::canvas_story::show_document(ui, body.rect, &craft_ui::testing::canvas_story::StoryDocument::STANDARD);
        let name_clicked = Pasteboard::new("Pasteboard", &board).show_at(ui, body.rect).name_clicked.is_some();
        (handles(body.rect).show_at(ui, body.rect), name_clicked)
    });
    let last = data.note("clicked", None::<(AddSide, bool)>);
    // A measuring run shows no canvas body; the readout still shows, so the card keeps its line.
    let (facts, hovered) = match shown.inner {
        Some((output, name_clicked)) => {
            if let Some(side) = output.added {
                *last = Some((side, output.copy));
            }
            let hovered = output.hovered;
            // The artboard's name is a button too: its click is a click of the story.
            let mut facts = StoryOutput::from(output);
            facts.clicked |= name_clicked;
            (facts, hovered)
        }
        None => (StoryOutput::default(), None),
    };
    let named = |template: &str, side: AddSide| craft_i18n::fmt(template, &[("side", side_name(&sides, side))]);
    let readout = match (hovered, *last) {
        (Some(side), _) => named(words.hovered.as_str(), side),
        (None, Some((side, copy))) => named(if copy { words.copy.as_str() } else { words.clicked.as_str() }, side),
        (None, None) => words.none.as_str().to_owned(),
    };
    facts.readout(readout)
}

#[story(AddBesideHandles<'static>, covers = [Default], drive = Drive::Never)]
fn add_beside(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rect = craft_ui::testing::stage(ui);
    let view = view(ui, rect);
    let neighbour = Rect::from_two_pos(
        view.to_screen(Point::new(artboard().x1 + 16.0, artboard().y0)),
        view.to_screen(Point::new(artboard().x1 + 80.0, artboard().y1)),
    );
    let (clear, ghosts) = ([neighbour], ghosts());
    on_canvas(ui, data, rect, |_| AddBesideHandles::new("Add beside", artboard()).clear_of(&clear).ghosts(&ghosts))
}

#[story(AddBesideHandles<'static>, covers = [Default], drive = Drive::Never)]
fn add_beside_connecting(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rect = craft_ui::testing::stage(ui);
    let ghosts = ghosts();
    on_canvas(ui, data, rect, |_| AddBesideHandles::new("Add beside", artboard()).connecting().ghosts(&ghosts))
}
