//! The navigation column of the File page in the Office-style apps: a labelled Back, the pages in
//! groups with a rule between them, and a foot for Options and About. One column for WordCraft,
//! GridCraft and DeckCraft, each in its own colour (storytold/craft-libs#53).
//!
//! The application owns the pages, their labels in the interface language and their icons, and
//! decides what each choice does. The column draws them and reports a [`FileNavAction`].
//!
//! Tab reaches every entry, Up and Down walk them, and Enter or Space chooses the one in focus.
//! An entry the application marks [disabled](FileNav::disabled) (Save with no document open) is
//! dimmed, cannot be chosen and is passed over by the keys.
//!
//! On macOS the File page covers the title bar, so the column starts with a band the height of
//! that bar ([`FileNav::title_band`]): it is left to the window buttons and drags the window.
//! Elsewhere the window buttons are on the right and the column starts with Back.

use egui::text::{LayoutJob, TextWrapping};
use egui::{Align, Color32, CornerRadius, FontId, Layout, Painter, Rect, Response, Sense, Stroke, Ui, UiBuilder, pos2, vec2};

use crate::places::finite;

/// Colours and metrics. [`FileNavStyle::on_accent`] is the column filled with the app's colour;
/// [`FileNavStyle::from_visuals`] is a neutral column with the accent only on the chosen page.
#[derive(Clone, Debug, PartialEq)]
pub struct FileNavStyle {
    pub fill: Color32,
    pub text: Color32,
    /// The label and icon of an entry that cannot be chosen now.
    pub disabled: Color32,
    pub hover: Color32,
    /// Behind the page that is showing.
    pub selected: Color32,
    /// The bar at the left edge of the page that is showing.
    pub mark: Color32,
    pub rule: Color32,
    pub width: f32,
    pub row_height: f32,
    pub foot_row_height: f32,
    /// Space above Back where there is no title band, and under the foot.
    pub padding: f32,
    pub font: FontId,
    /// The page that is showing; a heavier face where the app has one.
    pub selected_font: FontId,
    pub foot_font: FontId,
}

impl FileNavStyle {
    /// White on the app's own colour.
    pub fn on_accent(accent: Color32) -> Self {
        Self {
            fill: accent,
            text: Color32::WHITE,
            disabled: Color32::from_white_alpha(120),
            hover: Color32::from_white_alpha(26),
            selected: Color32::from_white_alpha(46),
            mark: Color32::WHITE,
            rule: Color32::from_white_alpha(70),
            width: 200.0,
            row_height: 38.0,
            foot_row_height: 32.0,
            padding: 8.0,
            font: FontId::proportional(14.0),
            selected_font: FontId::proportional(14.0),
            foot_font: FontId::proportional(13.0),
        }
    }

    /// The panel's own colours, with the accent as the mark of the page that is showing.
    pub fn from_visuals(v: &egui::Visuals) -> Self {
        Self {
            fill: v.panel_fill,
            text: v.text_color(),
            disabled: v.weak_text_color(),
            hover: v.widgets.hovered.weak_bg_fill,
            selected: v.widgets.inactive.weak_bg_fill,
            mark: v.selection.bg_fill,
            rule: v.widgets.noninteractive.bg_stroke.color,
            ..Self::on_accent(v.selection.bg_fill)
        }
    }
}

/// One page of the File page, or one entry of the column's foot.
#[derive(Clone, Copy, Debug)]
pub struct FileNavItem<'a, K> {
    /// How the application knows this entry; it comes back in [`FileNavAction::Choose`].
    pub key: K,
    pub label: &'a str,
}

/// What the user asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileNavAction<K> {
    /// Leave the File page.
    Back,
    /// A page, or an entry of the foot, was chosen.
    Choose(K),
    /// A drag began in the title band: move the window.
    DragWindow,
}

/// What happened this frame.
#[derive(Debug)]
pub struct FileNavResponse<K> {
    pub action: Option<FileNavAction<K>>,
    /// The whole column.
    pub response: Response,
}

/// Paints the icon of an entry (`None` is Back) in the given colour.
type IconPainter<'a, K> = &'a dyn Fn(Option<K>, &Painter, Rect, Color32);

const ICON: f32 = 18.0;
const ICON_X: f32 = 30.0;
const TEXT_X: f32 = 50.0;
const RULE_INSET: f32 = 22.0;
/// The height a rule takes between two groups.
const RULE: f32 = 11.0;

/// See the module documentation.
pub struct FileNav<'a, K> {
    back: &'a str,
    groups: &'a [&'a [FileNavItem<'a, K>]],
    foot: &'a [FileNavItem<'a, K>],
    selected: Option<K>,
    disabled: &'a [K],
    style: &'a FileNavStyle,
    icons: IconPainter<'a, K>,
    title_band: f32,
}

impl<'a, K: Copy + PartialEq> FileNav<'a, K> {
    /// `back` is Back's label. `groups` are the pages from the top, with a rule between one group
    /// and the next; `selected` is the page that is showing.
    pub fn new(
        back: &'a str,
        groups: &'a [&'a [FileNavItem<'a, K>]],
        selected: Option<K>,
        style: &'a FileNavStyle,
        icons: IconPainter<'a, K>,
    ) -> Self {
        Self { back, groups, foot: &[], selected, disabled: &[], style, icons, title_band: 0.0 }
    }

    /// The entries at the bottom of the column, top to bottom: Options, About.
    pub fn foot(mut self, foot: &'a [FileNavItem<'a, K>]) -> Self {
        self.foot = foot;
        self
    }

    /// The entries that cannot be chosen now, by key: dimmed, never reported as chosen, announced
    /// as unavailable and passed over by Tab and the arrow keys.
    pub fn disabled(mut self, disabled: &'a [K]) -> Self {
        self.disabled = disabled;
        self
    }

    /// Leave a band this high above Back for the window buttons, where the File page covers the
    /// title bar (macOS). It holds nothing else and reports [`FileNavAction::DragWindow`].
    pub fn title_band(mut self, height: f32) -> Self {
        self.title_band = height;
        self
    }

    /// As wide as [`FileNavStyle::width`] and as high as the space it is given.
    pub fn show(self, ui: &mut Ui) -> FileNavResponse<K> {
        let Self { back, groups, foot, selected, disabled, style: s, icons, title_band } = self;
        let (rect, response) = ui.allocate_exact_size(vec2(s.width, finite(ui.available_height(), 480.0)), Sense::hover());
        let mut action = None;
        if !ui.is_rect_visible(rect) {
            return FileNavResponse { action, response };
        }
        ui.painter().rect_filled(rect, CornerRadius::ZERO, s.fill);
        let band = if title_band > 0.0 { title_band } else { s.padding };
        let entries: usize = groups.iter().map(|group| group.len()).sum();
        let pages = s.row_height * (1 + entries) as f32 + RULE * groups.len() as f32;
        let foot_height = s.foot_row_height * foot.len() as f32 + s.padding;
        // Room for everything: the foot sits at the bottom. Otherwise it follows the pages and the
        // column scrolls, so no entry is ever drawn over another.
        let fits = band + pages + foot_height <= rect.height();
        ui.scope_builder(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)), |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            if title_band > 0.0 {
                // Dragged, never focused: a band with nothing in it is no stop for the Tab key.
                let (_, band) = ui.allocate_exact_size(vec2(s.width, title_band), Sense::DRAG);
                if band.drag_started() {
                    action = Some(FileNavAction::DragWindow);
                }
            } else {
                ui.add_space(s.padding);
            }
            // The rows from the top down, whatever order they are drawn in.
            let mut order: Vec<egui::Id> = Vec::with_capacity(1 + entries + foot.len());
            // A disabled entry is no stop for the keys, so it gives no id to walk to.
            let mut entry = |ui: &mut Ui, item: &FileNavItem<'_, K>, height: f32, font: &FontId| {
                let showing = selected == Some(item.key);
                let enabled = !disabled.contains(&item.key);
                let font = if showing && height == s.row_height { &s.selected_font } else { font };
                let response = row(ui, s, icons, Some(item.key), item.label, Some(showing), enabled, height, font);
                if enabled && response.clicked() {
                    action = Some(FileNavAction::Choose(item.key));
                }
                enabled.then_some(response.id)
            };
            let mut back_clicked = false;
            let mut pages_and = |ui: &mut Ui, order: &mut Vec<egui::Id>, foot_follows: bool| {
                let response = row(ui, s, icons, None, back, None, true, s.row_height, &s.font);
                back_clicked |= response.clicked();
                order.push(response.id);
                for group in groups {
                    rule(ui, s);
                    for item in *group {
                        order.extend(entry(ui, item, s.row_height, &s.font));
                    }
                }
                if foot_follows {
                    rule(ui, s);
                    for item in foot {
                        order.extend(entry(ui, item, s.foot_row_height, &s.foot_font));
                    }
                    ui.add_space(s.padding);
                }
            };
            if fits {
                pages_and(ui, &mut order, false);
                // The foot sits at the bottom but is built from the top down, so the Tab key meets
                // its entries in the order they are drawn, as the arrow keys do.
                ui.add_space((rect.bottom() - foot_height - ui.cursor().min.y).max(0.0));
                for item in foot {
                    order.extend(entry(ui, item, s.foot_row_height, &s.foot_font));
                }
            } else {
                egui::ScrollArea::vertical().id_salt("craft-ui/file-nav").auto_shrink([false, false]).show(ui, |ui| pages_and(ui, &mut order, true));
            }
            if back_clicked {
                action = Some(FileNavAction::Back);
            }
            walk(ui, &order);
        });
        FileNavResponse { action, response }
    }
}

/// One entry: icon, label, and the mark when it is the page that is showing. `showing` is `None`
/// for Back, which is an action and not a page. A row that is not `enabled` only senses the
/// pointer, for the tip of a cut label: no click, no focus, no hover fill.
#[allow(clippy::too_many_arguments)]
fn row<K: Copy>(
    ui: &mut Ui,
    s: &FileNavStyle,
    icons: IconPainter<'_, K>,
    key: Option<K>,
    label: &str,
    showing: Option<bool>,
    enabled: bool,
    height: f32,
    font: &FontId,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(s.width, height), if enabled { Sense::click() } else { Sense::hover() });
    if showing == Some(true) {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, s.selected);
        ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), CornerRadius::ZERO, s.mark);
    } else if enabled && (response.hovered() || response.has_focus()) {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, s.hover);
    }
    let colour = if enabled { s.text } else { s.disabled };
    icons(key, ui.painter(), Rect::from_center_size(pos2(rect.left() + ICON_X, rect.center().y), vec2(ICON, ICON)), colour);
    // A label longer than the column (a translation, usually) ends in an ellipsis instead of
    // running off the edge, and is whole in its tip.
    let mut job = LayoutJob::simple_singleline(label.to_owned(), font.clone(), colour);
    job.wrap = TextWrapping::truncate_at_width((s.width - TEXT_X - 10.0).max(0.0));
    let galley = ui.painter().layout_job(job);
    let cut = galley.elided;
    ui.painter().galley(pos2(rect.left() + TEXT_X, rect.center().y - galley.size().y / 2.0), galley, colour);
    response.widget_info(|| match showing {
        Some(showing) => egui::WidgetInfo::selected(egui::Role::Button, enabled, showing, label),
        None => egui::WidgetInfo::labeled(egui::Role::Button, enabled, label),
    });
    if cut { response.on_hover_text(label) } else { response }
}

/// Up and Down walk the rows and wrap at the ends, Home and End reach the ends, as in a menu.
/// While one of the rows has the focus the column keeps those keys, so the focus does not jump
/// to whatever lies nearest on the page beside it.
fn walk(ui: &Ui, order: &[egui::Id]) {
    let Some(at) = order.iter().position(|id| ui.memory(|m| m.has_focus(*id))) else { return };
    let Some(focused) = order.get(at).copied() else { return };
    ui.memory_mut(|m| m.set_focus_lock_filter(focused, egui::EventFilter { vertical_arrows: true, ..Default::default() }));
    let count = order.len();
    let to = ui.input_mut(|i| {
        let mut pressed = |key| i.consume_key(egui::Modifiers::NONE, key);
        if pressed(egui::Key::ArrowDown) {
            Some((at + 1) % count)
        } else if pressed(egui::Key::ArrowUp) {
            Some((at + count - 1) % count)
        } else if pressed(egui::Key::Home) {
            Some(0)
        } else if pressed(egui::Key::End) {
            Some(count - 1)
        } else {
            None
        }
    });
    if let Some(next) = to.and_then(|i| order.get(i)).copied() {
        ui.memory_mut(|m| {
            // egui has seen the same key and would move the focus once more at the end of the
            // frame, from the row it is about to get here.
            m.move_focus(egui::FocusDirection::None);
            m.request_focus(next);
        });
    }
}

fn rule(ui: &mut Ui, s: &FileNavStyle) {
    let (rect, _) = ui.allocate_exact_size(vec2(s.width, RULE), Sense::hover());
    ui.painter().hline((rect.left() + RULE_INSET)..=(rect.right() - RULE_INSET), rect.center().y, Stroke::new(1.0, s.rule));
}

#[cfg(test)]
mod tests;
