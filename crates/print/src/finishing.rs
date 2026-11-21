//! Finishing a job asks for: staple, hole punch and output bin. On CUPS they are the IPP job
//! attributes `finishings` (RFC 8011 §5.2.6, with the values PWG 5100.1 adds to the IANA IPP
//! registry) and `output-bin` (PWG 5100.2); on Windows they are Print Schema options
//! (`print_schema`), checked against what the driver lists.

#[cfg(any(test, not(target_arch = "wasm32")))]
use crate::Error;

#[cfg(any(test, windows))]
pub(crate) mod print_schema;
#[cfg(test)]
mod tests;

/// Where the staples go.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Staple {
    /// One staple, top-left corner.
    TopLeft,
    /// One staple, top-right corner.
    TopRight,
    /// One staple, bottom-left corner.
    BottomLeft,
    /// One staple, bottom-right corner.
    BottomRight,
    /// Two staples along the left edge.
    DualLeft,
    /// Two staples along the right edge.
    DualRight,
    /// Two staples along the top edge.
    DualTop,
    /// Two staples along the bottom edge.
    DualBottom,
    /// Saddle stitch: staples through the fold of a booklet.
    Saddle,
}

/// A sheet edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Edge {
    Left,
    Top,
    Right,
    Bottom,
}

/// Holes punched along one edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Punch {
    pub edge: Edge,
    /// 2, 3 or 4 holes; `None` leaves the count to the printer. CUPS only: a Windows job that
    /// names a count is refused, since the Print Schema has none. On CUPS `None` asks for two.
    pub holes: Option<u8>,
}

/// The finishing one job asks for. `Default` asks for none, and a field left `None` leaves that
/// choice to the printer's own settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Finishing {
    pub staple: Option<Staple>,
    pub punch: Option<Punch>,
    /// The output bin, by name: an IPP `output-bin` keyword on CUPS (`face-down`, `stacker-1`),
    /// and on Windows the name of one of the driver's output bins, matched without case.
    /// Letters, digits, `-`, `_` and `.` only, at most [`MAX_BIN`] bytes.
    pub output_bin: Option<String>,
}

/// The longest output bin name a job may give.
pub const MAX_BIN: usize = 255;

impl Finishing {
    /// Whether nothing is asked for.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Refuse a hole count other than 2, 3 or 4, and an output bin name that isn't a keyword:
    /// neither can be sent as asked, and a job is not sent without what it asked for.
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub(crate) fn check(&self) -> Result<(), Error> {
        if let Some(Punch { holes: Some(holes), .. }) = self.punch
            && !(2..=4).contains(&holes)
        {
            return Err(Error::Spool(format!("a punch of {holes} holes can't be asked for; choose 2, 3 or 4, or leave the count to the printer")));
        }
        if let Some(bin) = &self.output_bin
            && !is_bin_name(bin)
        {
            return Err(Error::Spool("the output bin's name can hold only letters, digits, '-', '_' and '.'".into()));
        }
        Ok(())
    }
}

/// A name `lp -o` and an IPP keyword can carry unquoted, and a Print Schema option name can match.
fn is_bin_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_BIN && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

impl Staple {
    /// The IPP `finishings` value.
    pub const fn ipp(self) -> i32 {
        match self {
            Self::Saddle => 8,
            Self::TopLeft => 20,
            Self::BottomLeft => 21,
            Self::TopRight => 22,
            Self::BottomRight => 23,
            Self::DualLeft => 28,
            Self::DualTop => 29,
            Self::DualRight => 30,
            Self::DualBottom => 31,
        }
    }
}

impl Punch {
    /// The IPP `finishings` value: the `punch-dual-*`, `punch-triple-*` or `punch-quad-*` value for
    /// the edge (each run is left, top, right, bottom), the dual one when no count is given.
    pub const fn ipp(self) -> i32 {
        let start = match self.holes {
            Some(3) => 78,
            Some(4) => 82,
            _ => 74,
        };
        start
            + match self.edge {
                Edge::Left => 0,
                Edge::Top => 1,
                Edge::Right => 2,
                Edge::Bottom => 3,
            }
    }
}

/// The IPP `finishings` values the job asks for, staple first; empty when it asks for none.
pub(crate) fn ipp_finishings(f: &Finishing) -> Vec<i32> {
    f.staple.map(Staple::ipp).into_iter().chain(f.punch.map(Punch::ipp)).collect()
}

/// The `lp -o` options for the finishing: `finishings` as registry values, comma-separated as
/// CUPS takes a set, and `output-bin`. A bin name that isn't a keyword is left out; [`Finishing::check`]
/// refuses such a job before `lp` runs.
pub(crate) fn lp_options(f: &Finishing) -> Vec<String> {
    let values = ipp_finishings(f);
    let mut out = Vec::new();
    if !values.is_empty() {
        out.push(format!("finishings={}", values.iter().map(i32::to_string).collect::<Vec<_>>().join(",")));
    }
    if let Some(bin) = f.output_bin.as_deref().filter(|b| is_bin_name(b)) {
        out.push(format!("output-bin={bin}"));
    }
    out
}

/// The finishing as `key=value` words for a preview, `staple=TopLeft`; empty when none is asked.
pub(crate) fn describe(f: &Finishing) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(staple) = f.staple {
        out.push(format!("staple={staple:?}"));
    }
    if let Some(punch) = f.punch {
        match punch.holes {
            Some(holes) => out.push(format!("punch={:?} ({holes} holes)", punch.edge)),
            None => out.push(format!("punch={:?}", punch.edge)),
        }
    }
    if let Some(bin) = &f.output_bin {
        out.push(format!("output-bin={bin}"));
    }
    out
}
