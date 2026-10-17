//! The two controls that offer more than one action.
//!
//! An overflow menu is the `…` that holds what did not earn a place on the
//! toolbar. A split button is a primary action with a chevron beside it, where
//! the chevron offers the variations. Share is the usual example. Most of the
//! time a person wants to share, and once in a while they want to copy a link
//! or start a call about the document instead.
//!
//! They are one module because they are the same control underneath. Building
//! them twice would mean getting the accessible naming right twice, and an
//! icon-only button that announces itself as an ellipsis character is useless
//! to anybody reading the screen rather than looking at it.
//!
//! Neither control knows what any item does. Each answers with the identifier
//! of the item a person chose. The application holds the meaning.

use egui::{Response, RichText, Ui};

/// One line in a menu.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuItem {
    /// What the caller recognises this by. Never shown.
    pub id: String,
    pub label: String,
    /// A second line, smaller. Worth having where the label alone leaves a
    /// person guessing what the action will do to their document.
    pub detail: Option<String>,
    /// A leading glyph. Decoration only, so it is never the accessible name.
    pub icon: Option<String>,
    pub enabled: bool,
    /// Draw a rule above this item. A group of related actions reads as a
    /// group only when something separates it from the last one.
    pub separator_before: bool,
}

impl MenuItem {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> MenuItem {
        MenuItem { id: id.into(), label: label.into(), enabled: true, ..MenuItem::default() }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> MenuItem {
        self.detail = Some(detail.into());
        self
    }

    pub fn icon(mut self, icon: impl Into<String>) -> MenuItem {
        self.icon = Some(icon.into());
        self
    }

    /// Present but unavailable, with a reason on hover.
    ///
    /// Greying an item out says the action exists and is not possible now.
    /// Removing it says the action does not exist. They are different facts and
    /// a person reads them differently.
    pub fn disabled(mut self, why: impl Into<String>) -> MenuItem {
        self.enabled = false;
        self.detail = Some(why.into());
        self
    }

    pub fn separated(mut self) -> MenuItem {
        self.separator_before = true;
        self
    }
}

/// What a person did to a split button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SplitHit {
    /// They pressed the button itself.
    Primary,
    /// They chose something from the chevron's menu.
    Item(String),
}

/// The accessible name of the chevron, and of the overflow button.
///
/// Public so an application can translate them. A glyph is not a name, and
/// these two are the only strings in this module that are not supplied by the
/// caller.
#[derive(Clone, Debug)]
pub struct MenuLabels {
    /// The `…` button.
    pub more: String,
    /// The chevron on a split button. `{action}` is replaced with the primary
    /// action's own label, so a screen reader says "more share options" rather
    /// than "more options" on a toolbar holding four of them.
    pub more_for: String,
}

impl Default for MenuLabels {
    fn default() -> MenuLabels {
        MenuLabels { more: "More options".into(), more_for: "More {action} options".into() }
    }
}

/// The `…` button and its menu.
///
/// Answers with the identifier of the item chosen, if any.
pub fn overflow(ui: &mut Ui, items: &[MenuItem], labels: &MenuLabels) -> Option<String> {
    // A horizontal ellipsis, not three periods. Three periods are three
    // characters to anything reading the screen aloud.
    let button = egui::Button::new(RichText::new("\u{2026}").size(15.0)).frame(false);
    let hit = ui.add(button);
    let name = labels.more.clone();
    hit.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name.as_str()));
    let hit = hit.on_hover_text(labels.more.as_str());
    items_of(&hit, items)
}

/// A primary action with a chevron beside it.
///
/// The two halves are two controls. Pressing the label does the thing. Pressing
/// the chevron offers the variations. One control that did both would leave a
/// person guessing which half they had hit.
pub fn split(ui: &mut Ui, primary: &str, items: &[MenuItem], labels: &MenuLabels) -> Option<SplitHit> {
    let mut hit = None;
    ui.horizontal(|ui| {
        // The halves touch, so they read as one control with two parts. Square
        // off the inside corners as well. Two default radii leave a pair of
        // notches at the seam, which reads as two buttons that happen to abut.
        ui.spacing_mut().item_spacing.x = 0.0;
        let radius = ui.visuals().widgets.inactive.corner_radius;
        // A toolbar that fills from its right edge hands this control a
        // right-to-left layout, and whatever is added first lands furthest
        // right. The label still belongs before the chevron, so the halves are
        // added in the order that puts them there.
        let backwards = ui.layout().main_dir() == egui::Direction::RightToLeft;
        let (outer, inner) = if backwards {
            (egui::CornerRadius { nw: 0, sw: 0, ..radius }, egui::CornerRadius { ne: 0, se: 0, ..radius })
        } else {
            (egui::CornerRadius { ne: 0, se: 0, ..radius }, egui::CornerRadius { nw: 0, sw: 0, ..radius })
        };
        let label = |ui: &mut Ui, hit: &mut Option<SplitHit>| {
            if ui.add(egui::Button::new(primary).corner_radius(if backwards { inner } else { outer })).clicked() {
                *hit = Some(SplitHit::Primary);
            }
        };
        let chevron = |ui: &mut Ui, hit: &mut Option<SplitHit>| {
            // Room for the triangle, which is painted rather than typed.
            let more = ui.add(egui::Button::new("    ").corner_radius(if backwards { outer } else { inner }));
            crate::buttons::chevron(ui, more.rect, ui.visuals().text_color());
            let name = labels.more_for.replace("{action}", &primary.to_lowercase());
            more.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name.as_str()));
            let more = more.on_hover_text(name.as_str());
            if let Some(id) = items_of(&more, items) {
                *hit = Some(SplitHit::Item(id));
            }
        };
        if backwards {
            chevron(ui, &mut hit);
            label(ui, &mut hit);
        } else {
            label(ui, &mut hit);
            chevron(ui, &mut hit);
        }
    });
    hit
}

/// The menu both controls open.
fn items_of(anchor: &Response, items: &[MenuItem]) -> Option<String> {
    let mut chosen = None;
    egui::Popup::menu(anchor).show(|ui| {
        ui.set_min_width(210.0);
        let weak = ui.visuals().weak_text_color();
        for item in items {
            if item.separator_before {
                ui.separator();
            }
            let label = match &item.icon {
                Some(icon) => format!("{icon}  {}", item.label),
                None => item.label.clone(),
            };
            let row = ui.add_enabled(item.enabled, egui::Button::new(label).frame(false));
            // The icon is decoration. The name is the label alone.
            let name = item.label.clone();
            let enabled = item.enabled;
            row.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name.as_str()));
            if row.clicked() {
                chosen = Some(item.id.clone());
                ui.close();
            }
            if let Some(detail) = &item.detail {
                ui.label(RichText::new(detail.as_str()).small().color(weak));
                ui.add_space(2.0);
            }
        }
    });
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_is_enabled_until_it_is_not() {
        assert!(MenuItem::new("copy", "Copy link").enabled);
        let off = MenuItem::new("copy", "Copy link").disabled("this server forbids public links");
        assert!(!off.enabled);
        assert_eq!(off.detail.as_deref(), Some("this server forbids public links"));
    }

    #[test]
    fn the_chevrons_name_says_which_action_it_belongs_to() {
        let labels = MenuLabels::default();
        assert_eq!(labels.more_for.replace("{action}", "share"), "More share options");
    }

    #[test]
    fn a_builder_keeps_what_it_was_given() {
        let item = MenuItem::new("talk", "Work together").icon("\u{1f4de}").detail("Start a call").separated();
        assert_eq!(item.id, "talk");
        assert_eq!(item.label, "Work together");
        assert_eq!(item.icon.as_deref(), Some("\u{1f4de}"));
        assert_eq!(item.detail.as_deref(), Some("Start a call"));
        assert!(item.separator_before);
        assert!(item.enabled);
    }
}
