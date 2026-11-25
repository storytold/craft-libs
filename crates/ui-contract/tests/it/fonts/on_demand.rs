//! On-demand faces: a candidate file is read once per install, at most `MAX_SCRIPT_READS` per script, and unreadable
//! ones are skipped; a collection's covering face is chosen by coverage and a malformed face is skipped; a running
//! lookup keeps its script waiting; registered faces are drawn at regular weight; the Han preference follows the UI
//! language and then the OS locale; and the deflated defaults are egui's default faces.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use craft_i18n::Han;
use craft_ui::fonts::{
    FontCandidates, FontRef, I18nScripts, MAX_SCRIPT_READS, SystemCandidates, SystemFonts, UiFonts, deflated_default_definitions, han_for_locale,
    set_weight,
};
use craft_ui::testing::{Frames, trial};
use egui::FontId;

const ARABIC: &[u8] = include_bytes!("../../fonts/NotoSansArabic-test.ttf");
const CJK: &[u8] = include_bytes!("../../fonts/NotoSansCJKsc-test.otf");

/// Candidate files per script, counting the character-map checks: one per readable file the loader read. `pending`
/// stands for a running background lookup; `cover_all` makes every readable file map every character.
struct Files {
    by_script: Vec<(&'static str, Vec<PathBuf>)>,
    checks: AtomicUsize,
    pending: AtomicBool,
    cover_all: AtomicBool,
}

impl Files {
    fn new(by_script: Vec<(&'static str, Vec<PathBuf>)>) -> Arc<Self> {
        Arc::new(Files { by_script, checks: AtomicUsize::new(0), pending: AtomicBool::new(false), cover_all: AtomicBool::new(false) })
    }

    fn checks(&self) -> usize {
        self.checks.load(Ordering::Relaxed)
    }
}

impl FontCandidates for Files {
    fn files(&self, script: &str, _han: Option<Han>) -> Vec<PathBuf> {
        self.by_script.iter().find(|(s, _)| *s == script).map(|(_, files)| files.clone()).unwrap_or_default()
    }

    fn covers(&self, data: &[u8], c: char) -> Option<u32> {
        self.checks.fetch_add(1, Ordering::Relaxed);
        if self.cover_all.load(Ordering::Relaxed) {
            return Some(0);
        }
        SystemCandidates.covers(data, c)
    }

    fn pending(&self) -> bool {
        self.pending.load(Ordering::Relaxed)
    }
}

/// A file under this process's temp folder.
fn write(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("craft-on-demand-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create the temp folder");
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write a test font");
    path
}

fn egui_face(name: &str) -> egui::FontData {
    (**deflated_default_definitions().font_data.get(name).expect("an egui default face")).clone()
}

/// Installs the stack with on-demand faces from `candidates`, as an app does at start and again on a language change.
fn install(frames: &Frames, candidates: &Arc<Files>) {
    UiFonts::new(&I18nScripts)
        .app_face("ui", egui_face("Ubuntu-Light"), &[FontRef::proportional()])
        .system_fonts(SystemFonts::Off)
        .on_demand(Arc::clone(candidates) as Arc<dyn FontCandidates>)
        .install(&frames.ctx);
}

/// Passes that draw `text`, enough for the loader to register a face and egui to add it.
fn show(frames: &mut Frames, text: &str) {
    for _ in 0..10 {
        frames.frame(vec![], |ui| {
            ui.label(text);
        });
    }
}

fn draws(frames: &Frames, c: char) -> bool {
    frames.ctx.fonts_mut(|f| f.has_glyph(&FontId::proportional(14.0), c))
}

/// (path, face index) of the faces the loader registered.
fn registered(frames: &Frames) -> Vec<(PathBuf, u32)> {
    craft_ui::fonts::on_demand_registered(&frames.ctx).into_iter().map(|(_, path, index)| (path, index)).collect()
}

#[trial]
fn a_candidate_is_read_once_per_install() {
    // The loader keeps no blob of its own: a file is read once and handed to egui. What bounds its memory is that no
    // file is read twice.
    let missing = std::env::temp_dir().join("craft-on-demand-missing.ttf");
    let empty = write("empty.ttf", b"");
    let cjk = write("cjk.otf", CJK);
    let arabic = write("arabic.ttf", ARABIC);
    let files = Files::new(vec![("Arab", vec![missing, empty, cjk, arabic.clone()])]);
    let mut frames = Frames::new();
    install(&frames, &files);

    show(&mut frames, "Layer 1 – café");
    assert_eq!(files.checks(), 0, "Latin text reads no candidate");

    show(&mut frames, "ع");
    assert!(draws(&frames, 'ع'));
    assert_eq!(registered(&frames), [(arabic, 0)], "the first candidate that maps the character");
    assert_eq!(files.checks(), 2, "a missing or empty file is never checked; the readable ones are read once each");

    show(&mut frames, "ع");
    assert_eq!(files.checks(), 2, "later passes read nothing again");

    // A language change installs again: the stack drops the face, and it is registered once more, not once per pass.
    install(&frames, &files);
    show(&mut frames, "ع");
    assert!(draws(&frames, 'ع'));
    assert_eq!(registered(&frames).len(), 1);
    assert_eq!(files.checks(), 4, "one more read of each readable candidate per install");
}

/// A collection of `faces`, with each face's table directory rewritten for offsets from the start of the collection.
fn collection(faces: &[&[u8]]) -> Vec<u8> {
    let tables = |font: &[u8]| u16::from_be_bytes([font[4], font[5]]) as usize;
    let mut dir_offsets = Vec::new();
    let mut cursor = 12 + 4 * faces.len();
    for font in faces {
        dir_offsets.push(cursor);
        cursor += 12 + 16 * tables(font);
    }
    let mut bases = Vec::new();
    let mut total = cursor.next_multiple_of(4);
    for font in faces {
        bases.push(total);
        total += font.len().next_multiple_of(4);
    }
    let mut out = vec![0u8; total];
    out[..4].copy_from_slice(b"ttcf");
    out[4..8].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    out[8..12].copy_from_slice(&u32::try_from(faces.len()).expect("few faces").to_be_bytes());
    for (i, offset) in dir_offsets.iter().enumerate() {
        out[12 + 4 * i..16 + 4 * i].copy_from_slice(&u32::try_from(*offset).expect("collection fits").to_be_bytes());
    }
    for (i, font) in faces.iter().enumerate() {
        let (dir, base) = (dir_offsets[i], bases[i]);
        out[dir..dir + 12].copy_from_slice(&font[..12]);
        for record in 0..tables(font) {
            let (src, dst) = (12 + 16 * record, dir + 12 + 16 * record);
            out[dst..dst + 8].copy_from_slice(&font[src..src + 8]);
            let table_offset = u32::from_be_bytes(font[src + 8..src + 12].try_into().expect("four bytes"));
            out[dst + 8..dst + 12].copy_from_slice(&(u32::try_from(base).expect("collection fits") + table_offset).to_be_bytes());
            out[dst + 12..dst + 16].copy_from_slice(&font[src + 12..src + 16]);
        }
        out[base..base + font.len()].copy_from_slice(font);
    }
    out
}

/// Points face `face`'s `hhea` table of the collection outside the file: its character map still works, but the face
/// cannot be installed.
fn break_hhea(collection: &mut [u8], face: usize) {
    let dir = u32::from_be_bytes(collection[12 + 4 * face..16 + 4 * face].try_into().expect("four bytes")) as usize;
    let tables = u16::from_be_bytes([collection[dir + 4], collection[dir + 5]]) as usize;
    let record = (0..tables).map(|i| dir + 12 + 16 * i).find(|r| &collection[*r..*r + 4] == b"hhea").expect("an hhea table");
    collection[record + 8..record + 12].copy_from_slice(&0xFFFF_FFF0u32.to_be_bytes());
}

#[trial]
fn a_collection_face_is_chosen_by_coverage_and_a_malformed_one_is_skipped() {
    // Face 0 of both collections lacks Arabic, face 1 has it.
    let good = collection(&[CJK, ARABIC]);
    let mut bad = good.clone();
    break_hhea(&mut bad, 1);
    let good = write("good.ttc", &good);
    let bad = write("bad.ttc", &bad);
    let files = Files::new(vec![("Arab", vec![bad, good.clone()])]);
    let mut frames = Frames::new();
    install(&frames, &files);

    show(&mut frames, "ع");
    assert!(draws(&frames, 'ع'));
    assert_eq!(registered(&frames), [(good, 1)], "the face whose character map has the character, not face 0; the malformed collection is skipped");
    assert_eq!(files.checks(), 2, "each collection is read once, however many faces it holds");
    show(&mut frames, "ع");
    assert_eq!(files.checks(), 2);
}

/// `n` copies of the CJK test face under different names: readable, and none maps Arabic.
fn non_covering(n: usize) -> Vec<PathBuf> {
    (0..n).map(|i| write(&format!("other-{i}.otf"), CJK)).collect()
}

#[trial]
fn at_most_the_read_bound_of_candidates_is_read_per_script_and_symbols_read_none() {
    let files = Files::new(vec![("Arab", non_covering(MAX_SCRIPT_READS + 4))]);
    let mut frames = Frames::new();
    install(&frames, &files);

    show(&mut frames, "☃ ∑ α");
    assert_eq!(files.checks(), 0, "symbols, math and Greek are drawn by the stack's own faces: no candidate is read");

    show(&mut frames, "ع");
    assert!(!draws(&frames, 'ع'));
    assert!(registered(&frames).is_empty());
    assert_eq!(files.checks(), MAX_SCRIPT_READS, "one bounded pass per script");
    show(&mut frames, "ع");
    assert_eq!(files.checks(), MAX_SCRIPT_READS, "a script that ran out of candidates is not tried again");
}

#[trial]
fn a_script_without_a_covering_file_blocks_no_other_script() {
    let arabic = write("arabic-after-han.ttf", ARABIC);
    let files = Files::new(vec![("Arab", vec![arabic.clone()])]);
    let mut frames = Frames::new();
    install(&frames, &files);

    // Han has no candidate at all; the Arabic character after it is still loaded.
    show(&mut frames, "日ع");
    assert!(draws(&frames, 'ع'));
    assert_eq!(registered(&frames), [(arabic, 0)]);
}

/// Removes its file when dropped, so a failed assertion leaves no sparse file behind.
struct Removing(PathBuf);

impl Drop for Removing {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[trial]
fn unreadable_candidates_are_skipped_and_use_up_no_read_bound() {
    let dir = std::env::temp_dir();
    let empty = write("skip-empty.ttf", b"");
    let garbage = write("skip-garbage.ttf", b"this is not a font");
    let oversized = write("skip-oversized.ttf", b"");
    let _removing = Removing(oversized.clone());
    std::fs::File::options().write(true).open(&oversized).and_then(|file| file.set_len((128 << 20) + 1)).expect("make a sparse oversized file");
    let arabic = write("skip-arabic.ttf", ARABIC);
    let mut candidates: Vec<PathBuf> = (0..MAX_SCRIPT_READS).map(|i| dir.join(format!("craft-on-demand-missing-{i}.ttf"))).collect();
    candidates.extend([dir, empty, oversized, garbage, arabic.clone()]);
    let files = Files::new(vec![("Arab", candidates)]);
    let mut frames = Frames::new();
    install(&frames, &files);

    show(&mut frames, "ع");
    assert!(draws(&frames, 'ع'), "the candidate after the missing, empty, oversized, directory and non-font ones is reached");
    assert_eq!(registered(&frames), [(arabic, 0)]);
    assert_eq!(files.checks(), 1, "only a file that parses is checked for the character");
}

#[trial]
fn a_running_lookup_keeps_the_script_waiting_instead_of_exhausting_it() {
    // The lookup that lists Han candidates is still running and what is listed so far covers nothing.
    let other = write("lookup-other.ttf", ARABIC);
    let files = Files::new(vec![("Hani", vec![other.clone()])]);
    files.pending.store(true, Ordering::Relaxed);
    let mut frames = Frames::new();
    install(&frames, &files);

    show(&mut frames, "日");
    assert!(registered(&frames).is_empty());

    // The lookup ends with a candidate that covers: the script was not given up on.
    files.pending.store(false, Ordering::Relaxed);
    files.cover_all.store(true, Ordering::Relaxed);
    show(&mut frames, "日");
    assert_eq!(registered(&frames), [(other, 0)]);
}

#[trial]
fn an_on_demand_face_is_drawn_at_regular_weight() {
    // The Arabic test face's default instance is Thin (100), like many variable system fonts.
    let arabic = write("weight-arabic.ttf", ARABIC);
    let files = Files::new(vec![("Arab", vec![arabic])]);
    let mut frames = Frames::new();
    install(&frames, &files);
    show(&mut frames, "ع");

    let (name, ..) = registered_names(&frames).into_iter().next().expect("the Arabic face is registered");
    let face = frames.ctx.fonts(|f| f.definitions().font_data.get(&name).cloned()).expect("egui holds the registered face");
    let wght = face.tweak.coords.as_ref().iter().find(|(tag, _)| *tag == egui::epaint::text::Tag::new(b"wght")).map(|(_, value)| *value);
    assert_eq!(wght, Some(400.0));

    // Bytes with no weight axis or no metrics are left alone.
    let mut plain = egui::FontData::from_static(b"");
    assert!(!set_weight(&mut plain, 400));
    assert!(plain.tweak.coords.as_ref().is_empty());
}

fn registered_names(frames: &Frames) -> Vec<(String, PathBuf, u32)> {
    craft_ui::fonts::on_demand_registered(&frames.ctx)
}

#[trial]
fn the_han_preference_follows_the_ui_language_then_the_os_locale() {
    let is =
        |ui: &str, os: Option<&str>, want: Option<Han>| assert_eq!(format!("{:?}", han_for_locale(ui, os)), format!("{want:?}"), "{ui} on {os:?}");
    is("ja", Some("en_US"), Some(Han::Japanese));
    is("zh-hant", Some("ja_JP"), Some(Han::Traditional));
    is("zh-hans", Some("ko_KR"), Some(Han::Simplified));
    is("ko", None, Some(Han::Korean));
    // A UI language that isn't written with Han defers to the OS locale.
    is("en", Some("ja_JP"), Some(Han::Japanese));
    is("de", Some("ko_KR"), Some(Han::Korean));
    is("en", Some("zh-Hant-TW"), Some(Han::Traditional));
    is("en", Some("zh-Hans-CN"), Some(Han::Simplified));
    is("en", Some("en_US"), None);
    is("en", None, None);
}

#[trial]
fn the_deflated_defaults_are_eguis_default_faces_and_draw_its_icons() {
    use epaint_default_fonts::special_emojis::{GIT, GITHUB, OS_ANDROID, OS_APPLE, OS_WINDOWS};
    use epaint_default_fonts::{EGUI_ICONS, EMOJI_ICON, HACK_REGULAR, NOTO_EMOJI_REGULAR, UBUNTU_LIGHT};
    let defaults = deflated_default_definitions();
    for (name, bytes, scale) in [("Hack", HACK_REGULAR, 1.0), ("Ubuntu-Light", UBUNTU_LIGHT, 1.0), ("egui-icons", EGUI_ICONS, 0.90)] {
        let data = defaults.font_data.get(name).unwrap_or_else(|| panic!("{name} is a default face"));
        assert!(data.bytes() == bytes, "{name} holds egui's bytes");
        assert_eq!(data.tweak.scale, scale, "{name}'s scale");
    }
    // The opt-in monochrome emoji faces.
    for (data, bytes, scale) in [(craft_ui::fonts::noto_emoji(), NOTO_EMOJI_REGULAR, 0.81), (craft_ui::fonts::emoji_icon_font(), EMOJI_ICON, 0.90)] {
        let data = data.expect("the emoji face inflates");
        assert!(data.bytes() == bytes);
        assert_eq!(data.tweak.scale, scale);
    }

    // egui's own icons draw in both families, which the strict missing-glyph policy of headless runs depends on. The
    // penguin (`OS_LINUX`) is a plain emoji that only an emoji face draws, so it is left out: a runner's system fonts
    // decide whether it draws.
    let mut frames = Frames::new();
    frames.ctx.set_fonts(defaults);
    for _ in 0..2 {
        frames.frame(vec![], |_| {});
    }
    for family in [FontId::proportional(14.0), FontId::monospace(14.0)] {
        for icon in [GIT, GITHUB, OS_ANDROID, OS_APPLE, OS_WINDOWS] {
            assert!(frames.ctx.fonts_mut(|f| f.has_glyph(&family, icon)), "{icon} in {:?}", family.family);
        }
    }
}
