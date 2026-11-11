//! Drag and drop (8.4): the session, drop targets, the operation, cancel and the ghost layer's preview.

use craft_behavior::{DropOperation, PartKey, PressPhase};
use craft_ui::contract::{
    Common, Component, Cx, DragPreview, DropMark, DropTarget, MeasureCx, OsFiles, PartsCx, Pointer as _, Props as _, Role, Show as _,
    WindowDropTarget,
};
use craft_ui::runtime::drag;
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, StyleCx};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::windows::Windows;
use egui::{Pos2, Rect, Response, Ui, Vec2, ViewportBuilder, ViewportId, ViewportInfo};
use std::cell::RefCell;

const GRIP: PartKey = PartKey::new("grip");

const CHIP: Rect = Rect { min: Pos2::new(10.0, 10.0), max: Pos2::new(60.0, 40.0) };

const SLOT: Rect = Rect { min: Pos2::new(200.0, 10.0), max: Pos2::new(300.0, 60.0) };

const BADGE: &str = "3 files";

#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PlainStyle {
    #[style(metrics)]
    metrics: craft_behavior::Metrics,
}

/// What the payload is: the number the source drags.
#[derive(Clone, Debug, PartialEq)]
struct Payload(u32);

/// What the components saw.
#[derive(Default)]
struct Seen {
    dropped: Vec<(u32, DropOperation)>,
    ended: Vec<DropOperation>,
}

/// Starts a drag from its grip and reads how the drop went.
#[derive(Component)]
struct Chip<'a> {
    #[common]
    common: Common,
    seen: &'a RefCell<Seen>,
    preview: DragPreview,
}

impl Component for Chip<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: ChipStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        CHIP.size()
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.part(GRIP, rect).sense(craft_behavior::Sense::DRAG).pointer_only("the grip is dragged with the pointer");
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if matches!(cx.part(GRIP).press.phase, PressPhase::DragStart { .. }) {
            cx.start_drag(Payload(7), self.preview.clone());
        }
        if let Some(op) = cx.drag_ended::<Payload>() {
            self.seen.borrow_mut().ended.push(op);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

struct Accepts;

impl DropTarget<Payload> for Accepts {
    type Drop = (u32, DropOperation);

    fn accept(&self, payload: &Payload, _: Pos2, op: DropOperation) -> Option<Self::Drop> {
        Some((payload.0, op))
    }

    fn mark(&self, _: &Self::Drop) -> DropMark {
        DropMark::Into(SLOT)
    }
}

/// Takes drops of the payload.
#[derive(Component)]
struct Slot<'a> {
    #[common]
    common: Common,
    seen: &'a RefCell<Seen>,
}

impl Component for Slot<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: SlotStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        SLOT.size()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if let Some(drop) = cx.drop_target(&Accepts) {
            self.seen.borrow_mut().dropped.push(drop);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// A drag that starts in one window and ends over a target in another: the session maps the pointer to desktop
/// space, the other window's target takes the drop one frame after the release, and the source learns the
/// operation in the frame after that.
#[trial]
fn a_drag_crosses_windows_and_the_target_in_the_other_window_takes_the_drop() {
    let seen = RefCell::new(Seen::default());
    let windows = Windows::new();
    let other = ViewportId::from_hash_of("other");
    let at = |min: Pos2, size: Vec2| ViewportInfo { inner_rect: Some(Rect::from_min_size(min, size)), ..ViewportInfo::default() };
    windows.root_info(at(Pos2::ZERO, Vec2::new(800.0, 600.0)));
    windows.child_info(at(Pos2::new(100.0, 100.0), Vec2::new(400.0, 300.0)));
    let frame = |events: Vec<egui::Event>| {
        windows.frame(events, vec![], |ui| {
            Chip { common: Common::new(), seen: &seen, preview: DragPreview::Badge(Label::Static(BADGE)) }.id_salt("chip").show_at(ui, CHIP);
            ui.ctx().show_viewport_immediate(other, ViewportBuilder::default(), |other_ui, _| {
                Slot { common: Common::new(), seen: &seen }.id_salt("slot").show_at(other_ui, SLOT);
            });
        });
    };
    // The slot's centre in the other window is (250, 35); that window sits at (100, 100) on the desktop, so the
    // source window's pointer is at (350, 135).
    let over = Pos2::new(350.0, 135.0);
    let from = CHIP.center();
    for _ in 0..2 {
        frame(vec![]);
    }
    frame(vec![moved(from)]);
    frame(vec![press(from)]);
    for to in [from + Vec2::new(30.0, 0.0), from + Vec2::new(100.0, 0.0), over] {
        frame(vec![moved(to)]);
    }
    frame(vec![]);
    assert!(seen.borrow().dropped.is_empty(), "nothing drops while the button is down");
    frame(vec![release(over)]);
    for _ in 0..3 {
        frame(vec![]);
    }
    let seen = seen.borrow();
    assert_eq!(seen.dropped, vec![(7, DropOperation::Move)], "the other window's target took the drop once");
    assert_eq!(seen.ended, vec![DropOperation::Move], "the source saw the operation once");
}

const LIST: Rect = Rect { min: Pos2::new(20.0, 100.0), max: Pos2::new(220.0, 220.0) };

const ROW: f32 = 30.0;

const WINDOW: Rect = Rect { min: Pos2::ZERO, max: Pos2::new(800.0, 600.0) };

/// Where files landed: the row of the list, or the window.
#[derive(Default)]
struct Landed {
    rows: Vec<(usize, Vec<String>)>,
    window: Vec<Vec<String>>,
}

fn names(files: &OsFiles) -> Vec<String> {
    files.files.iter().map(|file| file.name.clone()).collect()
}

/// A drop of files on the row under the pointer.
struct OnRow;

impl DropTarget<OsFiles> for OnRow {
    type Drop = (usize, Vec<String>);

    fn accept(&self, files: &OsFiles, at: Pos2, _: DropOperation) -> Option<Self::Drop> {
        LIST.contains(at).then(|| (((at.y - LIST.min.y) / ROW) as usize, names(files)))
    }

    fn mark(&self, (row, _): &Self::Drop) -> DropMark {
        DropMark::Into(Rect::from_min_size(LIST.min + Vec2::new(0.0, *row as f32 * ROW), Vec2::new(LIST.width(), ROW)))
    }
}

/// A drop of files anywhere in the window.
struct OnWindow;

impl WindowDropTarget<OsFiles> for OnWindow {
    type Drop = Vec<String>;

    fn accept(&self, files: &OsFiles, _: DropOperation) -> Option<Self::Drop> {
        Some(names(files))
    }

    fn mark(&self, _: &Self::Drop) -> DropMark {
        DropMark::Into(WINDOW)
    }
}

/// A list of rows that takes files on a row, with the window as the target of every other drop.
#[derive(Component)]
struct FileList<'a> {
    #[common]
    common: Common,
    landed: &'a RefCell<Landed>,
}

impl Component for FileList<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: FileListStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        LIST.size()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if let Some(row) = cx.drop_target(&OnRow) {
            self.landed.borrow_mut().rows.push(row);
        }
        if let Some(files) = cx.window_drop_target(&OnWindow) {
            self.landed.borrow_mut().window.push(files);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

#[trial]
fn files_that_leave_the_window_end_the_drag_without_a_drop() {
    let landed = RefCell::new(Landed::default());
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        FileList { common: Common::new(), landed: &landed }.id_salt("files").show_at(ui, LIST);
    };
    frames.idle(2, &mut show);
    frames.hover_files(&["a.png"]);
    frames.frame(vec![moved(LIST.center())], &mut show);
    assert!(drag::active::<OsFiles>(&frames.ctx).is_some(), "hovering files are the active payload");
    frames.leave_files();
    frames.frame(vec![moved(LIST.center())], &mut show);
    assert!(drag::active::<OsFiles>(&frames.ctx).is_none(), "the drag ends when the files leave");
    assert!(landed.borrow().rows.is_empty() && landed.borrow().window.is_empty(), "nothing landed");
}
