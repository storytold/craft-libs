//! The Windows backend's tests: the printer list, the sheets read from a print-ready PDF and
//! the incremental update that gives them one shape (all platforms), and a job reaching a
//! paused test printer (Windows).

use super::*;

#[test]
fn windows_printers_are_sorted_once_with_their_default() {
    // Queue names hold spaces, commas, parentheses and non-ASCII letters; a connection can be
    // listed twice; the default is matched whole, never by prefix; whether the driver takes
    // two-sided settings travels with each name (unknown when its capabilities can't be read).
    let queues = [
        ("Zebra Label", Some(false)),
        ("HP LaserJet 400 (Copy 2)", Some(true)),
        ("Копир Canon iR", None),
        (r"\\print01\Office, 2nd floor", Some(true)),
        ("Zebra Label", Some(false)),
        (" ", None),
    ];
    let printers = spool::printer_list(queues.map(|(n, s)| (n.to_string(), s)), Some(r"\\print01\Office, 2nd floor"));
    assert_eq!(
        printers,
        [
            Printer { name: "HP LaserJet 400 (Copy 2)".into(), default: false, two_sided: Some(true) },
            Printer { name: "Zebra Label".into(), default: false, two_sided: Some(false) },
            Printer { name: r"\\print01\Office, 2nd floor".into(), default: true, two_sided: Some(true) },
            Printer { name: "Копир Canon iR".into(), default: false, two_sided: None },
        ]
    );
    assert!(spool::printer_list([("Office".to_string(), None)], Some("Office 2")).iter().all(|p| !p.default));
    assert!(spool::printer_list([("Office".to_string(), None)], None).iter().all(|p| !p.default));
}

/// One page of a test PDF: its MediaBox (`None` = inherited from the page tree's root, which is
/// Letter), CropBox and `/Rotate`.
#[derive(Clone, Copy)]
struct TestPage {
    media: Option<[f64; 4]>,
    crop: Option<[f64; 4]>,
    rotate: Option<i32>,
}

const LETTER: TestPage = TestPage { media: Some([0.0, 0.0, 612.0, 792.0]), crop: None, rotate: None };
const LETTER_LANDSCAPE: TestPage = TestPage { media: Some([0.0, 0.0, 792.0, 612.0]), crop: None, rotate: None };
const TABLOID: TestPage = TestPage { media: Some([0.0, 0.0, 792.0, 1224.0]), crop: None, rotate: None };

/// A print-ready PDF of `pages`, each with a small black square: a classic cross-reference
/// table, or (`compressed`) the pages inside a Flate object stream with a predicted
/// cross-reference stream, as modern writers save. The root page node sets a Letter MediaBox
/// for pages that inherit it.
fn test_pdf(pages: &[TestPage], compressed: bool) -> Vec<u8> {
    let n = pages.len();
    // 1 catalog, 2 page tree, 3 content, then the pages.
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 4 + i)).collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{}] /Count {n} /MediaBox [0 0 612 792] >>", kids.join(" ")),
        "<< /Length 26 >>\nstream\n0 0 0 rg 36 36 72 72 re f\nendstream".to_string(),
    ];
    let rect = |r: [f64; 4]| format!("[{} {} {} {}]", r[0], r[1], r[2], r[3]);
    for p in pages {
        let mut page = "<< /Type /Page /Parent 2 0 R /Resources << >> /Contents 3 0 R".to_string();
        if let Some(m) = p.media {
            page += &format!(" /MediaBox {}", rect(m));
        }
        if let Some(c) = p.crop {
            page += &format!(" /CropBox {}", rect(c));
        }
        if let Some(r) = p.rotate {
            page += &format!(" /Rotate {r}");
        }
        objects.push(page + " >>");
    }
    let mut pdf = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    if !compressed {
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f\r\n", offsets.len() + 1).as_bytes());
        for offset in &offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n\r\n").as_bytes());
        }
        pdf.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
        return pdf;
    }
    // Objects 1, 2 and the pages go in object stream `stm`; the content stream can't.
    let stm = objects.len() + 1;
    let inside: Vec<usize> = (1..=objects.len()).filter(|&i| i != 3).collect();
    let (mut header, mut body) = (String::new(), String::new());
    for &i in &inside {
        header += &format!("{i} {} ", body.len());
        body += &objects[i - 1];
        body += "\n";
    }
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(format!("{header}{body}").as_bytes(), 6);
    let content_at = pdf.len();
    pdf.extend_from_slice(format!("3 0 obj\n{}\nendobj\n", objects[2]).as_bytes());
    let stm_at = pdf.len();
    pdf.extend_from_slice(
        format!(
            "{stm} 0 obj\n<< /Type /ObjStm /N {} /First {} /Filter /FlateDecode /Length {} >>\nstream\n",
            inside.len(),
            header.len(),
            packed.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(&packed);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");
    let xref_num = stm + 1;
    let xref_at = pdf.len();
    // Rows of 1 + 2 + 1 bytes, PNG "Up"-predicted (filter byte 2), as Acrobat writes them.
    let mut rows: Vec<[u8; 4]> = vec![[0, 0, 0, 255]];
    for i in 1..=xref_num {
        rows.push(match i {
            3 => [1, (content_at >> 8) as u8, content_at as u8, 0],
            i if i == stm => [1, (stm_at >> 8) as u8, stm_at as u8, 0],
            i if i == xref_num => [1, (xref_at >> 8) as u8, xref_at as u8, 0],
            i => [2, 0, stm as u8, inside.iter().position(|&x| x == i).unwrap() as u8],
        });
    }
    let mut predicted = Vec::new();
    let mut prev = [0u8; 4];
    for row in rows {
        predicted.push(2);
        predicted.extend(row.iter().zip(prev).map(|(b, p)| b.wrapping_sub(p)));
        prev = row;
    }
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&predicted, 6);
    pdf.extend_from_slice(
        format!(
            "{xref_num} 0 obj\n<< /Type /XRef /Size {} /W [1 2 1] /Root 1 0 R /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 4 >> /Length {} >>\nstream\n",
            xref_num + 1,
            packed.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(&packed);
    pdf.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    pdf
}

/// `pdf` with `from` replaced by `to` (bytes, so the binary comment line survives; the same
/// length, unless after the last object, so the cross-reference offsets still hold).
fn replaced(pdf: &[u8], from: &str, to: &str) -> Vec<u8> {
    let at = pdf.windows(from.len()).position(|w| w == from.as_bytes()).unwrap();
    [&pdf[..at], to.as_bytes(), &pdf[at + from.len()..]].concat()
}

/// Each sheet's printed size (rounded to a point) and rotation.
fn shapes(pdf: &[u8]) -> Vec<((i64, i64), u32)> {
    let read = crate::sheets::read(pdf).unwrap();
    read.sheets.iter().map(|s| ((s.size().0.round() as i64, s.size().1.round() as i64), s.rotate)).collect()
}

#[test]
fn sheets_are_read_from_classic_and_compressed_files() {
    let pages = [
        LETTER,
        TestPage { media: None, crop: None, rotate: Some(90) },
        TestPage { media: Some([0.0, 0.0, 595.28, 841.89]), crop: Some([10.0, 20.0, 410.0, 320.0]), rotate: Some(-90) },
        TestPage { media: Some([0.0, 0.0, 612.0, 792.0]), crop: None, rotate: Some(45) },
    ];
    for compressed in [false, true] {
        assert_eq!(
            shapes(&test_pdf(&pages, compressed)),
            [((612, 792), 0), ((792, 612), 90), ((300, 400), 270), ((612, 792), 0)],
            "inherited MediaBox, CropBox, -90 is 270, 45 is not a rotation (compressed: {compressed})"
        );
    }
}

#[test]
fn mixed_orientations_turn_to_the_first_sheets() {
    use crate::sheets::{Fit, plan, read};
    for compressed in [false, true] {
        let pdf = test_pdf(&[LETTER, LETTER_LANDSCAPE, LETTER], compressed);
        let sheets = read(&pdf).unwrap();
        let edits = plan(&sheets.sheets, (612.0, 792.0), Fit::Exact).unwrap();
        assert_eq!(edits.len(), 1, "only the landscape sheet changes");
        let updated = sheets.update(&edits).unwrap();
        assert!(updated.starts_with(&pdf), "an incremental update: the original bytes stay as they were");
        assert_eq!(
            shapes(&updated),
            [((612, 792), 0), ((612, 792), 270), ((612, 792), 0)],
            "the landscape sheet turned a quarter anticlockwise (compressed: {compressed})"
        );
        let tail = String::from_utf8_lossy(&updated[pdf.len()..]).into_owned();
        assert!(tail.contains("/Contents 3 0 R"), "a turned sheet keeps its content: {tail}");
        assert_eq!(tail.contains("/Type /XRef"), compressed, "the update's cross-reference is the kind the file had: {tail}");
        // The updated file reads again, its /Prev chain followed, and needs nothing more.
        let again = read(&updated).unwrap();
        assert!(plan(&again.sheets, (612.0, 792.0), Fit::Exact).unwrap().is_empty());
    }
    // A landscape first sheet makes it a landscape job: the portrait sheets turn the other way.
    let pdf = test_pdf(&[LETTER_LANDSCAPE, TestPage { rotate: Some(180), ..LETTER }], false);
    let sheets = read(&pdf).unwrap();
    let updated = sheets.update(&plan(&sheets.sheets, (792.0, 612.0), Fit::Exact).unwrap()).unwrap();
    assert_eq!(shapes(&updated), [((792, 612), 0), ((792, 612), 270)], "180 + 90");
    // One shape already: nothing to do, and the job is the PDF as it came.
    let pdf = test_pdf(&[LETTER, LETTER], true);
    let sheets = read(&pdf).unwrap();
    assert!(plan(&sheets.sheets, (612.0, 792.0), Fit::Exact).unwrap().is_empty());
    assert_eq!(sheets.update(&[]).unwrap(), pdf);
}

#[test]
fn exact_size_refuses_a_sheet_of_another_size() {
    let pdf = test_pdf(&[LETTER, TestPage { media: Some([0.0, 0.0, 612.0, 1008.0]), ..LETTER }], false);
    let sheets = crate::sheets::read(&pdf).unwrap();
    let refused = crate::sheets::plan(&sheets.sheets, (612.0, 792.0), crate::sheets::Fit::Exact);
    assert!(matches!(refused, Err(Error::Spool(ref m)) if m.contains("sheet 2") && m.contains("one paper size")), "{refused:?}");
}

/// The `q … cm … re W n` line a reframed page's content starts with.
fn frame_of(updated: &[u8], original: usize) -> String {
    let tail = String::from_utf8_lossy(&updated[original..]).into_owned();
    tail.lines().find(|l| l.starts_with("q ")).unwrap_or_default().to_string()
}

#[test]
fn shrink_to_fit_reframes_sheets_on_the_paper() {
    use crate::sheets::{Fit, plan, read};
    // Tabloid onto Letter with an 18 pt margin: 756 / 1224 of the height wins, centred.
    let pdf = test_pdf(&[TABLOID], true);
    let sheets = read(&pdf).unwrap();
    let edits = plan(&sheets.sheets, (612.0, 792.0), Fit::Shrink { margin: 18.0 }).unwrap();
    let updated = sheets.update(&edits).unwrap();
    assert_eq!(shapes(&updated), [((612, 792), 0)]);
    assert_eq!(frame_of(&updated, pdf.len()), "q 0.6176 0 0 0.6176 61.4118 18 cm 0 0 792 1224 re W n");
    let tail = String::from_utf8_lossy(&updated[pdf.len()..]).into_owned();
    assert!(tail.contains("/Contents [7 0 R 3 0 R 8 0 R]"), "the page's own content between q … Q: {tail}");

    // A smaller sheet keeps its size, centred; a sheet with a CropBox is clipped to it, and the
    // crop is what lands centred.
    let card = TestPage { media: Some([0.0, 0.0, 360.0, 504.0]), ..LETTER };
    let cropped = TestPage { crop: Some([100.0, 100.0, 400.0, 500.0]), ..LETTER };
    let pdf = test_pdf(&[card, cropped], false);
    let sheets = read(&pdf).unwrap();
    let edits = plan(&sheets.sheets, (612.0, 792.0), Fit::Shrink { margin: 18.0 }).unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0].frame.as_ref().unwrap().matrix, [1.0, 0.0, 0.0, 1.0, 126.0, 144.0], "never enlarged");
    assert_eq!(edits[1].frame.as_ref().unwrap().matrix, [1.0, 0.0, 0.0, 1.0, 56.0, 96.0]);
    assert_eq!(edits[1].frame.as_ref().unwrap().clip, [100.0, 100.0, 300.0, 400.0]);
    assert_eq!(shapes(&sheets.update(&edits).unwrap()), [((612, 792), 0), ((612, 792), 0)]);

    // The paper's own size: shrunk into the printable area when the printer has margins, as
    // CUPS' auto-fit does, and left alone when it prints to the edge.
    let pdf = test_pdf(&[LETTER], false);
    let sheets = read(&pdf).unwrap();
    assert_eq!(plan(&sheets.sheets, (612.0, 792.0), Fit::Shrink { margin: 12.0 }).unwrap().len(), 1);
    assert!(plan(&sheets.sheets, (612.0, 792.0), Fit::Shrink { margin: 0.0 }).unwrap().is_empty());

    // Sheets of the other orientation turn first, then fit. A Tabloid page with /Rotate 90 is a
    // landscape sheet: turned back to 0 and shrunk; a landscape Tabloid page is turned to 270.
    let pdf = test_pdf(&[LETTER, TestPage { media: Some([0.0, 0.0, 1224.0, 792.0]), ..LETTER }, TestPage { rotate: Some(90), ..TABLOID }], true);
    let sheets = read(&pdf).unwrap();
    let edits = plan(&sheets.sheets, (612.0, 792.0), Fit::Shrink { margin: 0.0 }).unwrap();
    assert_eq!(edits.iter().map(|e| (e.sheet, e.rotate)).collect::<Vec<_>>(), [(1, 270), (2, 0)]);
    assert_eq!(edits[0].frame.as_ref().unwrap().media, (792.0, 612.0), "the paper in the page's unrotated space");
    assert_eq!(shapes(&sheets.update(&edits).unwrap()), [((612, 792), 0), ((612, 792), 270), ((612, 792), 0)]);
}

#[test]
fn an_encrypted_file_is_turned_but_not_reframed() {
    // An /Encrypt entry is all it takes: new content streams would have to be encrypted too.
    let pdf = test_pdf(&[LETTER, LETTER_LANDSCAPE], false);
    let encrypted = replaced(&pdf, "/Root 1 0 R >>", "/Root 1 0 R /Encrypt << /Filter /Standard >> >>");
    let sheets = crate::sheets::read(&encrypted).unwrap();
    let turned = crate::sheets::plan(&sheets.sheets, (612.0, 792.0), crate::sheets::Fit::Exact).unwrap();
    let updated = sheets.update(&turned).unwrap();
    assert_eq!(shapes(&updated), [((612, 792), 0), ((612, 792), 270)]);
    assert!(String::from_utf8_lossy(&updated[encrypted.len()..]).contains("/Encrypt"), "the new trailer keeps /Encrypt");
    let shrunk = crate::sheets::plan(&sheets.sheets, (612.0, 792.0), crate::sheets::Fit::Shrink { margin: 18.0 }).unwrap();
    assert!(matches!(sheets.update(&shrunk), Err(Error::Spool(ref m)) if m.contains("encrypted")));
}

#[test]
fn unreadable_sheets_are_errors_never_panics() {
    for bad in [&b""[..], b"not a pdf", b"%PDF-1.7\nstartxref\n999999\n%%EOF", b"%PDF-1.7\nxref\n0 1\ntrailer\n<< >>\nstartxref\n9\n%%EOF"] {
        assert!(crate::sheets::read(bad).is_err());
    }
    // A page tree that contains itself, and one without pages.
    let pdf = test_pdf(&[LETTER], false);
    let looped = replaced(&pdf, "/Kids [4 0 R]", "/Kids [2 0 R]");
    assert!(matches!(crate::sheets::read(&looped), Err(Error::Spool(ref m)) if m.contains("loops")));
    let empty = replaced(&pdf, "/Kids [4 0 R]", "/Kids [     ]");
    assert!(matches!(crate::sheets::read(&empty), Err(Error::Spool(ref m)) if m.contains("no pages")));
    // Every truncation and thousands of corruptions of both kinds of file: read and update what
    // is still readable, without panicking.
    for compressed in [false, true] {
        let pdf = test_pdf(&[LETTER, LETTER_LANDSCAPE, TABLOID], compressed);
        let mut variants: Vec<Vec<u8>> = (0..pdf.len()).map(|n| pdf[..n].to_vec()).collect();
        for (i, b) in [b'0', b'9', b'/', b'<', b'(', b']', b' ', 0xff].into_iter().cycle().take(4000).enumerate() {
            let mut v = pdf.clone();
            let at = (i * 7919) % v.len();
            v[at] = b;
            variants.push(v);
        }
        for v in variants {
            if let Ok(sheets) = crate::sheets::read(&v)
                && let Ok(edits) = crate::sheets::plan(&sheets.sheets, (612.0, 792.0), crate::sheets::Fit::Shrink { margin: 18.0 })
            {
                let _ = sheets.update(&edits);
            }
        }
    }
}

/// Windows: the printer list comes from the spooler. A bare machine may have no printer; the
/// list that exists has real names and at most one default.
#[cfg(windows)]
#[test]
fn windows_lists_the_printers_the_system_knows() {
    let printers = spool::printers();
    assert!(printers.iter().all(|p| !p.name.trim().is_empty()), "{printers:?}");
    assert!(printers.iter().filter(|p| p.default).count() <= 1, "{printers:?}");
}

/// Run a PowerShell script for the Windows test's printer setup: its stdout, or everything it
/// said when it failed (the test prints that as the skip reason). Any error stops the script and
/// fails it: `-Command` otherwise exits 0 after a failed command, such as a parameter the cmdlet
/// doesn't have. Started by its full path: given a bare name, Rust looks in the test binary's own
/// folder before System32. The library itself starts no PowerShell.
#[cfg(windows)]
fn windows_powershell(script: &str, printer: &str) -> Result<String, String> {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let exe = std::path::Path::new(&root).join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe");
    let mut c = std::process::Command::new(exe);
    c.args(["-NoProfile", "-NonInteractive", "-Command", &format!("$ErrorActionPreference = 'Stop'; $name = $env:CRAFT_TEST_PRINTER; {script}")]);
    c.env("CRAFT_TEST_PRINTER", printer);
    match c.output() {
        Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
        Ok(o) => Err(format!("{} {}", String::from_utf8_lossy(&o.stderr).trim(), String::from_utf8_lossy(&o.stdout).trim())),
        Err(e) => Err(e.to_string()),
    }
}

/// Windows, end to end: two jobs reach a paused test printer's queue, each as one job under its
/// title. One mixes portrait and landscape Letter sheets at exact size (the landscape sheet is
/// turned); the other is shrink-to-fit with a Tabloid sheet (reframed onto the driver's paper),
/// so both kinds of update pass through Windows.Data.Pdf. craft-print starts no program to print
/// them; the test uses PowerShell only to add, inspect and remove the printer. Its driver is the
/// inbox "Generic / Text Only"; a machine that cannot add one (no rights, no driver) skips the
/// test, unless `CRAFT_PRINT_REQUIRE_QUEUE=1` (CI) makes it mandatory.
#[cfg(windows)]
#[test]
fn windows_jobs_reach_the_spooler() {
    let name = format!("Craft Test Print {}", std::process::id());
    let name = name.as_str();
    // The printer goes on LPT1:, one of the ports every Windows install has, so the test adds no
    // port: Remove-PrinterPort can't remove a local port again (it reports an invalid name). Not
    // FILE:, which asks where to save; a paused FILE: queue never takes the job and the print call
    // waits for it. Paused through Win32_Printer, since Set-Printer can't pause a printer: the
    // job stays in the queue and nothing is sent to the port.
    let added = windows_powershell(
        "if (Get-Printer -Name $name -ErrorAction SilentlyContinue) { throw 'test queue already exists' }; Add-Printer -Name $name -DriverName 'Generic / Text Only' -PortName 'LPT1:'; $r = Invoke-CimMethod -InputObject (Get-CimInstance Win32_Printer | Where-Object Name -eq $name) -MethodName Pause; if ($r.ReturnValue -ne 0) { Remove-Printer -Name $name; throw ('Pause returned ' + $r.ReturnValue) }",
        name,
    );
    if let Err(why) = added {
        assert_ne!(std::env::var("CRAFT_PRINT_REQUIRE_QUEUE").as_deref(), Ok("1"), "CI requires a prepared paused printer: {why}");
        eprintln!("skipping the real print: the test printer could not be prepared: {why}");
        return;
    }
    let listed = spool::printers().into_iter().find(|p| p.name == name);
    let mixed = test_pdf(&[LETTER, LETTER_LANDSCAPE, LETTER], true);
    let job = Job { printer: Some(name.into()), copies: 2, duplex: Some(Duplex::LongEdge), title: "mixed.pdf".into(), ..Job::default() };
    let caller = std::thread::current().id();
    // Rc proves callers are not required to make their callback Send just because native
    // printing uses a COM worker. Counts describe PDF sheets, despite copies and duplex.
    let processed = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = std::rc::Rc::clone(&processed);
    let sent = spool::submit_with_progress(&mixed, &job, &mut move |done, total| {
        assert_eq!(std::thread::current().id(), caller);
        observed.borrow_mut().push((done, total));
    });
    let tabloid = test_pdf(&[TABLOID, LETTER], false);
    let shrink = Job { printer: Some(name.into()), scaling: Scaling::ShrinkToFit, title: "shrink".into(), ..Job::default() };
    let shrunk = spool::submit(&tabloid, &shrink);
    let mut refused_progress = Vec::new();
    let exact_refused = spool::submit_with_progress(&tabloid, &Job { printer: Some(name.into()), ..Job::default() }, &mut |done, total| {
        refused_progress.push((done, total));
    });
    let queued = windows_powershell(
        "@(Get-PrintJob -PrinterName $name | Sort-Object Id | ForEach-Object { $_.DocumentName + ' ' + $_.TotalPages }) -join '|'",
        name,
    );
    // A printer that still has jobs is only marked for deletion, so the queue is emptied first.
    // `exit 0`: `-Command` exits 1 when its last command failed, even silently.
    let cleaned = windows_powershell(
        "Get-PrintJob -PrinterName $name | Remove-PrintJob; foreach ($i in 1..20) { if (-not (Get-PrintJob -PrinterName $name)) { break }; Start-Sleep -Milliseconds 250 }; Remove-Printer -Name $name; if (Get-Printer -Name $name -ErrorAction SilentlyContinue) { throw 'the printer is still there' }; exit 0",
        name,
    );
    assert!(listed.as_ref().is_some_and(|p| p.two_sided.is_some()), "the new printer is listed, its duplex support known: {listed:?}");
    assert_eq!(sent.as_deref(), Ok("spooled 3 sheet(s)"), "{sent:?}");
    assert_eq!(*processed.borrow(), [(0, 3), (1, 3), (2, 3), (3, 3)]);
    assert_eq!(shrunk.as_deref(), Ok("spooled 2 sheet(s)"), "{shrunk:?}");
    assert!(matches!(exact_refused, Err(Error::Spool(ref m)) if m.contains("one paper size")), "{exact_refused:?}");
    assert!(refused_progress.is_empty(), "rejection before a job starts has no progress: {refused_progress:?}");
    assert_eq!(queued.as_deref().map(str::trim), Ok("mixed.pdf 3|shrink.pdf 2"), "one job each, all sheets, in the paused queue: {queued:?}");
    assert!(cleaned.is_ok(), "the test printer is removed again: {cleaned:?}");
    // Asking for a printer that isn't there is an error, never a job on another one.
    let missing = spool::submit(&mixed, &Job { printer: Some(name.into()), ..Job::default() });
    assert!(matches!(missing, Err(Error::Spool(ref m)) if m.contains(name)), "{missing:?}");
}
