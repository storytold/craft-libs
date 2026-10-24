# craft-devices

Device tools the Craft apps share, with no UI and no platform code. An app registers its platform backends into
`craft-scan` (and later `craft-print`) and uses these tools from its own command line and tool list instead of
copying them.

## In

- `scan_jobs`: the journal a command-line scan leaves beside its pages (`DIR/scan-job.json`) and the read-only views
  over it.
  - `JobRecord`: a small versioned wrapper around `craft_scan::job::ScanJobJournal` with the request the scan repeats
    on `--continue`, the `craft_job::JobView` and the attempt timeline. It never holds page bytes. Written with
    `craft-atomic` (staged beside the file, then renamed), read leniently (unknown fields ignored, a newer version
    refused with a message to upgrade).
  - Listing and detail views (`jobs_listing`, `list_line`, `job_json`, `detail_text`, `next_step`), exit codes, and
    the request round-trip (`request_json`, `settings_from_request`). The command name in the next-step hints is the
    app's.
  - The folder check (`page_files_for`, `page_mismatch`): the journal does not verify files, so the views count
    `page-NNN.*` and say when they differ.
- **Manual duplex and piles larger than the feeder**, as one job. A finished whole-feeder job can take another feeder
  load of sheets (`JobRecord::can_add_sheets`) or turn for its backs (`can_turn`). `ManualDuplex` records the sheets,
  the direction and the cursor (the sheet whose back is next); the flipped stack feeds the last sheet first. Every
  page file is written straight to its place in the document (`duplex_page_number`: the front of sheet `i` is
  `page-(2i-1)`, its back `page-2i`), so a backs pass can stop at any sheet and be continued, and nothing is
  rearranged at the end. `place_fronts_for_duplex` renumbers the fronts once when the job turns, highest first, and
  is safe to run again after it was cut off. The journal keeps the pages in scan order, each numbered by its page
  file (`JobRecord::number_pages_by_file`). A job scanned through the scanner's duplex unit (`request.duplex`) already
  has its backs and never turns.

## Not in (deliberately)

- Opening a scanner, or any backend: that is `craft-scan` and the app's platform component.
- UI, translations, argument parsing: the app's.
- Writing page files: the app writes them where `duplex_page_number` says.

## Tests

`cargo test -p craft-devices`. The unit tests in `src/scan_jobs.rs` cover the journal format, the views, the request
round-trip and the manual-duplex numbering and renumbering (including a renumbering that was cut off).
`tests/public_surface.rs` is a compile check of the names an app imports, and `tests/manual_duplex.rs` walks a
folder through a pile scanned in batches and its backs through the public API only, as an app's command does.
