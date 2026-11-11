//! The words, number format and dates of the installed Host.

use crate::runtime::egui::memory::Service;
use craft_host::Host;
use craft_i18n::{DateFormat, Format};
use craft_i18n::{WordRows, Words};
use egui::Context;
use std::sync::Arc;

/// The words of the installed `Host`, kept until the `Host` changes so that the word sets built from them are
/// built once.
#[derive(Default)]
struct HostWords {
    host: Option<std::sync::Weak<Host>>,
    words: Option<Arc<Words>>,
}

/// The catalogs of `host` in its language, English without a host or a catalog.
pub(super) fn host_words(ctx: &Context, host: Option<&Arc<Host>>) -> Arc<Words> {
    Service::<HostWords>::run(ctx, |cached| {
        let same = match (&cached.host, host) {
            (Some(kept), Some(host)) => std::sync::Weak::ptr_eq(kept, &Arc::downgrade(host)),
            (None, None) => true,
            _ => false,
        };
        if let (true, Some(words)) = (same, &cached.words) {
            return Arc::clone(words);
        }
        let words = Arc::new(match host.and_then(|host| host.i18n_shared().map(|i18n| (Arc::clone(i18n), host.lang()))) {
            Some((i18n, lang)) => Words::from_i18n(i18n, lang),
            None => Words::english(),
        });
        cached.host = host.map(Arc::downgrade);
        cached.words = Some(Arc::clone(&words));
        words
    })
}

/// The number format of `host`'s catalogs, else the system region's with the locale's digits, built once.
pub(super) fn host_format(ctx: &Context, host: Option<&Arc<Host>>) -> Arc<Format> {
    if let Some(i18n) = host.and_then(|host| host.i18n_shared()) {
        return i18n.format();
    }
    #[derive(Default)]
    struct System(Option<Arc<Format>>);
    Service::<System>::of(ctx)
        .with(|system| Arc::clone(system.0.get_or_insert_with(|| Arc::new(Format::new(&craft_i18n::Region::system(), craft_i18n::Digits::Locale)))))
}

/// The date format of `host`'s catalogs, else the system region's, built once.
pub(super) fn host_dates(ctx: &Context, host: Option<&Arc<Host>>) -> Arc<DateFormat> {
    if let Some(i18n) = host.and_then(|host| host.i18n_shared()) {
        return i18n.dates();
    }
    #[derive(Default)]
    struct System(Option<Arc<DateFormat>>);
    Service::<System>::of(ctx).with(|system| Arc::clone(system.0.get_or_insert_with(|| Arc::new(DateFormat::new(&craft_i18n::Region::system())))))
}

/// `words` with `rows` over them, built once per (inherited words, rows): a clone of a `Words` starts with an
/// empty set cache, so a fresh overlay each pass would rebuild every word set each frame.
pub(super) fn overlaid_words(ctx: &Context, words: &Arc<Words>, rows: WordRows) -> Arc<Words> {
    /// Overlays kept before the oldest goes.
    const KEPT: usize = 16;
    struct Entry {
        inherited: std::sync::Weak<Words>,
        rows: WordRows,
        words: Arc<Words>,
    }
    #[derive(Default)]
    struct Overlays(Vec<Entry>);
    Service::<Overlays>::run(ctx, |cache| {
        cache.0.retain(|entry| entry.inherited.strong_count() > 0);
        let inherited = Arc::downgrade(words);
        if let Some(entry) = cache.0.iter().find(|entry| std::sync::Weak::ptr_eq(&entry.inherited, &inherited) && entry.rows == rows) {
            return Arc::clone(&entry.words);
        }
        let overlaid = Arc::new(words.overlay(&rows));
        if cache.0.len() == KEPT {
            cache.0.remove(0);
        }
        cache.0.push(Entry { inherited, rows, words: Arc::clone(&overlaid) });
        overlaid
    })
}
