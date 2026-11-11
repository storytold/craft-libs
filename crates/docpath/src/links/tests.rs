//! Link status, the fingerprint, folder matches and the search, over a fake `LinkFiles` and
//! (desktop) a temporary folder.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::*;

fn stamp(secs: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
}

fn link(stored: &str, identity: Option<LinkIdentity>) -> Link<u32> {
    Link { key: 1, name: "a".into(), stored: stored.into(), identity, page: None, embedded: false }
}

fn identity_for(bytes: &[u8], secs: u64) -> LinkIdentity {
    identity_of(&mut Cursor::new(bytes), bytes.len() as u64, Some(stamp(secs))).unwrap()
}

// Same stamp: Ok without `identity`; new mtime, same fingerprint: Ok; new bytes: Modified; read error: Unreadable.
#[test]
fn a_touched_file_is_ok_and_a_changed_file_is_modified() {
    let recorded = identity_for(b"hello world", 10);
    let l = link("/m/a.png", Some(recorded));
    let at = |bytes: &[u8], secs: u64| FakeLinkFiles::new().with("/m/a.png", bytes, secs);

    let same = at(b"hello world", 10);
    assert_eq!(status(&l, None, &same), LinkStatus::Ok);
    assert_eq!(same.calls(), 1, "the stamp matched, so nothing was hashed");
    assert_eq!(status(&l, None, &at(b"hello world", 99)), LinkStatus::Ok);
    assert_eq!(status(&l, None, &at(b"hello w0rld", 99)), LinkStatus::Modified);
    assert_eq!(status(&l, None, &at(b"hello", 99)), LinkStatus::Modified);
    let locked = at(b"hello world", 99).unreadable("/m/a.png");
    assert_eq!(status(&l, None, &locked), LinkStatus::Unreadable);
    assert_eq!(status(&l, None, &FakeLinkFiles::new()), LinkStatus::Missing);
    assert_eq!(status(&link("/m/a.png", None), None, &at(b"x", 1)), LinkStatus::Ok);
    let embedded = Link { embedded: true, ..l };
    assert_eq!(status(&embedded, None, &FakeLinkFiles::new()), LinkStatus::Embedded);
}

// `fingerprint` of a 3 MiB file equals FilmCraft's for the same bytes; a 2 MiB file is hashed whole.
#[test]
fn the_fingerprint_reads_the_head_and_the_tail() {
    let span = FINGERPRINT_SPAN;
    let big: Vec<u8> = (0..3 * span).map(|i| (i * 7 % 251) as u8).collect();
    let id = identity_of(&mut Cursor::new(&big), big.len() as u64, None).unwrap();
    assert_eq!(id.fingerprint, fingerprint(big.len() as u64, &big[..span], &big[big.len() - span..]));

    // A byte in the middle of a file over twice the span is outside what is hashed.
    let mut changed = big.clone();
    changed[span + 10] ^= 0xff;
    assert_eq!(identity_of(&mut Cursor::new(&changed), changed.len() as u64, None).unwrap().fingerprint, id.fingerprint);

    // Up to twice the span, the whole file is hashed: head plus the rest.
    let whole = &big[..2 * span];
    let id = identity_of(&mut Cursor::new(whole), whole.len() as u64, None).unwrap();
    assert_eq!(id.fingerprint, fingerprint(whole.len() as u64, &whole[..span], &whole[span..]));
    let mut changed = whole.to_vec();
    changed[span + 10] ^= 0xff;
    assert_ne!(identity_of(&mut Cursor::new(&changed), changed.len() as u64, None).unwrap().fingerprint, id.fingerprint);

    // A file shorter than it claims is an error.
    assert!(identity_of(&mut Cursor::new(&big[..100]), 200, None).is_err());
}

// `folder_match` takes only a plain name: `file:%2Fx.png`, `C:x.png`, `NUL`, `..` give None; an extension swap works.
#[test]
fn relink_to_folder_matches_only_the_plain_name_in_that_folder() {
    let folder = Path::new("/new");
    let m = |stored: &str, ext: Option<&str>| folder_match(&link(stored, None), folder, ext);
    assert_eq!(m(r"C:\old\a.png", None), Some(PathBuf::from("/new/a.png")));
    assert_eq!(m("/old/a.png", Some("jpg")), Some(PathBuf::from("/new/a.jpg")));
    for bad in ["file:%2Ftmp%2Fx.png", "C:x.png", "NUL", "..", "/old/.", ""] {
        assert_eq!(m(bad, None), None, "{bad}");
    }
    assert_eq!(m("/old/a.png", Some("j/pg")), None);
    assert_eq!(m("/old/a.png", Some(r"j\pg")), None);

    let fake = FakeLinkFiles::new().with("/new/a.png", b"x", 1);
    let links = [Link { key: 1, ..link("/old/a.png", None) }, Link { key: 2, ..link("/old/b.png", None) }, Link { key: 3, ..link("NUL", None) }];
    let (found, missing) = folder_matches(&links, folder, None, &fake);
    assert_eq!(found, vec![(1, PathBuf::from("/new/a.png"))]);
    assert_eq!(missing, vec![2, 3]);
}

// A folder link loop is not followed; dot files are skipped; the limit sets `truncated`.
#[cfg(unix)]
#[test]
fn the_search_never_follows_directory_links_and_stops_at_the_limit() {
    let root = std::env::temp_dir().join(format!("craft-docpath-index-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("A.png"), b"1").unwrap();
    std::fs::write(root.join(".hidden.png"), b"1").unwrap();
    std::fs::write(root.join("sub/a.PNG"), b"2").unwrap();
    std::os::unix::fs::symlink(&root, root.join("sub/loop")).unwrap();
    std::os::unix::fs::symlink(root.join("A.png"), root.join("sub/link.png")).unwrap();

    let index = index_folder(&root, 100, &SystemLinkFiles, &|| false);
    assert!(!index.truncated);
    assert_eq!(index.files, 3);
    assert_eq!(index.by_name["a.png"], vec![root.join("A.png"), root.join("sub/a.PNG")]);
    assert_eq!(index.by_name["link.png"], vec![root.join("sub/link.png")]);
    assert!(!index.by_name.contains_key(".hidden.png"));

    let cut = index_folder(&root, 2, &SystemLinkFiles, &|| false);
    assert!(cut.truncated);
    assert_eq!(cut.files, 2);

    let id = SystemLinkFiles.identity(&root.join("A.png")).unwrap();
    assert_eq!(id.size, 1);
    assert!(SystemLinkFiles.identity(&root.join("sub")).is_err());
    assert!(SystemLinkFiles.stat(&root.join("sub")).is_none());
    let _ = std::fs::remove_dir_all(&root);
}
