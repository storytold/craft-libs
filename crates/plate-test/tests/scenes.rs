#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_plate_test::reference::{self, Context};
use craft_plate_test::scene::{self, Scene};
use craft_plate_test::{Error, structure};

fn scenes() -> Vec<Scene> {
    scene::load_dir(&scene::scenes_dir()).unwrap()
}

/// On CI (`CI` set) the workflow provisions the font, so a scene that left text out fails.
/// Local runs without the font skip text and only note it.
fn text_must_be_written() -> bool {
    std::env::var_os("CI").is_some_and(|v| !v.is_empty() && v != "false")
}

fn missing_font(scene: &Scene) -> String {
    format!("{}: text skipped; {} must name a directory with {} on CI", scene.id, reference::FONT_DIR_ENV, reference::FONT_FILE)
}

#[test]
fn every_scene_parses_and_is_consistent() {
    let scenes = scenes();
    assert!(!scenes.is_empty());
    let problems: Vec<String> = scenes.iter().flat_map(|s| s.validate(&scenes)).collect();
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    let mut ids: Vec<&str> = scenes.iter().map(|s| s.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), scenes.len(), "scene ids must be unique");
}

/// Non-pending scenes are written and match their `structure` block (with text left out for
/// lack of fonts, a mismatch is only a note, except on CI). Pending scenes must
/// either write or stop at a feature the writer lacks, so their other fields stay valid.
#[test]
fn written_scenes_match_their_structure() {
    let scenes = scenes();
    let ctx = Context::from_env(&scenes);
    if ctx.font.is_none() {
        println!("note: {} not set; text objects are skipped", reference::FONT_DIR_ENV);
    }
    let mut failures = Vec::new();
    let mut written = 0;
    for scene in &scenes {
        let result = reference::write(scene, &ctx);
        if let Some(task) = &scene.pending {
            match result {
                Ok(_) | Err(Error::Unsupported(_)) => println!("{} pending ({task})", scene.id),
                Err(e) => failures.push(format!("{} (pending {task}): {e}", scene.id)),
            }
            continue;
        }
        let output = match result {
            Ok(output) => output,
            Err(e) => {
                failures.push(format!("{}: {e}", scene.id));
                continue;
            }
        };
        for note in &output.notes {
            println!("note: {note}");
        }
        let summary = structure::summarize(&output.pdf).unwrap();
        let problems = scene.structure.check(&summary);
        // A scene whose overprint or other states sit only on text can't match without fonts.
        let text_skipped = output.notes.iter().any(|note| note.contains(reference::FONT_DIR_ENV));
        if text_skipped && text_must_be_written() {
            failures.push(missing_font(scene));
        } else if !problems.is_empty() && text_skipped {
            println!("note: {}: structure not checked with text skipped: {}", scene.id, problems.join("; "));
        } else if !problems.is_empty() {
            let json = serde_json::to_string_pretty(&summary).unwrap();
            failures.push(format!("{}:\n  {}\nsummary: {json}", scene.id, problems.join("\n  ")));
        }
        written += 1;
    }
    println!("{written} scenes written, {} pending", scenes.len() - written);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The print preset (the FOGRA39 output intent) adds one `GTS_PDFX` intent and leaves the rest
/// of a scene's structure as the scene expects. Three scenes cover blending, overprint and
/// DeviceN; embedding the 700 KB profile in every scene would make this the slowest test.
#[test]
fn print_preset_adds_one_gts_pdfx_intent() {
    let scenes = scenes();
    let ctx = Context::from_env(&scenes);
    let mut failures = Vec::new();
    for scene in scenes.iter().filter(|s| ["S01", "S06", "S14"].contains(&s.id.as_str())) {
        let mut print = scene.clone();
        print.settings.output_intent = Some("fogra39".to_owned());
        let output = reference::write(&print, &ctx).unwrap();
        let summary = structure::summarize(&output.pdf).unwrap();
        let mut want = scene.structure.clone();
        want.output_intents = Some(vec!["GTS_PDFX".to_owned()]);
        let problems = want.check(&summary);
        let text_skipped = output.notes.iter().any(|note| note.contains(reference::FONT_DIR_ENV));
        if text_skipped && text_must_be_written() {
            failures.push(missing_font(scene));
        } else if !problems.is_empty() && text_skipped {
            assert!(summary.output_intents.iter().filter(|i| *i == "GTS_PDFX").count() == 1, "{}: {summary:?}", scene.id);
        } else if !problems.is_empty() {
            failures.push(format!("{}: {}", scene.id, problems.join("; ")));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The check catches the RGB blending regression: S06 written with the writer's default group
/// space has an RGB form group and no page group, which its `structure` block rejects.
#[test]
fn structure_check_rejects_rgb_blending() {
    let scenes = scenes();
    let mut s06 = scenes.iter().find(|s| s.id == "S06").unwrap().clone();
    s06.settings.group_cs = None;
    let pdf = reference::write(&s06, &Context { scenes: &scenes, font: None }).unwrap().pdf;
    let summary = structure::summarize(&pdf).unwrap();
    assert!(summary.groups.iter().any(|g| g.at == "form" && g.s == "Transparency" && g.cs.as_deref() == Some("DeviceRGB")), "{summary:?}");
    assert!(summary.color_spaces.contains("DeviceCMYK"), "{summary:?}");
    let problems = s06.structure.check(&summary);
    assert!(problems.iter().any(|p| p.starts_with("page_group")), "{problems:?}");
    assert!(problems.iter().any(|p| p.starts_with("group_cs")), "{problems:?}");
    assert!(structure::summarize(b"not a pdf").is_err());
}

/// N1 waits for overprint (T4); without it, the DeviceN fill and its color spaces already
/// match the scene.
#[test]
fn device_n_scene_writes_its_color_spaces() {
    let scenes = scenes();
    let n1 = scenes.iter().find(|s| s.id == "N1").unwrap();
    let output = reference::write_partial(n1, &Context { scenes: &scenes, font: None }).unwrap();
    let summary = structure::summarize(&output.pdf).unwrap();
    assert_eq!(Some(&summary.color_spaces), n1.structure.color_spaces.as_ref(), "{summary:?}");
    assert!(!summary.device_rgb, "{summary:?}");
}
