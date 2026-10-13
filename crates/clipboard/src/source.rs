//! Coordinate shared-display reads with winit, including wakeups from channels and pipes.
use sctk::reexports::calloop::{EventIterator, EventSource, Poll, PostAction, Readiness, Token, TokenFactory};
use sctk::reexports::calloop_wayland_source::WaylandSource;
use sctk::reexports::client::{DispatchError, EventQueue};
use winit::platform::wayland::WaylandClipboard;
pub(crate) struct Source<D> {
    pub inner: WaylandSource<D>,
    pub clipboard: WaylandClipboard,
}
impl<D> EventSource for Source<D> {
    type Error = sctk::reexports::calloop::Error;
    type Event = ();
    type Metadata = EventQueue<D>;
    type Ret = Result<usize, DispatchError>;
    const NEEDS_EXTRA_LIFECYCLE_EVENTS: bool = true;
    fn process_events<F>(&mut self, readiness: Readiness, token: Token, callback: F) -> Result<PostAction, Self::Error>
    where
        F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        self.inner.process_events(readiness, token, callback)
    }
    fn register(&mut self, poll: &mut Poll, factory: &mut TokenFactory) -> sctk::reexports::calloop::Result<()> {
        self.inner.register(poll, factory)
    }
    fn reregister(&mut self, poll: &mut Poll, factory: &mut TokenFactory) -> sctk::reexports::calloop::Result<()> {
        self.inner.reregister(poll, factory)
    }
    fn unregister(&mut self, poll: &mut Poll) -> sctk::reexports::calloop::Result<()> {
        self.inner.unregister(poll)
    }
    fn before_sleep(&mut self) -> sctk::reexports::calloop::Result<Option<(Readiness, Token)>> {
        self.inner.before_sleep()
    }
    fn before_handle_events(&mut self, events: EventIterator<'_>) {
        // A pipe/channel can wake this loop while winit is waiting on the display. libwayland
        // coordinates prepared reads across queues; wake winit before entering that read.
        self.clipboard.wake_event_loop();
        self.inner.before_handle_events(events);
    }
}
