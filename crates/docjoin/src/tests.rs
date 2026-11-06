#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

/// The tests' today: after every fixture date.
fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 30).unwrap()
}

fn join(cfg: &Config, files: &[File]) -> Result<Joined, Vec<String>> {
    super::join(cfg, files, today())
}

fn check(cfg: &Config, base: &[File], head: &[File]) -> Vec<String> {
    super::check(cfg, base, head, today())
}

const ROADMAP: &str = "# App roadmap\n\n> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-11-01 · **Change:** trivial (old thing); previously minor (older) · **Target:** Word 16\n\nIntro.\n\n## Progress log\n\n- 2026-11-01: old entry\n  wrapped.\n- 2026-10-30: older entry\n\n## Next\n\nStuff.\n";

const GAPS: &str = "# Gaps\n\n## Every known gap, ranked\n\nProse above.\n\n| # | Gap | Evidence |\n|---|---|---|\n| 1 | First | e1 |\n| 3 | Third | e3 |\n| 2 | Second | e2 |\n\nProse below.\n\n## Revision history\n\n| Date | Change | Summary |\n|---|---|---|\n| 2026-11-01 | minor | x |\n";

const LOGS: &[Log] = &[Log { dir: "docs/progress.d", doc: "ROADMAP.md", heading: "## Progress log", status: true, ..Log::DEFAULT }];
const TABLES: &[Table] = &[Table {
    dir: "docs/gaps.d",
    doc: "docs/gaps.md",
    heading: "## Every known gap, ranked",
    columns: &[("#", "number"), ("Gap", "gap"), ("Evidence", "body")],
    ..Table::DEFAULT
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
    let mut out =
        vec![f("ROADMAP.md", ROADMAP, 0), f("docs/gaps.md", GAPS, 0), f("docs/progress.d/README.md", "how", 0), f("docs/gaps.d/README.md", "how", 0)];
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
    const L: &[Log] = &[Log { dir: "d", doc: "R.md", heading: "## Progress log", ..Log::DEFAULT }];
    let cfg = Config { logs: L, tables: &[] };
    let doc = "## Progress log\n\n| Date | Entry |\n|---|---|\n| 2026-11-01 | old |\n";
    let ok = join(&cfg, &[f("R.md", doc, 0), f("d/1-x.md", "| 2026-11-06 | new |", NOV6)]).unwrap();
    assert_eq!(ok.docs.get("R.md").unwrap(), "## Progress log\n\n| Date | Entry |\n|---|---|\n| 2026-11-06 | new |\n| 2026-11-01 | old |\n");
    let bad = join(&cfg, &[f("R.md", doc, 0), f("d/1-x.md", "| only one |", NOV6)]).unwrap_err();
    assert!(bad[0].contains("isn't a row of the log's 2 columns"), "{bad:?}");
}

#[test]
fn oldest_first_log_appends() {
    const L: &[Log] = &[Log { dir: "d", doc: "R.md", heading: "## Change log", newest_first: false, ..Log::DEFAULT }];
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
    files.push(f("docs/gaps.d/3-third.md", "---\nstatus: closed\ngap: Third\n---\ne3", 0));
    assert!(joined(&files, "docs/gaps.md").contains("| 3 | ~~Third~~ | ~~e3~~ |"));
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
    files.push(f("docs/progress.d/3-x.md", "---\nkind: minor\n- no closing fence", 0));
    let errors = join(&CFG, &files).unwrap_err();
    for want in [
        "1-empty.md: is empty",
        "isn't trivial, minor or major",
        "Bad Name.md: a fragment is",
        "unknown front matter key `staus`",
        "has no `gap:`",
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

// ---- review findings ----

fn set(files: &mut [File], path: &str, edit: impl Fn(&str) -> String) {
    for d in files.iter_mut().filter(|d| d.path == path) {
        d.text = edit(&d.text);
    }
}

fn fold_of(cfg: &Config, base: &[File]) -> Vec<File> {
    let j = join(cfg, base).unwrap();
    let mut head: Vec<File> = base.iter().filter(|f| !j.consumed.contains(&f.path)).cloned().collect();
    for d in head.iter_mut() {
        if let Some(t) = j.docs.get(&d.path) {
            d.text = t.clone();
        }
    }
    head
}

fn zero(v: &[File]) -> Vec<File> {
    v.iter().map(|f| File { time: 0, ..f.clone() }).collect()
}

#[track_caller]
fn has(errors: &[String], want: &str) {
    assert!(errors.iter().any(|e| e.contains(want)), "missing {want:?} in {errors:#?}");
}

#[test]
fn readme_is_never_consumed_and_cannot_be_deleted() {
    let base = docs();
    let mut head = base.clone();
    head.retain(|f| f.path != "docs/progress.d/README.md");
    set(&mut head, "ROADMAP.md", |t| {
        t.replace(
            "**Last updated:** 2026-11-01 · **Change:** trivial (old thing); previously minor (older)",
            "**Last updated:** 2099-01-01 · **Change:** major",
        )
    });
    let errors = check(&CFG, &base, &head);
    has(&errors, "docs/progress.d/README.md: was deleted");
    has(&errors, "**Last updated:** was edited by hand");
}

#[test]
fn a_pending_fragment_cannot_be_dropped() {
    let mut base = docs();
    base.push(f("docs/progress.d/12-a.md", "- 2026-11-05: someone's entry\n", NOV5));
    let mut head = base.clone();
    head.retain(|f| f.path != "docs/progress.d/12-a.md");
    has(&check(&CFG, &base, &head), "docs/progress.d/12-a.md was deleted but its entry isn't in `## Progress log`");
}

#[test]
fn folding_cannot_reorder_the_log() {
    let mut base = docs();
    base.push(f("docs/progress.d/12-a.md", "- 2026-11-05: a\n", NOV5));
    let mut head = base.clone();
    head.retain(|f| f.path != "docs/progress.d/12-a.md");
    set(&mut head, "ROADMAP.md", |t| {
        t.replace(
            "- 2026-11-01: old entry\n  wrapped.\n- 2026-10-30: older entry\n",
            "  wrapped.\n- 2026-10-30: older entry\n- 2026-11-05: a\n- 2026-11-01: old entry\n",
        )
    });
    has(&check(&CFG, &base, &head), "`## Progress log` was edited by hand");
}

#[test]
fn the_fold_block_may_hold_fragments_in_any_order() {
    let mut base = docs();
    base.push(f("docs/progress.d/12-z.md", "- 2026-11-05: a\n", NOV6));
    base.push(f("docs/progress.d/13-a.md", "- 2026-11-06: b\n  more\n", NOV5));
    let head = fold_of(&CFG, &base);
    assert_eq!(check(&CFG, &zero(&base), &zero(&head)), Vec::<String>::new());
}

#[test]
fn a_repeated_heading_is_an_error() {
    let mut files = docs();
    set(&mut files, "docs/gaps.md", |t| {
        t.replacen(
            "# Gaps\n",
            "# Gaps\n\n<details>\n\n## Every known gap, ranked\n\n| # | Gap | Evidence |\n|---|---|---|\n| 1 | First | e1 |\n\n</details>\n",
            1,
        )
    });
    has(&join(&CFG, &files).unwrap_err(), "has `## Every known gap, ranked` 2 times");
    has(&check(&CFG, &docs(), &files), "2 times");
}

#[test]
fn fragment_lines_cannot_be_headings_or_status_lines() {
    for text in ["- 2026-11-05: a\n## Next\n", "- a\n#\n", "- a\n  > **Last updated:** 2026-01-01"] {
        let mut files = docs();
        files.push(f("docs/progress.d/12-a.md", text, NOV5));
        has(&join(&CFG, &files).unwrap_err(), "a fragment is only its log entry");
        has(&check(&CFG, &docs(), &files), "12-a.md");
    }
}

#[test]
fn a_backslash_before_a_pipe_stays_in_its_cell() {
    let mut files = docs();
    set(&mut files, "docs/gaps.d/1-first.md", |_| "---\nrank: 1\ngap: C:\\|D \\\\ E\n---\ne1".into());
    let text = joined(&files, "docs/gaps.md");
    let row = text.lines().find(|l| l.starts_with("| 1 |")).unwrap();
    assert_eq!(cells(row).unwrap().len(), 3, "{row}");
    let (items, _) = items_from_doc(&CFG, &[f("docs/gaps.md", &text, 0)]).unwrap();
    assert!(items[0].text.contains("gap: C:\\|D \\\\ E\n"), "{}", items[0].text);
    assert_eq!(cells("| a\\\\| b |").unwrap(), ["a\\\\", "b"]);
}

#[test]
fn carriage_returns_become_breaks() {
    let mut files = docs();
    set(&mut files, "docs/gaps.d/1-first.md", |_| "---\nrank: 1\ngap: a\rb\n---\ne1\r\nmore\rlast".into());
    let text = joined(&files, "docs/gaps.md");
    assert!(text.contains("| 1 | a<br>b | e1<br>more<br>last |"), "{text:?}");
}

#[test]
fn escaped_and_struck_rows_round_trip() {
    let doc = GAPS.replace("| 3 | Third | e3 |", "| 3 | ~~Third~~ | ~~a \\| b<br>c~~ |");
    let (items, _) = items_from_doc(&CFG, &[f("docs/gaps.md", &doc, 0)]).unwrap();
    let third = items.iter().find(|i| i.path == "docs/gaps.d/3-third.md").unwrap();
    assert_eq!(third.text, "---\nrank: 2\nstatus: closed\ngap: Third\n---\n\na | b\nc\n");
    let mut files = vec![f("docs/gaps.md", &doc, 0), f("ROADMAP.md", ROADMAP, 0)];
    files.extend(items);
    assert_eq!(joined(&files, "docs/gaps.md"), doc);
}

#[test]
fn deleting_an_item_fails() {
    let base = docs();
    let mut head = base.clone();
    head.retain(|f| f.path != "docs/gaps.d/3-third.md");
    has(&check(&CFG, &base, &head), "docs/gaps.d/3-third.md: was deleted; items are never deleted");
    let mut all = base.clone();
    all.retain(|f| !f.path.starts_with("docs/gaps.d/"));
    set(&mut all, "docs/gaps.md", |t| t.replace("| 1 | First | e1 |", "| 1 | anything | x |"));
    let errors = check(&CFG, &base, &all);
    has(&errors, "docs/gaps.d/1-first.md: was deleted");
}

#[test]
fn a_status_line_of_another_shape_is_an_error() {
    let mut files = docs();
    set(&mut files, "ROADMAP.md", |t| {
        t.replace(
            "**Last updated:** 2026-11-01 · **Change:** trivial (old thing); previously minor (older) · **Target:** Word 16",
            "**Last updated:** 2026-11-01 | **Change:** trivial | **Target:** Word 16",
        )
    });
    has(&join(&CFG, &files).unwrap_err(), "ROADMAP.md line 3: the status line isn't");
}

#[test]
fn a_second_status_line_is_an_error() {
    let base = docs();
    let mut head = base.clone();
    set(&mut head, "ROADMAP.md", |t| {
        t.replacen("# App roadmap\n", "# App roadmap\n<!-- **Last updated:** 2026-11-01 · **Change:** trivial · -->\n", 1).replace(
            "**Last updated:** 2026-11-01 · **Change:** trivial (old thing); previously minor (older) · **Target",
            "**Last updated:** 2099-12-31 · **Change:** major · **Target",
        )
    });
    has(&join(&CFG, &head).unwrap_err(), "ROADMAP.md has `**Last updated:**` on lines 2 and 4");
    assert!(!check(&CFG, &base, &head).is_empty());
}

#[test]
fn change_list_is_not_change() {
    let mut files = docs();
    set(&mut files, "ROADMAP.md", |t| t.replace("· **Change:** trivial (old thing); previously minor (older) ·", "· **Change list:** x ·"));
    files.push(f("docs/progress.d/12-a.md", "- 2026-11-06: a\n", NOV6));
    let text = joined(&files, "ROADMAP.md");
    assert!(text.contains("**Last updated:** 2026-11-06 · **Change:** minor · **Change list:** x · **Target"), "{text}");
}

#[test]
fn the_status_date_never_goes_back() {
    let mut files = docs();
    files.push(f("docs/progress.d/12-a.md", "- a\n", 0));
    assert!(joined(&files, "ROADMAP.md").contains("**Last updated:** 2026-11-01 · **Change:** minor ·"));
}

#[test]
fn item_numbers_are_bounded_and_plain() {
    let mut files = docs();
    for (name, text) in [
        ("1000001-big.md", "---\ngap: x\n---\n"),
        ("+4-plus.md", "---\ngap: x\n---\n"),
        ("04-zero.md", "---\ngap: x\n---\n"),
        ("0-zero.md", "---\ngap: x\n---\n"),
        ("5-rank.md", "---\nrank: 3000000000\ngap: x\n---\n"),
    ] {
        files.push(f(&format!("docs/gaps.d/{name}"), text, 0));
    }
    let errors = join(&CFG, &files).unwrap_err();
    for want in [
        "1000001-big.md: an item's number",
        "+4-plus.md: an item's number",
        "04-zero.md: an item's number",
        "0-zero.md: an item's number",
        "isn't a whole number",
    ] {
        has(&errors, want);
    }
    let (items, _) = items_from_doc(&CFG, &[f("docs/gaps.md", &GAPS.replace("| 2 | Second |", "| 2000000 | Second |"), 0)]).unwrap_or_default();
    assert!(items.is_empty());
    has(&items_from_doc(&CFG, &[f("docs/gaps.md", &GAPS.replace("| 2 | Second |", "| 99999999999 | Second |"), 0)]).unwrap_err(), "above 1000000");
}

#[test]
fn migrate_survives_the_largest_number() {
    let mut main = docs();
    main.push(f("docs/gaps.d/1000000-big.md", "---\ngap: big\n---\nx", 0));
    let old = f("docs/gaps.md", GAPS, 0);
    let pr = f("docs/gaps.md", &GAPS.replace("| 2 | Second | e2 |", "| 2 | Second | e2 |\n| 9 | New | e9 |"), 0);
    let (out, notes) = migrate(&CFG, &main, &old, &pr, "77-x");
    assert!(out.is_empty());
    has(&notes, "no free number");
}

#[test]
fn an_empty_log_gets_blank_lines_around_its_entry() {
    const L: &[Log] = &[Log { dir: "d", doc: "R.md", heading: "## Progress log", ..Log::DEFAULT }];
    let cfg = Config { logs: L, tables: &[] };
    let doc = "## Progress log\n\n## Next\n";
    let j = join(&cfg, &[f("R.md", doc, 0), f("d/1-x.md", "- new", NOV6)]).unwrap();
    assert_eq!(j.docs.get("R.md").unwrap(), "## Progress log\n\n- new\n\n## Next\n");
    let j = join(&cfg, &[f("R.md", "## Progress log\n## Next\n", 0), f("d/1-x.md", "- new", NOV6)]).unwrap();
    assert_eq!(j.docs.get("R.md").unwrap(), "## Progress log\n\n- new\n\n## Next\n");
    // and the check accepts that fold
    let base = vec![f("R.md", doc, 0), f("d/1-x.md", "- new", 0), f("d/README.md", "how", 0)];
    assert_eq!(check(&cfg, &base, &fold_of(&cfg, &base)), Vec::<String>::new());
}

#[test]
fn a_row_without_a_trailing_pipe_is_read() {
    assert_eq!(cells("| a | b").unwrap(), ["a", "b"]);
    let doc = GAPS.replace("| 3 | Third | e3 |", "| 3 | Third | e3");
    let (items, notes) = items_from_doc(&CFG, &[f("docs/gaps.md", &doc, 0)]).unwrap();
    assert_eq!(items.len(), 3, "{notes:?}");
    has(
        &items_from_doc(&CFG, &[f("docs/gaps.md", &GAPS.replace("| 3 | Third | e3 |", "| 3 | Third |"), 0)]).unwrap_err(),
        "row 2: 2 cells, 3 columns",
    );
}

const UI: &[Table] =
    &[Table { dir: "docs/ui.d", doc: "docs/ui.md", heading: "# UI", columns: &[("Area", "area"), ("Parity", "parity")], ..Table::DEFAULT }];
const UICFG: Config = Config { logs: &[], tables: UI };
const UIDOC: &str = "# UI\n\n| Area | Parity |\n|---|---|\n| Ribbon | 50% |\n| Find | 80% |\n";

#[test]
fn migrate_edits_an_unnumbered_row_in_place() {
    let (items, _) = items_from_doc(&UICFG, &[f("docs/ui.md", UIDOC, 0)]).unwrap();
    let mut main = vec![f("docs/ui.md", UIDOC, 0)];
    main.extend(items);
    let pr = f("docs/ui.md", &UIDOC.replace("| Find | 80% |", "| Find | 90% |"), 0);
    let (out, notes) = migrate(&UICFG, &main, &f("docs/ui.md", UIDOC, 0), &pr, "9-x");
    for o in &out {
        main.retain(|m| m.path != o.path);
        main.push(o.clone());
    }
    let text = join(&UICFG, &main).unwrap().docs.get("docs/ui.md").unwrap().clone();
    assert_eq!(text, "# UI\n\n| Area | Parity |\n|---|---|\n| Ribbon | 50% |\n| Find | 90% |\n", "notes {notes:?}");
}

#[test]
fn migrate_keeps_cells_main_changed() {
    let mut main = docs();
    set(&mut main, "docs/gaps.d/3-third.md", |t| t.replace("e3", "e3 updated on main"));
    let old = f("docs/gaps.md", GAPS, 0);
    let pr = f("docs/gaps.md", &GAPS.replace("| 3 | Third | e3 |", "| 3 | Third renamed | e3 |"), 0);
    let (out, notes) = migrate(&CFG, &main, &old, &pr, "77-x");
    assert!(out[0].text.contains("e3 updated on main") && out[0].text.contains("gap: Third renamed"), "{}", out[0].text);
    assert!(notes.is_empty(), "{notes:?}");
    let pr = f("docs/gaps.md", &GAPS.replace("| 3 | Third | e3 |", "| 3 | Third | e3 from the pr |"), 0);
    let (out, notes) = migrate(&CFG, &main, &old, &pr, "77-x");
    assert!(out.is_empty(), "{out:?}");
    has(&notes, "conflict");
}

const TWO: &str = "## Tools\n\n| A | B |\n|---|---|\n| x | 1 |\n\nToolbar:\n\n| Tool | Key |\n|---|---|\n| Pen | P |\n";

#[test]
fn nth_picks_a_later_table_under_the_heading() {
    const T: &[Table] =
        &[Table { dir: "t.d", doc: "t.md", heading: "## Tools", columns: &[("Tool", "tool"), ("Key", "key")], nth: 1, ..Table::DEFAULT }];
    let cfg = Config { logs: &[], tables: T };
    let (items, _) = items_from_doc(&cfg, &[f("t.md", TWO, 0)]).unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0].text.contains("tool: Pen"));
    let mut files = vec![f("t.md", TWO, 0)];
    files.extend(items);
    assert_eq!(join(&cfg, &files).unwrap().docs.get("t.md").unwrap(), TWO);
    set(&mut files, "t.d/1-pen.md", |t| t.replace("key: P", "key: Q"));
    assert_eq!(join(&cfg, &files).unwrap().docs.get("t.md").unwrap(), &TWO.replace("| Pen | P |", "| Pen | Q |"));
}

#[test]
fn a_heading_matches_with_a_bracketed_suffix() {
    let doc = UIDOC.replace("# UI\n", "# UI (≈ 45%)\n");
    let (items, _) = items_from_doc(&UICFG, &[f("docs/ui.md", &doc, 0)]).unwrap();
    assert_eq!(items.len(), 2);
    has(&items_from_doc(&UICFG, &[f("docs/ui.md", &UIDOC.replace("# UI\n", "# UI parity\n"), 0)]).unwrap_err(), "has no `# UI` heading");
}

const SGAPS: &str = "# Gaps\n\n> **Last updated:** 2026-11-01 · **Change:** trivial (x); previously minor · **Owner:** y\n\n## Every known gap, ranked\n\n| # | Gap | Evidence |\n|---|---|---|\n| 1 | First | e1 |\n| 2 | Second | e2 |\n\n## Revision history\n\n| Date | Change | Summary |\n|---|---|---|\n| 2026-11-01 | minor | x |\n";
const SLOGS: &[Log] = &[Log { dir: "docs/gaps-log.d", doc: "docs/gaps.md", heading: "## Revision history", status: true, ..Log::DEFAULT }];
const STABLES: &[Table] = &[Table { status: true, ..TABLES[0] }];
const SCFG: Config = Config { logs: SLOGS, tables: STABLES };

fn sdocs() -> Vec<File> {
    let (items, _) = items_from_doc(&SCFG, &[f("docs/gaps.md", SGAPS, 0)]).unwrap();
    let mut out = vec![f("docs/gaps.md", SGAPS, 0), f("docs/gaps-log.d/README.md", "how", 0), f("docs/gaps.d/README.md", "how", 0)];
    out.extend(items);
    out
}

#[test]
fn a_table_docs_status_line_follows_its_items() {
    let mut files = sdocs();
    assert_eq!(join(&SCFG, &files).unwrap().docs.get("docs/gaps.md").unwrap(), SGAPS);
    for i in files.iter_mut().filter(|i| i.path == "docs/gaps.d/2-second.md") {
        i.text = i.text.replace("e2", "e2 better");
        i.time = NOV6;
    }
    let text = join(&SCFG, &files).unwrap().docs.get("docs/gaps.md").unwrap().clone();
    assert!(text.contains("> **Last updated:** 2026-11-06 · **Change:** minor · **Owner:** y\n"), "{text}");
    files.push(f("docs/gaps-log.d/1-x.md", "---\nkind: major\n---\n| 2026-11-05 | major | z |", NOV5));
    let text = join(&SCFG, &files).unwrap().docs.get("docs/gaps.md").unwrap().clone();
    assert!(text.contains("> **Last updated:** 2026-11-06 · **Change:** major · **Owner:** y\n"), "{text}");
}

#[test]
fn check_guards_a_table_docs_status_line() {
    let base = sdocs();
    let mut hand = base.clone();
    set(&mut hand, "docs/gaps.md", |t| t.replace("2026-11-01 · **Change:** trivial (x); previously minor", "2026-11-07 · **Change:** minor"));
    has(&check(&SCFG, &base, &hand), "docs/gaps.md: the status line's **Last updated:** was edited by hand");
    // The fold after an item edit passes, with the fold's own Change only.
    let mut edited = base.clone();
    for i in edited.iter_mut().filter(|i| i.path == "docs/gaps.d/2-second.md") {
        i.text = i.text.replace("e2", "e2 better");
        i.time = NOV6;
    }
    let fold = fold_of(&SCFG, &edited);
    assert_eq!(check(&SCFG, &zero(&edited), &zero(&fold)), Vec::<String>::new());
    let mut wrong = fold.clone();
    set(&mut wrong, "docs/gaps.md", |t| t.replace("**Change:** minor", "**Change:** major"));
    has(&check(&SCFG, &edited, &wrong), "**Change:** is `major`; the fold writes `minor`");
    // No item changes, no fragments: the status line can't move.
    let mut moved = fold.clone();
    set(&mut moved, "docs/gaps.md", |t| t.replace("2026-11-06", "2026-11-08"));
    has(&check(&SCFG, &fold, &moved), "**Last updated:** was edited by hand");
}

#[test]
fn a_log_folder_needs_its_readme() {
    let mut base = docs();
    base.retain(|f| f.path != "docs/progress.d/README.md");
    let head = base.clone();
    let errors = check(&CFG, &base, &head);
    assert!(errors.iter().any(|e| e.contains("docs/progress.d/README.md: missing")), "{errors:?}");
}

// ---- second review ----

#[test]
fn a_pr_cannot_write_a_status_date_after_today() {
    let mut base = docs();
    base.push(f("docs/progress.d/12-a.md", "- 2026-11-05: a\n", NOV5));
    let mut head = fold_of(&CFG, &base);
    set(&mut head, "ROADMAP.md", |t| t.replace("**Last updated:** 2026-11-05", "**Last updated:** 2099-12-31"));
    has(&check(&CFG, &zero(&base), &zero(&head)), "**Last updated:** is 2099-12-31, after today (2026-11-30)");
    // Nor by editing an item of a table that writes the status line.
    let base = sdocs();
    let mut head = base.clone();
    set(&mut head, "docs/gaps.d/2-second.md", |t| t.replace("e2", "e2 better"));
    set(&mut head, "docs/gaps.md", |t| t.replace("2026-11-01 · **Change:** trivial (x); previously minor", "2099-01-01 · **Change:** minor"));
    has(&check(&SCFG, &base, &head), "after today");
    // Today itself is fine.
    set(&mut head, "docs/gaps.md", |t| t.replace("2099-01-01", "2026-11-30"));
    assert_eq!(check(&SCFG, &base, &head), Vec::<String>::new());
}

#[test]
fn the_fold_never_writes_a_date_after_today() {
    let mut files = docs();
    files.push(f("docs/progress.d/12-a.md", "- a\n", 4_102_444_800)); // 2100-01-01
    assert!(joined(&files, "ROADMAP.md").contains("**Last updated:** 2026-11-30 ·"));
    // A future date already on main comes back to today at the next fold.
    set(&mut files, "ROADMAP.md", |t| t.replace("**Last updated:** 2026-11-01", "**Last updated:** 2099-12-31"));
    let head = fold_of(&CFG, &files);
    assert!(head.iter().any(|f| f.path == "ROADMAP.md" && f.text.contains("**Last updated:** 2026-11-30 ·")));
    assert_eq!(check(&CFG, &zero(&files), &zero(&head)), Vec::<String>::new());
}

#[test]
fn a_line_right_after_a_table_is_one_of_its_rows() {
    // GFM continues a table with a line that has no leading pipe: it's a row nobody joined.
    let base = docs();
    let mut head = base.clone();
    set(&mut head, "docs/gaps.md", |t| t.replace("| 2 | Second | e2 |\n", "| 2 | Second | e2 |\n99 | Fake gap | made up\n"));
    has(&check(&CFG, &base, &head), "table under `## Every known gap, ranked` was edited by hand");
    assert!(!joined(&head, "docs/gaps.md").contains("Fake gap"), "the fold drops it");
    // A line that isn't a row of the table's width is an error, not prose.
    let mut prose = base.clone();
    set(&mut prose, "docs/gaps.md", |t| t.replace("| 2 | Second | e2 |\n", "| 2 | Second | e2 |\nProse glued on.\n"));
    has(&join(&CFG, &prose).unwrap_err(), "row 4: 1 cells, 3 columns: \"Prose glued on.\"");
    has(&check(&CFG, &base, &prose), "1 cells, 3 columns");
    // A block that isn't a paragraph ends the table.
    for block in ["## Next", "- a list", "1. a list", "> quote", "<details>", "```", "~~~"] {
        let mut ended = base.clone();
        set(&mut ended, "docs/gaps.md", |t| t.replace("| 2 | Second | e2 |\n", &format!("| 2 | Second | e2 |\n{block}\n")));
        let j = join(&CFG, &ended);
        assert!(j.as_ref().is_ok_and(|j| j.docs.get("docs/gaps.md").is_some_and(|d| d.contains(&format!("e2 |\n{block}\n")))), "{block}: {j:?}");
    }
}

#[test]
fn a_decoy_table_cannot_take_the_joined_ones_place() {
    let base = docs();
    let decoy = "| # | Gap | Evidence |\n|---|---|---|\n| 1 | First | e1 |\n| 3 | Third | e3 |\n| 2 | Second | e2 |\n";
    let mut head = base.clone();
    set(&mut head, "docs/gaps.md", |t| {
        t.replace("| 3 | Third | e3 |\n", "| 3 | Third | e3, hand edit |\n| 77 | Hand-added | x |\n")
            .replace("Prose above.\n", &format!("Prose above.\n\n<details><summary>old</summary>\n\n{decoy}\n</details>\n"))
    });
    has(&join(&CFG, &head).unwrap_err(), "has 2 tables headed [\"#\", \"Gap\", \"Evidence\"] under `## Every known gap, ranked`");
    has(&check(&CFG, &base, &head), "2 tables headed");
    // Another table in the section, of other columns, is a change the check sees.
    let mut other = base.clone();
    set(&mut other, "docs/gaps.md", |t| t.replace("Prose below.\n", "Prose below.\n\n| A | B |\n|---|---|\n| x | y |\n"));
    has(&check(&CFG, &base, &other), "has 2 table(s), it had 1");
    // Not at the cutover, before the folder is started.
    let pre = vec![f("ROADMAP.md", ROADMAP, 0), f("docs/gaps.md", GAPS, 0)];
    let mut cut = docs();
    set(&mut cut, "docs/gaps.md", |t| t.replace("Prose below.\n", "Prose below.\n\n| A | B |\n|---|---|\n| x | y |\n"));
    assert_eq!(check(&CFG, &pre, &cut), Vec::<String>::new());
}

#[test]
fn a_hand_fold_keeps_each_entry_whole() {
    let mut base = docs();
    base.push(f("docs/progress.d/12-a.md", "- 2026-11-05: a\n  a-more\n", NOV5));
    base.push(f("docs/progress.d/13-b.md", "- 2026-11-06: b\n  b-more\n", NOV6));
    let good = fold_of(&CFG, &base);
    let mut head = good.clone();
    set(&mut head, "ROADMAP.md", |t| {
        t.replace("- 2026-11-06: b\n  b-more\n- 2026-11-05: a\n  a-more\n", "- 2026-11-06: b\n  a-more\n- 2026-11-05: a\n  b-more\n")
    });
    assert_ne!(head, good);
    has(&check(&CFG, &zero(&base), &zero(&head)), "`## Progress log` was edited by hand");
    // The entries swapped, each whole, is fine.
    let mut swapped = good.clone();
    set(&mut swapped, "ROADMAP.md", |t| {
        t.replace("- 2026-11-06: b\n  b-more\n- 2026-11-05: a\n  a-more\n", "- 2026-11-05: a\n  a-more\n- 2026-11-06: b\n  b-more\n")
    });
    assert_eq!(check(&CFG, &zero(&base), &zero(&swapped)), Vec::<String>::new());
    // One entry that starts like another still matches.
    assert!(is_blocks(&["- x", "- x", "y"], &[&["- x"], &["- x", "y"]]));
    assert!(!is_blocks(&["- x", "y", "- x", "z"], &[&["- x"], &["- x", "y", "z"]]));
}

#[test]
fn a_number_two_merged_prs_both_took_doesnt_stop_the_fold() {
    let base = docs();
    let mut a = base.clone();
    a.push(f("docs/gaps.d/4-alpha.md", "---\ngap: Alpha\n---\nea", 0));
    let mut main = a.clone();
    main.push(f("docs/gaps.d/4-beta.md", "---\ngap: Beta\n---\neb", 0));
    // The pull request that adds the second 4 fails.
    has(&check(&CFG, &a, &main), "docs/gaps.d/4-alpha.md and docs/gaps.d/4-beta.md have the same number 4");
    // Once both are on main, the fold goes on, by slug, and warns.
    let j = join(&CFG, &main).unwrap();
    has(&j.warnings, "have the same number 4");
    assert!(j.docs.get("docs/gaps.md").unwrap().contains("| 4 | Alpha | ea |\n| 4 | Beta | eb |\n"), "{:?}", j.docs);
    // And an unrelated pull request passes, with the warning.
    let mut unrelated = main.clone();
    unrelated.push(f("docs/progress.d/50-x.md", "- 2026-11-07: x\n", 0));
    let r = check_report(&CFG, &main, &unrelated, today());
    assert_eq!(r.errors, Vec::<String>::new());
    has(&r.warnings, "have the same number 4");
}

#[test]
fn what_was_already_broken_on_the_base_is_a_warning() {
    let mut base = docs();
    base.push(f("docs/gaps.d/9-typo.md", "---\nstaus: open\ngap: x\n---\n", 0));
    let mut head = base.clone();
    head.push(f("docs/progress.d/50-x.md", "- 2026-11-07: x\n", 0));
    let r = check_report(&CFG, &base, &head, today());
    assert_eq!(r.errors, Vec::<String>::new());
    has(&r.warnings, "9-typo.md: unknown front matter key `staus`");
    // Touching the broken file makes it the pull request's.
    let mut touched = head.clone();
    set(&mut touched, "docs/gaps.d/9-typo.md", |t| t.replace("gap: x", "gap: y"));
    has(&check(&CFG, &base, &touched), "9-typo.md: unknown front matter key `staus`");
}

#[test]
fn closing_an_item_strikes_its_row() {
    let base = docs();
    let mut head = base.clone();
    set(&mut head, "docs/gaps.d/3-third.md", |t| t.replacen("---\n", "---\nstatus: closed\n", 1));
    let text = joined(&head, "docs/gaps.md");
    assert!(text.contains("| 3 | ~~Third~~ | ~~e3~~ |\n"), "{text}");
    // A cell already struck in part, or empty, is left alone; the cutover reads the row back.
    set(&mut head, "docs/gaps.d/3-third.md", |_| "---\nstatus: closed\ngap: ~~Third~~ done\n---\n".into());
    let text = joined(&head, "docs/gaps.md");
    assert!(text.contains("| 3 | ~~Third~~ done | |\n"), "{text}");
    let (items, _) = items_from_doc(&CFG, &[f("docs/gaps.md", &text, 0)]).unwrap();
    assert!(items.iter().any(|i| i.text.contains("status: closed\ngap: ~~Third~~ done\n")), "{items:?}");
}

#[test]
fn migrate_notes_a_log_line_the_pr_edited_or_removed() {
    let old = f("ROADMAP.md", ROADMAP, 0);
    let pr = f("ROADMAP.md", &ROADMAP.replace("- 2026-10-30: older entry", "- 2026-10-30: older entry, typo fixed"), 0);
    let (out, notes) = migrate(&CFG, &docs(), &old, &pr, "7-fix");
    assert!(out.is_empty(), "no second entry: {out:?}");
    has(&notes, "edited or removed the log line \"- 2026-10-30: older entry\"");
    has(&notes, "\"- 2026-10-30: older entry, typo fixed\" replaces one");
    let removed = f("ROADMAP.md", &ROADMAP.replace("- 2026-10-30: older entry\n", ""), 0);
    let (out, notes) = migrate(&CFG, &docs(), &old, &removed, "7-rm");
    assert!(out.is_empty());
    has(&notes, "edited or removed the log line \"- 2026-10-30: older entry\"");
    // A new entry whose second line repeats an old one keeps it.
    let pr = f("ROADMAP.md", &ROADMAP.replace("- 2026-11-01: old entry", "- 2026-11-04: new\n  wrapped.\n- 2026-11-01: old entry"), 0);
    let (out, notes) = migrate(&CFG, &docs(), &old, &pr, "7-new");
    assert_eq!(out, [f("docs/progress.d/7-new.md", "- 2026-11-04: new\n  wrapped.\n", 0)], "{notes:?}");
    assert!(notes.is_empty(), "{notes:?}");
}

#[test]
fn a_table_docs_status_date_is_capped_at_today() {
    let mut files = sdocs();
    for i in files.iter_mut().filter(|i| i.path == "docs/gaps.d/2-second.md") {
        i.text = i.text.replace("e2", "e2 better");
        i.time = 4_102_444_800; // 2100-01-01
    }
    let text = join(&SCFG, &files).unwrap().docs.get("docs/gaps.md").unwrap().clone();
    assert!(text.contains("> **Last updated:** 2026-11-30 · **Change:** minor · **Owner:** y\n"), "{text}");
}

#[test]
fn a_second_change_field_is_an_error() {
    let base = docs();
    let mut head = base.clone();
    set(&mut head, "ROADMAP.md", |t| t.replace("· **Target:** Word 16", "· **Target:** Word 16 · **Change:** major"));
    has(&check(&CFG, &base, &head), "ROADMAP.md line 3: the status line has two `**Change:**` fields");
}

#[test]
fn a_fragment_cannot_open_a_fence_or_a_comment() {
    for text in ["- a\n```\n", "- a\n  ~~~rust\n", "- a <!-- b\n"] {
        let mut files = docs();
        files.push(f("docs/progress.d/12-a.md", text, NOV5));
        has(&join(&CFG, &files).unwrap_err(), "12-a.md: ");
        has(&join(&CFG, &files).unwrap_err(), "opens a code fence or an HTML comment");
    }
}

#[test]
fn a_heading_may_have_a_tab_after_its_hashes() {
    let mut files = docs();
    files.push(f("docs/progress.d/12-a.md", "- a\n##\tNext\n", NOV5));
    has(&join(&CFG, &files).unwrap_err(), "is a heading or a status line");
}

#[test]
fn item_file_names_are_lower_case() {
    for name in ["Readme.md", "readme.MD", "4-New.md"] {
        let mut files = docs();
        files.push(f(&format!("docs/gaps.d/{name}"), "---\ngap: x\n---\n", 0));
        has(&join(&CFG, &files).unwrap_err(), &format!("{name}: an item's file name is lower case"));
    }
}

#[test]
fn a_started_table_folder_needs_its_readme() {
    let base = docs();
    let mut head = base.clone();
    head.retain(|f| f.path != "docs/gaps.d/README.md");
    has(&check(&CFG, &base, &head), "docs/gaps.d/README.md: was deleted");
    let mut base = base;
    base.retain(|f| f.path != "docs/gaps.d/README.md");
    has(&check(&CFG, &base, &head), "docs/gaps.d/README.md: missing");
    // Not before the folder is started.
    let mut none = head.clone();
    none.retain(|f| !f.path.starts_with("docs/gaps.d/"));
    let pre = vec![f("ROADMAP.md", ROADMAP, 0), f("docs/gaps.md", GAPS, 0), f("docs/progress.d/README.md", "how", 0)];
    assert!(!check(&CFG, &pre, &none).iter().any(|e| e.contains("docs/gaps.d/README.md")));
}

#[test]
fn the_fold_pr_passes_on_odd_logs() {
    let shapes: &[(&str, bool)] = &[
        ("# R\n\n**Last updated:** 2026-11-01 · **Change:** minor\n\n## Progress log\n\n## Next\n", true),
        ("# R\n\n**Last updated:** 2026-11-01 · **Change:** minor\n\n## Progress log\nIntro text.\n## Next\n", true),
        ("# R\n\n**Last updated:** 2026-11-01 · **Change:** minor\n\n## Progress log\n\n- old\n\nTrailing prose.\n", false),
        ("# R\r\n\r\n**Last updated:** 2026-11-01 · **Change:** minor\r\n\r\n## Progress log\r\n\r\n- old\r\n", true),
        ("# R\n\n**Last updated:** 2026-11-01 · **Change:** minor\n\n## Progress log\n\n- old", false),
        ("# R\n\n**Last updated:** 2026-11-01 · **Change:** minor\n\n## Progress log (2026)\n\nIntro:\n\n- old\n- older\n", true),
    ];
    for (doc, newest) in shapes {
        let logs: &'static [Log] =
            Box::leak(Box::new([Log { dir: "p", doc: "R.md", heading: "## Progress log", newest_first: *newest, status: true }]));
        let cfg = Config { logs, tables: &[] };
        let base = vec![f("R.md", doc, 0), f("p/README.md", "x", 0), f("p/1-a.md", "- new a\n  more a\n", NOV5), f("p/2-b.md", "- new b\n", NOV6)];
        let head = fold_of(&cfg, &base);
        assert_eq!(check(&cfg, &zero(&base), &zero(&head)), Vec::<String>::new(), "{doc:?}\n{}", head[0].text);
        assert_eq!(fold_of(&cfg, &head), head, "second fold not a no-op for {doc:?}");
    }
}
