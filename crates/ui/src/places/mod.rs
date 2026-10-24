//! The places panel: where a Craft app opens a document from and saves it to. Places on the left
//! (Recent, one per connected account, the synced cloud folders, This computer), the chosen place's
//! files and folders on the right, as a list or as thumbnails.
//!
//! One panel serves three hosts: the File › Open and Save As pages of the Office-style apps, the
//! home screen of the graphics apps (which put their New and Open buttons in [`PlacesPanel::header`])
//! and, through [`menu`], the "Open from" submenu of a classic File menu.
//!
//! The panel performs no IO and knows no storage provider. The application passes the places and
//! the entries it has, and acts on the [`PlacesAction`] it gets back: it lists a folder, opens a
//! file, starts a sign-in or shows the system's own file dialog. An application without remote
//! storage passes no account places and leaves [`PlacesPanel::add_place`] off; nothing else changes.

pub mod local;
pub mod synced;

use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Color32, CornerRadius, CursorIcon, FontId, Painter, Pos2, Rect, Response, ScrollArea, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};

/// Colours and metrics; [`PlacesStyle::from_visuals`] derives them from egui's visuals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacesStyle {
    pub fill: Color32,
    pub border: Color32,
    pub text: Color32,
    pub weak: Color32,
    pub hover: Color32,
    /// The chosen place's row.
    pub selected: Color32,
    /// The app's accent, for state only: the focus ring and the hovered thumbnail's outline, and
    /// through [`PlacesStyle::selected`] the chosen place. It never colours an icon: icons look
    /// the same in every app.
    pub accent: Color32,
    pub icon: Color32,
    /// An entry's detail that needs noticing, such as somebody else editing it.
    pub attention: Color32,
    /// Behind a thumbnail, and all there is when the app paints none.
    pub thumbnail: Color32,
    pub radius: u8,
    /// The places column; never more than 45% of the panel.
    pub rail_width: f32,
    pub row_height: f32,
    pub font: f32,
    pub small_font: f32,
}

impl PlacesStyle {
    pub fn from_visuals(v: &egui::Visuals) -> Self {
        let (fill, text, accent) = (v.extreme_bg_color, v.text_color(), v.selection.bg_fill);
        Self {
            fill,
            border: v.widgets.noninteractive.bg_stroke.color,
            text,
            weak: v.weak_text_color(),
            hover: v.widgets.hovered.weak_bg_fill,
            selected: mix(fill, accent, 0.14),
            accent,
            icon: mix(text, fill, 0.3),
            attention: v.warn_fg_color,
            thumbnail: mix(fill, text, 0.1),
            radius: 6,
            rail_width: 200.0,
            row_height: 30.0,
            font: 13.0,
            small_font: 11.5,
        }
    }
}

/// What a place is, which decides its icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceKind {
    /// Recently used documents, wherever they live.
    Recent,
    /// Remote storage reached through a signed-in account.
    Account,
    /// A folder a cloud service's own app keeps in step on this computer (see [`synced`]).
    Synced,
    /// This computer.
    Local,
    /// Anything else the application lists, such as a pinned folder.
    Other,
}

/// One row of the places column.
#[derive(Clone, Copy, Debug)]
pub struct Place<'a> {
    pub kind: PlaceKind,
    pub label: &'a str,
    /// A second line that tells two places of one kind apart: the account's address.
    pub detail: Option<&'a str>,
}

/// A file or folder of the chosen place.
#[derive(Clone, Copy, Debug)]
pub struct PlaceEntry<'a> {
    pub name: &'a str,
    pub folder: bool,
    /// Beside the name in a list, under it in the thumbnail view: when it changed, where it lives,
    /// who shared it. Empty for none.
    pub detail: &'a str,
    /// Draw `detail` in [`PlacesStyle::attention`].
    pub attention: bool,
    /// Lives in remote storage. Marks its thumbnail, which matters in Recent, where local and
    /// remote documents are mixed.
    pub remote: bool,
}

/// The right-hand side.
#[derive(Clone, Copy, Debug)]
pub enum PlacesContent<'a> {
    Entries(&'a [PlaceEntry<'a>]),
    /// The application is still asking; shows [`PlacesLabels::loading`].
    Loading,
    /// Nothing to list, with the reason in one plain line in the interface language.
    Message(&'a str),
    /// Nothing to list, said properly: see [`PlacesNotice`].
    Notice(PlacesNotice<'a>),
}

/// Why there is nothing to list, and what to do about it: an icon, a short title, a sentence or
/// two and, where the user can act, a button. For an empty folder, a folder that is gone, or one
/// the system will not let the app read.
#[derive(Clone, Copy, Debug)]
pub struct PlacesNotice<'a> {
    pub icon: PlacesIcon,
    pub title: &'a str,
    /// Empty for none.
    pub body: &'a str,
    /// The label of a button under the text; pressing it reports [`PlacesAction::Notice`].
    pub action: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlacesView {
    #[default]
    List,
    Thumbnails,
}

/// Every string the panel shows, in the interface language.
#[derive(Clone, Copy, Debug)]
pub struct PlacesLabels<'a> {
    /// The caption above the places.
    pub places: &'a str,
    pub add_place: &'a str,
    pub browse: &'a str,
    pub loading: &'a str,
}

/// The glyphs the panel draws; [`PlacesPanel::icons`] replaces [`paint_icon`] with the app's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacesIcon {
    Recent,
    Account,
    /// A folder with a cloud on it: a synced folder, on this computer and in the cloud.
    Synced,
    Local,
    Other,
    Add,
    Browse,
    Folder,
    File,
    /// A place or folder that is closed to the app.
    Locked,
}

/// What the user asked for. Indices point into the slices the panel was given this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacesAction {
    /// A place was chosen.
    Select(usize),
    /// A file or folder was chosen: open the file, or list the folder.
    Activate(usize),
    /// A breadcrumb step other than the last was chosen.
    Crumb(usize),
    AddPlace,
    /// Show the system's own file dialog.
    Browse,
    /// The button of a [`PlacesNotice`] was pressed.
    Notice,
}

/// What happened this frame.
#[derive(Debug)]
pub struct PlacesResponse {
    pub action: Option<PlacesAction>,
    /// The whole panel.
    pub response: Response,
}

type IconPainter<'a> = &'a dyn Fn(PlacesIcon, &Painter, Rect, Color32);
type ThumbnailPainter<'a> = &'a dyn Fn(usize, &Painter, Rect);
type Header<'a> = Box<dyn FnOnce(&mut Ui) + 'a>;

const PAD: f32 = 8.0;
const ICON: f32 = 16.0;
const TEXT_X: f32 = PAD + ICON + 8.0;
const TILE_WIDTH: f32 = 132.0;
const TILE_GAP: f32 = 12.0;
const THUMBNAIL_HEIGHT: f32 = 84.0;
const CRUMB_SPACING: f32 = 4.0;
const CRUMB_SEPARATOR: f32 = 8.0;
const ELLIPSIS: &str = "…";

/// See the module documentation.
pub struct PlacesPanel<'a> {
    places: &'a [Place<'a>],
    selected: Option<usize>,
    content: PlacesContent<'a>,
    style: &'a PlacesStyle,
    labels: PlacesLabels<'a>,
    crumbs: &'a [&'a str],
    view: PlacesView,
    add_place: bool,
    browse: bool,
    icons: Option<IconPainter<'a>>,
    thumbnails: Option<ThumbnailPainter<'a>>,
    header: Option<Header<'a>>,
}

impl<'a> PlacesPanel<'a> {
    /// `selected` indexes `places`; `None` highlights no place.
    pub fn new(
        places: &'a [Place<'a>],
        selected: Option<usize>,
        content: PlacesContent<'a>,
        style: &'a PlacesStyle,
        labels: PlacesLabels<'a>,
    ) -> Self {
        Self {
            places,
            selected,
            content,
            style,
            labels,
            crumbs: &[],
            view: PlacesView::List,
            add_place: false,
            browse: true,
            icons: None,
            thumbnails: None,
            header: None,
        }
    }

    /// The path inside the chosen place, outermost first. Every step but the last can be chosen.
    pub fn crumbs(mut self, crumbs: &'a [&'a str]) -> Self {
        self.crumbs = crumbs;
        self
    }

    pub fn view(mut self, view: PlacesView) -> Self {
        self.view = view;
        self
    }

    /// Offer "Add a place". Off by default: an application without remote storage has nothing to
    /// add, and a control that cannot work is worse than none.
    pub fn add_place(mut self, add_place: bool) -> Self {
        self.add_place = add_place;
        self
    }

    /// Offer Browse, the way to the system's own file dialog. On by default; a host with its own
    /// Open button above the places turns it off.
    pub fn browse(mut self, browse: bool) -> Self {
        self.browse = browse;
        self
    }

    /// The application's own glyphs instead of [`paint_icon`].
    pub fn icons(mut self, icons: IconPainter<'a>) -> Self {
        self.icons = Some(icons);
        self
    }

    /// Paints the thumbnail of the file at the given entry index, over [`PlacesStyle::thumbnail`].
    /// Without it a file's thumbnail is its icon. Folders always show the folder icon.
    pub fn thumbnails(mut self, thumbnails: ThumbnailPainter<'a>) -> Self {
        self.thumbnails = Some(thumbnails);
        self
    }

    /// The host's own controls above the places, such as a home screen's New and Open buttons.
    pub fn header(mut self, header: impl FnOnce(&mut Ui) + 'a) -> Self {
        self.header = Some(Box::new(header));
        self
    }

    /// Fills the space it is given; where that is unbounded it takes 640 × 360.
    pub fn show(self, ui: &mut Ui) -> PlacesResponse {
        let Self { places, selected, content, style: s, labels, crumbs, view, add_place, browse, icons, thumbnails, header } = self;
        let available = ui.available_size_before_wrap();
        let (rect, response) = ui.allocate_exact_size(vec2(finite(available.x, 640.0), finite(available.y, 360.0)), Sense::hover());
        let mut action = None;
        if !ui.is_rect_visible(rect) {
            return PlacesResponse { action, response };
        }
        let icon = |which: PlacesIcon, painter: &Painter, at: Rect, colour: Color32| match icons {
            Some(paint) => paint(which, painter, at, colour),
            None => paint_icon(which, painter, at, colour),
        };
        ui.painter().rect_filled(rect, CornerRadius::ZERO, s.fill);
        let rail = Rect::from_min_max(rect.min, pos2(rect.left() + s.rail_width.min(rect.width() * 0.45).max(0.0), rect.bottom()));
        let body = Rect::from_min_max(pos2(rail.right(), rect.top()), rect.max);
        ui.painter().vline(rail.right(), rect.y_range(), Stroke::new(1.0, s.border));

        ui.scope_builder(UiBuilder::new().max_rect(rail.shrink(PAD)), |ui| {
            ui.set_clip_rect(rail.intersect(ui.clip_rect()));
            ScrollArea::vertical().id_salt("craft-ui/places/rail").auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                if let Some(header) = header {
                    header(ui);
                    ui.add_space(PAD);
                }
                caption(ui, s, labels.places);
                for (i, place) in places.iter().enumerate() {
                    if place_row(ui, s, &icon, place, selected == Some(i)).clicked() {
                        action = Some(PlacesAction::Select(i));
                    }
                }
                if add_place || browse {
                    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
                    ui.painter().hline(line.x_range(), line.center().y, Stroke::new(1.0, s.border));
                }
                if add_place && action_row(ui, s, &icon, PlacesIcon::Add, labels.add_place).clicked() {
                    action = Some(PlacesAction::AddPlace);
                }
                if browse && action_row(ui, s, &icon, PlacesIcon::Browse, labels.browse).clicked() {
                    action = Some(PlacesAction::Browse);
                }
            });
        });

        ui.scope_builder(UiBuilder::new().max_rect(body.shrink2(vec2(14.0, PAD + 2.0))), |ui| {
            ui.set_clip_rect(body.intersect(ui.clip_rect()));
            if !crumbs.is_empty() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(CRUMB_SPACING, 0.0);
                    let widths: Vec<f32> = crumbs.iter().map(|step| text_width(ui, s, step)).collect();
                    let dots = text_width(ui, s, ELLIPSIS);
                    let shown = fit_crumbs(&widths, CRUMB_SEPARATOR + 2.0 * CRUMB_SPACING, dots, finite(ui.available_width(), f32::MAX));
                    for (position, step) in shown.iter().enumerate() {
                        let last = position + 1 == shown.len();
                        match step.and_then(|i| crumbs.get(i).map(|label| (i, label))) {
                            Some((i, label)) => {
                                if crumb(ui, s, label, last).clicked() && !last {
                                    action = Some(PlacesAction::Crumb(i));
                                }
                            }
                            // The steps that did not fit. Not a control: the step before it and
                            // the ones after it are, and the full path is the app's to show.
                            None => {
                                let (at, _) = ui.allocate_exact_size(vec2(dots, s.row_height - 6.0), Sense::hover());
                                text(ui.painter(), at.left_center(), at.width() + 1.0, ELLIPSIS, s.font, s.weak);
                            }
                        }
                        if !last {
                            let (at, _) = ui.allocate_exact_size(vec2(CRUMB_SEPARATOR, s.row_height - 6.0), Sense::hover());
                            text(ui.painter(), at.left_center(), at.width(), "›", s.font, s.weak);
                        }
                    }
                });
            }
            ScrollArea::vertical().id_salt("craft-ui/places/content").auto_shrink([false, false]).show(ui, |ui| match content {
                PlacesContent::Loading => message(ui, s, labels.loading),
                PlacesContent::Message(why) => message(ui, s, why),
                PlacesContent::Notice(what) => {
                    if notice(ui, s, &icon, &what) {
                        action = Some(PlacesAction::Notice);
                    }
                }
                PlacesContent::Entries(entries) if view == PlacesView::List => {
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    for (i, entry) in entries.iter().enumerate() {
                        if list_row(ui, s, &icon, entry).clicked() {
                            action = Some(PlacesAction::Activate(i));
                        }
                    }
                }
                PlacesContent::Entries(entries) => {
                    let columns = (((finite(ui.available_width(), TILE_WIDTH) + TILE_GAP) / (TILE_WIDTH + TILE_GAP)).floor() as usize).max(1);
                    ui.spacing_mut().item_spacing = vec2(TILE_GAP, TILE_GAP);
                    for (row, tiles) in entries.chunks(columns).enumerate() {
                        ui.horizontal(|ui| {
                            for (column, entry) in tiles.iter().enumerate() {
                                let i = row * columns + column;
                                if tile(ui, s, &icon, thumbnails, i, entry).clicked() {
                                    action = Some(PlacesAction::Activate(i));
                                }
                            }
                        });
                    }
                }
            });
        });
        PlacesResponse { action, response }
    }
}

/// The same places as items of a menu: the "Open from" and "Save to" submenus of a classic File
/// menu. Each account's address sits where a shortcut would.
pub fn menu(ui: &mut Ui, places: &[Place<'_>], labels: PlacesLabels<'_>, add_place: bool) -> Option<PlacesAction> {
    let mut action = None;
    for (i, place) in places.iter().enumerate() {
        let mut item = egui::Button::new(place.label);
        if let Some(detail) = place.detail {
            item = item.shortcut_text(detail);
        }
        if ui.add(item).clicked() {
            action = Some(PlacesAction::Select(i));
        }
    }
    if add_place {
        ui.separator();
        if ui.button(labels.add_place).clicked() {
            action = Some(PlacesAction::AddPlace);
        }
    }
    action
}

fn caption(ui: &mut Ui, s: &PlacesStyle, label: &str) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), s.small_font + 10.0), Sense::hover());
    text(ui.painter(), pos2(rect.left() + PAD, rect.center().y), rect.width() - PAD, label, s.small_font, s.weak);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label));
}

fn place_row(ui: &mut Ui, s: &PlacesStyle, icon: IconPainter<'_>, place: &Place<'_>, selected: bool) -> Response {
    let height = if place.detail.is_some() { s.row_height + s.small_font + 3.0 } else { s.row_height };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, CornerRadius::same(s.radius), s.selected);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(s.radius), s.hover);
    }
    focus_ring(ui, s, &response, rect);
    let which = match place.kind {
        PlaceKind::Recent => PlacesIcon::Recent,
        PlaceKind::Account => PlacesIcon::Account,
        PlaceKind::Synced => PlacesIcon::Synced,
        PlaceKind::Local => PlacesIcon::Local,
        PlaceKind::Other => PlacesIcon::Other,
    };
    icon(which, ui.painter(), icon_rect(rect), s.icon);
    let (x, width) = (rect.left() + TEXT_X, rect.width() - TEXT_X - PAD);
    match place.detail {
        Some(detail) => {
            let lift = (s.small_font + 3.0) / 2.0;
            text(ui.painter(), pos2(x, rect.center().y - lift), width, place.label, s.font, s.text);
            text(ui.painter(), pos2(x, rect.center().y + (s.font + 3.0) / 2.0), width, detail, s.small_font, s.weak);
        }
        None => {
            text(ui.painter(), pos2(x, rect.center().y), width, place.label, s.font, s.text);
        }
    }
    let name = match place.detail {
        Some(detail) => format!("{}, {detail}", place.label),
        None => place.label.to_owned(),
    };
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, &name));
    response
}

fn action_row(ui: &mut Ui, s: &PlacesStyle, icon: IconPainter<'_>, which: PlacesIcon, label: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), s.row_height), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(s.radius), s.hover);
    }
    focus_ring(ui, s, &response, rect);
    icon(which, ui.painter(), icon_rect(rect), s.weak);
    text(ui.painter(), pos2(rect.left() + TEXT_X, rect.center().y), rect.width() - TEXT_X - PAD, label, s.font, s.weak);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    response
}

fn crumb(ui: &mut Ui, s: &PlacesStyle, label: &str, last: bool) -> Response {
    let width = text_width(ui, s, label);
    let (rect, response) = ui.allocate_exact_size(vec2(width, s.row_height - 6.0), if last { Sense::hover() } else { Sense::click() });
    text(ui.painter(), rect.left_center(), rect.width() + 1.0, label, s.font, if last { s.text } else { s.weak });
    if last {
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label));
        return response;
    }
    if response.hovered() || response.has_focus() {
        ui.painter().hline(rect.x_range(), rect.center().y + s.font * 0.62, Stroke::new(1.0, s.weak));
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Link, true, label));
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// The widest a line of running text gets: past this a sentence is hard to follow back.
const READING_WIDTH: f32 = 440.0;

fn message(ui: &mut Ui, s: &PlacesStyle, why: &str) {
    ui.allocate_ui(vec2(READING_WIDTH.min(ui.available_width()), 0.0), |ui| {
        ui.add(egui::Label::new(egui::RichText::new(why).size(s.font).color(s.weak)).selectable(false).wrap());
    });
}

/// Draws a [`PlacesNotice`] in the middle of the width, a little down from the top. True when its
/// button was pressed.
fn notice(ui: &mut Ui, s: &PlacesStyle, icon: IconPainter<'_>, what: &PlacesNotice<'_>) -> bool {
    let mut pressed = false;
    ui.add_space((ui.available_height() * 0.14).clamp(12.0, 72.0));
    ui.vertical_centered(|ui| {
        let (disc, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
        ui.painter().circle_filled(disc.center(), 28.0, s.thumbnail);
        icon(what.icon, ui.painter(), Rect::from_center_size(disc.center(), vec2(26.0, 26.0)), s.icon);
        ui.add_space(12.0);
        ui.allocate_ui_with_layout(vec2(READING_WIDTH.min(ui.available_width()), 0.0), egui::Layout::top_down(egui::Align::Center), |ui| {
            let line = |text: &str, size: f32, colour: Color32| {
                egui::Label::new(egui::RichText::new(text).size(size).color(colour)).selectable(false).wrap().halign(egui::Align::Center)
            };
            ui.add(line(what.title, s.font + 2.0, s.text));
            if !what.body.is_empty() {
                ui.add_space(4.0);
                ui.add(line(what.body, s.font, s.weak));
            }
            if let Some(label) = what.action {
                ui.add_space(14.0);
                // Room either side of the label: a button hugging its text reads as a tag.
                ui.spacing_mut().button_padding = vec2(20.0, 4.0);
                pressed = ui.add(egui::Button::new(egui::RichText::new(label).size(s.font)).min_size(vec2(112.0, s.row_height))).clicked();
            }
        });
    });
    pressed
}

fn list_row(ui: &mut Ui, s: &PlacesStyle, icon: IconPainter<'_>, entry: &PlaceEntry<'_>) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), s.row_height + 2.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(s.radius), s.hover);
    }
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, s.border.gamma_multiply(0.6)));
    focus_ring(ui, s, &response, rect);
    let which = if entry.folder { PlacesIcon::Folder } else { PlacesIcon::File };
    icon(which, ui.painter(), icon_rect(rect), s.icon);
    // The detail keeps its place at the right edge and the name gives way: a long name must not
    // push "Anna is editing" out of sight.
    let detail = galley(ui.painter(), rect.width() * 0.5, entry.detail, s.small_font + 0.5, if entry.attention { s.attention } else { s.weak });
    let taken = detail.size().x;
    ui.painter().galley(pos2(rect.right() - PAD - taken, rect.center().y - detail.size().y / 2.0), detail, s.weak);
    text(ui.painter(), pos2(rect.left() + TEXT_X, rect.center().y), rect.width() - TEXT_X - PAD * 2.0 - taken, entry.name, s.font, s.text);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, spoken(entry)));
    response
}

fn tile(
    ui: &mut Ui,
    s: &PlacesStyle,
    icon: IconPainter<'_>,
    thumbnails: Option<ThumbnailPainter<'_>>,
    index: usize,
    entry: &PlaceEntry<'_>,
) -> Response {
    let height = THUMBNAIL_HEIGHT + s.font + s.small_font + 16.0;
    let (rect, response) = ui.allocate_exact_size(vec2(TILE_WIDTH, height), Sense::click());
    let picture = Rect::from_min_size(rect.min, vec2(TILE_WIDTH, THUMBNAIL_HEIGHT));
    let radius = CornerRadius::same(s.radius);
    ui.painter().rect_filled(picture, radius, s.thumbnail);
    let glyph = Rect::from_center_size(picture.center(), vec2(28.0, 28.0));
    match thumbnails {
        _ if entry.folder => icon(PlacesIcon::Folder, ui.painter(), glyph, s.icon),
        Some(paint) => paint(index, &ui.painter().with_clip_rect(picture.intersect(ui.clip_rect())), picture),
        None => icon(PlacesIcon::File, ui.painter(), glyph, s.icon),
    }
    if entry.remote {
        let badge = Rect::from_min_size(picture.max - vec2(26.0, 26.0), vec2(20.0, 20.0));
        ui.painter().rect_filled(badge, CornerRadius::same(4), s.fill);
        icon(PlacesIcon::Account, ui.painter(), badge.shrink(3.0), s.icon);
    }
    if response.hovered() || response.has_focus() {
        ui.painter().rect_stroke(picture, radius, Stroke::new(1.5, s.accent), StrokeKind::Inside);
    }
    let name_y = picture.bottom() + 6.0 + s.font / 2.0;
    text(ui.painter(), pos2(rect.left(), name_y), TILE_WIDTH, entry.name, s.font, s.text);
    let colour = if entry.attention { s.attention } else { s.weak };
    text(ui.painter(), pos2(rect.left(), name_y + (s.font + s.small_font) / 2.0 + 3.0), TILE_WIDTH, entry.detail, s.small_font, colour);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, spoken(entry)));
    response
}

/// Keyboard focus, which hover does not show.
fn focus_ring(ui: &Ui, s: &PlacesStyle, response: &Response, rect: Rect) {
    if response.has_focus() {
        ui.painter().rect_stroke(rect, CornerRadius::same(s.radius), Stroke::new(1.5, s.accent), StrokeKind::Inside);
    }
}

/// How wide `label` is in the panel's font.
fn text_width(ui: &Ui, s: &PlacesStyle, label: &str) -> f32 {
    ui.fonts_mut(|f| f.layout_no_wrap(label.to_owned(), FontId::proportional(s.font), s.text).size().x)
}

/// What a screen reader says for an entry: its name and, where there is one, the detail beside
/// it. "Anna is editing" is not only for people who can see the row.
fn spoken(entry: &PlaceEntry<'_>) -> String {
    if entry.detail.is_empty() { entry.name.to_owned() } else { format!("{}, {}", entry.name, entry.detail) }
}

/// Which breadcrumb steps to draw in `available` width, in order; `None` stands for the steps
/// left out. A path that does not fit keeps its first step and as many of its last ones as there
/// is room for, the last always among them.
fn fit_crumbs(widths: &[f32], between: f32, ellipsis: f32, available: f32) -> Vec<Option<usize>> {
    let count = widths.len();
    let all = || (0..count).map(Some).collect();
    let total = widths.iter().sum::<f32>() + between * count.saturating_sub(1) as f32;
    let (Some(first), Some(last)) = (widths.first(), widths.last()) else { return all() };
    if count <= 2 || total <= available {
        return all();
    }
    let mut used = first + between + ellipsis + between + last;
    let mut tail = count - 1;
    while tail > 1 {
        let Some(step) = widths.get(tail - 1) else { break };
        if used + step + between > available {
            break;
        }
        used += step + between;
        tail -= 1;
    }
    if tail == 1 {
        return all();
    }
    [Some(0), None].into_iter().chain((tail..count).map(Some)).collect()
}

fn icon_rect(row: Rect) -> Rect {
    Rect::from_center_size(pos2(row.left() + PAD + ICON / 2.0, row.center().y), vec2(ICON, ICON))
}

/// One line, cut with an ellipsis at `width`.
fn galley(painter: &Painter, width: f32, label: &str, size: f32, colour: Color32) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::simple_singleline(label.to_owned(), FontId::proportional(size), colour);
    job.wrap = TextWrapping::truncate_at_width(width.max(0.0));
    painter.layout_job(job)
}

/// Paints one line starting at `left_centre`, cut at `width`.
fn text(painter: &Painter, left_centre: Pos2, width: f32, label: &str, size: f32, colour: Color32) {
    let galley = galley(painter, width, label, size, colour);
    painter.galley(pos2(left_centre.x, left_centre.y - galley.size().y / 2.0), galley, colour);
}

/// The panel's own line glyphs, for an application that passes none through [`PlacesPanel::icons`].
pub fn paint_icon(icon: PlacesIcon, painter: &Painter, rect: Rect, colour: Color32) {
    let stroke = Stroke::new((rect.width() / 12.0).clamp(1.0, 2.0), colour);
    let at = |x: f32, y: f32| pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height());
    let line = |points: &[(f32, f32)]| Shape::line(points.iter().map(|&(x, y)| at(x, y)).collect(), stroke);
    let outline = |points: &[(f32, f32)]| Shape::closed_line(points.iter().map(|&(x, y)| at(x, y)).collect(), stroke);
    match icon {
        PlacesIcon::Recent => {
            painter.circle_stroke(rect.center(), rect.width() * 0.42, stroke);
            painter.add(line(&[(0.5, 0.26), (0.5, 0.52), (0.67, 0.62)]));
        }
        PlacesIcon::Account => {
            painter.add(Shape::closed_line(cloud(&at, (0.5, 0.5), 1.0), stroke));
        }
        PlacesIcon::Synced => {
            painter.add(outline(&[(0.1, 0.24), (0.4, 0.24), (0.5, 0.36), (0.9, 0.36), (0.9, 0.8), (0.1, 0.8)]));
            painter.add(Shape::closed_line(cloud(&at, (0.5, 0.59), 0.58), stroke));
        }
        PlacesIcon::Local => {
            painter.rect_stroke(Rect::from_min_max(at(0.14, 0.2), at(0.86, 0.68)), CornerRadius::same(1), stroke, StrokeKind::Middle);
            painter.add(line(&[(0.04, 0.84), (0.96, 0.84)]));
        }
        PlacesIcon::Add => {
            painter.add(line(&[(0.5, 0.2), (0.5, 0.8)]));
            painter.add(line(&[(0.2, 0.5), (0.8, 0.5)]));
        }
        PlacesIcon::Other | PlacesIcon::Browse | PlacesIcon::Folder => {
            painter.add(outline(&[(0.1, 0.24), (0.4, 0.24), (0.5, 0.36), (0.9, 0.36), (0.9, 0.8), (0.1, 0.8)]));
        }
        PlacesIcon::Locked => {
            painter.rect_stroke(Rect::from_min_max(at(0.2, 0.46), at(0.8, 0.88)), CornerRadius::same(2), stroke, StrokeKind::Middle);
            // The shackle: half a circle on two short posts.
            let mut points = vec![at(0.33, 0.46)];
            points.extend((0..=10).map(|step| {
                let angle = (180.0 + 18.0 * step as f32).to_radians();
                at(0.5 + 0.17 * angle.cos(), 0.31 + 0.17 * angle.sin())
            }));
            points.push(at(0.67, 0.46));
            painter.add(Shape::line(points, stroke));
        }
        PlacesIcon::File => {
            painter.add(outline(&[(0.22, 0.1), (0.6, 0.1), (0.8, 0.3), (0.8, 0.9), (0.22, 0.9)]));
            painter.add(line(&[(0.6, 0.1), (0.6, 0.3), (0.8, 0.3)]));
            painter.add(line(&[(0.36, 0.56), (0.66, 0.56)]));
            painter.add(line(&[(0.36, 0.72), (0.66, 0.72)]));
        }
    }
}

/// A cloud, three arcs over a flat base, centred on `centre` at `scale` of the icon.
fn cloud(at: &dyn Fn(f32, f32) -> Pos2, centre: (f32, f32), scale: f32) -> Vec<Pos2> {
    let mut points = Vec::with_capacity(39);
    for (cx, cy, r, from, to) in [(0.29, 0.61, 0.21, 90.0, 245.0), (0.48, 0.43, 0.25, 195.0, 335.0), (0.73, 0.62, 0.2, 285.0, 450.0)] {
        for step in 0..=12 {
            let angle = (from + (to - from) * step as f32 / 12.0_f32).to_radians();
            let (x, y) = (cx + r * angle.cos(), cy + r * angle.sin());
            points.push(at(centre.0 + (x - 0.5) * scale, centre.1 + (y - 0.5) * scale));
        }
    }
    points
}

pub(crate) fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value.max(0.0) } else { fallback }
}

/// `a` moved `t` of the way to `b`, for opaque colours.
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let channel = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(channel(a.r(), b.r()), channel(a.g(), b.g()), channel(a.b(), b.b()))
}

#[cfg(test)]
mod tests;
