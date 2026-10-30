//! The `payload` object of `link.open`.
//!
//! Formats are listed in `PROTOCOL.md` section 4. The server
//! calls [`OpenPayload::from_value`] on every `link.open` before it forwards the
//! request, and answers `payload_invalid` on an error. An app may call it again.
//!
//! This module uses only `std`, `serde_json`, [`crate::moniker`] and the size constants in
//! [`crate::wire`].

use serde_json::Value;

use crate::error::LinkError;
use crate::moniker::Moniker;
use crate::wire::MAX_LINK_PAYLOAD;

/// A parsed `link.open` payload.
#[derive(Debug, Clone, PartialEq)]
pub enum OpenPayload {
    /// Video: an OTIO JSON string.
    OtioJson { data: String },
    /// A still or vector: the producer document path and the asset moniker.
    ImageDoc { path: String, asset: Moniker },
}

impl OpenPayload {
    pub fn from_value(payload: &Value) -> Result<Self, LinkError> {
        let obj = payload.as_object().ok_or_else(|| LinkError::PayloadInvalid("payload must be an object".into()))?;
        let format = obj.get("format").and_then(Value::as_str).ok_or_else(|| LinkError::PayloadInvalid("payload.format must be a string".into()))?;
        match format {
            "otio-json" => {
                let data = text_field(obj, "data")?;
                if data.len() as u64 > MAX_LINK_PAYLOAD {
                    return Err(LinkError::TooLarge);
                }
                let missing = missing_media(&data)?;
                if !missing.is_empty() {
                    return Err(LinkError::MediaMissing { paths: missing });
                }
                Ok(Self::OtioJson { data })
            }
            "image-doc" => {
                let path = text_field(obj, "path")?;
                check_peer_path(&path, "payload.path")?;
                let asset = Moniker::parse(&text_field(obj, "asset")?).map_err(|err| LinkError::PayloadInvalid(format!("payload.asset: {err}")))?;
                Ok(Self::ImageDoc { path, asset })
            }
            other => Err(LinkError::PayloadInvalid(format!("payload.format {other:?} is not supported"))),
        }
    }
}

/// The most distinct files one OTIO document may name. A timeline with more is not an editor's
/// timeline, and each one costs a file system call.
pub const MAX_MEDIA_REFS: usize = 4096;
/// The most JSON values one OTIO document may hold. The line cap (4 MiB) allows about two million.
pub const MAX_NODES: usize = 1 << 20;
/// Deepest nesting walked. `serde_json` stops parsing at 128, so a deeper document never gets here.
pub const MAX_DEPTH: usize = 128;

/// Check a path that a peer names (`payload.path`, a media `target_url`). It must be absolute, hold
/// no NUL byte and no `..` component. The server does not canonicalise it: the app opens it, and a
/// symlink to a file elsewhere is a normal way to keep footage. This check only refuses a path
/// that is written to climb out of the directory it starts in.
pub fn check_peer_path(path: &str, what: &str) -> Result<(), LinkError> {
    let parsed = std::path::Path::new(path);
    if !parsed.is_absolute() {
        return Err(LinkError::PayloadInvalid(format!("{what} must be absolute")));
    }
    if path.contains('\0') {
        return Err(LinkError::PayloadInvalid(format!("{what} must not hold a NUL byte")));
    }
    if parsed.components().any(|part| matches!(part, std::path::Component::ParentDir)) {
        return Err(LinkError::PayloadInvalid(format!("{what} must not hold a `..` component")));
    }
    Ok(())
}

/// The files that `ExternalReference`s of an OTIO document name and that cannot be used. Each path
/// appears once, in document order.
///
/// A file is usable when it is a regular file (after symlinks) and opens for reading. A missing
/// file, a directory, a FIFO, a device and a file without read access all give the same answer, so
/// the reply says nothing beyond "this path from your payload does not work". The probe never opens
/// anything that is not a regular file, so a FIFO cannot hold the connection thread. Only paths the
/// payload itself names are probed, and at most [`MAX_MEDIA_REFS`] of them.
///
/// Empty when every file works, when `data` is not JSON (the app reports that), and for references
/// that are not an absolute path or a `file://` URL: they have no base here to resolve against.
/// `Err` when the document is too deep or too big, or names an absolute path with `..` or NUL.
pub fn missing_media(data: &str) -> Result<Vec<String>, LinkError> {
    let Ok(document) = serde_json::from_str::<Value>(data) else { return Ok(Vec::new()) };
    let mut walk = Walk { seen: Vec::new(), nodes: 0 };
    walk.collect(&document, 0)?;
    walk.seen.retain(|path| !media_usable(path));
    Ok(walk.seen)
}

fn media_usable(path: &str) -> bool {
    let path = std::path::Path::new(path);
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file()) && std::fs::File::open(path).is_ok()
}

struct Walk {
    seen: Vec<String>,
    nodes: usize,
}

impl Walk {
    fn collect(&mut self, value: &Value, depth: usize) -> Result<(), LinkError> {
        if depth > MAX_DEPTH {
            return Err(LinkError::PayloadInvalid(format!("the OTIO document nests deeper than {MAX_DEPTH}")));
        }
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(LinkError::PayloadInvalid(format!("the OTIO document holds more than {MAX_NODES} values")));
        }
        match value {
            Value::Object(map) => {
                let external = map.get("OTIO_SCHEMA").and_then(Value::as_str).is_some_and(|schema| schema.starts_with("ExternalReference"));
                if external && let Some(path) = map.get("target_url").and_then(Value::as_str).and_then(local_path) {
                    check_peer_path(&path, "target_url")?;
                    if !self.seen.contains(&path) {
                        if self.seen.len() >= MAX_MEDIA_REFS {
                            return Err(LinkError::PayloadInvalid(format!("the OTIO document names more than {MAX_MEDIA_REFS} media files")));
                        }
                        self.seen.push(path);
                    }
                }
                for child in map.values() {
                    self.collect(child, depth + 1)?;
                }
            }
            Value::Array(items) => {
                for child in items {
                    self.collect(child, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// The local path a `target_url` names: a `file://` URL (percent-decoded, host empty or
/// `localhost`) or an absolute path. `None` for anything else.
fn local_path(url: &str) -> Option<String> {
    let path = match url.strip_prefix("file://") {
        Some(rest) => {
            let rest = rest.strip_prefix("localhost").unwrap_or(rest);
            if !rest.starts_with('/') {
                return None;
            }
            percent_decode(rest)?
        }
        None => url.to_string(),
    };
    // `/C:/dir/a.mov` is how a Windows path sits in a URL.
    let path = match path.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() && cfg!(windows) => path[1..].to_string(),
        _ => path,
    };
    std::path::Path::new(&path).is_absolute().then_some(path)
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        if byte == b'%' {
            let pair = bytes.get(index + 1..index + 3)?;
            out.push(u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?);
            index += 3;
        } else {
            out.push(byte);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn text_field(obj: &serde_json::Map<String, Value>, key: &str) -> Result<String, LinkError> {
    match obj.get(key) {
        Some(Value::String(text)) if !text.is_empty() => Ok(text.clone()),
        _ => Err(LinkError::PayloadInvalid(format!("payload.{key} must be a non-empty string"))),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use serde_json::json;

    use super::*;

    const ASSET: &str = "craft://photocraft/550e8400-e29b-41d4-a716-446655440000/image/flat";

    #[test]
    fn parses_otio() {
        let got = OpenPayload::from_value(&json!({"format": "otio-json", "data": "{}"})).unwrap();
        assert_eq!(got, OpenPayload::OtioJson { data: "{}".into() });
    }

    fn otio_with(url: &str) -> String {
        json!({
            "OTIO_SCHEMA": "Timeline.1",
            "tracks": {"OTIO_SCHEMA": "Stack.1", "children": [{"OTIO_SCHEMA": "Track.1", "children": [{
                "OTIO_SCHEMA": "Clip.2",
                "media_references": {"DEFAULT_MEDIA": {"OTIO_SCHEMA": "ExternalReference.1", "target_url": url}},
                "active_media_reference_key": "DEFAULT_MEDIA"
            }]}]}
        })
        .to_string()
    }

    #[test]
    fn rejects_media_that_does_not_exist_with_the_paths() {
        let payload = json!({"format": "otio-json", "data": otio_with("file:///nowhere/My%20Clip.mov")});
        let err = OpenPayload::from_value(&payload).unwrap_err();
        assert_eq!(err, LinkError::MediaMissing { paths: vec!["/nowhere/My Clip.mov".to_string()] });
        assert_eq!(err.code(), "media_missing");
        assert_eq!(err.to_object().data.get("missing"), Some(&json!(["/nowhere/My Clip.mov"])));
        assert!(err.to_string().contains("/nowhere/My Clip.mov"), "{err}");
        // A bare absolute path counts the same.
        let bare = json!({"format": "otio-json", "data": otio_with("/nowhere/clip.mov")});
        assert!(matches!(OpenPayload::from_value(&bare), Err(LinkError::MediaMissing { .. })));
    }

    #[test]
    fn accepts_media_that_opens_and_references_it_cannot_resolve() {
        let dir = std::env::temp_dir().join(format!("craft-payload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("My Clip.mov");
        std::fs::write(&file, b"x").unwrap();
        let url = format!("file://{}", file.to_str().unwrap().replace(' ', "%20"));
        let good = json!({"format": "otio-json", "data": otio_with(&url)});
        assert!(OpenPayload::from_value(&good).is_ok());
        // A relative target has no base here. The app resolves it.
        let relative = json!({"format": "otio-json", "data": otio_with("clips/a.mov")});
        assert!(OpenPayload::from_value(&relative).is_ok());
        // A generator or a missing reference names no file.
        let none = json!({"format": "otio-json", "data": "{\"OTIO_SCHEMA\":\"GeneratorReference.1\"}"});
        assert!(OpenPayload::from_value(&none).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parses_image_doc() {
        let got = OpenPayload::from_value(&json!({"format": "image-doc", "path": "/tmp/a.photocraft", "asset": ASSET})).unwrap();
        let OpenPayload::ImageDoc { path, asset } = got else {
            panic!("wrong variant");
        };
        assert_eq!(path, "/tmp/a.photocraft");
        assert_eq!(asset.to_string(), ASSET);
    }

    fn payload_with(url: &str) -> Value {
        json!({"format": "otio-json", "data": otio_with(url)})
    }

    #[test]
    fn traversal_in_a_named_path_is_payload_invalid() {
        for bad in ["/tmp/../etc/passwd", "file:///tmp/a/../../etc/passwd", "file:///tmp/%2e%2e/etc/passwd", "/tmp/a\u{0}b", "file:///tmp/a%00b"] {
            let err = OpenPayload::from_value(&payload_with(bad)).unwrap_err();
            assert!(matches!(err, LinkError::PayloadInvalid(_)), "{bad:?}: {err:?}");
        }
        let err = OpenPayload::from_value(&json!({"format": "image-doc", "path": "/tmp/../etc/x.png", "asset": ASSET})).unwrap_err();
        assert!(matches!(err, LinkError::PayloadInvalid(_)), "{err:?}");
        // A relative target is the app's business, `..` or not.
        assert!(OpenPayload::from_value(&payload_with("clips/../a.mov")).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn only_regular_files_count_as_present_and_a_fifo_does_not_block() {
        // Forking `mkfifo` while another test holds a discovery lock would hand the child a copy of the lock fd.
        let _spawning = crate::SPAWN_GUARD.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = std::env::temp_dir().join(format!("craft-payload-fifo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fifo = dir.join("pipe.mov");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status().map(|s| s.success()).unwrap_or(false);
        assert!(made, "mkfifo failed");
        let real = dir.join("real.mov");
        std::fs::write(&real, b"x").unwrap();
        let link = dir.join("link.mov");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let start = std::time::Instant::now();
        let data = json!({"OTIO_SCHEMA": "Timeline.1", "refs": [
            {"OTIO_SCHEMA": "ExternalReference.1", "target_url": fifo.to_str().unwrap()},
            {"OTIO_SCHEMA": "ExternalReference.1", "target_url": dir.to_str().unwrap()},
            {"OTIO_SCHEMA": "ExternalReference.1", "target_url": link.to_str().unwrap()},
            {"OTIO_SCHEMA": "ExternalReference.1", "target_url": real.to_str().unwrap()},
        ]})
        .to_string();
        let missing = missing_media(&data).unwrap();
        assert!(start.elapsed() < std::time::Duration::from_secs(2), "the probe waited on a FIFO");
        assert_eq!(missing, vec![fifo.to_str().unwrap().to_string(), dir.to_str().unwrap().to_string()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn size_depth_and_count_are_bounded() {
        // Too many distinct media files.
        let refs: Vec<Value> =
            (0..=MAX_MEDIA_REFS).map(|n| json!({"OTIO_SCHEMA": "ExternalReference.1", "target_url": format!("/nowhere/{n}.mov")})).collect();
        let many = json!({"OTIO_SCHEMA": "Timeline.1", "refs": refs}).to_string();
        let err = OpenPayload::from_value(&json!({"format": "otio-json", "data": many})).unwrap_err();
        assert!(matches!(err, LinkError::PayloadInvalid(_)), "{err:?}");
        // The same file named many times counts once.
        let same: Vec<Value> =
            (0..MAX_MEDIA_REFS * 2).map(|_| json!({"OTIO_SCHEMA": "ExternalReference.1", "target_url": "/nowhere/a.mov"})).collect();
        let doc = json!({"OTIO_SCHEMA": "Timeline.1", "refs": same}).to_string();
        assert_eq!(missing_media(&doc).unwrap(), vec!["/nowhere/a.mov".to_string()]);
        // Too deep. serde_json refuses at 128, so the document reads as "not JSON" and the app reports it.
        let deep = format!("{}{}", "[".repeat(300), "]".repeat(300));
        assert_eq!(missing_media(&deep).unwrap(), Vec::<String>::new());
        // Just inside serde_json's limit the walker still finishes.
        let nested = format!("{}{}", "[".repeat(MAX_DEPTH - 1), "]".repeat(MAX_DEPTH - 1));
        assert_eq!(missing_media(&nested).unwrap(), Vec::<String>::new());
        // Too big.
        let big = format!("{{\"pad\":\"{}\"}}", "a".repeat(usize::try_from(MAX_LINK_PAYLOAD).unwrap()));
        let err = OpenPayload::from_value(&json!({"format": "otio-json", "data": big})).unwrap_err();
        assert_eq!(err, LinkError::TooLarge);
        assert_eq!(err.code(), "payload_too_large");
    }

    #[test]
    fn rejects_bad_payloads() {
        for bad in [
            json!("x"),
            json!({}),
            json!({"format": "otio-json"}),
            json!({"format": "otio-json", "data": ""}),
            json!({"format": "image-doc", "path": "rel.png", "asset": ASSET}),
            json!({"format": "image-doc", "path": "/a.png", "asset": "craft://bad"}),
            json!({"format": "image-doc", "asset": ASSET}),
            json!({"format": "svg", "data": "x"}),
        ] {
            let err = OpenPayload::from_value(&bad).unwrap_err();
            assert!(matches!(err, LinkError::PayloadInvalid(_)), "{bad}: {err:?}");
            assert_eq!(err.code(), "payload_invalid");
        }
    }
}
