//! `from_items` groups flat rows by menu path in one pass; the tree is the one the row-by-row walk made: submenus where their
//! first row is, separators tidied at every depth.

use craft_menu::{CommandRef, MenuBar, Node, Row, from_items, tidy};
use craft_ui::testing::trial;

/// The row-by-row walk `from_items` replaced: every submenu filters all rows of its level again.
fn reference(tops: &[&str], rows: &[Row]) -> MenuBar {
    fn level(rows: &[&Row], depth: usize) -> Vec<Node> {
        let mut out = Vec::new();
        let mut shown: Vec<&str> = Vec::new();
        for it in rows {
            if it.path.len() == depth {
                out.push(if it.label == "---" { Node::Separator } else { Node::Item(craft_menu::Item::new(it.command.clone(), it.label.clone())) });
            } else if let Some(name) = it.path.get(depth) {
                let name = name.as_str();
                if shown.contains(&name) {
                    continue;
                }
                shown.push(name);
                let child: Vec<&Row> = rows.iter().copied().filter(|c| c.path.get(depth).map(String::as_str) == Some(name)).collect();
                out.push(Node::submenu(name, level(&child, depth + 1)));
            }
        }
        old_tidy(out)
    }
    let menus = tops
        .iter()
        .map(|top| {
            let rows: Vec<&Row> = rows.iter().filter(|r| r.path.first().map(String::as_str) == Some(*top)).collect();
            craft_menu::Menu::new(*top, level(&rows, 1))
        })
        .collect();
    MenuBar::new(menus)
}

/// The tidy `from_items` used before it worked in place.
fn old_tidy(nodes: Vec<Node>) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::with_capacity(nodes.len());
    for mut n in nodes {
        if let Node::Submenu { children, .. } = &mut n {
            *children = old_tidy(std::mem::take(children));
            if children.is_empty() {
                continue;
            }
        }
        if matches!(n, Node::Separator) && matches!(out.last(), None | Some(Node::Separator)) {
            continue;
        }
        out.push(n);
    }
    while matches!(out.last(), Some(Node::Separator)) {
        out.pop();
    }
    out
}

fn row(path: &[&str], id: &str) -> Row {
    Row::new(path, CommandRef::new(id), id)
}

#[trial]
fn from_items_builds_the_tree_the_row_by_row_walk_built() {
    let rows = [
        Row::separator(&["File"]),
        row(&["File"], "file.new"),
        row(&["File", "Recent"], "file.a"),
        Row::separator(&["File"]),
        Row::separator(&["File"]),
        row(&["File", "Recent", "More"], "file.deep"),
        row(&["File", "Recent"], "file.b"),
        row(&["File"], "file.save"),
        Row::separator(&["File", "Recent"]),
        row(&["File", "Export"], "file.png"),
        Row::separator(&["File"]),
        row(&["Edit"], "edit.undo"),
        row(&["Stray"], "stray"),
        row(&["Edit", "Find", "In Files"], "edit.find"),
        Row::separator(&["Edit"]),
        Row::separator(&["Edit", "Empty"]),
    ];
    let tops = ["File", "Edit", "View"];
    assert_eq!(from_items(&tops, &rows), reference(&tops, &rows));
    let bar = from_items(&tops, &rows);
    assert_eq!(bar.menus.len(), 3, "a top with no rows stays an empty menu");
    assert!(bar.menus[2].children.is_empty());
    let Node::Submenu { label, .. } = &bar.menus[0].children[1] else { panic!("the submenu sits where its first row is") };
    assert_eq!(label, "Recent");
}

#[trial]
fn tidy_drops_edge_and_repeated_separators_and_empty_submenus_in_place() {
    let mut nodes = vec![
        Node::Separator,
        Node::Separator,
        Node::Item(craft_menu::Item::new("a", "A")),
        Node::submenu("Empty", vec![Node::Separator]),
        Node::Separator,
        Node::Separator,
        Node::submenu("Kept", vec![Node::Separator, Node::Item(craft_menu::Item::new("b", "B")), Node::Separator]),
        Node::Separator,
    ];
    tidy(&mut nodes);
    let labels: Vec<&str> = nodes
        .iter()
        .map(|n| match n {
            Node::Item(it) => it.label.as_str(),
            Node::Submenu { label, children, .. } => {
                assert_eq!(children.len(), 1, "the submenu's own separators are tidied");
                label.as_str()
            }
            _ => "-",
        })
        .collect();
    assert_eq!(labels, ["A", "-", "Kept"]);
}
