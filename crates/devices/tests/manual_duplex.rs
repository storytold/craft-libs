//! A pile larger than the feeder, scanned in batches, then its backs, through the public API only: what an app's
//! scan command does with the journal and the page files between scanner runs. No scanner is involved; each "run"
//! writes the page files where `duplex_page_number` says and updates the journal as the app would.

use std::path::{Path, PathBuf};

use craft_devices::scan_jobs::{
    JobRecord, ManualDuplex, detail_text, duplex_page_number, exit_code, job_json, page_mismatch, page_numbers, place_fronts_for_duplex, read_record,
    request_json, write_record,
};
use craft_scan::ScanSettings;
use craft_scan::job::{Outcome, PageInfo, ScanJob, ScanJobConfig, ScanJobState};
use serde_json::{Value, json};

/// A temp folder that is removed when it is dropped, so a failing assertion does not leave it behind.
struct Folder(PathBuf);

impl std::ops::Deref for Folder {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A fresh, empty temp folder for the test `name`.
fn folder(name: &str) -> Folder {
    let dir = std::env::temp_dir().join(format!("craft-devices-duplex-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp folder");
    Folder(dir)
}

/// The journal entry of a 2x2 bmp page numbered `number`.
fn info(number: u32) -> PageInfo {
    serde_json::from_value(json!({ "number": number, "attempt": 1, "format": "bmp", "width": 2, "height": 2, "dpi": 200, "bytes": 8 }))
        .expect("page info")
}

/// One feeder load: `sheets` more pages arrive and are written where they belong; the journal keeps them.
fn feed(dir: &Path, record: &mut JobRecord, labels: &[&str]) {
    for label in labels {
        let index = record.journal.pages.len();
        let number = match record.manual_duplex {
            Some(duplex) => duplex_page_number(duplex.sheets, index),
            None => index + 1,
        };
        std::fs::write(dir.join(format!("page-{number:03}.bmp")), label).expect("page");
        record.journal.pages.push(info(u32::try_from(index + 1).unwrap_or(0)));
    }
    if let Some(duplex) = record.manual_duplex {
        record.manual_duplex = Some(duplex.after(record.journal.pages.len()));
    }
    write_record(dir, record).expect("journal");
}

/// The text of every page file in `dir`, in page-number order.
fn contents(dir: &Path) -> Vec<String> {
    page_numbers(dir).expect("numbers").iter().map(|n| std::fs::read_to_string(dir.join(format!("page-{n:03}.bmp"))).expect("page")).collect()
}

#[test]
fn fronts_in_two_loads_then_backs_in_two_parts_end_in_document_order() {
    let dir = folder("pile");
    let settings = ScanSettings { pages: 0, ..ScanSettings::default() };
    let mut record = JobRecord {
        id: "scan-pile".into(),
        started: 1,
        updated: 1,
        request: request_json(&settings),
        journal: ScanJob::new(settings, ScanJobConfig::default()).snapshot().into_journal(),
        view: Value::Null,
        attempts: Vec::new(),
        manual_duplex: None,
    };
    // Two feeder loads of fronts; each load finishes the whole-feeder job, which can then take more sheets.
    feed(&dir, &mut record, &["front 1", "front 2"]);
    record.journal.state = ScanJobState::Terminal(Outcome::Completed);
    assert!(record.can_add_sheets() && record.can_turn());
    feed(&dir, &mut record, &["front 3"]);
    assert_eq!(contents(&dir), ["front 1", "front 2", "front 3"]);

    // The turn: the journal says so first, then the fronts move to the odd pages.
    record.manual_duplex = Some(ManualDuplex::backs_of(3));
    record.journal.expected_pages = Some(6);
    record.journal.state = ScanJobState::Interrupted;
    write_record(&dir, &record).expect("journal");
    place_fronts_for_duplex(&dir, 3).expect("place");
    assert_eq!(page_numbers(&dir).expect("numbers"), [1, 3, 5]);

    // The first part of the flipped stack: the backs of sheets 3 and 2. The folder agrees with the journal at once.
    feed(&dir, &mut record, &["back 3", "back 2"]);
    let read = read_record(&dir).expect("read");
    assert_eq!(read.manual_duplex.map(|d| d.cursor), Some(1), "the back of sheet 1 is next");
    assert!(page_mismatch(&dir, &read).is_none());

    // The rest of the stack.
    feed(&dir, &mut record, &["back 1"]);
    assert_eq!(contents(&dir), ["front 1", "back 1", "front 2", "back 2", "front 3", "back 3"]);
    let read = read_record(&dir).expect("read");
    assert_eq!(read.manual_duplex.map(|d| d.cursor), Some(0));
    assert!(page_mismatch(&dir, &read).is_none());
}

#[test]
fn a_backs_pass_that_ran_the_feeder_empty_still_says_the_backs_are_not_finished() {
    let dir = folder("backs-left");
    let settings = ScanSettings { pages: 0, ..ScanSettings::default() };
    let mut record = JobRecord {
        id: "scan-backs-left".into(),
        started: 1,
        updated: 1,
        request: request_json(&settings),
        journal: ScanJob::new(settings, ScanJobConfig::default()).snapshot().into_journal(),
        view: Value::Null,
        attempts: Vec::new(),
        manual_duplex: None,
    };
    feed(&dir, &mut record, &["front 1", "front 2", "front 3"]);
    record.manual_duplex = Some(ManualDuplex::backs_of(3));
    place_fronts_for_duplex(&dir, 3).expect("place");
    // A whole-feeder backs pass: the part of the stack in the feeder ran out, which ends the job Completed.
    feed(&dir, &mut record, &["back 3"]);
    record.journal.state = ScanJobState::Terminal(Outcome::Completed);
    write_record(&dir, &record).expect("journal");
    let read = read_record(&dir).expect("read");
    assert!(read.backs_remaining());
    assert_eq!(read.exit_class(), ScanJobState::NeedsAttention.exit_class(), "backs are missing: not a success");
    assert_ne!(exit_code(read.exit_class()), 0);
    assert_eq!(job_json(&dir, &read)["exit_code"], 10, "the JSON says so too");
    let text = detail_text("app", &dir, &read);
    assert!(!text.contains("nothing; the scan is complete"), "{text}");
    assert!(text.contains("next: scan the rest of the backs") && text.contains("the back of sheet 2 is next"), "{text}");

    // Every back in: done.
    feed(&dir, &mut record, &["back 2", "back 1"]);
    let read = read_record(&dir).expect("read");
    assert!(!read.backs_remaining());
    assert_eq!(read.exit_class(), ScanJobState::Terminal(Outcome::Completed).exit_class());
    assert!(detail_text("app", &dir, &read).contains("nothing; the scan is complete"));
}

#[test]
fn a_journal_with_no_sheets_is_refused_and_the_numbering_is_total() {
    let dir = folder("no-sheets");
    let settings = ScanSettings::default();
    let record = JobRecord {
        id: "scan-no-sheets".into(),
        started: 1,
        updated: 1,
        request: request_json(&settings),
        journal: ScanJob::new(settings, ScanJobConfig::default()).snapshot().into_journal(),
        view: Value::Null,
        attempts: Vec::new(),
        manual_duplex: Some(ManualDuplex::backs_of(1)),
    };
    write_record(&dir, &record).expect("journal");
    // Hand-edit the file to {"sheets": 0, ...}: reading it is an error, not a panic in the listing.
    let path = dir.join(craft_devices::scan_jobs::JOB_FILE);
    let mut file: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("file")).expect("json");
    file["manual_duplex"] = json!({ "sheets": 0, "direction": "backs", "cursor": 0 });
    std::fs::write(&path, file.to_string()).expect("write");
    let error = read_record(&dir).expect_err("refused");
    assert!(error.contains("sheets is 0"), "{error}");
    // The numbering itself never overflows.
    assert_eq!((0..3).map(|i| duplex_page_number(0, i)).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(duplex_page_number(u32::MAX, usize::MAX), 2);
}
