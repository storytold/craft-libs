//! Save for Web on a `DialogHost`: slice selection, palette edits and the animation clock.

use craft_behavior::PixelSize;
use craft_theme::Srgba;
use craft_ui::contract::Bind;
use craft_ui::dialog::{DialogHost, DialogResult, StackDialog};
use craft_ui::patterns::form_dialogs::{
    Animation, ColorTableEdit, ColorTableEvent, HostedSaveForWeb, PALETTE_COLOR_KEY, SAVE_PATH_KEY, SaveForWebDialog, SaveForWebOptions, WebFormat,
    WebPictures, WebPreset, WebSlice, WebSlices, WebVariant,
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
    fixture.settle();
    (host, fixture)
}

fn current(host: &Rc<RefCell<DialogHost>>) -> SaveForWebOptions {
    host.borrow_mut().get_mut::<HostedSaveForWeb>().map(|web| web.current().clone()).expect("the dialog is open")
}

#[trial]
fn image_size_can_unlink_and_preserves_exact_pixels() {
    let document = PixelSize::new(50_001, 100);
    let mut preset_options = SaveForWebOptions::default();
    preset_options.format = WebFormat::Png24;
    let preset = WebPreset::new("PNG exact", preset_options);
    let (host, mut fixture) = open(HostedSaveForWeb::new("wide.psd", document, texture()).presets(vec![preset.clone()]));
    assert!(current(&host).constrain_proportions);

    let field = |fixture: &mut Fixture, label, value| {
        let field = fixture.by_label(label).expect("the size field");
        fixture.click(&field).idle(2);
        fixture.key(Key::A, Modifiers::COMMAND).type_text(value).key(Key::Tab, Modifiers::NONE).idle(4);
    };
    field(&mut fixture, "Width", "22501");
    assert_eq!(current(&host).percent, 45.0);

    let link = fixture.by_label("Constrain proportions").expect("the proportions check box");
    fixture.click(&link).idle(2);
    field(&mut fixture, "Height", "37");
    assert_eq!(current(&host).size, Some(PixelSize::new(22_501, 37)));

    let select = fixture.by_label("Preset").expect("the Preset list");
    fixture.click(&select).idle(2);
    let entry = fixture.by_label("PNG exact").expect("the preset entry");
    fixture.click(&entry).idle(4);
    let current = current(&host);
    assert!(preset.is_set_in(&current), "the preset remains selected after editing the size");
    assert!(!current.constrain_proportions);
    assert_eq!(current.size, Some(PixelSize::new(22_501, 37)));

    let mut host = host.borrow_mut();
    let web = host.get_mut::<HostedSaveForWeb>().expect("the dialog is open");
    let wanted = web.wanted_variants();
    assert_eq!(wanted[0].request.options.size, Some(PixelSize::new(22_501, 37)));
    web.answer_path(SAVE_PATH_KEY, PathBuf::from("wide.png"));
    let result = web.confirm(fixture.ctx()).expect("Save returns the exact unlinked size");
    assert_eq!((result.width, result.height), (22_501, 37));
}

#[trial]
fn initial_exact_size_respects_the_document_side_limits() {
    let mut options = SaveForWebOptions::default();
    options.size = Some(PixelSize::new(300_001, 0));
    let (host, fixture) = open(HostedSaveForWeb::new("poster.psd", IMAGE, texture()).options(options));
    let normalized = Some(PixelSize::new(300_000, 1));

    let mut host = host.borrow_mut();
    let web = host.get_mut::<HostedSaveForWeb>().expect("the dialog is open");
    let wanted = web.wanted_variants();
    assert_eq!(wanted[0].request.options.size, normalized);
    assert_eq!(web.current().size, normalized);
    let fields = web.json_fields().expect("the dialog fields");
    assert_eq!(fields["width"], 300_000);
    assert_eq!(fields["height"], 1);

    web.answer_path(SAVE_PATH_KEY, PathBuf::from("poster.jpg"));
    let result = web.confirm(fixture.ctx()).expect("Save returns the preview size");
    assert_eq!((result.width, result.height), (300_000, 1));
    assert_eq!(result.options.size, normalized);

    let input = serde_json::json!({ "width": 0, "height": 300_001 });
    web.set_json_field(fixture.ctx(), "size", input).expect("the automation accepts supplied dimensions");
    let result = web.confirm(fixture.ctx()).expect("Save uses the new dimensions before the next frame");
    assert_eq!((result.width, result.height), (1, 300_000));
    assert_eq!(result.options.size, Some(PixelSize::new(1, 300_000)));
}

#[trial]
fn bound_exact_size_matches_preview_size() {
    let mut options = SaveForWebOptions::default();
    options.size = Some(PixelSize::new(300_001, 0));
    let options = Rc::new(RefCell::new(options));
    let bound = Rc::clone(&options);
    let size = Rc::new(RefCell::new(None));
    let shown = Rc::clone(&size);
    let mut fixture = Fixture::new(move |ui| {
        let mut options = bound.borrow_mut();
        let output = SaveForWebDialog::new("poster.psd", IMAGE, WebPictures::new().original(texture()), Bind::bound(&mut *options)).show(ui);
        if let Some(variant) = output.wanted_variants.first() {
            *shown.borrow_mut() = Some((output.size, variant.request.options.size));
        }
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(10);

    let expected = PixelSize::new(300_000, 1);
    assert_eq!(*size.borrow(), Some((expected, Some(expected))));
    assert_eq!(options.borrow().size, Some(expected));
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

/// The colours of the palette the shown encode reports.
const PALETTE: [[u8; 3]; 4] = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]];

/// Answers the encode the dialog asked for with `PALETTE` and selects the Optimized pane, which shows the Color Table.
fn show_palette(host: &Rc<RefCell<DialogHost>>, fixture: &mut Fixture) {
    let wanted = host.borrow_mut().get_mut::<HostedSaveForWeb>().map(HostedSaveForWeb::wanted_variants).expect("open");
    let revision = wanted.first().map_or(1, |want| want.request.revision);
    let variant = WebVariant::new(0, revision, texture(), 1000).colors(4).palette(PALETTE.to_vec());
    host.borrow_mut().get_mut::<HostedSaveForWeb>().expect("open").set_variant(variant);
    fixture.settle();
    select_optimized_pane(fixture);
}

/// The colour table events the dialog filed since the last call.
fn table_events(host: &Rc<RefCell<DialogHost>>) -> Vec<(ColorTableEdit, Vec<[u8; 3]>)> {
    let events = host.borrow_mut().get_mut::<HostedSaveForWeb>().map(HostedSaveForWeb::take_table_events).expect("the dialog is open");
    events.into_iter().map(|event: ColorTableEvent| (event.edit, event.colors)).collect()
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
    fixture.click(&actual).settle();

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

    // An answered swatch is a Shift event on its colour; a setting that leaves the palette alone files nothing, and a
    // colour-count change restores the generated table.
    show_palette(&host, &mut fixture);
    assert_eq!(current(&host).slices, WebSlices::Selected(vec![1, 2]), "a click beside the picture leaves the slices");
    let cell = fixture.by_label("1 #00ff00").expect("the Color Table lists the palette");
    fixture.click(&cell).key(Key::Enter, Modifiers::NONE).settle();
    let (id, request) = host.borrow_mut().take_color_request().expect("opening a cell asks for its colour");
    assert_eq!(request.key, PALETTE_COLOR_KEY);
    host.borrow_mut().answer_color(id, PALETTE_COLOR_KEY, Srgba::rgb(1.0, 0.0, 1.0));
    fixture.settle();
    let shift = (ColorTableEdit::Shift([255, 0, 255]), vec![[0, 255, 0]]);
    assert_eq!(table_events(&host), vec![shift], "the answer shifts the opened entry's colour");

    let interlaced = fixture.by_label("Interlaced").expect("the Interlaced check box");
    fixture.click(&interlaced).settle();
    assert!(table_events(&host).is_empty(), "a setting that leaves the palette alone files no table event");
    let colors = fixture.by_label("Colors").expect("the Colors field");
    fixture.click(&colors).key(Key::ArrowUp, Modifiers::NONE).settle();
    assert_eq!(current(&host).colors, 9, "the colour count changed");
    let restore = (ColorTableEdit::Restore, Vec::new());
    assert_eq!(table_events(&host), vec![restore], "a colour-count change restores the generated table");

    // A preset pick keeps the slices.
    let select = fixture.by_label("Preset").expect("the Preset list");
    fixture.click(&select).settle();
    let entry = fixture.by_label("GIF 16").expect("the preset entry");
    fixture.click(&entry).settle();
    let after = current(&host);
    assert_eq!(after.colors, 16, "the preset applied");
    assert_eq!(after.slices, WebSlices::Selected(vec![1, 2]), "a preset keeps the slices");

    // Save answers the selection in the order it was made.
    let save = fixture.by_label("Save…").expect("the Save button");
    fixture.click(&save).settle();
    let request = host.borrow_mut().get_mut::<HostedSaveForWeb>().and_then(HostedSaveForWeb::pick_path).expect("Save asks for a path");
    host.borrow_mut().get_mut::<HostedSaveForWeb>().expect("open").answer_path(request.key.clone(), PathBuf::from("/tmp/poster.gif"));
    fixture.settle();
    let result = host.borrow_mut().take::<HostedSaveForWeb>().and_then(DialogResult::ok).expect("the path finishes the dialog");
    assert_eq!(result.options.slices, WebSlices::Selected(vec![1, 2]), "Save answers the selected slices");
}

#[trial]
fn table_edits_carry_the_selected_colors() {
    let (host, mut fixture) = open(HostedSaveForWeb::new("poster.psd", IMAGE, texture()).options(gif(8)));
    show_palette(&host, &mut fixture);
    let chosen = serde_json::json!(["#ff0000", "#0000ff"]);
    host.borrow_mut()
        .get_mut::<HostedSaveForWeb>()
        .expect("open")
        .set_json_field(&egui::Context::default(), "colorSelection", chosen.clone())
        .expect("a selection");
    fixture.idle(4);
    let listed = host.borrow_mut().get_mut::<HostedSaveForWeb>().and_then(|web| web.json_fields()).expect("the fields");
    assert_eq!(listed["colorSelection"], chosen, "the selection lists back as the colours of the selected cells");

    let lock = fixture.by_label("Lock Colors").expect("the Lock button");
    fixture.click(&lock).idle(4);
    let selected = vec![[255, 0, 0], [0, 0, 255]];
    assert_eq!(table_events(&host), vec![(ColorTableEdit::Lock, selected.clone())]);
    let delete = fixture.by_label("Delete Color").expect("the Delete button");
    fixture.click(&delete).idle(4);
    assert_eq!(table_events(&host), vec![(ColorTableEdit::Delete, selected)]);
}

#[trial]
fn enter_in_the_optimize_rows_files_the_search_and_never_saves() {
    let (host, mut fixture) = open(HostedSaveForWeb::new("poster.psd", IMAGE, texture()).optimize());
    let opener = fixture.by_label("Optimize to File Size…").expect("the Optimize button");
    fixture.click(&opener).idle(4);
    let web = |host: &Rc<RefCell<DialogHost>>| host.borrow_mut().take::<HostedSaveForWeb>();
    let _ = web;
    assert!(host.borrow_mut().get_mut::<HostedSaveForWeb>().expect("open").take_optimize_open(), "opening the rows asks for the opening size");
    host.borrow_mut().get_mut::<HostedSaveForWeb>().expect("open").answer_optimize_start(Some(48 * 1024));
    fixture.idle(4);

    let field = fixture.by_label("Desired File Size (K)").expect("the Desired File Size field");
    fixture.click(&field).key(Key::Enter, Modifiers::NONE).idle(4);
    let request = host.borrow_mut().get_mut::<HostedSaveForWeb>().and_then(HostedSaveForWeb::take_optimize_request).expect("Enter files the search");
    assert_eq!(request.desired_k, 48.0, "the search asks for the size the rows opened at");
    assert_eq!(request.options, current(&host), "the search starts from the current settings");
    assert!(host.borrow_mut().get_mut::<HostedSaveForWeb>().and_then(HostedSaveForWeb::pick_path).is_none(), "Enter in the rows never saves");

    fixture.key(Key::Enter, Modifiers::CTRL).idle(4);
    let again = host.borrow_mut().get_mut::<HostedSaveForWeb>().and_then(HostedSaveForWeb::take_optimize_request);
    assert!(again.is_none(), "Enter with Ctrl held files nothing");

    let cancel = fixture.by_label("Cancel Optimize").expect("the rows' Cancel");
    fixture.click(&cancel).idle(4);
    fixture.key(Key::Enter, Modifiers::NONE).idle(4);
    assert!(host.borrow_mut().get_mut::<HostedSaveForWeb>().and_then(HostedSaveForWeb::pick_path).is_some(), "with the rows closed Enter saves");
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

/// A palette answer that arrives as the child dialog that picked it closes and the room changes, so the dialog measures
/// its body again before it runs it, still shifts the opened entry. Guards against a body that took the host's answers
/// in whichever run of the pass came first: the measuring run used the answer up and the real run reported nothing.
#[trial]
fn a_palette_answer_survives_a_measuring_run_of_the_body() {
    use craft_ui::dialog::{Alert, HostedAlert};
    let (host, mut fixture) = open(HostedSaveForWeb::new("poster.psd", IMAGE, texture()).options(gif(8)));
    show_palette(&host, &mut fixture);
    let cell = fixture.by_label("1 #00ff00").expect("the Color Table lists the palette");
    fixture.click(&cell).key(Key::Enter, Modifiers::NONE).idle(4);
    let (id, _) = host.borrow_mut().take_color_request().expect("opening a cell asks for its colour");
    let picker = host.borrow_mut().open_child(HostedAlert::new(Alert::new("Color Picker", "Pick a colour.")), id);
    fixture.idle(4);
    host.borrow_mut().close_by_id(picker);
    host.borrow_mut().answer_color(id, PALETTE_COLOR_KEY, Srgba::rgb(1.0, 0.0, 1.0));
    fixture.room(Vec2::new(1000.0, 700.0));
    fixture.idle(4);
    let shift = (ColorTableEdit::Shift([255, 0, 255]), vec![[0, 255, 0]]);
    assert_eq!(table_events(&host), vec![shift], "the answer shifts the opened entry's colour");
}

/// A palette colour the host answers in a frame whose first run measures the dialog still shifts the opened entry, once.
/// Guards against a hosted dialog that took its host's answers in the measuring run, whose form drops them.
#[trial]
fn a_palette_answer_in_a_measured_frame_shifts_the_entry_once() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = super::hosted::measured_fixture(&host);
    host.borrow_mut().open(HostedSaveForWeb::new("poster.psd", IMAGE, texture()).options(gif(8)));
    fixture.idle(10);
    show_palette(&host, &mut fixture);
    let cell = fixture.by_label("1 #00ff00").expect("the Color Table lists the palette");
    fixture.click(&cell).key(Key::Enter, Modifiers::NONE).idle(4);
    let (id, _) = host.borrow_mut().take_color_request().expect("opening a cell asks for its colour");
    host.borrow_mut().answer_color(id, PALETTE_COLOR_KEY, Srgba::rgb(1.0, 0.0, 1.0));
    fixture.idle(4);
    let shift = (ColorTableEdit::Shift([255, 0, 255]), vec![[0, 255, 0]]);
    assert_eq!(table_events(&host), vec![shift], "the answer shifts the opened entry's colour once");
}

/// The rects of the preview panes the view shows, found by their accessible names (`{pane}: {first}, {second}`).
fn pane_rects(fixture: &Fixture) -> Vec<Rect> {
    let tree = fixture.tree();
    let is_pane = |name: &str| (name.starts_with("Original") || name.starts_with("Optimized")) && name.contains(": ");
    tree.keys().into_iter().filter(|key| tree.name_of(*key).is_some_and(is_pane)).filter_map(|key| fixture.target(key)).map(|t| t.rect).collect()
}

/// The panes take the height of the settings column beside them, so they reach down to its Image Size rows, and 4-Up
/// splits that height between its two rows of panes.
#[trial]
fn the_preview_panes_take_the_height_beside_the_settings() {
    let (_, mut fixture) = open(HostedSaveForWeb::new("poster.psd", IMAGE, texture()));
    for (view, count) in [("2-Up", 2), ("4-Up", 4)] {
        let tab = fixture.by_label(view).expect("the view tab");
        fixture.click(&tab).idle(4);
        let percent = fixture.by_label("Percent").expect("the Image Size rows").rect;
        let panes = pane_rects(&fixture);
        assert_eq!(panes.len(), count, "{view} shows {count} panes");
        let bottom = panes.iter().map(|pane| pane.bottom()).fold(f32::MIN, f32::max);
        assert!(bottom > percent.top(), "{view}: the panes end at {bottom}, above the Percent row at {}", percent.top());
        let top = panes.iter().map(|pane| pane.top()).fold(f32::MAX, f32::min);
        let (upper, lower): (Vec<&Rect>, Vec<&Rect>) = panes.iter().partition(|pane| pane.top() <= top + 0.5);
        assert_eq!(upper.len(), 2, "{view}: two panes to a row");
        assert!(lower.iter().all(|pane| upper.iter().all(|up| pane.top() >= up.bottom())), "{view}: the second row is below the first");
    }
}
