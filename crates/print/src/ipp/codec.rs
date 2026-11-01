//! The RFC 8010 binary encoding: a strict, bounded reader and writer for IPP messages.
//!
//! A message is a version, an operation id (request) or status code (response), a request id, a
//! sequence of attribute groups, an end-of-attributes tag and then, for a request that carries a
//! document, the document's bytes. Values are typed ([`Value`]); collections (`media-size-supported`
//! and friends) are read as nested [`Value::Collection`]s.
//!
//! # What the reader refuses
//!
//! The reader is for bytes that came from a network device, so it never trusts a length. It refuses
//! (with a [`CodecError`], never a panic):
//!
//! * a message larger than [`Limits::max_message`] and a count of attributes, values or groups
//!   beyond the [`Limits`], so a hostile reply cannot make it allocate without bound;
//! * collections nested deeper than [`Limits::max_depth`] (the nesting is read with an explicit
//!   depth counter);
//! * a delimiter tag that is not a group tag, a value outside a group, an additional value
//!   (`name-length` 0) with nothing to add to, a missing end-of-attributes tag;
//! * a value whose length does not fit its syntax (an `integer` is four bytes, a `boolean` one, a
//!   `dateTime` eleven, ...), a token that is not valid UTF-8, an attribute name that is not
//!   printable ASCII;
//! * a truncated message anywhere.
//!
//! A value tag this module has no name for is kept as [`Value::Other`] (the bytes are not
//! interpreted), so that a printer's extension does not make an otherwise good reply unreadable.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The size and shape caps of the reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The largest message, document bytes included.
    pub max_message: usize,
    /// The most groups.
    pub max_groups: usize,
    /// The most attributes in the whole message (collection members count).
    pub max_attributes: usize,
    /// The most values in the whole message.
    pub max_values: usize,
    /// The most values of one attribute.
    pub max_values_per_attribute: usize,
    /// The deepest collection nesting.
    pub max_depth: usize,
}

impl Default for Limits {
    /// 4 MiB messages, 1024 groups, 16 384 attributes, 65 536 values (4096 per attribute), and
    /// collections nested six deep.
    fn default() -> Self {
        Self { max_message: 4 << 20, max_groups: 1024, max_attributes: 16_384, max_values: 65_536, max_values_per_attribute: 4096, max_depth: 6 }
    }
}

impl Limits {
    /// These limits with a different message cap.
    #[must_use]
    pub fn with_max_message(self, max_message: usize) -> Self {
        Self { max_message, ..self }
    }
}

/// Why a message could not be read or written.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CodecError {
    /// The message ends inside a field.
    Truncated { at: usize },
    /// A cap was hit; the text names which.
    TooLarge(&'static str),
    /// A tag in the group position that is not a group delimiter.
    BadGroupTag(u8),
    /// A value tag where none may stand (a delimiter inside a collection, a member name outside).
    BadValueTag(u8),
    /// A length that does not fit the syntax of the tag.
    BadLength { tag: u8, length: usize },
    /// An attribute name that is empty, too long or not printable ASCII.
    BadName,
    /// A value that is not valid for its syntax (invalid UTF-8 in a token, an out-of-range field).
    BadValue(&'static str),
    /// An attribute or a value outside any group, or an additional value with nothing before it.
    Misplaced(&'static str),
    /// The message has no end-of-attributes tag.
    NoEnd,
    /// Collections nested too deeply.
    TooDeep,
    /// A version this client does not speak.
    BadVersion { major: u8, minor: u8 },
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { at } => write!(f, "the IPP message ends early (at byte {at})"),
            Self::TooLarge(what) => write!(f, "the IPP message has too many {what}"),
            Self::BadGroupTag(tag) => write!(f, "{tag:#04x} is not an IPP attribute-group tag"),
            Self::BadValueTag(tag) => write!(f, "{tag:#04x} may not stand here"),
            Self::BadLength { tag, length } => write!(f, "a value of tag {tag:#04x} cannot be {length} bytes long"),
            Self::BadName => f.write_str("an IPP attribute name is empty, too long or not printable ASCII"),
            Self::BadValue(why) => write!(f, "an IPP value is invalid: {why}"),
            Self::Misplaced(what) => write!(f, "misplaced IPP {what}"),
            Self::NoEnd => f.write_str("the IPP message has no end-of-attributes tag"),
            Self::TooDeep => f.write_str("IPP collections are nested too deeply"),
            Self::BadVersion { major, minor } => write!(f, "IPP version {major}.{minor} is not supported"),
        }
    }
}

impl std::error::Error for CodecError {}

/// An attribute group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GroupTag {
    Operation,
    Job,
    Printer,
    Unsupported,
    Subscription,
    EventNotification,
    Resource,
    Document,
    System,
}

impl GroupTag {
    /// The delimiter tag that opens this group on the wire.
    pub const fn byte(self) -> u8 {
        match self {
            Self::Operation => 0x01,
            Self::Job => 0x02,
            Self::Printer => 0x04,
            Self::Unsupported => 0x05,
            Self::Subscription => 0x06,
            Self::EventNotification => 0x07,
            Self::Resource => 0x08,
            Self::Document => 0x09,
            Self::System => 0x0A,
        }
    }

    /// The group a delimiter tag opens; `None` for anything that is not a group tag (including
    /// end-of-attributes, `0x03`).
    pub const fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            0x01 => Self::Operation,
            0x02 => Self::Job,
            0x04 => Self::Printer,
            0x05 => Self::Unsupported,
            0x06 => Self::Subscription,
            0x07 => Self::EventNotification,
            0x08 => Self::Resource,
            0x09 => Self::Document,
            0x0A => Self::System,
            _ => return None,
        })
    }
}

/// The end-of-attributes tag.
const END: u8 = 0x03;

/// Value tags (RFC 8010 section 3.5.2 and the IANA registry).
mod tag {
    pub const UNSUPPORTED: u8 = 0x10;
    pub const UNKNOWN: u8 = 0x12;
    pub const NO_VALUE: u8 = 0x13;
    pub const NOT_SETTABLE: u8 = 0x15;
    pub const DELETE_ATTRIBUTE: u8 = 0x16;
    pub const ADMIN_DEFINE: u8 = 0x17;
    pub const INTEGER: u8 = 0x21;
    pub const BOOLEAN: u8 = 0x22;
    pub const ENUM: u8 = 0x23;
    pub const OCTET_STRING: u8 = 0x30;
    pub const DATE_TIME: u8 = 0x31;
    pub const RESOLUTION: u8 = 0x32;
    pub const RANGE: u8 = 0x33;
    pub const BEG_COLLECTION: u8 = 0x34;
    pub const TEXT_LANG: u8 = 0x35;
    pub const NAME_LANG: u8 = 0x36;
    pub const END_COLLECTION: u8 = 0x37;
    pub const TEXT: u8 = 0x41;
    pub const NAME: u8 = 0x42;
    pub const KEYWORD: u8 = 0x44;
    pub const URI: u8 = 0x45;
    pub const URI_SCHEME: u8 = 0x46;
    pub const CHARSET: u8 = 0x47;
    pub const NATURAL_LANGUAGE: u8 = 0x48;
    pub const MIME_MEDIA_TYPE: u8 = 0x49;
    pub const MEMBER_ATTR_NAME: u8 = 0x4A;
}

/// An RFC 2579 date and time (`dateTime`, 11 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub deci: u8,
    /// The offset from UTC is ahead (`+`) of UTC rather than behind (`-`).
    pub ahead: bool,
    pub offset_hours: u8,
    pub offset_minutes: u8,
}

/// Days from 1970-01-01 to the given civil date (proleptic Gregorian).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of a day count from 1970-01-01.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

impl DateTime {
    /// The time as seconds since the Unix epoch in UTC (the offset is applied).
    pub fn unix_seconds(&self) -> i64 {
        let days = days_from_civil(i64::from(self.year), i64::from(self.month), i64::from(self.day));
        let local = days * 86_400 + i64::from(self.hour) * 3600 + i64::from(self.minute) * 60 + i64::from(self.second);
        let offset = i64::from(self.offset_hours) * 3600 + i64::from(self.offset_minutes) * 60;
        if self.ahead { local - offset } else { local + offset }
    }

    /// As a wall-clock time; `None` before the epoch, or past what the platform's clock can hold
    /// (a printer's year 40000 is no reason to panic).
    pub fn to_system_time(&self) -> Option<SystemTime> {
        let secs = u64::try_from(self.unix_seconds()).ok()?;
        UNIX_EPOCH.checked_add(Duration::new(secs, u32::from(self.deci.min(9)) * 100_000_000))
    }

    /// The UTC date and time of a wall-clock time, to a tenth of a second (before the epoch is the epoch).
    pub fn from_system_time(time: SystemTime) -> Self {
        let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
        let secs = i64::try_from(since.as_secs()).unwrap_or(i64::MAX / 2);
        let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
        let rem = secs.rem_euclid(86_400);
        let small = |n: i64| u8::try_from(n).unwrap_or(0);
        Self {
            year: u16::try_from(year).unwrap_or(u16::MAX),
            month: small(month),
            day: small(day),
            hour: small(rem / 3600),
            minute: small(rem % 3600 / 60),
            second: small(rem % 60),
            deci: small(i64::from(since.subsec_millis() / 100)),
            ahead: true,
            offset_hours: 0,
            offset_minutes: 0,
        }
    }
}

/// One value, typed by its syntax.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Integer(i32),
    Boolean(bool),
    Enum(i32),
    OctetString(Vec<u8>),
    DateTime(DateTime),
    /// `cross-feed`, `feed` and `units` (3 is dots per inch, 4 dots per centimetre).
    Resolution {
        cross_feed: i32,
        feed: i32,
        units: i8,
    },
    Range {
        lower: i32,
        upper: i32,
    },
    TextWithLanguage {
        language: String,
        text: String,
    },
    NameWithLanguage {
        language: String,
        name: String,
    },
    Text(String),
    Name(String),
    Keyword(String),
    Uri(String),
    UriScheme(String),
    Charset(String),
    NaturalLanguage(String),
    MimeMediaType(String),
    /// A collection: member names and their (possibly several) values, in order.
    Collection(Vec<(String, Vec<Value>)>),
    /// The attribute is not supported (out-of-band, in the unsupported group).
    Unsupported,
    Unknown,
    NoValue,
    NotSettable,
    DeleteAttribute,
    AdminDefine,
    /// A syntax this module does not interpret; the tag and the bytes are kept.
    Other {
        tag: u8,
        bytes: Vec<u8>,
    },
}

impl Value {
    /// The integer of an `integer` or `enum`.
    pub fn as_i32(&self) -> Option<i32> {
        match self {
            Self::Integer(n) | Self::Enum(n) => Some(*n),
            _ => None,
        }
    }

    /// The value of a `boolean`.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// The text of any string syntax (keyword, uri, name, text, mime type, charset, ...); of a
    /// value with a language, the text without the language.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(s)
            | Self::Name(s)
            | Self::Keyword(s)
            | Self::Uri(s)
            | Self::UriScheme(s)
            | Self::Charset(s)
            | Self::NaturalLanguage(s)
            | Self::MimeMediaType(s) => Some(s),
            Self::TextWithLanguage { text, .. } => Some(text),
            Self::NameWithLanguage { name, .. } => Some(name),
            _ => None,
        }
    }

    /// The members of a `collection`, each a name and its values.
    pub fn as_collection(&self) -> Option<&[(String, Vec<Value>)]> {
        match self {
            Self::Collection(members) => Some(members),
            _ => None,
        }
    }

    /// An out-of-band value (no data).
    pub fn is_out_of_band(&self) -> bool {
        matches!(self, Self::Unsupported | Self::Unknown | Self::NoValue | Self::NotSettable | Self::DeleteAttribute | Self::AdminDefine)
    }
}

/// An attribute: a name and one or more values (1setOf).
#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub values: Vec<Value>,
}

impl Attribute {
    /// An attribute with one value. The name is checked when the message is written, not here.
    pub fn new(name: impl Into<String>, value: Value) -> Self {
        Self { name: name.into(), values: vec![value] }
    }

    /// An attribute with several values (a 1setOf); writing refuses an empty set.
    pub fn set(name: impl Into<String>, values: Vec<Value>) -> Self {
        Self { name: name.into(), values }
    }

    /// An `integer` attribute.
    pub fn integer(name: &str, n: i32) -> Self {
        Self::new(name, Value::Integer(n))
    }

    /// A `boolean` attribute.
    pub fn boolean(name: &str, b: bool) -> Self {
        Self::new(name, Value::Boolean(b))
    }

    /// An `enum` attribute (`print-quality`, `orientation-requested`, ...).
    pub fn enumeration(name: &str, n: i32) -> Self {
        Self::new(name, Value::Enum(n))
    }

    /// A `keyword` attribute.
    pub fn keyword(name: &str, k: &str) -> Self {
        Self::new(name, Value::Keyword(k.to_owned()))
    }

    /// A 1setOf `keyword` attribute, in the order given.
    pub fn keywords(name: &str, ks: &[&str]) -> Self {
        Self::set(name, ks.iter().map(|k| Value::Keyword((*k).to_owned())).collect())
    }

    /// A `uri` attribute.
    pub fn uri(name: &str, u: &str) -> Self {
        Self::new(name, Value::Uri(u.to_owned()))
    }

    /// A `name` attribute without a language (`requesting-user-name`, `job-name`).
    pub fn name_value(name: &str, n: &str) -> Self {
        Self::new(name, Value::Name(n.to_owned()))
    }

    /// A `text` attribute without a language.
    pub fn text(name: &str, t: &str) -> Self {
        Self::new(name, Value::Text(t.to_owned()))
    }

    /// The first value.
    pub fn first(&self) -> Option<&Value> {
        self.values.first()
    }

    /// The string values (every value of a string syntax).
    pub fn strings(&self) -> Vec<&str> {
        self.values.iter().filter_map(Value::as_str).collect()
    }

    /// The integer and enum values.
    pub fn integers(&self) -> Vec<i32> {
        self.values.iter().filter_map(Value::as_i32).collect()
    }
}

/// A group of attributes.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub tag: GroupTag,
    pub attributes: Vec<Attribute>,
}

impl Group {
    /// An empty group of this kind.
    pub fn new(tag: GroupTag) -> Self {
        Self { tag, attributes: Vec::new() }
    }

    /// Append `attribute`; names are not deduplicated.
    #[must_use]
    pub fn with(mut self, attribute: Attribute) -> Self {
        self.attributes.push(attribute);
        self
    }

    /// The first attribute called `name` in this group.
    pub fn get(&self, name: &str) -> Option<&Attribute> {
        self.attributes.iter().find(|a| a.name == name)
    }

    /// The first value of `name` as a string (any string syntax), if it has one.
    pub fn first_str(&self, name: &str) -> Option<&str> {
        self.get(name).and_then(Attribute::first).and_then(Value::as_str)
    }

    /// The first value of `name` as an integer (`integer` or `enum`).
    pub fn first_i32(&self, name: &str) -> Option<i32> {
        self.get(name).and_then(Attribute::first).and_then(Value::as_i32)
    }

    /// The first value of `name` as a boolean.
    pub fn first_bool(&self, name: &str) -> Option<bool> {
        self.get(name).and_then(Attribute::first).and_then(Value::as_bool)
    }

    /// Every string value of `name`; empty when the attribute is absent.
    pub fn strings(&self, name: &str) -> Vec<&str> {
        self.get(name).map(Attribute::strings).unwrap_or_default()
    }
}

/// One IPP message.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub major: u8,
    pub minor: u8,
    /// The operation id of a request or the status code of a response.
    pub code: u16,
    pub request_id: u32,
    pub groups: Vec<Group>,
    /// The bytes after the end-of-attributes tag: the document of Print-Job and Send-Document.
    pub data: Vec<u8>,
}

impl Message {
    /// A message with no groups and no document. `code` is the operation id of a request or the status
    /// code of a response.
    pub fn new(major: u8, minor: u8, code: u16, request_id: u32) -> Self {
        Self { major, minor, code, request_id, groups: Vec::new(), data: Vec::new() }
    }

    /// Append `group`; groups are written in the order they were added.
    #[must_use]
    pub fn with_group(mut self, group: Group) -> Self {
        self.groups.push(group);
        self
    }

    /// Set the document bytes that follow the end-of-attributes tag, replacing any already set.
    #[must_use]
    pub fn with_data(mut self, data: Vec<u8>) -> Self {
        self.data = data;
        self
    }

    /// The first group with this tag.
    pub fn group(&self, tag: GroupTag) -> Option<&Group> {
        self.groups.iter().find(|g| g.tag == tag)
    }

    /// Every group with this tag, in order.
    pub fn groups_of(&self, tag: GroupTag) -> impl Iterator<Item = &Group> {
        self.groups.iter().filter(move |g| g.tag == tag)
    }

    /// Write the message. Names longer than 255 bytes, values longer than 65535 bytes and
    /// collections nested deeper than the default [`Limits::max_depth`] are refused.
    pub fn encode(&self) -> Result<Vec<u8>, CodecError> {
        let mut out = Vec::with_capacity(64 + self.data.len());
        out.extend_from_slice(&[self.major, self.minor]);
        out.extend_from_slice(&self.code.to_be_bytes());
        out.extend_from_slice(&self.request_id.to_be_bytes());
        for group in &self.groups {
            out.push(group.tag.byte());
            for attribute in &group.attributes {
                check_name(&attribute.name)?;
                if attribute.values.is_empty() {
                    return Err(CodecError::Misplaced("attribute without a value"));
                }
                for (index, value) in attribute.values.iter().enumerate() {
                    let name = if index == 0 { attribute.name.as_str() } else { "" };
                    write_value(&mut out, name, value, 0)?;
                }
            }
        }
        out.push(END);
        out.extend_from_slice(&self.data);
        Ok(out)
    }

    /// Read a message. See the module documentation for what is refused.
    pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Self, CodecError> {
        if bytes.len() > limits.max_message {
            return Err(CodecError::TooLarge("bytes"));
        }
        let mut reader = Reader { bytes, pos: 0, attributes: 0, values: 0, limits };
        let major = reader.u8()?;
        let minor = reader.u8()?;
        if !(1..=2).contains(&major) {
            return Err(CodecError::BadVersion { major, minor });
        }
        let code = reader.u16()?;
        let request_id = reader.u32()?;
        let mut message = Message::new(major, minor, code, request_id);
        let mut current: Option<Group> = None;
        loop {
            let tag = reader.u8().map_err(|_| CodecError::NoEnd)?;
            match tag {
                END => break,
                0x00 | 0x0B..=0x0F => return Err(CodecError::BadGroupTag(tag)),
                0x01..=0x0A => {
                    if let Some(done) = current.take() {
                        message.groups.push(done);
                    }
                    let Some(group) = GroupTag::from_byte(tag) else { return Err(CodecError::BadGroupTag(tag)) };
                    if message.groups.len() >= limits.max_groups {
                        return Err(CodecError::TooLarge("groups"));
                    }
                    current = Some(Group::new(group));
                }
                _ => {
                    let Some(group) = current.as_mut() else { return Err(CodecError::Misplaced("value outside a group")) };
                    reader.attribute_or_value(tag, group)?;
                }
            }
        }
        if let Some(done) = current.take() {
            message.groups.push(done);
        }
        message.data = bytes.get(reader.pos..).unwrap_or(&[]).to_vec();
        Ok(message)
    }
}

/// Refuse an attribute or member name that is empty, longer than 255 bytes, or not printable ASCII.
fn check_name(name: &str) -> Result<(), CodecError> {
    if name.is_empty() || name.len() > 255 || !name.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(CodecError::BadName);
    }
    Ok(())
}

/// Append `n` as a two-byte big-endian length; more than 65 535 is a [`CodecError::BadLength`].
fn put_len(out: &mut Vec<u8>, n: usize) -> Result<(), CodecError> {
    let n = u16::try_from(n).map_err(|_| CodecError::BadLength { tag: 0, length: n })?;
    out.extend_from_slice(&n.to_be_bytes());
    Ok(())
}

/// Append one encoded value: the tag, the name and the value, each length-prefixed.
fn put(out: &mut Vec<u8>, tag: u8, name: &str, value: &[u8]) -> Result<(), CodecError> {
    out.push(tag);
    put_len(out, name.len())?;
    out.extend_from_slice(name.as_bytes());
    put_len(out, value.len())?;
    out.extend_from_slice(value);
    Ok(())
}

/// The tag and text of a string-syntax value without a language; `None` for every other syntax.
fn string_tag(value: &Value) -> Option<(u8, &str)> {
    Some(match value {
        Value::Text(s) => (tag::TEXT, s),
        Value::Name(s) => (tag::NAME, s),
        Value::Keyword(s) => (tag::KEYWORD, s),
        Value::Uri(s) => (tag::URI, s),
        Value::UriScheme(s) => (tag::URI_SCHEME, s),
        Value::Charset(s) => (tag::CHARSET, s),
        Value::NaturalLanguage(s) => (tag::NATURAL_LANGUAGE, s),
        Value::MimeMediaType(s) => (tag::MIME_MEDIA_TYPE, s),
        _ => return None,
    })
}

/// Append `value` under `name` (empty for an additional value or a collection member's value).
/// Collections are written recursively and refused past the default depth cap.
fn write_value(out: &mut Vec<u8>, name: &str, value: &Value, depth: usize) -> Result<(), CodecError> {
    if !name.is_empty() {
        check_name(name)?;
    }
    if let Some((t, s)) = string_tag(value) {
        return put(out, t, name, s.as_bytes());
    }
    match value {
        Value::Integer(n) => put(out, tag::INTEGER, name, &n.to_be_bytes()),
        Value::Enum(n) => put(out, tag::ENUM, name, &n.to_be_bytes()),
        Value::Boolean(b) => put(out, tag::BOOLEAN, name, &[u8::from(*b)]),
        Value::OctetString(bytes) => put(out, tag::OCTET_STRING, name, bytes),
        Value::DateTime(d) => {
            let mut v = Vec::with_capacity(11);
            v.extend_from_slice(&d.year.to_be_bytes());
            v.extend_from_slice(&[
                d.month,
                d.day,
                d.hour,
                d.minute,
                d.second,
                d.deci,
                if d.ahead { b'+' } else { b'-' },
                d.offset_hours,
                d.offset_minutes,
            ]);
            put(out, tag::DATE_TIME, name, &v)
        }
        Value::Resolution { cross_feed, feed, units } => {
            let mut v = Vec::with_capacity(9);
            v.extend_from_slice(&cross_feed.to_be_bytes());
            v.extend_from_slice(&feed.to_be_bytes());
            v.extend_from_slice(&units.to_be_bytes());
            put(out, tag::RESOLUTION, name, &v)
        }
        Value::Range { lower, upper } => {
            let mut v = Vec::with_capacity(8);
            v.extend_from_slice(&lower.to_be_bytes());
            v.extend_from_slice(&upper.to_be_bytes());
            put(out, tag::RANGE, name, &v)
        }
        Value::TextWithLanguage { language, text } => put(out, tag::TEXT_LANG, name, &with_language(language, text)?),
        Value::NameWithLanguage { language, name: n } => put(out, tag::NAME_LANG, name, &with_language(language, n)?),
        Value::Collection(members) => {
            if depth >= Limits::default().max_depth {
                return Err(CodecError::TooDeep);
            }
            put(out, tag::BEG_COLLECTION, name, &[])?;
            for (member, values) in members {
                check_name(member)?;
                if values.is_empty() {
                    return Err(CodecError::Misplaced("collection member without a value"));
                }
                put(out, tag::MEMBER_ATTR_NAME, "", member.as_bytes())?;
                for v in values {
                    write_value(out, "", v, depth + 1)?;
                }
            }
            put(out, tag::END_COLLECTION, "", &[])
        }
        Value::Unsupported => put(out, tag::UNSUPPORTED, name, &[]),
        Value::Unknown => put(out, tag::UNKNOWN, name, &[]),
        Value::NoValue => put(out, tag::NO_VALUE, name, &[]),
        Value::NotSettable => put(out, tag::NOT_SETTABLE, name, &[]),
        Value::DeleteAttribute => put(out, tag::DELETE_ATTRIBUTE, name, &[]),
        Value::AdminDefine => put(out, tag::ADMIN_DEFINE, name, &[]),
        Value::Other { tag, bytes } => {
            if *tag < 0x10 {
                return Err(CodecError::BadValueTag(*tag));
            }
            put(out, *tag, name, bytes)
        }
        // The string syntaxes were written above.
        _ => Err(CodecError::BadValue("unwritable value")),
    }
}

/// The value bytes of a `textWithLanguage` or `nameWithLanguage`: the language and the text, each
/// length-prefixed.
fn with_language(language: &str, text: &str) -> Result<Vec<u8>, CodecError> {
    let mut v = Vec::with_capacity(4 + language.len() + text.len());
    put_len(&mut v, language.len())?;
    v.extend_from_slice(language.as_bytes());
    put_len(&mut v, text.len())?;
    v.extend_from_slice(text.as_bytes());
    Ok(v)
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    attributes: usize,
    values: usize,
    limits: &'a Limits,
}

impl Reader<'_> {
    /// The next `n` bytes, advancing past them; [`CodecError::Truncated`] if fewer remain.
    fn take(&mut self, n: usize) -> Result<&[u8], CodecError> {
        let end = self.pos.checked_add(n).ok_or(CodecError::Truncated { at: self.pos })?;
        let slice = self.bytes.get(self.pos..end).ok_or(CodecError::Truncated { at: self.pos })?;
        self.pos = end;
        Ok(slice)
    }

    /// The next byte.
    fn u8(&mut self) -> Result<u8, CodecError> {
        self.take(1)?.first().copied().ok_or(CodecError::Truncated { at: self.pos })
    }

    /// The next two bytes as a big-endian `u16`.
    fn u16(&mut self) -> Result<u16, CodecError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)]))
    }

    /// The next four bytes as a big-endian `u32`.
    fn u32(&mut self) -> Result<u32, CodecError> {
        let b = self.take(4)?;
        let mut a = [0u8; 4];
        a.copy_from_slice(b);
        Ok(u32::from_be_bytes(a))
    }

    /// Count one more value against [`Limits::max_values`].
    fn count_value(&mut self) -> Result<(), CodecError> {
        self.values += 1;
        if self.values > self.limits.max_values {
            return Err(CodecError::TooLarge("values"));
        }
        Ok(())
    }

    /// Count one more attribute against [`Limits::max_attributes`].
    fn count_attribute(&mut self) -> Result<(), CodecError> {
        self.attributes += 1;
        if self.attributes > self.limits.max_attributes {
            return Err(CodecError::TooLarge("attributes"));
        }
        Ok(())
    }

    /// The name-length, name, value-length and value after a tag.
    fn name_and_value(&mut self) -> Result<(String, &[u8]), CodecError> {
        let name_len = usize::from(self.u16()?);
        let name = if name_len == 0 {
            String::new()
        } else {
            if name_len > 255 {
                return Err(CodecError::BadName);
            }
            let raw = self.take(name_len)?;
            if !raw.iter().all(u8::is_ascii_graphic) {
                return Err(CodecError::BadName);
            }
            String::from_utf8_lossy(raw).into_owned()
        };
        let value_len = usize::from(self.u16()?);
        let value = self.take(value_len)?;
        Ok((name, value))
    }

    /// Read the value with tag `t` into `group`: a named value starts a new attribute, an unnamed one
    /// adds to the last attribute (refused if there is none). A `begCollection` reads the whole collection.
    fn attribute_or_value(&mut self, t: u8, group: &mut Group) -> Result<(), CodecError> {
        let (name, raw) = self.name_and_value()?;
        let raw = raw.to_vec();
        self.count_value()?;
        let value = if t == tag::BEG_COLLECTION {
            if !raw.is_empty() {
                return Err(CodecError::BadLength { tag: t, length: raw.len() });
            }
            Value::Collection(self.collection(1)?)
        } else {
            decode_value(t, &raw)?
        };
        if name.is_empty() {
            let attribute = group.attributes.last_mut().ok_or(CodecError::Misplaced("additional value with no attribute"))?;
            if attribute.values.len() >= self.limits.max_values_per_attribute {
                return Err(CodecError::TooLarge("values in one attribute"));
            }
            attribute.values.push(value);
        } else {
            self.count_attribute()?;
            group.attributes.push(Attribute { name, values: vec![value] });
        }
        Ok(())
    }

    /// The members of a collection whose `begCollection` was just read, up to its `endCollection`.
    fn collection(&mut self, depth: usize) -> Result<Vec<(String, Vec<Value>)>, CodecError> {
        if depth > self.limits.max_depth {
            return Err(CodecError::TooDeep);
        }
        let mut members: Vec<(String, Vec<Value>)> = Vec::new();
        loop {
            let t = self.u8()?;
            let (name, raw) = self.name_and_value()?;
            let raw = raw.to_vec();
            if !name.is_empty() {
                return Err(CodecError::Misplaced("named attribute inside a collection"));
            }
            match t {
                tag::END_COLLECTION => {
                    if !raw.is_empty() {
                        return Err(CodecError::BadLength { tag: t, length: raw.len() });
                    }
                    return Ok(members);
                }
                tag::MEMBER_ATTR_NAME => {
                    let member = std::str::from_utf8(&raw).map_err(|_| CodecError::BadValue("member name"))?;
                    check_name(member)?;
                    self.count_attribute()?;
                    members.push((member.to_owned(), Vec::new()));
                }
                0x00..=0x0F => return Err(CodecError::BadValueTag(t)),
                _ => {
                    self.count_value()?;
                    let value = if t == tag::BEG_COLLECTION {
                        if !raw.is_empty() {
                            return Err(CodecError::BadLength { tag: t, length: raw.len() });
                        }
                        Value::Collection(self.collection(depth + 1)?)
                    } else {
                        decode_value(t, &raw)?
                    };
                    let Some((_, values)) = members.last_mut() else {
                        return Err(CodecError::Misplaced("collection value before its member name"));
                    };
                    if values.len() >= self.limits.max_values_per_attribute {
                        return Err(CodecError::TooLarge("values in one attribute"));
                    }
                    values.push(value);
                }
            }
        }
    }
}

/// `raw` as a string; invalid UTF-8 is a [`CodecError::BadValue`] naming `what`.
fn utf8(raw: &[u8], what: &'static str) -> Result<String, CodecError> {
    std::str::from_utf8(raw).map(str::to_owned).map_err(|_| CodecError::BadValue(what))
}

/// The big-endian `i32` at `at`, with missing bytes read as zero (callers check the length first).
fn be_i32(raw: &[u8], at: usize) -> i32 {
    let mut a = [0u8; 4];
    for (slot, byte) in a.iter_mut().zip(raw.iter().skip(at)) {
        *slot = *byte;
    }
    i32::from_be_bytes(a)
}

/// Refuse a value of tag `tag` unless it is exactly `want` bytes long.
fn fixed(tag: u8, raw: &[u8], want: usize) -> Result<(), CodecError> {
    if raw.len() == want { Ok(()) } else { Err(CodecError::BadLength { tag, length: raw.len() }) }
}

/// A string preceded by its two-byte length at `at`; returns it and the offset after it.
fn counted(tag: u8, raw: &[u8], at: usize) -> Result<(&[u8], usize), CodecError> {
    let bad = CodecError::BadLength { tag, length: raw.len() };
    let head = raw.get(at..at + 2).ok_or(bad.clone())?;
    let n = usize::from(u16::from_be_bytes([head.first().copied().unwrap_or(0), head.get(1).copied().unwrap_or(0)]));
    let body = raw.get(at + 2..at + 2 + n).ok_or(bad)?;
    Ok((body, at + 2 + n))
}

/// Decode one value of tag `t`. Fixed-size syntaxes must have their exact length, tokens must be
/// UTF-8 (text and names are decoded lossily), collection delimiters are refused here, and an unknown
/// tag is kept as [`Value::Other`].
fn decode_value(t: u8, raw: &[u8]) -> Result<Value, CodecError> {
    Ok(match t {
        tag::INTEGER | tag::ENUM => {
            fixed(t, raw, 4)?;
            let n = be_i32(raw, 0);
            if t == tag::INTEGER { Value::Integer(n) } else { Value::Enum(n) }
        }
        tag::BOOLEAN => {
            fixed(t, raw, 1)?;
            match raw.first() {
                Some(0) => Value::Boolean(false),
                Some(1) => Value::Boolean(true),
                _ => return Err(CodecError::BadValue("boolean")),
            }
        }
        tag::OCTET_STRING => Value::OctetString(raw.to_vec()),
        tag::DATE_TIME => {
            fixed(t, raw, 11)?;
            let b = |i: usize| raw.get(i).copied().unwrap_or(0);
            let ahead = match b(8) {
                b'+' => true,
                b'-' => false,
                _ => return Err(CodecError::BadValue("dateTime offset direction")),
            };
            if !(1..=12).contains(&b(2)) || !(1..=31).contains(&b(3)) || b(4) > 23 || b(5) > 59 || b(6) > 60 || b(7) > 9 {
                return Err(CodecError::BadValue("dateTime field out of range"));
            }
            Value::DateTime(DateTime {
                year: u16::from_be_bytes([b(0), b(1)]),
                month: b(2),
                day: b(3),
                hour: b(4),
                minute: b(5),
                second: b(6),
                deci: b(7),
                ahead,
                offset_hours: b(9),
                offset_minutes: b(10),
            })
        }
        tag::RESOLUTION => {
            fixed(t, raw, 9)?;
            let units = i8::from_be_bytes([raw.get(8).copied().unwrap_or(0)]);
            Value::Resolution { cross_feed: be_i32(raw, 0), feed: be_i32(raw, 4), units }
        }
        tag::RANGE => {
            fixed(t, raw, 8)?;
            Value::Range { lower: be_i32(raw, 0), upper: be_i32(raw, 4) }
        }
        tag::TEXT_LANG | tag::NAME_LANG => {
            let (lang, next) = counted(t, raw, 0)?;
            let (text, end) = counted(t, raw, next)?;
            if end != raw.len() {
                return Err(CodecError::BadLength { tag: t, length: raw.len() });
            }
            let language = utf8(lang, "language")?;
            let text = String::from_utf8_lossy(text).into_owned();
            if t == tag::TEXT_LANG { Value::TextWithLanguage { language, text } } else { Value::NameWithLanguage { language, name: text } }
        }
        // Text a person reads may be sloppy in the field; do not refuse a reply over it.
        tag::TEXT => Value::Text(String::from_utf8_lossy(raw).into_owned()),
        tag::NAME => Value::Name(String::from_utf8_lossy(raw).into_owned()),
        tag::KEYWORD => Value::Keyword(utf8(raw, "keyword")?),
        tag::URI => Value::Uri(utf8(raw, "uri")?),
        tag::URI_SCHEME => Value::UriScheme(utf8(raw, "uriScheme")?),
        tag::CHARSET => Value::Charset(utf8(raw, "charset")?),
        tag::NATURAL_LANGUAGE => Value::NaturalLanguage(utf8(raw, "naturalLanguage")?),
        tag::MIME_MEDIA_TYPE => Value::MimeMediaType(utf8(raw, "mimeMediaType")?),
        tag::UNSUPPORTED | tag::UNKNOWN | tag::NO_VALUE | tag::NOT_SETTABLE | tag::DELETE_ATTRIBUTE | tag::ADMIN_DEFINE => {
            fixed(t, raw, 0)?;
            match t {
                tag::UNSUPPORTED => Value::Unsupported,
                tag::UNKNOWN => Value::Unknown,
                tag::NO_VALUE => Value::NoValue,
                tag::NOT_SETTABLE => Value::NotSettable,
                tag::DELETE_ATTRIBUTE => Value::DeleteAttribute,
                _ => Value::AdminDefine,
            }
        }
        tag::END_COLLECTION | tag::MEMBER_ATTR_NAME | tag::BEG_COLLECTION => return Err(CodecError::BadValueTag(t)),
        _ => Value::Other { tag: t, bytes: raw.to_vec() },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default reader limits.
    fn limits() -> Limits {
        Limits::default()
    }

    /// A Get-Printer-Attributes-shaped request that uses every value syntax, for round trips.
    fn sample() -> Message {
        Message::new(1, 1, 0x000B, 7)
            .with_group(
                Group::new(GroupTag::Operation)
                    .with(Attribute::new("attributes-charset", Value::Charset("utf-8".into())))
                    .with(Attribute::new("attributes-natural-language", Value::NaturalLanguage("en".into())))
                    .with(Attribute::uri("printer-uri", "ipp://127.0.0.1:631/ipp/print"))
                    .with(Attribute::keywords("requested-attributes", &["printer-name", "copies-supported"])),
            )
            .with_group(
                Group::new(GroupTag::Job)
                    .with(Attribute::integer("copies", 2))
                    .with(Attribute::enumeration("print-quality", 4))
                    .with(Attribute::boolean("flag", true))
                    .with(Attribute::new("printer-resolution", Value::Resolution { cross_feed: 600, feed: 300, units: 3 }))
                    .with(Attribute::new("copies-supported", Value::Range { lower: 1, upper: 99 }))
                    .with(Attribute::new("note", Value::TextWithLanguage { language: "en".into(), text: "hello".into() }))
                    .with(Attribute::new("who", Value::NameWithLanguage { language: "en".into(), name: "me".into() }))
                    .with(Attribute::new("blob", Value::OctetString(vec![0, 1, 2])))
                    .with(Attribute::new(
                        "when",
                        Value::DateTime(DateTime {
                            year: 2026,
                            month: 3,
                            day: 4,
                            hour: 5,
                            minute: 6,
                            second: 7,
                            deci: 8,
                            ahead: false,
                            offset_hours: 2,
                            offset_minutes: 30,
                        }),
                    ))
                    .with(Attribute::set("empty", vec![Value::NoValue, Value::Unknown]))
                    .with(Attribute::new("nope", Value::Unsupported))
                    .with(Attribute::new("t", Value::Text("text".into())))
                    .with(Attribute::new("n", Value::Name("name".into())))
                    .with(Attribute::new("s", Value::UriScheme("ipp".into())))
                    .with(Attribute::new("m", Value::MimeMediaType("application/pdf".into())))
                    .with(Attribute::new("x", Value::Other { tag: 0x5A, bytes: vec![9, 9] })),
            )
            .with_group(Group::new(GroupTag::Printer).with(Attribute::set(
                "media-size-supported",
                vec![
                    Value::Collection(vec![
                        ("x-dimension".into(), vec![Value::Integer(21000)]),
                        ("y-dimension".into(), vec![Value::Range { lower: 100, upper: 29700 }]),
                    ]),
                    Value::Collection(vec![(
                        "nested".into(),
                        vec![Value::Collection(vec![("a".into(), vec![Value::Integer(1), Value::Integer(2)])])],
                    )]),
                ],
            )))
            .with_data(b"%PDF-1.4 data".to_vec())
    }

    #[test]
    fn a_message_round_trips_with_every_syntax() {
        let message = sample();
        let bytes = message.encode().unwrap();
        let back = Message::decode(&bytes, &limits()).unwrap();
        assert_eq!(back, message);
        assert_eq!(&bytes[..8], &[1, 1, 0, 0x0B, 0, 0, 0, 7]);
    }

    #[test]
    fn the_known_bytes_of_a_request_are_what_rfc_8010_says() {
        let message = Message::new(2, 0, 0x0002, 1)
            .with_group(Group::new(GroupTag::Operation).with(Attribute::new("attributes-charset", Value::Charset("utf-8".into()))));
        let mut want = vec![2, 0, 0, 2, 0, 0, 0, 1, 0x01, 0x47, 0, 18];
        want.extend_from_slice(b"attributes-charset");
        want.extend_from_slice(&[0, 5]);
        want.extend_from_slice(b"utf-8");
        want.push(0x03);
        assert_eq!(message.encode().unwrap(), want);
    }

    #[test]
    fn every_prefix_of_a_message_is_an_error_not_a_panic() {
        let bytes = sample().encode().unwrap();
        let data_start = bytes.len() - b"%PDF-1.4 data".len();
        for cut in 0..data_start {
            let err = Message::decode(&bytes[..cut], &limits());
            assert!(err.is_err(), "a message cut at {cut} of {data_start} must not decode");
        }
        assert!(Message::decode(&bytes, &limits()).is_ok());
    }

    #[test]
    fn single_byte_corruption_never_panics() {
        let bytes = sample().encode().unwrap();
        for i in 0..bytes.len() {
            for flip in [0x00u8, 0xFF, 0x80, 0x01] {
                let mut copy = bytes.clone();
                copy[i] ^= flip;
                let _ = Message::decode(&copy, &limits());
            }
        }
    }

    /// A version 1.1 header with operation 0 and request id 1.
    fn header() -> Vec<u8> {
        vec![1, 1, 0, 0, 0, 0, 0, 1]
    }

    /// One raw value: `tag`, then `name` and `value`, each with a two-byte length.
    fn attr(tag: u8, name: &str, value: &[u8]) -> Vec<u8> {
        let mut v = vec![tag];
        v.extend_from_slice(&(name.len() as u16).to_be_bytes());
        v.extend_from_slice(name.as_bytes());
        v.extend_from_slice(&(value.len() as u16).to_be_bytes());
        v.extend_from_slice(value);
        v
    }

    /// [`header`] followed by `parts`, with no end tag added.
    fn message(parts: &[&[u8]]) -> Vec<u8> {
        let mut v = header();
        for p in parts {
            v.extend_from_slice(p);
        }
        v
    }

    #[test]
    fn hostile_structure_is_refused_with_a_reason() {
        let l = limits();
        assert_eq!(Message::decode(&message(&[&[0x01]]), &l), Err(CodecError::NoEnd));
        for bad in [0x00u8, 0x0B, 0x0C, 0x0F] {
            assert_eq!(Message::decode(&message(&[&[bad], &[0x03]]), &l), Err(CodecError::BadGroupTag(bad)));
        }
        assert!(matches!(Message::decode(&message(&[&attr(0x21, "copies", &[0, 0, 0, 1]), &[0x03]]), &l), Err(CodecError::Misplaced(_))));
        assert!(matches!(Message::decode(&message(&[&[0x02], &attr(0x21, "", &[0, 0, 0, 1]), &[0x03]]), &l), Err(CodecError::Misplaced(_))));
        for (t, len) in [(0x21u8, 3usize), (0x21, 5), (0x22, 2), (0x23, 0), (0x31, 10), (0x32, 8), (0x33, 7), (0x13, 1)] {
            let e = Message::decode(&message(&[&[0x02], &attr(t, "x", &vec![1; len]), &[0x03]]), &l);
            assert!(matches!(e, Err(CodecError::BadLength { .. })), "tag {t:#x} length {len}: {e:?}");
        }
        assert!(Message::decode(&message(&[&[0x02], &attr(0x22, "x", &[2]), &[0x03]]), &l).is_err());
        assert!(Message::decode(&message(&[&[0x02], &attr(0x31, "x", &[7, 234, 13, 1, 0, 0, 0, 0, b'+', 0, 0]), &[0x03]]), &l).is_err());
        assert_eq!(Message::decode(&message(&[&[0x02], &attr(0x21, "co pies", &[0, 0, 0, 1]), &[0x03]]), &l), Err(CodecError::BadName));
        assert_eq!(Message::decode(&message(&[&[0x02], &attr(0x21, "caf\u{e9}", &[0, 0, 0, 1]), &[0x03]]), &l), Err(CodecError::BadName));
        assert!(Message::decode(&message(&[&[0x02], &attr(0x44, "x", &[0xFF, 0xFE]), &[0x03]]), &l).is_err());
        for (major, minor) in [(0u8, 9u8), (3, 0), (9, 9)] {
            let v = vec![major, minor, 0, 0, 0, 0, 0, 1, 0x03];
            assert_eq!(Message::decode(&v, &l), Err(CodecError::BadVersion { major, minor }));
        }
        assert!(Message::decode(&message(&[&[0x02], &attr(0x35, "x", &[0, 5, b'e', b'n']), &[0x03]]), &l).is_err());
        assert!(Message::decode(&message(&[&[0x02], &attr(0x35, "x", &[0, 2, b'e', b'n', 0, 9, b'a']), &[0x03]]), &l).is_err());
        assert!(Message::decode(&message(&[&[0x02], &attr(0x35, "x", &[0, 2, b'e', b'n', 0, 1, b'a', b'b']), &[0x03]]), &l).is_err());
    }

    #[test]
    fn lengths_that_run_past_the_end_are_truncation() {
        let mut v = message(&[&[0x02], &[0x21, 0, 1, b'x', 0xFF, 0xFF]]);
        v.extend_from_slice(&[0, 0]);
        assert!(matches!(Message::decode(&v, &limits()), Err(CodecError::Truncated { .. })));
    }

    /// A message whose job group holds one collection with `depth` more collections nested inside it.
    fn nested(depth: usize) -> Vec<u8> {
        let mut v = vec![0x02];
        v.extend_from_slice(&attr(0x34, "c", &[]));
        for _ in 0..depth {
            v.extend_from_slice(&attr(0x4A, "", b"m"));
            v.extend_from_slice(&attr(0x34, "", &[]));
        }
        for _ in 0..=depth {
            v.extend_from_slice(&attr(0x37, "", &[]));
        }
        v.push(0x03);
        message(&[&v])
    }

    #[test]
    fn nested_garbage_cannot_recurse_without_bound() {
        let l = limits();
        assert!(Message::decode(&nested(2), &l).is_ok());
        assert!(Message::decode(&nested(l.max_depth - 1), &l).is_ok());
        assert_eq!(Message::decode(&nested(l.max_depth + 1), &l), Err(CodecError::TooDeep));
        assert_eq!(Message::decode(&nested(50_000), &Limits::default().with_max_message(usize::MAX)), Err(CodecError::TooDeep));
        let open = message(&[&[0x02], &attr(0x34, "c", &[]), &attr(0x4A, "", b"m")]);
        assert!(Message::decode(&open, &l).is_err());
        let stray = message(&[&[0x02], &attr(0x37, "", &[]), &[0x03]]);
        assert!(Message::decode(&stray, &l).is_err());
        let named =
            message(&[&[0x02], &attr(0x34, "c", &[]), &attr(0x4A, "", b"m"), &attr(0x21, "oops", &[0, 0, 0, 1]), &attr(0x37, "", &[]), &[0x03]]);
        assert!(Message::decode(&named, &l).is_err());
        let early = message(&[&[0x02], &attr(0x34, "c", &[]), &attr(0x21, "", &[0, 0, 0, 1]), &attr(0x37, "", &[]), &[0x03]]);
        assert!(Message::decode(&early, &l).is_err());
        let delimiter = message(&[&[0x02], &attr(0x34, "c", &[]), &[0x01, 0, 0, 0, 0]]);
        assert!(Message::decode(&delimiter, &l).is_err());
        assert!(Message::decode(&message(&[&[0x02], &attr(0x34, "c", &[1]), &attr(0x37, "", &[]), &[0x03]]), &l).is_err());
    }

    #[test]
    fn counts_and_sizes_are_capped() {
        let tight = Limits { max_attributes: 10, max_values: 20, max_groups: 3, max_values_per_attribute: 4, ..Limits::default() };
        let many: Vec<u8> = (0..11).flat_map(|i| attr(0x21, &format!("a{i}"), &[0, 0, 0, 1])).collect();
        assert_eq!(Message::decode(&message(&[&[0x02], &many, &[0x03]]), &tight), Err(CodecError::TooLarge("attributes")));
        let mut values = attr(0x21, "a", &[0, 0, 0, 1]);
        for _ in 0..4 {
            values.extend_from_slice(&attr(0x21, "", &[0, 0, 0, 1]));
        }
        assert_eq!(Message::decode(&message(&[&[0x02], &values, &[0x03]]), &tight), Err(CodecError::TooLarge("values in one attribute")));
        let groups = message(&[&[0x01, 0x02, 0x04, 0x05], &[0x03]]);
        assert_eq!(Message::decode(&groups, &tight), Err(CodecError::TooLarge("groups")));
        let big = message(&[&[0x01], &[0x03], &[0u8; 100]]);
        assert_eq!(Message::decode(&big, &Limits::default().with_max_message(50)), Err(CodecError::TooLarge("bytes")));
        let tight_values = Limits { max_values: 5, ..Limits::default() };
        let ten: Vec<u8> = (0..10).flat_map(|i| attr(0x21, &format!("a{i}"), &[0, 0, 0, 1])).collect();
        assert_eq!(Message::decode(&message(&[&[0x02], &ten, &[0x03]]), &tight_values), Err(CodecError::TooLarge("values")));
    }

    #[test]
    fn the_writer_refuses_what_cannot_be_written() {
        let one = |a: Attribute| Message::new(1, 1, 2, 1).with_group(Group::new(GroupTag::Job).with(a)).encode();
        assert_eq!(one(Attribute::integer("", 1)), Err(CodecError::BadName));
        assert_eq!(one(Attribute::integer(&"a".repeat(256), 1)), Err(CodecError::BadName));
        assert_eq!(one(Attribute::integer("a b", 1)), Err(CodecError::BadName));
        assert!(matches!(one(Attribute::new("x", Value::OctetString(vec![0; 70_000]))), Err(CodecError::BadLength { .. })));
        assert!(one(Attribute::set("x", vec![])).is_err());
        assert!(one(Attribute::new("x", Value::Other { tag: 3, bytes: vec![] })).is_err());
        let mut deep = Value::Collection(vec![("m".into(), vec![Value::Integer(1)])]);
        for _ in 0..20 {
            deep = Value::Collection(vec![("m".into(), vec![deep])]);
        }
        assert_eq!(one(Attribute::new("x", deep)), Err(CodecError::TooDeep));
    }

    #[test]
    fn a_date_past_the_platforms_clock_is_none_not_a_panic() {
        let far = DateTime {
            year: 65535,
            month: 12,
            day: 31,
            hour: 23,
            minute: 59,
            second: 59,
            deci: 200,
            ahead: true,
            offset_hours: 0,
            offset_minutes: 0,
        };
        // Out of range on some platforms and representable on others: either way, no panic.
        let _ = far.to_system_time();
        assert!(UNIX_EPOCH.checked_add(Duration::from_secs(u64::MAX)).is_none());
    }

    #[test]
    fn dates_convert_to_the_epoch_and_back() {
        let t = DateTime { year: 1970, month: 1, day: 1, hour: 0, minute: 0, second: 0, deci: 0, ahead: true, offset_hours: 0, offset_minutes: 0 };
        assert_eq!(t.unix_seconds(), 0);
        let later =
            DateTime { year: 2026, month: 2, day: 28, hour: 23, minute: 59, second: 59, deci: 5, ahead: false, offset_hours: 1, offset_minutes: 0 };
        let back = DateTime::from_system_time(later.to_system_time().unwrap());
        assert_eq!((back.year, back.month, back.day, back.hour, back.minute, back.second, back.deci), (2026, 3, 1, 0, 59, 59, 5));
        let leap =
            DateTime { year: 2024, month: 2, day: 29, hour: 12, minute: 0, second: 0, deci: 0, ahead: true, offset_hours: 0, offset_minutes: 0 };
        assert_eq!(DateTime::from_system_time(leap.to_system_time().unwrap()).day, 29);
        assert_eq!(leap.unix_seconds(), 1_709_208_000);
        assert!(DateTime { year: 1969, ..t }.to_system_time().is_none());
    }

    #[test]
    fn accessors_read_values() {
        let m = sample();
        let g = m.group(GroupTag::Job).unwrap();
        assert_eq!(g.first_i32("copies"), Some(2));
        assert_eq!(g.first_i32("print-quality"), Some(4));
        assert_eq!(g.first_bool("flag"), Some(true));
        assert_eq!(g.first_str("note"), Some("hello"));
        assert_eq!(g.first_str("copies"), None);
        assert!(g.get("nope").unwrap().first().unwrap().is_out_of_band());
        let op = m.group(GroupTag::Operation).unwrap();
        assert_eq!(op.strings("requested-attributes"), ["printer-name", "copies-supported"]);
        assert_eq!(m.groups_of(GroupTag::Printer).count(), 1);
        let sizes = m.group(GroupTag::Printer).unwrap().get("media-size-supported").unwrap();
        assert_eq!(sizes.values.len(), 2);
        assert_eq!(sizes.values[0].as_collection().unwrap().len(), 2);
    }
}
