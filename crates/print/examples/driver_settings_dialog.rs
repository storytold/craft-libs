//! Open a printer driver's own settings dialog for one job, then print a one-page Letter test
//! sheet with the settings chosen there (Windows).
//!
//! `cargo run -p craft-print --example driver_settings_dialog -- "Printer name"`
//!
//! Change an option in the dialog and choose OK to print the sheet. Cancel, or OK with nothing
//! changed, prints nothing: some drivers return OK from Cancel, so the two look the same. With
//! `--preview` the job is described instead of sent.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let printer = args.next().ok_or("usage: driver_settings_dialog <printer> [--preview]")?;
    let preview = args.next().as_deref() == Some("--preview");
    let mut job = craft_print::Job { printer: Some(printer), title: "Driver settings test".into(), ..craft_print::Job::default() };
    if !craft_print::driver_settings_dialog(&mut job, None)? {
        println!("canceled or unchanged: nothing was printed");
        return Ok(());
    }
    if let Some(settings) = &job.driver_settings {
        println!("settings for {}: {} bytes", settings.printer(), settings.dev_mode().as_bytes().len());
    }
    if preview {
        println!("{}", craft_print::command_preview(&job)?.join(" "));
        return Ok(());
    }
    println!("{}", craft_print::submit(&test_sheet(), &job)?);
    Ok(())
}

/// A one-page Letter PDF: a black square near each corner and a frame half an inch in.
fn test_sheet() -> Vec<u8> {
    let content = "0 0 0 rg 36 36 36 36 re f 540 36 36 36 re f 36 720 36 36 re f 540 720 36 36 re f 0 0 0 RG 2 w 36 36 540 720 re S";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Contents 4 0 R >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ];
    let mut pdf = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
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
    pdf
}
