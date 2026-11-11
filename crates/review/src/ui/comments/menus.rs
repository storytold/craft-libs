//! The Sort by, Filter, Pages and card menus.

use craft_behavior::IconId;
use craft_ui::buttons::{Button, IconButton};
use craft_ui::contract::{Props as _, Show as _, ShowContainer as _, Trigger};
use craft_ui::display::Divider;
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::Label;
use egui::{Response, Ui};

use super::{CardMenu, CommentsPanelState, CommentsWords, FilterRow, color_name, kind_name, status_label, toggle};
use crate::annotate::AnnotationKind;
use crate::comments::{CommentSort, CommentThread, ReviewStatus};
use crate::ui::annotate::AnnotateWords;

/// The Sort by button and its menu.
pub(super) fn sort_menu(ui: &mut Ui, words: &CommentsWords, state: &mut CommentsPanelState) {
    let sorts = [
        (CommentSort::Page, &words.sort_page),
        (CommentSort::Author, &words.sort_author),
        (CommentSort::Date, &words.sort_date),
        (CommentSort::Type, &words.sort_type),
        (CommentSort::Color, &words.sort_color),
    ];
    let current = sorts.iter().find(|(sort, _)| *sort == state.sort).map_or("", |(_, label)| label.as_str());
    let name = words.sort_by.fill(&[("sort", current)]);
    let button = Button::new(name.clone()).trailing_icon(craft_ui::controls::disclosure::DisclosureGlyph::menu_button()).show(ui);
    let mut chosen = None;
    let active = state.sort;
    Menu::new(name).opened_by(&button, Trigger::Click).show(ui, |ui, _| {
        for (sort, label) in &sorts {
            if MenuItem::new(label.as_str().to_owned()).radio_mark(active == *sort).show(ui).ran {
                chosen = Some(*sort);
            }
        }
    });
    if let Some(sort) = chosen {
        state.sort = sort;
    }
}

/// The funnel and its menu: a submenu per property that lists its values as checks (checked = shown), and
/// Show all while anything is hidden.
pub(super) fn filter_menu(ui: &mut Ui, words: &CommentsWords, kinds: &AnnotateWords, threads: &[CommentThread], state: &mut CommentsPanelState) {
    let active = state.filter.is_active();
    let button = IconButton::new(IconSpec::svg(IconId::new("funnel"))).label(words.filter.clone()).selected(active).show(ui);
    let mut kind_rows: Vec<Option<AnnotationKind>> = threads.iter().map(|thread| thread.kind).collect();
    kind_rows.sort_by_cached_key(|kind| kind_name(words, kinds, *kind));
    kind_rows.dedup();
    let mut authors: Vec<&str> = threads.iter().map(|thread| thread.messages.first().map_or("", |first| first.author.as_str())).collect();
    authors.sort_unstable();
    authors.dedup();
    let mut colors: Vec<Option<[u8; 3]>> = threads.iter().map(|thread| thread.color).collect();
    colors.sort_unstable();
    colors.dedup();
    let filter = state.filter.clone();
    let mut ran: Option<FilterRow> = None;
    let check = |ui: &mut Ui, ran: &mut Option<FilterRow>, label: String, row: FilterRow, hidden: bool| {
        if MenuItem::new(label).check(!hidden).keep_open(true).show(ui).ran {
            *ran = Some(row);
        }
    };
    Menu::new(words.filter.clone()).opened_by(&button, Trigger::Click).show(ui, |ui, _| {
        let _ = Menu::submenu_row(ui, words.filter_type.clone(), |ui, _| {
            for &kind in &kind_rows {
                check(ui, &mut ran, kind_name(words, kinds, kind), FilterRow::Type(kind), filter.hidden_types.contains(&kind));
            }
        });
        let _ = Menu::submenu_row(ui, words.filter_author.clone(), |ui, _| {
            for &author in &authors {
                let name = if author.is_empty() { words.unknown_author.as_str().to_owned() } else { author.to_owned() };
                check(ui, &mut ran, name, FilterRow::Author(author.to_owned()), filter.hidden_authors.iter().any(|hidden| hidden == author));
            }
        });
        let _ = Menu::submenu_row(ui, words.filter_status.clone(), |ui, _| {
            for status in ReviewStatus::MENU {
                check(
                    ui,
                    &mut ran,
                    status_label(words, status).as_str().to_owned(),
                    FilterRow::Status(status),
                    filter.hidden_statuses.contains(&status),
                );
            }
        });
        let _ = Menu::submenu_row(ui, words.filter_color.clone(), |ui, _| {
            for &color in &colors {
                let name = color.map_or_else(|| words.no_color.as_str().to_owned(), |color| color_name(kinds, color));
                check(ui, &mut ran, name, FilterRow::Color(color), filter.hidden_colors.contains(&color));
            }
        });
        let _ = Menu::submenu_row(ui, words.filter_check.clone(), |ui, _| {
            for (marked, name) in [(true, &words.checked), (false, &words.unchecked)] {
                check(ui, &mut ran, name.as_str().to_owned(), FilterRow::Check(marked), filter.hidden_checks.contains(&marked));
            }
        });
        if active {
            Divider::horizontal().show(ui);
            if MenuItem::new(words.show_all.as_str().to_owned()).show(ui).ran {
                ran = Some(FilterRow::ShowAll);
            }
        }
    });
    let filter = &mut state.filter;
    match ran {
        Some(FilterRow::Type(kind)) => toggle(&mut filter.hidden_types, kind),
        Some(FilterRow::Author(author)) => toggle(&mut filter.hidden_authors, author),
        Some(FilterRow::Status(status)) => toggle(&mut filter.hidden_statuses, status),
        Some(FilterRow::Color(color)) => toggle(&mut filter.hidden_colors, color),
        Some(FilterRow::Check(marked)) => toggle(&mut filter.hidden_checks, marked),
        Some(FilterRow::ShowAll) => filter.clear(),
        None => {}
    }
}

/// Collapse all pages and Expand all pages, shown under the Page sort.
pub(super) fn pages_menu(ui: &mut Ui, words: &CommentsWords, threads: &[CommentThread], state: &mut CommentsPanelState) {
    let button = IconButton::new(IconSpec::svg(IconId::new("more"))).label(words.pages_menu.clone()).show(ui);
    let mut collapse = None;
    Menu::new(words.pages_menu.clone()).opened_by(&button, Trigger::Click).show(ui, |ui, _| {
        if MenuItem::new(words.collapse_all.as_str().to_owned()).show(ui).ran {
            collapse = Some(true);
        }
        if MenuItem::new(words.expand_all.as_str().to_owned()).show(ui).ran {
            collapse = Some(false);
        }
    });
    match collapse {
        Some(true) => {
            let mut pages: Vec<usize> = threads.iter().filter_map(|thread| thread.anchor.page()).collect();
            pages.sort_unstable();
            pages.dedup();
            state.collapsed_pages = pages;
        }
        Some(false) => state.collapsed_pages.clear(),
        None => {}
    }
}

/// The menu of message `index` of `thread`, hanging from the message's "..." button.
pub(super) fn card_menu(
    ui: &mut Ui,
    words: &CommentsWords,
    button: &Response,
    index: usize,
    thread: &CommentThread,
    editable: bool,
) -> Option<CardMenu> {
    let ran = std::cell::Cell::new(None);
    Menu::new(words.more.clone()).id_salt(index).opened_by(button, Trigger::Click).show(ui, |ui, _| {
        let row = |ui: &mut Ui, label: &Label, action: CardMenu| {
            if MenuItem::new(label.as_str().to_owned()).show(ui).ran {
                ran.set(Some(action));
            }
        };
        if index == 0 {
            row(ui, &words.edit, CardMenu::Edit);
            let _ = Menu::submenu_row(ui, words.set_status.clone(), |ui, _| {
                for status in ReviewStatus::MENU {
                    if MenuItem::new(status_label(words, status).as_str().to_owned()).radio_mark(thread.status == status).show(ui).ran {
                        ran.set(Some(CardMenu::Status(status)));
                    }
                }
            });
            Divider::horizontal().show(ui);
            row(ui, &words.delete, CardMenu::Delete);
        } else {
            if editable {
                row(ui, &words.edit, CardMenu::Edit);
            }
            row(ui, &words.copy_text, CardMenu::Copy);
            if editable {
                Divider::horizontal().show(ui);
                row(ui, &words.delete, CardMenu::Delete);
            }
        }
    });
    ran.into_inner()
}
