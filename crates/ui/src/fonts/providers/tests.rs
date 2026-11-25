use std::sync::mpsc;
use std::time::Duration;

use egui::{FontDefinitions, FontId};

use super::*;

const WAIT: Duration = Duration::from_secs(10);
const ARABIC: &[u8] = include_bytes!("../../../tests/fonts/NotoSansArabic-test.ttf");

fn provider(name: &'static str) -> Arc<dyn FontProvider> {
    Arc::new(move |_: &FallbackRequest<'_>| Some(FontInsert::new(name, FontData::from_static(ARABIC), vec![])))
}

fn selected(ctx: &egui::Context) -> Option<String> {
    ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
    ctx.fonts_mut(|fonts| {
        fonts.has_glyph(&FontId::proportional(14.0), 'ع');
        fonts.discovered_fonts().last().map(|font| font.name.clone())
    })
}

#[test]
fn completion_cannot_overwrite_opt_out_or_a_new_locale() {
    overlapping_updates(false);
}

#[test]
fn installation_cannot_overwrite_opt_out_or_a_new_locale() {
    overlapping_updates(true);
}

fn overlapping_updates(install: bool) {
    for replacement in [None, Some("new locale")] {
        let ctx = egui::Context::default();
        ctx.set_fonts(FontDefinitions::empty());
        set_font_providers(&ctx, vec![provider("old locale")]);
        assert_eq!(selected(&ctx).as_deref(), Some("old locale"));
        let state: ContextProviders = ctx.data(|d| d.get_temp(providers_id()).unwrap());
        let (snapshot, snapshot_ready) = mpsc::channel();
        let (resume, resumed) = mpsc::channel();
        let (reloaded, reload_done) = mpsc::channel();
        let reload_ctx = ctx.clone();
        let reload = std::thread::spawn(move || {
            let pause = || {
                snapshot.send(()).unwrap();
                resumed.recv_timeout(WAIT).unwrap();
            };
            if install {
                set_with(&reload_ctx, vec![provider("intermediate locale")], pause);
            } else {
                reload_with(&reload_ctx, pause);
            }
            reloaded.send(()).unwrap();
        });
        snapshot_ready.recv_timeout(WAIT).unwrap();

        // Exercise the exact stale-snapshot interleaving without sleeps. If a reload has
        // released its lock before applying, let the replacement finish first. A correctly
        // serialized replacement instead waits until we release the paused reload.
        let snapshot_unprotected = state.0.try_lock().is_ok();
        let (replaced, replacement_done) = mpsc::channel();
        let replace_ctx = ctx.clone();
        let replace = std::thread::spawn(move || {
            set_font_providers(&replace_ctx, replacement.map(provider).into_iter().collect());
            replaced.send(()).unwrap();
        });
        if snapshot_unprotected {
            replacement_done.recv_timeout(WAIT).unwrap();
        }
        resume.send(()).unwrap();
        reload_done.recv_timeout(WAIT).unwrap();
        if !snapshot_unprotected {
            replacement_done.recv_timeout(WAIT).unwrap();
        }
        reload.join().unwrap();
        replace.join().unwrap();
        assert_eq!(selected(&ctx).as_deref(), replacement, "completion restored a stale provider list");
        reload_font_providers(&ctx);
        assert_eq!(selected(&ctx).as_deref(), replacement, "later reloads also use the latest list");
    }
}
