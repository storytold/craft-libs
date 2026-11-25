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
    assert_eq!(with.height() - without.height(), style.label_band);
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

fn g(natural: f32, compact: f32) -> fit::Group {
    fit::Group::measured(natural, None, compact)
}

/// A ribbon row at `width` with `measured` planed from an earlier frame; returns the card's rect
/// and each group's level and width.
fn planned_row(ctx: &egui::Context, width: f32, measured: Option<&[fit::Group]>) -> (Rect, Vec<(Level, f32)>) {
    let style = RibbonStyle::from_visuals(&egui::Visuals::light());
    let mut outer = Rect::NOTHING;
    let mut seen = Vec::new();
    let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 200.0))), ..Default::default() };
    let mut full = ctx.run_ui(input, |ui| {
        let mut ribbon = Ribbon::new(&style).frame(RibbonFrame::Flush).captions(true);
        if let Some(groups) = measured {
            ribbon = ribbon.groups(groups);
        }
        let r = ribbon.show(ui, |ui| {
            for caption in ["Clipboard", "Font", "Paragraph"] {
                let before = ui.cursor().min.x;
                let resp = RibbonGroup::new(caption, &style).launcher("More").show(ui, |ui| {
                    ui.allocate_exact_size(vec2(40.0, 20.0), Sense::hover());
                });
                seen.push((resp.level, ui.cursor().min.x - before));
            }
        });
        outer = r.response.rect;
    });
    full.textures_delta.clear();
    (outer, seen)
}

#[test]
fn a_row_that_fits_keeps_every_group_in_its_natural_rows() {
    let groups = [g(100.0, 40.0), g(120.0, 50.0)];
    let plan = fit::plan(&groups, 220.0, 0.0, fit::Collapse::FirstMisfit);
    assert_eq!(plan.levels, vec![Level::Natural, Level::Natural]);
    assert!((plan.used - 220.0).abs() < 0.01, "{plan:?}");
    assert!(!plan.overflow);
}

#[test]
fn the_first_group_that_does_not_fit_and_the_rest_fold_to_one_row() {
    let groups = [g(100.0, 40.0), g(100.0, 40.0), g(100.0, 40.0)];
    let fits = fit::plan(&groups, 300.0, 0.0, fit::Collapse::FirstMisfit);
    assert_eq!(fits.levels, vec![Level::Natural, Level::Natural, Level::Natural]);
    let one = fit::plan(&groups, 260.0, 0.0, fit::Collapse::FirstMisfit);
    assert_eq!(one.levels, vec![Level::Natural, Level::Natural, Level::Compact]);
    let none = fit::plan(&groups, 120.0, 0.0, fit::Collapse::FirstMisfit);
    assert_eq!(none.levels, vec![Level::Natural, Level::Compact, Level::Compact]);
    // The edge room comes off the budget: a row that only fits flush does not fit here.
    let flush = fit::plan(&groups, 320.0, 0.0, fit::Collapse::FirstMisfit);
    let edged = fit::plan(&groups, 320.0, 40.0, fit::Collapse::FirstMisfit);
    assert_eq!(flush.levels, vec![Level::Natural, Level::Natural, Level::Natural]);
    assert_eq!(edged.levels, vec![Level::Natural, Level::Natural, Level::Compact]);
}

#[test]
fn groups_fold_before_the_row_overflows() {
    let groups = [g(300.0, 40.0), g(300.0, 40.0), g(300.0, 40.0)];
    let plan = fit::plan(&groups, 400.0, 0.0, fit::Collapse::FirstMisfit);
    assert_eq!(plan.levels, vec![Level::Natural, Level::Compact, Level::Compact]);
    assert!(!plan.overflow, "the folded row fits: {plan:?}");
    assert!(plan.used <= 400.0, "{plan:?}");
    // Too narrow even with every group folded: say so instead of drawing past the card.
    let tight = fit::plan(&groups, 100.0, 0.0, fit::Collapse::FirstMisfit);
    assert!(tight.overflow, "{tight:?}");
    assert!(tight.used > 100.0);
}

#[test]
fn folding_from_the_end_keeps_the_groups_before_it() {
    let groups = [g(100.0, 40.0), g(100.0, 40.0), g(100.0, 40.0)];
    let one = fit::plan(&groups, 260.0, 0.0, fit::Collapse::FromEnd);
    assert_eq!(one.levels, vec![Level::Natural, Level::Natural, Level::Compact]);
    let two = fit::plan(&groups, 180.0, 0.0, fit::Collapse::FromEnd);
    assert_eq!(two.levels, vec![Level::Natural, Level::Compact, Level::Compact]);
    // A group whose folded form is not narrower is left alone: folding it could never shorten the
    // row, so the groups to its left fold instead.
    let odd = [g(100.0, 40.0), g(40.0, 80.0)];
    let plan = fit::plan(&odd, 120.0, 0.0, fit::Collapse::FromEnd);
    assert_eq!(plan.levels, vec![Level::Compact, Level::Natural]);
}

#[test]
fn a_measured_stacked_form_is_kept_where_the_caller_prefers_it() {
    let groups = [fit::Group::measured(300.0, Some(200.0), 60.0), fit::Group::measured(300.0, Some(200.0), 60.0)];
    let preferred = [Level::Stacked, Level::Stacked];
    let roomy = fit::plan_preferred(&groups, 420.0, 0.0, &preferred);
    assert_eq!(roomy.levels, vec![Level::Stacked, Level::Stacked]);
    assert!(!roomy.overflow);
    let tight = fit::plan_preferred(&groups, 380.0, 0.0, &preferred);
    assert_eq!(tight.levels, vec![Level::Stacked, Level::Compact]);
    // A preferred level the group was never measured at keeps its widest measured form.
    let unmeasured_stack = [fit::Group::measured(300.0, None, 60.0)];
    let plan = fit::plan_preferred(&unmeasured_stack, 400.0, 0.0, &[Level::Stacked]);
    assert_eq!(plan.levels, vec![Level::Natural]);
}

#[test]
fn an_unmeasured_group_takes_no_room_and_hostile_widths_do_not_panic() {
    let groups = [fit::Group::unmeasured(), g(100.0, 40.0)];
    let plan = fit::plan(&groups, 120.0, 0.0, fit::Collapse::FirstMisfit);
    assert_eq!(plan.levels, vec![Level::Natural, Level::Natural]);
    assert!((plan.used - 100.0).abs() < 0.01, "{plan:?}");
    let hostile = [fit::Group::natural(f32::NAN), fit::Group::measured(f32::INFINITY, Some(-1.0), f32::NAN)];
    for plan in [
        fit::plan(&hostile, f32::NAN, f32::INFINITY, fit::Collapse::FromEnd),
        fit::plan(&hostile, -500.0, 0.0, fit::Collapse::FirstMisfit),
        fit::plan_preferred(&hostile, f32::NAN, f32::NAN, &[Level::Compact; 9]),
    ] {
        assert_eq!(plan.levels.len(), 2);
        assert!(plan.used.is_finite(), "{plan:?}");
    }
    // A tab past the plan's bound is planned to it and reported as overflowing.
    let many = vec![fit::Group::unmeasured(); fit::MAX_GROUPS + 1];
    let plan = fit::plan(&many, 1000.0, 0.0, fit::Collapse::FirstMisfit);
    assert_eq!(plan.levels.len(), fit::MAX_GROUPS);
    assert!(plan.overflow);
}

#[test]
fn a_1024_by_768_card_plans_its_groups_without_clipping() {
    // Five groups in the widths an app measures on its first frame.
    let groups = [g(150.0, 70.0), g(300.0, 100.0), g(260.0, 95.0), g(200.0, 90.0), g(120.0, 65.0)];
    let avail = 1024.0 - PAD_X * 2.0;
    let plan = fit::plan(&groups, avail, 0.0, fit::Collapse::FirstMisfit);
    assert!(!plan.overflow, "{plan:?}");
    assert!(plan.used + PAD_X * 2.0 <= 1024.0, "{plan:?}");
    assert_eq!(plan.level(0), Level::Natural);
    assert_eq!(plan.level(plan.levels.len().saturating_sub(1)), Level::Compact);
    // The three bands stay far inside a 768 pt window.
    let style = RibbonStyle::from_visuals(&egui::Visuals::light());
    let bands = Ribbon::new(&style).bands();
    assert!(bands.card() < 768.0);
    assert!(bands.total() >= bands.card());
}

#[test]
fn the_three_bands_stack_and_the_label_band_sits_under_the_commands() {
    let bands = Bands::office(24.0, 66.0);
    assert_eq!(bands.labels, fit::LABEL_BAND);
    assert!((bands.card() - (66.0 + fit::LABEL_BAND)).abs() < 0.01);
    assert!((bands.total() - (24.0 + bands.card())).abs() < 0.01);
    let card = Rect::from_min_size(egui::Pos2::ZERO, vec2(1000.0, bands.card()));
    let commands = bands.command_band(card);
    let labels = bands.label_band(card);
    assert!((commands.bottom() - labels.top()).abs() < 0.01, "{commands:?} {labels:?}");
    assert!((labels.height() - fit::LABEL_BAND).abs() < 0.01);
    assert!(card.contains_rect(commands) && card.contains_rect(labels));
    // Captions off means no label band in the card at all.
    assert_eq!(Ribbon::new(&RibbonStyle::from_visuals(&egui::Visuals::light())).captions(false).bands().labels, 0.0);
}

#[test]
fn a_caption_never_reaches_into_its_launcher() {
    let band = Rect::from_min_size(egui::Pos2::ZERO, vec2(120.0, fit::LABEL_BAND));
    let (text, launcher) = fit::caption_slots(band, LAUNCHER_W);
    assert!(launcher.is_some(), "a launcher was asked for");
    if let Some(launcher) = launcher {
        assert!(text.right() <= launcher.left(), "{text:?} {launcher:?}");
        assert!(band.contains_rect(text) && band.left() <= launcher.left() && launcher.right() <= band.right());
        assert!(launcher.height() <= fit::LAUNCHER_BOX);
    }
    assert!((text.height() - band.height()).abs() < 0.01);
    // No launcher, no reserved room; a hostile room reserves nothing either.
    assert_eq!(fit::caption_slots(band, 0.0), (band, None));
    assert_eq!(fit::caption_slots(band, f32::NEG_INFINITY), (band, None));
    assert!(fit::caption_slots(band, f32::NAN).1.is_none());
}

#[test]
fn a_group_is_at_least_as_wide_as_its_caption_and_its_launcher() {
    // The content is wider than the caption: the caption does not shrink it.
    assert!((fit::group_width(200.0, 60.0, LAUNCHER_W, GROUP_PAD) - (200.0 + GROUP_PAD * 2.0)).abs() < 0.01);
    // The caption and launcher are wider: the group grows to hold them with the same padding.
    let narrow = fit::group_width(40.0, 60.0, LAUNCHER_W, GROUP_PAD);
    assert!((narrow - (60.0 + LAUNCHER_W + GROUP_PAD * 2.0)).abs() < 0.01, "{narrow}");
    // A folded button is never narrower than its floor and holds its name.
    assert!(fit::compact_width(80.0, 0.0, CHEVRON, COMPACT_PAD, COMPACT_MIN) >= 80.0 + CHEVRON + COMPACT_PAD);
    assert_eq!(fit::compact_width(0.0, 0.0, 0.0, 0.0, COMPACT_MIN), COMPACT_MIN);
    assert!(fit::group_width(f32::NAN, -5.0, f32::INFINITY, GROUP_PAD).is_finite());
}

#[test]
fn a_narrow_card_folds_the_groups_that_do_not_fit() {
    let ctx = egui::Context::default();
    let groups = [g(200.0, 60.0), g(200.0, 60.0), g(200.0, 60.0)];
    let (outer, seen) = planned_row(&ctx, 400.0, Some(&groups));
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[0].0, Level::Natural, "{seen:?}");
    assert_eq!(seen[1].0, Level::Compact, "{seen:?}");
    assert_eq!(seen[2].0, Level::Compact, "{seen:?}");
    // A folded group is narrower than the rows it replaces and the row stays inside the card.
    assert!(seen[2].1 < 200.0, "{seen:?}");
    let drawn: f32 = seen.iter().map(|(_, width)| *width).sum();
    assert!(drawn <= outer.width(), "{drawn} in {}", outer.width());
}

#[test]
fn a_ribbon_without_measurements_draws_every_group_in_its_rows() {
    let ctx = egui::Context::default();
    let (_, seen) = planned_row(&ctx, 120.0, None);
    assert_eq!(seen.len(), 3);
    assert!(seen.iter().all(|(level, _)| *level == Level::Natural), "{seen:?}");
}
