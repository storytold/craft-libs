//! The Sort by, Filter, Pages, More and card menus.

use craft_behavior::{IconId, Value};
use craft_ui::buttons::{Button, IconButton};
use craft_ui::contract::{Entry, Props as _, Show as _, ShowContainer as _, Trigger};
use craft_ui::display::Divider;
use craft_ui::menu::{Menu, MenuItem, entry_rows};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label};
use egui::{Response, Ui};

use super::{CardMenu, CommentsPanelState, CommentsWords, FilterRow, color_name, kind_name, status_label, toggle};
use crate::annotate::AnnotationKind;
use crate::comments::{CommentSort, CommentThread, ReviewStatus};
use crate::ui::annotate::AnnotateWords;

/// The sorts the Sort by rows offer, in Acrobat's order, with their names.
fn sorts(words: &CommentsWords) -> [(CommentSort, &Label); 5] {
    [
        (CommentSort::Page, &words.sort_page),
        (CommentSort::Author, &words.sort_author),
        (CommentSort::Date, &words.sort_date),
        (CommentSort::Type, &words.sort_type),
        (CommentSort::Color, &words.sort_color),
    ]
}

/// The Sort by radio rows, `active` marked; the sort picked.
fn sort_rows(ui: &mut Ui, words: &CommentsWords, active: CommentSort) -> Option<CommentSort> {
    let mut chosen = None;
    for (sort, label) in sorts(words) {
        if MenuItem::new(label.as_str().to_owned()).radio_mark(active == sort).show(ui).ran {
            chosen = Some(sort);
        }
    }
    chosen
}

/// The Collapse all pages and Expand all pages rows; `Some(true)` when Collapse ran, `Some(false)` when Expand did.
fn pages_rows(ui: &mut Ui, words: &CommentsWords) -> Option<bool> {
    let mut collapse = None;
    if MenuItem::new(words.collapse_all.as_str().to_owned()).show(ui).ran {
        collapse = Some(true);
    }
    if MenuItem::new(words.expand_all.as_str().to_owned()).show(ui).ran {
        collapse = Some(false);
    }
    collapse
}

/// Folds every page of `threads` when `collapse`, else unfolds them all.
fn fold_pages(threads: &[CommentThread], state: &mut CommentsPanelState, collapse: bool) {
    if collapse {
        let mut pages: Vec<usize> = threads.iter().filter_map(|thread| thread.anchor.page()).collect();
        pages.sort_unstable();
        pages.dedup();
        state.collapsed_pages = pages;
    } else {
        state.collapsed_pages.clear();
    }
}

/// The Sort by button and its menu.
pub(super) fn sort_menu(ui: &mut Ui, words: &CommentsWords, state: &mut CommentsPanelState) {
    let current = sorts(words).into_iter().find(|(sort, _)| *sort == state.sort).map_or("", |(_, label)| label.as_str());
    let name = words.sort_by.fill(&[("sort", current)]);
    let button = Button::new(name.clone()).trailing_icon(craft_ui::controls::disclosure::DisclosureGlyph::menu_button()).show(ui);
    let mut chosen = None;
    let active = state.sort;
    Menu::new(name).opened_by(&button, Trigger::Click).show(ui, |ui, _| {
        chosen = sort_rows(ui, words, active);
    });
    if let Some(sort) = chosen {
        state.sort = sort;
    }
}

/// The funnel and its menu.
pub(super) fn filter_menu(ui: &mut Ui, words: &CommentsWords, kinds: &AnnotateWords, threads: &[CommentThread], state: &mut CommentsPanelState) {
    let active = state.filter.is_active();
    let button = IconButton::new(IconSpec::svg(IconId::new("funnel"))).label(words.filter.clone()).selected(active).show(ui);
    filter_popup(ui, &button, words, kinds, threads, state);
}

/// The filter menu hanging from `opener`: a submenu per property that lists its values as checks (checked = shown),
/// and Show all while anything is hidden.
fn filter_popup(
    ui: &mut Ui,
    opener: &Response,
    words: &CommentsWords,
    kinds: &AnnotateWords,
    threads: &[CommentThread],
    state: &mut CommentsPanelState,
) {
    let active = state.filter.is_active();
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
    Menu::new(words.filter.clone()).opened_by(opener, Trigger::Click).show(ui, |ui, _| {
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
        collapse = pages_rows(ui, words);
    });
    if let Some(collapse) = collapse {
        fold_pages(threads, state, collapse);
    }
}

/// The panel's More menu for a [`Hosted`](super::CommentsHeader::Hosted) header, hanging from the host's `opener`:
/// the Sort by rows, Collapse all pages and Expand all pages under the Page sort, then `host_rows` behind a
/// separator. It applies its own rows to `state` and returns the value of the host row picked. The `Own` header
/// shows the same sort and page rows from its Sort by and Pages buttons.
pub fn comments_more_menu<H: Value>(
    ui: &mut Ui,
    opener: &Response,
    threads: &[CommentThread],
    state: &mut CommentsPanelState,
    host_rows: &[Entry<H>],
) -> Option<H> {
    let words = Env::of(ui).words::<CommentsWords>();
    let active = state.sort;
    let (mut sort, mut collapse, mut ran) = (None, None, None);
    Menu::new(words.more.clone()).opened_by(opener, Trigger::Click).show(ui, |ui, _| {
        MenuItem::heading(words.sort_heading.clone()).show(ui);
        sort = sort_rows(ui, &words, active);
        if active == CommentSort::Page {
            collapse = pages_rows(ui, &words);
        }
        if !host_rows.is_empty() {
            Divider::horizontal().show(ui);
            ran = entry_rows(ui, host_rows);
        }
    });
    if let Some(sort) = sort {
        state.sort = sort;
    }
    if let Some(collapse) = collapse {
        fold_pages(threads, state, collapse);
    }
    ran.and_then(|at| host_rows.get(at)).and_then(Entry::value).cloned()
}

/// The panel's filter menu over `threads` for a [`Hosted`](super::CommentsHeader::Hosted) header, hanging from the
/// host's `opener`: a submenu per property (Type, Author, Status, Color, Checkmark) whose checks show or hide what
/// they name, and Show all while anything is hidden. The `Own` header's funnel opens the same menu.
pub fn comments_filter_menu(ui: &mut Ui, opener: &Response, threads: &[CommentThread], state: &mut CommentsPanelState) {
    let env = Env::of(ui);
    let (words, kinds) = (env.words::<CommentsWords>(), env.words::<AnnotateWords>());
    filter_popup(ui, opener, &words, &kinds, threads, state);
}

/// The menu of message `index` of `thread`, hanging from the message's "..." button; the host's `host_rows` follow
/// the card's rows behind a separator.
pub(super) fn card_menu<H>(
    ui: &mut Ui,
    words: &CommentsWords,
    button: &Response,
    index: usize,
    thread: &CommentThread,
    editable: bool,
    host_rows: &[Entry<H>],
) -> Option<CardMenu> {
    let ran = std::cell::Cell::new(None);
    Menu::new(words.more.clone()).id_salt(index).opened_by(button, Trigger::Click).show(ui, |ui, _| {
        let row = |ui: &mut Ui, label: &Label, action: CardMenu, enabled: bool| {
            if MenuItem::new(label.as_str().to_owned()).enabled(enabled).show(ui).ran {
                ran.set(Some(action));
            }
        };
        if index == 0 {
            row(ui, &words.edit, CardMenu::Edit, editable && !thread.locked);
            let _ = Menu::submenu_row(ui, words.set_status.clone(), |ui, _| {
                for status in ReviewStatus::MENU {
                    if MenuItem::new(status_label(words, status).as_str().to_owned()).radio_mark(thread.status() == status).show(ui).ran {
                        ran.set(Some(CardMenu::Status(status)));
                    }
                }
            });
            Divider::horizontal().show(ui);
            // Deleting the root also deletes its descendants; the host's flag covers that whole operation.
            row(ui, &words.delete, CardMenu::Delete, editable && !thread.locked && thread.messages.first().is_some_and(|message| message.deletable));
        } else {
            if editable {
                row(ui, &words.edit, CardMenu::Edit, true);
            }
            row(ui, &words.copy_text, CardMenu::Copy, true);
            if editable {
                Divider::horizontal().show(ui);
                row(ui, &words.delete, CardMenu::Delete, thread.messages.get(index).is_some_and(|message| message.deletable));
            }
        }
        if !host_rows.is_empty() {
            Divider::horizontal().show(ui);
            if let Some(at) = entry_rows(ui, host_rows) {
                ran.set(Some(CardMenu::Host(at)));
            }
        }
    });
    ran.into_inner()
}
