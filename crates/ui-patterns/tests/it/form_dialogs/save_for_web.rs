//! Save for Web on a `DialogHost`: slice selection, palette edits and the animation clock.

use craft_behavior::PixelSize;
use craft_theme::Srgba;
use craft_ui::dialog::{DialogHost, DialogResult};
use craft_ui::patterns::form_dialogs::{
    Animation, HostedSaveForWeb, PALETTE_COLOR_KEY, SaveForWebOptions, WebFormat, WebPreset, WebSlice, WebSlices, WebVariant,
};
use craft_ui::testing::{Fixture, trial};
use egui::load::SizedTexture;
use egui::{Key, Modifiers, Pos2, Rect, Vec2, pos2, vec2};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// The seconds one Fixture frame adds to the clock.
const FRAME: f64 = 1.0 / 60.0;

/// The seconds the clock runs between the frame Play is pressed in and `playing` returning: the click's idle frame
/// and `idle(2)`.
const AFTER_PLAY: f64 = 3.0 * FRAME;

const IMAGE: PixelSize = PixelSize::new(200, 100);

/// The Original pane's accessible name for `IMAGE` named `poster.psd`.
const ORIGINAL_PANE: &str = "Original: Original, poster.psd  ·  200 × 100 px";

fn texture() -> SizedTexture {
    SizedTexture::new(egui::TextureId::Managed(1), Vec2::new(200.0, 100.0))
}

/// Two side-by-side slices, 1 on the left half and 2 on the right.
fn slices() -> Vec<WebSlice> {
    [(1, 0.0), (2, 100.0)].into_iter().map(|(number, left)| WebSlice::new(number, Rect::from_min_size(pos2(left, 0.0), vec2(100.0, 100.0)))).collect()
}

/// A host running `web` every pass, in a room large enough for the dialog.
fn open(web: HostedSaveForWeb) -> (Rc<RefCell<DialogHost>>, Fixture) {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(1400.0, 1000.0));
    host.borrow_mut().open(web);
    fixture.idle(10);
    (host, fixture)
}

fn current(host: &Rc<RefCell<DialogHost>>) -> SaveForWebOptions {
    host.borrow_mut().get_mut::<HostedSaveForWeb>().map(|web| web.current().clone()).expect("the dialog is open")
}

/// Shows the pictures at 100% and clicks the point `at` of the Original pane's picture.
fn click_picture(fixture: &mut Fixture, at: Pos2) {
    let pane = fixture.by_label(ORIGINAL_PANE).expect("the Original pane");
    let origin = pane.rect.center() - vec2(IMAGE.width as f32 / 2.0, IMAGE.height as f32 / 2.0);
    fixture.click_at(origin + at.to_vec2()).idle(2);
}

/// Selects the Optimized pane with a click in its margin, beside the picture, where no slice lies. The Color Table
/// shows the selected pane's palette, and a slice click on the Original pane selects the Original pane.
fn select_optimized_pane(fixture: &mut Fixture) {
    let tree = fixture.tree();
    let pane = tree
        .keys()
        .into_iter()
        .find(|key| tree.name_of(*key).is_some_and(|name| name.starts_with("Optimized: ")))
        .and_then(|key| fixture.target(key))
        .expect("the Optimized pane");
    fixture.click_at(pos2(pane.rect.left() + 4.0, pane.rect.center().y)).idle(2);
}

/// The default options as a GIF of `colors` colours.
fn gif(colors: u16) -> SaveForWebOptions {
    let mut options = SaveForWebOptions::default();
    options.format = WebFormat::Gif;
    options.colors = colors;
    options
}

#[trial]
fn a_slice_warning_appears_once_below_the_preview_captions() {
    let warning = "5 stored slices past the first 10000 were ignored";
    let web = || HostedSaveForWeb::new("poster.psd", IMAGE, texture()).slices(slices());
    let (_, normal) = open(web());
    assert!(!normal.texts().iter().any(|run| run.text == warning));

    let (_, mut overflow) = open(web().slice_warning(Some(warning.into())));
    let four_up = overflow.by_label("4-Up").expect("the 4-Up view");
    overflow.click(&four_up).idle(4);
    let notices = overflow.texts().iter().filter(|run| run.text == warning).collect::<Vec<_>>();
    assert_eq!(notices.len(), 1, "the dialog shows one warning before Save is pressed");
    let captions = overflow.texts().iter().filter(|run| run.text == "Encoding").collect::<Vec<_>>();
    assert_eq!(captions.len(), 3, "the three encoded preview captions");
    assert!(captions.iter().all(|caption| notices[0].rect.top() >= caption.rect.bottom()), "the warning follows both rows");
}

#[trial]
fn slices_and_palette_edits_follow_the_settings() {
    let start = gif(8);
    let preset = WebPreset::new("GIF 16", gif(16));
    let web = HostedSaveForWeb::new("poster.psd", IMAGE, texture()).options(start).slices(slices()).presets(vec![preset]);
    let (host, mut fixture) = open(web);
    let actual = fixture.by_label("100%").expect("the 100% button");
    fixture.click(&actual).idle(4);

    // A click selects one slice; Shift adds and removes.
    click_picture(&mut fixture, pos2(50.0, 50.0));
    assert_eq!(current(&host).slices, WebSlices::Selected(vec![1]), "a click selects the slice under it");
    fixture.hold(Modifiers::SHIFT);
    click_picture(&mut fixture, pos2(150.0, 50.0));
    assert_eq!(current(&host).slices, WebSlices::Selected(vec![1, 2]), "Shift adds a slice");
    // A second click at one point inside the double-click time is a double click, which the pane does not report.
    fixture.advance(1.0);
    click_picture(&mut fixture, pos2(150.0, 50.0));
    assert_eq!(current(&host).slices, WebSlices::Selected(vec![1]), "Shift on a selected slice removes it");
    fixture.advance(1.0);
    click_picture(&mut fixture, pos2(150.0, 50.0));
    fixture.hold(Modifiers::NONE);

    // An edited swatch survives a change that leaves the palette alone and goes with a colour-count change.
    let wanted = host.borrow_mut().get_mut::<HostedSaveForWeb>().map(HostedSaveForWeb::wanted_variants).expect("open");
    let revision = wanted.first().map_or(1, |want| want.request.revision);
    let palette = vec![[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]];
    let variant = WebVariant::new(0, revision, texture(), 1000).colors(4).palette(palette);
    host.borrow_mut().get_mut::<HostedSaveForWeb>().expect("open").set_variant(variant);
    fixture.idle(4);
    select_optimized_pane(&mut fixture);
    assert_eq!(current(&host).slices, WebSlices::Selected(vec![1, 2]), "a click beside the picture leaves the slices");
    let cell = fixture.by_label("1 #00ff00").expect("the Color Table lists the palette");
    fixture.click(&cell).key(Key::Enter, Modifiers::NONE).idle(4);
    let request = host.borrow_mut().get_mut::<HostedSaveForWeb>().and_then(HostedSaveForWeb::pick_color).expect("opening a cell asks for its colour");
    assert_eq!(request.key, PALETTE_COLOR_KEY);
    host.borrow_mut().get_mut::<HostedSaveForWeb>().expect("open").answer_color(PALETTE_COLOR_KEY, Srgba::rgb(1.0, 0.0, 1.0));
    fixture.idle(4);
    assert_eq!(current(&host).palette_edits.get(&1), Some(&[255, 0, 255]), "the answer is the edit of the opened entry");

    let interlaced = fixture.by_label("Interlaced").expect("the Interlaced check box");
    fixture.click(&interlaced).idle(4);
    assert_eq!(current(&host).palette_edits.len(), 1, "a setting that leaves the palette alone keeps the edit");
    let colors = fixture.by_label("Colors").expect("the Colors field");
    fixture.click(&colors).key(Key::ArrowUp, Modifiers::NONE).idle(4);
    let after = current(&host);
    assert_eq!(after.colors, 9, "the colour count changed");
    assert!(after.palette_edits.is_empty(), "a colour-count change clears the edits");

    // A preset pick keeps the slices.
    let select = fixture.by_label("Preset").expect("the Preset list");
    fixture.click(&select).idle(4);
    let entry = fixture.by_label("GIF 16").expect("the preset entry");
    fixture.click(&entry).idle(4);
    let after = current(&host);
    assert_eq!(after.colors, 16, "the preset applied");
    assert_eq!(after.slices, WebSlices::Selected(vec![1, 2]), "a preset keeps the slices");

    // Save answers the selection in the order it was made.
    let save = fixture.by_label("Save…").expect("the Save button");
    fixture.click(&save).idle(4);
    let request = host.borrow_mut().get_mut::<HostedSaveForWeb>().and_then(HostedSaveForWeb::pick_path).expect("Save asks for a path");
    host.borrow_mut().get_mut::<HostedSaveForWeb>().expect("open").answer_path(request.key.clone(), PathBuf::from("/tmp/poster.gif"));
    fixture.idle(6);
    let result = host.borrow_mut().take::<HostedSaveForWeb>().and_then(DialogResult::ok).expect("the path finishes the dialog");
    assert_eq!(result.options.slices, WebSlices::Selected(vec![1, 2]), "Save answers the selected slices");
}

/// The frame the animation shows after the host's passes.
fn frame_of(host: &Rc<RefCell<DialogHost>>) -> u32 {
    host.borrow_mut().get_mut::<HostedSaveForWeb>().map(|web| web.frame()).expect("the dialog is open")
}

/// A playing animation of four frames at `fps`, after Play.
fn playing(fps: f32) -> (Rc<RefCell<DialogHost>>, Fixture) {
    let (host, mut fixture) = open(HostedSaveForWeb::new("poster.psd", IMAGE, texture()).animation(Animation::new(4, fps)));
    let play = fixture.by_label("Play").expect("the Play button");
    fixture.click(&play).idle(2);
    (host, fixture)
}

#[trial]
fn advance_frame_keeps_time_across_slow_frames_pauses_and_bad_rates() {
    // Every Fixture frame adds 1/60 s to the clock, so a step of `seconds` is `advance(seconds - FRAME)` and the
    // frame that follows it.
    let step = |fixture: &mut Fixture, seconds: f64| {
        fixture.advance(seconds - FRAME).idle(1);
    };
    let (host, mut fixture) = playing(10.0);
    // Four steps of 0.06 s at 10 fps, after the half frame Play's own frames left: 1.1, 1.7, 2.3 and 2.9 frames.
    // The unspent fraction carries over.
    let mut seen = Vec::new();
    for _ in 0..4 {
        step(&mut fixture, 0.06);
        seen.push(frame_of(&host));
    }
    let expected: Vec<u32> = (1..=4).map(|n| (((AFTER_PLAY + 0.06 * f64::from(n)) * 10.0).floor() as u32) % 4).collect();
    assert_eq!(seen, expected, "whole frames step and the fraction of a frame is kept");

    // Paused time does not advance the frames, and Play resumes without a jump.
    let pause = fixture.by_label("Pause").expect("the Pause button");
    fixture.click(&pause).idle(2);
    let held = frame_of(&host);
    fixture.advance(5.0).idle(2);
    assert_eq!(frame_of(&host), held, "a paused animation holds its frame");
    let play = fixture.by_label("Play").expect("the Play button");
    fixture.click(&play).idle(2);
    assert_eq!(frame_of(&host), held, "Play resumes where the animation stopped");

    // Past the last frame the animation wraps: 0.33 s and the half frame Play left are 3.8 frames, so three steps.
    assert!(held >= 1, "the frames to come run past the last one");
    step(&mut fixture, 0.33);
    assert_eq!(frame_of(&host), (held + 3) % 4, "the animation wraps to the first frame");

    // A rate that is not positive or finite runs at the default 30 fps: 0.07 s and the half frame of Play's own
    // frames are 3.6 frames, so three steps.
    for fps in [0.0, f32::NAN, -5.0] {
        let (host, mut fixture) = playing(fps);
        step(&mut fixture, 0.07);
        assert_eq!(frame_of(&host), 3, "a rate of {fps} runs at the default");
    }
}

/// The options 4-Up's panes 1 and 2 are asked to encode, from the dialog's request for them.
fn four_up_options(host: &Rc<RefCell<DialogHost>>) -> [SaveForWebOptions; 2] {
    let wanted = host.borrow_mut().get_mut::<HostedSaveForWeb>().map(HostedSaveForWeb::wanted_variants).expect("the dialog is open");
    let options = |index: usize| wanted.iter().find(|variant| variant.index == index).expect("the 4-Up pane is asked for").request.options.clone();
    [options(1), options(2)]
}

/// Lossy WebP's 4-Up panes show half and a quarter of the quality; lossless WebP has no quality, so they show PNG-8
/// and GIF instead.
#[trial]
fn four_up_webp_halves_the_quality_until_lossless() {
    let mut options = SaveForWebOptions::default();
    options.format = WebFormat::WebP;
    options.quality = 80;
    let (host, mut fixture) = open(HostedSaveForWeb::new("poster.psd", IMAGE, texture()).options(options));
    let four_up = fixture.by_label("4-Up").expect("the 4-Up view");
    fixture.click(&four_up).idle(4);
    let [half, quarter] = four_up_options(&host);
    assert_eq!((half.format, half.quality, quarter.quality), (WebFormat::WebP, 40, 20));

    let lossless = fixture.by_label("Lossless").expect("the Lossless check box");
    fixture.click(&lossless).idle(4);
    let [half, quarter] = four_up_options(&host);
    assert_eq!((half.format, quarter.format), (WebFormat::Png8, WebFormat::Gif));
}
