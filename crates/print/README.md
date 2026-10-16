# craft-print

Submit a print-ready PDF to the system spooler. Used by PdfCraft and DesignCraft;
there are no dependencies on either app or on a UI framework. Requires Rust 1.95.

```rust,no_run
let job = craft_print::Job { title: "Report".into(), ..Default::default() };
let receipt = craft_print::submit(&pdf_bytes, &job)?;
```

The caller generates the PDF, checks document permissions, and handles page selection,
imposition, marks and bleed. This crate discovers printers, exposes CUPS driver options,
opens Windows printer preferences and submits jobs with copies, collation, duplex and colour.
`command_preview` returns diagnostics without creating files or starting a process.

- **macOS/Linux:** CUPS `lpstat`, `lpoptions` and `lp`. Jobs stream through stdin. Exact-size
  sheets use `fit-to-page=false`; shrink-to-fit uses IPP `print-scaling=auto-fit`.
- **Windows:** in this process, with no PowerShell, script or other program started, so
  printing works where App Control for Business or AppLocker enforce script rules. Printers
  come from `EnumPrinters` (local queues and `\\server\queue` connections) and the default from
  `GetDefaultPrinter`. A job is drawn as vectors by the in-box `Windows.Data.Pdf` with Direct2D
  into the XPS print API (the `winprint` crate), and copies, collation, duplex, colour, paper,
  orientation and "no scaling" travel in a PrintTicket that the driver validates first; a
  driver that would change the paper, orientation or scaling is refused before anything is
  sent. Paper is the driver's own option for the sheet's size (to 0.25 mm), preferring the one
  named like the standard size ("ARCH D" over a custom 24 × 36 in entry). This backend comes
  from storytold/pdfcraft#599 and #1132.
  A job has one ticket, so one paper and orientation. Sheets of the other orientation are
  turned back with `/Rotate`, so portrait and landscape sheets still print as one job. Exact
  size refuses sheets of different sizes and paper the driver lacks. Shrink-to-fit uses the
  driver's matching paper, else its default paper, and puts every sheet on it: a sheet that
  doesn't fit inside the paper's widest hardware margin is shrunk, a smaller one keeps its
  size, and both are centred. Both are done by appending an incremental update to the PDF
  (new versions of the pages that need it, the original bytes untouched); the crate reads
  only cross-reference sections, object streams and the page tree for this, leniently and
  with every size bounded. Encrypted PDFs can be turned but not reframed. Annotations on a
  reframed page are dropped; print-ready PDFs have them flattened. Nondefault or invalid
  `/UserUnit` is refused because the native renderer does not reliably honor physical units.
  Parsing caps cumulative cross-reference/object-stream records at 500,000, decoded streams
  at 16 MiB each and 32 MiB per job, and parsed nodes/tokens at 250,000/16 MiB per job and
  32,000/1 MiB per object. Files exceeding these limits return a clear error.
  Long-edge duplex is `TwoSidedLongEdge` and short-edge `TwoSidedShortEdge`; `duplex: None`
  leaves duplex out of the ticket. Driver-specific defaults remain in Windows printer
  preferences (`rundll32 printui.dll`, the one process this crate starts, and only when asked).
- **Browser/other targets:** discovery is empty and submission reports an unsupported
  platform; callers can offer PDF export instead.

Windows jobs briefly use a file in a private temporary folder (random name, this user's only),
named after the job's title because the spooler shows it as the document name; the folder is
removed once the spooler has the job, on success and failure. Failed and canceled terminal
package statuses are errors, including failures received after every sheet was processed.
A missing completion notification times out after two minutes after the native print control
is closed and requests cancellation. Calls into a driver and interactive output-file dialogs
are not covered by this notification timeout. Windows answers when the job is in
the queue ("spooled 3 sheet(s)"): a paused queue or an offline printer keeps it there. CUPS jobs
never use a PDF temporary file. Printing is synchronous; UI callers should submit on a worker.
`submit_with_progress` reports `(processed, sheets)` before the first Windows sheet and after
each sheet is successfully added to the native print job. These are source PDF sheets, excluding
copies and duplex; the events describe sheet processing, and do not confirm physical output.
The callback runs on the thread calling submission, can capture non-`Send` state, and should
return promptly. It is never retained. Keep waiting for the final `Result` even at full progress:
the job can still fail. The API does not provide cancellation. CUPS reports no progress.
UI callers must forward events to their own progress state; adopting the API alone does not
change an application's UI.
Receipts are backend messages, not portable job IDs. `Job::duplex = None` preserves the queue's
duplex default; `Some(Duplex::Off)` forces simplex. `Job::scaling` defaults to `Scaling::Exact`;
callers exporting arbitrary document sizes can choose `Scaling::ShrinkToFit`. Exact mode reports
a driver paper-size mismatch. On Windows `command_preview` describes the in-process job
(`xps-print`, then the printer and options as `key=value`) instead of a command line. The crate
does not provide PostScript output, separations, trapping or colour-managed device rendering.

Windows shrink-to-fit refuses pages with annotations when their geometry needs fitting.
Callers must flatten annotation appearances into page content first. Exact-size printing and
rotation retain the annotations. This avoids losing printed comments, forms or other marks.
The Windows renderer uses viewer visibility. Native submission therefore accepts only plain
visible printable annotation flags (`F=4`); annotations without Print, with NoView, or with
other visibility/rotation flags require caller flattening. This prevents printing comments
that were marked nonprinting or omitting print-only marks. A visible Square appearance with
`F=4` was rendered in the Windows qualification fixture; other appearance types still need
their own fidelity checks.

Native Windows PDF submission checks the driver's validated copy count and explicit
simplex/duplex and grayscale requests before sending. Collation must be retained when both
the page count and copy count exceed one. A driver that drops or changes these options is
refused; select supported options or flatten grayscale into the print-ready page content.
Without an explicit duplex request, the queue default applies. Normal color output retains
the driver's color policy. Single-page or single-copy jobs have no collating order to check.

`cargo test -p craft-print` tests CUPS transport (including pipe-sized jobs and refusal),
printer parsing, PPD options and argument mapping, and on every platform the Windows pieces that
need no printer: the PrintTicket, the checks on the driver's validated ticket and capabilities,
paper matching, job file names, and the PDF sheet reader and its update (classic and compressed
cross-references, turning, reframing, refusal, and thousands of truncated and corrupted files
that must fail without a panic). On Windows it also submits a mixed-orientation exact-size job
and a shrink-to-fit job to a temporary paused queue (inbox "Generic / Text Only" driver on
`LPT1:`) and checks each arrives as one job with all its sheets. The queue portion explicitly
skips if the driver or administrative rights are absent. Windows CI installs the inbox test
driver and sets `CRAFT_PRINT_REQUIRE_QUEUE=1`, making queue preparation/submission mandatory on
its runner. Two ignored tests check a real printer by hand: `an_installed_driver_validates_the_ticket`
(`CRAFT_PRINT_TEST_PRINTER`; sends no job) and `an_installed_printer_prints_a_file`
(`CRAFT_PRINT_TEST_PRINTER`, `CRAFT_PRINT_TEST_FILE`, optional `CRAFT_PRINT_TEST_SCALING=shrink`
and `CRAFT_PRINT_TEST_DUPLEX=long`). Physical printer fidelity remains a hardware check.

The cross-platform GitHub Actions workflow is defined in `.github/workflows/print.yml`.
It runs formatting, Clippy and this crate's own tests on Windows, Linux and macOS when print
code changes, and supports manual runs. The Windows job executes the spooler regression above;
physical printer fidelity still requires hardware verification.

Windows discovery reports driver duplex support in `Printer::two_sided` (a two-sided option in
the driver's PrintCapabilities); CUPS leaves it unknown. Explicit simplex/duplex choices that the driver
drops are refused before submission.

Windows dependencies: the maintained [`winprint` fork](https://github.com/storytold/winprint.rs)
(BSD-3-Clause, pinned to an immutable Git revision, its PDFium-downloading `pdfium` feature off) and `winsafe` (MIT) hold every Win32 and WinRT call, so the crate keeps `unsafe` forbidden;
`roxmltree` reads PrintTickets and `miniz_oxide` inflates PDF streams. `winprint` 0.2.1 lists
reqwest (with rustls) and sigstore crates as non-optional build-dependencies, used only by its
`pdfium` feature: they compile into its build script on Windows but not into the library. Apps
running `cargo deny` need a licence exception for `webpki-root-certs` (CDLA-Permissive-2.0, CA
list data reqwest brings) and an advisory ignore for RUSTSEC-2024-0370 (`proc-macro-error`
unmaintained, a compile-time proc macro of winprint's `fmt-derive`).

`craft_print::rgb` exposes the safe `craft-winprint-gdi` API for native driver settings,
printable geometry, RGB pages and caller-composed vector strokes. Applications retain ICC
conversion, document layout, labels and print-mark composition. The shared transport validates
single-copy DEVMODE settings before emitting manual copies and checks GDI errors. Color policy
is explicit: application-managed/unmanaged RGB disables DC and old-style halftone correction
with `ICM_DONE_OUTSIDEDC`, reapplied after `StartPage`; driver-managed RGB uses `ICM_OFF`.
Vendor driver automatic color correction remains a driver setting and requires qualification.
Other platforms return `rgb::PrintError::Unsupported`. The Windows PDF backend and its existing
completion dependency pin are independent of this additional MIT OR Apache-2.0 API.
