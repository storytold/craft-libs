//! The names an app's scan command imports from this crate. A compile check: removing or renaming any of them is a
//! breaking change for the apps that mount these tools.

use craft_devices::scan_jobs::{
    self, Direction, JOB_FILE, JOB_FILE_VERSION, JobRecord, ManualDuplex, candidate_dirs, detail_text, duplex_page_number, exit_code, find_jobs,
    iso8601, job_json, jobs_listing, list_line, next_step, now_millis, page_files_for, page_files_on_disk, page_mismatch, page_numbers,
    place_fronts_for_duplex, read_record, request_json, settings_from_request, state_name, unreadable_json, write_record,
};
use craft_scan::ScanSettings;
use craft_scan::job::{ExitClass, ScanJob, ScanJobConfig, ScanJobState};
use serde_json::Value;

#[test]
fn the_names_an_app_imports_exist_and_have_the_methods_it_calls() {
    let settings = ScanSettings { pages: 2, ..ScanSettings::default() };
    let record = JobRecord {
        id: "scan-1".into(),
        started: now_millis(),
        updated: now_millis(),
        request: request_json(&settings),
        journal: ScanJob::new(settings.clone(), ScanJobConfig::default()).snapshot().into_journal(),
        view: Value::Null,
        attempts: Vec::new(),
        manual_duplex: None,
    };
    assert_eq!(settings_from_request(&record.request).map(|s| s.pages), Ok(2));
    assert_eq!(record.effective_state(), ScanJobState::Preparing);
    let _ = (record.was_cut_off(), record.exit_class(), record.pages_kept(), record.summary_line(), record.last_error_kind());
    let _ = (record.can_add_sheets(), record.can_turn(), record.two_sided(), record.to_json());
    let mut numbered = record.clone();
    numbered.number_pages_by_file();
    assert!(JobRecord::from_json(&record.to_json()).is_ok());
    assert_eq!(exit_code(ExitClass::Completed), 0);
    let _ = (state_name(ScanJobState::Preparing), iso8601(0), JOB_FILE, JOB_FILE_VERSION);

    let duplex = ManualDuplex::backs_of(2);
    assert_eq!((duplex.direction, duplex.cursor, duplex.backs_scanned()), (Direction::Backs, 2, 0));
    assert_eq!(duplex.after(3).cursor, 1);
    assert_eq!(duplex_page_number(2, 2), 4);
    let _ = (Direction::parse("fronts"), Direction::Fronts.as_str());

    let dir = std::env::temp_dir().join(format!("craft-devices-surface-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp folder");
    write_record(&dir, &record).expect("write");
    let read = read_record(&dir).expect("read");
    let _ = (page_files_on_disk(&dir), page_files_for(&dir, &read), page_mismatch(&dir, &read), page_numbers(&dir));
    let _ = (list_line(&dir, &read), job_json(&dir, &read), detail_text("app", &dir, &read), next_step("app", &dir, ScanJobState::Preparing));
    let _ = (unreadable_json(&dir, "why"), find_jobs(&candidate_dirs(&[], &dir)), jobs_listing(std::slice::from_ref(&dir), &dir));
    assert!(place_fronts_for_duplex(&dir, 0).is_ok());
    let _ = scan_jobs::request_json;
    let _ = std::fs::remove_dir_all(&dir);
}
