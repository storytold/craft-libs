//! `BreadcrumbBar`: how a path splits into crumbs, which crumbs a room shows, the nodes it reports and how a long
//! ancestor elides.

use craft_ui::bars::path_crumbs;
use craft_ui::testing::trial;

fn pairs(path: &str) -> Vec<(String, String)> {
    path_crumbs(path).iter().map(|crumb| (crumb.label().as_str().to_owned(), crumb.value().cloned().unwrap_or_default())).collect()
}

fn expected(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter().map(|(label, key)| ((*label).to_owned(), (*key).to_owned())).collect()
}

/// Each crumb's value is the path up to it in the path's own spelling; empty parts are skipped.
#[trial]
fn path_crumbs_keep_the_spelling_of_the_path() {
    assert_eq!(pairs("/a/b"), expected(&[("a", "/a"), ("b", "/a/b")]));
    assert_eq!(pairs("a//b/"), expected(&[("a", "a"), ("b", "a//b")]), "empty parts are skipped, the spelling kept");
    assert_eq!(pairs("C:\\x"), expected(&[("C:", "C:\\"), ("x", "C:\\x")]), "a bare drive gets its separator back");
    assert_eq!(pairs("C:/x"), expected(&[("C:", "C:/"), ("x", "C:/x")]));
    assert_eq!(pairs("\\\\server\\share\\x"), expected(&[("server", "\\\\server"), ("share", "\\\\server\\share"), ("x", "\\\\server\\share\\x")]));
    assert!(pairs("").is_empty() && pairs("/").is_empty());
}
