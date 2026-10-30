//! Bulk frames (contract section 6).
//!
//! Server side. An app implements [`FrameSource`]. [`FrameTickets`] adapts it
//! to the server's [`Frames`] trait: it mints the 32 hex tickets, checks every
//! `CFRQ` header, and checks every reply before it reaches the wire. The server
//! drops a ticket when its control connection closes, which calls
//! [`Frames::close_ticket`] and ends the source's session for that ticket.
//!
//! Client side. [`Client::frames_open`] asks for a ticket. [`BulkClient`] opens
//! the second connection, attaches the ticket, and requests frames.

use std::collections::HashMap;
use std::io::BufReader;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::InstanceRecord as Record;
use crate::auth::{encode_hex, is_lower_hex};
use crate::client::{Client, Transport, map_reply, open_reader, read_json, write_json};
use crate::error::LinkError;
use crate::server::{FrameOpen, Frames};
use crate::wire;

/// Largest `scale` a client may ask for in `frames.open`.
pub const SCALE_MAX: f64 = 16.0;

/// Pixel format of a frame. Both are premultiplied alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameFormat {
    Rgba8Premul,
    Rgba16fPremul,
}

impl FrameFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rgba8Premul => "rgba8_premul",
            Self::Rgba16fPremul => "rgba16f_premul",
        }
    }

    pub fn parse(text: &str) -> Result<Self, LinkError> {
        match text {
            "rgba8_premul" => Ok(Self::Rgba8Premul),
            "rgba16f_premul" => Ok(Self::Rgba16fPremul),
            other => Err(LinkError::BadParams(format!("unknown frame format {other}"))),
        }
    }
}

/// What a client asked for in `frames.open`.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameSpec {
    pub link_id: String,
    pub format: FrameFormat,
    pub scale: f64,
}

impl FrameSpec {
    pub fn from_params(params: &Map<String, Value>) -> Result<Self, LinkError> {
        let link_id = params
            .get("link_id")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| LinkError::BadParams("link_id must be a non-empty string".into()))?
            .to_string();
        let format =
            FrameFormat::parse(params.get("format").and_then(Value::as_str).ok_or_else(|| LinkError::BadParams("format is required".into()))?)?;
        let scale = match params.get("scale") {
            None => 1.0,
            Some(value) => value.as_f64().ok_or_else(|| LinkError::BadParams("scale must be a number".into()))?,
        };
        if !scale.is_finite() || scale <= 0.0 || scale > SCALE_MAX {
            return Err(LinkError::BadParams(format!("scale must be above 0 and at most {SCALE_MAX}")));
        }
        Ok(Self { link_id, format, scale })
    }
}

/// Size of the frames a ticket yields. `len` is `stride * h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameLayout {
    pub w: u32,
    pub h: u32,
    pub stride: u32,
    pub len: u64,
}

impl FrameLayout {
    fn check(&self) -> Result<(), LinkError> {
        if u64::from(self.stride) * u64::from(self.h) != self.len {
            return Err(LinkError::Invalid("frame layout: len is not stride * h".into()));
        }
        if self.len > wire::FRAME_PAYLOAD_MAX {
            return Err(LinkError::TooLarge);
        }
        Ok(())
    }
}

/// One `CFRQ` header.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameRequest {
    pub req: u64,
    pub time_s: f64,
    /// The epoch the client last saw. The source may answer with a newer one.
    pub epoch: u64,
    /// `[x, y, w, h]` in pixels, or `None` for the whole frame.
    pub region: Option<[u32; 4]>,
}

impl FrameRequest {
    pub fn from_header(header: &Value) -> Result<Self, LinkError> {
        let req = header.get("req").and_then(Value::as_u64).ok_or_else(|| LinkError::BadParams("req must be an unsigned integer".into()))?;
        let time_s = header
            .get("time_s")
            .and_then(Value::as_f64)
            .filter(|t| t.is_finite())
            .ok_or_else(|| LinkError::BadParams("time_s must be a finite number".into()))?;
        let epoch = header.get("epoch").and_then(Value::as_u64).ok_or_else(|| LinkError::BadParams("epoch must be an unsigned integer".into()))?;
        let region = match header.get("region") {
            None | Some(Value::Null) => None,
            Some(Value::Array(items)) if items.len() == 4 => {
                let mut out = [0u32; 4];
                for (slot, item) in out.iter_mut().zip(items) {
                    let number = item.as_u64().ok_or_else(|| LinkError::BadParams("region entries must be unsigned integers".into()))?;
                    *slot = u32::try_from(number).map_err(|_| LinkError::BadParams("region entry is out of range".into()))?;
                }
                Some(out)
            }
            Some(_) => return Err(LinkError::BadParams("region must be null or [x, y, w, h]".into())),
        };
        Ok(Self { req, time_s, epoch, region })
    }

    pub fn to_header(&self) -> Value {
        json!({"req": self.req, "time_s": self.time_s, "epoch": self.epoch, "region": self.region})
    }
}

/// A rendered frame from the source.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedFrame {
    /// The epoch this frame was rendered at.
    pub epoch: u64,
    pub w: u32,
    pub h: u32,
    pub stride: u32,
    pub format: FrameFormat,
    pub render_ms: f64,
    pub pixels: Vec<u8>,
}

/// The app side of bulk frames. Calls may come from several connection threads.
pub trait FrameSource: Send + Sync {
    /// Validate `spec` and report the frame layout. Errors become `frames.open` errors.
    fn open(&self, spec: &FrameSpec) -> Result<FrameLayout, LinkError>;
    /// Render one frame. The returned epoch must be at least `request.epoch`.
    fn render(&self, spec: &FrameSpec, request: &FrameRequest) -> Result<RenderedFrame, LinkError>;
    /// The ticket for `spec` is gone. Release anything held for it.
    fn close(&self, spec: &FrameSpec);
}

/// [`Frames`] over a [`FrameSource`]. Pass it as `ServerConfig::frames`.
pub struct FrameTickets {
    source: Arc<dyn FrameSource>,
    open: Mutex<HashMap<String, FrameSpec>>,
}

impl FrameTickets {
    pub fn new(source: Arc<dyn FrameSource>) -> Self {
        Self { source, open: Mutex::new(HashMap::new()) }
    }

    /// Count of tickets that are still valid.
    pub fn open_tickets(&self) -> usize {
        crate::server::lock(&self.open).len()
    }

    fn spec(&self, ticket: &str) -> Result<FrameSpec, LinkError> {
        crate::server::lock(&self.open).get(ticket).cloned().ok_or_else(|| LinkError::BadParams("unknown or closed ticket".into()))
    }
}

fn new_ticket() -> Result<String, LinkError> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(LinkError::io)?;
    Ok(encode_hex(&bytes))
}

impl Frames for FrameTickets {
    fn open(&self, params: &Map<String, Value>) -> Result<FrameOpen, LinkError> {
        let spec = FrameSpec::from_params(params)?;
        let layout = self.source.open(&spec)?;
        layout.check()?;
        let ticket = new_ticket()?;
        crate::server::lock(&self.open).insert(ticket.clone(), spec);
        Ok(FrameOpen { ticket, w: layout.w, h: layout.h, stride: layout.stride, len: layout.len })
    }

    fn render(&self, ticket: &str, header: &Value) -> Result<(Value, Vec<u8>), LinkError> {
        let spec = self.spec(ticket)?;
        let request = FrameRequest::from_header(header)?;
        let frame = self.source.render(&spec, &request)?;
        if frame.epoch < request.epoch {
            return Err(LinkError::NotReady(format!("source is at epoch {}, request wants {}", frame.epoch, request.epoch)));
        }
        let expect = u64::from(frame.stride) * u64::from(frame.h);
        if u64::try_from(frame.pixels.len()).ok() != Some(expect) {
            return Err(LinkError::Invalid("rendered payload is not stride * h bytes".into()));
        }
        let reply = json!({
            "req": request.req,
            "epoch": frame.epoch,
            "w": frame.w,
            "h": frame.h,
            "stride": frame.stride,
            "format": frame.format.as_str(),
            "render_ms": frame.render_ms,
        });
        Ok((reply, frame.pixels))
    }

    fn close_ticket(&self, ticket: &str) {
        let removed = crate::server::lock(&self.open).remove(ticket);
        if let Some(spec) = removed {
            self.source.close(&spec);
        }
    }
}

/// The `frames.open` reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameTicket {
    pub ticket: String,
    pub w: u32,
    pub h: u32,
    pub stride: u32,
    pub len: u64,
}

impl Client {
    /// `frames.open`. The ticket lives until this connection closes.
    pub fn frames_open(&mut self, link_id: &str, format: FrameFormat, scale: f64) -> Result<FrameTicket, LinkError> {
        let result = self.call("frames.open", json!({"link_id": link_id, "format": format.as_str(), "scale": scale}))?;
        let ticket = result
            .get("ticket")
            .and_then(Value::as_str)
            .filter(|t| t.len() == 32 && is_lower_hex(t))
            .ok_or_else(|| LinkError::Invalid("frames.open ticket is not 32 hex characters".into()))?;
        Ok(FrameTicket {
            ticket: ticket.to_string(),
            w: result_u32(&result, "w")?,
            h: result_u32(&result, "h")?,
            stride: result_u32(&result, "stride")?,
            len: result.get("len").and_then(Value::as_u64).ok_or_else(|| LinkError::Invalid("frames.open result has no len".into()))?,
        })
    }
}

fn result_u32(result: &Value, key: &str) -> Result<u32, LinkError> {
    result
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| LinkError::Invalid(format!("frames.open result has no {key}")))
}

/// One `CFRM` reply that carried pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub req: u64,
    /// The epoch the frame was rendered at. It can be newer than the epoch asked for.
    pub epoch: u64,
    pub w: u32,
    pub h: u32,
    pub stride: u32,
    pub format: FrameFormat,
    pub render_ms: f64,
    pub pixels: Vec<u8>,
}

/// The second connection of contract section 6. It is binary after the attach.
pub struct BulkClient {
    reader: BufReader<Transport>,
    next_req: u64,
    io_timeout: Duration,
}

impl BulkClient {
    /// Connect, authenticate, and `bulk.attach` with `ticket`.
    pub fn attach(record: &Record, ticket: &str) -> Result<Self, LinkError> {
        let mut reader = open_reader(record)?;
        write_json(&mut reader, &json!({"id": 1, "method": "bulk.attach", "params": {"ticket": ticket}}))?;
        let deadline = std::time::Instant::now() + Duration::from_secs(wire::REPLY_DEADLINE_SECS);
        let reply = read_json(&mut reader, deadline)?.ok_or(LinkError::Closed)?;
        map_reply(&reply)?;
        Ok(Self { reader, next_req: 1, io_timeout: Duration::from_secs(wire::REPLY_DEADLINE_SECS) })
    }

    /// How long one request may wait for the socket to take the request or give the reply.
    /// The default is [`wire::REPLY_DEADLINE_SECS`]. A request that runs out of time is
    /// `Err(Timeout)`, and the connection must then be dropped, because a reply that arrives
    /// late cannot be told apart from the next one.
    pub fn set_io_timeout(&mut self, timeout: Duration) {
        self.io_timeout = timeout;
    }

    /// Ask for one frame and wait for its reply.
    ///
    /// A reply with an `error` header is `Err(Invalid(message))`. The
    /// connection stays usable after it. A short or malformed record is an
    /// error and the connection must be dropped. [`BulkClient::request_reply`]
    /// tells the two apart.
    pub fn request(&mut self, time_s: f64, epoch: u64, region: Option<[u32; 4]>) -> Result<Frame, LinkError> {
        self.request_reply(time_s, epoch, region)?.map_err(LinkError::Invalid)
    }

    /// Like [`BulkClient::request`], with the two failures apart. The outer `Err` is the
    /// connection: drop it. The inner `Err` is the text of an `error` header from the server:
    /// the connection is still good.
    pub fn request_reply(&mut self, time_s: f64, epoch: u64, region: Option<[u32; 4]>) -> Result<Result<Frame, String>, LinkError> {
        if !time_s.is_finite() {
            return Err(LinkError::BadParams("time_s must be finite".into()));
        }
        let req = self.next_req;
        self.next_req = req.checked_add(1).ok_or_else(|| LinkError::Invalid("request id overflow".into()))?;
        let header = FrameRequest { req, time_s, epoch, region }.to_header();
        let transport = self.reader.get_ref();
        transport.arm_send_timeout(Some(self.io_timeout))?;
        transport.arm_recv_timeout(Some(self.io_timeout))?;
        wire::write_cfrq(self.reader.get_mut(), &header)?;
        let (reply, pixels) = wire::read_cfrm(&mut self.reader)?;
        if reply.get("req").and_then(Value::as_u64) != Some(req) {
            return Err(LinkError::Invalid("reply req does not match the request".into()));
        }
        if let Some(message) = reply.get("error") {
            return Ok(Err(message.as_str().unwrap_or("frame error").to_string()));
        }
        parse_reply(req, &reply, pixels).map(Ok)
    }
}

fn parse_reply(req: u64, reply: &Value, pixels: Vec<u8>) -> Result<Frame, LinkError> {
    let number = |key: &str| reply.get(key).and_then(Value::as_u64).ok_or_else(|| LinkError::Invalid(format!("frame header has no {key}")));
    let small = |key: &str| number(key).and_then(|n| u32::try_from(n).map_err(|_| LinkError::Invalid(format!("frame header {key} is out of range"))));
    let (w, h, stride) = (small("w")?, small("h")?, small("stride")?);
    let format =
        FrameFormat::parse(reply.get("format").and_then(Value::as_str).ok_or_else(|| LinkError::Invalid("frame header has no format".into()))?)?;
    if u64::try_from(pixels.len()).ok() != Some(u64::from(stride) * u64::from(h)) {
        return Err(LinkError::Invalid("payload is not stride * h bytes".into()));
    }
    let render_ms = reply.get("render_ms").and_then(Value::as_f64).ok_or_else(|| LinkError::Invalid("frame header has no render_ms".into()))?;
    Ok(Frame { req, epoch: number("epoch")?, w, h, stride, format, render_ms, pixels })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_header_round_trips() {
        let request = FrameRequest { req: 9, time_s: 2.5, epoch: 4, region: Some([1, 2, 3, 4]) };
        assert_eq!(FrameRequest::from_header(&request.to_header()).unwrap(), request);
        let whole = FrameRequest { req: 1, time_s: 0.0, epoch: 0, region: None };
        assert_eq!(FrameRequest::from_header(&whole.to_header()).unwrap(), whole);
    }

    #[test]
    fn bad_request_headers_are_rejected() {
        for header in [
            json!({"time_s": 1.0, "epoch": 0}),
            json!({"req": 1, "epoch": 0}),
            json!({"req": 1, "time_s": 1.0}),
            json!({"req": 1, "time_s": 1.0, "epoch": 0, "region": [1, 2, 3]}),
            json!({"req": 1, "time_s": 1.0, "epoch": 0, "region": [1, 2, 3, -4]}),
            json!({"req": -1, "time_s": 1.0, "epoch": 0}),
        ] {
            assert!(FrameRequest::from_header(&header).is_err(), "{header}");
        }
    }

    #[test]
    fn spec_rejects_bad_scale_and_format() {
        let ok = json!({"link_id": "a", "format": "rgba8_premul", "scale": 2.0});
        assert!(FrameSpec::from_params(ok.as_object().unwrap()).is_ok());
        for bad in [
            json!({"link_id": "a", "format": "rgba8_premul", "scale": 0.0}),
            json!({"link_id": "a", "format": "rgba8_premul", "scale": 17.0}),
            json!({"link_id": "a", "format": "bgra"}),
            json!({"format": "rgba8_premul"}),
        ] {
            assert!(FrameSpec::from_params(bad.as_object().unwrap()).is_err(), "{bad}");
        }
    }

    #[test]
    fn tickets_are_32_lower_hex_and_distinct() {
        let a = new_ticket().unwrap();
        let b = new_ticket().unwrap();
        assert_eq!(a.len(), 32);
        assert!(is_lower_hex(&a));
        assert_ne!(a, b);
    }
}
