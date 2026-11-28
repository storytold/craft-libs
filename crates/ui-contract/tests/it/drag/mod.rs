//! Drag and drop (8.4): the session, drop targets, the operation, cancel and the ghost layer's preview.

use craft_behavior::{DropOperation, PartKey, PressPhase};
use craft_host::target::ShellInfo;
use craft_ui::contract::{
    Common, Component, Container, ContainerProps, Cx, DragPreview, DropMark, DropTarget, MeasureCx, Mount, OsFiles, Overlay, PartsCx, Pointer as _,
    Props as _, Role, Show as _, ShowContainer as _, WindowDropTarget,
};
use craft_ui::runtime::InstallSpec;
use craft_ui::runtime::drag;
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, StyleCx};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::windows::FakeSystem;
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::windows::{FilesOver, Windows};
use egui::{Pos2, Rect, Response, Shape, Ui, Vec2, ViewportBuilder, ViewportId, ViewportInfo};
use std::cell::RefCell;
use std::sync::Arc;

const GRIP: PartKey = PartKey::new("grip");

const CHIP: Rect = Rect { min: Pos2::new(10.0, 10.0), max: Pos2::new(60.0, 40.0) };

const SLOT: Rect = Rect { min: Pos2::new(200.0, 10.0), max: Pos2::new(300.0, 60.0) };

const BADGE: &str = "3 files";

/// The name a named drag shows.
const NAME: &str = "Layers";

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
    /// Drops a window-wide target took.
    caught: usize,
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

/// Takes every drop and marks it with its mark.
struct Accepts(DropMark);

impl DropTarget<Payload> for Accepts {
    type Drop = (u32, DropOperation);

    fn accept(&self, payload: &Payload, _: Pos2, op: DropOperation) -> Option<Self::Drop> {
        Some((payload.0, op))
    }

    fn mark(&self, _: &Self::Drop) -> DropMark {
        self.0
    }
}

/// Takes drops of the payload, and marks them with `mark`.
#[derive(Component)]
struct Slot<'a> {
    #[common]
    common: Common,
    seen: &'a RefCell<Seen>,
    mark: DropMark,
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
        if let Some(drop) = cx.drop_target(&Accepts(self.mark)) {
            self.seen.borrow_mut().dropped.push(drop);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// A drag whose preview names its payload shows nothing away from every target, and its name once, inside the region a
/// target marks.
#[trial]
fn a_named_drag_shows_its_name_inside_a_region_mark_and_nothing_without_a_mark() {
    let seen = RefCell::new(Seen::default());
    let mut show = |ui: &mut Ui| {
        Chip { common: Common::new(), seen: &seen, preview: DragPreview::Named(Label::Static(NAME)) }.id_salt("chip").show_at(ui, CHIP);
        Slot { common: Common::new(), seen: &seen, mark: DropMark::region(SLOT) }.id_salt("slot").show_at(ui, SLOT);
    };
    let mut frames = Frames::new();
    let (from, open) = (CHIP.center(), Pos2::new(120.0, 200.0));
    frames.frame(vec![moved(from)], &mut show);
    frames.frame(vec![press(from)], &mut show);
    for step in 1..=4 {
        frames.frame(vec![moved(from + (open - from) * (step as f32 / 4.0))], &mut show);
    }
    assert!(drag::mark(&frames.ctx).is_none(), "no target takes the drag in open room");
    let away = texts(&frames.frame_shapes(&mut show));
    assert!(away.is_empty(), "nothing shows beside the pointer without a mark: {away:?}");
    frames.frame(vec![moved(SLOT.center())], &mut show);
    frames.idle(1, &mut show);
    assert!(matches!(drag::mark(&frames.ctx), Some(DropMark::Region { .. })), "the slot marks the region a drop takes");
    let over = texts(&frames.frame_shapes(&mut show));
    assert!(over.len() == 1 && SLOT.contains_rect(over[0]), "the name shows once, inside the region: {over:?}");
    frames.frame(vec![release(SLOT.center())], &mut show);
}

/// A drop lands where the button went up: the pointer moving on out of the target after the release, in the same
/// frame, does not take the drop away from it.
#[trial]
fn a_drop_lands_where_the_button_went_up_though_the_pointer_moves_on_in_its_frame() {
    let seen = RefCell::new(Seen::default());
    let mut show = |ui: &mut Ui| {
        Chip { common: Common::new(), seen: &seen, preview: DragPreview::None }.id_salt("chip").show_at(ui, CHIP);
        Slot { common: Common::new(), seen: &seen, mark: DropMark::region(SLOT) }.id_salt("slot").show_at(ui, SLOT);
    };
    let mut frames = Frames::new();
    let (from, to) = (CHIP.center(), SLOT.center());
    frames.frame(vec![moved(from)], &mut show);
    frames.frame(vec![press(from)], &mut show);
    for step in 1..=4 {
        frames.frame(vec![moved(from + (to - from) * (step as f32 / 4.0))], &mut show);
    }
    frames.idle(1, &mut show);
    let away = Pos2::new(120.0, 200.0);
    assert!(!SLOT.contains(away));
    frames.frame(vec![release(to), moved(away)], &mut show);
    frames.idle(2, &mut show);
    let dropped: Vec<u32> = seen.borrow().dropped.iter().map(|(payload, _)| *payload).collect();
    assert_eq!(dropped, [7], "the slot under the release takes the drop");
}

/// The bounds of the text shapes among `shapes`.
fn texts(shapes: &[Shape]) -> Vec<Rect> {
    shapes.iter().filter(|shape| matches!(shape, Shape::Text(_))).map(Shape::visual_bounding_rect).collect()
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
                Slot { common: Common::new(), seen: &seen, mark: DropMark::Into(SLOT) }.id_salt("slot").show_at(other_ui, SLOT);
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

/// Sb-9 (3): a drag out of an OS window whose window-wide target (a tab strip's tear-off) takes what no other target
/// does, released over a target in the main window in the frame after the pointer reached it: that target takes the
/// drop, not the window-wide one. Released where no window lies, the window-wide target takes it.
#[trial]
fn a_drag_out_of_an_os_window_drops_on_a_main_window_target_before_its_own_windows_target() {
    let run = |release_at: Pos2| {
        let seen = RefCell::new(Seen::default());
        let windows = Windows::new();
        let other = ViewportId::from_hash_of("other");
        let at = |min: Pos2, size: Vec2| ViewportInfo { inner_rect: Some(Rect::from_min_size(min, size)), ..ViewportInfo::default() };
        windows.root_info(at(Pos2::ZERO, Vec2::new(800.0, 600.0)));
        windows.child_info(at(Pos2::new(100.0, 100.0), Vec2::new(400.0, 300.0)));
        let frame = |events: Vec<egui::Event>| {
            windows.frame(vec![], events, |ui| {
                Slot { common: Common::new(), seen: &seen, mark: DropMark::Into(SLOT) }.id_salt("slot").show_at(ui, SLOT);
                ui.ctx().show_viewport_immediate(other, ViewportBuilder::default(), |other_ui, _| {
                    Chip { common: Common::new(), seen: &seen, preview: DragPreview::None }.id_salt("chip").show_at(other_ui, CHIP);
                    Catch { common: Common::new(), seen: &seen }.id_salt("catch").show_at(other_ui, SLOT);
                });
            });
        };
        let from = CHIP.center();
        for _ in 0..2 {
            frame(vec![]);
        }
        frame(vec![moved(from)]);
        frame(vec![press(from)]);
        for step in 1..=3 {
            frame(vec![moved(from + (release_at - from) * (step as f32 / 3.0))]);
        }
        frame(vec![release(release_at)]);
        for _ in 0..3 {
            frame(vec![]);
        }
        seen.into_inner()
    };
    // The slot's centre in the main window, (250, 35), lies at (150, -65) in the OS window at (100, 100).
    let over = run(Pos2::new(150.0, -65.0));
    assert_eq!(over.dropped, vec![(7, DropOperation::Move)], "the main window's target took the drop");
    assert_eq!(over.caught, 0, "the OS window's own window-wide target did not");
    // (150, 600) in the OS window lies below both windows.
    let away = run(Pos2::new(150.0, 600.0));
    assert!(away.dropped.is_empty(), "no target lies there");
    assert_eq!(away.caught, 1, "the window-wide target took the drop");
}

/// A window-wide target that takes every drop of the payload.
struct Anywhere;

impl WindowDropTarget<Payload> for Anywhere {
    type Drop = ();

    fn accept(&self, _: &Payload, _: DropOperation) -> Option<()> {
        Some(())
    }

    fn mark(&self, _: &()) -> DropMark {
        DropMark::Into(Rect::ZERO)
    }
}

/// Takes every drop of the payload over its whole window.
#[derive(Component)]
struct Catch<'a> {
    #[common]
    common: Common,
    seen: &'a RefCell<Seen>,
}

impl Component for Catch<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: CatchStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        SLOT.size()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if cx.window_drop_target(&Anywhere).is_some() {
            self.seen.borrow_mut().caught += 1;
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
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

/// A drop of files on the row under the pointer; it takes them under a modal layer when it holds `true`.
struct OnRow(bool);

impl DropTarget<OsFiles> for OnRow {
    type Drop = (usize, Vec<String>);

    fn accept(&self, files: &OsFiles, at: Pos2, _: DropOperation) -> Option<Self::Drop> {
        LIST.contains(at).then(|| (((at.y - LIST.min.y) / ROW) as usize, names(files)))
    }

    fn mark(&self, (row, _): &Self::Drop) -> DropMark {
        DropMark::Into(Rect::from_min_size(LIST.min + Vec2::new(0.0, *row as f32 * ROW), Vec2::new(LIST.width(), ROW)))
    }

    fn under_modal(&self) -> bool {
        self.0
    }
}

/// A drop of files anywhere in the window; it takes them under a modal layer when it holds `true`.
struct OnWindow(bool);

struct OnPositionalWindow;

impl WindowDropTarget<OsFiles> for OnPositionalWindow {
    type Drop = Vec<String>;

    fn requires_pointer(&self) -> bool {
        true
    }

    fn accept(&self, files: &OsFiles, _: DropOperation) -> Option<Self::Drop> {
        Some(names(files))
    }

    fn mark(&self, _: &Self::Drop) -> DropMark {
        DropMark::Into(WINDOW)
    }
}

impl WindowDropTarget<OsFiles> for OnWindow {
    type Drop = Vec<String>;

    fn accept(&self, files: &OsFiles, _: DropOperation) -> Option<Self::Drop> {
        Some(names(files))
    }

    fn mark(&self, _: &Self::Drop) -> DropMark {
        DropMark::Into(WINDOW)
    }

    fn under_modal(&self) -> bool {
        self.0
    }
}

/// A list of rows that takes files on a row, with the window as the target of every other drop.
#[derive(Component)]
struct FileList<'a> {
    #[common]
    common: Common,
    landed: &'a RefCell<Landed>,
    /// The targets take files under a modal layer.
    under_modal: bool,
}

#[derive(Component)]
struct RowOnly<'a> {
    #[common]
    common: Common,
    landed: &'a RefCell<Landed>,
}

#[derive(Component)]
struct WindowOnly<'a> {
    #[common]
    common: Common,
    landed: &'a RefCell<Landed>,
}

#[derive(Component)]
struct PositionalWindow<'a> {
    #[common]
    common: Common,
    landed: &'a RefCell<Landed>,
}

impl Component for PositionalWindow<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: PositionalWindowStyleKey) -> PlainStyle {
        PlainStyle::default()
    }
    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        WINDOW.size()
    }
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if let Some(files) = cx.window_drop_target(&OnPositionalWindow) {
            self.landed.borrow_mut().window.push(files);
        }
    }
    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

#[trial]
fn positional_window_waits_for_pointer_to_choose_a_row() {
    let landed = RefCell::new(Landed::default());
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        PositionalWindow { common: Common::new(), landed: &landed }.id_salt("window").show_at(ui, WINDOW);
        RowOnly { common: Common::new(), landed: &landed }.id_salt("rows").show_at(ui, LIST);
    };
    frames.idle(2, &mut show);
    frames.hover_files(&["a.png"]);
    frames.frame(vec![], &mut show);
    frames.idle(2, &mut show);
    frames.drop_files(&["a.png"]);
    frames.frame(vec![], &mut show);
    frames.idle(2, &mut show);
    assert!(landed.borrow().window.is_empty(), "the window waits while the pointer is unknown");
    frames.frame(vec![moved(LIST.center())], &mut show);
    frames.idle(3, &mut show);
    assert_eq!(landed.borrow().rows.len(), 1, "the row under the pointer takes the drop");
    assert!(landed.borrow().window.is_empty());
}

impl Component for WindowOnly<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: WindowOnlyStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        WINDOW.size()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if let Some(files) = cx.window_drop_target(&OnWindow(false)) {
            self.landed.borrow_mut().window.push(files);
        }
    }
    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Component for RowOnly<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: RowOnlyStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        LIST.size()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if let Some(row) = cx.drop_target(&OnRow(false)) {
            self.landed.borrow_mut().rows.push(row);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

#[trial]
fn file_drop_uses_the_target_under_the_final_pointer() {
    let landed = RefCell::new(Landed::default());
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        WindowOnly { common: Common::new(), landed: &landed }.id_salt("window").show_at(ui, WINDOW);
        RowOnly { common: Common::new(), landed: &landed }.id_salt("rows").show_at(ui, LIST);
    };
    frames.idle(2, &mut show);
    frames.hover_files(&["a.png"]);
    frames.frame(vec![moved(Pos2::new(400.0, 400.0))], &mut show);
    frames.idle(1, &mut show);
    frames.drop_files(&["a.png"]);
    frames.frame(vec![moved(LIST.center())], &mut show);
    frames.idle(2, &mut show);
    assert_eq!(landed.borrow().rows.len(), 1, "the positional target takes the drop");
    assert!(landed.borrow().window.is_empty(), "the previous window target must not take it");
}

#[trial]
fn file_drop_without_pointer_waits_for_its_target() {
    let landed = RefCell::new(Landed::default());
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        RowOnly { common: Common::new(), landed: &landed }.id_salt("files").show_at(ui, LIST);
    };
    frames.idle(2, &mut show);
    frames.hover_files(&["a.png"]);
    frames.frame(vec![], &mut show);
    frames.drop_files(&["a.png"]);
    frames.frame(vec![], &mut show);
    assert!(landed.borrow().rows.is_empty(), "the position is not known yet");
    frames.frame(vec![moved(LIST.center())], &mut show);
    frames.idle(2, &mut show);
    assert_eq!(landed.borrow().rows.len(), 1, "the drop reaches the row once its position arrives");
    assert!(landed.borrow().window.is_empty(), "the row takes the drop");
}

#[trial]
fn window_file_drop_without_hover_or_pointer_does_not_wait_for_pointer() {
    let landed = RefCell::new(Landed::default());
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        FileList { common: Common::new(), landed: &landed, under_modal: false }.id_salt("files").show_at(ui, LIST);
    };
    frames.idle(2, &mut show);
    frames.drop_files(&["a.png"]);
    frames.frame(vec![], &mut show);
    frames.idle(3, &mut show);
    assert_eq!(landed.borrow().window.len(), 1, "the window receives the drop without a pointer");
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
        if let Some(row) = cx.drop_target(&OnRow(self.under_modal)) {
            self.landed.borrow_mut().rows.push(row);
        }
        if let Some(files) = cx.window_drop_target(&OnWindow(self.under_modal)) {
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
        FileList { common: Common::new(), landed: &landed, under_modal: false }.id_salt("files").show_at(ui, LIST);
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

/// Files that hover in the pass right after a drop start a drag of their own, which their drop ends on the target under
/// the pointer, with or without the pointer moving.
#[trial]
fn files_hovering_right_after_a_drop_drag_and_drop_on_their_own() {
    let landed = RefCell::new(Landed::default());
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        FileList { common: Common::new(), landed: &landed, under_modal: false }.id_salt("files").show_at(ui, LIST);
    };
    let row = |n: usize| LIST.min + Vec2::new(LIST.width() / 2.0, (n as f32 + 0.5) * ROW);
    frames.idle(2, &mut show);
    for (file, events) in [("a.png", vec![moved(row(0))]), ("b.png", vec![moved(row(1))]), ("c.png", vec![])] {
        frames.hover_files(&[file]);
        frames.frame(events, &mut show);
        let held = drag::active::<OsFiles>(&frames.ctx).map(|files| names(&files));
        assert_eq!(held, Some(vec![file.to_owned()]), "the hovering files are the payload, not the last drop's");
        frames.drop_files(&[file]);
        frames.frame(vec![], &mut show);
    }
    frames.frame(vec![], &mut show);
    let on = |n: usize, file: &str| (n, vec![file.to_owned()]);
    assert_eq!(landed.borrow().rows, vec![on(0, "a.png"), on(1, "b.png"), on(1, "c.png")], "each drop lands on its row");
    assert!(drag::active::<OsFiles>(&frames.ctx).is_none(), "the last drop ends the drag");
}

/// A modal layer with an empty body, in the middle of the window.
#[derive(Component)]
#[component(container, unregistered)]
struct Modal {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl Component for Modal {
    const ROLE: Role = Role::GROUP;
    type Style = craft_ui_test_kit::contract::PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: ModalStyleKey) -> craft_ui_test_kit::contract::PlainStyle {
        craft_ui_test_kit::contract::PlainStyle::min(Vec2::new(120.0, 60.0))
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Modal {}

/// What a file list whose targets take files `under_modal` took of a file dropped on its first row and one dropped
/// beside it, while a modal layer is open.
fn landed_under_a_modal(under_modal: bool) -> Landed {
    let landed = RefCell::new(Landed::default());
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        FileList { common: Common::new(), landed: &landed, under_modal }.id_salt("files").show_at(ui, LIST);
        let props = ContainerProps::new().mount(Mount::Layer(Overlay::modal()));
        Modal { common: Common::new(), props }.id_salt("alert").show(ui, |_, _| {});
    };
    frames.idle(2, &mut show);
    for (file, at) in [("a.png", LIST.min + Vec2::new(LIST.width() / 2.0, ROW / 2.0)), ("b.png", Pos2::new(LIST.center().x, 500.0))] {
        frames.hover_files(&[file]);
        frames.frame(vec![moved(at)], &mut show);
        frames.frame(vec![], &mut show);
        frames.drop_files(&[file]);
        frames.frame(vec![], &mut show);
        frames.frame(vec![], &mut show);
    }
    landed.into_inner()
}

/// Tb-8 (4): a modal refuses OS files on the layers below it and on the window, unless the target says it takes them
/// there (`DropTarget::under_modal`), as VectorCraft placed files dropped under its alert.
#[trial]
fn files_dropped_under_a_modal_land_only_on_targets_that_take_them_there() {
    let refused = landed_under_a_modal(false);
    assert!(refused.rows.is_empty() && refused.window.is_empty(), "by default the modal refuses the drops");
    let taken = landed_under_a_modal(true);
    assert_eq!(taken.rows, vec![(0, vec!["a.png".to_owned()])], "the row below the modal takes its file");
    assert_eq!(taken.window, vec![vec!["b.png".to_owned()]], "the window takes the file dropped beside the row");
}

/// A drop of files anywhere in the window, with each file's content.
struct ReadOnWindow;

impl WindowDropTarget<OsFiles> for ReadOnWindow {
    type Drop = Vec<(String, Option<Vec<u8>>)>;

    fn accept(&self, files: &OsFiles, _: DropOperation) -> Option<Self::Drop> {
        Some(files.files.iter().map(|file| (file.name.clone(), file.bytes.as_deref().map(<[u8]>::to_vec))).collect())
    }

    fn mark(&self, _: &Self::Drop) -> DropMark {
        DropMark::Into(WINDOW)
    }
}

/// The drops a window took: the pass each landed in, with each file's name and content.
type Drops = Vec<(usize, Vec<(String, Option<Vec<u8>>)>)>;

/// A window that takes dropped files with their content, and the pass of its frame each drop landed in.
#[derive(Component)]
struct ReadDrop<'a> {
    #[common]
    common: Common,
    landed: &'a RefCell<Drops>,
    pass: usize,
}

impl Component for ReadDrop<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: ReadDropStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        LIST.size()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if let Some(files) = cx.window_drop_target(&ReadOnWindow) {
            self.landed.borrow_mut().push((self.pass, files));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// L-54: a web page reads a dropped file's bytes only after the drop, so the drop used to reach its target with no
/// bytes. The drag waits for the read: the drop frame delivers nothing, and the file arrives once, with its bytes, in
/// the first pass of the frame after its bytes came; a rerun pass of that frame and the next frame deliver nothing.
#[trial]
fn a_pathless_drop_is_delivered_once_its_bytes_are_read() {
    let landed = RefCell::new(Vec::new());
    let mut frames = Frames::new();
    let pass = std::cell::Cell::new(0);
    let mut show = |ui: &mut Ui| {
        ReadDrop { common: Common::new(), landed: &landed, pass: pass.get() }.id_salt("window").show_at(ui, LIST);
        pass.set(pass.get() + 1);
    };
    frames.idle(2, &mut show);
    frames.hover_files(&["scan.pdf"]);
    frames.idle(2, &mut show);
    assert!(drag::active::<OsFiles>(&frames.ctx).is_some(), "the hovering file is the active payload");

    frames.read_dropped("scan.pdf", b"%PDF-1.7");
    pass.set(0);
    frames.idle(1, &mut show);
    assert!(landed.borrow().is_empty(), "the drop frame delivers nothing while the file is read");
    assert!(drag::active::<OsFiles>(&frames.ctx).is_some(), "the drag waits for the read");

    pass.set(0);
    let mut first = true;
    let passes = frames.frame(Vec::new(), |ui| {
        if std::mem::take(&mut first) {
            craft_ui::runtime::frame::discard_pass(ui.ctx(), "test");
        }
        show(ui);
    });
    assert_eq!(passes, 2, "the frame after the read reruns");
    let delivered = vec![(0, vec![("scan.pdf".to_owned(), Some(b"%PDF-1.7".to_vec()))])];
    assert_eq!(*landed.borrow(), delivered, "the file arrives once, with its bytes, in the first pass after the read");

    frames.idle(2, &mut show);
    assert_eq!(*landed.borrow(), delivered, "later frames deliver nothing more");
    assert!(drag::active::<OsFiles>(&frames.ctx).is_none(), "the drop ends the drag");
}

/// A browser hands a dropped file over by a relative name, which locates no file, with its bytes. The drop used to keep
/// only that name as a path, so nothing could be opened; it lands with the bytes in the frame of the drop.
#[trial]
fn a_drop_by_relative_name_lands_with_its_bytes() {
    let landed = RefCell::new(Vec::new());
    let mut frames = Frames::new();
    let pass = std::cell::Cell::new(0);
    let mut show = |ui: &mut Ui| {
        ReadDrop { common: Common::new(), landed: &landed, pass: pass.get() }.id_salt("window").show_at(ui, LIST);
        pass.set(pass.get() + 1);
    };
    frames.idle(2, &mut show);
    frames.hover_files(&["scan.pdf"]);
    frames.idle(2, &mut show);

    frames.drop_with_bytes("scan.pdf", b"%PDF-1.7");
    pass.set(0);
    frames.idle(1, &mut show);
    assert_eq!(
        *landed.borrow(),
        vec![(0, vec![("scan.pdf".to_owned(), Some(b"%PDF-1.7".to_vec()))])],
        "the file lands once, with its bytes, in the drop frame"
    );
}

/// Where the window's inner rect sits on the desktop in the desktop-pointer trial.
const WINDOW_AT: Pos2 = Pos2::new(100.0, 50.0);

/// Files held over the window target the row under the desktop pointer, which the platform tells while the OS sends
/// the window no pointer events, and drop on it; every pass while they hover asks for the next. Where the platform
/// tells no pointer, the frame's pointer targets.
#[trial]
fn files_over_the_window_are_targeted_at_the_desktop_pointer_while_the_frame_has_none() {
    let landed = RefCell::new(Landed::default());
    let system = Arc::new(FakeSystem::new());
    let windows = Windows::with(InstallSpec::new(ShellInfo::default(), system.clone()));
    windows.root_info(ViewportInfo { inner_rect: Some(Rect::from_min_size(WINDOW_AT, WINDOW.size())), ..ViewportInfo::default() });
    let frame = |events: Vec<egui::Event>, hovered: Vec<egui::HoveredFile>, dropped: Vec<String>| {
        let _ = windows.frame_with_files(events, vec![], FilesOver::Root, hovered, dropped, |ui| {
            FileList { common: Common::new(), landed: &landed, under_modal: false }.id_salt("files").show_at(ui, LIST);
        });
        windows.ctx.has_requested_repaint()
    };
    let hover = || vec![egui::HoveredFile { path: Some("a.png".into()), ..egui::HoveredFile::default() }];
    let row = |n: usize| Rect::from_min_size(LIST.min + Vec2::new(0.0, n as f32 * ROW), Vec2::new(LIST.width(), ROW));
    for _ in 0..2 {
        frame(vec![], vec![], vec![]);
    }
    let over = WINDOW_AT + row(2).center().to_vec2();
    system.set_pointer(Some([over.x, over.y]));
    for pass in 0..3 {
        assert!(frame(vec![], hover(), vec![]), "frame {pass} with files over the window asks for the next");
    }
    assert!(drag::active::<OsFiles>(&windows.ctx).is_some(), "hovering files are the active payload");
    assert_eq!(drag::mark(&windows.ctx), Some(DropMark::Into(row(2))), "the row under the desktop pointer would take the files");
    frame(vec![], vec![], vec!["a.png".to_owned()]);
    frame(vec![], vec![], vec![]);
    assert_eq!(landed.borrow().rows, vec![(2, vec!["a.png".to_owned()])], "the files dropped on the row under the desktop pointer");
    assert!(landed.borrow().window.is_empty(), "the row took the drop, not the window");
    assert!(drag::active::<OsFiles>(&windows.ctx).is_none(), "the drop ends the drag");

    system.set_pointer(None);
    for _ in 0..2 {
        assert!(!frame(vec![], vec![], vec![]), "idle frames with no hovered files and no events request no repaint");
    }
    frame(vec![], hover(), vec![]);
    assert!(frame(vec![], hover(), vec![]), "hovered files that stay the same still ask for the next frame");
    let first = row(0).center();
    for _ in 0..2 {
        frame(vec![moved(first)], hover(), vec![]);
    }
    assert_eq!(drag::mark(&windows.ctx), Some(DropMark::Into(row(0))), "without a platform pointer the frame's pointer targets");
}
