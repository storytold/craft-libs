//! The collection rows of the 22.4 table: navigation keys, selection, type-ahead, rename, reorder, the empty state,
//! virtualisation and scrolling. Each case runs over the stories of a type of the `Items` shape and reads the rows
//! from the AccessKit tree and the events from `StoryOutput`.
//!
//! The collection's own settings (selection mode, `rename`, reorder rule) are not in the probe or the output, so a
//! case that applies only to a collection with a setting starts the gesture and checks the rest when the collection
//! answered it: a rename case runs on the stories whose F2 began a rename.

use super::nodes::{focused_row, position_in_set, root, rows, subtree};
use super::{Subject, row_click};
use crate::contract::ColumnCommands;
use crate::contract::ShapeKind;
use crate::runtime::access::Role;
use crate::runtime::commands::{self, FireSource};
use crate::runtime::egui::access::Declared;
use crate::runtime::egui::fixture::AccessActionKind;
use crate::runtime::focus::EndKey;
use crate::testing::fixture::{Fixture, Target};
use crate::testing::story::{CollectionKind, EditKind, Story};
use craft_host::target::Os;
use egui::{Key, Modifiers};

fn is_items(subject: &Subject) -> bool {
    matches!(subject.probe.shape, ShapeKind::Items)
}

/// A fixture of `story` on Linux keys, settled for input, and the rows it shows when there are at least `least` of
/// them.
fn with_rows(story: &Story, least: usize) -> Option<(Fixture, Target, Vec<Target>)> {
    let mut fixture = Fixture::story(story);
    fixture.os(Os::Linux);
    let fixture = super::drive::for_input(fixture);
    let container = root(&fixture)?;
    let shown = rows(&fixture, container.node);
    (shown.len() >= least).then_some((fixture, container, shown))
}

/// The collection events of every pass since the last call.
fn events(fixture: &mut Fixture) -> Vec<CollectionKind> {
    fixture.take_outputs().into_iter().flat_map(|output| output.collection).collect()
}

/// The place of the row holding the focus, in its set.
fn lead(fixture: &Fixture, container: &Target) -> Option<usize> {
    let shown = rows(fixture, container.node);
    let (_, key) = focused_row(fixture, &shown)?;
    position_in_set(fixture, key).map(|(at, _)| at)
}

/// Whether the focus is on a cell: the collection navigates a grid, where Home and End move along the row and the
/// first and last row are Ctrl+Home and Ctrl+End (17.4).
fn focus_on_cell(fixture: &Fixture) -> bool {
    let focused = fixture.tree().focused_key();
    fixture.by_role(Role::CELL).iter().any(|cell| cell.node == focused)
}

/// What End means for the focused stop, as its component declares it.
fn end_meaning(fixture: &Fixture) -> EndKey {
    crate::runtime::focus::focused_rec(fixture.ctx()).map_or(EndKey::LastItem, |rec| rec.stop.keys.end)
}

/// Whether the focused stop leaves the arrows to the component around the collection (`ItemKeys::Owner`).
fn owner_arrows(fixture: &Fixture) -> bool {
    crate::runtime::focus::focused_rec(fixture.ctx()).is_some_and(|rec| rec.stop.role.owner_arrows)
}

/// The row Ctrl+Home goes to, as the focused stop declares it.
fn home_row(fixture: &Fixture) -> usize {
    crate::runtime::focus::focused_rec(fixture.ctx()).map_or(0, |rec| rec.stop.role.home_row)
}

/// A click on the first row puts the lead there; the keys then move it: Down and Up by one row, End to the last,
/// Home to the first, Page Down forward. A grid takes Ctrl+End and Ctrl+Home for the last and first row.
pub(super) fn keys_move_the_lead(subject: &Subject) -> Result<(), String> {
    if !is_items(subject) {
        return Ok(());
    }
    for story in &subject.stories {
        let Some((mut fixture, container, shown)) = with_rows(story, 3) else { continue };
        let Some((_, size)) = position_in_set(&fixture, shown[0].node) else { continue };
        row_click::click(&mut fixture, &shown[0]);
        let name = story.name;
        let ends = if focus_on_cell(&fixture) { Modifiers::COMMAND } else { Modifiers::NONE };
        let expect = |fixture: &mut Fixture, key: Key, mods: Modifiers, at: usize, what: &str| -> Result<(), String> {
            fixture.key(key, mods);
            fixture.settle();
            match lead(fixture, &container) {
                Some(found) if found == at => Ok(()),
                found => Err(format!("{name}: after {what} the lead is {found:?}, expected {at}")),
            }
        };
        let first = lead(&fixture, &container).ok_or_else(|| format!("{name}: a click on the first row left no lead"))?;
        // Where the second item sits beside the first, the items wrap in rows and Down moves a whole row.
        let wrapped = shown.iter().skip(1).any(|other| (other.rect.min.y - shown[0].rect.min.y).abs() < shown[0].rect.height() / 2.0);
        if owner_arrows(&fixture) {
            // The arrows reach the owner's commands: they are no moves of the lead.
            fixture.key(Key::ArrowDown, Modifiers::NONE);
            fixture.settle();
            expect(&mut fixture, Key::ArrowUp, Modifiers::NONE, first, "Down and Up (the owner's arrows)")?;
        } else {
            fixture.key(Key::ArrowDown, Modifiers::NONE);
            fixture.settle();
            match lead(&fixture, &container) {
                Some(found) if found == first + 1 || (wrapped && found > first) => {}
                found => return Err(format!("{name}: after Down the lead is {found:?}, expected {}", first + 1)),
            }
            expect(&mut fixture, Key::ArrowUp, Modifiers::NONE, first, "Up")?;
        }
        match end_meaning(&fixture) {
            EndKey::LastItem => {
                expect(&mut fixture, Key::End, ends, size - 1, "End")?;
                expect(&mut fixture, Key::Home, ends, 0, "Home")?;
            }
            EndKey::EndMode => {
                // End alone arms the mode and leaves the lead; the next arrow jumps to the data edge, which can lie
                // outside the shown rows, where no row reports the lead.
                fixture.key(Key::End, Modifiers::NONE);
                fixture.settle();
                if lead(&fixture, &container) != Some(first) {
                    return Err(format!("{name}: End moved the lead, but it declares End mode"));
                }
                fixture.key(Key::ArrowDown, Modifiers::NONE);
                fixture.settle();
                let jumped = lead(&fixture, &container);
                if jumped == Some(first + 1) {
                    return Err(format!("{name}: End then Down moved the lead one row, expected the jump to the data edge"));
                }
                // Ctrl+End goes to the last used cell and scrolls it into view, so the lead is read again.
                fixture.key(Key::End, Modifiers::COMMAND);
                fixture.settle();
                match lead(&fixture, &container) {
                    Some(found) if found > first && jumped.is_none_or(|edge| found >= edge) => {}
                    found => {
                        return Err(format!(
                            "{name}: after Ctrl+End the lead is {found:?}, expected the last used cell (past row {first}, and not before {jumped:?})"
                        ));
                    }
                }
                let home = home_row(&fixture);
                expect(&mut fixture, Key::Home, Modifiers::COMMAND, home, "Ctrl+Home")?;
            }
        }
        fixture.key(Key::PageDown, Modifiers::NONE);
        fixture.settle();
        if lead(&fixture, &container).is_none_or(|found| found == 0) {
            return Err(format!("{name}: Page Down left the lead on the first row"));
        }
    }
    Ok(())
}

/// A click on a row that selects reports one selection event and shows that row selected.
pub(super) fn click_selects_one_row(subject: &Subject) -> Result<(), String> {
    if !is_items(subject) {
        return Ok(());
    }
    for story in &subject.stories {
        let Some((mut fixture, container, shown)) = with_rows(story, 2) else { continue };
        fixture.take_outputs();
        row_click::click(&mut fixture, &shown[1]);
        let kinds = events(&mut fixture);
        let selections = kinds.iter().filter(|kind| **kind == CollectionKind::Selection).count();
        if selections == 0 {
            continue;
        }
        if selections != 1 {
            return Err(format!("{}: one click gave {selections} selection events", story.name));
        }
        let now = rows(&fixture, container.node);
        if !now.iter().any(|row| row.node == shown[1].node && super::nodes::is_selected(&fixture, row.node)) {
            return Err(format!("{}: the clicked row does not report itself selected", story.name));
        }
    }
    Ok(())
}

/// Typing the first letter of a later row's name moves the lead to the first row that starts with it.
pub(super) fn type_ahead_moves_the_lead(subject: &Subject) -> Result<(), String> {
    if !is_items(subject) {
        return Ok(());
    }
    for story in &subject.stories {
        let Some((mut fixture, container, shown)) = with_rows(story, 3) else { continue };
        let names: Vec<Option<char>> = shown.iter().map(|row| fixture.tree().name_of(row.node).and_then(|name| name.chars().next())).collect();
        let Some(first) = names[0] else { continue };
        let Some(target) = (1..shown.len()).find(|at| names[*at].is_some_and(|letter| !letter.to_lowercase().eq(first.to_lowercase()))) else {
            continue;
        };
        let Some(letter) = names[target] else { continue };
        row_click::click(&mut fixture, &shown[0]);
        fixture.type_text(&letter.to_string());
        fixture.settle();
        let Some((_, size)) = position_in_set(&fixture, shown[0].node) else { continue };
        let found = lead(&fixture, &container);
        if found != Some(target) {
            return Err(format!("{}: typing `{letter}` left the lead at {found:?} of {size}, expected row {target}", story.name));
        }
    }
    Ok(())
}

/// F2 on the lead, for a collection that renames: the editor takes focus, Enter ends it (one commit, or no event for a name nobody typed in), and Escape gives
/// one cancel and returns focus to a row.
pub(super) fn rename_commits_and_cancels_once(subject: &Subject) -> Result<(), String> {
    if !is_items(subject) {
        return Ok(());
    }
    let rename = |kinds: &[CollectionKind], edit: EditKind| kinds.iter().filter(|kind| **kind == CollectionKind::Rename(edit)).count();
    for story in &subject.stories {
        let Some((mut fixture, container, shown)) = with_rows(story, 1) else { continue };
        row_click::click(&mut fixture, &shown[0]);
        fixture.take_outputs();
        fixture.key(Key::F2, Modifiers::NONE);
        fixture.settle();
        let began = events(&mut fixture);
        if rename(&began, EditKind::Begin) == 0 {
            continue;
        }
        let name = story.name;
        if fixture.tree().role_of(fixture.tree().focused_key()) != Some(Role::TEXT_INPUT) {
            return Err(format!("{name}: F2 began a rename and the editor does not hold the focus"));
        }
        fixture.key(Key::Enter, Modifiers::NONE);
        fixture.settle();
        let ended = events(&mut fixture);
        // Enter on a name nobody typed in ends the rename with no event: the editor closes and focus returns to the rows.
        let unchanged_end = rename(&ended, EditKind::Commit) == 0
            && rename(&ended, EditKind::Cancel) == 0
            && fixture.tree().role_of(fixture.tree().focused_key()) != Some(Role::TEXT_INPUT)
            && lead(&fixture, &container).is_some();
        if !unchanged_end && (rename(&ended, EditKind::Commit) != 1 || rename(&ended, EditKind::Cancel) != 0) {
            return Err(format!("{name}: Enter gave {ended:?}, expected one rename commit or the rename ended with the name unchanged"));
        }
        fixture.key(Key::F2, Modifiers::NONE);
        fixture.settle();
        events(&mut fixture);
        fixture.key(Key::Escape, Modifiers::NONE);
        fixture.settle();
        let cancelled = events(&mut fixture);
        if rename(&cancelled, EditKind::Cancel) != 1 || rename(&cancelled, EditKind::Commit) != 0 {
            return Err(format!("{name}: Escape gave {cancelled:?}, expected one rename cancel"));
        }
        if lead(&fixture, &container).is_none() {
            return Err(format!("{name}: Escape left the focus off the rows"));
        }
    }
    Ok(())
}

/// Alt+Down on the lead, for a reorderable collection: one end event and no cancel.
pub(super) fn keyboard_reorder_ends_once(subject: &Subject) -> Result<(), String> {
    if !is_items(subject) {
        return Ok(());
    }
    for story in &subject.stories {
        let Some((mut fixture, _, shown)) = with_rows(story, 2) else { continue };
        row_click::click(&mut fixture, &shown[0]);
        fixture.take_outputs();
        fixture.key(Key::ArrowDown, Modifiers::ALT);
        fixture.settle();
        let kinds = events(&mut fixture);
        let reorders: Vec<EditKind> = kinds
            .iter()
            .filter_map(|kind| match kind {
                CollectionKind::Reorder(edit) => Some(*edit),
                _ => None,
            })
            .collect();
        if reorders.is_empty() {
            continue;
        }
        let ends = reorders.iter().filter(|edit| matches!(edit, EditKind::Commit | EditKind::Set)).count();
        if ends != 1 || reorders.contains(&EditKind::Cancel) {
            return Err(format!("{}: Alt+Down gave the reorder events {reorders:?}", story.name));
        }
    }
    Ok(())
}

/// An empty collection shows no rows and reports an empty visible range.
pub(super) fn empty_collection_shows_no_rows(subject: &Subject) -> Result<(), String> {
    if !is_items(subject) {
        return Ok(());
    }
    for story in subject.stories.iter().filter(|story| story.covers.contains(&crate::testing::story::Required::Empty)) {
        let mut fixture = Fixture::story(story);
        fixture.settle();
        let Some(container) = root(&fixture) else { continue };
        let shown = rows(&fixture, container.node);
        if !shown.is_empty() {
            return Err(format!("{}: an empty collection shows {} rows", story.name, shown.len()));
        }
        if fixture.shown().height().partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
            return Err(format!("{}: an empty collection allocates no height", story.name));
        }
        let visible = fixture.take_outputs().into_iter().last().and_then(|output| output.visible);
        if visible.is_some_and(|range| !range.is_empty()) {
            return Err(format!("{}: an empty collection reports visible rows", story.name));
        }
    }
    Ok(())
}

/// A collection whose rows reach past its room builds fewer rows than it has: out-of-view rows run nowhere
/// (`runtime::lazy`).
pub(super) fn long_collection_builds_rows_within_the_room_only(subject: &Subject) -> Result<(), String> {
    if !is_items(subject) {
        return Ok(());
    }
    for story in &subject.stories {
        let Some((fixture, container, shown)) = with_rows(story, 1) else { continue };
        let Some((_, size)) = position_in_set(&fixture, shown[0].node) else { continue };
        let row = shown[0].rect.height();
        if row.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
            continue;
        }
        // Items that wrap fill a row of their own: the set takes as many rows of the room as it has rows of items.
        let across = shown.iter().filter(|other| (other.rect.min.y - shown[0].rect.min.y).abs() < row / 2.0).count().max(1);
        let room = container.rect.size();
        // A row that starts inside the room is built, so the last one is out of it only when it starts below.
        if ((size.div_ceil(across) - 1) as f32) * row <= room.y {
            continue;
        }
        if shown.len() >= size {
            return Err(format!("{}: all {size} rows are built though the room holds about {}", story.name, (room.y / row) as usize + 1));
        }
    }
    Ok(())
}

/// One wheel step moves the rows and the frame it happens in shows rows over the whole viewport.
pub(super) fn scroll_shows_rows_in_the_same_frame(subject: &Subject) -> Result<(), String> {
    if !is_items(subject) {
        return Ok(());
    }
    for story in &subject.stories {
        let Some((mut fixture, container, shown)) = with_rows(story, 3) else { continue };
        let Some((_, size)) = position_in_set(&fixture, shown[0].node) else { continue };
        if shown.len() >= size {
            continue;
        }
        fixture.frame(vec![crate::testing::pointer::moved(container.rect.center())]);
        fixture.frame(vec![crate::testing::pointer::wheel(-3.0 * shown[0].rect.height())]);
        let now = rows(&fixture, container.node);
        let (Some(top), Some(bottom)) = (now.first(), now.last()) else {
            return Err(format!("{}: no row is shown in the frame of the scroll", story.name));
        };
        let row = top.rect.height();
        let last_row = position_in_set(&fixture, bottom.node).is_some_and(|(at, count)| at + 1 == count);
        if top.rect.min.y > shown[0].rect.min.y + row || (!last_row && bottom.rect.max.y < container.rect.max.y - row) {
            return Err(format!("{}: the frame of the scroll leaves part of the viewport without rows", story.name));
        }
        // A section header between two items fills the space it takes; any other gap is a row that is missing.
        let headings = nodes_of(&fixture, &container, Role::HEADING);
        let filled = |above: f32, below: f32| headings.iter().any(|heading| heading.rect.min.y >= above && heading.rect.max.y <= below);
        if now.windows(2).any(|pair| pair[1].rect.min.y - pair[0].rect.max.y > row && !filled(pair[0].rect.max.y, pair[1].rect.min.y)) {
            return Err(format!("{}: the frame of the scroll shows rows with a gap between them", story.name));
        }
    }
    Ok(())
}

/// The cell sub-nodes and the column header sub-nodes of the collection below `container`.
fn nodes_of(fixture: &Fixture, container: &Target, role: Role) -> Vec<Target> {
    let inside = subtree(fixture.tree(), container.node);
    let mut found: Vec<Target> = fixture.by_role(role).into_iter().filter(|target| inside.contains(&target.node)).collect();
    found.sort_by(|a, b| (a.rect.min.y, a.rect.min.x).partial_cmp(&(b.rect.min.y, b.rect.min.x)).unwrap_or(std::cmp::Ordering::Equal));
    found
}

/// A click on a cell that changes the selection leaves that cell's sub-node reporting `SELECTED`, and a cell in
/// another row reporting none. A collection whose click selects nothing is not read.
fn selected_cell_reports_selected(story: &Story) -> Result<(), String> {
    let Some((mut fixture, container, _)) = with_rows(story, 2) else { return Ok(()) };
    let cells = nodes_of(&fixture, &container, Role::CELL);
    let Some(clicked) = cells.first().copied() else { return Ok(()) };
    fixture.take_outputs();
    fixture.click(&clicked);
    if !events(&mut fixture).contains(&CollectionKind::Selection) {
        return Ok(());
    }
    let now = nodes_of(&fixture, &container, Role::CELL);
    if !now.iter().any(|cell| cell.node == clicked.node && super::nodes::is_selected(&fixture, cell.node)) {
        return Err(format!("{}: a clicked cell does not report `SELECTED`", story.name));
    }
    let row = clicked.rect.height();
    if let Some(other) = now.iter().find(|cell| (cell.rect.center().y - clicked.rect.center().y).abs() > row)
        && super::nodes::is_selected(&fixture, other.node)
    {
        return Err(format!("{}: a cell outside the selection reports `SELECTED`", story.name));
    }
    Ok(())
}

/// Header cells carry a name. From the keyboard, Enter on a header cell that declares `Click` and a sort direction (a sortable column) gives a column event,
/// and the resize and move commands that are enabled for the focused column give one: a resize through the arrow
/// keys, a move at once. A column that is not movable or resizable leaves its command disabled and is not read.
fn header_cells_report_column_events(story: &Story) -> Result<(), String> {
    let Some((mut fixture, container, _)) = with_rows(story, 0) else { return Ok(()) };
    let headers = nodes_of(&fixture, &container, Role::COLUMN_HEADER);
    let Some(first) = headers.first().copied() else { return Ok(()) };
    let name = story.name;
    if headers.iter().any(|header| fixture.tree().name_of(header.node).is_none_or(str::is_empty)) {
        return Err(format!("{name}: a column header has no name"));
    }
    let columns = |fixture: &mut Fixture| events(fixture).contains(&CollectionKind::Column);
    fixture.access(AccessActionKind::Focus, &first);
    fixture.settle();
    fixture.take_outputs();
    if fixture.tree().declares_key(first.node, Declared::Click) && fixture.tree().declares_sort(first.node) {
        fixture.key(Key::Enter, Modifiers::NONE);
        fixture.settle();
        if !columns(&mut fixture) {
            return Err(format!("{name}: Enter on a sortable header gave no column event"));
        }
    }
    let enabled = |fixture: &Fixture, command: ColumnCommands| commands::resolve(fixture.ctx(), command).enabled;
    if enabled(&fixture, ColumnCommands::Resize) {
        commands::fire(fixture.ctx(), ColumnCommands::Resize, FireSource::Automation);
        fixture.settle();
        fixture.take_outputs();
        fixture.key(Key::ArrowRight, Modifiers::NONE);
        fixture.settle();
        if !columns(&mut fixture) {
            return Err(format!("{name}: the Resize Column command and an arrow key gave no column event"));
        }
        fixture.key(Key::Enter, Modifiers::NONE);
        fixture.settle();
    }
    for command in [ColumnCommands::MoveRight, ColumnCommands::MoveLeft] {
        if enabled(&fixture, command) {
            fixture.take_outputs();
            commands::fire(fixture.ctx(), command, FireSource::Automation);
            fixture.settle();
            if !columns(&mut fixture) {
                return Err(format!("{name}: {command:?} gave no column event"));
            }
        }
    }
    Ok(())
}

/// The sub-nodes of cells and columns: a selected cell's sub-node reports `SELECTED`, and header cells are announced
/// and report column events from the keyboard.
pub(super) fn cells_and_columns_are_reported(subject: &Subject) -> Result<(), String> {
    if !is_items(subject) {
        return Ok(());
    }
    for story in &subject.stories {
        selected_cell_reports_selected(story)?;
        header_cells_report_column_events(story)?;
    }
    Ok(())
}
