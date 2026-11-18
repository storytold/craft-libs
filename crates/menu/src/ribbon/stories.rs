//! Stories of the ribbon over a small model: three tabs of four groups each.

use craft_behavior::IconId;
use craft_keymap::CommandRef;
use craft_ui::buttons::{Arrangement, Button};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::runtime::env::{Direction, Env, with_env};
use craft_ui::tabs::StripEnd;
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::Ui;

use super::{Ribbon, RibbonGroup, RibbonRow, RibbonTab};

/// The group titles of each tab.
const GROUPS: [[&str; 4]; 3] =
    [["Clipboard", "Font", "Paragraph", "Styles"], ["Pages", "Tables", "Illustrations", "Links"], ["Views", "Show", "Zoom", "Window"]];

/// The buttons of every group.
const BUTTONS: [&str; 3] = ["Large action", "Another action", "Third action"];

/// The widths the stories hold the ribbon to: the groups' compact form, their collapsed form, and a row that
/// scrolls even with every group collapsed.
const COMPACT_ROOM: f32 = 640.0;
const COLLAPSED_ROOM: f32 = 380.0;
const SCROLLED_ROOM: f32 = 160.0;

/// What a story changes about the ribbon.
#[derive(Clone, Copy, Default)]
struct Setup {
    /// The width the ribbon is held to; its natural width without one.
    room: Option<f32>,
    /// The ribbon starts minimised.
    minimised: bool,
    /// A contextual tab shows after the plain ones.
    contextual: bool,
    /// An item shows at the end of the strip.
    end: bool,
    /// The ribbon reads right to left.
    rtl: bool,
}

/// The groups of `tab`.
fn groups(row: &mut RibbonRow<'_>, tab: &usize) {
    for (at, title) in GROUPS[*tab % GROUPS.len()].iter().enumerate() {
        let icon = IconId::new("list");
        row.group(RibbonGroup::new((*tab, at), *title, icon.into()), |ui| {
            for label in BUTTONS {
                Button::new(label).icon(IconId::new("list")).arrangement(Arrangement::Stacked).show(ui);
            }
        });
    }
}

/// Shows the ribbon as `setup` says and applies the tab it reports.
fn show(ui: &mut Ui, data: &mut StoryData, setup: Setup) -> StoryOutput {
    let shown = data.value("shown", 0_usize);
    let mut tabs = vec![RibbonTab::new(0_usize, "Home"), RibbonTab::new(1, "Insert"), RibbonTab::new(2, "View")];
    if setup.contextual {
        tabs.push(RibbonTab::new(3, "Picture").contextual());
    }
    let items = [StripEnd::command("Share", IconId::new("link").into(), CommandRef::new("ribbon.share"))];
    let mut ribbon = Ribbon::new("Ribbon", &tabs, Bind::given(*shown));
    if setup.end {
        ribbon = ribbon.trailing(&items);
    }
    if setup.minimised {
        ribbon = ribbon.minimised(Bind::given(true));
    }
    let direction = if setup.rtl { Direction::Rtl } else { Env::of(ui).direction };
    let out = with_env(
        ui,
        |overrides| {
            overrides.direction = Some(direction);
        },
        |ui| match setup.room {
            Some(room) => craft_ui::testing::memory::scoped(ui, |ui| {
                ui.set_max_width(room);
                ribbon.show(ui, groups)
            }),
            None => ribbon.show(ui, groups),
        },
    );
    if let Some(next) = out.shown {
        *shown = next;
    }
    StoryOutput::default()
}

#[story(Ribbon<'static, usize>, covers = [Default], drive = Drive::Never)]
fn ribbon(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, Setup::default())
}

#[story(Ribbon<'static, usize>, covers = [], drive = Drive::Never)]
fn ribbon_compact(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, Setup { room: Some(COMPACT_ROOM), ..Setup::default() })
}

#[story(Ribbon<'static, usize>, covers = [], drive = Drive::Never)]
fn ribbon_collapsed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, Setup { room: Some(COLLAPSED_ROOM), ..Setup::default() })
}

fn open_group(fixture: &mut Fixture) {
    if let Some(group) = fixture.by_label(GROUPS[0][0]) {
        fixture.click(&group);
    }
}

#[story(Ribbon<'static, usize>, covers = [Open], drive = Drive::Custom(open_group))]
fn ribbon_collapsed_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, Setup { room: Some(COLLAPSED_ROOM), ..Setup::default() })
}

#[story(Ribbon<'static, usize>, covers = [], drive = Drive::Never)]
fn ribbon_scrolled(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, Setup { room: Some(SCROLLED_ROOM), ..Setup::default() })
}

#[story(Ribbon<'static, usize>, covers = [], drive = Drive::Never)]
fn ribbon_rtl(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, Setup { room: Some(COMPACT_ROOM), rtl: true, ..Setup::default() })
}

#[story(Ribbon<'static, usize>, covers = [], drive = Drive::Never)]
fn ribbon_minimised(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, Setup { minimised: true, ..Setup::default() })
}

#[story(Ribbon<'static, usize>, covers = [], drive = Drive::Never)]
fn ribbon_contextual_tab(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, Setup { contextual: true, ..Setup::default() })
}

#[story(Ribbon<'static, usize>, covers = [], drive = Drive::Never)]
fn ribbon_end_items_narrow(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, Setup { room: Some(SCROLLED_ROOM), end: true, ..Setup::default() })
}

fn pick_last_tab(fixture: &mut Fixture) {
    if let Some(tab) = fixture.by_label("View") {
        fixture.click(&tab);
    }
}

#[story(Ribbon<'static, usize>, covers = [], drive = Drive::Custom(pick_last_tab))]
fn ribbon_tab_change(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data, Setup::default())
}
