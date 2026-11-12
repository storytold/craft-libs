//! Stories of painting symmetry: the menu, the guides in each mode, the handles, and Transform Symmetry run live.

use super::handles::{CENTRE, SYMMETRY};
use super::{
    Symmetry, SymmetryChoice, SymmetryEvent, SymmetryHandles, SymmetryMenu, SymmetryMode, SymmetryOverlay, SymmetryPathRow, SymmetryTool,
    SymmetryTransform, SymmetryWords,
};
use craft_behavior::kurbo::{Size, Vec2};
use craft_behavior::{CanvasView, EditEvent};
use craft_ui::bars::{Bar, BarKind};
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Drive, EditKind, Grip, StoryData, StoryOutput, ToolStory, story, tool_story};
use egui::Ui;

craft_ui::words! {
    /// The status line of the Transform Symmetry story.
    #[non_exhaustive]
    pub(crate) struct SymmetryStoryWords {
        /// The preview's centre and angle.
        centre: template "symmetryStory.centre" = "Center {x}, {y} · {angle}°";
        /// The centre line, then how the last session ended.
        ended: template "symmetryStory.ended" = "{text}. {ended}";
        /// The last session stored its symmetry.
        stored: "symmetryStory.stored" = "Stored";
        /// The last session was discarded.
        cancelled: "symmetryStory.cancelled" = "Cancelled";
    }
}

/// The document the canvas shows, whose page the guides run over.
const SHOWN: craft_ui::testing::canvas_story::StoryDocument = craft_ui::testing::canvas_story::StoryDocument::STANDARD;
/// The page the guides run over, in document units: the document's.
const PAGE: Size = Size::new(640.0, 400.0);

/// The symmetry of the stories: Dual Axis at the page centre, turned a little.
fn sample(mode: SymmetryMode) -> Symmetry {
    Symmetry { mode, center: SHOWN.centre(), rotation: 20.0 }
}

/// The count of axes the symmetry draws over the page.
fn axes(symmetry: &Symmetry) -> String {
    format!("{} axes on the page", symmetry.segments(PAGE).len())
}

/// A view that shows the page at 100 %, centred.
fn view(ui: &Ui, room: egui::Rect) -> CanvasView {
    SHOWN.view(room, ui.pixels_per_point(), 100.0)
}

#[story(SymmetryMenu<'static>, covers = [Default], drive = Drive::Never)]
fn symmetry_off(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let choice = SymmetryChoice::Off;
    SymmetryMenu::new("Symmetry", &choice).show(ui);
    StoryOutput::default()
}

#[story(SymmetryMenu<'static>, covers = [Default], drive = Drive::Never)]
fn symmetry_on(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let choice = SymmetryChoice::Mode(SymmetryMode::Dual);
    let paths = [SymmetryPathRow::new("work", "Work Path")];
    SymmetryMenu::new("Symmetry", &choice).paths(&paths).can_transform(true).show(ui);
    StoryOutput::default()
}

#[story(SymmetryOverlay<'static>, covers = [Default], drive = Drive::Never)]
fn dual_axis(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let symmetry = sample(SymmetryMode::Dual);
    let room = craft_ui::testing::stage(ui);
    let view = view(ui, room);
    Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
        craft_ui::testing::canvas_story::show_document(ui, body.rect, &SHOWN);
        SymmetryOverlay::new("Symmetry", &symmetry, PAGE).show_at(ui, body.rect);
    });
    StoryOutput::default().readout(axes(&symmetry))
}

#[story(SymmetryOverlay<'static>, covers = [Default], drive = Drive::Never)]
fn editing_guides(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let symmetry = sample(SymmetryMode::Vertical);
    let room = craft_ui::testing::stage(ui);
    let view = view(ui, room);
    Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
        craft_ui::testing::canvas_story::show_document(ui, body.rect, &SHOWN);
        SymmetryOverlay::new("Symmetry", &symmetry, PAGE).editing(true).show_at(ui, body.rect);
    });
    StoryOutput::default().readout(axes(&symmetry))
}

#[story(SymmetryHandles<'static>, covers = [Default], drive = Drive::Drag(Grip::part(SYMMETRY.at(CENTRE))))]
fn handles_on_a_canvas(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let symmetry = data.value("symmetry", sample(SymmetryMode::Vertical));
    let room = craft_ui::testing::stage(ui);
    let view = view(ui, room);
    let shown = Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
        craft_ui::testing::canvas_story::show_document(ui, body.rect, &SHOWN);
        let drawn = *symmetry;
        SymmetryOverlay::new("Symmetry", &drawn, PAGE).editing(true).show_at(ui, body.rect);
        SymmetryHandles::new("Symmetry", Bind::bound(&mut *symmetry), PAGE).show_at(ui, body.rect)
    });
    let output = shown.inner.map(|output| StoryOutput::from(output.edit)).unwrap_or_default();
    output.readout(axes(symmetry))
}

/// The modes by their menu names.
const MODES: [(&str, SymmetryMode); 4] = [
    ("Vertical", SymmetryMode::Vertical),
    ("Horizontal", SymmetryMode::Horizontal),
    ("Dual Axis", SymmetryMode::Dual),
    ("Diagonal", SymmetryMode::Diagonal),
];

/// The symmetry the story values describe: a mode, the centre moved from the page centre, and an angle.
fn described(data: &mut StoryData) -> Symmetry {
    let mode = *data.choice("mode", SymmetryMode::Vertical, &MODES);
    let x = *data.ranged("centre x", 0.0_f64, -300.0..=300.0);
    let y = *data.ranged("centre y", 0.0_f64, -200.0..=200.0);
    let angle = *data.ranged("angle", 0.0_f64, 0.0..=360.0);
    Symmetry::new(mode, SHOWN.centre() + Vec2::new(x, y), angle).unwrap_or(Symmetry { mode, center: SHOWN.centre(), rotation: 0.0 })
}

/// The guides of each mode over the page, at the centre or off it and turned as the story values say; Handles shows the
/// centre and the rotation handle, as while transforming.
#[story(SymmetryOverlay<'static>, covers = [], drive = Drive::Never)]
fn modes(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let symmetry = described(data);
    let handles = *data.value("handles", false);
    let room = craft_ui::testing::stage(ui);
    let view = view(ui, room);
    Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
        craft_ui::testing::canvas_story::show_document(ui, body.rect, &SHOWN);
        SymmetryOverlay::new("Symmetry", &symmetry, PAGE).editing(handles).show_at(ui, body.rect);
    });
    StoryOutput::default().readout(axes(&symmetry))
}

/// The open Transform Symmetry session, kept across passes, and how the last one ended.
struct SymmetryKept {
    session: SymmetryTransform,
    ended: Option<bool>,
}

/// Transform Symmetry live over the page, under its options bar: the handles move the centre and turn the guides, Enter
/// or ✓ stores the symmetry and reopens the session on it, Esc or ⊘ drops the session and reopens it on the stored
/// symmetry. The stored symmetry is the story's document; the story values set it anew.
#[story(SymmetryHandles<'static>, covers = [], drive = Drive::Never)]
fn transform_symmetry(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let env = Env::of(ui);
    let (words, units) = (env.words::<SymmetryStoryWords>(), env.units());
    let symmetry = env.words::<SymmetryWords>();
    let described = described(data);
    let set = data.note("described", described);
    if *set != described {
        *set = described;
        *data.value("document", described) = described;
    }
    let body = |ui: &mut Ui, at: egui::Rect, _: &CanvasView, kept: &mut SymmetryKept, stored: &Symmetry| {
        if kept.session.original() != *stored {
            kept.session = SymmetryTransform::begin(*stored);
        }
        let mut preview = kept.session.preview();
        SymmetryOverlay::new("Symmetry", &preview, PAGE).editing(true).show_at(ui, at);
        let shown = SymmetryHandles::new("Symmetry", Bind::bound(&mut preview), PAGE).session().show_at(ui, at);
        (shown.edit.events.iter().cloned().collect::<Vec<_>>(), shown.apply, shown.discard)
    };
    let start = || SymmetryKept { session: SymmetryTransform::begin(described), ended: None };
    let picture = SHOWN;
    // The bar shows before the canvas, as the app's options bar does: its commands reach the session on this pass.
    let bar =
        Bar::new(symmetry.transform.clone(), BarKind::Options).commit_cancel(symmetry.commit.clone(), symmetry.cancel.clone()).show(ui, |_, _| {});
    let (committed, cancelled) = (bar.output.committed, bar.output.cancelled);
    let story = ToolStory::new("Document", &picture);
    tool_story(ui, data, story, start, described, body, |run| {
        let (stored, kept) = (run.document, run.tool);
        let mut output = StoryOutput::default();
        let (edits, apply, discard): (Vec<EditEvent<Symmetry>>, bool, bool) = run.shown.unwrap_or_default();
        output.edits = edits.iter().filter_map(EditKind::of).collect();
        for edit in &edits {
            kept.session.apply(edit);
        }
        let mut tool = SymmetryTool::new(&kept.session, true);
        let _ = run.host.run(&mut tool);
        let mut events = tool.take_events();
        if apply || committed {
            events.push(kept.session.commit());
        }
        if discard || cancelled {
            events.push(kept.session.discard());
        }
        output.clicked = committed || cancelled;
        for event in events {
            match event {
                SymmetryEvent::Apply(symmetry) => {
                    *stored = symmetry;
                    kept.ended = Some(true);
                }
                _ => kept.ended = Some(false),
            }
            kept.session = SymmetryTransform::begin(*stored);
        }
        let preview = kept.session.preview();
        let number = |value: f64| units.number(value, 0..=1);
        let text = craft_i18n::fmt(
            words.centre.as_str(),
            &[("x", number(preview.center.x).as_str()), ("y", number(preview.center.y).as_str()), ("angle", number(preview.rotation).as_str())],
        );
        let ended = kept.ended.map(|stored| if stored { words.stored.as_str() } else { words.cancelled.as_str() });
        output
            .readout(ended.map_or_else(|| text.clone(), |ended| craft_i18n::fmt(words.ended.as_str(), &[("text", text.as_str()), ("ended", ended)])))
    })
}
