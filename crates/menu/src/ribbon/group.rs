//! `RibbonGroup`: a titled group of controls in the ribbon's row.

use std::hash::Hash;

use craft_behavior::IconId;
use craft_keymap::CommandRef;
use craft_ui::buttons::{Arrangement, Button, ControlPlace, IconButton};
use craft_ui::contract::{Bind, Gap, Props as _, Show as _, ShowContainer as _, Trigger};
use craft_ui::display::{Divider, Shade, Text};
use craft_ui::layout::levelled_unit;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::popover::Popover;
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label, with_env};
use craft_ui::runtime::measure;
use craft_ui::runtime::memory::{key_salt, with_session};
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

    /// The group's content over its title row, with no gap between them: the title sits directly under the content,
    /// `body` under the content's top whatever the content's height, so every group's title shares one line at the foot
    /// of the row; a taller content pushes it down. `kept` is the slot of this group's content height at this level.
    ///
    /// The spacer under the content and the separator's length come from the content height the last real run kept,
    /// never from the content's output in this run: a measuring run has no real size for the content yet, so a spacer
    /// sized from it there took the whole `body`, the group measured its content's height too tall (the ribbon stood
    /// one body too high under its titles), and the real run, placing another spacer, never matched what was measured,
    /// so the row measured again on every pass. Both runs of a pass now place the same sizes; a changed content height
    /// is kept by the real run and shows in the next pass.
    ///
    /// Returns the content's height as the real run found it (none in a measuring run), which sets `body` for the
    /// row's next pass, and the height of the whole group, which its separator takes.
    fn open(&self, ui: &mut Ui, add: &mut dyn FnMut(&mut Ui), room: &TitleRoom, kept: Id) -> (Option<f32>, f32) {
        let ctx = ui.ctx().clone();
        let line = if room.line.is_finite() { room.line.max(0.0) } else { 0.0 };
        let body = if room.body.is_finite() { room.body.max(0.0) } else { 0.0 };
        let held = with_session(&ctx, kept, |kept: &mut ContentHeight| kept.0);
        let held = if held.is_finite() { held.max(0.0) } else { 0.0 };
        let side = if self.launcher.is_some() { line } else { 0.0 };
        let centred = kept.with("title");
        let centre = with_session(&ctx, centred, |kept: &mut TitleCentre| *kept);
        let mut took = f32::NAN;
        let mut took_width = f32::NAN;
        let mut title = Rect::NOTHING;
        let shown = Stack::new().gap(Gap::None).show(ui, |ui, _| {
            // A look that sets the row height (`RibbonLook::row`) packs its columns as close as its rows: a tight gap
            // apart, as the old ribbon packed them.
            let columns = if room.icons.row > 0 { Gap::Tight } else { Gap::Row };
            let content = Row::new().gap(columns).show(ui, |ui, _| add(ui)).output.rect;
            took = content.height();
            took_width = content.width();
            // Every group places this spacer, an empty one too, so each has the same gaps above its title.
            craft_ui::layout::place_in_flow(ui, Vec2::new(0.0, (body - held).max(0.0)));
            // The title on a line `line` high in every group, so the titles of groups with and without a launcher
            // share one line, centred across the group's width as the old ribbon centred it, the launcher's square
            // included. The row keeps the launcher's room on both sides of the title, so a group as narrow as its
            // title still centres it clear of the launcher; under wider content a lead centres it, sized from the
            // widths the last real run kept, so both runs of a pass place the same sizes (as the spacer above).
            let lead = ((centre.content - centre.title) / 2.0).max(0.0);
            title = Row::new()
                .gap(Gap::None)
                .show(ui, |ui, _| {
                    craft_ui::layout::place_in_flow(ui, Vec2::new(lead + side, line));
                    Text::new(self.title.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                    craft_ui::layout::place_in_flow(ui, Vec2::new(side, 0.0));
                })
                .output
                .rect;
        });
        // Only the real run keeps what the content took; a measuring run's output is no size of it.
        let found = (!measure::active(&ctx) && took.is_finite()).then_some(took.max(0.0));
        if let Some(found) = found
            && (found - held).abs() >= 0.5
        {
            with_session(&ctx, kept, |kept: &mut ContentHeight| kept.0 = found);
            ctx.request_repaint();
        }
        // The real run keeps the content's and the title row's own widths for the next pass's lead.
        if !measure::active(&ctx) && took_width.is_finite() && title.is_finite() {
            let lead = ((centre.content - centre.title) / 2.0).max(0.0);
            let found = TitleCentre { content: took_width.max(0.0), title: (title.width() - lead).max(0.0) };
            if (found.content - centre.content).abs() >= 0.5 || (found.title - centre.title).abs() >= 0.5 {
                with_session(&ctx, centred, |kept: &mut TitleCentre| *kept = found);
                ctx.request_repaint();
            }
        }
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
        // What the stack places: the content or the body, whichever is taller, and the title line.
        (found, held.max(body) + line)
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
    /// `Row` reports `Role::GROUP`), holding the body and the separator after it. An open group's title sits `body`
    /// under its top on a line `line` high (`open`). The separator is as long as the group is high: a line that filled
    /// the row would take whatever room the row was offered, and every group would set the ribbon's height by it.
    /// Returns the rect it took and the height of its open content (none when collapsed).
    fn unit(&self, ui: &mut Ui, id: Id, add: &mut dyn FnMut(&mut Ui), level: GroupLevel, room: &TitleRoom) -> (Rect, Option<f32>) {
        let mut content = None;
        let rect = Row::new()
            .id(id)
            .label(self.title.clone())
            .show(ui, |ui, _| {
                let high = if level == GroupLevel::Collapsed {
                    self.collapsed(ui, add, room.icons);
                    room.body + room.line
                } else {
                    let (took, high) = self.open(ui, add, room, id.with(("content", level)));
                    content = took;
                    high
                };
                Divider::vertical().length(high).show(ui);
            })
            .output
            .rect;
        (rect, content)
    }
}

/// The height of the tallest open group content the ribbon's row showed in its last pass: the line every group's
/// title sits under. Kept in the ribbon's session memory.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct BodyHeight(pub(super) f32);

/// The height of one open group's content at one level, as the last real run found it: what the spacer under it and
/// the group's separator are sized from in every run of the next pass. Kept in the ribbon's session memory.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ContentHeight(f32);

/// The width of one open group's content at one level and of its title row without its lead, as the last real run
/// found them: what the lead that centres the title is sized from in every run of the next pass.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct TitleCentre {
    content: f32,
    title: f32,
}

/// Where a group's title goes: `body` under the group's top, on a line `line` high.
#[derive(Clone, Debug)]
struct TitleRoom {
    body: f32,
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
        let room = TitleRoom { body: self.body, line: self.title_line, icons: self.icons, launcher: self.launcher.clone() };
        let mut tallest = self.tallest;
        levelled_unit(self.ui, id, LEVELS.len(), |ui, at| {
            let level = LEVELS.get(at).copied().unwrap_or(GroupLevel::Collapsed);
            // A ribbon's controls take the ribbon's look: flat buttons, fields as long as the ribbon's.
            let (rect, content) = with_env(
                ui,
                |overrides| {
                    overrides.values.set(level);
                    overrides.values.set(ControlPlace::Ribbon);
                    overrides.values.set(room.icons);
                },
                |ui| group.unit(ui, id, &mut add, level, &room),
            );
            if let Some(content) = content.filter(|height| height.is_finite()) {
                tallest = tallest.max(content);
            }
            shown = Some(rect);
        });
        self.tallest = tallest;
        if let Some(rect) = shown {
            self.groups.push(rect);
        }
        self
    }
}
