use std::{
    io,
    net::{IpAddr, SocketAddr},
    os::unix::prelude::{FromRawFd, IntoRawFd}, sync::Arc,
};

use anyhow::Result;
use async_trait::async_trait;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{TcpSocket, UdpSocket, TcpStream},
};

mod tun;
pub enum NetworkType {
    TCP,
    UDP,
}

pub struct TransportNetwork {
    pub addr: SocketAddr,
    pub net_type: NetworkType,
}
pub mod socks;

pub trait RWSocketTrait: AsyncRead + AsyncWrite + Unpin + Sync + Send {}

pub type RWSocket = Box<dyn RWSocketTrait>;

pub struct DomainSession {
    name: String,
    port: u16,
}

pub enum Address {
    Domain(String),
    Ip(IpAddr),
}
// connection session
pub struct ConnSession {
    host: Address,
    port: u16,
}

pub fn create_bounded_udp_socket(addr: IpAddr) -> io::Result<UdpSocket> {
    let socket = match addr {
        IpAddr::V4(..) => Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?,
        IpAddr::V6(..) => Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?,
    };
    // let s: SockAddr = ;
    match socket.bind(&SockAddr::from(SocketAddr::new(addr, 0))) {
        Ok(..) => {},
        Err(err) => {
            log::error!("failed to bind socket {}", err.to_string())
        }
    }
    match socket.set_nonblocking(true) {
        Ok(..) => {},
        Err(err) => {
            log::error!("failed to set non blocking {}", err)
        }
    }
    Ok(UdpSocket::from_std(socket.into())?)
}

pub fn create_bounded_tcp_socket(addr: SocketAddr) -> io::Result<TcpSocket> {
    let socket = match addr {
        SocketAddr::V4(..) => Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?,
        SocketAddr::V6(..) => Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))?,
    };
    socket.bind(&addr.into());
    socket.set_nonblocking(true);
    Ok(TcpSocket::from_std_stream(socket.into()))
}



// ----------------------------
// INBOUND
pub enum InboundResult {
    Stream(TcpStream, ConnSession),
    Datagram(UdpSocket, ConnSession),
}

pub type AnyTcpInboundHandler = Arc<dyn TcpInboundHandlerTrait>;
pub type AnyUdpInboundHandler = Arc<dyn UdpInboundHandlerTrait>;
pub type AnyInboundHandler = Arc<dyn InboundHandlerTrait>;

pub struct InboundHandler {
    tag: String,
    tcp_handler: Option<AnyTcpInboundHandler>,
    udp_handler: Option<AnyUdpInboundHandler>,
}

impl InboundHandler {
    pub fn new(tag: String, tcp: Option<AnyTcpInboundHandler>, udp: Option<AnyUdpInboundHandler>) -> InboundHandler {
        InboundHandler {
            tag,
            tcp_handler: tcp,
            udp_handler: udp,
        }
    }
}

pub trait InboundHandlerTrait: TcpInboundHandlerTrait + UdpInboundHandlerTrait + Sync + Send {
    fn has_tcp(&self) -> bool;
    fn has_udp(&self) -> bool;
}

#[async_trait]
pub trait TcpInboundHandlerTrait {
    async fn handle(&self, session: ConnSession, stream: TcpStream) -> io::Result<InboundResult>;
}

#[async_trait]
pub trait UdpInboundHandlerTrait {
    async fn handle(&self, session: ConnSession, socket: tokio::net::UdpSocket) -> io::Result<InboundResult>;
}


// OUTBOUND


pub enum OutboundResult {
    Stream(TcpStream),
    Datagram(UdpSocket),
}

#[async_trait]
pub trait TcpOutboundHandlerTrait {
    async fn handle(&self, sess: ConnSession) -> io::Result<OutboundResult>;
}

#[async_trait]
pub trait UdpOutboundHandlerTrait {
    async fn handle(&self, sess: ConnSession) -> io::Result<OutboundResult>;
}

pub type AnyTcpOutboundHandler = Arc<dyn TcpOutboundHandlerTrait>;
pub type AnyUdpOutboundHandler = Arc<dyn UdpOutboundHandlerTrait>;

pub struct OutboundHandler {
    tag: String,
    tcp_handler: Option<AnyTcpOutboundHandler>,
    udp_handler: Option<AnyUdpOutboundHandler>,
}