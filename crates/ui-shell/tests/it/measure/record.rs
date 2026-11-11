//! A container's sizes come from a measuring run: in a pass that measures, it measures its contents once, before its
//! real run, and the containers inside it reuse what that run recorded. A later pass with the same inputs takes the
//! kept measurement; the trials that count the runs of a measuring pass drop the kept measurements first
//! (`forget_measurements`).

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::sections::Section;
use craft_ui::testing::{Frames, forget_measurements, last_pass_measuring_runs, measuring_runs, trial};
use craft_ui::text::TextField;
use egui::{Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};

const SLACK: f32 = 0.5;

/// Six nested content-sized containers of three kinds: in a settled frame each one's contents run once in a measuring
/// run (an enclosing one's, reused by the rest) and once in the real run.
#[trial]
fn a_six_deep_nest_runs_each_container_once_measuring_and_once_real() {
    fn nest(ui: &mut Ui, depth: usize, calls: &RefCell<Vec<u32>>) {
        if depth == 0 {
            Text::new("Inside").show(ui);
            return;
        }
        calls.borrow_mut()[depth - 1] += 1;
        let inner = |ui: &mut Ui| nest(ui, depth - 1, calls);
        match depth % 3 {
            0 => {
                let _ = Stack::new().show(ui, |ui, _| inner(ui));
            }
            1 => {
                let _ = Row::new().show(ui, |ui, _| inner(ui));
            }
            _ => {
                let _ = Section::fixed("S").id_salt(("nest", depth)).show(ui, |ui, _| inner(ui));
            }
        }
    }
    let calls = RefCell::new(vec![0; 6]);
    let mut frames = Frames::new();
    frames.idle(3, |ui| nest(ui, 6, &calls));
    forget_measurements(&frames.ctx);
    calls.borrow_mut().fill(0);
    let passes = frames.frame(vec![], |ui| nest(ui, 6, &calls));
    let runs = measuring_runs(&frames.ctx);
    assert_eq!(passes, 1);
    assert!(runs.len() >= 6 && runs.iter().all(|(_, count)| *count == 1), "one measuring run per container: {runs:?}");
    // The outermost closure runs outside any container; every other one runs in one measuring run and one real run.
    assert_eq!(calls.borrow()[..5], [2; 5], "each container's contents run twice: {calls:?}");
}

/// A row that shrinks its children takes its kept measurement in settled frames as any body does, whether its
/// children fit (one sweep) or shrink (a second sweep at their widths): the real run shows them under the ids the
/// measuring run gave them, so it finds each in the kept transcript.
#[trial]
fn a_settled_row_that_shrinks_its_children_runs_one_pass_and_no_measuring_run() {
    for width in [900.0, 260.0] {
        let show = |ui: &mut Ui| {
            let at = Rect::from_min_size(ui.cursor().min, Vec2::new(width, 200.0));
            Stack::new().fill().show_at(ui, at, |ui, _| {
                Row::new().show(ui, |ui, _| {
                    let mut path = String::from("/home/studio/Documents/brochure-final.pdf");
                    let _ = TextField::new("Path", Bind::bound(&mut path)).show(ui);
                    let _ = Button::new("Choose File...").show(ui);
                    let _ = Button::new("Another").show(ui);
                });
            });
        };
        let mut frames = Frames::new();
        frames.idle(3, show);
        for _ in 0..3 {
            let passes = frames.frame(vec![], show);
            assert_eq!(passes, 1, "a settled frame at width {width} runs one pass");
            let runs = last_pass_measuring_runs(&frames.ctx);
            assert!(runs.is_empty(), "no body measures again at width {width}: {runs:?}");
        }
    }
}

/// A room change measures the containers again in that frame, in one pass: the room is an input the kept
/// measurement was taken under.
#[trial]
fn a_room_change_measures_again_in_its_frame() {
    let width = Cell::new(300.0);
    let seen = Cell::new((Rect::NOTHING, Rect::NOTHING));
    let show = |ui: &mut Ui| {
        let at = Rect::from_min_size(ui.cursor().min, Vec2::new(width.get(), 200.0));
        Stack::new().fill().show_at(ui, at, |ui, _| {
            let inner = Stack::new().show(ui, |ui, _| {
                Text::new("A line of text that wraps when the room is narrow enough").show(ui);
            });
            seen.set((at, inner.output.rect));
        });
    };
    let mut frames = Frames::new();
    frames.idle(4, show);
    width.set(120.0);
    let passes = frames.frame(vec![], show);
    let (at, inner) = seen.get();
    assert_eq!(passes, 1, "the new room is known before the run");
    assert!(!last_pass_measuring_runs(&frames.ctx).is_empty(), "the stacks measure at the new room");
    assert!(inner.width() <= at.width() + SLACK, "the inner stack fits the narrower room: {inner:?} in {at:?}");
    // A fresh measure under new inputs is not motion: the next frame takes what it found.
    let passes = frames.frame(vec![], show);
    assert_eq!(passes, 1, "the frame after");
    let runs = last_pass_measuring_runs(&frames.ctx);
    assert!(runs.is_empty(), "the frame after the room change measures nothing again: {runs:?}");
}

/// The widest cell of a content-sized grid narrows: in that frame the second column moves to the narrower first one.
/// The check asks each cell at the room its measuring run gave it, not at the track width the kept records give.
#[trial]
fn a_grid_narrows_in_the_frame_its_widest_cell_shrinks() {
    use craft_behavior::ColumnWidth;
    use craft_ui::layout_components::Grid;
    static COLUMNS: [ColumnWidth; 2] = [ColumnWidth::Content, ColumnWidth::Content];
    let first = Cell::new("A much longer first column label");
    let second = Cell::new(Rect::NOTHING);
    let show = |ui: &mut Ui| {
        Stack::new().show(ui, |ui, _| {
            Grid::new(&COLUMNS).show(ui, |ui, _| {
                Text::new(first.get()).show(ui);
                second.set(Text::new("Second").show(ui).rect);
                Text::new("Short").show(ui);
                Text::new("Again").show(ui);
            });
        });
    };
    let mut frames = Frames::new();
    frames.idle(4, show);
    let wide = second.get().min.x;
    first.set("Tiny");
    frames.frame(vec![], show);
    let narrow = second.get().min.x;
    assert!(narrow < wide - SLACK, "the second column moves left in the same frame: {wide} -> {narrow}");
    frames.idle(3, show);
    assert!((second.get().min.x - narrow).abs() <= SLACK, "and stays there: {narrow} -> {}", second.get().min.x);
}

/// A leaf whose measure reads an input besides its room, which the measuring run and the real run give differently,
/// keeps its kept call in settled frames: the real run asks it again under the input the call read. A menu row before
/// the level's first check mark read a narrower mark column in the measuring run's sweep; a bar's title, measured to be
/// arranged, read its column at the room the measuring run gave the bar's gap.
#[trial]
fn a_settled_leaf_that_read_an_input_besides_its_room_runs_one_pass() {
    use craft_ui::contract::Trigger;
    use craft_ui::menu::{Menu, MenuItem};
    use craft_ui::shell::{Caption, CaptionFamily, Captions, Frame, Title, TitleBar};
    let menu = |ui: &mut Ui| {
        let opener = Button::new("File").show(ui);
        let mut open = true;
        Menu::new("File").open(Bind::bound(&mut open)).opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
            MenuItem::new("New").shortcut("Ctrl+N").show(ui);
            MenuItem::new("Show Rulers").check(true).show(ui);
        });
    };
    let bar = |ui: &mut Ui| {
        let layout = craft_window::Layout { left: vec![], right: vec![Caption::Minimize, Caption::Maximize, Caption::Close] };
        let frame = Frame::Integrated(Captions { family: CaptionFamily::Windows, layout });
        let _ = TitleBar::new().frame(frame).show_titled(
            ui,
            Title::new("Landscape.psd"),
            |ui, _| {
                Text::new("File  Edit").show(ui);
            },
            |ui, _| {
                Text::new("Share").show(ui);
            },
        );
    };
    let shows: [(&str, &dyn Fn(&mut Ui)); 2] = [("menu", &menu), ("title bar", &bar)];
    for (name, show) in shows {
        let mut frames = Frames::new();
        frames.idle(4, show);
        // A kept call that mismatches makes every other settled frame run two passes.
        for frame in 0..4 {
            assert_eq!(frames.frame(vec![], show), 1, "{name}: settled frame {frame} runs one pass");
        }
    }
}
