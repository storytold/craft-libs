#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

const ROADMAP: &str = "# App roadmap\n\n> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-11-01 · **Change:** trivial (old thing); previously minor (older) · **Target:** Word 16\n\nIntro.\n\n## Progress log\n\n- 2026-11-01: old entry\n  wrapped.\n- 2026-10-30: older entry\n\n## Next\n\nStuff.\n";

const GAPS: &str = "# Gaps\n\n## Every known gap, ranked\n\nProse above.\n\n| # | Gap | Evidence |\n|---|---|---|\n| 1 | First | e1 |\n| 3 | Third | e3 |\n| 2 | Second | e2 |\n\nProse below.\n\n## Revision history\n\n| Date | Change | Summary |\n|---|---|---|\n| 2026-11-01 | minor | x |\n";

const LOGS: &[Log] = &[Log { dir: "docs/progress.d", doc: "ROADMAP.md", heading: "## Progress log", newest_first: true, status: true }];
const TABLES: &[Table] = &[Table {
    dir: "docs/gaps.d",
    doc: "docs/gaps.md",
    heading: "## Every known gap, ranked",
    columns: &[("#", "number"), ("Gap", "gap"), ("Evidence", "body")],
    show_closed: true,
}];
const CFG: Config = Config { logs: LOGS, tables: TABLES };

fn f(path: &str, text: &str, time: i64) -> File {
    File { path: path.into(), text: text.into(), time }
}

// 2026-11-05 12:00 UTC and 2026-11-06 12:00 UTC.
const NOV5: i64 = 1_793_880_000;
const NOV6: i64 = 1_793_966_400;

fn docs() -> Vec<File> {
    let (items, _) = items_from_doc(&CFG, &[f("docs/gaps.md", GAPS, 0)]).unwrap();
    let mut out = vec![f("ROADMAP.md", ROADMAP, 0), f("docs/gaps.md", GAPS, 0), f("docs/progress.d/README.md", "how", 0)];
    out.extend(items);
    out
}

fn joined(files: &[File], doc: &str) -> String {
    join(&CFG, files).unwrap().docs.get(doc).unwrap().clone()
}

#[test]
fn no_fragments_leaves_the_log_alone() {
    assert_eq!(joined(&docs(), "ROADMAP.md"), ROADMAP);
}

#[test]
fn fragments_go_on_top_newest_first_and_are_consumed() {
    let mut files = docs();
    files.push(f("docs/progress.d/12-a.md", "- 2026-11-05: a\n", NOV5));
    files.push(f("docs/progress.d/13-b.md", "---\nkind: major\n---\n- 2026-11-06: b\n  two lines\n", NOV6));
    let j = join(&CFG, &files).unwrap();
    let text = j.docs.get("ROADMAP.md").unwrap();
    assert!(text.contains("## Progress log\n\n- 2026-11-06: b\n  two lines\n- 2026-11-05: a\n- 2026-11-01: old entry\n"), "{text}");
    assert_eq!(j.consumed, ["docs/progress.d/12-a.md", "docs/progress.d/13-b.md"]);
}

#[test]
fn status_line_gets_date_and_highest_kind_and_drops_the_chain() {
    let mut files = docs();
    files.push(f("docs/progress.d/12-a.md", "- 2026-11-05: a\n", NOV5));
    files.push(f("docs/progress.d/13-b.md", "---\nkind: trivial\n---\n- b\n", NOV6));
    let text = joined(&files, "ROADMAP.md");
    assert!(text.contains("> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-11-06 · **Change:** minor · **Target:** Word 16\n"), "{text}");
}

#[test]
fn table_log_rows_go_under_the_separator_and_must_fit() {
    const L: &[Log] = &[Log { dir: "d", doc: "R.md", heading: "## Progress log", newest_first: true, status: false }];
    let cfg = Config { logs: L, tables: &[] };
    let doc = "## Progress log\n\n| Date | Entry |\n|---|---|\n| 2026-11-01 | old |\n";
    let ok = join(&cfg, &[f("R.md", doc, 0), f("d/1-x.md", "| 2026-11-06 | new |", NOV6)]).unwrap();
    assert_eq!(ok.docs.get("R.md").unwrap(), "## Progress log\n\n| Date | Entry |\n|---|---|\n| 2026-11-06 | new |\n| 2026-11-01 | old |\n");
    let bad = join(&cfg, &[f("R.md", doc, 0), f("d/1-x.md", "| only one |", NOV6)]).unwrap_err();
    assert!(bad[0].contains("isn't a row of the log's 2 columns"), "{bad:?}");
}

#[test]
fn oldest_first_log_appends() {
    const L: &[Log] = &[Log { dir: "d", doc: "R.md", heading: "## Change log", newest_first: false, status: false }];
    let cfg = Config { logs: L, tables: &[] };
    let doc = "## Change log\n\n| Date | Change |\n|---|---|\n| 2026-11-01 | old |\n\n## Next\n";
    let j = join(&cfg, &[f("R.md", doc, 0), f("d/1-x.md", "| 2026-11-06 | new |", NOV6)]).unwrap();
    assert_eq!(j.docs.get("R.md").unwrap(), "## Change log\n\n| Date | Change |\n|---|---|\n| 2026-11-01 | old |\n| 2026-11-06 | new |\n\n## Next\n");
}

#[test]
fn cutover_round_trips_the_table() {
    // Items made from the doc's rows rebuild the same table, in the same order (rank = row place).
    assert_eq!(joined(&docs(), "docs/gaps.md"), GAPS);
}

#[test]
fn cutover_splits_a_repeated_number() {
    let doc = GAPS.replace("| 2 | Second |", "| 3 | Second |");
    let (items, notes) = items_from_doc(&CFG, &[f("docs/gaps.md", &doc, 0)]).unwrap();
    let names: Vec<&str> = items.iter().map(|i| i.path.as_str()).collect();
    assert_eq!(names, ["docs/gaps.d/1-first.md", "docs/gaps.d/3-third.md", "docs/gaps.d/4-second.md"]);
    assert!(notes[0].contains("repeated; it becomes 4"), "{notes:?}");
}

#[test]
fn rank_then_number_and_unranked_last() {
    let mut files = vec![f("docs/gaps.md", GAPS, 0)];
    files.push(f("docs/gaps.d/1-first.md", "---\nrank: 2\ngap: First\n---\ne1", 0));
    files.push(f("docs/gaps.d/2-second.md", "---\nrank: 1\ngap: Second\n---\ne2", 0));
    files.push(f("docs/gaps.d/5-new.md", "---\ngap: New\n---\nline one\nline | two", 0));
    files.push(f("docs/gaps.d/4-tie.md", "---\nrank: 1\ngap: Tie\n---\ne4", 0));
    files.push(f("ROADMAP.md", ROADMAP, 0));
    let text = joined(&files, "docs/gaps.md");
    assert!(
        text.contains(
            "|---|---|---|\n| 2 | Second | e2 |\n| 4 | Tie | e4 |\n| 1 | First | e1 |\n| 5 | New | line one<br>line \\| two |\n\nProse below.\n"
        ),
        "{text}"
    );
    assert!(text.starts_with("# Gaps\n\n## Every known gap, ranked\n\nProse above.\n\n"));
}

#[test]
fn closed_items_strike_or_omit() {
    const OMIT: &[Table] = &[Table { show_closed: false, ..TABLES[0] }];
    let mut files = docs();
    files.retain(|f| f.path != "docs/gaps.d/3-third.md");
    files.push(f("docs/gaps.d/3-third.md", "---\nstatus: closed\ngap: ~~Third~~ closed\n---\ne3", 0));
    assert!(joined(&files, "docs/gaps.md").contains("| 3 | ~~Third~~ closed | e3 |"));
    let omitted = join(&Config { logs: LOGS, tables: OMIT }, &files).unwrap();
    assert!(!omitted.docs.get("docs/gaps.md").unwrap().contains("Third"));
}

#[test]
fn bad_files_are_all_reported() {
    let mut files = docs();
    files.push(f("docs/progress.d/1-empty.md", "\n\n", 0));
    files.push(f("docs/progress.d/2-kind.md", "---\nkind: huge\n---\n- x", 0));
    files.push(f("docs/progress.d/Bad Name.md", "- x", 0));
    files.push(f("docs/gaps.d/9-typo.md", "---\nstaus: open\ngap: x\n---\n", 0));
    files.push(f("docs/gaps.d/8-nogap.md", "---\nrank: 1\n---\nbody", 0));
    files.push(f("docs/gaps.d/1-dupe.md", "---\ngap: dupe\n---\n", 0));
    files.push(f("docs/progress.d/3-x.md", "---\nkind: minor\n- no closing fence", 0));
    let errors = join(&CFG, &files).unwrap_err();
    for want in [
        "1-empty.md: is empty",
        "isn't trivial, minor or major",
        "Bad Name.md: a fragment is",
        "unknown front matter key `staus`",
        "has no `gap:`",
        "the same number 1",
        "isn't `key: value`",
    ] {
        assert!(errors.iter().any(|e| e.contains(want)), "missing {want:?} in {errors:#?}");
    }
}

#[test]
fn table_columns_must_match_the_config() {
    let mut files = docs();
    files.retain(|f| f.path != "docs/gaps.md");
    files.push(f("docs/gaps.md", &GAPS.replace("| # | Gap | Evidence |", "| # | Gap | Proof |"), 0));
    let errors = join(&CFG, &files).unwrap_err();
    assert!(errors[0].contains("the config says"), "{errors:?}");
}

#[test]
fn check_passes_a_pr_that_only_adds_files() {
    let base = docs();
    let mut head = base.clone();
    head.push(f("docs/progress.d/12-a.md", "- 2026-11-05: a\n", NOV5));
    for i in head.iter_mut().filter(|i| i.path == "docs/gaps.d/1-first.md") {
        i.text = i.text.replace("e1", "better evidence");
    }
    assert_eq!(check(&CFG, &base, &head), Vec::<String>::new());
}

#[test]
fn check_fails_hand_edits_to_joined_regions() {
    let base = docs();
    let mut head = base.clone();
    for d in head.iter_mut() {
        if d.path == "ROADMAP.md" {
            d.text = d
                .text
                .replace("- 2026-11-01: old entry", "- 2026-11-06: by hand\n- 2026-11-01: old entry")
                .replace("2026-11-01 · **Change", "2026-11-06 · **Change");
        }
        if d.path == "docs/gaps.md" {
            d.text = d.text.replace("| 3 | Third | e3 |", "| 3 | Third | edited by hand |");
        }
    }
    let errors = check(&CFG, &base, &head);
    assert!(errors.iter().any(|e| e.contains("`## Progress log` was edited by hand")), "{errors:#?}");
    assert!(errors.iter().any(|e| e.contains("**Last updated:** was edited by hand")), "{errors:#?}");
    assert!(errors.iter().any(|e| e.contains("table under `## Every known gap, ranked` was edited by hand")), "{errors:#?}");
}

#[test]
fn check_allows_prose_outside_regions() {
    let base = docs();
    let mut head = base.clone();
    for d in head.iter_mut().filter(|d| d.path == "docs/gaps.md") {
        d.text = d.text.replace("Prose below.", "Better prose.");
    }
    assert_eq!(check(&CFG, &base, &head), Vec::<String>::new());
}

#[test]
fn check_passes_the_fold_pr() {
    let mut base = docs();
    base.push(f("docs/progress.d/12-a.md", "---\nkind: major\n---\n- 2026-11-05: a\n", NOV5));
    for i in base.iter_mut().filter(|i| i.path == "docs/gaps.d/1-first.md") {
        i.text = i.text.replace("e1", "better evidence");
    }
    // The fold: docs rewritten, the fragment gone, items kept.
    let j = join(&CFG, &base).unwrap();
    let mut head: Vec<File> = base.iter().filter(|f| !j.consumed.contains(&f.path)).cloned().collect();
    for d in head.iter_mut() {
        if let Some(t) = j.docs.get(&d.path) {
            d.text = t.clone();
        }
    }
    // CI's checkout is shallow: commit times are unknown there.
    let zero = |v: &[File]| v.iter().map(|f| File { time: 0, ..f.clone() }).collect::<Vec<_>>();
    assert_eq!(check(&CFG, &zero(&base), &zero(&head)), Vec::<String>::new());
    // A fold that slips in an extra log line is a hand edit.
    let mut sneaky = head.clone();
    for d in sneaky.iter_mut().filter(|d| d.path == "ROADMAP.md") {
        d.text = d.text.replace("- 2026-11-01: old entry", "- 2026-11-06: sneaked in\n- 2026-11-01: old entry");
    }
    assert!(check(&CFG, &base, &sneaky).iter().any(|e| e.contains("`## Progress log` was edited by hand")));
    // A second fold changes nothing.
    assert_eq!(join(&CFG, &head).unwrap().docs.get("docs/gaps.md"), j.docs.get("docs/gaps.md"));
}

#[test]
fn migrate_moves_log_lines_and_row_edits() {
    let main = docs();
    let old = f("docs/gaps.md", GAPS, 0);
    let pr = f(
        "docs/gaps.md",
        &GAPS.replace("| 3 | Third | e3 |", "| 3 | Third | e3 and more |").replace("| 2 | Second | e2 |", "| 2 | Second | e2 |\n| 4 | Fourth | e4 |"),
        0,
    );
    let (out, notes) = migrate(&CFG, &main, &old, &pr, "77-x");
    let paths: Vec<&str> = out.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["docs/gaps.d/3-third.md", "docs/gaps.d/4-fourth.md"], "{notes:?}");
    assert!(out[0].text.contains("rank: 2") && out[0].text.contains("e3 and more"), "{}", out[0].text);

    let old = f("ROADMAP.md", ROADMAP, 0);
    let pr = f("ROADMAP.md", &ROADMAP.replace("- 2026-11-01: old entry", "- 2026-11-04: from the pr\n- 2026-11-01: old entry"), 0);
    let (out, _) = migrate(&CFG, &main, &old, &pr, "77-x");
    assert_eq!(out, [f("docs/progress.d/77-x.md", "- 2026-11-04: from the pr\n", 0)]);
}

#[test]
fn migrate_reports_what_it_cannot_move() {
    let main = docs();
    let old = f("docs/gaps.md", GAPS, 0);
    let pr = f("docs/gaps.md", &GAPS.replace("| 3 | Third | e3 |\n", "").replace("| 2 | Second | e2 |", "| 2 | Second |"), 0);
    let (_, notes) = migrate(&CFG, &main, &old, &pr, "77-x");
    assert!(notes.iter().any(|n| n.contains("removed row 3")), "{notes:?}");
    assert!(notes.iter().any(|n| n.contains("move it by hand")), "{notes:?}");
}

#[test]
fn unclosed_front_matter_is_an_error() {
    assert!(front::parse("---\nkind: minor\n", &["kind"]).unwrap_err().contains("no closing"));
}

#[test]
fn check_lets_the_cutover_pr_through() {
    // Before the cutover there are no folders; the cutover PR adds them and may rewrite the regions.
    let base = vec![f("ROADMAP.md", ROADMAP, 0), f("docs/gaps.md", GAPS, 0)];
    let mut head = docs();
    for d in head.iter_mut().filter(|d| d.path == "ROADMAP.md") {
        d.text = d.text.replace("**Change:** trivial (old thing); previously minor (older)", "**Change:** trivial");
    }
    assert_eq!(check(&CFG, &base, &head), Vec::<String>::new());
}
