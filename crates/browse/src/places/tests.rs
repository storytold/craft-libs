//! `Places::resolve`, folder equality and `FolderCache` over a fake `Directories` and `ManualJobs`.

use craft_ui::testing::trial;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use craft_host::{ManualJobs, Wake};

use super::*;

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

fn standard(pairs: &[(StandardFolder, &str)]) -> Vec<(StandardFolder, PathBuf)> {
    pairs.iter().map(|(k, s)| (*k, p(s))).collect()
}

fn resolve(standard: &[(StandardFolder, PathBuf)], settings: &PlacesSettings, browsing: Option<&str>, unlisted: &[PathBuf]) -> Places {
    let browsing = browsing.map(PathBuf::from);
    Places::resolve(&PlacesInput { standard, volumes: &[], settings, browsing: browsing.as_deref(), unlisted, groups: &[PlaceGroupKind::Places] })
}

fn names(places: &Places) -> Vec<&str> {
    places.groups.iter().flat_map(|g| &g.places).map(|p| p.name.as_str()).collect()
}

// Keep, remove (a favourite drops, a standard root hides), restore and restore all; recents keep 10, newest first.

// LightCraft's `browsing_below_an_unlisted_folder_has_a_session_root` and `session_root_stays_while_browsing_below_it`.
#[trial]
fn a_browsed_folder_outside_every_root_gets_a_session_root() {
    let home = "/home/example";
    let hidden = p("/home/example/.hidden");
    let current = "/home/example/.hidden/Photos";
    let std = standard(&[(StandardFolder::Home, home)]);
    let unlisted = vec![hidden.clone()];
    let mut settings = PlacesSettings::default();

    let first = resolve(&std, &settings, Some(current), &unlisted);
    assert_eq!(first.session_root, Some(p(current)));
    assert_eq!(first.owner, Some((0, 1)));
    assert_eq!(first.get((0, 1)).map(|p| p.kind), Some(PlaceKind::Session));

    settings.session_root = first.session_root.clone();
    let child = format!("{current}/Trip");
    let next = resolve(&std, &settings, Some(&child), &unlisted);
    assert_eq!(next.session_root, first.session_root, "the root stays while browsing below it");
    assert_eq!(next.owner, Some((0, 1)));

    // A kept folder holds it: no session root.
    let kept = PlacesSettings { favorites: vec![p(current)], ..PlacesSettings::default() };
    let r = resolve(&std, &kept, Some(&child), &unlisted);
    assert_eq!((r.session_root, r.owner), (None, Some((0, 1))));

    // A new hidden folder deeper down starts a new root.
    let deeper = p("/home/example/.hidden/Photos/.private");
    let inside = format!("{}/Trip", deeper.display());
    let moved = resolve(&std, &settings, Some(&inside), &[hidden, deeper]);
    assert_eq!(moved.session_root, Some(p(&inside)));

    // Outside everything, and plain: the folder itself.
    let outside = resolve(&std, &PlacesSettings::default(), Some("/t/base"), &[]);
    assert_eq!(outside.session_root, Some(p("/t/base")));
    // A hidden root gets no row at all.
    let hidden_root = PlacesSettings { hidden: vec![p("/t/base/")], ..PlacesSettings::default() };
    let r = resolve(&[], &hidden_root, Some("/t/base"), &[]);
    assert!(r.groups[0].places.is_empty() && r.session_root.is_none());
}

// Home and a kept folder inside it both hold the browsed folder: the kept one owns the reveal.

// LightCraft's `hidden_folder_is_not_revealed_in_an_outer_tree`.
#[trial]
fn a_hidden_folder_never_opens_a_tree_through_itself() {
    let std = standard(&[(StandardFolder::Pictures, "/home/example/Pictures"), (StandardFolder::Home, "/home/example")]);
    let kept = p("/home/example/AppData/Temp/lc");
    let settings = PlacesSettings { favorites: vec![kept.clone()], ..PlacesSettings::default() };
    let r = resolve(&std, &settings, Some("/home/example/AppData/Temp/lc"), &[]);
    assert_eq!(r.owner, Some((0, 2)), "shown as its own kept row");

    let hidden = PlacesSettings { hidden: vec![p("/home/example/AppData/Temp/lc/")], ..settings };
    for browsing in ["/home/example/AppData/Temp/lc", "/home/example/AppData/Temp/lc/Day 1"] {
        let r = resolve(&std, &hidden, Some(browsing), &[]);
        assert_eq!(names(&r), ["Pictures", "example"], "{browsing}");
        assert_eq!((r.owner, r.session_root), (None, None), "Home does not open down to {browsing}");
        assert_eq!(r.hidden.len(), 1);
    }
    // Hiding Home itself still lets Pictures open down to a folder inside it.
    let home_hidden = PlacesSettings { hidden: vec![p("/home/example")], ..PlacesSettings::default() };
    let r = resolve(&std, &home_hidden, Some("/home/example/Pictures/Trip"), &[]);
    assert_eq!((names(&r), r.owner), (vec!["Pictures"], Some((0, 0))));
    assert_eq!(r.hidden[0].kind, PlaceKind::Standard(StandardFolder::Home), "a hidden standard root keeps its kind");
}

// LightCraft's `equivalent_paths_are_one_local_location`: `/a/b/`, `/a/./b` are one root.
#[trial]
fn equal_folders_are_one_root_however_spelled() {
    assert!(same_folder(Path::new("/a/b/"), Path::new("/a/./b")));
    assert!(same_folder(Path::new("/a/x/../b"), Path::new("/a//b")));
    assert!(same_folder(Path::new("D:\\Example\\Pictures"), Path::new("d:/Example/Pictures/")));
    assert!(!same_folder(Path::new("/a/b"), Path::new("/a/bc")));
    assert!(folder_within(Path::new("/a/b/c"), Path::new("/a/b/")));
    assert!(!folder_within(Path::new("/a/bc"), Path::new("/a/b")));

    for (home, sep) in [("D:\\Example", '\\'), ("D:/Example", '/'), ("/home/example", '/')] {
        let std: Vec<(StandardFolder, PathBuf)> = [(StandardFolder::Pictures, "Pictures"), (StandardFolder::Desktop, "Desktop")]
            .iter()
            .map(|(k, s)| (*k, p(&format!("{home}{sep}{s}"))))
            .collect();
        let pictures = std[0].1.to_string_lossy().into_owned();
        let flipped = pictures.replace(['/', '\\'], if sep == '/' { "\\" } else { "/" });
        for browsing in [flipped.clone(), format!("{pictures}{sep}"), pictures.replacen("D:", "d:", 1)] {
            let settings = PlacesSettings { favorites: vec![p(&browsing)], ..PlacesSettings::default() };
            let r = resolve(&std, &settings, Some(&browsing), &[]);
            assert_eq!(names(&r), ["Pictures", "Desktop"], "{pictures} browsed as {browsing}");
            assert_eq!(r.get(r.owner.unwrap()).unwrap().path, p(&pictures), "the standard row is the one opened");
        }
        let hidden = PlacesSettings { hidden: vec![p(&flipped)], ..PlacesSettings::default() };
        assert_eq!(names(&resolve(&std, &hidden, None, &[])), ["Desktop"], "hiding one spelling hides the other");
    }
    // A different folder is still added.
    let std = standard(&[(StandardFolder::Pictures, "/home/example/Pictures")]);
    let r = resolve(&std, &PlacesSettings::default(), Some("/home/example/Pictures2"), &[]);
    assert_eq!(names(&r), ["Pictures", "Pictures2"]);
}

fn cache() -> (FolderCache, Arc<FakeDirectories>, Arc<ManualJobs>) {
    let fake = Arc::new(FakeDirectories::new().standard_folder(StandardFolder::Home, "/home/u"));
    let jobs = Arc::new(ManualJobs::new());
    (FolderCache::new(fake.clone(), jobs.clone(), Wake::new(|| {})), fake, jobs)
}

fn names_of(listing: Listing<'_>) -> Vec<String> {
    match listing {
        Listing::Ready(folders) => folders.iter().map(|f| f.name.clone()).collect(),
        other => panic!("not ready: {other:?}"),
    }
}

// A listing that started before a refresh shows, but stays stale and is listed again; before the fix its landing cleared the refresh.
#[trial]
fn a_listing_started_before_a_refresh_is_listed_again() {
    let (mut cache, fake, jobs) = cache();
    fake.add_folder("/x/a");
    let dir = Path::new("/x");
    assert_eq!(cache.subfolders(dir), Listing::Loading);
    jobs.run_all();
    // The folder changes and is refreshed while the finished listing is not yet taken in.
    fake.add_folder("/x/b");
    cache.refresh_folder(dir);
    assert_eq!(names_of(cache.subfolders(dir)), ["a"], "the old listing shows");
    jobs.run_all();
    assert_eq!(names_of(cache.subfolders(dir)), ["a", "b"]);
    assert_eq!(fake.calls(), 2);

    // The same for the roots and `refresh`.
    assert!(cache.roots().is_none());
    jobs.run_all();
    cache.refresh();
    assert!(cache.roots().is_some());
    jobs.run_all();
    assert!(cache.roots().is_some());
    assert_eq!(jobs.waiting(), 0, "the refreshed roots landed fresh");
}

fn entries_of(entries: Entries<'_>) -> (Vec<(String, bool)>, bool) {
    match entries {
        Entries::Ready { entries, truncated } => (entries.iter().map(|e| (e.name.clone(), e.folder)).collect(), truncated),
        other => panic!("not ready: {other:?}"),
    }
}

// The listing half of WordCraft's `listing_filters_sorts_and_caps`: files and folders in one natural order, hidden ones out, a link to a folder is a folder, a limit cuts off and says so.
#[trial]
fn a_folder_listing_with_files_skips_hidden_entries_and_counts_links_to_folders() {
    let fake = Arc::new(
        FakeDirectories::new()
            .folder("/d/zeta")
            .folder("/d/.git")
            .folder("/target")
            .file("/d/file10.txt")
            .file("/d/file2.txt")
            .file("/d/.hidden.txt")
            .file("/d/Alpha.txt")
            .link("/d/shortcut", "/target")
            .link("/d/doc-link", "/d/file2.txt"),
    );
    let jobs = Arc::new(ManualJobs::new());
    let mut cache = FolderCache::new(fake.clone(), jobs.clone(), Wake::new(|| {}));
    let dir = Path::new("/d");

    assert_eq!(cache.entries(dir, 100), Entries::Loading, "nothing is read in the pass");
    jobs.run_all();
    let (listed, truncated) = entries_of(cache.entries(dir, 100));
    let expected = [("Alpha.txt", false), ("doc-link", false), ("file2.txt", false), ("file10.txt", false), ("shortcut", true), ("zeta", true)];
    assert_eq!(listed, expected.map(|(n, f)| (n.to_string(), f)));
    assert!(!truncated, "eight entries read, none left over");

    // Exactly the limit read: the folder ended, so nothing was cut off.
    assert_eq!(cache.entries(dir, 8), Entries::Loading);
    jobs.run_all();
    assert!(!entries_of(cache.entries(dir, 8)).1);

    // Hidden entries count as read: three read (`.git`, `zeta`, `.hidden.txt`), one shown.
    assert_eq!(cache.entries(dir, 3), Entries::Loading, "another limit is another listing");
    jobs.run_all();
    let (listed, truncated) = entries_of(cache.entries(dir, 3));
    assert_eq!(listed, [("zeta".to_string(), true)]);
    assert!(truncated);

    // A folder of files only lists them; an unreadable folder reports why.
    let fake = Arc::new(FakeDirectories::new().file("/e/a.txt"));
    let mut cache = FolderCache::new(fake, jobs.clone(), Wake::new(|| {}));
    let failing = Arc::new(FakeDirectories::new().failing("/e"));
    let mut broken = FolderCache::new(failing, jobs.clone(), Wake::new(|| {}));
    assert_eq!(cache.entries(Path::new("/e"), 10), Entries::Loading);
    assert_eq!(broken.entries(Path::new("/e"), 10), Entries::Loading);
    jobs.run_all();
    assert_eq!(entries_of(cache.entries(Path::new("/e"), 10)).0, [("a.txt".to_string(), false)]);
    assert!(matches!(broken.entries(Path::new("/e"), 10), Entries::Failed(_)), "an unreadable folder fails, never panics");
}
