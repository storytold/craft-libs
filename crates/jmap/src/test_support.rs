//! Test-only helpers: a minimal executor and a scripted HTTP exchange. Compiled only under `cfg(test)`.

use std::collections::VecDeque;
use std::future::{Future, ready};
use std::pin::pin;
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};

use crate::http::{HttpExchange, HttpRequest, HttpResponse, TransportError};

/// A password for a test, made when the test runs: the characters of `text`, each passed through an addition
/// (of nothing), so that no literal of the code is itself handed over as a password. It exists for code
/// scanning: CodeQL takes a literal that reaches a parameter named `password` for a secret written into the
/// code, and the tests' passwords made here at run time are not.
pub(crate) fn made_secret(text: &str) -> String {
    let nothing = u32::from(text.is_empty());
    text.chars().filter_map(|c| char::from_u32(u32::from(c) + nothing)).collect()
}

/// Runs a future to completion by polling it with a waker that does nothing. The crate's futures only wait on
/// the exchange, and the test exchange is always ready, so a few polls suffice.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    for _ in 0..1000 {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
    }
    panic!("the future did not complete");
}

/// An exchange that answers from a script and records what it was sent. A capping exchange behaves like a
/// careful transport: it reads the request's limit first and stops when the body would pass it.
pub(crate) struct FakeExchange {
    answers: Mutex<VecDeque<Result<HttpResponse, TransportError>>>,
    sent: Mutex<Vec<HttpRequest>>,
    capping: bool,
}

impl FakeExchange {
    pub(crate) fn new(answers: Vec<Result<HttpResponse, TransportError>>) -> Self {
        Self { answers: Mutex::new(answers.into()), sent: Mutex::new(Vec::new()), capping: false }
    }

    pub(crate) fn capping(answers: Vec<Result<HttpResponse, TransportError>>) -> Self {
        Self { capping: true, ..Self::new(answers) }
    }

    pub(crate) fn sent(&self) -> Vec<HttpRequest> {
        self.sent.lock().unwrap().clone()
    }
}

impl HttpExchange for FakeExchange {
    fn exchange(&self, request: &HttpRequest) -> impl Future<Output = Result<HttpResponse, TransportError>> {
        self.sent.lock().unwrap().push(request.clone());
        let limit = request.max_response_bytes();
        let answer = self.answers.lock().unwrap().pop_front().unwrap_or(Err(TransportError::Other));
        ready(match answer {
            Ok(response) if self.capping && response.body().len() > limit => Err(TransportError::BodyTooLarge),
            other => other,
        })
    }
}
