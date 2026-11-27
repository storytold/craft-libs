//! FCPXML: a hand-written document (from the public FCPXML element reference) with connected
//! clips, a transition, a retimed clip, a gap and a compound clip; plus export → import round trips.

use super::support;

use craft_timeline::interchange::{
    BinEntry, ClipId, ItemId, ItemKind, Keyframe, Label, Marker, MarkerId, MarkerKind, MediaRef, Param, ParamValue, Project, Sequence, TrackKind,
    TransitionAlign,
};
use craft_timeline::{ExportOptions, FcpxmlVersion, Format, Report, detect_text as detect};
use craft_timeline::{FrameRate, TICKS_PER_SECOND, Tick};
use craft_ui::testing::trial;
use proptest::prelude::*;
#[allow(clippy::wildcard_imports, reason = "tests use most of the module under test and its helpers")]
use support::*;

/// Frame `n` at 23.976 as an FCPXML time, offset by `base` frames.
fn ft(n: i64) -> String {
    format!("{}/24000s", n * 1001)
}

fn doc() -> String {
    let tc = 86_400; // 01:00:00:00 at 23.976 NDF
    let tm_t = format!("{}/24000s", 3610 * 24000 + 48 * 1001);
    let tm_v = format!("{}/24000s", 3610 * 24000 + 96 * 1001);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE fcpxml>
<fcpxml version="1.10">
  <resources>
    <format id="r1" frameDuration="1001/24000s" width="1920" height="1080"/>
    <asset id="r2" name="Pier" start="0s" duration="{d240}" hasVideo="1" hasAudio="1" format="r1" audioChannels="2" audioRate="48000">
      <media-rep kind="original-media" src="file:///Media/pier%20wide.mov"/>
    </asset>
    <asset id="r3" name="Boats" start="3600s" duration="120s" hasVideo="1" format="r1">
      <media-rep kind="original-media" src="boats.mov"/>
    </asset>
    <asset id="r4" name="Theme" start="0s" duration="60s" hasAudio="1" audioRate="44100">
      <media-rep kind="original-media" src="file:///Media/theme.wav"/>
    </asset>
    <effect id="r5" name="Cross Dissolve" uid="example.dissolve"/>
    <media id="r6" name="Compound">
      <sequence format="r1" tcStart="0s">
        <spine>
          <asset-clip ref="r3" name="Boats" offset="0s" start="3600s" duration="{d48}"/>
        </spine>
      </sequence>
    </media>
  </resources>
  <library>
    <event name="Day 1">
      <project name="Harbour">
        <sequence format="r1" tcStart="{tc0}" tcFormat="NDF" audioRate="48k">
          <spine>
            <asset-clip ref="r2" name="Pier" offset="{tc0}" start="{d24}" duration="{d72}">
              <adjust-transform position="10 -5" scale="1.5 1.5" rotation="90"/>
              <asset-clip ref="r4" lane="-1" name="Theme" offset="{d48}" start="0s" duration="{d96}">
                <adjust-volume amount="-6dB"/>
              </asset-clip>
              <marker start="{d48}" duration="{d1}" value="Seagull" note="nice"/>
            </asset-clip>
            <transition name="Cross Dissolve" offset="{t60}" duration="{d24}">
              <filter-video ref="r5" name="Cross Dissolve"/>
            </transition>
            <asset-clip ref="r3" name="Boats" offset="{t72}" start="3610s" duration="{d48}">
              <timeMap>
                <timept time="3610s" value="3610s" interp="linear"/>
                <timept time="{tm_t}" value="{tm_v}" interp="linear"/>
              </timeMap>
              <asset-clip ref="r2" lane="1" name="Pier Insert" offset="3610s" start="0s" duration="{d24}" srcEnable="video">
                <adjust-blend amount="0.5">
                  <param name="amount">
                    <keyframeAnimation>
                      <keyframe time="0s" value="0"/>
                      <keyframe time="{d12}" value="1"/>
                    </keyframeAnimation>
                  </param>
                </adjust-blend>
              </asset-clip>
            </asset-clip>
            <gap name="Gap" offset="{t120}" start="3600s" duration="{d48}">
              <ref-clip ref="r6" lane="1" name="Compound" offset="3600s" start="0s" duration="{d48}"/>
            </gap>
          </spine>
        </sequence>
      </project>
    </event>
  </library>
</fcpxml>
"#,
        tc0 = ft(tc),
        t60 = ft(tc + 60),
        t72 = ft(tc + 72),
        t120 = ft(tc + 120),
        d1 = ft(1),
        d12 = ft(12),
        d24 = ft(24),
        d48 = ft(48),
        d72 = ft(72),
        d96 = ft(96),
        d240 = ft(240),
    )
}

#[trial]
fn self_referencing_compound_media_is_ignored_without_recursing() {
    let xml = r#"<fcpxml version="1.10">
      <resources>
        <format id="r1" frameDuration="1/24s" width="1920" height="1080"/>
        <media id="r2" name="Loop"><sequence format="r1"><spine>
          <ref-clip ref="r2" offset="0s" start="0s" duration="1s"/>
        </spine></sequence></media>
      </resources>
      <library><event><project name="P"><sequence format="r1"><spine>
        <ref-clip ref="r2" offset="0s" start="0s" duration="1s"/>
      </spine></sequence></project></event></library>
    </fcpxml>"#;
    let (imported, report) = import(xml.as_bytes(), Format::Fcpxml, None).unwrap();
    assert!(!imported.sequences.is_empty());
    assert!(report.mentions("recursive FCPXML media"), "{report:?}");
}

#[trial]
fn golden_connected_clips() {
    let d = doc();
    assert_eq!(detect(d.as_bytes(), Some("fcpxml")), Some(Format::Fcpxml));
    let (imp, rep) = import(d.as_bytes(), Format::Fcpxml, Some("/proj")).unwrap();
    let p = &imp.project;
    let sid = only_seq(&imp);
    assert_eq!(p.item(sid).unwrap().name, "Harbour");
    let s = p.sequence(sid).unwrap();
    let r = FrameRate::FPS_23_976;
    assert_eq!(s.settings.frame_rate, r);
    assert_eq!(s.start_timecode, 86_400);

    assert_connected_primary_video(p, s, r);
    assert_connected_secondary_video(p, s, r);
    assert_connected_audio_markers_and_bins(p, s, r, &rep);
}

fn assert_connected_primary_video(p: &Project, s: &Sequence, r: FrameRate) {
    let v1 = &s.video_tracks[0];
    assert_eq!(v1.items.len(), 2);
    let (pier, boats) = (&v1.items[0], &v1.items[1]);
    assert_eq!((pier.start, pier.duration, pier.source_in), (r.tick_of(0), r.tick_of(72), r.tick_of(24)));
    assert_eq!(media_key(p, pier.item), "/Media/pier wide.mov");
    let pos = pier.effect("motion").unwrap().param("position").unwrap().value.as_vec2().unwrap();
    assert!((pos.x - 1068.0).abs() < 1e-9 && (pos.y - 594.0).abs() < 1e-9, "{pos:?}");
    assert_eq!(pier.effect("motion").unwrap().param("scale").unwrap().value, ParamValue::Float(150.0));
    assert_eq!(pier.effect("motion").unwrap().param("rotation").unwrap().value, ParamValue::Float(-90.0));
    // retimed clip: 2x, source in = 10 s into an asset whose timecode starts at 3600 s
    assert_eq!(boats.speed, 2.0);
    assert_eq!(boats.source_in, Tick(10 * TICKS_PER_SECOND));
    assert_eq!(media_key(p, boats.item), "/proj/boats.mov");
    let t = &v1.transitions[0];
    assert_eq!(
        (t.start, t.duration, t.from, t.to, t.align),
        (r.tick_of(60), r.tick_of(24), Some(pier.id), Some(boats.id), TransitionAlign::CenterAtCut)
    );
}

fn assert_connected_secondary_video(p: &Project, s: &Sequence, r: FrameRate) {
    // connected video on lane 1 (V2) with keyframed opacity; compound clip in a gap
    let v2 = &s.video_tracks[1];
    assert_eq!(v2.items.len(), 2);
    assert_eq!((v2.items[0].start, v2.items[0].duration), (r.tick_of(72), r.tick_of(24)));
    let op = v2.items[0].effect("opacity").unwrap().param("opacity").unwrap();
    assert_eq!(op.keyframes.len(), 2);
    assert_eq!(op.keyframes[1].value, ParamValue::Float(100.0));
    assert_eq!(op.keyframes[1].time, r.tick_of(12));
    assert_eq!(v2.items[1].start, r.tick_of(120));
    assert!(matches!(p.item(v2.items[1].item).unwrap().kind, ItemKind::Sequence(_)));
    assert_eq!(p.item(v2.items[1].item).unwrap().name, "Compound");
}

fn assert_connected_audio_markers_and_bins(p: &Project, s: &Sequence, r: FrameRate, rep: &Report) {
    // audio: pier's own audio on A1 (linked), the lane -1 music pushed to A2 by the overlap
    let a1 = &s.audio_tracks[0].items;
    let pier_link = s.video_tracks[0].items[0].link;
    assert_eq!(a1.len(), 1);
    assert_eq!(a1[0].link, pier_link);
    assert!(pier_link.is_some());
    let theme = &s.audio_tracks[1].items[0];
    assert_eq!((theme.start, theme.duration), (r.tick_of(24), r.tick_of(96)));
    assert_eq!(theme.effect("volume").unwrap().param("level").unwrap().value, ParamValue::Float(-6.0));
    assert!(p.item(theme.item).unwrap().as_media().unwrap().info.video.is_none());
    // the boats asset has no audio: nothing on audio from it
    let boats = s.video_tracks[0].items[1].item;
    assert!(s.audio_tracks.iter().flat_map(|t| &t.items).all(|i| i.item != boats));
    // marker on a primary element is a timeline marker
    assert_eq!(s.markers.len(), 1);
    assert_eq!(
        (s.markers[0].name.as_str(), s.markers[0].comment.as_str(), s.markers[0].start, s.markers[0].duration),
        ("Seagull", "nice", r.tick_of(24), Tick::ZERO)
    );
    // event became a bin
    assert!(p.root.children.iter().any(|c| matches!(c, BinEntry::Bin(b) if b.name == "Day 1")));
    assert!(!rep.has_warnings(), "{rep}");
    let m = p.items.values().find(|i| i.name == "Boats").unwrap().as_media().unwrap();
    assert!(matches!(&m.media, MediaRef::File { path } if path == "/proj/boats.mov"));
}

#[trial]
fn hostile_lane_numbers_do_not_create_sparse_tracks() {
    let d = doc().replace("lane=\"-1\"", "lane=\"-2147483648\"").replace("lane=\"1\"", "lane=\"2147483647\"");
    let (imp, rep) = import(d.as_bytes(), Format::Fcpxml, Some("/proj")).unwrap();
    let s = imp.project.sequence(only_seq(&imp)).unwrap();
    assert!(s.video_tracks.len() <= 256);
    assert!(s.audio_tracks.len() <= 256);
    assert!(rep.has_warnings());
}

/// Final Cut writes slideshow stills directly as video elements, including zero-duration assets.
#[trial]
fn standalone_video_stills_keep_timing_paths_and_transitions() {
    let xml = r#"<fcpxml version="1.11"><resources>
      <format id="f" frameDuration="1/30s" width="1920" height="1080"/>
      <asset id="a" name="Slide A" start="0s" duration="0s" hasVideo="1" format="f">
        <media-rep kind="original-media" src="file:///Missing/slide%20A.jpg"/>
      </asset>
      <asset id="b" name="Slide B" start="0s" duration="0s" hasVideo="1" format="f">
        <media-rep kind="original-media" src="slide-b.jpg"/>
      </asset>
    </resources><project name="Slideshow"><sequence format="f" tcStart="0s"><spine>
      <video ref="a" name="First slide" offset="0s" start="3600s" duration="10s">
        <marker start="3601s" duration="1/30s" value="Slide marker"/>
      </video>
      <transition name="Cross Dissolve" offset="19/2s" duration="1s"/>
      <video ref="b" name="Second slide" offset="10s" start="3600s" duration="10s"/>
    </spine></sequence></project></fcpxml>"#;
    let (imp, report) = import(xml.as_bytes(), Format::Fcpxml, Some("/Slides")).unwrap();
    assert!(!report.has_warnings(), "{report}");
    let p = &imp.project;
    let seq = p.sequence(only_seq(&imp)).unwrap();
    let rate = FrameRate::FPS_30;
    let clips = &seq.video_tracks[0].items;
    assert_eq!(clips.len(), 2);
    for (i, clip) in clips.iter().enumerate() {
        assert_eq!((clip.start, clip.duration, clip.source_in), (rate.tick_of(i as i64 * 300), rate.tick_of(300), rate.tick_of(108_000)));
        assert!(clip.link.is_none());
        assert!(matches!(p.item(clip.item).unwrap().kind, ItemKind::Media(_)));
    }
    assert_eq!(media_key(p, clips[0].item), "/Missing/slide A.jpg");
    assert_eq!(media_key(p, clips[1].item), "/Slides/slide-b.jpg");
    assert!(seq.audio_tracks.iter().all(|t| t.items.is_empty()));
    let transition = &seq.video_tracks[0].transitions[0];
    assert_eq!((transition.from, transition.to), (Some(clips[0].id), Some(clips[1].id)));
    assert_eq!((transition.start, transition.duration), (rate.tick_of(285), rate.tick_of(30)));
    assert_eq!((seq.markers[0].name.as_str(), seq.markers[0].start), ("Slide marker", rate.tick_of(30)));
}

/// A video element selects only picture from an AV asset, including on connected lanes.
#[trial]
fn standalone_video_uses_only_video_and_preserves_connected_placement() {
    let xml = doc().replace("<asset-clip ref=\"r2\"", "<video ref=\"r2\"");
    // The first r2 occurrence has children; the connected r2 occurrence does too.
    let xml = xml.replacen("</asset-clip>\n            <transition", "</video>\n            <transition", 1).replacen(
        "</asset-clip>\n            </asset-clip>",
        "</video>\n            </asset-clip>",
        1,
    );
    let (imp, report) = import(xml.as_bytes(), Format::Fcpxml, Some("/proj")).unwrap();
    assert!(!report.has_warnings(), "{report}");
    let seq = imp.project.sequence(only_seq(&imp)).unwrap();
    let main = &seq.video_tracks[0].items[0];
    let connected = &seq.video_tracks[1].items[0];
    assert_eq!(main.item, connected.item);
    assert_eq!(connected.start, FrameRate::FPS_23_976.tick_of(72));
    assert!(main.link.is_none() && connected.link.is_none());
    assert!(seq.audio_tracks.iter().flat_map(|t| &t.items).all(|i| i.item != main.item));
    assert_eq!(main.effect("motion").unwrap().param("scale").unwrap().value, ParamValue::Float(150.0));
    assert_eq!(connected.effect("opacity").unwrap().param("opacity").unwrap().keyframes.len(), 2);
}

/// Unsupported generators, unresolved references and alternate sources keep an actionable warning.
#[trial]
fn standalone_video_invalid_sources_are_reported_without_panicking() {
    for attrs in ["", "ref=\"missing\"", "ref=\"effect\"", "ref=\"audio\"", "ref=\"picture\" srcID=\"2\""] {
        let xml = format!(
            r#"<fcpxml version="1.11"><resources>
          <effect id="effect" name="Generator" uid="example.generator"/>
          <asset id="audio" name="Audio" hasAudio="1" src="audio.wav"/>
          <asset id="picture" name="Picture" hasVideo="1" src="picture.jpg"/>
        </resources><project name="Unsupported"><sequence><spine>
          <video {attrs} name="Unsupported source" duration="1s"/>
        </spine></sequence></project></fcpxml>"#
        );
        let (imp, report) =
            std::panic::catch_unwind(|| import(xml.as_bytes(), Format::Fcpxml, None)).expect("bad video sources must not panic").unwrap();
        let seq = imp.project.sequence(only_seq(&imp)).unwrap();
        assert!(seq.all_tracks().all(|t| t.items.is_empty()));
        assert!(report.entries.iter().any(|e| e.message.contains("Unsupported source") && e.message.contains("skipped")), "{report}");
    }
}

fn project() -> (Project, ItemId) {
    let r = FrameRate::FPS_29_97;
    let mut p = Project::new("Lib");
    let a = media(&mut p, "/m/a.mov", true, true, r);
    let b = media(&mut p, "/m/b.mov", true, false, r);
    let mus = media(&mut p, "/m/mus.wav", false, true, r);
    let nest = sequence(&mut p, "Nest", r, false);
    clip(&mut p, nest, TrackKind::Video, 0, b, 0..60, 0);
    let s = sequence(&mut p, "Cut", r, true);
    p.sequence_mut(s).unwrap().start_timecode = 107_892;
    let v1 = clip(&mut p, s, TrackKind::Video, 0, a, 0..90, 30);
    let a1 = clip(&mut p, s, TrackKind::Audio, 0, a, 0..90, 30);
    link(&mut p, s, &[v1, a1]);
    let v2 = clip(&mut p, s, TrackKind::Video, 0, b, 90..90 + (60), 0);
    transition(&mut p, s, (TrackKind::Video, 0), "cross_dissolve", (Some(v1), Some(v2)), 80..80 + (20), TransitionAlign::CenterAtCut);
    let v3 = clip(&mut p, s, TrackKind::Video, 0, a, 200..200 + (40), 500);
    transition(&mut p, s, (TrackKind::Video, 0), "dip_to_black", (None, Some(v3)), 200..200 + (10), TransitionAlign::StartAtCut);
    transition(&mut p, s, (TrackKind::Video, 0), "dip_to_black", (Some(v3), None), 230..230 + (10), TransitionAlign::EndAtCut);
    let up = clip(&mut p, s, TrackKind::Video, 1, nest, 30..30 + (60), 0);
    clip(&mut p, s, TrackKind::Video, 2, b, 160..160 + (30), 10);
    clip(&mut p, s, TrackKind::Audio, 2, mus, 0..260, 0);
    {
        let q = p.sequence_mut(s).unwrap();
        let (_, it) = q.find_item_mut(v2).unwrap();
        it.speed = 2.0;
        let mut sc = Param::new(ParamValue::Float(100.0));
        sc.keyframes.push(Keyframe::new(r.tick_of(0), ParamValue::Float(100.0)));
        sc.keyframes.push(Keyframe::new(r.tick_of(60), ParamValue::Float(120.0)));
        it.effect_mut("motion").unwrap().params.insert("scale".into(), sc);
        it.effect_mut("motion").unwrap().params.insert("rotation".into(), Param::new(ParamValue::Float(15.0)));
        let (_, u) = q.find_item_mut(up).unwrap();
        u.enabled = false;
        u.effect_mut("opacity").unwrap().params.insert("opacity".into(), Param::new(ParamValue::Float(40.0)));
        u.markers.push(Marker {
            url: String::new(),
            author: String::new(),
            ext: None,
            id: MarkerId(77),
            start: r.tick_of(10),
            duration: Tick::ZERO,
            name: "in nest".into(),
            comment: String::new(),
            kind: MarkerKind::Comment,
            color: Label::Blue,
        });
        q.markers.push(Marker {
            url: String::new(),
            author: String::new(),
            ext: None,
            id: MarkerId(78),
            start: r.tick_of(100),
            duration: r.tick_of(5),
            name: "chapter".into(),
            comment: "c".into(),
            kind: MarkerKind::Chapter,
            color: Label::Mango,
        });
        let (_, ai) = q.find_item_mut(a1).unwrap();
        ai.effect_mut("volume").unwrap().params.insert("level".into(), Param::new(ParamValue::Float(-3.5)));
    }
    (p, s)
}

#[trial]
fn roundtrip_all_versions() {
    for v in [FcpxmlVersion::V1_9, FcpxmlVersion::V1_10, FcpxmlVersion::V1_11] {
        let (p, s) = project();
        let o = ExportOptions { fcpxml_version: v, ..xopts() };
        let (imp, text, rep) = roundtrip(&p, s, Format::Fcpxml, &o);
        assert!(text.contains(&format!("<fcpxml version=\"{}\">", v.as_str())));
        assert!(text.contains("tcFormat=\"DF\""));
        assert!(!rep.has_warnings(), "{rep}");
        let si = only_seq(&imp);
        let q = &imp.project;
        for kind in [TrackKind::Video, TrackKind::Audio] {
            assert_eq!(structure(q, si, kind), structure(&p, s, kind), "{text}");
        }
        assert_eq!(links(q, si), links(&p, s));
        let qs = q.sequence(si).unwrap();
        let ps = p.sequence(s).unwrap();
        assert_eq!(qs.start_timecode, 107_892);
        assert_eq!(
            qs.markers.iter().map(|m| (&m.name, m.start, m.duration, m.kind)).collect::<Vec<_>>(),
            ps.markers.iter().map(|m| (&m.name, m.start, m.duration, m.kind)).collect::<Vec<_>>()
        );
        let pv = &ps.video_tracks[0].items[1];
        let qv = &qs.video_tracks[0].items[1];
        assert_eq!(qv.effect("motion").unwrap().param("scale"), pv.effect("motion").unwrap().param("scale"), "{text}");
        assert_eq!(qv.effect("motion").unwrap().param("rotation").unwrap().value, ParamValue::Float(15.0));
        let up = &qs.video_tracks[1].items[0];
        assert_eq!(up.effect("opacity").unwrap().param("opacity").unwrap().value, ParamValue::Float(40.0));
        assert_eq!(
            up.markers.iter().map(|m| (&m.name, m.start)).collect::<Vec<_>>(),
            vec![(&"in nest".to_string(), FrameRate::FPS_29_97.tick_of(10))]
        );
        let lvl = qs.audio_tracks[0].items[0].effect("volume").unwrap().param("level").unwrap().value.as_f64().unwrap();
        assert!((lvl + 3.5).abs() < 1e-9);
    }
}

#[derive(Clone, Debug)]
struct G {
    track: usize,
    gap: i64,
    dur: i64,
    media: usize,
    src: i64,
    speed: u8,
    enabled: bool,
    tr: u8,
    tr_len: i64,
    audio: u8,
}

fn gen_clip() -> impl Strategy<Value = G> {
    (0usize..3, 0i64..3, 20i64..150, 0usize..3, 0i64..2000, 0u8..5, prop::bool::weighted(0.9), 0u8..6, 1i64..9, 0u8..3).prop_map(
        |(track, gap, dur, media, src, speed, enabled, tr, tr_len, audio)| G {
            track,
            gap: gap * 5,
            dur,
            media,
            src,
            speed,
            enabled,
            tr,
            tr_len,
            audio,
        },
    )
}

#[trial]
fn fcpxml_roundtrip() {
    let mut config = ProptestConfig::with_cases(48);
    config.source_file = Some(file!());
    let mut runner = proptest::test_runner::TestRunner::new(config);
    runner
        .run(&(prop::collection::vec(gen_clip(), 1..20), 0usize..5), |(clips, ri)| {
            let rate = [FrameRate::FPS_23_976, FrameRate::FPS_25, FrameRate::FPS_29_97, FrameRate::FPS_59_94, FrameRate::FPS_24][ri];
            let mut p = Project::new("P");
            let ms = [
                media(&mut p, "/x/one.mov", true, true, rate),
                media(&mut p, "/x/two.mov", true, true, rate),
                media(&mut p, "/y/three four.mxf", true, true, rate),
            ];
            let wav = media(&mut p, "/y/vo.wav", false, true, rate);
            let s = sequence(&mut p, "S", rate, false);
            let mut ends = [0i64; 3];
            let mut prev: [Option<ClipId>; 3] = [None; 3];
            let mut aends = [0i64; 3];
            for g in &clips {
                let t = ends[g.track] + g.gap;
                let v = clip(&mut p, s, TrackKind::Video, g.track, ms[g.media], t..t + (g.dur), g.src);
                {
                    let q = p.sequence_mut(s).unwrap();
                    let (_, it) = q.find_item_mut(v).unwrap();
                    it.enabled = g.enabled;
                    match g.speed {
                        1 => it.speed = 2.0,
                        2 => it.speed = 0.5,
                        3 => it.reverse = true,
                        _ => {}
                    }
                }
                let d = g.tr_len;
                let adjacent = g.gap == 0 && prev[g.track].is_some();
                if g.track == 0 {
                    match g.tr {
                        0 if adjacent && d >= 2 => transition(
                            &mut p,
                            s,
                            (TrackKind::Video, 0),
                            "cross_dissolve",
                            (prev[0], Some(v)),
                            t - d / 2..t - d / 2 + (d),
                            TransitionAlign::CenterAtCut,
                        ),
                        1 if adjacent => {
                            transition(&mut p, s, (TrackKind::Video, 0), "dip_to_white", (prev[0], Some(v)), t..t + (d), TransitionAlign::StartAtCut)
                        }
                        2 if adjacent => {
                            transition(&mut p, s, (TrackKind::Video, 0), "push", (prev[0], Some(v)), t - d..t - d + (d), TransitionAlign::EndAtCut)
                        }
                        3 if !adjacent => {
                            transition(&mut p, s, (TrackKind::Video, 0), "cross_dissolve", (None, Some(v)), t..t + (d), TransitionAlign::StartAtCut)
                        }
                        _ => {}
                    }
                }
                if aends[g.track] <= t {
                    match g.audio {
                        // linked, same-numbered audio track, identical timing
                        1 => {
                            let a = clip(&mut p, s, TrackKind::Audio, g.track, ms[g.media], t..t + (g.dur), g.src);
                            {
                                let q = p.sequence_mut(s).unwrap();
                                let (sp, rv, en) = {
                                    let (_, it) = q.find_item(v).unwrap();
                                    (it.speed, it.reverse, it.enabled)
                                };
                                let (_, ai) = q.find_item_mut(a).unwrap();
                                ai.speed = sp;
                                ai.reverse = rv;
                                ai.enabled = en;
                            }
                            link(&mut p, s, &[v, a]);
                            aends[g.track] = t + g.dur;
                        }
                        // independent audio-only clip
                        2 => {
                            clip(&mut p, s, TrackKind::Audio, g.track, wav, t..t + (g.dur), g.src);
                            aends[g.track] = t + g.dur;
                        }
                        _ => {}
                    }
                }
                ends[g.track] = t + g.dur;
                prev[g.track] = Some(v);
            }
            let (imp, text, _) = roundtrip(&p, s, Format::Fcpxml, &xopts());
            let si = only_seq(&imp);
            prop_assert_eq!(structure(&imp.project, si, TrackKind::Video), structure(&p, s, TrackKind::Video), "{}", text);
            prop_assert_eq!(structure(&imp.project, si, TrackKind::Audio), structure(&p, s, TrackKind::Audio), "{}", text);
            prop_assert_eq!(links(&imp.project, si), links(&p, s));
            Ok(())
        })
        .unwrap();
}

/// filmcraft#2195: a relative `src` is a URL like an absolute one: `boats%20b.mov` is the file `boats b.mov`.
#[trial]
fn relative_media_urls_are_percent_decoded() {
    let d = doc().replace(r#"src="boats.mov""#, r#"src="media/boats%20b.mov""#);
    let (imp, _) = import(d.as_bytes(), Format::Fcpxml, Some("/proj")).unwrap();
    let p = &imp.project;
    let boats = &p.sequence(only_seq(&imp)).unwrap().video_tracks[0].items[1];
    assert_eq!(media_key(p, boats.item), "/proj/media/boats b.mov");
}

// Parity regressions from FilmCraft e6df318 (including #4799), adapted to the neutral model.
#[trial]
fn media_cut_off_by_the_depth_limit_is_complete_when_reached_higher_up() {
    // m0 … m64 nest one inside the next. Project A reaches m63 at the depth limit, so m64 is left
    // out of it; project B refers to m63 directly, where m64 fits, and must not get the cut-off copy.
    let mut media = String::new();
    for i in 0..64 {
        media.push_str(&format!(
            r#"<media id="m{i}" name="M{i}"><sequence format="r1"><spine><ref-clip ref="m{}" offset="0s" start="0s" duration="1s"/></spine></sequence></media>"#,
            i + 1
        ));
    }
    media.push_str(r#"<media id="m64" name="M64"><sequence format="r1"><spine/></sequence></media>"#);
    let xml = format!(
        r#"<fcpxml version="1.10"><resources><format id="r1" frameDuration="1/24s" width="1920" height="1080"/>{media}</resources>
        <library><event>
          <project name="A"><sequence format="r1"><spine><ref-clip ref="m0" offset="0s" start="0s" duration="1s"/></spine></sequence></project>
          <project name="B"><sequence format="r1"><spine><ref-clip ref="m63" offset="0s" start="0s" duration="1s"/></spine></sequence></project>
        </event></library></fcpxml>"#
    );
    let (imp, rep) = import(xml.as_bytes(), Format::Fcpxml, None).unwrap();
    assert!(rep.mentions("nested too deeply"), "{rep:?}");
    let m63 = imp.project.items.values().find(|i| i.name == "M63").and_then(|i| imp.project.sequence(i.id)).unwrap();
    assert_eq!(m63.video_tracks[0].items.len(), 1, "M63 keeps its nested M64");
}

#[trial]
fn media_cut_off_by_xml_layout_is_complete_when_revisited_at_the_same_media_depth() {
    // Both references have media-chain depth zero, but the first is reached through connected
    // clips. Its resource must not remain cached as complete after the XML-layout guard fires.
    for depth in [125, 126] {
        let nested = format!(
            "{}<ref-clip lane=\"1\" ref=\"m0\" offset=\"0s\" duration=\"1s\"/>{}",
            "<gap lane=\"1\" offset=\"0s\" duration=\"1s\">".repeat(depth),
            "</gap>".repeat(depth)
        );
        let xml = format!(
            r#"<fcpxml version="1.10"><resources>
          <format id="r1" frameDuration="1/24s" width="1920" height="1080"/>
          <media id="m0" name="M0"><sequence format="r1"><spine>
            <ref-clip ref="m1" offset="0s" duration="1s"/>
          </spine></sequence></media>
          <media id="m1" name="M1"><sequence format="r1"><spine/></sequence></media>
        </resources><library><event>
          <project name="A"><sequence format="r1"><spine>{nested}</spine></sequence></project>
          <project name="B"><sequence format="r1"><spine>
            <ref-clip ref="m0" offset="0s" duration="1s"/>
          </spine></sequence></project>
        </event></library></fcpxml>"#
        );
        let (imp, rep) = import(xml.as_bytes(), Format::Fcpxml, None).unwrap();
        assert!(rep.mentions("XML") && rep.mentions("nested"), "{rep:?}");
        let m0 = imp.project.items.values().find(|i| i.name == "M0").and_then(|i| imp.project.sequence(i.id)).unwrap();
        assert_eq!(m0.video_tracks[0].items.len(), 1, "M0 keeps its nested M1 after the shallower revisit");
    }
}

#[trial]
fn flat_resource_chain_is_bounded_even_when_xml_is_shallow() {
    let mut media = String::new();
    for i in 0..512 {
        media.push_str(&format!(
            r#"<media id="m{i}" name="M{i}"><sequence><spine><ref-clip ref="m{}" duration="1s"/></spine></sequence></media>"#,
            i + 1
        ));
    }
    let xml = format!(
        r#"<fcpxml version="1.10"><resources>{media}</resources><project name="P"><sequence><spine><ref-clip ref="m0" duration="1s"/></spine></sequence></project></fcpxml>"#
    );
    let (imp, rep) = import(xml.as_bytes(), Format::Fcpxml, None).unwrap();
    assert!(rep.mentions("nested too deeply"), "{rep:?}");
    assert!(imp.project.items.len() <= 65, "the flat resource graph is not expanded beyond the semantic limit");
}

#[trial]
fn shallower_retry_does_not_link_a_cached_ancestor_back_to_the_rebuilt_item() {
    let mut media = String::new();
    for i in 0..64 {
        media.push_str(&format!(
            r#"<media id="m{i}" name="M{i}"><sequence><spine><ref-clip ref="m{}" duration="1s"/></spine></sequence></media>"#,
            (i + 1) % 64
        ));
    }
    let xml = format!(
        r#"<fcpxml version="1.10"><resources>{media}</resources>
        <project name="A"><sequence><spine><ref-clip ref="m0" duration="1s"/></spine></sequence></project>
        <project name="B"><sequence><spine><ref-clip ref="m62" duration="1s"/></spine></sequence></project></fcpxml>"#
    );
    let (imp, rep) = import(xml.as_bytes(), Format::Fcpxml, None).unwrap();
    assert!(rep.mentions("cached media"), "{rep:?}");
    // Check the final graph, not just the resource-name recursion stack during import.
    fn visit(p: &Project, id: ItemId, active: &mut std::collections::HashSet<ItemId>, done: &mut std::collections::HashSet<ItemId>) {
        if done.contains(&id) {
            return;
        }
        assert!(active.insert(id), "cached media introduced a cycle at {id:?}");
        if let Some(seq) = p.sequence(id) {
            for item in seq.all_tracks().flat_map(|track| &track.items) {
                visit(p, item.item, active, done);
            }
        }
        active.remove(&id);
        done.insert(id);
    }
    let mut active = std::collections::HashSet::new();
    let mut done = std::collections::HashSet::new();
    for id in imp.project.items.keys() {
        visit(&imp.project, *id, &mut active, &mut done);
    }
}

#[trial]
fn optional_report_limit_bounds_distinct_fcpxml_diagnostics() {
    let unknown = (0..10).map(|i| format!("<video name=\"missing{i}\" duration=\"1s\"/> ")).collect::<String>();
    let doc = format!(
        "<fcpxml version=\"1.10\"><resources/><library><event><project name=\"P\"><sequence><spine>{unknown}</spine></sequence></project></event></library></fcpxml>"
    );
    let opts = iopts(None);
    let mut limited = Report::default();
    assert!(craft_timeline::fcpxml::import_with_report_limit(&doc, &opts, &mut limited, Some(2)).is_ok());
    assert_eq!(limited.entries.len(), 3);
    assert!(limited.mentions("further diagnostics omitted"));
    let mut ordinary = Report::default();
    assert!(craft_timeline::fcpxml::import(&doc, &opts, &mut ordinary).is_ok());
    assert_eq!(ordinary.entries.len(), 10);
}

#[trial]
fn optional_report_limit_bounds_fcpxml_export_diagnostics_without_changing_output() {
    let (p, s) = project();
    let mut ordinary = Report::default();
    let text = craft_timeline::fcpxml::export(&p, s, &xopts(), &mut ordinary).unwrap();
    let mut limited = Report::default();
    let bounded = craft_timeline::fcpxml::export_with_report_limit(&p, s, &xopts(), &mut limited, Some(0)).unwrap();
    assert_eq!(bounded, text);
    let total = |r: &Report| r.entries.iter().map(|e| e.count).sum::<u32>();
    assert_eq!(total(&limited), total(&ordinary));
    assert!(limited.entries.iter().all(|e| e.message == "further diagnostics omitted"), "{limited}");
}

// Original metadata-only resources: A first cuts off M64, then B rebuilds M63/M64 at a
// shallower depth. C repeats that same cache hit, without another reason to rebuild it.
fn deferred_audio_document(m62: &str, leaf_extra: &str, audio_only_chain: bool) -> String {
    deferred_audio_document_at(62, m62, leaf_extra, audio_only_chain)
}

fn deferred_audio_document_at(custom_index: usize, custom_spine: &str, leaf_extra: &str, audio_only_chain: bool) -> String {
    let selection = if audio_only_chain { r#" srcEnable="audio""# } else { "" };
    let mut resources = String::new();
    for i in 0..64 {
        let spine = if i == custom_index && !custom_spine.is_empty() {
            custom_spine.to_string()
        } else {
            format!(r#"<ref-clip ref="m{}" name="Next" duration="20s"{selection}/>"#, i + 1)
        };
        resources.push_str(&format!(r#"<media id="m{i}" name="M{i}"><sequence format="f"><spine>{spine}</spine></sequence></media>"#));
    }
    format!(
        r#"<fcpxml version="1.10"><resources>
          <format id="f" frameDuration="1/24s" width="1920" height="1080"/>
          <asset id="av" name="Original AV" start="0s" duration="60s" hasVideo="1" hasAudio="1" format="f" audioChannels="2">
            <media-rep kind="original-media" src="file:///original-deferred-audio.mov"/>
          </asset>
          {resources}
          <media id="m64" name="M64"><sequence format="f"><spine>
            <asset-clip ref="av" name="Real audio" duration="20s"/>{leaf_extra}
          </spine></sequence></media>
        </resources><library><event>
          <project name="A"><sequence format="f"><spine><ref-clip ref="m0" name="Earlier parent" duration="20s"{selection}/></spine></sequence></project>
          <project name="B"><sequence format="f"><spine><ref-clip ref="m63" duration="20s"/></spine></sequence></project>
          <project name="C"><sequence format="f"><spine><ref-clip ref="m63" duration="20s"/></spine></sequence></project>
        </event></library></fcpxml>"#
    )
}

fn deferred_audio_sequence<'a>(p: &'a Project, name: &str) -> (ItemId, &'a Sequence) {
    let item = p.items.values().find(|i| i.name == name).unwrap();
    (item.id, p.sequence(item.id).unwrap())
}

fn deferred_audio_pair(seq: &Sequence, name: &str) {
    let video = seq.video_tracks.iter().flat_map(|t| &t.items).find(|i| i.name == name).unwrap();
    let audio = seq.audio_tracks.iter().flat_map(|t| &t.items).filter(|i| i.name == name).collect::<Vec<_>>();
    assert_eq!(audio.len(), 1, "deferred parent audio was not restored exactly once: {name}");
    let audio = audio[0];
    assert!(video.link.is_some());
    assert_eq!(audio.link, video.link);
    assert_eq!((audio.item, audio.start, audio.duration, audio.source_in), (video.item, video.start, video.duration, video.source_in));
    assert_eq!((audio.speed, audio.reverse, audio.frame_hold, audio.enabled), (video.speed, video.reverse, video.frame_hold, video.enabled));
}

#[trial]
fn deferred_audio_returns_after_shallower_rebuild() {
    let xml = deferred_audio_document("", "", false);
    let (imp, report) = import(xml.as_bytes(), Format::Fcpxml, None).unwrap();
    let p = &imp.project;
    assert_eq!(imp.sequences.len(), 3);
    let (_, a) = deferred_audio_sequence(p, "A");
    let (_, b) = deferred_audio_sequence(p, "B");
    let (_, c) = deferred_audio_sequence(p, "C");
    assert_eq!(a.video_tracks[0].items.len(), 1, "the earlier picture must already exist");
    assert_eq!(p.item(a.video_tracks[0].items[0].item).unwrap().name, "M0");
    assert!(report.mentions("nested too deeply"), "{report}");
    let (m63_id, m63) = deferred_audio_sequence(p, "M63");
    let (m64_id, m64) = deferred_audio_sequence(p, "M64");
    assert_eq!(m63.audio_tracks[0].items.len(), 1, "the permitted shallower rebuild must really reach audio");
    assert_eq!(m63.audio_tracks[0].items[0].item, m64_id);
    assert_eq!(media_key(p, m64.audio_tracks[0].items[0].item), "/original-deferred-audio.mov");
    assert_eq!(b.audio_tracks[0].items[0].item, m63_id);
    assert_eq!(c.audio_tracks[0].items[0].item, m63_id);
    // The baseline fails here, after proving that parsing, depth truncation and child recovery worked.
    deferred_audio_pair(a, "Earlier parent");
    for i in 0..64 {
        let (_, seq) = deferred_audio_sequence(p, &format!("M{i}"));
        deferred_audio_pair(seq, "Next");
    }
    assert_eq!(b.audio_tracks.iter().map(|t| t.items.len()).sum::<usize>(), 1);
    assert_eq!(c.audio_tracks.iter().map(|t| t.items.len()).sum::<usize>(), 1);
}

#[trial]
fn deferred_audio_keeps_selection_adjustments_and_overlap_lanes() {
    use craft_timeline::interchange::{Catalog, EffectDef, EffectInstance, EffectKind, ParamDef};

    struct AudioCatalog;
    impl Catalog for AudioCatalog {
        fn effect(&self, id: &str) -> Option<EffectDef> {
            if id == "original_audio_filter" {
                Some(EffectDef {
                    id: id.into(),
                    name: "Original audio filter".into(),
                    kind: EffectKind::Audio,
                    category: "Original test".into(),
                    intrinsic: false,
                    params: vec![ParamDef { id: "mix".into(), label: "Mix".into(), default: ParamValue::Float(0.0) }],
                })
            } else {
                TestCatalog.effect(id)
            }
        }
        fn effect_by_name(&self, name: &str) -> Option<EffectDef> {
            if name == "Original audio filter" { self.effect("original_audio_filter") } else { TestCatalog.effect_by_name(name) }
        }
        fn intrinsic(&self, kind: TrackKind) -> Vec<EffectInstance> {
            TestCatalog.intrinsic(kind)
        }
    }
    let refs = r#"
      <asset-clip ref="av" name="Existing sound" offset="0s" duration="4s" srcEnable="audio"/>
      <ref-clip ref="m63" name="Adjusted" offset="0s" start="3s" duration="4s" enabled="0" label="Rose">
        <adjust-volume amount="-6dB"><param name="amount"><keyframeAnimation>
          <keyframe time="3s" value="-6dB"/><keyframe time="5s" value="-3dB"/>
        </keyframeAnimation></param></adjust-volume>
        <filter-audio name="Original audio filter" enabled="0"><param key="mix" value="0.25"/></filter-audio>
        <marker start="4s" duration="1/24s" value="Cue" note="original"/>
        <ref-clip ref="m63" name="Positive lane" lane="2" offset="3s" duration="2s"/>
        <ref-clip ref="m63" name="Negative lane" lane="-3" offset="3s" duration="2s" srcEnable="audio"/>
      </ref-clip>
      <ref-clip ref="m63" name="Audio only" offset="4s" duration="2s" srcEnable="audio"/>
      <ref-clip ref="m63" name="Video only" offset="6s" duration="2s" srcEnable="video"/>
      <ref-clip ref="m63" name="Explicit all" offset="8s" duration="2s" srcEnable="all"/>
    "#;
    // Leave room for the connected children under the unchanged XML-depth cap. The direct
    // M61 → M63 edge still reaches M64's storyline at depth128, deferring audio until B rebuilds it.
    let xml = deferred_audio_document_at(61, refs, "", false);
    let mut report = Report::default();
    let opts = craft_timeline::ImportOptions { catalog: &AudioCatalog, ..iopts(None) };
    let imp = craft_timeline::fcpxml::import(&xml, &opts, &mut report).unwrap();
    assert!(report.mentions("nested too deeply"), "{report}");
    let (_, seq) = deferred_audio_sequence(&imp.project, "M61");
    let videos = seq.video_tracks.iter().flat_map(|t| &t.items).collect::<Vec<_>>();
    let audios = seq.audio_tracks.iter().flat_map(|t| &t.items).collect::<Vec<_>>();
    assert!(videos.iter().all(|i| !matches!(i.name.as_str(), "Audio only" | "Negative lane" | "Existing sound")));
    assert!(audios.iter().all(|i| i.name != "Video only"));
    assert!(videos.iter().find(|i| i.name == "Video only").unwrap().link.is_none());
    for name in ["Adjusted", "Positive lane", "Explicit all"] {
        deferred_audio_pair(seq, name);
    }
    let adjusted = seq.audio_tracks[1].items.iter().find(|i| i.name == "Adjusted").unwrap();
    assert_eq!(seq.audio_tracks[0].items[0].name, "Existing sound");
    assert_eq!(adjusted.source_in, Tick(3 * TICKS_PER_SECOND));
    assert!(!adjusted.enabled);
    assert_eq!(adjusted.label, Label::Rose);
    assert_eq!(adjusted.effect("volume").unwrap().param("level").unwrap().value, ParamValue::Float(-6.0));
    let keys = &adjusted.effect("volume").unwrap().param("level").unwrap().keyframes;
    assert_eq!(keys.len(), 2);
    assert_eq!((keys[1].time, keys[1].value.clone()), (Tick(5 * TICKS_PER_SECOND), ParamValue::Float(-3.0)));
    let effect = adjusted.effect("original_audio_filter").unwrap();
    assert!(!effect.enabled);
    assert_eq!(effect.param("mix").unwrap().value, ParamValue::Float(0.25));
    assert_eq!(adjusted.markers.len(), 1);
    assert_eq!((adjusted.markers[0].name.as_str(), adjusted.markers[0].comment.as_str()), ("Cue", "original"));
    assert_eq!(adjusted.markers[0].start, Tick(4 * TICKS_PER_SECOND));
    assert!(seq.audio_tracks[2].items.iter().any(|i| i.name == "Positive lane"));
    assert!(seq.audio_tracks[3].items.iter().any(|i| i.name == "Negative lane"), "overlapping lane -3 must move past occupied A3");
    let audio_only = audios.iter().filter(|i| i.name == "Audio only").collect::<Vec<_>>();
    assert_eq!(audio_only.len(), 1);
    assert!(audio_only[0].link.is_none(), "audio-only restoration must not manufacture a picture anchor");
}

#[trial]
fn deferred_audio_keeps_retime_reverse_hold_and_export_links() {
    for (end, expected_speed, reverse, hold, source) in [(11, 2.0, false, false, 3), (1, 0.5, true, false, 1), (3, 1.0, false, true, 3)] {
        let refs = format!(
            r#"<ref-clip ref="m63" name="Timed" start="3s" duration="4s"><timeMap>
              <timept time="3s" value="3s"/><timept time="7s" value="{end}s"/>
            </timeMap></ref-clip>"#
        );
        let xml = deferred_audio_document(&refs, "", false);
        let (imp, _) = import(xml.as_bytes(), Format::Fcpxml, None).unwrap();
        let (id, seq) = deferred_audio_sequence(&imp.project, "M62");
        deferred_audio_pair(seq, "Timed");
        let audio = &seq.audio_tracks[0].items[0];
        assert_eq!((audio.start, audio.duration, audio.source_in), (Tick::ZERO, Tick(4 * TICKS_PER_SECOND), Tick(source * TICKS_PER_SECOND)));
        assert_eq!((audio.speed, audio.reverse), (expected_speed, reverse));
        assert_eq!(audio.frame_hold, hold.then_some(Tick(source * TICKS_PER_SECOND)));
        // Export the shallow rebuilt parent, not an artificially unbounded 65-level traversal.
        let (reopened, _, report) = roundtrip(&imp.project, id, Format::Fcpxml, &xopts());
        let seq = reopened.project.sequence(only_seq(&reopened)).unwrap();
        assert!(!report.mentions("nested too deeply"), "{report}");
        deferred_audio_pair(seq, "Timed");
        let again = &seq.audio_tracks[0].items[0];
        assert_eq!(
            (again.duration, again.source_in, again.speed, again.reverse, again.frame_hold),
            (audio.duration, audio.source_in, audio.speed, audio.reverse, audio.frame_hold)
        );
    }
}

#[trial]
fn deferred_audio_export_keeps_unmerged_nested_sound_on_its_own_lane() {
    let refs = r#"
      <asset-clip ref="av" name="Existing sound" offset="0s" duration="4s" srcEnable="audio"/>
      <ref-clip ref="m63" name="Restored" offset="0s" duration="4s"/>
      <asset-clip ref="av" name="Neighbour" offset="6s" duration="1s" srcEnable="video"/>
    "#;
    let xml = deferred_audio_document_at(61, refs, "", false);
    let (imported, _) = import(xml.as_bytes(), Format::Fcpxml, None).unwrap();
    let (id, seq) = deferred_audio_sequence(&imported.project, "M61");
    deferred_audio_pair(seq, "Restored");
    assert_eq!(seq.audio_tracks[1].items[0].name, "Restored", "the collision must leave picture and sound on different lanes");
    let expected = deferred_audio_lanes(seq);
    let original =
        [seq.video_tracks.iter().flat_map(|t| &t.items).find(|i| i.name == "Restored").unwrap().clone(), seq.audio_tracks[1].items[0].clone()];
    let (reopened, text, report) = roundtrip(&imported.project, id, Format::Fcpxml, &xopts());
    assert!(!report.mentions("track import limit"), "{report}");
    // Separate source selections carry the two lanes; FCPXML ref-clip has no cross-reference
    // for the model's internal LinkId (Apple's FCPXML 1.10 DTD, ref-clip/clip_attrs).
    let xml = craft_xml::parse(text.as_bytes()).unwrap();
    let spine = xml.child("library").unwrap().child("event").unwrap().child("project").unwrap().child("sequence").unwrap().child("spine").unwrap();
    let refs = spine.find_all("ref-clip").filter(|e| e.attr("name") == Some("Restored")).collect::<Vec<_>>();
    assert_eq!(refs.len(), 2, "exactly one picture ref and one sound ref must be exported");
    let picture = refs.iter().find(|e| e.attr("lane").is_none()).unwrap();
    let sound = refs.iter().find(|e| e.attr("lane") == Some("-2")).unwrap();
    assert_eq!(picture.attr("srcEnable"), Some("video"), "an unmerged nested picture must not reintroduce its sound");
    assert_eq!(sound.attr("srcEnable"), Some("audio"));
    assert!(picture.attr("ref").is_some());
    assert_eq!(picture.attr("ref"), sound.attr("ref"), "both selections must still reference the same nested resource");
    let again = reopened.project.sequence(only_seq(&reopened)).unwrap();
    assert_eq!(deferred_audio_lanes(again), expected, "the unmerged picture must not create a second audio reference");
    let picture = again.video_tracks.iter().flat_map(|t| &t.items).find(|i| i.name == "Restored").unwrap();
    let sound = again.audio_tracks[1].items.iter().find(|i| i.name == "Restored").unwrap();
    assert_eq!(picture.item, sound.item);
    // Reimporting separate video-only and audio-only refs does not invent a shared LinkId.
    // Preserve all other clip state, normalizing only IDs local to the newly imported project.
    for (before, after) in original.iter().zip([picture, sound]) {
        assert!(after.link.is_none());
        assert_eq!(reopened.project.item(after.item).unwrap().name, imported.project.item(before.item).unwrap().name);
        let mut expected = before.clone();
        expected.id = after.id;
        expected.item = after.item;
        expected.link = None;
        // Auto motion points use NaN; compare effects as in the OTIO round-trip control.
        assert_eq!(format!("{:?}", after.effects), format!("{:?}", expected.effects));
        expected.effects.clear();
        let mut actual = after.clone();
        actual.effects.clear();
        assert_eq!(actual, expected, "split source selections must retain their timing and state");
    }
    let neighbour = again.video_tracks.iter().flat_map(|t| &t.items).find(|i| i.name == "Neighbour").unwrap();
    assert_eq!((neighbour.start, neighbour.duration), (Tick(6 * TICKS_PER_SECOND), Tick(TICKS_PER_SECOND)));
}

#[trial]
fn deferred_audio_diagnostics_require_final_placed_sound() {
    let refs = r#"
      <ref-clip ref="m63" name="Pending" duration="4s"><filter-audio name="Missing deferred filter"/></ref-clip>
      <title name="Actual omission" offset="5s" duration="1s"/>
    "#;
    let xml = deferred_audio_document_at(61, refs, "", false);
    let leaf = r#"<asset-clip ref="av" name="Real audio" duration="20s"/>"#;
    let silent = xml.replacen(leaf, r#"<asset-clip ref="av" name="Real audio" duration="20s" srcEnable="video"/>"#, 1);
    for (text, recovered) in [(&xml, true), (&silent, false)] {
        let (_, report) = import(text.as_bytes(), Format::Fcpxml, None).unwrap();
        let count = report.entries.iter().filter(|e| e.message.contains("Missing deferred filter")).map(|e| e.count).sum::<u32>();
        assert_eq!(count, u32::from(recovered), "only placed audio may report its filter, once: {report}");
    }
    let mut limited = Report::default();
    let opts = iopts(None);
    craft_timeline::fcpxml::import_with_report_limit(&silent, &opts, &mut limited, Some(2)).unwrap();
    assert!(limited.mentions("<title>"), "a nonexistent audio omission must not consume the actual omission's report slot: {limited}");
    assert!(!limited.mentions("Missing deferred filter"), "{limited}");

    let filters = r#"<filter-audio name="A"/><filter-audio name="B"/><filter-audio name="X"/>"#;
    let global_xml = xml.replace(r#"<filter-audio name="Missing deferred filter"/>"#, filters);
    let mut global = Report::default();
    let existing = "effect \"X\" is not supported and was skipped";
    global.warn(existing);
    craft_timeline::fcpxml::import_with_report_limit(&global_xml, &opts, &mut global, Some(2)).unwrap();
    assert_eq!(
        global.entries.iter().find(|e| e.message == existing).unwrap().count,
        2,
        "deferred A/B must not hide another occurrence of already reported X: {global}"
    );
    assert!(global.entries.len() <= 3, "the global quota plus its overflow message still bounds retention: {global}");
}

#[trial]
fn deferred_audio_replay_updates_ordinary_video_partners() {
    for (offset, placed) in [(0, false), (6, true)] {
        let refs = format!(
            r#"<gap duration="10s">
              <ref-clip ref="m63" name="Earlier sound" lane="-256" offset="0s" duration="5s" srcEnable="audio"/>
              <asset-clip ref="av" name="Ordinary pair" lane="255" offset="{offset}s" duration="3s"/>
            </gap>"#
        );
        let xml = deferred_audio_document_at(61, &refs, "", false);
        let (imported, _) = import(xml.as_bytes(), Format::Fcpxml, None).unwrap();
        let (_, seq) = deferred_audio_sequence(&imported.project, "M61");
        let video = seq.video_tracks.iter().flat_map(|t| &t.items).find(|i| i.name == "Ordinary pair").unwrap();
        let audio = seq.audio_tracks.iter().flat_map(|t| &t.items).find(|i| i.name == "Ordinary pair");
        assert_eq!(audio.is_some(), placed, "the earlier deferred sound has first choice of the final lane");
        assert_eq!(video.link.is_some(), placed, "a displaced ordinary sound must not leave a dangling video link");
        if let Some(audio) = audio {
            assert_eq!(audio.link, video.link);
        }
    }
    // The last AV clip initially loses its audio to a collision, then fits after replay
    // moves O1. Its existing picture must regain the same pair identity.
    let refs = r#"<gap duration="13s">
      <ref-clip ref="m63" name="D" lane="-255" offset="0s" duration="5s" srcEnable="audio"/>
      <asset-clip ref="av" name="O0" lane="-255" offset="0s" duration="10s" srcEnable="audio"/>
      <asset-clip ref="av" name="O1" lane="-255" offset="9s" duration="3s" srcEnable="audio"/>
      <asset-clip ref="av" name="O2 pair" lane="255" offset="11s" duration="2s"/>
    </gap>"#;
    let xml = deferred_audio_document_at(61, refs, "", false);
    let (imported, report) = import(xml.as_bytes(), Format::Fcpxml, None).unwrap();
    let (_, seq) = deferred_audio_sequence(&imported.project, "M61");
    assert!(!report.mentions("track import limit"), "{report}");
    deferred_audio_pair(seq, "O2 pair");

    // No successful replay is available to clear this pair: either there is no pending
    // child, or the pending child becomes video-only when rebuilt.
    for silent_child in [false, true] {
        let pending =
            if silent_child { r#"<ref-clip ref="m63" name="Silent child" lane="-1" offset="6s" duration="1s" srcEnable="audio"/>"# } else { "" };
        let refs = format!(
            r#"<gap duration="10s">
              <asset-clip ref="av" name="Occupier" lane="-256" offset="0s" duration="5s" srcEnable="audio"/>
              <asset-clip ref="av" name="Failed pair" lane="255" offset="0s" duration="3s"/>
              {pending}
            </gap>"#
        );
        let xml = deferred_audio_document_at(61, &refs, "", false).replace(
            r#"<asset-clip ref="av" name="Real audio" duration="20s"/>"#,
            r#"<asset-clip ref="av" name="Real audio" duration="20s" srcEnable="video"/>"#,
        );
        let (imported, report) = import(xml.as_bytes(), Format::Fcpxml, None).unwrap();
        let (_, seq) = deferred_audio_sequence(&imported.project, "M61");
        assert!(seq.audio_tracks.iter().flat_map(|t| &t.items).all(|a| a.name != "Failed pair"));
        let video = seq.video_tracks.iter().flat_map(|t| &t.items).find(|v| v.name == "Failed pair").unwrap();
        assert!(video.link.is_none(), "an initially unplaced sound has no partner even without a ready replay");
        let cap_count = report.entries.iter().filter(|e| e.message.contains("Audio lane exceeds")).map(|e| e.count).sum::<u32>();
        assert_eq!(cap_count, 1, "only the final ordinary placement failed: {report}");
    }
}

#[trial]
fn deferred_audio_only_cached_back_edges_stay_acyclic_and_bounded() {
    let back = r#"<ref-clip ref="m0" name="Back edge" offset="20s" duration="1s" srcEnable="audio"/>"#;
    let xml = deferred_audio_document("", back, true);
    let (imp, report) = import(xml.as_bytes(), Format::Fcpxml, None).unwrap();
    assert!(report.mentions("nested too deeply"), "{report}");
    assert!(imp.project.items.len() <= 69, "reconciliation must reuse the existing resource identities");
    let (_, a) = deferred_audio_sequence(&imp.project, "A");
    assert!(a.video_tracks.iter().all(|t| t.items.is_empty()));
    assert_eq!(a.audio_tracks.iter().map(|t| t.items.len()).sum::<usize>(), 1);
    let mut active = std::collections::HashSet::new();
    let mut done = std::collections::HashSet::new();
    fn visit(p: &Project, id: ItemId, active: &mut std::collections::HashSet<ItemId>, done: &mut std::collections::HashSet<ItemId>) {
        if done.contains(&id) {
            return;
        }
        assert!(active.insert(id), "deferred audio introduced a cycle at {id:?}");
        if let Some(seq) = p.sequence(id) {
            assert!(seq.video_tracks.len() <= 256 && seq.audio_tracks.len() <= 256);
            for item in seq.all_tracks().flat_map(|t| &t.items) {
                visit(p, item.item, active, done);
            }
        }
        active.remove(&id);
        done.insert(id);
    }
    for id in imp.project.items.keys() {
        visit(&imp.project, *id, &mut active, &mut done);
    }
}

fn deferred_audio_lanes(seq: &Sequence) -> Vec<(String, Tick, Tick, usize)> {
    let mut items = seq
        .audio_tracks
        .iter()
        .enumerate()
        .flat_map(|(track, t)| t.items.iter().map(move |i| (i.name.clone(), i.start, i.duration, track)))
        .collect::<Vec<_>>();
    items.sort_by(|a, b| a.0.cmp(&b.0));
    items
}
fn assert_outside_audio_control(seq: &Sequence, report: &Report, expected: &[(String, Tick, Tick, usize)]) {
    assert_eq!(deferred_audio_lanes(seq).as_slice(), expected, "permanently outside preferences must not move valid neighbours");
    assert!(seq.audio_tracks.len() <= 256 && seq.video_tracks.len() <= 256);
    assert!(seq.all_tracks().flat_map(|t| &t.items).all(|i| !i.name.starts_with("Outside ")));
    assert!(seq.markers.iter().all(|m| !m.name.starts_with("Outside ")));
    assert!(seq.all_tracks().flat_map(|t| &t.items).flat_map(|i| &i.markers).all(|m| !m.name.starts_with("Outside ")));
    deferred_audio_pair(seq, "Adjusted");
    deferred_audio_pair(seq, "Positive lane");
    assert!(seq.audio_tracks[3].items[0].link.is_none(), "the ordinary audio neighbour must remain unpaired");
    let cap_failures = report
        .entries
        .iter()
        .filter(|e| e.message == "FCPXML Audio lane exceeds the 256-track import limit; clip was skipped")
        .map(|e| e.count)
        .sum::<u32>();
    assert_eq!(cap_failures, 2, "each outside reference contributes exactly one final diagnostic: {report}");
}

#[trial]
fn deferred_audio_preserves_encounter_order_with_ordinary_audio() {
    fn prewarm(xml: &str) -> String {
        let start = xml.find("<project name=\"B\">").unwrap();
        let end = start + xml[start..].find("</project>").unwrap() + "</project>".len();
        let project_b = &xml[start..end];
        let without_b = format!("{}{}", &xml[..start], &xml[end..]);
        without_b.replacen("<project name=\"A\">", &format!("{project_b}<project name=\"A\">"), 1)
    }
    let refs = r#"<ref-clip ref="m63" name="Adjusted" start="3s" duration="4s">
      <ref-clip ref="m63" name="Positive lane" lane="2" offset="3s" duration="2s"/>
      <asset-clip ref="av" name="Negative lane" lane="-3" offset="3s" duration="2s" srcEnable="audio"/>
    </ref-clip>"#;
    let xml = deferred_audio_document_at(61, refs, "", false);

    // With the same B project merely visited first, the earlier Positive lane gets A3.
    let (warm, warm_report) = import(prewarm(&xml).as_bytes(), Format::Fcpxml, None).unwrap();
    assert!(!warm_report.mentions("nested too deeply"), "{warm_report}");
    let (_, warm_seq) = deferred_audio_sequence(&warm.project, "M61");
    deferred_audio_pair(warm_seq, "Adjusted");
    deferred_audio_pair(warm_seq, "Positive lane");
    assert_eq!(warm_seq.audio_tracks[2].items[0].name, "Positive lane");
    assert_eq!(warm_seq.audio_tracks[3].items[0].name, "Negative lane");
    let expected = deferred_audio_lanes(warm_seq);
    assert_eq!(expected.len(), 3);

    // The audio asset remains available, but an intentionally video-only leaf can never
    // recover nested audio. Such deferred references must not reserve or displace A3.
    let leaf = r#"<asset-clip ref="av" name="Real audio" duration="20s"/>"#;
    assert_eq!(xml.matches(leaf).count(), 1);
    let silent_xml = xml.replacen(leaf, r#"<asset-clip ref="av" name="Real audio" duration="20s" srcEnable="video"/>"#, 1);
    let (silent, silent_report) = import(silent_xml.as_bytes(), Format::Fcpxml, None).unwrap();
    assert!(silent_report.mentions("nested too deeply"), "{silent_report}");
    let (_, silent_seq) = deferred_audio_sequence(&silent.project, "M61");
    let (_, silent_child) = deferred_audio_sequence(&silent.project, "M63");
    assert!(silent_child.audio_tracks.iter().all(|t| t.items.is_empty()));
    assert!(silent_seq.video_tracks[2].items.iter().any(|i| i.name == "Positive lane"));
    assert_eq!(deferred_audio_lanes(silent_seq), vec![("Negative lane".into(), Tick::ZERO, Tick(2 * TICKS_PER_SECOND), 2)]);
    let ordinary = &silent_seq.audio_tracks[2].items[0];
    assert!(ordinary.link.is_none());
    assert_eq!(media_key(&silent.project, ordinary.item), "/original-deferred-audio.mov");

    let (cold, cold_report) = import(xml.as_bytes(), Format::Fcpxml, None).unwrap();
    assert!(cold_report.mentions("nested too deeply"), "{cold_report}");
    let (_, cold_seq) = deferred_audio_sequence(&cold.project, "M61");
    let (_, child) = deferred_audio_sequence(&cold.project, "M63");
    assert_eq!(child.audio_tracks[0].items.len(), 1, "the later B visit must really recover child audio");
    deferred_audio_pair(cold_seq, "Adjusted");
    deferred_audio_pair(cold_seq, "Positive lane");
    assert_eq!(deferred_audio_lanes(cold_seq).len(), 3, "no missing or duplicate restoration may explain the lane difference");

    // Four entries suffice to reach the existing track cap. O2 is rejected during the
    // premature ordinary-only placement, but fits when all recoverable entries replay in order.
    let capped_refs = r#"<gap offset="0s" start="0s" duration="13s">
      <ref-clip ref="m63" name="D" lane="-255" offset="0s" duration="5s" srcEnable="audio"/>
      <asset-clip ref="av" name="O0" lane="-255" offset="0s" duration="10s" srcEnable="audio"/>
      <asset-clip ref="av" name="O1" lane="-255" offset="9s" duration="3s" srcEnable="audio"/>
      <asset-clip ref="av" name="O2" lane="-256" offset="11s" duration="2s" srcEnable="audio"/>
    </gap>"#;
    let capped_xml = deferred_audio_document_at(61, capped_refs, "", false);
    let (warm_cap, warm_cap_report) = import(prewarm(&capped_xml).as_bytes(), Format::Fcpxml, None).unwrap();
    assert!(!warm_cap_report.mentions("track import limit"), "{warm_cap_report}");
    let (_, warm_cap_seq) = deferred_audio_sequence(&warm_cap.project, "M61");
    let cap_expected = deferred_audio_lanes(warm_cap_seq);
    assert_eq!(
        cap_expected,
        vec![
            ("D".into(), Tick::ZERO, Tick(5 * TICKS_PER_SECOND), 254),
            ("O0".into(), Tick::ZERO, Tick(10 * TICKS_PER_SECOND), 255),
            ("O1".into(), Tick(9 * TICKS_PER_SECOND), Tick(3 * TICKS_PER_SECOND), 254),
            ("O2".into(), Tick(11 * TICKS_PER_SECOND), Tick(2 * TICKS_PER_SECOND), 255),
        ]
    );
    let (cold_cap, cold_cap_report) = import(capped_xml.as_bytes(), Format::Fcpxml, None).unwrap();
    assert!(cold_cap_report.mentions("nested too deeply"), "{cold_cap_report}");
    let (_, cold_cap_seq) = deferred_audio_sequence(&cold_cap.project, "M61");
    assert!(cold_cap_seq.audio_tracks.len() <= 256);
    let mut failures = Vec::new();
    if deferred_audio_lanes(cold_seq) != expected {
        failures.push(format!("ordinary collision: cold {:?}, prewarmed {expected:?}", deferred_audio_lanes(cold_seq)));
    }
    if deferred_audio_lanes(cold_cap_seq) != cap_expected {
        failures.push(format!("track-cap collision: cold {:?}, prewarmed {cap_expected:?}", deferred_audio_lanes(cold_cap_seq)));
    }
    if cold_cap_report.mentions("track import limit") {
        failures.push("a recoverable encounter-order placement reported a false track-cap skip".into());
    }
    assert!(failures.is_empty(), "deferred audio changed encounter-order lanes: {failures:?}");

    // Unlike the in-range O2 collision above, preferred index256 can never fit. Both an
    // ordinary asset and a deferred reference must leave only their final cap diagnostics.
    let outside = r#"
      <asset-clip ref="av" name="Outside ordinary" lane="-257" offset="3s" duration="2s" srcEnable="audio">
        <adjust-volume amount="-9dB"/><marker start="1s" value="Outside ordinary marker"/>
      </asset-clip>
      <ref-clip ref="m63" name="Outside deferred" lane="-257" offset="3s" duration="2s" srcEnable="audio">
        <adjust-volume amount="-12dB"/><marker start="1s" value="Outside deferred marker"/>
      </ref-clip>
    </ref-clip>"#;
    assert_eq!(refs.matches("</ref-clip>").count(), 1);
    let outside_refs = refs.replacen("</ref-clip>", outside, 1);
    let outside_xml = deferred_audio_document_at(61, &outside_refs, "", false);
    let prewarmed_outside = prewarm(&outside_xml);
    for text in [&outside_xml, &prewarmed_outside] {
        let (imported, report) = import(text.as_bytes(), Format::Fcpxml, None).unwrap();
        let (_, seq) = deferred_audio_sequence(&imported.project, "M61");
        assert_outside_audio_control(seq, &report, &expected);
    }
}
