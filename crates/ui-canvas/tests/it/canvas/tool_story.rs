//! The live tool story: the tool and the document it edits outlive the pass, so a gesture over several frames edits
//! the story's document, and Reset puts the document back while the tool stays.

use craft_behavior::kurbo::Point;
use craft_behavior::{CancelReason, PointerButton};
use craft_ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolPath, ToolPointer};
use craft_ui::primitives::Draw;
use craft_ui::runtime::access::Role;
use craft_ui::testing::canvas_story::StoryDocument;
use craft_ui::testing::{Fixture, Story, StoryData, StoryOutput, ToolStory, pointer, tool_story, trial};
use egui::{Pos2, Vec2};

/// A tool that draws a line from the press to the release and reports each line it drew.
#[derive(Default)]
struct Liner {
    start: Option<Point>,
    drawn: Vec<(Point, Point)>,
}

impl CanvasTool for Liner {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(space) = cx.space() else { return ToolAction::Ignored };
        self.start = Some(space.view.to_doc(pointer.pos));
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if let (Some(start), Some(space)) = (self.start.take(), cx.space()) {
            self.drawn.push((start, space.view.to_doc(pointer.pos)));
        }
    }

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {
        self.start = None;
    }

    fn takes(&self, button: PointerButton, _: craft_keymap::Mods) -> bool {
        button == PointerButton::Primary
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("a story's line tool")
    }
}

type Lines = Vec<(Point, Point)>;

fn show(ui: &mut egui::Ui, data: &mut StoryData) -> StoryOutput {
    let picture = StoryDocument::STANDARD;
    tool_story(
        ui,
        data,
        ToolStory::new("Document", &picture),
        Liner::default,
        Lines::new(),
        |_, _, _, _, _| (),
        |run| {
            let _ = run.host.run(run.tool);
            run.document.append(&mut run.tool.drawn);
            StoryOutput::default()
        },
    )
}

fn lines(fixture: &Fixture) -> Lines {
    fixture.data().present::<Lines>("document").cloned().unwrap_or_default()
}

fn drag(fixture: &mut Fixture, from: Pos2, by: Vec2) {
    fixture.frame(vec![pointer::moved(from)]);
    fixture.frame(vec![pointer::press(from)]);
    for step in 1..=4 {
        fixture.frame(vec![pointer::moved(from + by * (step as f32 / 4.0))]);
    }
    fixture.frame(vec![pointer::release(from + by)]);
    fixture.frame(Vec::new());
}

#[trial]
fn a_drag_over_several_frames_edits_the_kept_document_and_reset_puts_it_back() {
    let mut fixture = Fixture::story(&Story::new("live tool", show));
    fixture.settle();
    let canvas = fixture.by_role(Role::CANVAS)[0].rect;
    let before = fixture.data().snapshot();

    drag(&mut fixture, canvas.center(), Vec2::new(60.0, 0.0));
    let drawn = lines(&fixture);
    assert_eq!(drawn.len(), 1, "a drag draws one line into the story's document: {drawn:?}");
    assert!(drawn[0].1.x > drawn[0].0.x, "the line runs from the press towards the release: {drawn:?}");

    fixture.idle(3);
    assert_eq!(lines(&fixture), drawn, "later passes keep the document");

    drag(&mut fixture, canvas.center() + Vec2::new(0.0, 40.0), Vec2::new(-50.0, 0.0));
    assert_eq!(lines(&fixture).len(), 2, "a second drag adds to the document");

    fixture.data().restore(&before);
    fixture.idle(1);
    assert!(lines(&fixture).is_empty(), "Reset puts the document back");

    drag(&mut fixture, canvas.center(), Vec2::new(0.0, 50.0));
    assert_eq!(lines(&fixture).len(), 1, "the kept tool draws again after Reset");
}
