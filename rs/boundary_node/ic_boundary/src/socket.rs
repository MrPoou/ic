use std::{
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::Path,
};

use axum::extract::connect_info::Connected;
use futures_util::ready;
use hyper::server::accept::Accept;
use hyper::server::{Builder, Server};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::net::{TcpListener, TcpSocket, TcpStream, UnixListener, UnixSocket, UnixStream};

// These are used in case the peer_addr() below fails for whatever reason
const DEFAULT_IP_ADDR: IpAddr = IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0));
const DEFAULT_SOCK_ADDR: SocketAddr = SocketAddr::new(DEFAULT_IP_ADDR, 0);

// Custom extractor of ConnectInfo for our Tcp listener, default does not work with it
// TODO support TLS also
#[derive(Clone)]
pub struct TcpConnectInfo(pub SocketAddr);

impl Connected<&TcpStream> for TcpConnectInfo {
    fn connect_info(target: &TcpStream) -> Self {
        Self(target.peer_addr().unwrap_or(DEFAULT_SOCK_ADDR))
    }
}

// Unix socket handler
pub struct SocketUnix {
    listener: UnixListener,
}

impl SocketUnix {
    pub fn bind(path: impl AsRef<Path>, backlog: u32) -> Result<Self, std::io::Error> {
        let socket = UnixSocket::new_stream()?;
        socket.bind(path)?;
        let listener = socket.listen(backlog)?;
        Ok(Self { listener })
    }
}

impl Accept for SocketUnix {
    type Conn = UnixStream;
    type Error = io::Error;

    fn poll_accept(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        let conn = ready!(self.listener.poll_accept(cx))?.0;
        Poll::Ready(Some(Ok(conn)))
    }
}

// TCP socket handler
pub struct SocketTcp {
    listener: TcpListener,
}

impl SocketTcp {
    pub fn bind(addr: SocketAddr, backlog: u32) -> Result<Self, std::io::Error> {
        let socket = TcpSocket::new_v6()?;
        socket.bind(addr)?;
        let listener = socket.listen(backlog)?;
        Ok(Self { listener })
    }
}

impl Accept for SocketTcp {
    type Conn = TcpStream;
    type Error = io::Error;

    fn poll_accept(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        let conn = ready!(self.listener.poll_accept(cx))?.0;
        Poll::Ready(Some(Ok(conn)))
    }
}

// Convenience method for constructing a Hyper Server listening on a Unix socket.
pub trait UnixServerExt {
    fn bind_unix(path: impl AsRef<Path>, backlog: u32) -> Result<Builder<SocketUnix>, io::Error>;
}

pub trait TcpServerExt {
    fn bind_tcp(addr: SocketAddr, backlog: u32) -> Result<Builder<SocketTcp>, io::Error>;
}

impl UnixServerExt for Server<SocketUnix, ()> {
    fn bind_unix(path: impl AsRef<Path>, backlog: u32) -> Result<Builder<SocketUnix>, io::Error> {
        let incoming = SocketUnix::bind(path, backlog)?;
        Ok(Server::builder(incoming))
    }
}

impl TcpServerExt for Server<SocketTcp, ()> {
    fn bind_tcp(addr: SocketAddr, backlog: u32) -> Result<Builder<SocketTcp>, io::Error> {
        let incoming = SocketTcp::bind(addr, backlog)?;
        Ok(Server::builder(incoming))
    }
}
