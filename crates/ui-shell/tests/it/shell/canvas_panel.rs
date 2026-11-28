//! `CanvasPanel`: a double click on the empty document area is the panel's `empty_double` (the host opens a file);
//! over a document whose canvas takes the press, it is never reported. The tab strip is as high as its tabs.

use craft_behavior::{HitLevel, HitPriority, PartKey, Sense};
use craft_theme::{AppSizes, SizeValue};
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PartsCx, Role, Show as _};
use craft_ui::runtime::env::{Env, RootEnv};
use craft_ui::shell::CanvasPanel;
use craft_ui::tabs::{DocumentTab, DocumentTabs};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::trial;
use craft_ui_test_kit::contract::PlainStyle;
use craft_ui_test_kit::windows::Windows;
use egui::{Event, Rect, Response, Vec2};
use std::sync::Arc;

const BACKGROUND: PartKey = PartKey::new("background");

/// A document's canvas as the panel sees it: one background part over its rect that takes presses, as a `Canvas`'s
/// background part does (ui-shell cannot reach ui-canvas).
#[derive(craft_ui::contract::Component)]
struct Document {
    #[common]
    common: Common,
}

impl Component for Document {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &craft_ui::style::StyleCx<'_>, _: DocumentStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.part(BACKGROUND, rect)
            .priority(HitPriority::new(HitLevel::Background, 0))
            .sense(Sense::CLICK | Sense::DRAG)
            .pointer_only("the document's tool takes the press");
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// Shows a panel with tabs, and a document filling its document area when `with_document`; after two
/// settling frames, moves to the area's centre and clicks there `clicks` times, one frame per press and per release.
/// Returns the steps (0 the move) in which the panel reported `empty_double`.
fn empty_doubles(with_document: bool, clicks: usize) -> Vec<usize> {
    let windows = Windows::new();
    let mut document = Rect::NOTHING;
    let mut reported = false;
    let frame = |events: Vec<Event>, document: &mut Rect, reported: &mut bool| {
        windows.frame(events, vec![], |ui| {
            let shown = CanvasPanel::new().tabs(|_, _| {}).show(ui, |ui, cx| {
                if with_document {
                    Document { common: Common::new() }.show_at(ui, cx.rect);
                }
            });
            *document = shown.document;
            *reported |= shown.empty_double;
        });
    };
    for _ in 0..2 {
        frame(vec![], &mut document, &mut reported);
    }
    assert!(document.is_positive(), "the panel has a document area");
    let at = document.center();
    let mut steps = vec![vec![moved(at)]];
    for _ in 0..clicks {
        steps.extend([vec![press(at)], vec![release(at)]]);
    }
    let mut frames = Vec::new();
    for (step, events) in steps.into_iter().enumerate() {
        reported = false;
        frame(events, &mut document, &mut reported);
        if reported {
            frames.push(step);
        }
    }
    frames
}

#[trial]
fn an_empty_canvas_panel_reports_a_double_click_on_its_room() {
    assert_eq!(empty_doubles(false, 1), Vec::<usize>::new(), "a single click on the empty room is no double click");
    // Steps: move, press, release, press (the second press of the double click), release.
    assert_eq!(empty_doubles(false, 2), vec![3], "the second press on the empty room reports once");
    assert_eq!(empty_doubles(true, 2), Vec::<usize>::new(), "a document that takes the press leaves the room's double click unreported");
}

/// Regression: the tab pane had no extent, so it kept the height it first took (the strip before its tabs measured) and
/// cut the tabs of a taller `size.bar.tab`; large tabs took the title bar's height. Each form of the strip shows its
/// tabs whole in a pane at least as high as its own token, the large form above the regular.
#[trial]
fn the_tab_strip_is_as_high_as_its_tabs() {
    let tabs = [DocumentTab::new(0_usize, "One"), DocumentTab::new(1, "Two")];
    let shown = |large: bool| {
        let windows = Windows::new();
        let mut root = RootEnv::default();
        let mut sizes = AppSizes::default();
        sizes.all.insert("size.bar.tab".to_owned(), SizeValue::Number(44.0));
        sizes.all.insert("size.bar.tab_large".to_owned(), SizeValue::Number(60.0));
        root.sizes = Arc::new(sizes);
        windows.root_env(root);
        let (mut pane, mut strip, mut token) = (None, Rect::NOTHING, 0.0);
        for _ in 0..3 {
            windows.frame(vec![], vec![], |ui| {
                let bar = Env::of(ui).tokens.size.bar;
                token = if large { bar.tab_large } else { bar.tab };
                pane = CanvasPanel::new()
                    .large_tabs(large)
                    .tabs(|ui, _| strip = DocumentTabs::new("Documents", &tabs).large(large).show(ui).response.rect)
                    .show(ui, |_, _| {})
                    .tabs;
            });
        }
        let (pane, form) = (pane.expect("the panel shows its tab strip"), if large { "large" } else { "regular" });
        assert!(pane.height() >= token, "the {form} strip's pane {pane:?} is as high as its token {token}");
        assert!(strip.is_positive() && pane.contains_rect(strip), "the {form} strip's tabs {strip:?} lie whole in its pane {pane:?}");
        pane.height()
    };
    assert!(shown(true) > shown(false), "large tabs take the taller strip");
}
