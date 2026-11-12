//! Judging a response: status codes (RFC 9110 §15), problem details (RFC 9457, which obsoletes RFC 7807 and
//! which JMAP's request-level errors use, RFC 8620 §3.6.1), content types (RFC 9110 §8.3) and JSON (RFC 8259).

use serde_json::{Map, Value};

use crate::error::{Error, LimitName, RequestProblem, Unusable};
use crate::http::{HttpRequest, HttpResponse, PROBLEM_BODY_LIMIT};
use crate::redact::{Excerpt, Redactor};
use crate::retry::RetryAfter;

/// The URN prefix of JMAP's request-level problem types (RFC 8620 §3.6.1, §9.3).
const JMAP_PROBLEM_PREFIX: &str = "urn:ietf:params:jmap:error:";
/// The longest JMAP problem name kept.
const MAX_PROBLEM_NAME_LEN: usize = 64;

/// Whether a `Content-Type` value names `expected` (RFC 9110 §8.3.1: type and subtype are case-insensitive;
/// parameters such as `charset` are ignored, JSON being UTF-8 by RFC 8259 §8.1).
pub(crate) fn media_type_is(content_type: Option<&str>, expected: &str) -> bool {
    content_type.is_some_and(|value| {
        let essence = value.split(';').next().unwrap_or("").trim_matches([' ', '\t']);
        essence.eq_ignore_ascii_case(expected)
    })
}

/// The most JSON values a body may hold before it is parsed into memory.
pub const MAX_JSON_VALUES: usize = 1_000_000;
/// The most JSON objects a body may hold before it is parsed into memory.
pub const MAX_JSON_OBJECTS: usize = 100_000;

/// Counts, without building anything, an upper bound of the values and the objects of a JSON text, so that a
/// small text cannot expand into a huge tree (RFC 8259 §9 lets a parser limit what it accepts; RFC 8620 §8.4).
/// Every value but the first follows `[`, `,` or `:`; strings are skipped with their escapes.
fn within_budget(body: &[u8]) -> bool {
    let mut in_string = false;
    let mut escaped = false;
    let mut values = 1usize;
    let mut objects = 0usize;
    for &b in body {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => objects = objects.saturating_add(1),
            b'[' | b',' | b':' => values = values.saturating_add(1),
            _ => {}
        }
        if values > MAX_JSON_VALUES || objects > MAX_JSON_OBJECTS {
            return false;
        }
    }
    true
}

/// Parses a JSON text (RFC 8259), skipping a leading byte order mark (RFC 8259 §8.1 allows ignoring one).
/// Nesting is bounded by `serde_json`'s recursion limit (128); the number of values and objects by
/// [`MAX_JSON_VALUES`] and [`MAX_JSON_OBJECTS`].
pub(crate) fn parse_json(body: &[u8]) -> Result<Value, Unusable> {
    let body = body.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(body);
    if !within_budget(body) {
        return Err(Unusable::TooComplex);
    }
    serde_json::from_slice(body).map_err(|_| Unusable::InvalidJson)
}

/// What was understood of a problem details object.
#[derive(Debug, Default)]
struct Problem {
    jmap: Option<RequestProblem>,
    status: Option<u16>,
    limit: Option<LimitName>,
    text: Option<String>,
    names_bad_token: bool,
}

fn jmap_problem(kind: &str) -> Option<RequestProblem> {
    let prefix = kind.get(..JMAP_PROBLEM_PREFIX.len())?;
    if !prefix.eq_ignore_ascii_case(JMAP_PROBLEM_PREFIX) {
        return None;
    }
    let name = kind.get(JMAP_PROBLEM_PREFIX.len()..)?;
    Some(match name {
        "unknownCapability" => RequestProblem::UnknownCapability,
        "notJSON" => RequestProblem::NotJson,
        "notRequest" => RequestProblem::NotRequest,
        "limit" => RequestProblem::Limit,
        other if !other.is_empty() && other.len() <= MAX_PROBLEM_NAME_LEN && other.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') => {
            RequestProblem::Other(other.to_owned())
        }
        _ => return None,
    })
}

/// Reads a problem details object (RFC 9457 §3.1). Members of the wrong type are ignored, as §3.1 requires.
fn read_problem(response: &HttpResponse, limit: usize) -> Option<Problem> {
    let body = response.body();
    if body.is_empty() || body.len() > PROBLEM_BODY_LIMIT.min(limit) {
        return None;
    }
    let content_type = response.content_type();
    if content_type.is_some() && !media_type_is(content_type, "application/problem+json") && !media_type_is(content_type, "application/json") {
        return None;
    }
    let value = parse_json(body).ok()?;
    let object: &Map<String, Value> = value.as_object()?;
    let mut problem = Problem::default();
    if let Some(kind) = object.get("type").and_then(Value::as_str) {
        problem.jmap = jmap_problem(kind);
        let last = kind.rsplit([':', '/', '#']).next().unwrap_or("");
        problem.names_bad_token = last.eq_ignore_ascii_case("invalid_token") || last.eq_ignore_ascii_case("insufficient_scope");
    }
    problem.status = object.get("status").and_then(Value::as_u64).and_then(|status| u16::try_from(status).ok());
    problem.limit = object.get("limit").and_then(Value::as_str).and_then(LimitName::new);
    problem.text = object.get("detail").and_then(Value::as_str).or_else(|| object.get("title").and_then(Value::as_str)).map(str::to_owned);
    Some(problem)
}

fn excerpt(problem: Option<&Problem>, redactor: &Redactor) -> Option<Excerpt> {
    let text = problem?.text.as_deref()?;
    let excerpt = redactor.excerpt(text);
    if excerpt.as_str().is_empty() { None } else { Some(excerpt) }
}

/// Judges a response to `request`.
///
/// `Ok(())` means the body may be read as the request's kind expects (for session and API requests: a 2xx
/// status, `application/json`, and a body within the size limit). Everything else becomes the [`Error`]
/// variant the mail client can act on. Server text appears only as an [`Excerpt`] cleaned by `redactor`.
pub fn judge_response(request: &HttpRequest, response: &HttpResponse, redactor: &Redactor) -> Result<(), Error> {
    let status = response.status();
    if (200..=299).contains(&status) {
        if response.body().len() > request.max_response_bytes() {
            return Err(Error::Unusable(Unusable::BodyTooLarge));
        }
        if request.kind().expects_json() && !media_type_is(response.content_type(), "application/json") {
            return Err(Error::Unusable(Unusable::WrongContentType));
        }
        return Ok(());
    }
    if (300..=399).contains(&status) {
        return Err(judge_redirect(request, response));
    }
    let problem = read_problem(response, request.max_response_bytes());
    let detail = excerpt(problem.as_ref(), redactor);
    let retry_after = || response.retry_after().and_then(RetryAfter::parse);
    // A server-chosen identifier is shown in messages: drop it when it echoes a secret.
    let jmap = problem.as_ref().and_then(|problem| problem.jmap.clone()).map(|jmap| match jmap {
        RequestProblem::Other(name) if redactor.mentions_secret(&name) => RequestProblem::Other(String::new()),
        other => other,
    });
    let limit = problem.as_ref().and_then(|problem| problem.limit.clone()).filter(|limit| !redactor.mentions_secret(limit.as_str()));
    // RFC 9457 §3.1.2: the status member tells what the origin server meant when an intermediary changed it.
    let says_unauthorised = problem.as_ref().is_some_and(|problem| problem.status == Some(401) || problem.names_bad_token);
    Err(match status {
        401 => Error::CredentialsRefused,
        403 if says_unauthorised => Error::CredentialsRefused,
        429 | 503 => Error::Busy { retry_after: retry_after() },
        413 => Error::TooLarge { limit },
        _ if jmap == Some(RequestProblem::Limit) => match limit {
            Some(limit) if limit.is_concurrency() => Error::Busy { retry_after: retry_after() },
            limit => Error::TooLarge { limit },
        },
        403 => Error::Forbidden { detail },
        404 | 410 => Error::NotFound,
        400..=499 if jmap.is_some() => Error::Refused { problem: jmap, detail },
        400 | 405 | 406 | 415 | 422 => Error::Refused { problem: None, detail },
        _ => Error::Http { status, detail },
    })
}

/// A redirect (RFC 9110 §15.4) is never followed here: its target is checked and handed to the caller.
fn judge_redirect(request: &HttpRequest, response: &HttpResponse) -> Error {
    let status = response.status();
    let Some(location) = response.location() else {
        return Error::Http { status, detail: None };
    };
    if !matches!(status, 301 | 302 | 303 | 307 | 308) {
        return Error::Http { status, detail: None };
    }
    let from = request.address();
    let target = match from.resolve(location) {
        Ok(target) => target,
        Err(error) => return Error::RedirectRefused(error),
    };
    if let Err(error) = from.check_link(&target) {
        return Error::RedirectRefused(error);
    }
    Error::Redirected { same_origin: from.same_origin(&target), target }
}

#[cfg(test)]
mod tests;
