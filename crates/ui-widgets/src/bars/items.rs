//! `BarItems`: a strip of command items that overflows into a menu of command rows.

use super::words::BarWords;
use crate::buttons::{Button, IconButton, ToggleButton};
use crate::contract::{
    Access, Common, Component, Cx, MeasureCx, Name, OverlayUi, Overlays as _, PaintUi, PartsCx, Props as _, Role, Show as _, ShowContainer as _,
    WordSetCx as _,
};
use crate::display::Divider;
use crate::menu::{Menu, MenuItem};
use crate::primitives::IconSpec;
use crate::runtime::commands::{CommandRef, ResolvedCommand};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, GlyphRole, StyleCx};
use craft_behavior::fit::{FitItem, FitKeep, fit_items};
use craft_behavior::{Axis, IconId, Metrics, NavShape, PartKey, PartRef, Proposal, Roving, Sense};
use craft_theme::Variant;
use egui::{Rect, Response, Vec2, pos2};

const ITEM: PartKey = PartKey::new("item");
const MORE: PartKey = PartKey::new("more");

/// What an item shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ItemDisplay {
    /// The command's icon; the label is the tooltip and the accessible name. A command with no icon shows `Text`.
    #[default]
    Icon,
    /// The command's label.
    Text,
    /// The command's icon before its label. A command with no icon shows `Text`.
    IconText,
}

/// What the "More" menu of a `BarItems` strip lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum MoreMenu {
    /// The items the strip has no room for, and the `Menu` items; "More" shows only when there are any.
    #[default]
    Overflow,
    /// Every item, shown on the strip or not; "More" always shows.
    Every,
}

/// One command of a `BarItems` strip.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct BarItem {
    /// The command the item shows and runs.
    pub command: CommandRef,
    /// The section: neighbouring items of one section are a group, which the strip shows or moves into "More" whole.
    /// A rule separates the groups of a horizontal strip, extra space those of a vertical one.
    pub section: u16,
    /// What the item shows.
    pub display: ItemDisplay,
    /// Whether the item may overflow.
    pub keep: FitKeep,
}

impl BarItem {
    /// An item for `command` in section 0, shown as its icon.
    pub fn new(command: impl Into<CommandRef>) -> Self {
        BarItem { command: command.into(), section: 0, display: ItemDisplay::Icon, keep: FitKeep::Free }
    }

    /// Puts the item in `section`.
    #[must_use]
    pub const fn section(mut self, section: u16) -> Self {
        self.section = section;
        self
    }

    /// Sets what the item shows.
    #[must_use]
    pub const fn display(mut self, display: ItemDisplay) -> Self {
        self.display = display;
        self
    }

    /// Sets whether the item may overflow.
    #[must_use]
    pub const fn keep(mut self, keep: FitKeep) -> Self {
        self.keep = keep;
        self
    }
}

/// The style of a bar item strip: the gap between items, the space around the rule of a section change and the
/// space between the groups of a vertical strip.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct BarItemsStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// Between an item and the rule of a section change.
    joint: f32,
    /// The clear space of a section change, on both sides of its rule together.
    section_gap: f32,
    /// Between the groups of a vertical strip, which draws no rule: an item gap and `space.gap.stack`.
    stack_gap: f32,
    /// The strip's glyph role, which its item faces show their icons in.
    glyphs: GlyphRole,
}

/// An item whose command resolved this pass.
#[derive(Clone)]
struct Present {
    item: BarItem,
    resolved: ResolvedCommand,
}

/// Where the parts of a strip sit this pass.
struct Laid {
    present: Vec<Present>,
    shown: Vec<(usize, Rect)>,
    rules: Vec<Rect>,
    overflow: Vec<usize>,
    more: Option<Rect>,
}

/// The sizes the faces of a strip measure.
struct Sizes {
    items: Vec<Vec2>,
    rule: f32,
    more: Vec2,
}

/// A run of neighbouring items of one section: the items it shows on the strip, in order, and its length there.
struct Group {
    items: Vec<usize>,
    extent: f32,
    keep: FitKeep,
}

/// What an item's part wears.
enum Face {
    Action(IconButton<'static>),
    Toggle(ToggleButton<'static>),
    Text(Button<'static>),
}

impl Face {
    fn size<C: Component>(self, cx: &MeasureCx<'_, C>) -> Vec2 {
        match self {
            Face::Action(face) => cx.face(face),
            Face::Toggle(face) => cx.face(face),
            Face::Text(face) => cx.face(face),
        }
    }

    fn paint(self, ui: &mut PaintUi<'_>, part: PartRef) {
        match self {
            Face::Action(face) => ui.face(part, face),
            Face::Toggle(face) => ui.face(part, face),
            Face::Text(face) => ui.face(part, face),
        }
    }
}

/// The icon of a command, when the registry names one.
fn icon_of(resolved: &ResolvedCommand) -> Option<IconSpec<'static>> {
    resolved.icon.clone().map(|name| IconSpec::svg(IconId::from(name)))
}

/// What the item shows: its display, or its label when the command has no icon for it.
fn display_of(present: &Present) -> ItemDisplay {
    if present.item.display != ItemDisplay::Text && icon_of(&present.resolved).is_none() { ItemDisplay::Text } else { present.item.display }
}

/// The face of an item: an icon button, a toggle for a command with a checked state, or a text button. Icon-only faces
/// take the strip's glyph role.
fn face_of(present: &Present, glyphs: GlyphRole) -> Face {
    let (item, resolved) = (&present.item, &present.resolved);
    let icon = icon_of(resolved);
    let toggle = resolved.checked.is_some();
    match (display_of(present), icon) {
        (ItemDisplay::Icon, Some(icon)) if toggle => Face::Toggle(ToggleButton::command(item.command.clone()).icon(icon).icon_only().glyphs(glyphs)),
        (ItemDisplay::Icon, Some(icon)) => Face::Action(IconButton::command(icon, item.command.clone()).glyphs(glyphs)),
        (ItemDisplay::IconText, Some(icon)) if toggle => Face::Toggle(ToggleButton::command(item.command.clone()).icon(icon)),
        (ItemDisplay::IconText, Some(icon)) => Face::Text(Button::command(item.command.clone()).variant(Variant::GHOST).icon(icon)),
        (_, _) if toggle => Face::Toggle(ToggleButton::command(item.command.clone())),
        (_, _) => Face::Text(Button::command(item.command.clone()).variant(Variant::GHOST)),
    }
}

fn along(axis: Axis, size: Vec2) -> f32 {
    match axis {
        Axis::Y => size.y,
        _ => size.x,
    }
}

fn across(axis: Axis, size: Vec2) -> f32 {
    match axis {
        Axis::Y => size.x,
        _ => size.y,
    }
}

/// The part of `rect` that starts `from` points from the strip's start and runs `length` along it, across the
/// whole rect.
fn slot(rect: Rect, axis: Axis, rtl: bool, from: f32, length: f32) -> Rect {
    match axis {
        Axis::Y => Rect::from_min_max(pos2(rect.left(), rect.top() + from), pos2(rect.right(), rect.top() + from + length)),
        _ if rtl => Rect::from_min_max(pos2(rect.right() - from - length, rect.top()), pos2(rect.right() - from, rect.bottom())),
        _ => Rect::from_min_max(pos2(rect.left() + from, rect.top()), pos2(rect.left() + from + length, rect.bottom())),
    }
}

/// A strip of command items. Each item is a part that wears the face of a button resolved from its command: an
/// icon button, a toggle for a command with a checked state, or a text button. The items are one Tab stop with
/// arrow keys between them, and the strip joins the roving group of the body it is shown in. An icon item's tooltip
/// is the command's label and its chord. Neighbouring items of one section are a group. When the room is too short,
/// the last group with no `Pinned` item moves into a menu behind a "More" part, whole, as rows of the same commands,
/// a rule between the groups; `Menu` items are always there. With `MoreMenu::Every` the menu lists every item.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(overlay)]
pub struct BarItems {
    #[common]
    common: Common,
    items: Vec<BarItem>,
    /// The axis the strip runs along.
    #[prop]
    axis: Axis,
    /// What the "More" menu lists.
    #[prop]
    more: MoreMenu,
    laid: Option<Laid>,
}

impl BarItems {
    /// A strip of `items`, in the order given, running along the horizontal axis.
    pub fn new(items: Vec<BarItem>) -> Self {
        BarItems { common: Common::new(), items, axis: Axis::X, more: MoreMenu::Overflow, laid: None }
    }

    /// The items whose command the registry knows.
    fn present(&self, cx: &MeasureCx<'_, Self>) -> Vec<Present> {
        let resolve = |item: &BarItem| cx.resolve_command(item.command.clone()).map(|resolved| Present { item: item.clone(), resolved });
        self.items.iter().filter_map(resolve).collect()
    }

    fn sizes(&self, cx: &MeasureCx<'_, Self>, present: &[Present]) -> Sizes {
        let axis = self.axis;
        let items = present.iter().map(|present| face_of(present, cx.style.glyphs).size(cx)).collect();
        let rule = if axis == Axis::Y { cx.face(Divider::horizontal()).y } else { cx.face(Divider::vertical()).x };
        let more = cx.face(IconButton::new(IconId::new("more")).label(cx.env.words::<BarWords>().items_more.clone()).glyphs(cx.style.glyphs));
        Sizes { items, rule, more }
    }

    /// The space between two groups on the strip: around the rule of a horizontal strip, or the stack gap of a
    /// vertical one.
    fn group_gap(&self, style: &BarItemsStyle, sizes: &Sizes) -> f32 {
        if self.axis == Axis::Y { style.stack_gap } else { style.section_gap + sizes.rule }
    }

    /// The groups of the items the strip can show, in order. `Menu` items are in no group.
    fn groups(&self, sizes: &Sizes, present: &[Present], gap: f32) -> Vec<Group> {
        let mut groups: Vec<(u16, Group)> = Vec::new();
        for (at, present) in present.iter().enumerate().filter(|(_, present)| present.item.keep != FitKeep::Menu) {
            let length = along(self.axis, sizes.items[at]);
            match groups.last_mut() {
                Some((section, group)) if *section == present.item.section => {
                    group.items.push(at);
                    group.extent += gap + length;
                }
                _ => groups.push((present.item.section, Group { items: vec![at], extent: length, keep: FitKeep::Free })),
            }
            if present.item.keep == FitKeep::Pinned {
                groups.last_mut().expect("just pushed").1.keep = FitKeep::Pinned;
            }
        }
        groups.into_iter().map(|(_, group)| group).collect()
    }

    /// Whether "More" shows whatever the room.
    fn more_always(&self, present: &[Present]) -> bool {
        self.more == MoreMenu::Every || present.iter().any(|present| present.item.keep == FitKeep::Menu)
    }

    /// The groups the strip shows, by index, fitted into `available`.
    fn fit(&self, groups: &[Group], present: &[Present], available: f32, gap: f32, group_gap: f32, more: f32) -> Vec<usize> {
        // Each group is one fit item of a section of its own; a `Menu` item at the end keeps "More" on the strip.
        let mut items: Vec<FitItem> = groups.iter().enumerate().map(|(at, group)| FitItem::new(group.extent, at as u16, group.keep)).collect();
        if self.more_always(present) {
            items.push(FitItem::new(0.0, u16::MAX, FitKeep::Menu));
        }
        fit_items(&items, available, gap, group_gap, more).shown.into_iter().filter(|&at| at < groups.len()).collect()
    }
}

impl Component for BarItems {
    const ROLE: Role = Role::GROUP;
    type Style = BarItemsStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: BarItemsStyleKey) -> BarItemsStyle {
        let gap = cx.tokens.space.gap.inline;
        BarItemsStyle {
            metrics: Metrics::new().gap(gap),
            joint: gap,
            section_gap: gap * 2.0,
            stack_gap: gap + cx.tokens.space.gap.stack,
            glyphs: cx.glyph_role(),
        }
    }

    /// The whole strip, or the room along the strip when that is shorter. Asked for the narrowest it takes (a
    /// `Proposal::Min`), it gives the least that shows a group: the first group and the "More" part. The real run fits
    /// the groups into the width it is given, so the strip moves groups into "More" only when that is narrower than
    /// the whole strip. The strip with one group and no "More" is as long as its items.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let present = self.present(cx);
        if present.is_empty() {
            return Vec2::ZERO;
        }
        let sizes = self.sizes(cx, &present);
        let gap = cx.gap();
        let groups = self.groups(&sizes, &present, gap);
        let group_gap = self.group_gap(cx.style, &sizes);
        let more = along(self.axis, sizes.more);
        let run: f32 = groups.iter().map(|group| group.extent).sum::<f32>() + group_gap * groups.len().saturating_sub(1) as f32;
        let more_after = |run: f32| if run > 0.0 { run + gap + more } else { more };
        let natural = if self.more_always(&present) { more_after(run) } else { run };
        let least = if cx.proposal == Proposal::Min {
            let first = groups.first().map_or(0.0, |group| group.extent);
            (groups.len() > 1 || self.more_always(&present)).then(|| more_after(first).min(natural))
        } else {
            None
        };
        let natural = least.unwrap_or(natural);
        let cross = sizes.items.iter().map(|size| across(self.axis, *size)).fold(across(self.axis, sizes.more), f32::max);
        match self.axis {
            Axis::Y => Vec2::new(cross, natural),
            _ => Vec2::new(cx.room.map_or(natural, |room| natural.min(room)), cross),
        }
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    /// Fits the items into the rect, places the shown items, the rules between sections and the "More" part, and
    /// declares the items and "More" as one roving group in reading order.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let axis = self.axis;
        let present = self.present(cx);
        let sizes = self.sizes(cx, &present);
        let (gap, joint, rtl, os) = (cx.gap(), cx.style.joint, cx.flow.is_rtl(), cx.env.shell().os);
        let group_gap = self.group_gap(cx.style, &sizes);
        let more_length = along(axis, sizes.more);
        let groups = self.groups(&sizes, &present, gap);
        let fitted = self.fit(&groups, &present, along(axis, rect.size()), gap, group_gap, more_length);
        let mut at = 0.0;
        let (mut shown, mut rules) = (Vec::new(), Vec::new());
        for (n, group) in fitted.iter().map(|&index| &groups[index]).enumerate() {
            if n > 0 {
                if axis != Axis::Y {
                    rules.push(slot(rect, axis, rtl, at + joint, sizes.rule));
                }
                at += group_gap;
            }
            for (k, &index) in group.items.iter().enumerate() {
                if k > 0 {
                    at += gap;
                }
                let length = along(axis, sizes.items[index]);
                shown.push((index, slot(rect, axis, rtl, at, length)));
                at += length;
            }
        }
        let on_strip: Vec<usize> = shown.iter().map(|&(index, _)| index).collect();
        let overflow: Vec<usize> = match self.more {
            MoreMenu::Every => (0..present.len()).collect(),
            MoreMenu::Overflow => (0..present.len()).filter(|index| !on_strip.contains(index)).collect(),
        };
        let more = (!overflow.is_empty()).then(|| {
            let from = if shown.is_empty() { at } else { at + gap };
            slot(rect, axis, rtl, from, more_length)
        });
        for &(index, cell) in &shown {
            let part = cx.part(ITEM.at(index as u64), cell).sense(Sense::CLICK | Sense::FOCUS).focusable();
            if display_of(&present[index]) == ItemDisplay::Icon {
                let resolved = &present[index].resolved;
                part.tooltip(match &resolved.chord {
                    Some(chord) => Label::from(crate::contract::tooltip::with_chord(&resolved.label, chord, os)),
                    None => Label::Shared(resolved.label.clone()),
                });
            }
        }
        if let Some(more) = more {
            cx.part(MORE, more).sense(Sense::CLICK | Sense::FOCUS).focusable().toggles("menu");
        }
        cx.roving(Roving::new(NavShape::Flat));
        self.laid = Some(Laid { present, shown, rules, overflow, more });
    }

    /// The strip joins the body's roving group; a click or the activation key on an item runs its command.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        cx.join_roving();
        let Some(laid) = &self.laid else { return };
        for &(index, _) in &laid.shown {
            if let Some(source) = cx.part_activated(ITEM.at(index as u64)) {
                cx.fire(laid.present[index].item.command.clone(), source);
            }
        }
    }

    /// The "More" menu: one command row per item it lists, a rule between sections.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let Some(laid) = self.laid.as_ref().filter(|laid| laid.more.is_some()) else { return };
        let popup = cx.popup("menu");
        let (present, overflow) = (&laid.present, &laid.overflow);
        let name = cx.label();
        ui.show(popup, crate::menu::layer(false), |ui, _| {
            Menu::inline(name.clone()).show(ui, |ui, _| {
                let mut previous: Option<u16> = None;
                for &index in overflow {
                    let item = &present[index].item;
                    if previous.is_some_and(|section| section != item.section) {
                        Divider::horizontal().show(ui);
                    }
                    MenuItem::command(item.command.clone()).show(ui);
                    previous = Some(item.section);
                }
            });
        });
    }

    /// A strip the caller did not label is named by the words of the family.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if cx.resolved.label.is_empty() {
            access.name(Name::Text(cx.words::<BarWords>().items_actions.as_str().to_owned()));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(laid) = &self.laid else { return };
        for &(index, _) in &laid.shown {
            face_of(&laid.present[index], cx.style.glyphs).paint(ui, ITEM.at(index as u64));
        }
        for (at, rule) in laid.rules.iter().enumerate() {
            if self.axis == Axis::Y {
                ui.child(("rule", at), *rule, Divider::horizontal());
            } else {
                ui.child(("rule", at), *rule, Divider::vertical());
            }
        }
        if laid.more.is_some() {
            ui.face(MORE, IconButton::new(IconId::new("more")).label(cx.words::<BarWords>().items_more.clone()).glyphs(cx.style.glyphs));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
