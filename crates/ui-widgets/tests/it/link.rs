//! A link opens its URL in the same tab on a plain click and in a new tab on a click with a modifier held, as a
//! middle click does.

use craft_ui::buttons::Link;
use craft_ui::contract::{Role, Show as _};
use craft_ui::testing::focus::{egui_focused, key_release};
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, input, pointer, trial};
use egui::{Key, Modifiers, Rect, pos2, vec2};
use std::cell::Cell;

#[trial]
fn a_modified_click_opens_the_link_in_a_new_tab() {
    let mut frames = Frames::new();
    let shown = Cell::new(Rect::NOTHING);
    let mut show = |ui: &mut egui::Ui| {
        let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 40.0));
        shown.set(in_region(ui, region, |ui| Link::new("Shortcuts").url("https://example.com").show(ui).rect));
    };
    frames.frame(Vec::new(), &mut show);
    let at = shown.get().center();
    pointer::click(&mut frames, at, &mut show);
    frames.hold(Modifiers::COMMAND);
    pointer::click(&mut frames, at, &mut show);
    let tabs: Vec<bool> = frames.opened().iter().map(|(_, new_tab)| *new_tab).collect();
    assert_eq!(tabs, [false, true], "a plain click opens in the same tab, a Cmd/Ctrl click in a new one");
}

const BEFORE: &str = "Open the panel and press ";
const LABEL: &str = "Show where this tool is now";
const AFTER: &str = " to find it again later on.";

/// A paragraph with the link in it, `width` wide: 100 is narrow enough that the sentence and the link wrap.
fn sentence(frames: &mut Frames, width: f32) -> (Rect, impl FnMut(&mut egui::Ui) + use<>) {
    let shown = std::rc::Rc::new(Cell::new(Rect::NOTHING));
    let seen = shown.clone();
    let mut show = move |ui: &mut egui::Ui| {
        let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 300.0));
        seen.set(in_region(ui, region, |ui| Link::new(LABEL).in_sentence(BEFORE, AFTER).url("https://example.com").show(ui).rect));
    };
    frames.idle(2, &mut show);
    (shown.get(), show)
}

/// Drives the link in a sentence by the pointer over the whole paragraph and by the keyboard: only its own words take a
/// press, on every row they wrap onto, and it is one Tab stop that Enter and Space activate.
#[trial]
fn a_link_in_a_sentence_takes_the_press_only_on_its_words() {
    let mut frames = Frames::new();
    let (paragraph, mut show) = sentence(&mut frames, 100.0);
    pointer::click(&mut frames, paragraph.min + vec2(2.0, 2.0), &mut show);
    assert!(frames.opened().is_empty(), "a press on the words before the link does nothing");
    assert_eq!(egui_focused(&frames.ctx), None, "and moves no focus");

    let mut hits = Vec::new();
    let mut y = paragraph.min.y + 2.0;
    while y < paragraph.max.y {
        let mut x = paragraph.min.x + 2.0;
        while x < paragraph.max.x {
            let before = frames.opened().len();
            pointer::click(&mut frames, pos2(x, y), &mut show);
            if frames.opened().len() > before {
                hits.push(pos2(x, y));
            }
            x += 6.0;
        }
        y += 6.0;
    }
    let (top, bottom) = hits.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(top, bottom), at| (top.min(at.y), bottom.max(at.y)));
    assert!(!hits.is_empty(), "the link's characters take the press");
    assert!(bottom - top > 14.0, "the link takes the press on each row it wraps onto: {top}..{bottom}");
    let area = ((paragraph.height() / 6.0) * (paragraph.width() / 6.0)) as usize;
    assert!(hits.len() < area / 2, "the sentence's other words take none: {} of {area} points", hits.len());

    let mut frames = Frames::new();
    let (_, mut show) = sentence(&mut frames, 100.0);
    for key in [Key::Tab, Key::Enter, Key::Space] {
        frames.frame(vec![input::key_press(key, Modifiers::NONE)], &mut show);
        frames.frame(vec![key_release(key, Modifiers::NONE)], &mut show);
    }
    assert!(egui_focused(&frames.ctx).is_some(), "Tab stops on the link");
    assert_eq!(frames.opened().len(), 2, "Enter and Space each activate it once");
}

/// A link in a sentence is one `Link` node, named and bounded by its own words inside the paragraph, and Tab gives that
/// node the focus: a screen reader meets the link once, as a link.
#[trial]
fn a_link_in_a_sentence_is_one_link_node_that_takes_the_focus() {
    let mut frames = Frames::new();
    let (paragraph, mut show) = sentence(&mut frames, 1000.0);
    frames.frame(vec![input::key_press(Key::Tab, Modifiers::NONE)], &mut show);
    frames.frame(vec![key_release(Key::Tab, Modifiers::NONE)], &mut show);
    let tree = frames.tree();
    let links = tree.with_role(Role::LINK);
    let [link] = links[..] else { panic!("one Link node, not {}", links.len()) };
    assert_eq!(tree.name_of(link), Some(LABEL), "named by the link's words");
    let bounds = tree.bounds_of(link).expect("the link's bounds");
    assert!(paragraph.contains_rect(bounds) && bounds.width() < paragraph.width() / 2.0, "the link's words, not the paragraph: {bounds:?}");
    assert_eq!(tree.focused_key(), link, "Tab focuses the link's node");
}
