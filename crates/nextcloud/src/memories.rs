//! The Memories app's timeline and EXIF metadata.

use serde_json::Value;

use crate::Error;
use crate::client::Client;
use crate::http::{Kind, read_capped};

const MAX_DAYS: u64 = 256 << 20;
const MAX_INFO: u64 = 1 << 20;
const BATCH: usize = 100;
/// Largest integer an `f64` holds exactly.
const EXACT: f64 = 9_007_199_254_740_992.0;

/// A photo on the Memories timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct MemoriesPhoto {
    pub fileid: u64,
    pub dayid: i64,
    /// Upright, full-sensor width in pixels (already swapped for a rotated photo).
    pub w: u32,
    /// Upright, full-sensor height in pixels.
    pub h: u32,
    pub etag: String,
    pub basename: String,
    pub size: u64,
    /// The capture instant in UTC, Unix seconds. Unlike [`RemoteMeta::captured`], this is a real instant.
    pub epoch: Option<i64>,
    pub mimetype: Option<String>,
}

/// EXIF details of one photo; whatever the server doesn't know is `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RemoteMeta {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    /// Seconds.
    pub exposure_time: Option<f64>,
    pub f_number: Option<f64>,
    pub iso: Option<u32>,
    /// Millimetres.
    pub focal_length: Option<f64>,
    pub orientation: Option<u16>,
    /// `datetaken`: the local wall-clock time of capture written as if it were UTC (Unix seconds, no zone), so it
    /// differs from [`MemoriesPhoto::epoch`] by the camera's UTC offset. Read it with a UTC calendar to get the
    /// wall-clock fields.
    pub captured: Option<i64>,
    /// Upright, full-sensor width in pixels (already swapped for a rotated photo).
    pub width: Option<u32>,
    /// Upright, full-sensor height in pixels.
    pub height: Option<u32>,
}

/// The Memories app on a server; see [`Client::memories`].
#[derive(Clone, Copy, Debug)]
pub struct Memories<'a> {
    client: &'a Client,
}

impl Client {
    /// The server's Memories app, or `None` if it isn't installed.
    pub fn memories(&self) -> Result<Option<Memories<'_>>, Error> {
        let memories = Memories { client: self };
        // A 200 that isn't JSON (an HTML page for an unknown route) means no Memories.
        match memories.get_bytes("describe", &mut MAX_INFO.clone()) {
            Ok(body) => Ok(serde_json::from_slice::<Value>(&body).ok().filter(|b| b.get("version").is_some()).map(|_| memories)),
            Err(Error::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl Memories<'_> {
    /// Every photo on the timeline, in server order. At most 256 MiB is read in total.
    pub fn days(&self) -> Result<Vec<MemoriesPhoto>, Error> {
        let mut budget = MAX_DAYS;
        let days = self.get("days", &mut budget)?;
        let ids: Vec<i64> = array(&days)?.iter().filter_map(|d| int(d.get("dayid")?)).collect();
        let mut photos = Vec::new();
        for batch in ids.chunks(BATCH) {
            let list = batch.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
            let rows = self.get(&format!("days/{list}"), &mut budget)?;
            photos.extend(array(&rows)?.iter().filter_map(photo));
        }
        Ok(photos)
    }

    /// EXIF metadata of the file with this id.
    pub fn info(&self, fileid: u64) -> Result<RemoteMeta, Error> {
        let v = self.get(&format!("image/info/{fileid}"), &mut MAX_INFO.clone())?;
        if !v.is_object() {
            return Err(Error::Malformed("image info isn't an object".into()));
        }
        let exif = v.get("exif").filter(|e| e.is_object()).unwrap_or(&Value::Null);
        let text = |k: &str| exif.get(k).and_then(Value::as_str).map(str::to_string);
        let float = |k: &str| exif.get(k).and_then(num);
        Ok(RemoteMeta {
            make: text("Make"),
            model: text("Model"),
            lens: text("LensModel"),
            exposure_time: float("ExposureTime"),
            f_number: float("FNumber"),
            iso: exif.get("ISO").and_then(int).and_then(|n| n.try_into().ok()),
            focal_length: float("FocalLength"),
            orientation: exif.get("Orientation").and_then(int).and_then(|n| n.try_into().ok()),
            captured: v.get("datetaken").and_then(int),
            width: v.get("w").and_then(int).and_then(|n| n.try_into().ok()),
            height: v.get("h").and_then(int).and_then(|n| n.try_into().ok()),
        })
    }

    /// GETs `api/<route>` as JSON, spending at most `*budget` bytes and deducting what it read.
    fn get(&self, route: &str, budget: &mut u64) -> Result<Value, Error> {
        serde_json::from_slice(&self.get_bytes(route, budget)?).map_err(|e| Error::Malformed(e.to_string()))
    }

    fn get_bytes(&self, route: &str, budget: &mut u64) -> Result<Vec<u8>, Error> {
        let http = &self.client.http;
        let url = format!("{}/index.php/apps/memories/api/{route}", http.base());
        let res = http.request("GET", &url, &[("OCS-APIRequest", "true")], None, Kind::Quick)?;
        let status = res.status().as_u16();
        if status != 200 {
            return Err(Error::Malformed(format!("unexpected status {status}")));
        }
        let bytes = read_capped(res.into_body(), *budget)?;
        *budget = budget.saturating_sub(bytes.len() as u64);
        Ok(bytes)
    }
}

fn array(v: &Value) -> Result<&Vec<Value>, Error> {
    v.as_array().ok_or_else(|| Error::Malformed("expected a JSON array".into()))
}

/// A row of `days/<ids>`; `None` without a usable `fileid` and `dayid`.
fn photo(row: &Value) -> Option<MemoriesPhoto> {
    let get = |k: &str| row.get(k);
    let small = |k: &str| get(k).and_then(int).and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
    let text = |k: &str| get(k).and_then(Value::as_str).map(str::to_string);
    Some(MemoriesPhoto {
        fileid: int(get("fileid")?).and_then(|n| u64::try_from(n).ok())?,
        dayid: int(get("dayid")?)?,
        w: small("w"),
        h: small("h"),
        etag: text("etag").unwrap_or_default(),
        basename: text("basename").unwrap_or_default(),
        size: get("size").and_then(int).and_then(|n| u64::try_from(n).ok()).unwrap_or(0),
        epoch: get("epoch").and_then(int),
        mimetype: text("mimetype"),
    })
}

/// A JSON number, a numeric string (`"35.0 mm"`) or a fraction (`"1/250"`) as a finite `f64`.
fn num(v: &Value) -> Option<f64> {
    let f = match v {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => {
            let s = s.trim();
            match s.parse::<f64>() {
                Ok(f) => f,
                Err(_) => {
                    let (a, rest) = leading(s)?;
                    match rest.strip_prefix('/') {
                        Some(den) => {
                            let b = leading(den.trim_start())?.0;
                            if b == 0.0 {
                                return None;
                            }
                            a / b
                        }
                        None => a,
                    }
                }
            }
        }
        _ => return None,
    };
    f.is_finite().then_some(f)
}

/// The longest leading `[+-]?digits[.digits]` of `s` and what follows it.
fn leading(s: &str) -> Option<(f64, &str)> {
    let b = s.as_bytes();
    let digits = |from: usize| b.get(from..).map_or(0, |t| t.iter().take_while(|c| c.is_ascii_digit()).count());
    let sign = usize::from(matches!(b.first(), Some(b'+' | b'-')));
    let mut end = sign + digits(sign);
    if end == sign {
        return None;
    }
    if b.get(end) == Some(&b'.') && digits(end + 1) > 0 {
        end += 1 + digits(end + 1);
    }
    Some((s.get(..end)?.parse().ok()?, s.get(end..)?))
}

/// An integral value in the range an `f64` holds exactly.
fn int(v: &Value) -> Option<i64> {
    if let Some(n) = v.as_i64() {
        return Some(n);
    }
    let f = num(v)?;
    (f.fract() == 0.0 && f.abs() <= EXACT).then_some(f as i64)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn num_parses_leniently() {
        assert_eq!(num(&json!("1/250")), Some(0.004));
        assert_eq!(num(&json!("35.0 mm")), Some(35.0));
        assert_eq!(num(&json!(" -2.5")), Some(-2.5));
        assert_eq!(num(&json!(2.8)), Some(2.8));
        assert_eq!(num(&json!("abc")), None);
        assert_eq!(num(&json!("1/0")), None);
        assert_eq!(num(&json!("1e400")), None);
        assert_eq!(num(&json!("9".repeat(400))), None);
        assert_eq!(num(&json!(null)), None);
    }

    #[test]
    fn int_rejects_fractions_and_huge() {
        assert_eq!(int(&json!("1719571872")), Some(1_719_571_872));
        assert_eq!(int(&json!(2.5)), None);
        assert_eq!(int(&json!(1e300)), None);
        assert_eq!(int(&json!(u64::MAX)), None);
    }

    #[test]
    fn bad_rows_are_skipped() {
        assert!(photo(&json!({"dayid": 1})).is_none());
        assert!(photo(&json!({"fileid": -1, "dayid": 1})).is_none());
        assert!(photo(&json!(5)).is_none());
        assert_eq!(photo(&json!({"fileid": 3, "dayid": 1, "w": -4})).map(|p| (p.fileid, p.w)), Some((3, 0)));
    }
}
