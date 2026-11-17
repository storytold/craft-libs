//! HTTP Archive (HAR 1.2) captures, read into typed records. No redaction happens here; callers
//! decide what to keep (see the analysis crate's web capture inspector).

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HarError {
    #[error("not valid JSON: {0}")]
    BadJson(String),
    #[error("not a HAR capture: {0}")]
    NotHar(&'static str),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Har {
    pub version: Option<String>,
    pub creator: Option<String>,
    pub pages: usize,
    pub entries: Vec<HarEntry>,
    pub declared_entries: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HarEntry {
    /// 0-based position in `log.entries`.
    pub ordinal: usize,
    pub started: Option<String>,
    pub time_ms: Option<f64>,
    pub resource_type: Option<String>,
    pub server_ip: Option<String>,
    pub request: HarRequest,
    pub response: HarResponse,
    /// blocked, dns, connect, send, wait, receive, ssl (negative values mean "not applicable").
    pub timings: Vec<(String, f64)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HarRequest {
    pub method: String,
    pub url: String,
    pub http_version: Option<String>,
    pub headers: Vec<(String, String)>,
    pub cookie_names: Vec<String>,
    pub post: Option<HarBody>,
    pub body_size: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HarBody {
    pub mime: Option<String>,
    pub text: Option<String>,
    /// Form parameter names (values are deliberately not surfaced by this type's users).
    pub params: Vec<(String, Option<String>)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HarResponse {
    pub status: i64,
    pub status_text: Option<String>,
    pub http_version: Option<String>,
    pub headers: Vec<(String, String)>,
    pub cookie_names: Vec<String>,
    pub redirect_url: Option<String>,
    pub content_size: Option<i64>,
    pub mime: Option<String>,
    pub text: Option<String>,
    /// `base64` when `text` is base64-encoded.
    pub encoding: Option<String>,
    pub body_size: Option<i64>,
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(String::from)
}

fn pairs(v: Option<&Value>) -> Vec<(String, String)> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|h| Some((h.get("name")?.as_str()?.to_string(), h.get("value").and_then(Value::as_str).unwrap_or_default().to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn names(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(|c| c.get("name")?.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// Parse a HAR document, keeping at most `max_entries` entries.
pub fn parse(json: &[u8], max_entries: usize) -> Result<Har, HarError> {
    let json = json.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(json);
    let v: Value = serde_json::from_slice(json).map_err(|e| HarError::BadJson(e.to_string()))?;
    let log = v.get("log").ok_or(HarError::NotHar("no \"log\" object"))?;
    let entries = log.get("entries").and_then(Value::as_array).ok_or(HarError::NotHar("no \"log.entries\" array"))?;
    let mut har = Har {
        version: s(log, "version"),
        creator: log.get("creator").map(|c| format!("{} {}", s(c, "name").unwrap_or_default(), s(c, "version").unwrap_or_default()).trim().to_string()),
        pages: log.get("pages").and_then(Value::as_array).map_or(0, Vec::len),
        declared_entries: entries.len(),
        ..Default::default()
    };
    for (i, e) in entries.iter().enumerate() {
        if har.entries.len() >= max_entries {
            har.truncated = true;
            break;
        }
        let req = e.get("request").unwrap_or(&Value::Null);
        let res = e.get("response").unwrap_or(&Value::Null);
        let content = res.get("content").unwrap_or(&Value::Null);
        let post = req.get("postData").filter(|p| p.is_object()).map(|p| HarBody {
            mime: s(p, "mimeType"),
            text: s(p, "text"),
            params: p
                .get("params")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|x| Some((x.get("name")?.as_str()?.to_string(), s(x, "value")))).collect())
                .unwrap_or_default(),
        });
        let timings =
            e.get("timings").and_then(Value::as_object).map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_f64()?))).collect()).unwrap_or_default();
        har.entries.push(HarEntry {
            ordinal: i,
            started: s(e, "startedDateTime"),
            time_ms: e.get("time").and_then(Value::as_f64),
            resource_type: s(e, "_resourceType"),
            server_ip: s(e, "serverIPAddress"),
            request: HarRequest {
                method: s(req, "method").unwrap_or_default(),
                url: s(req, "url").unwrap_or_default(),
                http_version: s(req, "httpVersion"),
                headers: pairs(req.get("headers")),
                cookie_names: names(req.get("cookies")),
                post,
                body_size: req.get("bodySize").and_then(Value::as_i64),
            },
            response: HarResponse {
                status: res.get("status").and_then(Value::as_i64).unwrap_or(0),
                status_text: s(res, "statusText"),
                http_version: s(res, "httpVersion"),
                headers: pairs(res.get("headers")),
                cookie_names: names(res.get("cookies")),
                redirect_url: s(res, "redirectURL").filter(|u| !u.is_empty()),
                content_size: content.get("size").and_then(Value::as_i64),
                mime: s(content, "mimeType"),
                text: s(content, "text"),
                encoding: s(content, "encoding"),
                body_size: res.get("bodySize").and_then(Value::as_i64),
            },
            timings,
        });
    }
    Ok(har)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_har_with_a_utf8_bom() {
        let capture = br#"{"log":{"entries":[]}}"#;
        let mut encoded = vec![0xef, 0xbb, 0xbf];
        encoded.extend_from_slice(capture);
        assert_eq!(parse(&encoded, 1).unwrap(), parse(capture, 1).unwrap());
        assert!(parse(&[0xef, 0xbb], 1).is_err());
    }

    #[test]
    fn parses_minimal_har() {
        let j = br#"{"log":{"version":"1.2","creator":{"name":"x","version":"1"},"entries":[
          {"startedDateTime":"2026-01-01T00:00:00Z","time":12.5,
           "request":{"method":"POST","url":"https://api.example.test/v1/items/42","headers":[{"name":"Cookie","value":"s=1"}],"cookies":[{"name":"s","value":"1"}],
             "postData":{"mimeType":"application/json","text":"{\"a\":1}"}},
           "response":{"status":201,"headers":[],"content":{"size":2,"mimeType":"application/json","text":"{}"}},
           "timings":{"wait":10,"receive":2.5}}]}}"#;
        let h = parse(j, 10).unwrap();
        assert_eq!(h.entries.len(), 1);
        let e = &h.entries[0];
        assert_eq!(e.request.method, "POST");
        assert_eq!(e.response.status, 201);
        assert_eq!(e.request.cookie_names, vec!["s"]);
        assert_eq!(e.timings.len(), 2);
    }

    #[test]
    fn rejects_non_har() {
        assert!(parse(b"{}", 1).is_err());
        assert!(parse(b"nope", 1).is_err());
        assert!(parse(br#"{"log":{"entries":[1,null,"x"]}}"#, 10).is_ok());
    }
}
