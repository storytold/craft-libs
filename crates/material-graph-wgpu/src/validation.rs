use std::future::Future;
use std::task::{Context, Poll, Waker};

/// Native wgpu resolves a popped error scope immediately.
fn caught(error: impl Future<Output = Option<wgpu::Error>>) -> Option<wgpu::Error> {
    let mut error = std::pin::pin!(error);
    match error.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(error) => error,
        Poll::Pending => None,
    }
}

/// Turn validation and allocation failures into errors so the evaluator can fall back to the CPU.
pub(crate) fn scoped<T>(device: &wgpu::Device, what: &str, f: impl FnOnce() -> T) -> Result<T, String> {
    let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let invalid = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let out = f();
    let errors = [caught(invalid.pop()), caught(oom.pop())];
    match errors.into_iter().flatten().next() {
        Some(error) => Err(format!("{what}: {error}")),
        None => Ok(out),
    }
}
