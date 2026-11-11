//! The fit plan: which sheets change, and how, to match the paper.

use super::*;

/// How sheets of another size than the paper are printed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Fit {
    /// Every sheet must be the paper's size (either way round).
    #[cfg(windows)]
    Exact,
    /// Sheets are shrunk (never enlarged) to fit inside `margin` points of the paper's edges, and
    /// centred on it.
    Shrink { margin: f64 },
}

/// A page's new version: its rotation, and for a page brought to the paper's size, the new
/// geometry.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Edit {
    pub(crate) sheet: usize,
    pub(crate) rotate: u32,
    pub(crate) frame: Option<Frame>,
}

/// A page reframed onto the paper: its new MediaBox (unrotated space), the matrix placing the
/// old content on it, and the old visible box, which stays the clip.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Frame {
    pub(crate) media: (f64, f64),
    pub(crate) matrix: [f64; 6],
    pub(crate) clip: [f64; 4],
}

fn close(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() <= SAME && (a.1 - b.1).abs() <= SAME
}

/// The edits that put every sheet on `paper` (width and height as printed, in points; its
/// orientation is the job's). A sheet of the other orientation turns a quarter: a landscape sheet
/// in a portrait job anticlockwise, as drivers put a landscape job on the paper, a portrait sheet
/// in a landscape job clockwise. `Exact` refuses a sheet of another size; `Shrink` reframes it.
pub(crate) fn plan(sheets: &[Sheet], paper: (f64, f64), fit: Fit) -> Result<Vec<Edit>, Error> {
    let (pw, ph) = paper;
    if !(pw.is_finite() && ph.is_finite() && pw > 0.0 && ph > 0.0) {
        return Err(Error::Spool("the printer reported a paper size that is not usable".into()));
    }
    let mut edits = Vec::new();
    for (i, sheet) in sheets.iter().enumerate() {
        let (w, h) = sheet.size();
        let turn = if pw > ph + SAME && h > w + SAME {
            90
        } else if ph > pw + SAME && w > h + SAME {
            270
        } else {
            0
        };
        let rotate = (sheet.rotate + turn) % 360;
        let (w, h) = if turn == 0 { (w, h) } else { (h, w) };
        let frame = match fit {
            #[cfg(windows)]
            Fit::Exact if close((w, h), paper) => None,
            #[cfg(windows)]
            Fit::Exact => {
                return Err(Error::Spool(format!(
                    "Windows prints one paper size per job: sheet {} is {:.2} × {:.2} in, the job's paper {:.2} × {:.2} in; print the sheets of each size separately",
                    i + 1,
                    w / 72.0,
                    h / 72.0,
                    pw / 72.0,
                    ph / 72.0
                )));
            }
            Fit::Shrink { margin } => {
                let margin = if margin.is_finite() && margin > 0.0 && 2.0 * margin < pw.min(ph) { margin } else { 0.0 };
                let scale = ((pw - 2.0 * margin) / w).min((ph - 2.0 * margin) / h).min(1.0);
                if scale >= 1.0 && close((w, h), paper) {
                    None
                } else {
                    let [x0, y0, x1, y1] = sheet.bounds;
                    let (cw, ch) = (x1 - x0, y1 - y0);
                    // The paper in the page's unrotated space.
                    let (uw, uh) = if rotate % 180 == 90 { (ph, pw) } else { (pw, ph) };
                    let tx = (uw - scale * cw) / 2.0 - scale * x0;
                    let ty = (uh - scale * ch) / 2.0 - scale * y0;
                    Some(Frame { media: (uw, uh), matrix: [scale, 0.0, 0.0, scale, tx, ty], clip: [x0, y0, cw, ch] })
                }
            }
        };
        if turn != 0 || frame.is_some() {
            edits.push(Edit { sheet: i, rotate, frame });
        }
    }
    Ok(edits)
}
