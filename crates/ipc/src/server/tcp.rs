//! Optional loopback TCP. Off unless [`crate::server::ServerConfig::tcp_port`] is set.
//! It binds 127.0.0.1 only and runs the same token handshake, unauthenticated cap and
//! deadlines as the local socket. The 16-connection cap is shared
//! with the local listener. The instance record (0600) carries the port and the token.

use std::net::{TcpListener, TcpStream};

use crate::error::LinkError;

pub(super) struct BoundTcp {
    pub listener: TcpListener,
    pub port: u16,
}

pub(super) fn bind_loopback(port: u16) -> Result<BoundTcp, LinkError> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    Ok(BoundTcp { listener, port })
}

pub(super) fn poll_accept(listener: &TcpListener) -> std::io::Result<Option<TcpStream>> {
    match listener.accept() {
        Ok((stream, _addr)) => Ok(Some(stream)),
        Err(err) if is_idle(&err) => Ok(None),
        Err(err) => Err(err),
    }
}

fn is_idle(err: &std::io::Error) -> bool {
    matches!(err.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted)
}
