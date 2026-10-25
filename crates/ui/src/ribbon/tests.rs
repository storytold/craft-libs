use super::*;

fn frame(ctx: &egui::Context, frame: RibbonFrame, captions: bool) -> (Rect, Vec<Rect>) {
    let style = RibbonStyle::from_visuals(&egui::Visuals::light());
    let mut outer = Rect::NOTHING;
    let mut groups = Vec::new();
    let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 200.0))), ..Default::default() };
    let mut full = ctx.run_ui(input, |ui| {
        let r = Ribbon::new(&style).frame(frame).captions(captions).show(ui, |ui| {
            for caption in ["Clipboard", "Font", "Paragraph"] {
                let before = ui.cursor().min.x;
                RibbonGroup::new(caption, &style).launcher("More").show(ui, |ui| {
                    ui.allocate_exact_size(vec2(40.0, 20.0), Sense::hover());
                });
                groups.push(Rect::from_min_max(egui::pos2(before, 0.0), egui::pos2(ui.cursor().min.x, 0.0)));
            }
        });
        outer = r.response.rect;
    });
    full.textures_delta.clear();
    (outer, groups)
}

/// Two groups with the same caption get a launcher each, not one shared interaction.
#[test]
fn groups_with_the_same_caption_keep_their_launchers_apart() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let style = RibbonStyle::from_visuals(&egui::Visuals::light());
    let mut full = ctx.run_ui(egui::RawInput::default(), |ui| {
        Ribbon::new(&style).captions(true).show(ui, |ui| {
            for _ in 0..2 {
                RibbonGroup::new("Font", &style).launcher("More").show(ui, |ui| {
                    ui.allocate_exact_size(vec2(40.0, 20.0), Sense::hover());
                });
            }
        });
    });
    full.textures_delta.clear();
    let update = full.platform_output.accesskit_update.expect("accesskit is on");
    let launchers: Vec<_> = update.nodes.iter().filter(|(_, n)| n.label() == Some("More")).map(|(id, _)| *id).collect();
    assert_eq!(launchers.len(), 2, "{launchers:?}");
    assert_ne!(launchers[0], launchers[1]);
}

/// A ribbon partly inside a clipped panel paints its contents only where the panel allows.
#[test]
fn the_ribbon_keeps_the_parent_clip() {
    let ctx = egui::Context::default();
    let style = RibbonStyle::from_visuals(&egui::Visuals::light());
    let parent = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(120.0, 200.0));
    let mut clip = Rect::NOTHING;
    let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 200.0))), ..Default::default() };
    let mut full = ctx.run_ui(input, |ui| {
        ui.set_clip_rect(parent);
        Ribbon::new(&style).show(ui, |ui| clip = ui.clip_rect());
    });
    full.textures_delta.clear();
    assert!(parent.contains_rect(clip), "{clip:?} reaches past {parent:?}");
}

#[test]
fn captions_add_their_row_and_a_card_adds_its_margin() {
    let ctx = egui::Context::default();
    let style = RibbonStyle::from_visuals(&egui::Visuals::light());
    let (with, _) = frame(&ctx, RibbonFrame::Card { margin: 4.0 }, true);
    let (without, _) = frame(&ctx, RibbonFrame::Card { margin: 4.0 }, false);
    let (flush, _) = frame(&ctx, RibbonFrame::Flush, false);
    assert_eq!(with.height() - without.height(), CAPTION_H);
    assert_eq!(without.height(), style.content_height + PAD_Y * 2.0 + 4.0);
    assert_eq!(flush.height(), style.content_height + PAD_Y * 2.0 + 1.0);
}

#[test]
fn groups_follow_each_other_and_are_at_least_as_wide_as_their_caption() {
    let ctx = egui::Context::default();
    let (_, groups) = frame(&ctx, RibbonFrame::Flush, true);
    assert_eq!(groups.len(), 3);
    for pair in groups.windows(2) {
        assert!((pair[0].right() - pair[1].left()).abs() < 0.5, "groups are contiguous: {pair:?}");
    }
    // "Paragraph" plus the launcher is wider than the 40 px of content.
    assert!(groups[2].width() > 40.0 + GROUP_PAD * 2.0);
}

#[test]
fn a_caption_bound_group_leaves_the_launcher_its_own_room() {
    // Width comes from the caption here (the content is narrower): caption text plus the launcher
    // plus the padding on both sides, so the centred text ends before the launcher starts.
    let ctx = egui::Context::default();
    let (_, groups) = frame(&ctx, RibbonFrame::Flush, true);
    let style = RibbonStyle::from_visuals(&egui::Visuals::light());
    let text = ctx.fonts_mut(|f| f.layout_no_wrap("Paragraph".to_string(), FontId::proportional(style.caption_font), style.caption).size().x);
    let group = groups[2];
    assert!((group.width() - (text + LAUNCHER_W + GROUP_PAD * 2.0)).abs() < 0.5, "{group:?}");
    let text_right = group.left() + (group.width() - LAUNCHER_W) / 2.0 + text / 2.0;
    let launcher_left = group.right() - 9.0 - 7.0;
    assert!(text_right <= launcher_left, "caption ends at {text_right}, launcher starts at {launcher_left}");
}
