//! Combo boxes and drop-down menus whose menu follows its contents' height while it is open.
//!
//! egui lays a popup out in the size it had on the previous pass, and [`egui::ComboBox`] puts its
//! menu in a [`ScrollArea`] that takes no more than that height. Contents that shrink (a search
//! with no matches) shrink the menu; contents that grow again (the search cleared) only scroll
//! inside it, and the menu stays small until it is closed (emilk/egui#5225). [`ComboBox`] is
//! egui's combo box with a menu that offers its contents the full height on every pass, so the
//! menu grows back. [`Menu`] is that menu under a field an app draws itself.
//!
//! A searchable menu keeps its search field in place and scrolls only its list:
//! [`ComboBox::show_ui_with_header`] and [`ComboBox::show_rows_with_header`], and the same on
//! [`Menu`].

use std::cell::RefCell;
use std::ops::Range;

use egui::{
    Align, Align2, AsIdSalt, Atom, AtomExt as _, Atoms, Context, Direction, Frame, Id, IdSalt, InnerResponse, IntoAtoms, LayerId, NumExt as _, Order,
    Painter, Popup, PopupCloseBehavior, Rect, RectAlign, Response, Role, ScrollArea, Sense, Shape, TextStyle, TextWrapMode, Ui, Vec2, WidgetAtom,
    WidgetInfo,
    class::{ClassName, Classes, HasClasses},
    style::{StyleModifier, WidgetVisuals},
    vec2,
    widget_style::ButtonStyle,
};

pub use egui::IconPainter;

/// egui's [`egui::ComboBox`] with a menu that grows when its contents do.
///
/// The builder, the button and what the methods return are egui 0.37's. The one difference is
/// the menu: its scroll area is offered the full [`Self::height`] on every pass instead of the
/// menu's height on the previous pass, so a menu whose contents shrink and grow again while it
/// is open (a filtered list) grows back. It adds [`Self::show_ui_with_header`] and
/// [`Self::show_rows_with_header`] for menus with a search field.
///
/// ```
/// # egui::__run_test_ui(|ui| {
/// let mut selected = 0;
/// craft_ui::ComboBox::from_label("Size").show_index(ui, &mut selected, 3, |i| ["Small", "Medium", "Large"][i]);
/// # });
/// ```
#[must_use = "You should call .show*"]
pub struct ComboBox<'a> {
    id_salt: IdSalt,
    label: Option<Atoms<'a>>,
    selected_text: Atoms<'a>,
    width: Option<f32>,
    height: Option<f32>,
    icon: Option<IconPainter>,
    wrap_mode: Option<TextWrapMode>,
    close_behavior: Option<PopupCloseBehavior>,
    popup_style: StyleModifier,
    classes: Classes,
}

impl<'a> ComboBox<'a> {
    /// Present on the button of every combo box, as on egui's.
    #[expect(clippy::disallowed_types, reason = "egui's class names, so themes style both alike")]
    pub const CLASS: ClassName = egui::ComboBox::CLASS;

    /// Present on the button of a combo box while its menu is open, as on egui's.
    #[expect(clippy::disallowed_types, reason = "egui's class names, so themes style both alike")]
    pub const CLASS_OPEN: ClassName = egui::ComboBox::CLASS_OPEN;

    fn with(id_salt: IdSalt, label: Option<Atoms<'a>>) -> Self {
        let mut combo = Self {
            id_salt,
            label,
            selected_text: Atoms::default(),
            width: None,
            height: None,
            icon: None,
            wrap_mode: None,
            close_behavior: None,
            popup_style: StyleModifier::default(),
            classes: Classes::default(),
        };
        combo.add_class(Self::CLASS);
        combo
    }

    /// A combo box with `label` next to it, its id from `id_salt`.
    pub fn new(id_salt: impl AsIdSalt, label: impl IntoAtoms<'a>) -> Self {
        Self::with(IdSalt::new(id_salt), Some(label.into_atoms()))
    }

    /// A combo box with `label` next to it, its id from the label's contents.
    pub fn from_label(label: impl IntoAtoms<'a>) -> Self {
        let label = label.into_atoms();
        Self::with(label.salt(), Some(label))
    }

    /// A combo box without a label.
    pub fn from_id_salt(id_salt: impl AsIdSalt) -> Self {
        Self::with(IdSalt::new(id_salt), None)
    }

    /// The button's minimum width. Default: [`egui::Spacing::combo_width`].
    #[inline]
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// The maximum height of the menu's contents, inside its frame. Default:
    /// [`egui::Spacing::combo_height`].
    #[inline]
    pub fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    /// What the button shows as the selected value.
    #[inline]
    pub fn selected_text(mut self, selected_text: impl IntoAtoms<'a>) -> Self {
        self.selected_text = selected_text.into_atoms();
        self
    }

    /// Paints the icon in place of the down-pointing triangle.
    #[inline]
    pub fn icon(mut self, icon_fn: impl FnOnce(&Ui, Rect, &WidgetVisuals, bool) + 'static) -> Self {
        self.icon = Some(Box::new(icon_fn));
        self
    }

    /// How the selected text wraps. Default: the [`Ui`]'s.
    #[inline]
    pub fn wrap_mode(mut self, wrap_mode: TextWrapMode) -> Self {
        self.wrap_mode = Some(wrap_mode);
        self
    }

    /// Wraps the selected text: [`TextWrapMode::Wrap`].
    #[inline]
    pub fn wrap(mut self) -> Self {
        self.wrap_mode = Some(TextWrapMode::Wrap);
        self
    }

    /// Truncates the selected text: [`TextWrapMode::Truncate`].
    #[inline]
    pub fn truncate(mut self) -> Self {
        self.wrap_mode = Some(TextWrapMode::Truncate);
        self
    }

    /// When the menu closes. Default: [`PopupCloseBehavior::CloseOnClick`].
    #[inline]
    pub fn close_behavior(mut self, close_behavior: PopupCloseBehavior) -> Self {
        self.close_behavior = Some(close_behavior);
        self
    }

    /// The menu's style. Default: the [`Ui`]'s.
    #[inline]
    pub fn popup_style(mut self, popup_style: StyleModifier) -> Self {
        self.popup_style = popup_style;
        self
    }

    /// Shows the button, and `menu_contents` in the menu while it is open. `inner` is what
    /// `menu_contents` returned, or `None` while the menu is closed.
    pub fn show_ui<R>(self, ui: &mut Ui, menu_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<Option<R>> {
        self.show(ui, |menu| menu.show_ui(menu_contents))
    }

    /// Like [`Self::show_ui`], with `header` (a search field, filter chips) fixed at the top of
    /// the menu and only `list` scrolling under it. See [`Menu::show_ui_with_header`].
    pub fn show_ui_with_header<H, R>(
        self,
        ui: &mut Ui,
        header: impl FnOnce(&mut Ui) -> H,
        list: impl FnOnce(&mut Ui, H) -> R,
    ) -> InnerResponse<Option<R>> {
        self.show(ui, |menu| menu.show_ui_with_header(header, list))
    }

    /// Like [`Self::show_ui_with_header`], with a list of rows of one height that only lays out
    /// the rows in view. See [`Menu::show_rows_with_header`].
    pub fn show_rows_with_header<H, R>(
        self,
        ui: &mut Ui,
        header: impl FnOnce(&mut Ui) -> H,
        row_height: f32,
        total_rows: impl FnOnce(&H) -> usize,
        rows: impl FnOnce(&mut Ui, Range<usize>, H) -> R,
    ) -> InnerResponse<Option<R>> {
        self.show(ui, |menu| menu.show_rows_with_header(header, row_height, total_rows, rows))
    }

    /// Like [`Self::show_ui_with_header`], with `footer` (an "Add…" row) fixed under the list.
    /// See [`Menu::show_ui_with_header_footer`].
    pub fn show_ui_with_header_footer<H, R, F>(
        self,
        ui: &mut Ui,
        header: impl FnOnce(&mut Ui) -> H,
        list: impl FnOnce(&mut Ui, H) -> R,
        footer: impl FnOnce(&mut Ui) -> F,
    ) -> InnerResponse<Option<(R, F)>> {
        self.show(ui, |menu| menu.show_ui_with_header_footer(header, list, footer))
    }

    /// Like [`Self::show_rows_with_header`], with `footer` fixed under the list. See
    /// [`Menu::show_ui_with_header_footer`].
    pub fn show_rows_with_header_footer<H, R, F>(
        self,
        ui: &mut Ui,
        header: impl FnOnce(&mut Ui) -> H,
        row_height: f32,
        total_rows: impl FnOnce(&H) -> usize,
        rows: impl FnOnce(&mut Ui, Range<usize>, H) -> R,
        footer: impl FnOnce(&mut Ui) -> F,
    ) -> InnerResponse<Option<(R, F)>> {
        self.show(ui, |menu| menu.show_rows_with_header_footer(header, row_height, total_rows, rows, footer))
    }

    fn show<R>(self, ui: &mut Ui, open: impl FnOnce(Menu<'_>) -> Option<R>) -> InnerResponse<Option<R>> {
        let button_id = ui.make_persistent_id(self.id_salt);
        show(ui, button_id, self, open)
    }

    /// Shows `get(i)` for `i` in `0..len` as the choices, with `*selected` chosen. The response is
    /// marked changed when a choice is picked.
    pub fn show_index<Text: IntoAtoms<'a>>(self, ui: &mut Ui, selected: &mut usize, len: usize, get: impl Fn(usize) -> Text) -> Response {
        let combo = self.selected_text(get(*selected));
        let mut changed = false;
        let mut response = combo
            .show_ui(ui, |ui| {
                for i in 0..len {
                    if ui.selectable_label(i == *selected, get(i)).clicked() {
                        *selected = i;
                        changed = true;
                    }
                }
            })
            .response;
        if changed {
            response.mark_changed();
        }
        response
    }

    /// Whether the menu of the combo box `id` (its response's id) is open.
    pub fn is_open(ctx: &Context, id: Id) -> bool {
        Popup::is_id_open(ctx, popup_id(id))
    }
}

impl HasClasses for ComboBox<'_> {
    fn classes(&self) -> &Classes {
        &self.classes
    }

    fn classes_mut(&mut self) -> &mut Classes {
        &mut self.classes
    }
}

fn popup_id(button_id: Id) -> Id {
    button_id.with("popup")
}

// The button is egui 0.37's `combo_box_dyn`, so a combo box looks the same as egui's.
fn show<R>(ui: &mut Ui, button_id: Id, mut combo: ComboBox<'_>, open: impl FnOnce(Menu<'_>) -> Option<R>) -> InnerResponse<Option<R>> {
    let is_popup_open = Popup::is_id_open(ui.ctx(), popup_id(button_id));
    let wrap_mode = combo.wrap_mode.unwrap_or_else(|| ui.wrap_mode());
    let close_behavior = combo.close_behavior.unwrap_or(PopupCloseBehavior::CloseOnClick);
    combo.set_class(ComboBox::CLASS_OPEN, is_popup_open);
    let ComboBox { label, selected_text, width, height, icon, popup_style, classes, .. } = combo;

    // Built from the same atoms and style as a `Button`, so the two line up when put side by side.
    let ButtonStyle { atom_layout: atom_layout_style } = ui.widget_style(button_id, &classes);

    // The visuals passed to the icon painter, from the interaction state at the start of the pass.
    let visuals = if is_popup_open {
        ui.visuals().widgets.open
    } else if let Some(response) = ui.ctx().read_response(button_id) {
        *ui.style().interact(&response)
    } else {
        ui.visuals().widgets.inactive
    };

    let min_width = width.unwrap_or_else(|| ui.spacing().combo_width);
    let min_size = Vec2::new(min_width, 0.0).at_least(atom_layout_style.min_size);

    let icon = RefCell::new(icon);
    let rtl = ui.style().right_to_left;
    let icon_atom = Atom::paint(Vec2::splat(ui.spacing().icon_width), move |ui, args| {
        let rect = args.rect.expand(visuals.expansion);
        if let Some(icon) = icon.borrow_mut().take() {
            icon(ui, rect, &visuals, is_popup_open);
        } else {
            paint_default_icon(ui.painter(), rect, &visuals);
        }
    })
    .atom_grow(true)
    .atom_align(if rtl { Align2::LEFT_CENTER } else { Align2::RIGHT_CENTER });
    let direction = if rtl { Direction::RightToLeft } else { Direction::LeftToRight };

    let accessible_text = selected_text.text().map(String::from).unwrap_or_default();
    let button = atom_layout_style
        .apply(
            WidgetAtom::new((selected_text, icon_atom))
                .id(button_id)
                .sense(Sense::click())
                .fallback_font(TextStyle::Button)
                .wrap_mode(wrap_mode)
                .direction(direction),
        )
        .min_size(min_size);

    let (button_response, label_response) = match label {
        Some(label) => {
            let label_text = label.text().map(String::from).unwrap_or_default();
            let outer_response =
                WidgetAtom::new((Atom::widget(button), label)).gap(ui.spacing().item_spacing.x).direction(direction).show(ui).response;
            outer_response.widget_info(|| WidgetInfo::labeled(Role::Label, ui.is_enabled(), &label_text));
            let button_response = ui.ctx().read_response(button_id).unwrap_or_else(|| outer_response.clone());
            (button_response, Some((outer_response, !label_text.is_empty())))
        }
        None => (button.show(ui).response, None),
    };

    button_response.widget_info(|| {
        let mut info = WidgetInfo::new(Role::ComboBox);
        info.enabled = ui.is_enabled();
        info.current_text_value = Some(accessible_text.clone());
        info
    });

    let popup = Popup::menu(&button_response).id(popup_id(button_id));
    let mut menu = Menu::with_popup(popup, ui, &button_response).style(popup_style).close_behavior(close_behavior);
    if let Some(height) = height {
        menu = menu.height(height);
    }
    let inner = open(menu);

    InnerResponse {
        inner,
        response: match label_response {
            // An empty label names nothing, so the caller names the combo box.
            Some((label_response, true)) => button_response.labelled_by(label_response.id) | label_response,
            Some((label_response, false)) => button_response | label_response,
            None => button_response,
        },
    }
}

fn paint_default_icon(painter: &Painter, rect: Rect, visuals: &WidgetVisuals) {
    let rect = Rect::from_center_size(rect.center(), vec2(rect.width() * 0.7, rect.height() * 0.45));
    painter.add(Shape::rotated_triangle(rect, 0.0, visuals.fg_stroke.color));
}

/// The menu of a combo box, under a field an app draws itself: [`ComboBox`]'s menu without the
/// button. A click on the field opens and closes it, or the caller does with [`Self::open`]; the
/// menu is at least as wide as the field.
///
/// The menu opens below the field, or above it when there is more room there, and stays on that
/// side while it is open. It is never taller than the room on that side: a long list scrolls.
///
/// Searchable menus use the header forms ([`Self::show_ui_with_header`] and the rest): the header
/// stays in place and only the list scrolls. Anything fixed above the list goes in the header,
/// such as a "none" row above the search field; anything fixed below it goes in the footer.
/// [`Self::show_rows_with_header`] is for rows of one height; a list of rows of different
/// heights, or a tree, uses [`Self::show_ui_with_header`].
///
/// ```
/// # egui::__run_test_ui(|ui| {
/// let field = ui.button("Font");
/// craft_ui::combo::Menu::new(ui, &field).height(400.0).show_ui(|ui| ui.label("Arial"));
/// # });
/// ```
#[must_use = "You should call .show*"]
pub struct Menu<'a> {
    popup: Popup<'a>,
    ctx: Context,
    field: Rect,
    field_id: Id,
    height: f32,
    style: StyleModifier,
    close_behavior: PopupCloseBehavior,
    open: Option<&'a mut bool>,
}

impl<'a> Menu<'a> {
    /// The menu under `field`, a widget in `ui`.
    pub fn new(ui: &Ui, field: &Response) -> Self {
        Self::with_popup(Popup::menu(field), ui, field)
    }

    fn with_popup(popup: Popup<'a>, ui: &Ui, field: &Response) -> Self {
        // Below the field or above it, never beside it.
        let flipped: &'static [RectAlign] = if ui.ctx().global_style().right_to_left { &[RectAlign::TOP_END] } else { &[RectAlign::TOP_START] };
        Self {
            popup: popup.align_alternatives(flipped),
            ctx: field.ctx.clone(),
            field: field.rect,
            field_id: field.id,
            height: ui.spacing().combo_height,
            style: StyleModifier::from(egui::containers::menu::menu_style),
            close_behavior: PopupCloseBehavior::CloseOnClick,
            open: None,
        }
    }

    /// The maximum height of the menu's contents, inside its frame, kept to the room on its side
    /// of the field. Default: [`egui::Spacing::combo_height`].
    #[inline]
    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// When the menu closes. Default: [`PopupCloseBehavior::CloseOnClick`].
    #[inline]
    pub fn close_behavior(mut self, close_behavior: PopupCloseBehavior) -> Self {
        self.close_behavior = close_behavior;
        self
    }

    /// Shows the menu while `*open`, for a field that isn't a button, such as a text field that
    /// searches the menu while it keeps the keyboard focus. The caller sets `*open` to open the
    /// menu, and the menu clears it when it closes: on its close behavior, Escape,
    /// [`Ui::close`] inside it, or the field losing the focus to anything but the menu. Clicks on
    /// the field neither open nor close it. The menu doesn't take the focus.
    ///
    /// ```
    /// # egui::__run_test_ui(|ui| {
    /// # let (mut text, mut open) = (String::new(), false);
    /// let field = ui.text_edit_singleline(&mut text);
    /// open |= field.gained_focus() || field.changed();
    /// craft_ui::combo::Menu::new(ui, &field).open(&mut open).show_ui(|ui| ui.label("Arial"));
    /// # });
    /// ```
    #[inline]
    pub fn open(mut self, open: &'a mut bool) -> Self {
        self.open = Some(open);
        self
    }

    /// The menu's style. Default: egui's menu style.
    #[inline]
    pub fn style(mut self, style: impl Into<StyleModifier>) -> Self {
        self.style = style.into();
        self
    }

    /// Shows `add` in the menu while it is open, scrolling when it is taller than the menu.
    /// Returns what `add` returned, or `None` while the menu is closed.
    pub fn show_ui<R>(self, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
        self.show(|_| (), |scroll, ui, ()| scroll.show(ui, add).inner, None::<fn(&mut Ui)>).map(|(r, _)| r)
    }

    /// Shows `header` at the top of the menu and `list` under it while the menu is open, and
    /// returns what `list` returned. `list` gets what `header` returned, such as the rows a
    /// search field's text matches. Only `list` scrolls; the header stays in place. The menu is
    /// as tall as both, up to its height. `list` lays out in a vertical scroll area: give it
    /// rows, not a scroll area of its own.
    pub fn show_ui_with_header<H, R>(self, header: impl FnOnce(&mut Ui) -> H, list: impl FnOnce(&mut Ui, H) -> R) -> Option<R> {
        self.show(header, |scroll, ui, h| scroll.show(ui, |ui| list(ui, h)).inner, None::<fn(&mut Ui)>).map(|(r, _)| r)
    }

    /// Like [`Self::show_ui_with_header`], with a list of `total_rows(&header's)` rows, each
    /// `row_height` tall without the item spacing, that only lays out the rows in view: `rows`
    /// gets the range to add, as in [`ScrollArea::show_rows`], and what `header` returned. For
    /// rows of one height only.
    pub fn show_rows_with_header<H, R>(
        self,
        header: impl FnOnce(&mut Ui) -> H,
        row_height: f32,
        total_rows: impl FnOnce(&H) -> usize,
        rows: impl FnOnce(&mut Ui, Range<usize>, H) -> R,
    ) -> Option<R> {
        self.show(header, rows_list(row_height, total_rows, rows), None::<fn(&mut Ui)>).map(|(r, _)| r)
    }

    /// Like [`Self::show_ui_with_header`], with `footer` (an "Add…" row) fixed under the list.
    /// Returns what `list` and `footer` returned.
    pub fn show_ui_with_header_footer<H, R, F>(
        self,
        header: impl FnOnce(&mut Ui) -> H,
        list: impl FnOnce(&mut Ui, H) -> R,
        footer: impl FnOnce(&mut Ui) -> F,
    ) -> Option<(R, F)> {
        self.show(header, |scroll, ui, h| scroll.show(ui, |ui| list(ui, h)).inner, Some(footer)).and_then(|(r, f)| Some((r, f?)))
    }

    /// Like [`Self::show_rows_with_header`], with `footer` fixed under the list.
    pub fn show_rows_with_header_footer<H, R, F>(
        self,
        header: impl FnOnce(&mut Ui) -> H,
        row_height: f32,
        total_rows: impl FnOnce(&H) -> usize,
        rows: impl FnOnce(&mut Ui, Range<usize>, H) -> R,
        footer: impl FnOnce(&mut Ui) -> F,
    ) -> Option<(R, F)> {
        self.show(header, rows_list(row_height, total_rows, rows), Some(footer)).and_then(|(r, f)| Some((r, f?)))
    }

    fn show<H, R, F>(
        self,
        header: impl FnOnce(&mut Ui) -> H,
        list: impl FnOnce(ScrollArea, &mut Ui, H) -> R,
        footer: Option<impl FnOnce(&mut Ui) -> F>,
    ) -> Option<(R, Option<F>)> {
        let Self { popup, ctx, field, field_id, height, style, mut close_behavior, open } = self;
        let id = popup.get_id();
        let popup = match open {
            Some(open) => {
                let focus_key = id.with("craft-field-focus");
                let focused = ctx.memory(|m| m.has_focus(field_id));
                let had_focus = ctx.data_mut(|d| std::mem::replace(d.get_temp_mut_or_default::<bool>(focus_key), focused));
                if had_focus && !focused && !moved_into(&ctx, id) {
                    *open = false;
                }
                // A click in the field (moving a text field's cursor) leaves the menu open.
                if ctx.input(|i| i.pointer.interact_pos()).is_some_and(|pos| field.contains(pos)) {
                    close_behavior = PopupCloseBehavior::IgnoreClicks;
                }
                popup.open_bool(open)
            }
            None => popup,
        };
        let footer_id = id.with("craft-footer-height");
        let remeasure_id = id.with("craft-remeasure");
        let remeasure = ctx.data_mut(|d| d.remove_temp::<bool>(remeasure_id)).unwrap_or(false);
        let (align, settled) = side(&popup, remeasure);
        // At most two remeasures in a row, so a footer that changes height on every pass settles.
        // A pass where the footer keeps its height clears the count.
        let count_id = id.with("craft-remeasures");
        if ctx.read_response(id).is_none() {
            ctx.data_mut(|d| d.remove::<u8>(count_id));
        }
        let remeasures = ctx.data(|d| d.get_temp::<u8>(count_id)).unwrap_or(0);
        // Until the side is settled, the menu is measured against the whole screen.
        let room = match popup.get_anchor_rect() {
            Some(anchor) if settled => room(align, anchor, ctx.content_rect()),
            _ => ctx.content_rect().height(),
        };
        let mut popup_style = (*ctx.global_style()).clone();
        style.apply(&mut popup_style);
        let frame = Frame::popup(&popup_style).total_margin().sum().y;
        let height = height.min(room - frame).max(0.0);
        let inner = popup
            .align(align)
            .align_alternatives(&[])
            .width(field.width())
            .style(style)
            .close_behavior(close_behavior)
            .show(|ui| {
                // The pass that measures the menu before its side is settled: drawn at a side that
                // may change and at a height the room doesn't cap. Laid out and interactive (a
                // search field takes the focus), but not seen.
                if remeasure && !settled {
                    ui.multiply_opacity(0.0);
                }
                ui.set_min_width(ui.available_width());
                // The popup offers last pass's height. Offer the full height instead: the scroll
                // area takes what the header and footer leave of it, then shrinks to the list.
                ui.set_max_height(height);
                let header = header(ui);
                // The footer comes after the list; keep the room it took last pass.
                let footer_height = ui.data(|d| d.get_temp::<f32>(footer_id));
                let footer_room = if footer.is_some() { footer_height.unwrap_or(0.0) + ui.spacing().item_spacing.y } else { 0.0 };
                // Labels widen a narrow menu rather than wrap early, as in egui's combo box. A
                // list left with no room keeps ScrollArea's minimum scrolled height.
                ui.style_mut().wrap_mode = Some(TextWrapMode::Extend);
                let list = list(ScrollArea::vertical().max_height((ui.available_height() - footer_room).max(0.0)), ui, header);
                let footer = footer.map(|footer| {
                    let top = ui.cursor().top();
                    let f = footer(ui);
                    let height = ui.min_rect().bottom() - top;
                    let changed = footer_height.is_none_or(|h| (h - height).abs() > 0.5);
                    if !changed && remeasures > 0 {
                        ui.data_mut(|d| d.remove::<u8>(count_id));
                    }
                    if changed {
                        // Recorded even past the cap, so "unchanged" means unchanged since the last pass.
                        ui.data_mut(|d| d.insert_temp(footer_id, height));
                    }
                    if changed && remeasures < 2 {
                        // The list took the wrong room: lay the menu out again next pass. Not a
                        // discarded pass: the header's side effects (focusing a search field on
                        // open) would run twice.
                        ui.data_mut(|d| {
                            d.insert_temp(remeasure_id, true);
                            d.insert_temp(count_id, remeasures + 1);
                        });
                        ui.ctx().request_repaint();
                    }
                    f
                });
                (list, footer)
            })
            .map(|r| r.inner);
        if inner.is_some() {
            ctx.accesskit_node_builder(id, |node| node.set_role(Role::ListBox));
        }
        inner
    }
}

fn rows_list<H, R>(
    row_height: f32,
    total_rows: impl FnOnce(&H) -> usize,
    rows: impl FnOnce(&mut Ui, Range<usize>, H) -> R,
) -> impl FnOnce(ScrollArea, &mut Ui, H) -> R {
    move |scroll, ui, h| {
        let total = total_rows(&h);
        scroll.show_rows(ui, row_height, total, |ui, range| rows(ui, range, h)).inner
    }
}

/// Whether the pointer or the keyboard focus went into the menu `popup` this pass.
fn moved_into(ctx: &Context, popup: Id) -> bool {
    // The pointer counts only on a press or click: a resting mouse says nothing about focus.
    let pointer = ctx.input(|i| (i.pointer.any_pressed() || i.pointer.any_click()).then(|| i.pointer.interact_pos())).flatten();
    let in_menu = ctx.memory(|m| m.area_rect(popup)).zip(pointer).is_some_and(|(menu, pos)| menu.contains(pos));
    let focused = ctx.memory(|m| m.focused()).and_then(|id| ctx.read_response(id));
    in_menu || focused.is_some_and(|r| r.layer_id == LayerId::new(Order::Foreground, popup))
}

/// The side of its field `popup` shows on, and whether it is settled. The side is settled on the
/// first pass after the menu was shown and measured (`remeasure` when that measure is stale): the
/// side egui picks when the menu fits there, else the one with more room. It is kept while the
/// menu stays open, so a menu that shrinks or grows while open doesn't jump to the other side.
fn side(popup: &Popup<'_>, remeasure: bool) -> (RectAlign, bool) {
    let ctx = popup.ctx();
    let key = popup.get_id().with("craft-side");
    if !popup.is_open() || ctx.read_response(popup.get_id()).is_none() {
        ctx.data_mut(|d| d.remove::<RectAlign>(key));
        return (popup.get_best_align(), false);
    }
    if let Some(align) = ctx.data(|d| d.get_temp::<RectAlign>(key)) {
        return (align, true);
    }
    if remeasure {
        return (popup.get_best_align(), false);
    }
    // Outside the data lock: these read the popup's state from memory.
    let mut align = popup.get_best_align();
    if let (Some(size), Some(anchor)) = (popup.get_expected_size(), popup.get_anchor_rect()) {
        let screen = ctx.content_rect();
        let flipped = align.flipped_y();
        if room(align, anchor, screen) < size.y && room(flipped, anchor, screen) > room(align, anchor, screen) {
            align = flipped;
        }
    }
    ctx.data_mut(|d| d.insert_temp(key, align));
    (align, true)
}

/// The height a menu on the `align` side of `anchor` has inside `screen`.
fn room(align: RectAlign, anchor: Rect, screen: Rect) -> f32 {
    match (align.parent.y(), align.child.y()) {
        (Align::Max, Align::Min) => screen.bottom() - anchor.bottom(),
        (Align::Min, Align::Max) => anchor.top() - screen.top(),
        _ => screen.height(),
    }
}

#[cfg(test)]
mod tests;
