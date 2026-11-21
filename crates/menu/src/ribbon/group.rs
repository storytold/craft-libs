//! `RibbonGroup`: a titled group of controls in the ribbon's row.

use std::hash::Hash;

use craft_behavior::IconId;
use craft_keymap::CommandRef;
use craft_ui::buttons::{Arrangement, Button, ControlPlace, IconButton};
use craft_ui::contract::{Bind, Gap, Props as _, Show as _, ShowContainer as _, Sizing, Trigger};
use craft_ui::display::{Divider, Shade, Text};
use craft_ui::layout::{RowAlign, levelled_unit};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::popover::Popover;
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label, with_env};
use craft_ui::runtime::memory::key_salt;
use craft_ui::style::TypeRole;
use egui::{Id, Pos2, Rect, Ui, Vec2};

use super::{GroupLevel, RibbonLook, RibbonRow};

craft_ui::words! {
    /// The words of a ribbon group.
    struct GroupWords {
        launcher: template "ribbon.launcher" = "{title} settings";
    }
}

/// The arrow of a collapsed group's button where the look sets menu arrows: the small filled triangle.
const COLLAPSED_ARROW: craft_ui::controls::disclosure::DisclosureGlyph = craft_ui::controls::disclosure::DisclosureGlyph::new(1.0);

/// The levels a group shows at, widest first: the order the row's levelled units report their widths in.
pub(super) const LEVELS: [GroupLevel; 3] = [GroupLevel::Full, GroupLevel::Compact, GroupLevel::Collapsed];

/// A group of the ribbon: a title, an icon for its collapsed form, and an optional launcher. Each
/// group sets its `GroupLevel` for its content; groups are access nodes (role group, named by
/// their title), and a collapsed group opens a popover.
pub struct RibbonGroup<'a> {
    /// The group's identity: it keeps levels apart where titles repeat.
    key: u64,
    title: Label,
    icon: IconSpec<'a>,
    launcher: Option<CommandRef>,
}

impl<'a> RibbonGroup<'a> {
    /// A group identified by `key`, titled `title`, with `icon` on its collapsed button.
    pub fn new(key: impl Hash, title: impl Into<Label>, icon: IconSpec<'a>) -> Self {
        RibbonGroup { key: key_salt(&key), title: title.into(), icon, launcher: None }
    }

    /// The command of the group's dialog launcher.
    pub fn launcher(mut self, command: impl Into<CommandRef>) -> Self {
        self.launcher = Some(command.into());
        self
    }

    /// The group's content over its title, with no gap between them. The content sits at the group's top and the title
    /// at its foot, on a line `line` high in every group, so the titles of groups with and without a launcher share one
    /// line; the room between them fills. A group fills the line of the ribbon's row (`unit`), so every group's title
    /// sits at the foot of the row.
    fn open(&self, ui: &mut Ui, add: &mut dyn FnMut(&mut Ui), room: &TitleRoom) {
        let line = if room.line.is_finite() { room.line.max(0.0) } else { 0.0 };
        let side = if self.launcher.is_some() { line } else { 0.0 };
        let mut title = Rect::NOTHING;
        let shown = Stack::new().gap(Gap::None).sizing(Sizing::FillHeight).show(ui, |ui, _| {
            // A look that sets the row height (`RibbonLook::row`) packs its columns as close as its rows: a tight gap
            // apart, as the old ribbon packed them.
            let columns = if room.icons.row > 0 { Gap::Tight } else { Gap::Row };
            Row::new().gap(columns).show(ui, |ui, _| add(ui));
            // The room between the content and the title.
            Stack::new().sizing(Sizing::FillHeight).show(ui, |_, _| {});
            // The title centred across the group's width as the old ribbon centred it, the launcher's square included:
            // the row keeps the launcher's room on both sides of the title, so a group as narrow as its title still
            // centres it clear of the launcher.
            title = Row::new()
                .gap(Gap::None)
                .fill_width()
                .align(RowAlign::Center)
                .show(ui, |ui, _| {
                    craft_ui::layout::place_in_flow(ui, Vec2::new(side, line));
                    Text::new(self.title.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                    craft_ui::layout::place_in_flow(ui, Vec2::new(side, 0.0));
                })
                .output
                .rect;
        });
        let group = shown.output.rect;
        // The dialog launcher: a square as high as the title line, at the group's end edge on that line, as the old
        // ribbon placed it. It shows at its place, not in the flow, so it never makes its group's title line taller.
        if let Some(command) = &self.launcher
            && group.is_finite()
            && title.is_finite()
        {
            let rtl = Env::of(ui).direction.is_rtl();
            let x = if rtl { group.min.x } else { group.max.x - side };
            let at = Rect::from_min_size(Pos2::new(x, title.center().y - side / 2.0), Vec2::splat(side));
            let words = Env::of(ui).words::<GroupWords>();
            let name = words.launcher.fill(&[("title", self.title.as_str())]);
            // The launcher keeps the framework's glyph side: the ribbon's icon sides are its buttons', not this square's.
            let icon = room.launcher.clone();
            with_env(
                ui,
                |overrides| {
                    overrides.values.set(RibbonLook::default());
                },
                |ui| IconButton::command(icon, command.clone()).label(name).show_at(ui, at),
            );
        }
    }

    /// One button with the group's icon and title; its popover holds the content at the full level. A press in a
    /// popup the content opens does not close the popover: the overlay family keeps it open.
    fn collapsed(&self, ui: &mut Ui, add: &mut dyn FnMut(&mut Ui), icons: RibbonLook) {
        let face = Button::new(self.title.clone()).icon(self.icon.clone()).arrangement(Arrangement::Stacked);
        // A look that sets menu arrows (`RibbonLook::arrow`) marks the collapsed group's button as the menu it opens.
        let face = if icons.arrow > 0 { face.trailing_icon(IconSpec::glyph(&COLLAPSED_ARROW)) } else { face };
        let button = face.show(ui);
        Popover::new(self.title.clone(), Bind::kept()).overlay(|overlay| overlay.opened_by(&button.response, Trigger::Click)).show(ui, |ui, _| {
            with_env(
                ui,
                |overrides| {
                    overrides.values.set(GroupLevel::Full);
                    overrides.values.set(ControlPlace::Ribbon);
                    overrides.values.set(icons);
                },
                |ui| {
                    Row::new().show(ui, |ui, _| add(ui));
                },
            );
        });
    }

    /// The group as the row shows it at `level`: one container `id`, a group node named by the title (a labelled
    /// `Stack` reports `Role::GROUP`), holding the body and the separator after it. The group fills the height of the
    /// row's line: the tallest group's content and title, at least the row's least height (`Ribbon`'s row). The
    /// separator fills the group's height. The node is a stack around the row of body and separator, so a group never
    /// sets the line its neighbours sit on: an open group stands at the row's top, and a collapsed group's button sits
    /// in the middle of its height. Returns the rect it took.
    fn unit(&self, ui: &mut Ui, id: Id, add: &mut dyn FnMut(&mut Ui), level: GroupLevel, room: &TitleRoom) -> Rect {
        Stack::new()
            .id(id)
            .label(self.title.clone())
            .sizing(Sizing::FillHeight)
            .show(ui, |ui, _| {
                Row::new().sizing(Sizing::FillHeight).show(ui, |ui, _| {
                    if level == GroupLevel::Collapsed {
                        self.collapsed(ui, add, room.icons);
                    } else {
                        self.open(ui, add, room);
                    }
                    Divider::vertical().show(ui);
                });
            })
            .output
            .rect
    }
}

/// Where a group's title goes: at its foot, on a line `line` high.
#[derive(Clone, Debug)]
struct TitleRoom {
    line: f32,
    /// The icon sides the group's buttons take.
    icons: RibbonLook,
    /// The dialog launcher's icon (`Ribbon::launcher_icon`).
    launcher: IconId,
}

impl RibbonRow<'_> {
    /// Adds `group` to the row, at the level the row's fit gives it; `add` fills its content.
    ///
    /// The group is a levelled unit of the row (`layout::levelled_unit`): when the row measures (its room or its
    /// content changed), it runs `add` once at each level to measure the group, and gives each group its level by
    /// production's rule ([`plan`](super::plan)); a pass in which nothing the row measures changed shows each group
    /// once, at its kept level. So `add` must be free of side effects in a measuring run, as every row body is: it
    /// reads the outputs of the controls it shows, and the framework reports those in the real run only.
    pub fn group(&mut self, group: RibbonGroup<'_>, mut add: impl FnMut(&mut Ui)) -> &mut Self {
        let id = self.base.with(("ribbon.group", group.key));
        let mut shown = None;
        let room = TitleRoom { line: self.title_line, icons: self.icons, launcher: self.launcher.clone() };
        levelled_unit(self.ui, id, LEVELS.len(), |ui, at| {
            let level = LEVELS.get(at).copied().unwrap_or(GroupLevel::Collapsed);
            // A ribbon's controls take the ribbon's look: flat buttons, fields as long as the ribbon's.
            let rect = with_env(
                ui,
                |overrides| {
                    overrides.values.set(level);
                    overrides.values.set(ControlPlace::Ribbon);
                    overrides.values.set(room.icons);
                },
                |ui| group.unit(ui, id, &mut add, level, &room),
            );
            shown = Some(rect);
        });
        if let Some(rect) = shown {
            self.groups.push(rect);
        }
        self
    }
}
