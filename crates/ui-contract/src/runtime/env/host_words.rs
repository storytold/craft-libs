//! The words, number format and dates of the installed Host.

use crate::runtime::egui::memory::Service;
use craft_host::Host;
use craft_i18n::{DateFormat, Format};
use craft_i18n::{WordRows, Words};
use egui::Context;
use std::sync::Arc;

/// The words of the installed `Host`, kept until another host is installed (`Host::generation`; `None` without a
/// host) so that the word sets built from them are built once.
#[derive(Default)]
struct HostWords {
    words: Option<(Option<u64>, Arc<Words>)>,
}

/// The catalogs of `host` in its language, English without a host or a catalog.
pub(super) fn host_words(ctx: &Context, host: Option<&Arc<Host>>) -> Arc<Words> {
    let generation = host.map(|host| host.generation());
    Service::<HostWords>::run(ctx, |cached| {
        if let Some((kept, words)) = &cached.words
            && *kept == generation
        {
            return Arc::clone(words);
        }
        let words = Arc::new(host.map_or_else(Words::english, |host| host.words()));
        cached.words = Some((generation, Arc::clone(&words)));
        words
    })
}

/// The number format of `host`'s catalogs, else English, built once. The library's words are English
/// without a host (`host_words`), so its numbers are English too: a library that wrote its numbers in
/// the machine's region while its words stayed English would be half in one language and half in
/// another. An app that installs a host with catalogs writes both from the catalogs.
pub(super) fn host_format(ctx: &Context, host: Option<&Arc<Host>>) -> Arc<Format> {
    if let Some(i18n) = host.and_then(|host| host.i18n_shared()) {
        return i18n.format();
    }
    #[derive(Default)]
    struct Default(Option<Arc<Format>>);
    Service::<Default>::of(ctx).with(|default| Arc::clone(default.0.get_or_insert_with(|| Arc::new(Format::from_tag("en-US")))))
}

/// The date format of `host`'s catalogs, else English, built once: the same reason as the numbers
/// (`host_format`). A calendar that names its weekdays in the machine's language while every other
/// word of the library stays English is the one the trials cannot read.
pub(super) fn host_dates(ctx: &Context, host: Option<&Arc<Host>>) -> Arc<DateFormat> {
    if let Some(i18n) = host.and_then(|host| host.i18n_shared()) {
        return i18n.dates();
    }
    #[derive(Default)]
    struct Default(Option<Arc<DateFormat>>);
    Service::<Default>::of(ctx).with(|default| Arc::clone(default.0.get_or_insert_with(|| Arc::new(DateFormat::from_tag("en-US")))))
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
