//! Runs the spec 22.7 scenarios on the foundation's `craft-ui` and writes `baseline/foundation.json`.
//!
//! A scenario whose component the families have not delivered yet runs on the smallest stand-in in `kit`,
//! marked `F8-MERGE: <family>`; the combiner replaces the stand-in with the real component.

mod kit;

use std::path::PathBuf;
use std::sync::Arc;

use craft_behavior::kurbo::Point;
use craft_behavior::{Axis, CanvasView, ColumnKey, ColumnWidth, DisplayOrder, ItemSource, PixelSize, RulerUnits, Selection};
use craft_host::target::ShellInfo;
use craft_ui::canvas::{
    Canvas, Guide, GuideKey, HandTool, NavigationOptions, PixelRect, RasterDisplay, RasterSource, RulerText, Rulers, ToolHost, pan,
};
use craft_ui::canvas_controls::Guides;
use craft_ui::contract::ColumnSpec;
use craft_ui::contract::{
    Bind, Collapse, ContainerProps, Mount, Overlay, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing,
};
use craft_ui::dialog::Dialog;
use craft_ui::layout::ColumnGroup;
use craft_ui::list::{List, ListRow};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::patterns::form_dialogs::{ExportAsDialog, ExportOptions, ExportPreview, PreviewState, SizeEstimate};
use craft_ui::primitives::{Revision, TextureCache};
use craft_ui::runtime::env::RootEnv;
use craft_ui::runtime::env::UnitKind;
use craft_ui::runtime::frame::{FrameInput, set_input};
use craft_ui::runtime::{InstallSpec, install};
use craft_ui::table::Table;
use craft_ui::tabs::{Tab, TabKind, TabStrip};
use craft_ui::text::{NumberField, TextField};
use craft_ui::tile_grid::{Tile, TileGrid};
use craft_ui_bench::{CountingAlloc, Library, Scene, SceneBuild, gate_failures, print_table, read_json, run_all, write_json};
use craft_window::system::NoWindows;
use egui::load::SizedTexture;
use egui::{Color32, ColorImage, Context, Event, Modifiers, MouseWheelUnit, Pos2, Ui, Vec2, vec2};
use kit::{BenchCanvas, BenchList, Block, Count, Panel, ROW_H, Row, RowKind};

#[global_allocator]
static ALLOCATOR: CountingAlloc = CountingAlloc;

/// The app's root env, built once: the runtime caches the resolved tokens by the identity of its sizes.
struct Foundation {
    root: RootEnv,
}

impl Library for Foundation {
    fn name(&self) -> &'static str {
        "foundation"
    }

    fn install(&self, ctx: &Context) {
        if let Err(error) = install(ctx, InstallSpec::new(ShellInfo::default(), Arc::new(NoWindows))) {
            panic!("the foundation runtime did not install: {error:?}");
        }
        ctx.set_os(egui::os::OperatingSystem::Nix);
        ctx.all_styles_mut(|style| style.scroll_animation = egui::style::ScrollAnimation::none());
    }

    fn before_frame(&self, ctx: &Context) {
        set_input(ctx, FrameInput::new(self.root.clone()));
    }

    fn build(&self, name: &str) -> SceneBuild {
        let (scene, equivalent): (Box<dyn Scene>, &'static str) = match name {
            "inspector-500-fields" => (
                Box::new(InspectorScene::new()),
                // F8-MERGE: family (Field row): the field's own side label stands in for the `Field` row.
                "500 rows alternating NumberField and TextField, each with its side label (no Field row component yet)",
            ),
            "virtual-list-10000" => (Box::new(ListScene::new(Selected::None)), "List over 10,000 ListRow rows, one selected row"),
            "sheet-50x200" => (Box::new(SheetScene), "BenchList of 200 rows, each with 50 cell parts that paint their text"),
            // F8-MERGE: canvas: the canvas component and its declared controls.
            "canvas-50-controls" => (Box::new(CanvasScene), "a leaf with 50 control parts painted as knobs"),
            "menu-40-rows" => (Box::new(MenuScene), "a Menu open from the first frame over 40 MenuItem rows"),
            "table-1000x5" => (Box::new(TableScene), "a Table of 1,000 rows and 5 columns, text in every cell"),
            "tile-grid-500" => (Box::new(TileGridScene), "a TileGrid of 500 Tiles"),
            "text-field-100kb" => (Box::new(TextScene::new()), "TextField::lines(30) bound with Bind::bound over a 100 KB String"),
            "list-10000-selected" => (Box::new(ListScene::new(Selected::All)), "List over 10,000 ListRow rows, all 10,000 keys in the Selection"),
            "collapse-in-content-sized-panel" => {
                (Box::new(CollapseScene::new()), "a content-sized, animated-collapse container whose open state the scene toggles")
            }
            "auto-height-list-scroll-10000" => (
                Box::new(AutoHeightScene::new()),
                "List over 10,000 ListRow rows of one to four row heights (detail line and wrapped lines on three in four), mouse wheel every frame",
            ),
            "second-viewport" => (Box::new(ViewportScene), "a container with Mount::viewport (the fallback layer, as under an embedded backend)"),
            "dialog-20-fields" => (Box::new(DialogScene::new()), "a modal Dialog open from the first frame over 10 NumberFields and 10 TextFields"),
            "tab-strip-60" => (Box::new(TabStripScene), "a TabStrip of 60 TabKind::Document tabs that overflow, tab 30 shown"),
            "canvas-view-4k" => (
                Box::new(CanvasViewScene::new()),
                "a Canvas with a rotated CanvasView, RasterDisplay over a 4096 x 4096 RasterSource, Rulers, 40 Guides and the Hand tool, panning a step each frame",
            ),
            "param-dialog-export" => {
                (Box::new(ExportDialogScene::default()), "ExportAsDialog open with its format form and a PreviewPane in PreviewState::Ready")
            }
            "readout-and-content-column" => (
                Box::new(ReadoutScene),
                // F8-MERGE: layout: the grid's `Content` column and the status bar's end slot.
                "two labelled TextFields in a ColumnGroup whose label text changes width every frame, and an end-aligned readout block",
            ),
            _ => return SceneBuild::Missing { reason: "the scenario list names a scenario this binary does not know" },
        };
        SceneBuild::Scene { scene, equivalent }
    }
}

struct InspectorScene {
    values: Vec<Option<f64>>,
    texts: Vec<String>,
}

impl InspectorScene {
    fn new() -> Self {
        Self { values: (0..500).map(|i| Some(f64::from(i))).collect(), texts: (0..500).map(|i| format!("Value {i}")).collect() }
    }
}

impl Scene for InspectorScene {
    fn size(&self) -> Vec2 {
        vec2(360.0, 900.0)
    }

    fn show(&mut self, ui: &mut Ui, _frame: u64) {
        let Self { values, texts } = self;
        for (i, (value, text)) in values.iter_mut().zip(texts.iter_mut()).enumerate() {
            let label = format!("Property {i}:");
            if i % 2 == 0 {
                let _ = NumberField::new(label, Bind::bound(value)).id_salt(i).show(ui);
            } else {
                let _ = TextField::new(label, Bind::bound(text)).id_salt(i).show(ui);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Selected {
    None,
    All,
}

struct ListScene {
    source: Count,
    selection: Selection<usize>,
}

impl ListScene {
    fn new(selected: Selected) -> Self {
        let selection = match selected {
            Selected::None => Selection::new(vec![0], Some(0), Some(0)),
            Selected::All => Selection::new((0..10_000).collect(), Some(0), Some(0)),
        };
        Self { source: Count(10_000), selection }
    }
}

impl Scene for ListScene {
    fn show(&mut self, ui: &mut Ui, _frame: u64) {
        let list = List::new("Items").select(SelectMode::Multi).selection(Bind::bound(&mut self.selection));
        let _ = list.id_salt("list").show_items(ui, &self.source, &mut |&key, _| ListRow::new(format!("Item {key}")));
    }
}

const SHEET_COLUMNS: usize = 50;
const SHEET_ROWS: usize = 200;

struct SheetScene;

impl Scene for SheetScene {
    fn size(&self) -> Vec2 {
        vec2(SHEET_COLUMNS as f32 * kit::CELL_W + 40.0, SHEET_ROWS as f32 * ROW_H + 100.0)
    }

    fn show(&mut self, ui: &mut Ui, _frame: u64) {
        let rows = Count(SHEET_ROWS);
        let _ = BenchList::new(SelectMode::Single)
            .id_salt("sheet")
            .show_items(ui, &rows, &mut |&row, _| Row::new(RowKind::Cells { row, cells: SHEET_COLUMNS }));
    }
}

struct CanvasScene;

impl Scene for CanvasScene {
    fn size(&self) -> Vec2 {
        vec2(900.0, 700.0)
    }

    fn show(&mut self, ui: &mut Ui, _frame: u64) {
        let _ = BenchCanvas::new(vec2(840.0, 640.0), 50).id_salt("canvas").show(ui);
    }
}

struct MenuScene;

impl Scene for MenuScene {
    fn size(&self) -> Vec2 {
        vec2(800.0, 1400.0)
    }

    fn show(&mut self, ui: &mut Ui, _frame: u64) {
        let mut open = true;
        let _ = Menu::new("Items").open(Bind::bound(&mut open)).id_salt("menu").show(ui, |ui, _| {
            for key in 0..40 {
                let _ = MenuItem::new(format!("Item {key}")).id_salt(key).show(ui);
            }
        });
    }
}

const TABLE_ROWS: usize = 1_000;
/// The five columns of the table scenario: key and header.
const TABLE_COLUMNS: [(&str, &str); 5] = [("name", "Name"), ("kind", "Kind"), ("size", "Size"), ("pages", "Pages"), ("owner", "Owner")];

struct TableScene;

impl Scene for TableScene {
    fn size(&self) -> Vec2 {
        vec2(900.0, 700.0)
    }

    fn show(&mut self, ui: &mut Ui, _frame: u64) {
        let columns: Vec<ColumnSpec> =
            TABLE_COLUMNS.iter().map(|&(key, header)| ColumnSpec::new(ColumnKey::new(key), header, ColumnWidth::fill(1.0)).resizable()).collect();
        let rows = Count(TABLE_ROWS);
        let _ = Table::<usize>::new("Files", columns).id_salt("table").show_items(ui, &rows, &mut |&key, cx| {
            let mut row = ListRow::new(format!("File {key}")).cells(cx.cells());
            for (at, &(column, _)) in TABLE_COLUMNS.iter().enumerate().skip(1) {
                row = row.cell(ColumnKey::new(column), format!("{} {key}", at * 10));
            }
            row
        });
    }
}

struct TileGridScene;

impl Scene for TileGridScene {
    fn size(&self) -> Vec2 {
        vec2(900.0, 700.0)
    }

    fn show(&mut self, ui: &mut Ui, _frame: u64) {
        let tiles = Count(500);
        let _ = TileGrid::<usize>::new("Photos").id_salt("grid").show_items(ui, &tiles, &mut |&key, _| Tile::new(format!("Photo {key}")));
    }
}

struct TextScene {
    text: String,
}

impl TextScene {
    fn new() -> Self {
        let line = "The quick brown fox jumps over the lazy dog, again and again, to fill a line.\n";
        Self { text: line.repeat(100 * 1024 / line.len() + 1) }
    }
}

impl Scene for TextScene {
    fn size(&self) -> Vec2 {
        vec2(900.0, 700.0)
    }

    fn show(&mut self, ui: &mut Ui, _frame: u64) {
        let _ = TextField::new("Notes", Bind::bound(&mut self.text)).lines(30).id_salt("notes").show(ui);
    }
}

/// Frames between two toggles; the animation runs longer, so one is always under way.
const COLLAPSE_TOGGLE_FRAMES: u64 = 30;
const PANEL_ROWS: usize = 12;
const PANEL_ROW: Vec2 = vec2(100.0, 16.0);

struct CollapseScene {
    open: bool,
}

impl CollapseScene {
    fn new() -> Self {
        Self { open: false }
    }
}

impl Scene for CollapseScene {
    fn show(&mut self, ui: &mut Ui, frame: u64) {
        if frame.is_multiple_of(COLLAPSE_TOGGLE_FRAMES) {
            self.open = !self.open;
        }
        let props = ContainerProps::new().collapse(Collapse::new().initially_open(false)).sizing(Sizing::Content);
        let _ = Panel::new(props).bind_open(Bind::bound(&mut self.open)).id_salt("panel").show(ui, |ui, _| {
            for i in 0..PANEL_ROWS {
                let _ = Block::new(PANEL_ROW).id_salt(i).show(ui);
            }
        });
    }
}

const WRAPPED: &str = "A longer description of the row that wraps onto more lines when the list is narrow, to give rows different heights.";

struct AutoHeightScene {
    source: Count,
    selection: Selection<usize>,
}

impl AutoHeightScene {
    fn new() -> Self {
        Self { source: Count(10_000), selection: Selection::new(vec![0], Some(0), Some(0)) }
    }
}

impl Scene for AutoHeightScene {
    fn size(&self) -> Vec2 {
        vec2(320.0, 640.0)
    }

    fn events(&mut self, frame: u64, events: &mut Vec<Event>) {
        if frame == 0 {
            events.push(Event::PointerMoved(Pos2::new(100.0, 300.0)));
        }
        events.push(Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: vec2(0.0, -40.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: Modifiers::NONE,
        });
    }

    fn show(&mut self, ui: &mut Ui, _frame: u64) {
        let list = List::new("Versions").select(SelectMode::Single).selection(Bind::bound(&mut self.selection));
        let _ = list.id_salt("auto").show_items(ui, &self.source, &mut |&key, _| {
            let row = ListRow::new(format!("Row {key}"));
            if key % 4 == 0 { row } else { row.detail("Today").lines(WRAPPED) }
        });
    }
}

struct ViewportScene;

impl Scene for ViewportScene {
    fn show(&mut self, ui: &mut Ui, _frame: u64) {
        ui.label("Main window");
        let props = ContainerProps::new().mount(Mount::viewport(Overlay::floating())).sizing(Sizing::Content);
        let _ = Panel::new(props).id_salt("second").show(ui, |ui, _| {
            for i in 0..20 {
                let _ = Block::new(PANEL_ROW).id_salt(i).show(ui);
            }
        });
    }
}

const STRIP_TABS: usize = 60;

/// Documents keyed by position.
struct StripDocuments;

impl DisplayOrder<usize> for StripDocuments {
    fn len(&self) -> usize {
        STRIP_TABS
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < STRIP_TABS).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        (at < STRIP_TABS).then(|| std::borrow::Cow::Owned(format!("Document {at}.indd")))
    }
}

impl ItemSource<usize> for StripDocuments {}

struct TabStripScene;

impl Scene for TabStripScene {
    fn show(&mut self, ui: &mut Ui, _frame: u64) {
        let _ = TabStrip::new("Documents")
            .kind(TabKind::Document)
            .shown(30)
            .id_salt("strip")
            .show_items(ui, &StripDocuments, &mut |key, _| Tab::new(format!("Document {key}.indd")).closable(true));
    }
}

struct DialogScene {
    values: Vec<Option<f64>>,
    texts: Vec<String>,
}

impl DialogScene {
    fn new() -> Self {
        Self { values: (0..10).map(|i| Some(f64::from(i))).collect(), texts: (0..10).map(|i| format!("Value {i}")).collect() }
    }
}

impl Scene for DialogScene {
    fn size(&self) -> Vec2 {
        vec2(800.0, 900.0)
    }

    fn show(&mut self, ui: &mut Ui, _frame: u64) {
        let Self { values, texts } = self;
        let _ = Dialog::new("Settings").id_salt("dialog").show(ui, |ui, _| {
            for (i, (value, text)) in values.iter_mut().zip(texts.iter_mut()).enumerate() {
                let _ = NumberField::new(format!("Number {i}:"), Bind::bound(value)).id_salt(("number", i)).show(ui);
                let _ = TextField::new(format!("Text {i}:"), Bind::bound(text)).id_salt(("text", i)).show(ui);
            }
        });
    }
}

/// A 4096 x 4096 document whose pixels follow their position.
struct Wide;

impl RasterSource for Wide {
    fn size(&self) -> PixelSize {
        PixelSize::new(4096, 4096)
    }

    fn revision(&self) -> Revision {
        Revision::new(1)
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        Some(PixelRect::default())
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        let [width, height] = out.size;
        for row in 0..height {
            for column in 0..width {
                let x = region.x0 + (column as u32 * factor) as i32;
                let y = region.y0 + (row as u32 * factor) as i32;
                out.pixels[row * width + column] = Color32::from_rgb((x / 16) as u8, (y / 16) as u8, ((x + y) / 32) as u8);
            }
        }
    }
}

struct CanvasViewScene {
    view: Option<CanvasView>,
    hand: HandTool,
    guides: Vec<Guide>,
    zero: Point,
}

impl CanvasViewScene {
    fn new() -> Self {
        let guides = (0..40).map(|i| Guide::new(GuideKey::new(i), if i % 2 == 0 { Axis::Y } else { Axis::X }, 100.0 * i as f64)).collect();
        Self { view: None, hand: HandTool::new(NavigationOptions::default()), guides, zero: Point::ZERO }
    }
}

impl Scene for CanvasViewScene {
    fn show(&mut self, ui: &mut Ui, _frame: u64) {
        static SOURCE: Wide = Wide;
        let Self { view, hand, guides, zero } = self;
        let content = ui.available_rect_before_wrap();
        let view = view.get_or_insert_with(|| {
            let mut view = CanvasView::new(content, Point::new(2048.0, 2048.0), 0.5);
            view.rotation = 30.0;
            view
        });
        // The scene's motion: a pan every frame, as a user's drag would make.
        pan(view, vec2(3.0, 2.0));
        let text = RulerText::new(RulerUnits::pixels(), UnitKind::Pixels);
        let shown = Canvas::new("Document").view(*view).rulers(text, text, *zero).show_rulers(true).show(ui, |ui, body| {
            let _ = RasterDisplay::new("Picture", &SOURCE).show_at(ui, body.rect);
            let _ = Guides::new("Guides", Bind::bound(&mut *guides)).show_at(ui, body.rect);
            let _ = Rulers::new("Rulers", Bind::bound(&mut *zero)).show_at(ui, body.rect);
        });
        if let Some(next) = ToolHost::new().run(ui, &shown.output, hand).view {
            *view = next;
        }
    }
}

#[derive(Default)]
struct ExportDialogScene {
    options: ExportOptions,
}

impl Scene for ExportDialogScene {
    fn size(&self) -> Vec2 {
        vec2(900.0, 700.0)
    }

    fn show(&mut self, ui: &mut Ui, _frame: u64) {
        let id = TextureCache::texture(ui.ctx(), "bench-export-preview", || ColorImage::filled([360, 240], Color32::from_rgb(0x6f, 0x8a, 0x4d)));
        let preview = ExportPreview::new(1, PreviewState::Ready(SizedTexture::new(id, vec2(360.0, 240.0)))).estimate(SizeEstimate::Proxy(1_258_291));
        let mut open = true;
        let _ = ExportAsDialog::new("Poster", PixelSize::new(1200, 800), Bind::bound(&mut self.options))
            .opened(Bind::bound(&mut open))
            .preview(preview)
            .show(ui);
    }
}

const POSITION: ColumnKey = ColumnKey::new("position");

struct ReadoutScene;

impl Scene for ReadoutScene {
    fn show(&mut self, ui: &mut Ui, frame: u64) {
        // Each frame's text is a different width.
        let digits = 1 + (frame % 9) as usize;
        let wide = "8".repeat(digits);
        let mut position = String::from("0");
        let mut size = String::from("100");
        ColumnGroup::scope(ui, "grid", |ui, group| {
            let _ = group.width(ui, &POSITION, digits as f32 * 8.0);
            let _ = TextField::new(format!("Position {wide}:"), Bind::bound(&mut position)).id_salt("position").show(ui);
            let _ = TextField::new("Size:", Bind::bound(&mut size)).id_salt("size").show(ui);
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let _ = Block::new(vec2(digits as f32 * 8.0, 16.0)).id_salt("readout").show(ui);
        });
    }
}

fn main() -> std::process::ExitCode {
    let report = run_all(&Foundation { root: RootEnv::default() });
    print_table(&report);
    let baseline = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("baseline");
    let outcome = write_json(&report, &baseline.join("foundation.json")).and_then(|()| read_json(&baseline.join("legacy.json")));
    let legacy = match outcome {
        Ok(legacy) => legacy,
        Err(error) => {
            eprintln!("the baselines could not be written or read: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let failures = gate_failures(&report, &legacy);
    for failure in &failures {
        eprintln!("gate: {failure}");
    }
    if failures.is_empty() { std::process::ExitCode::SUCCESS } else { std::process::ExitCode::FAILURE }
}
